//! « My ref » : bibliothèque personnelle d'images de référence, rangées en collections (à la manière de Cosmos).
//!
//! Tout vit dans un dossier choisi par l'utilisateur, en fichiers ordinaires, jamais dans une base propre à
//! l'application : quelle que soit la version de Lumitype (ou sans Lumitype du tout), on retrouve ses images.
//!
//! ```text
//! My ref/
//! ├─ myref.json            format, version, ordre des collections, ordre des images, couverture, tri
//! ├─ Dessin/               une collection = un vrai dossier (renommer la collection renomme le dossier)
//! │  ├─ ville-brume.jpg    copie de l'image importée (nom d'origine, dédoublonné)
//! │  └─ ville-brume.jpg.json   fiche : nom affiché, source, tags, note, favori, date d'ajout, taille en pixels
//! ├─ .apercus/             vignettes WebP (cache : peut être effacé, il se reconstruit)
//! └─ .corbeille/           ce qui a été supprimé (jamais effacé pour de bon)
//! ```
//!
//! Robuste aux retouches à la main : au chargement, une image posée dans un dossier sans être connue de
//! `myref.json` y est ajoutée, un fichier disparu en est retiré sans bruit, un dossier nouveau devient une
//! collection. Les champs inconnus des fichiers JSON (versions futures) sont gardés tels quels à la réécriture.
//! Toute écriture est atomique (fichier temporaire puis renommage).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Identifiant du format, écrit dans `myref.json`.
pub const FORMAT: &str = "lumitype-myref";
/// Version du format. Une version plus récente reste lisible : ses champs inconnus sont conservés.
pub const VERSION: u32 = 1;
const INDEX_FILE: &str = "myref.json";
const THUMBS_DIR: &str = ".apercus";
const TRASH_DIR: &str = ".corbeille";
/// Côté long des vignettes, en pixels.
const THUMB_SIDE: u32 = 480;
/// Aperçu JPEG embarqué d'un RAW : utilisé à partir de cette taille (côté long) pour la visionneuse, plutôt que de
/// développer le capteur à chaque affichage. La plupart des boîtiers en embarquent un de 1600 px ou plus.
const RAW_PREVIEW_MIN: u32 = 1600;
/// Côté long de l'image montrée dans la visionneuse quand la webview ne sait pas lire le format d'origine.
const PREVIEW_SIDE: u32 = 2400;
/// Vignettes : WebP avec perte (libwebp). Mesuré sur des photos de 480 px : ~35 Ko contre ~65 Ko en JPEG q82,
/// pour 3 ms d'encodage de plus ; `method` 2 (sur 0 à 6) garde l'encodage rapide.
const THUMB_QUALITY: f32 = 75.0;
const THUMB_METHOD: i32 = 2;
/// Fond posé sous les zones transparentes des vignettes (sans couche alpha) : gris des cartes.
const THUMB_BACKDROP: [u8; 3] = [42, 40, 40];
/// Formats reconnus dans les dossiers : ceux que Lumitype sait ouvrir.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "jfif", "webp", "tif", "tiff", "avif", "heic", "heif"];
/// Formats que la webview affiche elle-même : la visionneuse reçoit le fichier tel quel.
const WEB_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "jfif", "webp", "avif"];

// ------------------------------------------------------------------ fichiers JSON

/// `myref.json`, à la racine de la bibliothèque.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
struct Index {
    #[serde(default)]
    format: String,
    #[serde(default)]
    version: u32,
    #[serde(default)]
    collections: Vec<CollectionEntry>,
    /// Champs inconnus (version future, ajout à la main) : relus et réécrits tels quels.
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct CollectionEntry {
    /// Nom de la collection, qui est aussi le nom de son dossier.
    name: String,
    /// Noms des fichiers, dans l'ordre d'affichage.
    #[serde(default)]
    order: Vec<String>,
    /// Image choisie comme couverture (sinon : mosaïque des quatre derniers ajouts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cover: Option<String>,
    /// Tri choisi dans le menu « Trier » (voir [`SortOrder`]), tenu à jour aux ajouts ; effacé par un ordre fait
    /// à la main (Organiser).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sort: Option<SortOrder>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl CollectionEntry {
    fn new(name: String) -> Self {
        Self { name, order: Vec::new(), cover: None, sort: None, extra: Map::new() }
    }
}

/// Tris proposés par le menu « Trier » d'une collection.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    /// Date d'ajout, les plus récentes d'abord.
    Recent,
    /// Date d'ajout, les plus anciennes d'abord.
    Oldest,
    /// Nom affiché, de A à Z (les nombres dans l'ordre naturel : « 2 » avant « 10 »).
    Name,
}

/// Fiche posée à côté de chaque image : `photo.jpg.json`.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Sidecar {
    /// Nom affiché (par défaut, le nom du fichier).
    #[serde(default)]
    pub name: String,
    /// D'où vient l'image : chemin du fichier d'origine, adresse web, ou vide (glisser-déposer).
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub favorite: bool,
    /// Date d'ajout, en millisecondes depuis 1970.
    #[serde(default)]
    pub added: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Couleurs dominantes (#rrggbb), calculées avec la vignette.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// ------------------------------------------------------------------ ce que reçoit l'interface

/// Une image de la bibliothèque. `id` = « collection/fichier ».
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub collection: String,
    pub file: String,
    /// Chemin complet de la copie dans la bibliothèque (pour l'intégrer à un projet, l'ouvrir…).
    pub path: String,
    pub name: String,
    pub source: String,
    pub tags: Vec<String>,
    pub note: String,
    pub favorite: bool,
    pub added: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Poids du fichier, en octets.
    pub size: u64,
    pub colors: Vec<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSummary {
    pub name: String,
    pub count: usize,
    /// Image choisie comme couverture (« Changer la couverture… ») : montrée seule, en grand.
    pub cover: Option<Item>,
    /// Sans couverture choisie : mosaïque des quatre images ajoutées en dernier (les plus récentes d'abord).
    pub covers: Vec<Item>,
    pub favorites: usize,
    /// Date du dernier ajout (ms), 0 si vide.
    pub modified: u64,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub root: String,
    pub collections: Vec<CollectionSummary>,
    pub favorites: Vec<Item>,
    pub recent: Vec<Item>,
    pub total: usize,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CollectionView {
    pub name: String,
    pub items: Vec<Item>,
    pub cover: Option<String>,
    /// Tri mémorisé (coche du menu « Trier »), `None` pour un ordre fait à la main.
    pub sort: Option<SortOrder>,
    pub modified: u64,
}

/// Modification de la fiche d'une image : seuls les champs présents changent.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ItemPatch {
    pub name: Option<String>,
    pub tags: Option<Vec<String>>,
    pub note: Option<String>,
    pub favorite: Option<bool>,
}

// ------------------------------------------------------------------ outils

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn mtime_ms(path: &Path) -> u64 {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64)
}

fn err(what: &str, path: &Path, error: impl std::fmt::Display) -> String {
    format!("{what} {} impossible : {error}", path.display())
}

/// Nom de fichier temporaire caché, à côté de `path`, propre à cet appel : le numéro du processus ne suffit pas,
/// deux threads (vignettes calculées en parallèle) écriraient le même fichier et l'un tronquerait l'autre.
fn temp_path_beside(path: &Path, dir: &Path) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("fichier");
    dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)))
}

/// Écriture atomique : fichier temporaire caché dans le même dossier, puis renommage par-dessus l'ancien.
/// Un fichier n'est jamais laissé à moitié écrit, même si l'application s'arrête au milieu.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or_else(|| format!("chemin sans dossier : {}", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| err("création de", dir, e))?;
    let tmp = temp_path_beside(path, dir);
    std::fs::write(&tmp, bytes).map_err(|e| err("écriture de", &tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        err("écriture de", path, e)
    })
}

/// Copie atomique : l'image n'apparaît dans la collection qu'une fois entière.
fn copy_atomic(from: &Path, to: &Path) -> Result<(), String> {
    let dir = to.parent().ok_or_else(|| format!("chemin sans dossier : {}", to.display()))?;
    let tmp = temp_path_beside(to, dir);
    std::fs::copy(from, &tmp).map_err(|e| err("copie de", from, e))?;
    std::fs::rename(&tmp, to).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        err("copie vers", to, e)
    })
}

fn extension_of(name: &str) -> String {
    Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase()
}

fn is_image_name(name: &str) -> bool {
    let extension = extension_of(name);
    IMAGE_EXTENSIONS.contains(&extension.as_str()) || engine::io::RAW_EXTENSIONS.contains(&extension.as_str())
}

/// Un nom de dossier ou de fichier déjà présent dans la bibliothèque (venu de l'interface) : jamais de
/// séparateur, jamais `..`, jamais un nom caché (les dossiers de service commencent par un point).
fn valid_component(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\']) && name != ".." && !name.contains('\0')
}

/// Nom propre pour un dossier ou un fichier, sur tous les systèmes : sans caractères interdits par Windows,
/// sans point ni espace en tête ou en fin, sans nom réservé (CON, NUL…), de longueur raisonnable.
/// Nom affiché d'une image : celui de sa fiche, sans l'extension du fichier (« photo.jpg » → « photo »).
fn display_name(name: &str, file: &str) -> String {
    let ext = Path::new(file).extension().and_then(|e| e.to_str()).unwrap_or_default();
    match name.rsplit_once('.') {
        Some((stem, tail)) if !stem.is_empty() && !ext.is_empty() && tail.eq_ignore_ascii_case(ext) => stem.to_owned(),
        _ => name.to_owned(),
    }
}

pub fn sanitize_name(raw: &str, fallback: &str) -> String {
    let cleaned: String = raw.chars().map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { ' ' } else { c }).collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let mut name: String = trimmed.chars().take(120).collect();
    name = name.trim_end_matches(|c: char| c == '.' || c.is_whitespace()).to_owned();
    if name.is_empty() {
        name = fallback.to_owned();
    }
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
    if RESERVED.contains(&stem.as_str()) {
        name.insert(0, '_');
    }
    name
}

/// `nom.ext` s'il est libre dans `dir`, sinon `nom (2).ext`, `nom (3).ext`… La comparaison ignore la casse :
/// deux fichiers ne différant que par elle se marcheraient dessus sous Windows et macOS.
fn unique_name(dir: &Path, wanted: &str, except: Option<&str>) -> String {
    let taken: HashSet<String> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().filter_map(|e| e.file_name().to_str().map(str::to_lowercase)).collect())
        .unwrap_or_default();
    let free = |name: &str| except.is_some_and(|e| e.eq_ignore_ascii_case(name)) || !taken.contains(&name.to_lowercase()) && !taken.contains(&format!("{}.json", name.to_lowercase()));
    if free(wanted) {
        return wanted.to_owned();
    }
    let path = Path::new(wanted);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(wanted);
    let ext = path.extension().and_then(|e| e.to_str());
    (2..)
        .map(|n| match ext {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        })
        .find(|candidate| free(candidate))
        .unwrap_or_else(|| wanted.to_owned())
}

/// Format d'une image d'après ses premiers octets (une image glissée depuis le web n'a pas toujours d'extension).
pub fn sniff_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some("tif")
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        match &bytes[8..12] {
            b"avif" | b"avis" => Some("avif"),
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"hevm" | b"hevs" | b"mif1" | b"msf1" => Some("heic"),
            _ => None,
        }
    } else {
        None
    }
}

/// Taille en pixels lue dans l'en-tête seulement (rapide) ; `None` pour les formats que la bibliothèque `image`
/// ne connaît pas (HEIC, AVIF, RAW) : elle sera remplie au calcul de la vignette.
fn header_dimensions(path: &Path) -> Option<(u32, u32)> {
    if matches!(extension_of(path.to_str().unwrap_or_default()).as_str(), "heic" | "heif" | "avif") || engine::io::is_raw_path(path) {
        return None;
    }
    image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_dimensions().ok()
}

/// Jusqu'à quatre couleurs dominantes, assez différentes les unes des autres : histogramme grossier
/// (4 bits par canal) des pixels de la vignette.
fn dominant_colors(rgb: &[u8]) -> Vec<String> {
    let mut bins = vec![(0u32, [0u64; 3]); 4096];
    for px in rgb.chunks_exact(3) {
        let key = (usize::from(px[0] >> 4) << 8) | (usize::from(px[1] >> 4) << 4) | usize::from(px[2] >> 4);
        let bin = &mut bins[key];
        bin.0 += 1;
        for c in 0..3 {
            bin.1[c] += u64::from(px[c]);
        }
    }
    let mut ranked: Vec<[u8; 3]> = Vec::new();
    let mut order: Vec<usize> = (0..bins.len()).filter(|&i| bins[i].0 > 0).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(bins[i].0));
    for i in order {
        let (count, sum) = bins[i];
        let color = [(sum[0] / u64::from(count)) as u8, (sum[1] / u64::from(count)) as u8, (sum[2] / u64::from(count)) as u8];
        let distinct = ranked.iter().all(|c| (0..3).map(|k| (i32::from(c[k]) - i32::from(color[k])).abs()).sum::<i32>() > 60);
        if distinct {
            ranked.push(color);
        }
        if ranked.len() == 4 {
            break;
        }
    }
    ranked.iter().map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])).collect()
}

/// Limite le nombre de décodages simultanés : une photo de 24 Mpx décodée occupe ~100 Mo, et l'interface
/// demande les vignettes d'une collection d'un coup.
struct Gate {
    busy: Mutex<usize>,
    freed: Condvar,
}
static DECODERS: Gate = Gate { busy: Mutex::new(0), freed: Condvar::new() };
const MAX_DECODERS: usize = 2;

/// Place prise dans la barrière, rendue à la destruction : même si `work` panique (image corrompue qui fait
/// échouer le décodeur), la place est libérée et les décodages suivants ne restent pas bloqués.
struct GateSlot<'a>(&'a Gate);

impl Drop for GateSlot<'_> {
    fn drop(&mut self) {
        *self.0.busy.lock().unwrap_or_else(std::sync::PoisonError::into_inner) -= 1;
        self.0.freed.notify_one();
    }
}

impl Gate {
    fn run<T>(&self, work: impl FnOnce() -> T) -> T {
        let _slot = self.acquire();
        work()
    }

    fn acquire(&self) -> GateSlot<'_> {
        let mut busy = self.busy.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while *busy >= MAX_DECODERS {
            busy = self.freed.wait(busy).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *busy += 1;
        GateSlot(self)
    }
}

/// Décode une image de la bibliothèque et la réduit à `side` pixels de côté long (jamais agrandie), posée sur le
/// fond des cartes. Rend le RGB réduit, ses dimensions, et la taille d'origine.
/// Réduction de l'image à `side` pixels de côté long, sur le fond des vignettes. La taille d'origine n'est rendue
/// que si l'image a été décodée en entier : un RAW passe par son aperçu JPEG embarqué (rapide), qui ne la donne pas.
fn decode_reduced(path: &Path, side: u32) -> Result<(Vec<u8>, u32, u32, Option<(u32, u32)>), String> {
    if extension_of(path.to_str().unwrap_or_default()) == "avif" {
        // Le moteur laisse l'AVIF à la webview (voir engine::io) : l'interface montre alors le fichier lui-même.
        return Err("AVIF : aperçu laissé à l'affichage".into());
    }
    let bytes = std::fs::read(path).map_err(|e| err("lecture de", path, e))?;
    let (image, full) = match engine::io::raw_preview(&bytes, path, side.min(RAW_PREVIEW_MIN)) {
        Some(preview) => (preview, false),
        None => {
            let (width, height, rgba) = DECODERS.run(|| engine::io::decode_premultiplied(&bytes, path)).map_err(|e| e.to_string())?;
            (image::RgbaImage::from_raw(width, height, rgba).ok_or("tampon décodé de taille inattendue")?, true)
        }
    };
    drop(bytes);
    let (width, height) = image.dimensions();
    let scale = (side as f32 / width.max(height) as f32).min(1.0);
    let (w, h) = (((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1));
    let small = if (w, h) == (width, height) { image } else { image::imageops::thumbnail(&image, w, h) };
    // Prémultiplié : composer sur le fond revient à ajouter fond × (1 − alpha).
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for px in small.pixels() {
        let rest = 255 - u16::from(px[3]);
        for c in 0..3 {
            rgb.push((u16::from(px[c]) + (u16::from(THUMB_BACKDROP[c]) * rest + 127) / 255).min(255) as u8);
        }
    }
    Ok((rgb, w, h, full.then_some((width, height))))
}

fn encode_jpeg(rgb: &[u8], width: u32, height: u32, quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode(rgb, width, height, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("encodage de l'image impossible : {e}"))?;
    Ok(out)
}

/// Vignette WebP avec perte (libwebp), réglages ci-dessus.
fn encode_webp(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut config = webp::WebPConfig::new().map_err(|()| "réglages WebP indisponibles".to_owned())?;
    config.lossless = 0;
    config.quality = THUMB_QUALITY;
    config.method = THUMB_METHOD;
    let encoded = webp::Encoder::from_rgb(rgb, width, height).encode_advanced(&config).map_err(|e| format!("encodage de la vignette impossible : {e:?}"))?;
    Ok(encoded.to_vec())
}

/// Comparaison « naturelle », sans tenir compte de la casse : les suites de chiffres comptent comme des nombres
/// (« croquis 2 » avant « croquis 10 »), comme le tri des gestionnaires de fichiers.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (m, n) = (take(&mut x), take(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let order = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(&d);
                }
                x.next();
                y.next();
            }
        }
    }
}

/// Ouvre un dossier dans le gestionnaire de fichiers du système (le dossier lui-même, pas son parent).
fn open_folder(dir: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let spawned = {
        let dir = std::fs::canonicalize(dir).map(|p| crate::commands::plain_path(&p)).unwrap_or_else(|_| dir.to_path_buf());
        std::process::Command::new("explorer").arg(dir).spawn()
    };
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(dir).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(dir).spawn();
    spawned.map(|_| ()).map_err(|e| err("ouverture de", dir, e))
}

// ------------------------------------------------------------------ bibliothèque

pub struct Library {
    root: PathBuf,
}

impl Library {
    /// Ouvre la bibliothèque rangée dans `root`, un dossier qui doit exister (choisi dans la boîte du système).
    /// Il n'est jamais recréé ici : un disque externe débranché donnerait sinon un dossier vide à sa place.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root: PathBuf = root.into();
        if root.as_os_str().is_empty() {
            return Err("aucun dossier My ref choisi".into());
        }
        if !root.is_dir() {
            return Err(format!("dossier My ref introuvable : {} (disque débranché, dossier déplacé ?)", root.display()));
        }
        Ok(Self { root })
    }

    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn index_path(&self) -> PathBuf {
        self.root.join(INDEX_FILE)
    }

    fn collection_dir(&self, name: &str) -> Result<PathBuf, String> {
        if !valid_component(name) {
            return Err(format!("nom de collection invalide : {name}"));
        }
        let dir = self.root.join(name);
        if !dir.is_dir() {
            return Err(format!("collection introuvable : {name}"));
        }
        Ok(dir)
    }

    /// `collection/fichier` → (collection, fichier, chemin), vérifiés.
    fn resolve(&self, id: &str) -> Result<(String, String, PathBuf), String> {
        let (collection, file) = id.split_once('/').ok_or_else(|| format!("image invalide : {id}"))?;
        if !valid_component(file) {
            return Err(format!("image invalide : {id}"));
        }
        let path = self.collection_dir(collection)?.join(file);
        if !path.is_file() {
            return Err(format!("image introuvable : {id}"));
        }
        Ok((collection.to_owned(), file.to_owned(), path))
    }

    fn sidecar_path(image: &Path) -> PathBuf {
        let mut name = image.file_name().unwrap_or_default().to_os_string();
        name.push(".json");
        image.with_file_name(name)
    }

    /// Vignette en cache : `.apercus/<collection>/<fichier>.webp`.
    fn thumb_path(&self, collection: &str, file: &str) -> PathBuf {
        self.root.join(THUMBS_DIR).join(collection).join(format!("{file}.webp"))
    }

    /// Vignette JPEG des versions précédentes (`<fichier>.jpg`) : encore servie tant qu'elle est à jour, supprimée
    /// dès que la vignette WebP la remplace.
    fn legacy_thumb_path(&self, collection: &str, file: &str) -> PathBuf {
        self.root.join(THUMBS_DIR).join(collection).join(format!("{file}.jpg"))
    }

    /// Image renommée, déplacée ou supprimée : ses vignettes (WebP et ancienne JPEG) n'ont plus lieu d'être.
    fn remove_thumbs(&self, collection: &str, file: &str) {
        let _ = std::fs::remove_file(self.thumb_path(collection, file));
        let _ = std::fs::remove_file(self.legacy_thumb_path(collection, file));
    }

    /// Dossier d'une collection, pour l'ouvrir dans le gestionnaire de fichiers. Le chemin est construit ici, d'après
    /// le nom seul, jamais reçu tel quel de l'interface : nom d'un seul élément (ni séparateur, ni `..`, ni dossier de
    /// service caché comme `.corbeille`), dossier existant, et collection connue de l'index.
    pub fn collection_folder(&self, name: &str) -> Result<PathBuf, String> {
        let dir = self.collection_dir(name)?;
        if !self.index()?.collections.iter().any(|c| c.name == name) {
            return Err(format!("collection introuvable : {name}"));
        }
        Ok(dir)
    }

    /// Lit `myref.json`. Absent : index vide. Illisible (retouche ratée) : mis de côté sous un autre nom, et un
    /// index neuf est reconstruit d'après les dossiers — ni les images ni leurs fiches ne sont perdues.
    fn load_index(&self) -> Index {
        let path = self.index_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Index>(&text) {
                Ok(index) => index,
                Err(_) => {
                    let _ = std::fs::rename(&path, self.root.join(format!("{INDEX_FILE}.illisible-{}", now_ms())));
                    Index::default()
                }
            },
            Err(_) => Index::default(),
        }
    }

    fn save_index(&self, index: &Index) -> Result<(), String> {
        let text = serde_json::to_string_pretty(index).map_err(|e| e.to_string())?;
        write_atomic(&self.index_path(), text.as_bytes())
    }

    /// Images d'un dossier de collection (fichiers visibles aux extensions reconnues), par nom.
    fn files_in(dir: &Path) -> Vec<String> {
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .filter_map(|e| e.file_name().to_str().map(str::to_owned))
                    .filter(|name| !name.starts_with('.') && is_image_name(name))
                    .collect()
            })
            .unwrap_or_default();
        files.sort_by_key(|name| name.to_lowercase());
        files
    }

    /// L'index accordé au contenu réel du dossier : collections et images ajoutées ou retirées à la main.
    /// Réécrit seulement s'il a changé.
    fn index(&self) -> Result<Index, String> {
        let original = self.load_index();
        let mut index = original.clone();
        index.format = FORMAT.to_owned();
        index.version = index.version.max(VERSION);
        let mut dirs: Vec<String> = std::fs::read_dir(&self.root)
            .map_err(|e| err("lecture de", &self.root, e))?
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .filter(|name| valid_component(name))
            .collect();
        dirs.sort_by_key(|name| name.to_lowercase());
        let mut seen = HashSet::new();
        index.collections.retain(|c| dirs.contains(&c.name) && seen.insert(c.name.clone()));
        for dir in &dirs {
            if !seen.contains(dir) {
                index.collections.push(CollectionEntry::new(dir.clone()));
            }
        }
        for entry in &mut index.collections {
            let files = Self::files_in(&self.root.join(&entry.name));
            let present: HashSet<&String> = files.iter().collect();
            let mut kept = HashSet::new();
            entry.order.retain(|f| present.contains(f) && kept.insert(f.clone()));
            // Images posées à la main : à la suite, par date de modification (les plus anciennes d'abord).
            let mut new: Vec<&String> = files.iter().filter(|f| !kept.contains(*f)).collect();
            new.sort_by_key(|f| mtime_ms(&self.root.join(&entry.name).join(f)));
            entry.order.extend(new.into_iter().cloned());
            if entry.cover.as_ref().is_some_and(|c| !present.contains(c)) {
                entry.cover = None;
            }
        }
        if index != original || !self.index_path().is_file() {
            self.save_index(&index)?;
        }
        Ok(index)
    }

    /// Fiche d'une image. Absente ou illisible : fiche par défaut (nom du fichier, date du fichier).
    fn read_sidecar(&self, image: &Path) -> Sidecar {
        let parsed = std::fs::read_to_string(Self::sidecar_path(image)).ok().and_then(|text| serde_json::from_str::<Sidecar>(&text).ok());
        let mut sidecar = parsed.unwrap_or_default();
        if sidecar.name.trim().is_empty() {
            sidecar.name = image.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        }
        if sidecar.added == 0 {
            sidecar.added = mtime_ms(image);
        }
        sidecar
    }

    fn write_sidecar(&self, image: &Path, sidecar: &Sidecar) -> Result<(), String> {
        let text = serde_json::to_string_pretty(sidecar).map_err(|e| e.to_string())?;
        write_atomic(&Self::sidecar_path(image), text.as_bytes())
    }

    fn item(&self, collection: &str, file: &str) -> Item {
        let path = self.root.join(collection).join(file);
        let sidecar = self.read_sidecar(&path);
        Item {
            id: format!("{collection}/{file}"),
            collection: collection.to_owned(),
            file: file.to_owned(),
            size: std::fs::metadata(&path).map_or(0, |m| m.len()),
            path: path.to_string_lossy().into_owned(),
            name: display_name(&sidecar.name, file),
            source: sidecar.source,
            tags: sidecar.tags,
            note: sidecar.note,
            favorite: sidecar.favorite,
            added: sidecar.added,
            width: sidecar.width,
            height: sidecar.height,
            colors: sidecar.colors,
        }
    }

    fn items_of(&self, entry: &CollectionEntry) -> Vec<Item> {
        entry.order.iter().map(|file| self.item(&entry.name, file)).collect()
    }

    // ---------------------------------------------------------------- lecture

    /// Vue d'ensemble : collections (nombre, couverture), favoris et derniers ajouts.
    pub fn overview(&self) -> Result<Overview, String> {
        let index = self.index()?;
        let mut all = Vec::new();
        let mut collections = Vec::new();
        for entry in &index.collections {
            let items = self.items_of(entry);
            let cover = entry.cover.as_ref().and_then(|c| items.iter().find(|i| &i.file == c)).cloned();
            let mut latest: Vec<&Item> = items.iter().collect();
            latest.sort_by_key(|i| std::cmp::Reverse(i.added));
            let covers = if cover.is_some() { Vec::new() } else { latest.into_iter().take(4).cloned().collect() };
            collections.push(CollectionSummary {
                name: entry.name.clone(),
                count: items.len(),
                cover,
                covers,
                favorites: items.iter().filter(|i| i.favorite).count(),
                modified: items.iter().map(|i| i.added).max().unwrap_or(0),
            });
            all.extend(items);
        }
        let total = all.len();
        all.sort_by_key(|i| std::cmp::Reverse(i.added));
        let favorites = all.iter().filter(|i| i.favorite).take(48).cloned().collect();
        let recent = all.into_iter().take(48).collect();
        Ok(Overview { root: self.root.to_string_lossy().into_owned(), collections, favorites, recent, total })
    }

    pub fn collection(&self, name: &str) -> Result<CollectionView, String> {
        self.collection_dir(name)?;
        let index = self.index()?;
        let entry = index.collections.iter().find(|c| c.name == name).ok_or_else(|| format!("collection introuvable : {name}"))?;
        let items = self.items_of(entry);
        let modified = items.iter().map(|i| i.added).max().unwrap_or(0);
        Ok(CollectionView { name: entry.name.clone(), items, cover: entry.cover.clone(), sort: entry.sort, modified })
    }

    /// Images dont le nom, le fichier, un tag ou la note contient `query` (sans tenir compte de la casse).
    pub fn search(&self, query: &str) -> Result<Vec<Item>, String> {
        let query = query.trim().to_lowercase();
        let index = self.index()?;
        let mut found = Vec::new();
        for entry in &index.collections {
            for item in self.items_of(entry) {
                let hit = query.is_empty()
                    || item.name.to_lowercase().contains(&query)
                    || item.file.to_lowercase().contains(&query)
                    || item.note.to_lowercase().contains(&query)
                    || item.tags.iter().any(|t| t.to_lowercase().contains(&query));
                if hit {
                    found.push(item);
                }
                if found.len() >= 500 {
                    return Ok(found);
                }
            }
        }
        Ok(found)
    }

    // ---------------------------------------------------------------- collections

    pub fn create_collection(&self, wanted: &str) -> Result<String, String> {
        let name = unique_name(&self.root, &sanitize_name(wanted, "Nouvelle collection"), None);
        let dir = self.root.join(&name);
        std::fs::create_dir(&dir).map_err(|e| err("création de", &dir, e))?;
        let mut index = self.index()?;
        // `index()` l'a déjà ajoutée à la fin ; rien d'autre à faire, sauf garantir l'écriture.
        if !index.collections.iter().any(|c| c.name == name) {
            index.collections.push(CollectionEntry::new(name.clone()));
            self.save_index(&index)?;
        }
        Ok(name)
    }

    /// Renomme la collection ET son dossier. Rend le nom retenu (nettoyé, dédoublonné).
    pub fn rename_collection(&self, old: &str, wanted: &str) -> Result<String, String> {
        let from = self.collection_dir(old)?;
        let clean = sanitize_name(wanted, old);
        if clean == old {
            return Ok(clean);
        }
        let name = unique_name(&self.root, &clean, Some(old));
        let to = self.root.join(&name);
        if name.eq_ignore_ascii_case(old) {
            // Changement de casse seul : en deux temps, sinon c'est le même dossier pour Windows et macOS.
            let step = self.root.join(format!(".renommage-{}", now_ms()));
            std::fs::rename(&from, &step).map_err(|e| err("renommage de", &from, e))?;
            std::fs::rename(&step, &to).map_err(|e| err("renommage de", &from, e))?;
        } else {
            std::fs::rename(&from, &to).map_err(|e| err("renommage de", &from, e))?;
        }
        // Vignettes : suivent le dossier (sinon elles seraient simplement recalculées).
        let thumbs = self.root.join(THUMBS_DIR);
        let _ = std::fs::remove_dir_all(thumbs.join(&name));
        let _ = std::fs::rename(thumbs.join(old), thumbs.join(&name));
        let mut index = self.load_index();
        for entry in &mut index.collections {
            if entry.name == old {
                entry.name = name.clone();
            }
        }
        self.save_index(&index)?;
        self.index()?;
        Ok(name)
    }

    /// Met la collection (dossier, images et fiches) à la corbeille de la bibliothèque. Rien n'est effacé.
    pub fn delete_collection(&self, name: &str) -> Result<(), String> {
        let dir = self.collection_dir(name)?;
        let trash = self.root.join(TRASH_DIR);
        std::fs::create_dir_all(&trash).map_err(|e| err("création de", &trash, e))?;
        let target = trash.join(unique_name(&trash, name, None));
        std::fs::rename(&dir, &target).map_err(|e| err("mise à la corbeille de", &dir, e))?;
        let _ = std::fs::remove_dir_all(self.root.join(THUMBS_DIR).join(name));
        self.index()?;
        Ok(())
    }

    /// Nouvel ordre des collections ; celles qui manquent à la liste gardent leur place relative, à la suite.
    pub fn reorder_collections(&self, names: &[String]) -> Result<(), String> {
        let mut index = self.index()?;
        let rank = |name: &str| names.iter().position(|n| n == name).unwrap_or(usize::MAX);
        index.collections.sort_by_key(|c| rank(&c.name));
        self.save_index(&index)
    }

    // ---------------------------------------------------------------- images

    /// Ajoute des fichiers à la fin de l'ordre de la collection, en une seule lecture et une seule écriture de
    /// l'index (un import de cent fichiers ne le réécrit pas cent fois).
    fn add_to_order(&self, collection: &str, files: &[String]) -> Result<(), String> {
        let mut index = self.index()?;
        if let Some(entry) = index.collections.iter_mut().find(|c| c.name == collection) {
            // `index()` les a peut-être déjà ajoutés (fichiers trouvés dans le dossier) : ils vont à la fin, sans doublon.
            entry.order.retain(|f| !files.contains(f));
            entry.order.extend(files.iter().cloned());
            self.apply_sort(entry);
        }
        self.save_index(&index)
    }

    /// Range `entry.order` selon son tri mémorisé (rien sans tri : l'ordre fait à la main reste tel quel).
    /// À égalité, l'ordre précédent est gardé (tri stable).
    fn apply_sort(&self, entry: &mut CollectionEntry) {
        let Some(sort) = entry.sort else { return };
        let mut items = self.items_of(entry);
        match sort {
            SortOrder::Recent => items.sort_by_key(|i| std::cmp::Reverse(i.added)),
            SortOrder::Oldest => items.sort_by_key(|i| i.added),
            SortOrder::Name => items.sort_by(|a, b| natural_cmp(&a.name, &b.name)),
        }
        entry.order = items.into_iter().map(|i| i.file).collect();
    }

    /// Trie la collection et retient ce tri (coche du menu « Trier », images ajoutées ensuite rangées à leur place).
    pub fn sort_items(&self, collection: &str, sort: SortOrder) -> Result<CollectionView, String> {
        self.collection_dir(collection)?;
        let mut index = self.index()?;
        let entry = index.collections.iter_mut().find(|c| c.name == collection).ok_or_else(|| format!("collection introuvable : {collection}"))?;
        entry.sort = Some(sort);
        self.apply_sort(entry);
        self.save_index(&index)?;
        self.collection(collection)
    }

    /// Copie un fichier du disque dans la collection (nom d'origine, dédoublonné) et écrit sa fiche.
    /// Un seul fichier (les tests) : l'application passe par `import_files`, qui ne relit l'index qu'une fois par lot.
    #[cfg(test)]
    pub fn import_file(&self, collection: &str, source: &Path) -> Result<Item, String> {
        let file = self.place_file(collection, source)?;
        self.add_to_order(collection, std::slice::from_ref(&file))?;
        Ok(self.item(collection, &file))
    }

    /// Plusieurs fichiers du disque copiés dans la collection, l'index réécrit une seule fois à la fin. Les fichiers
    /// refusés (format inconnu, illisibles) sont sautés ; l'erreur n'est rendue que si aucun n'est passé.
    pub fn import_files(&self, collection: &str, sources: &[PathBuf]) -> Result<Vec<Item>, String> {
        let mut files = Vec::new();
        let mut last_error = None;
        for source in sources {
            match self.place_file(collection, source) {
                Ok(file) => files.push(file),
                Err(error) => last_error = Some(error),
            }
        }
        if files.is_empty() {
            return match last_error {
                Some(error) => Err(error),
                None => Ok(Vec::new()),
            };
        }
        self.add_to_order(collection, &files)?;
        Ok(files.iter().map(|file| self.item(collection, file)).collect())
    }

    /// Copie le fichier dans le dossier de la collection et écrit sa fiche ; rend le nom retenu. L'index n'est
    /// pas touché : voir [`Self::add_to_order`].
    fn place_file(&self, collection: &str, source: &Path) -> Result<String, String> {
        let dir = self.collection_dir(collection)?;
        let original = source.file_name().and_then(|n| n.to_str()).ok_or_else(|| format!("nom de fichier illisible : {}", source.display()))?;
        if !is_image_name(original) {
            return Err(format!("format non pris en charge : {original}"));
        }
        let file = unique_name(&dir, &sanitize_name(original, "image"), None);
        let target = dir.join(&file);
        copy_atomic(source, &target)?;
        self.write_import_sidecar(&target, source.to_string_lossy().into_owned(), original)?;
        Ok(file)
    }

    /// Image reçue en mémoire (fichier déposé, image téléchargée). Sans extension reconnue, le format est
    /// deviné d'après le contenu ; ce qui n'est pas une image est refusé.
    pub fn import_bytes(&self, collection: &str, name: &str, bytes: &[u8], source: &str) -> Result<Item, String> {
        let dir = self.collection_dir(collection)?;
        let mut wanted = sanitize_name(name, "image");
        if !is_image_name(&wanted) {
            let ext = sniff_extension(bytes).ok_or_else(|| format!("« {name} » n'est pas une image que Lumitype sait lire"))?;
            wanted = format!("{wanted}.{ext}");
        }
        let file = unique_name(&dir, &wanted, None);
        let target = dir.join(&file);
        write_atomic(&target, bytes)?;
        let display = Path::new(name).file_name().and_then(|n| n.to_str()).filter(|n| !n.is_empty()).unwrap_or(&file).to_owned();
        self.write_import_sidecar(&target, source.to_owned(), &display)?;
        self.add_to_order(collection, std::slice::from_ref(&file))?;
        Ok(self.item(collection, &file))
    }

    /// Fiche d'une image qui vient d'être copiée : nom affiché, provenance, date, taille lue dans l'en-tête.
    fn write_import_sidecar(&self, target: &Path, source: String, display: &str) -> Result<(), String> {
        let dims = header_dimensions(target);
        let sidecar = Sidecar {
            name: display.to_owned(),
            source,
            added: now_ms(),
            width: dims.map(|d| d.0),
            height: dims.map(|d| d.1),
            ..Sidecar::default()
        };
        self.write_sidecar(target, &sidecar)
    }

    pub fn update_item(&self, id: &str, patch: ItemPatch) -> Result<Item, String> {
        let (collection, file, path) = self.resolve(id)?;
        let mut sidecar = self.read_sidecar(&path);
        if let Some(name) = patch.name {
            let name = name.trim();
            sidecar.name = if name.is_empty() { file.clone() } else { name.to_owned() };
        }
        if let Some(tags) = patch.tags {
            let mut seen = HashSet::new();
            sidecar.tags = tags.into_iter().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty() && seen.insert(t.to_lowercase())).collect();
        }
        if let Some(note) = patch.note {
            sidecar.note = note;
        }
        if let Some(favorite) = patch.favorite {
            sidecar.favorite = favorite;
        }
        self.write_sidecar(&path, &sidecar)?;
        Ok(self.item(&collection, &file))
    }

    /// Renomme le FICHIER de l'image (l'extension est gardée) et sa fiche ; le nom affiché suit. Un nom déjà pris
    /// dans la collection reçoit un suffixe « (2) ».
    pub fn rename_item(&self, id: &str, new_name: &str) -> Result<Item, String> {
        let (collection, file, path) = self.resolve(id)?;
        let ext = Path::new(&file).extension().and_then(|e| e.to_str()).map(str::to_owned);
        let stem = sanitize_name(new_name, Path::new(&file).file_stem().and_then(|s| s.to_str()).unwrap_or("image"));
        let wanted = match &ext {
            Some(ext) => format!("{stem}.{ext}"),
            None => stem.clone(),
        };
        let dir = self.collection_dir(&collection)?;
        let name = unique_name(&dir, &wanted, Some(&file));
        let mut sidecar = self.read_sidecar(&path);
        sidecar.name = Path::new(&name).file_stem().and_then(|s| s.to_str()).unwrap_or(&stem).to_owned();
        if name != file {
            let target = dir.join(&name);
            std::fs::rename(&path, &target).map_err(|e| err("renommage de", &path, e))?;
            let old_sidecar = Self::sidecar_path(&path);
            if old_sidecar.is_file() {
                let _ = std::fs::remove_file(&old_sidecar);
            }
            self.remove_thumbs(&collection, &file);
            let mut index = self.load_index();
            for entry in index.collections.iter_mut().filter(|c| c.name == collection) {
                for f in &mut entry.order {
                    if *f == file {
                        f.clone_from(&name);
                    }
                }
                if entry.cover.as_deref() == Some(file.as_str()) {
                    entry.cover = Some(name.clone());
                }
            }
            self.save_index(&index)?;
            self.write_sidecar(&target, &sidecar)?;
        } else {
            self.write_sidecar(&path, &sidecar)?;
        }
        Ok(self.item(&collection, &name))
    }

    /// Déplace l'image (et sa fiche) vers une autre collection, à la fin de celle-ci.
    pub fn move_item(&self, id: &str, to: &str) -> Result<Item, String> {
        let (collection, file, path) = self.resolve(id)?;
        if collection == to {
            return Ok(self.item(&collection, &file));
        }
        let target_dir = self.collection_dir(to)?;
        let name = unique_name(&target_dir, &file, None);
        let target = target_dir.join(&name);
        std::fs::rename(&path, &target).map_err(|e| err("déplacement de", &path, e))?;
        let sidecar = Self::sidecar_path(&path);
        if sidecar.is_file() {
            let _ = std::fs::rename(&sidecar, Self::sidecar_path(&target));
        }
        self.remove_thumbs(&collection, &file);
        let mut index = self.load_index();
        for entry in &mut index.collections {
            if entry.name == collection {
                entry.order.retain(|f| f != &file);
                if entry.cover.as_deref() == Some(file.as_str()) {
                    entry.cover = None;
                }
            } else if entry.name == to {
                entry.order.retain(|f| f != &name);
                entry.order.push(name.clone());
                self.apply_sort(entry);
            }
        }
        self.save_index(&index)?;
        self.index()?;
        Ok(self.item(to, &name))
    }

    // ---------------------------------------------------------------- une image dans plusieurs collections

    /// Copie de la même image (même contenu, octet pour octet) dans la collection `collection`, si elle y est.
    fn copy_in(&self, collection: &str, source: &Path) -> Result<Option<String>, String> {
        let dir = self.collection_dir(collection)?;
        let size = std::fs::metadata(source).map_err(|e| err("lecture de", source, e))?.len();
        let mut wanted: Option<Vec<u8>> = None;
        let entries = std::fs::read_dir(&dir).map_err(|e| err("lecture de", &dir, e))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
            if !is_image_name(&name) || entry.metadata().map(|m| m.len()).ok() != Some(size) {
                continue;
            }
            if path == source {
                return Ok(Some(name));
            }
            if wanted.is_none() {
                wanted = Some(std::fs::read(source).map_err(|e| err("lecture de", source, e))?);
            }
            if std::fs::read(&path).ok().as_deref() == wanted.as_deref() {
                return Ok(Some(name));
            }
        }
        Ok(None)
    }

    /// Collections qui contiennent cette image (elle-même comprise), dans l'ordre de la bibliothèque.
    pub fn memberships(&self, id: &str) -> Result<Vec<String>, String> {
        let (_, _, path) = self.resolve(id)?;
        let mut out = Vec::new();
        for entry in self.index()?.collections {
            if self.copy_in(&entry.name, &path)?.is_some() {
                out.push(entry.name);
            }
        }
        Ok(out)
    }

    /// Ajoute l'image à une autre collection (une copie, avec sa fiche : nom, tags, note, favori). Déjà présente :
    /// rien n'est copié, la copie existante est rendue.
    pub fn copy_item(&self, id: &str, to: &str) -> Result<Item, String> {
        let (_, file, path) = self.resolve(id)?;
        if let Some(existing) = self.copy_in(to, &path)? {
            return Ok(self.item(to, &existing));
        }
        let dir = self.collection_dir(to)?;
        let name = unique_name(&dir, &file, None);
        let target = dir.join(&name);
        copy_atomic(&path, &target)?;
        let mut sidecar = self.read_sidecar(&path);
        sidecar.added = now_ms();
        self.write_sidecar(&target, &sidecar)?;
        self.add_to_order(to, std::slice::from_ref(&name))?;
        Ok(self.item(to, &name))
    }

    /// Retire l'image de la collection `from` (sa copie va à la corbeille de la bibliothèque).
    pub fn remove_from(&self, id: &str, from: &str) -> Result<(), String> {
        let (_, _, path) = self.resolve(id)?;
        match self.copy_in(from, &path)? {
            Some(name) => self.delete_item(&format!("{from}/{name}")),
            None => Ok(()),
        }
    }

    /// Nouvel ordre des images d'une collection ; celles qui manquent à la liste restent à la suite.
    pub fn reorder_items(&self, collection: &str, files: &[String]) -> Result<(), String> {
        self.collection_dir(collection)?;
        let mut index = self.index()?;
        let entry = index.collections.iter_mut().find(|c| c.name == collection).ok_or_else(|| format!("collection introuvable : {collection}"))?;
        let rank = |file: &String| files.iter().position(|f| f == file).unwrap_or(usize::MAX);
        entry.order.sort_by_key(rank);
        // Ordre fait à la main : plus aucun tri n'est coché.
        entry.sort = None;
        self.save_index(&index)
    }

    /// Couverture de la collection : une de ses images (montrée seule, en grand), ou `None` pour revenir à la
    /// mosaïque automatique des quatre derniers ajouts. Un fichier qui n'est pas dans la collection est refusé.
    pub fn set_cover(&self, collection: &str, file: Option<&str>) -> Result<(), String> {
        self.collection_dir(collection)?;
        let mut index = self.index()?;
        let entry = index.collections.iter_mut().find(|c| c.name == collection).ok_or_else(|| format!("collection introuvable : {collection}"))?;
        if let Some(file) = file {
            if !entry.order.iter().any(|o| o == file) {
                return Err(format!("image introuvable : {collection}/{file}"));
            }
        }
        entry.cover = file.map(str::to_owned);
        self.save_index(&index)
    }

    /// Met l'image et sa fiche à la corbeille de la bibliothèque (`.corbeille/<collection>/`). Rien n'est effacé.
    pub fn delete_item(&self, id: &str) -> Result<(), String> {
        let (collection, file, path) = self.resolve(id)?;
        let trash = self.root.join(TRASH_DIR).join(&collection);
        std::fs::create_dir_all(&trash).map_err(|e| err("création de", &trash, e))?;
        let name = unique_name(&trash, &file, None);
        std::fs::rename(&path, trash.join(&name)).map_err(|e| err("mise à la corbeille de", &path, e))?;
        let sidecar = Self::sidecar_path(&path);
        if sidecar.is_file() {
            let _ = std::fs::rename(&sidecar, trash.join(format!("{name}.json")));
        }
        self.remove_thumbs(&collection, &file);
        self.index()?;
        Ok(())
    }

    // ---------------------------------------------------------------- vignettes et images

    /// Vignette WebP (côté long 480 px), tirée du cache `.apercus/` tant qu'elle est plus récente que l'image. Une
    /// vignette JPEG d'une version précédente, encore à jour, est servie telle quelle (l'interface reconnaît le
    /// format aux premiers octets) ; à la régénération suivante, la WebP la remplace et elle est supprimée.
    /// À la première fabrication, la taille en pixels et les couleurs dominantes sont notées dans la fiche.
    pub fn thumbnail(&self, id: &str) -> Result<Vec<u8>, String> {
        let (collection, file, path) = self.resolve(id)?;
        let cached = self.thumb_path(&collection, &file);
        let legacy = self.legacy_thumb_path(&collection, &file);
        let source_time = mtime_ms(&path);
        for candidate in [&cached, &legacy] {
            if candidate.is_file() && mtime_ms(candidate) >= source_time {
                if let Ok(bytes) = std::fs::read(candidate) {
                    return Ok(bytes);
                }
            }
        }
        let (rgb, w, h, size) = decode_reduced(&path, THUMB_SIDE)?;
        let encoded = encode_webp(&rgb, w, h)?;
        // Cache : son échec (disque en lecture seule) n'empêche pas l'affichage. L'ancienne JPEG ne part qu'une fois
        // la WebP écrite.
        if write_atomic(&cached, &encoded).is_ok() {
            let _ = std::fs::remove_file(&legacy);
        }
        // La fiche est relue et réécrite sous le verrou de la bibliothèque : une retouche (nom, note…) faite
        // pendant le décodage ne doit pas être écrasée par une fiche lue avant elle.
        let _guard = LIBRARY_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut sidecar = self.read_sidecar(&path);
        let (width, height) = match size {
            Some((width, height)) => (Some(width), Some(height)),
            None => (sidecar.width, sidecar.height),
        };
        let stored = Sidecar { width, height, colors: dominant_colors(&rgb), ..sidecar.clone() };
        if stored.width != sidecar.width || stored.height != sidecar.height || sidecar.colors.is_empty() {
            sidecar = stored;
            let _ = self.write_sidecar(&path, &sidecar);
        }
        Ok(encoded)
    }

    /// Image pour la visionneuse : le fichier lui-même si la webview sait l'afficher, sinon (HEIC, TIFF, DNG)
    /// une réduction JPEG de 2400 px de côté long.
    pub fn image(&self, id: &str) -> Result<Vec<u8>, String> {
        let (_, file, path) = self.resolve(id)?;
        if WEB_EXTENSIONS.contains(&extension_of(&file).as_str()) {
            return std::fs::read(&path).map_err(|e| err("lecture de", &path, e));
        }
        let (rgb, w, h, _) = decode_reduced(&path, PREVIEW_SIDE)?;
        encode_jpeg(&rgb, w, h, 90)
    }
}

// ------------------------------------------------------------------ commandes

/// Une seule opération à la fois sur une bibliothèque : `myref.json` est relu, modifié et réécrit en entier.
static LIBRARY_LOCK: Mutex<()> = Mutex::new(());

/// Travail sur le disque, hors du thread de l'interface et de la boucle asynchrone de Tauri.
async fn run<T: Send + 'static>(root: String, work: impl FnOnce(&Library) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let library = Library::open(root)?;
        let _guard = LIBRARY_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        work(&library)
    })
    .await
    .map_err(|_| "tâche interrompue".to_owned())?
}

#[tauri::command]
pub async fn myref_overview(root: String) -> Result<Overview, String> {
    run(root, Library::overview).await
}

#[tauri::command]
pub async fn myref_collection(root: String, name: String) -> Result<CollectionView, String> {
    run(root, move |lib| lib.collection(&name)).await
}

#[tauri::command]
pub async fn myref_search(root: String, query: String) -> Result<Vec<Item>, String> {
    run(root, move |lib| lib.search(&query)).await
}

#[tauri::command]
pub async fn myref_create_collection(root: String, name: String) -> Result<String, String> {
    run(root, move |lib| lib.create_collection(&name)).await
}

#[tauri::command]
pub async fn myref_rename_collection(root: String, name: String, new_name: String) -> Result<String, String> {
    run(root, move |lib| lib.rename_collection(&name, &new_name)).await
}

#[tauri::command]
pub async fn myref_delete_collection(root: String, name: String) -> Result<(), String> {
    run(root, move |lib| lib.delete_collection(&name)).await
}

#[tauri::command]
pub async fn myref_reorder_collections(root: String, names: Vec<String>) -> Result<(), String> {
    run(root, move |lib| lib.reorder_collections(&names)).await
}

/// Fichiers du disque (sélecteur « Importer », dépôt depuis l'explorateur) copiés dans la collection. Les
/// fichiers refusés (format inconnu, illisibles) sont sautés ; l'erreur n'est rendue que si aucun n'est passé.
#[tauri::command]
pub async fn myref_import_paths(root: String, collection: String, paths: Vec<PathBuf>) -> Result<Vec<Item>, String> {
    run(root, move |lib| lib.import_files(&collection, &paths)).await
}

/// Fichier déposé dont la webview ne connaît pas le chemin : contenu en corps binaire, paramètres en en-têtes
/// (encodés URL : `x-root`, `x-collection`, `x-file-name`, `x-source`).
#[tauri::command]
pub async fn myref_import_bytes(request: tauri::ipc::Request<'_>) -> Result<Item, String> {
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok()).map(crate::net::percent_decode).unwrap_or_default();
    let (root, collection, name, source) = (header("x-root"), header("x-collection"), header("x-file-name"), header("x-source"));
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("contenu du fichier manquant".into());
    };
    let bytes = bytes.clone();
    run(root, move |lib| lib.import_bytes(&collection, &name, &bytes, &source)).await
}

/// Image glissée depuis un navigateur qui n'en a fourni que l'adresse : téléchargée puis rangée.
#[tauri::command]
pub async fn myref_import_url(root: String, collection: String, url: String) -> Result<Item, String> {
    // Téléchargement hors du verrou : la bibliothèque reste utilisable pendant ce temps.
    let downloaded = tauri::async_runtime::spawn_blocking({
        let url = url.clone();
        move || crate::net::download_image(&url)
    })
    .await
    .map_err(|_| "tâche interrompue".to_owned())??;
    run(root, move |lib| lib.import_bytes(&collection, &downloaded.name.to_string_lossy(), &downloaded.bytes, &url)).await
}

#[tauri::command]
pub async fn myref_update_item(root: String, id: String, patch: ItemPatch) -> Result<Item, String> {
    run(root, move |lib| lib.update_item(&id, patch)).await
}

#[tauri::command]
pub async fn myref_move_item(root: String, id: String, to: String) -> Result<Item, String> {
    run(root, move |lib| lib.move_item(&id, &to)).await
}

/// Collections qui contiennent cette image (copies identiques comprises).
#[tauri::command]
pub async fn myref_memberships(root: String, id: String) -> Result<Vec<String>, String> {
    run(root, move |lib| lib.memberships(&id)).await
}

/// Ajoute l'image à une collection (copie, sauf si elle y est déjà).
#[tauri::command]
pub async fn myref_copy_item(root: String, id: String, to: String) -> Result<Item, String> {
    run(root, move |lib| lib.copy_item(&id, &to)).await
}

/// Retire l'image d'une collection (sa copie part à la corbeille de la bibliothèque).
#[tauri::command]
pub async fn myref_remove_from(root: String, id: String, from: String) -> Result<(), String> {
    run(root, move |lib| lib.remove_from(&id, &from)).await
}

#[tauri::command]
pub async fn myref_reorder_items(root: String, collection: String, files: Vec<String>) -> Result<(), String> {
    run(root, move |lib| lib.reorder_items(&collection, &files)).await
}

/// Trie la collection (« recent », « oldest », « name ») et retient ce tri dans `myref.json`.
#[tauri::command]
pub async fn myref_sort_items(root: String, collection: String, sort: SortOrder) -> Result<CollectionView, String> {
    run(root, move |lib| lib.sort_items(&collection, sort)).await
}

/// Ouvre LE dossier de la collection dans le gestionnaire de fichiers. L'interface ne donne que le nom : le chemin
/// est résolu ici, dans la bibliothèque (voir [`Library::collection_folder`]).
#[tauri::command]
pub async fn myref_open_collection(root: String, name: String) -> Result<(), String> {
    let dir = run(root, move |lib| lib.collection_folder(&name)).await?;
    open_folder(&dir)
}

#[tauri::command]
pub async fn myref_set_cover(root: String, collection: String, file: Option<String>) -> Result<(), String> {
    run(root, move |lib| lib.set_cover(&collection, file.as_deref())).await
}

#[tauri::command]
pub async fn myref_rename_item(root: String, id: String, name: String) -> Result<Item, String> {
    run(root, move |lib| lib.rename_item(&id, &name)).await
}

#[tauri::command]
pub async fn myref_delete_item(root: String, id: String) -> Result<(), String> {
    run(root, move |lib| lib.delete_item(&id)).await
}

/// Vignette WebP (ou ancienne JPEG encore à jour). Le décodage se fait HORS du verrou de la bibliothèque (il peut durer) : seul le cache et la
/// fiche sont écrits, chacun atomiquement.
#[tauri::command]
pub async fn myref_thumbnail(root: String, id: String) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || Library::open(root)?.thumbnail(&id).map(tauri::ipc::Response::new))
        .await
        .map_err(|_| "tâche interrompue".to_owned())?
}

#[tauri::command]
pub async fn myref_image(root: String, id: String) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || Library::open(root)?.image(&id).map(tauri::ipc::Response::new))
        .await
        .map_err(|_| "tâche interrompue".to_owned())?
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Petite image PNG valide, écrite sur le disque.
    fn png(path: &Path, w: u32, h: u32, color: [u8; 4]) {
        let image = image::RgbaImage::from_pixel(w, h, image::Rgba(color));
        let mut bytes = Vec::new();
        image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn library() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        assert!(Library::open(dir.path().join("My ref")).is_err());
        std::fs::create_dir(dir.path().join("My ref")).unwrap();
        let lib = Library::open(dir.path().join("My ref")).unwrap();
        (dir, lib)
    }

    fn index_json(lib: &Library) -> Value {
        serde_json::from_str(&std::fs::read_to_string(lib.root().join(INDEX_FILE)).unwrap()).unwrap()
    }

    #[test]
    fn create_import_and_list() {
        let (tmp, lib) = library();
        assert_eq!(lib.overview().unwrap().collections.len(), 0);
        assert_eq!(index_json(&lib)["format"], FORMAT);
        let name = lib.create_collection("Dessin").unwrap();
        assert_eq!(name, "Dessin");
        assert!(lib.root().join("Dessin").is_dir());
        let source = tmp.path().join("ville-brume.png");
        png(&source, 40, 20, [200, 10, 10, 255]);
        let item = lib.import_file("Dessin", &source).unwrap();
        assert_eq!(item.id, "Dessin/ville-brume.png");
        assert_eq!((item.width, item.height), (Some(40), Some(20)));
        assert!(lib.root().join("Dessin/ville-brume.png.json").is_file());
        // Même nom une seconde fois : dédoublonné.
        let again = lib.import_file("Dessin", &source).unwrap();
        assert_eq!(again.file, "ville-brume (2).png");
        let view = lib.collection("Dessin").unwrap();
        assert_eq!(view.items.iter().map(|i| i.file.as_str()).collect::<Vec<_>>(), ["ville-brume.png", "ville-brume (2).png"]);
        let overview = lib.overview().unwrap();
        assert_eq!(overview.collections[0].count, 2);
        assert_eq!(overview.collections[0].covers.len(), 2);
        assert_eq!(overview.recent.len(), 2);
        assert_eq!(index_json(&lib)["collections"][0]["order"][1], "ville-brume (2).png");
    }

    #[test]
    fn an_image_can_join_and_leave_other_collections() {
        let (tmp, lib) = library();
        for name in ["Graphisme", "Photos", "Dessin"] {
            lib.create_collection(name).unwrap();
        }
        let source = tmp.path().join("modem.png");
        png(&source, 30, 30, [40, 40, 40, 255]);
        let other = tmp.path().join("autre.png");
        png(&other, 30, 30, [41, 40, 40, 255]);
        let item = lib.import_file("Graphisme", &source).unwrap();
        lib.import_file("Photos", &other).unwrap();
        lib.update_item(&item.id, ItemPatch { favorite: Some(true), ..ItemPatch::default() }).unwrap();
        assert_eq!(lib.memberships(&item.id).unwrap(), ["Graphisme"]);
        let copy = lib.copy_item(&item.id, "Dessin").unwrap();
        assert_eq!(copy.id, "Dessin/modem.png");
        assert!(copy.favorite, "la fiche suit la copie");
        // Une seconde fois : rien de plus.
        assert_eq!(lib.copy_item(&item.id, "Dessin").unwrap().id, copy.id);
        assert_eq!(lib.collection("Dessin").unwrap().items.len(), 1);
        assert_eq!(lib.memberships(&item.id).unwrap(), ["Graphisme", "Dessin"]);
        assert_eq!(lib.memberships(&copy.id).unwrap(), ["Graphisme", "Dessin"], "vu depuis la copie aussi");
        lib.remove_from(&item.id, "Dessin").unwrap();
        assert_eq!(lib.memberships(&item.id).unwrap(), ["Graphisme"]);
        assert!(lib.collection("Dessin").unwrap().items.is_empty());
    }

    #[test]
    fn rename_item_renames_the_file_keeps_the_extension_and_the_order() {
        let (tmp, lib) = library();
        lib.create_collection("Dessin").unwrap();
        for n in ["a", "b"] {
            let source = tmp.path().join(format!("{n}.png"));
            png(&source, 4, 4, [0, 0, 0, 255]);
            lib.import_file("Dessin", &source).unwrap();
        }
        let first = lib.collection("Dessin").unwrap().items[0].clone();
        assert_eq!(first.name, "a", "nom affiché sans l'extension");
        lib.update_item(&first.id, ItemPatch { name: None, tags: Some(vec!["x".into()]), note: None, favorite: Some(true) }).unwrap();
        let renamed = lib.rename_item(&first.id, "Ville / brume").unwrap();
        assert_eq!(renamed.file, "Ville brume.png");
        assert_eq!(renamed.name, "Ville brume");
        assert!(renamed.favorite && renamed.tags == ["x"], "la fiche suit le fichier");
        assert!(!lib.root().join("Dessin/a.png").exists() && !lib.root().join("Dessin/a.png.json").exists());
        assert!(lib.root().join("Dessin/Ville brume.png.json").is_file());
        assert_eq!(lib.collection("Dessin").unwrap().items.iter().map(|i| i.file.as_str()).collect::<Vec<_>>(), ["Ville brume.png", "b.png"]);
        // Nom déjà pris : suffixe.
        assert_eq!(lib.rename_item("Dessin/b.png", "Ville brume").unwrap().file, "Ville brume (2).png");
    }

    #[test]
    fn import_bytes_guesses_extension_and_refuses_non_images() {
        let (tmp, lib) = library();
        lib.create_collection("Web").unwrap();
        let source = tmp.path().join("a.png");
        png(&source, 4, 4, [0, 0, 0, 255]);
        let bytes = std::fs::read(&source).unwrap();
        let item = lib.import_bytes("Web", "image-sans-extension", &bytes, "https://exemple.fr/x").unwrap();
        assert_eq!(item.file, "image-sans-extension.png");
        assert_eq!(item.source, "https://exemple.fr/x");
        assert!(lib.import_bytes("Web", "page.html", b"<html></html>", "").is_err());
        assert!(lib.import_bytes("Inconnue", "a.png", &bytes, "").is_err());
    }

    #[test]
    fn rename_collection_renames_folder_and_keeps_order() {
        let (tmp, lib) = library();
        lib.create_collection("Dessin").unwrap();
        lib.create_collection("Photos").unwrap();
        for n in ["b", "a"] {
            let source = tmp.path().join(format!("{n}.png"));
            png(&source, 4, 4, [0, 0, 0, 255]);
            lib.import_file("Dessin", &source).unwrap();
        }
        lib.thumbnail("Dessin/b.png").unwrap();
        // Nom nettoyé, et dédoublonné face à une collection existante.
        assert_eq!(lib.rename_collection("Dessin", "Photos").unwrap(), "Photos (2)");
        assert_eq!(lib.rename_collection("Photos (2)", "Croquis: villes?").unwrap(), "Croquis villes");
        assert!(lib.root().join("Croquis villes").is_dir());
        assert!(!lib.root().join("Dessin").exists());
        assert!(lib.root().join(".apercus/Croquis villes/b.png.webp").is_file());
        let view = lib.collection("Croquis villes").unwrap();
        assert_eq!(view.items.iter().map(|i| i.file.as_str()).collect::<Vec<_>>(), ["b.png", "a.png"]);
        let overview = lib.overview().unwrap();
        assert_eq!(overview.collections.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Croquis villes", "Photos"]);
        // Changement de casse seul.
        assert_eq!(lib.rename_collection("Photos", "photos").unwrap(), "photos");
        assert!(lib.collection("photos").is_ok());
    }

    #[test]
    fn sidecar_round_trip_keeps_unknown_fields() {
        let (tmp, lib) = library();
        lib.create_collection("Dessin").unwrap();
        let source = tmp.path().join("f.png");
        png(&source, 4, 4, [0, 0, 0, 255]);
        let item = lib.import_file("Dessin", &source).unwrap();
        // Champ ajouté par une version future (ou à la main).
        let sidecar = lib.root().join("Dessin/f.png.json");
        let mut json: Value = serde_json::from_str(&std::fs::read_to_string(&sidecar).unwrap()).unwrap();
        json["futur"] = serde_json::json!({ "x": 1 });
        std::fs::write(&sidecar, json.to_string()).unwrap();
        let patch = ItemPatch { tags: Some(vec![" végétal ".into(), "Végétal".into(), "lumière".into(), "".into()]), note: Some("Rim light".into()), favorite: Some(true), name: Some("Fougère".into()) };
        let updated = lib.update_item(&item.id, patch).unwrap();
        assert_eq!(updated.tags, ["végétal", "lumière"]);
        assert!(updated.favorite);
        assert_eq!(updated.name, "Fougère");
        let json: Value = serde_json::from_str(&std::fs::read_to_string(&sidecar).unwrap()).unwrap();
        assert_eq!(json["futur"]["x"], 1);
        assert_eq!(json["note"], "Rim light");
        assert_eq!(lib.overview().unwrap().favorites.len(), 1);
        assert_eq!(lib.search("lumi").unwrap().len(), 1);
        assert_eq!(lib.search("rien").unwrap().len(), 0);
        // Champ inconnu de myref.json, conservé lui aussi.
        let mut index = index_json(&lib);
        index["avenir"] = Value::Bool(true);
        index["collections"][0]["couleur"] = Value::String("rouge".into());
        std::fs::write(lib.root().join(INDEX_FILE), index.to_string()).unwrap();
        lib.create_collection("Autre").unwrap();
        let index = index_json(&lib);
        assert_eq!(index["avenir"], true);
        assert_eq!(index["collections"][0]["couleur"], "rouge");
    }

    #[test]
    fn recovers_from_manual_edits() {
        let (_tmp, lib) = library();
        lib.create_collection("Dessin").unwrap();
        // Ajouts à la main : une image (sans fiche), un dossier, un fichier qui n'est pas une image.
        png(&lib.root().join("Dessin/posee.png"), 8, 6, [9, 9, 9, 255]);
        std::fs::write(lib.root().join("Dessin/notes.txt"), "x").unwrap();
        std::fs::create_dir(lib.root().join("Voyage")).unwrap();
        png(&lib.root().join("Voyage/plage.png"), 8, 6, [9, 9, 200, 255]);
        let overview = lib.overview().unwrap();
        assert_eq!(overview.collections.iter().map(|c| (c.name.as_str(), c.count)).collect::<Vec<_>>(), [("Dessin", 1), ("Voyage", 1)]);
        let item = &lib.collection("Dessin").unwrap().items[0];
        assert_eq!(item.name, "posee", "nom affiché sans l'extension");
        assert!(item.added > 0);
        // Suppressions à la main : image et dossier disparus, retirés sans erreur.
        std::fs::remove_file(lib.root().join("Dessin/posee.png")).unwrap();
        std::fs::remove_dir_all(lib.root().join("Voyage")).unwrap();
        let overview = lib.overview().unwrap();
        assert_eq!(overview.collections.len(), 1);
        assert_eq!(overview.collections[0].count, 0);
        assert_eq!(index_json(&lib)["collections"][0]["order"].as_array().unwrap().len(), 0);
        // Index cassé : mis de côté, reconstruit d'après les dossiers.
        std::fs::write(lib.root().join(INDEX_FILE), "{ pas du json").unwrap();
        assert_eq!(lib.overview().unwrap().collections.len(), 1);
        assert!(std::fs::read_dir(lib.root()).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("myref.json.illisible-")));
        // Les dossiers de service ne sont pas des collections.
        assert!(lib.overview().unwrap().collections.iter().all(|c| !c.name.starts_with('.')));
    }

    #[test]
    fn trash_move_reorder_and_cover() {
        let (tmp, lib) = library();
        lib.create_collection("A").unwrap();
        lib.create_collection("B").unwrap();
        for n in ["un", "deux", "trois"] {
            let source = tmp.path().join(format!("{n}.png"));
            png(&source, 4, 4, [0, 0, 0, 255]);
            lib.import_file("A", &source).unwrap();
        }
        lib.reorder_items("A", &["trois.png".into(), "un.png".into()]).unwrap();
        assert_eq!(lib.collection("A").unwrap().items.iter().map(|i| i.file.as_str()).collect::<Vec<_>>(), ["trois.png", "un.png", "deux.png"]);
        lib.set_cover("A", Some("deux.png")).unwrap();
        assert_eq!(lib.overview().unwrap().collections[0].cover.as_ref().unwrap().file, "deux.png");
        let moved = lib.move_item("A/deux.png", "B").unwrap();
        assert_eq!(moved.id, "B/deux.png");
        assert!(lib.root().join("B/deux.png.json").is_file());
        assert_eq!(lib.collection("A").unwrap().cover, None);
        lib.delete_item("A/un.png").unwrap();
        assert!(lib.root().join(".corbeille/A/un.png").is_file());
        assert!(lib.root().join(".corbeille/A/un.png.json").is_file());
        assert_eq!(lib.collection("A").unwrap().items.len(), 1);
        lib.delete_collection("B").unwrap();
        assert!(lib.root().join(".corbeille/B/deux.png").is_file());
        assert_eq!(lib.overview().unwrap().collections.len(), 1);
        lib.reorder_collections(&[]).unwrap();
        // Identifiants hors de la bibliothèque : refusés.
        assert!(lib.delete_item("../x.png").is_err());
        assert!(lib.delete_item("A/../../x.png").is_err());
        assert!(lib.collection(".corbeille").is_err());
    }

    #[test]
    fn thumbnail_is_cached_and_fills_sidecar() {
        let (tmp, lib) = library();
        lib.create_collection("A").unwrap();
        let source = tmp.path().join("grand.png");
        png(&source, 1200, 600, [30, 160, 40, 255]);
        let item = lib.import_file("A", &source).unwrap();
        let thumb = lib.thumbnail(&item.id).unwrap();
        assert_eq!(sniff_extension(&thumb), Some("webp"));
        let decoded = image::load_from_memory(&thumb).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (480, 240));
        assert!(lib.root().join(".apercus/A/grand.png.webp").is_file());
        let again = lib.collection("A").unwrap().items.remove(0);
        assert_eq!(again.colors.len(), 1);
        assert_eq!(lib.thumbnail(&item.id).unwrap(), thumb);
        // Le cache effacé se reconstruit.
        std::fs::remove_dir_all(lib.root().join(".apercus")).unwrap();
        assert!(lib.thumbnail(&item.id).is_ok());
        // Visionneuse : un PNG part tel quel.
        assert_eq!(lib.image(&item.id).unwrap(), std::fs::read(&source).unwrap());
    }

    /// Pose une ancienne vignette JPEG (versions précédentes), datée juste après (à jour) ou juste avant (périmée) l'image.
    fn legacy_thumb(lib: &Library, collection: &str, file: &str, newer_than_image: bool) -> PathBuf {
        let legacy = lib.legacy_thumb_path(collection, file);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        let mut bytes = Vec::new();
        image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3])).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Jpeg).unwrap();
        std::fs::write(&legacy, &bytes).unwrap();
        let image_time = std::fs::metadata(lib.root().join(collection).join(file)).unwrap().modified().unwrap();
        let stamp = if newer_than_image { image_time + std::time::Duration::from_secs(60) } else { image_time - std::time::Duration::from_secs(60) };
        std::fs::File::options().write(true).open(&legacy).unwrap().set_modified(stamp).unwrap();
        legacy
    }

    #[test]
    fn webp_thumbnails_replace_legacy_jpeg() {
        let (tmp, lib) = library();
        lib.create_collection("A").unwrap();
        for n in ["vieux", "perime"] {
            let source = tmp.path().join(format!("{n}.png"));
            png(&source, 64, 32, [200, 100, 50, 255]);
            lib.import_file("A", &source).unwrap();
        }
        // Ancienne vignette encore à jour : servie telle quelle (JPEG), sans recalcul.
        let fresh = legacy_thumb(&lib, "A", "vieux.png", true);
        let served = lib.thumbnail("A/vieux.png").unwrap();
        assert_eq!(sniff_extension(&served), Some("jpg"));
        assert_eq!(served, std::fs::read(&fresh).unwrap());
        assert!(!lib.thumb_path("A", "vieux.png").exists());
        // Ancienne vignette périmée (image plus récente) : remplacée par une WebP, puis supprimée.
        let stale = legacy_thumb(&lib, "A", "perime.png", false);
        let rebuilt = lib.thumbnail("A/perime.png").unwrap();
        assert_eq!(sniff_extension(&rebuilt), Some("webp"));
        assert!(lib.root().join(".apercus/A/perime.png.webp").is_file());
        assert!(!stale.exists(), "l'ancienne JPEG est supprimée une fois la WebP écrite");
        assert_eq!(lib.thumbnail("A/perime.png").unwrap(), rebuilt);
        // Une image supprimée emporte ses deux vignettes.
        lib.delete_item("A/vieux.png").unwrap();
        assert!(!fresh.exists());
        // Réglages : WebP avec perte (VP8), pas sans perte (VP8L).
        assert_eq!(&rebuilt[12..16], b"VP8 ");
    }

    #[test]
    fn cover_is_one_image_or_the_automatic_mosaic() {
        let (tmp, lib) = library();
        lib.create_collection("Art").unwrap();
        for n in ["a", "b", "c", "d", "e"] {
            let source = tmp.path().join(format!("{n}.png"));
            png(&source, 4, 4, [0, 0, 0, 255]);
            lib.import_file("Art", &source).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        // Sans couverture choisie : mosaïque des quatre derniers ajouts, le plus récent d'abord.
        let summary = &lib.overview().unwrap().collections[0];
        assert!(summary.cover.is_none());
        assert_eq!(summary.covers.iter().map(|i| i.file.as_str()).collect::<Vec<_>>(), ["e.png", "d.png", "c.png", "b.png"]);
        // Une image choisie : elle seule, en grand ; enregistrée dans myref.json.
        lib.set_cover("Art", Some("a.png")).unwrap();
        let summary = &lib.overview().unwrap().collections[0];
        assert_eq!(summary.cover.as_ref().map(|i| i.file.as_str()), Some("a.png"));
        assert!(summary.covers.is_empty());
        assert_eq!(index_json(&lib)["collections"][0]["cover"], "a.png");
        assert_eq!(lib.collection("Art").unwrap().cover.as_deref(), Some("a.png"));
        // Une image hors de la collection est refusée, la couverture reste.
        assert!(lib.set_cover("Art", Some("inconnue.png")).is_err());
        assert!(lib.set_cover("Art", Some("../x.png")).is_err());
        assert_eq!(index_json(&lib)["collections"][0]["cover"], "a.png");
        // Le renommage de l'image couverture la garde comme couverture.
        lib.rename_item("Art/a.png", "alpha").unwrap();
        assert_eq!(lib.overview().unwrap().collections[0].cover.as_ref().map(|i| i.file.as_str()), Some("alpha.png"));
        // Retour à la mosaïque automatique : le champ disparaît de myref.json.
        lib.set_cover("Art", None).unwrap();
        assert!(index_json(&lib)["collections"][0].get("cover").is_none());
        assert_eq!(lib.overview().unwrap().collections[0].covers.len(), 4);
    }

    #[test]
    fn sort_is_remembered_and_kept_on_import() {
        let (tmp, lib) = library();
        lib.create_collection("A").unwrap();
        lib.create_collection("B").unwrap();
        for n in ["croquis 10", "Croquis 2", "brume"] {
            let source = tmp.path().join(format!("{n}.png"));
            png(&source, 4, 4, [0, 0, 0, 255]);
            lib.import_file("A", &source).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        let files = |lib: &Library| lib.collection("A").unwrap().items.into_iter().map(|i| i.file).collect::<Vec<_>>();
        assert_eq!(lib.collection("A").unwrap().sort, None);
        let view = lib.sort_items("A", SortOrder::Name).unwrap();
        assert_eq!(view.sort, Some(SortOrder::Name));
        assert_eq!(files(&lib), ["brume.png", "Croquis 2.png", "croquis 10.png"], "ordre naturel, sans la casse");
        assert_eq!(index_json(&lib)["collections"][0]["sort"], "name");
        // Une image ajoutée ensuite prend sa place dans le tri, au lieu d'aller à la fin.
        let source = tmp.path().join("avant.png");
        png(&source, 4, 4, [0, 0, 0, 255]);
        lib.import_file("A", &source).unwrap();
        assert_eq!(files(&lib)[0], "avant.png");
        // Plus récentes d'abord ; une image déplacée depuis une autre collection suit le tri elle aussi.
        lib.sort_items("A", SortOrder::Recent).unwrap();
        assert_eq!(files(&lib)[0], "avant.png");
        let source = tmp.path().join("venue.png");
        png(&source, 4, 4, [0, 0, 0, 255]);
        lib.import_file("B", &source).unwrap();
        lib.move_item("B/venue.png", "A").unwrap();
        assert_eq!(files(&lib)[0], "venue.png");
        lib.sort_items("A", SortOrder::Oldest).unwrap();
        assert_eq!(files(&lib)[0], "croquis 10.png");
        assert_eq!(lib.collection("A").unwrap().sort, Some(SortOrder::Oldest));
        // Ordre fait à la main (Organiser) : le tri n'est plus coché.
        lib.reorder_items("A", &["brume.png".into()]).unwrap();
        assert_eq!(lib.collection("A").unwrap().sort, None);
        assert!(index_json(&lib)["collections"][0].get("sort").is_none());
        // Le tri suit la collection renommée.
        lib.sort_items("A", SortOrder::Name).unwrap();
        lib.rename_collection("A", "Z").unwrap();
        assert_eq!(lib.collection("Z").unwrap().sort, Some(SortOrder::Name));
        assert!(serde_json::from_str::<SortOrder>("\"inconnu\"").is_err());
    }

    #[test]
    fn open_collection_resolves_only_real_collections() {
        let (_tmp, lib) = library();
        lib.create_collection("Dessin").unwrap();
        std::fs::create_dir_all(lib.root().join(".corbeille/Vieux")).unwrap();
        // Le dossier de la collection lui-même, dans la bibliothèque.
        assert_eq!(lib.collection_folder("Dessin").unwrap(), lib.root().join("Dessin"));
        // Refusés : remontée, séparateurs, dossiers de service, nom vide, collection absente.
        for bad in ["..", "../Dessin", "Dessin/..", "a/b", "a\\b", ".corbeille", ".corbeille/Vieux", ".apercus", "", "Absente"] {
            assert!(lib.collection_folder(bad).is_err(), "{bad:?} devrait être refusé");
        }
        // Un fichier n'est pas une collection.
        std::fs::write(lib.root().join("note.txt"), "x").unwrap();
        assert!(lib.collection_folder("note.txt").is_err());
        assert!(lib.collection_folder(INDEX_FILE).is_err());
    }

    #[test]
    fn natural_order() {
        use std::cmp::Ordering::*;
        assert_eq!(natural_cmp("img 2", "img 10"), Less);
        assert_eq!(natural_cmp("Img 02", "img 2"), Equal);
        assert_eq!(natural_cmp("b", "A"), Greater);
        assert_eq!(natural_cmp("a", "a1"), Less);
    }

    /// Import par lot : les fichiers refusés sont sautés, l'ordre de la collection reçoit les autres d'un coup ;
    /// l'erreur n'est rendue que si rien n'est passé.
    #[test]
    fn batch_import_skips_rejected_files() {
        let (tmp, lib) = library();
        lib.create_collection("A").unwrap();
        let a = tmp.path().join("a.png");
        let b = tmp.path().join("b.png");
        let bad = tmp.path().join("note.txt");
        png(&a, 4, 4, [0, 0, 0, 255]);
        png(&b, 4, 4, [0, 0, 0, 255]);
        std::fs::write(&bad, "x").unwrap();
        let items = lib.import_files("A", &[a, bad.clone(), b]).unwrap();
        assert_eq!(items.iter().map(|i| i.file.as_str()).collect::<Vec<_>>(), ["a.png", "b.png"]);
        let view = lib.collection("A").unwrap();
        assert_eq!(view.items.iter().map(|i| i.file.as_str()).collect::<Vec<_>>(), ["a.png", "b.png"]);
        assert!(lib.import_files("A", &[bad]).is_err());
        assert!(lib.import_files("A", &[]).unwrap().is_empty());
    }

    /// Écritures atomiques concurrentes du même fichier : chaque thread a son propre temporaire, le contenu final
    /// est entier (celui d'un des threads), et aucun temporaire ne traîne.
    #[test]
    fn concurrent_atomic_writes_keep_whole_files() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("fiche.json");
        let a = tmp.path().join("a.bin");
        let b = tmp.path().join("b.bin");
        std::fs::write(&a, vec![1u8; 300_000]).unwrap();
        std::fs::write(&b, vec![2u8; 300_000]).unwrap();
        std::thread::scope(|scope| {
            for k in 0..8u8 {
                let target = &target;
                let (a, b) = (&a, &b);
                scope.spawn(move || {
                    for _ in 0..20 {
                        let bytes = vec![k; 100_000];
                        write_atomic(target, &bytes).unwrap();
                        copy_atomic(if k % 2 == 0 { a } else { b }, target).unwrap();
                    }
                });
            }
        });
        let bytes = std::fs::read(&target).unwrap();
        assert_eq!(bytes.len(), 300_000);
        assert!(bytes.iter().all(|&v| v == bytes[0]));
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(leftovers.is_empty(), "temporaires restants : {leftovers:?}");
    }

    /// Un décodage qui panique rend sa place : les décodages suivants ne restent pas bloqués.
    #[test]
    fn decoder_gate_frees_slot_on_panic() {
        let gate = Gate { busy: Mutex::new(0), freed: Condvar::new() };
        for _ in 0..MAX_DECODERS + 1 {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gate.run(|| panic!("décodeur en échec"))));
        }
        assert_eq!(*gate.busy.lock().unwrap(), 0);
        assert_eq!(gate.run(|| 7), 7);
    }

    #[test]
    fn names_are_sanitized() {
        assert_eq!(sanitize_name("  ..Dessin/Croquis.. ", "x"), "Dessin Croquis");
        assert_eq!(sanitize_name("CON", "x"), "_CON");
        assert_eq!(sanitize_name("???", "Nouvelle collection"), "Nouvelle collection");
        assert_eq!(sniff_extension(b"\xFF\xD8\xFF\xE0"), Some("jpg"));
    }
}
