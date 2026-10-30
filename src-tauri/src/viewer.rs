//! Visionneuse Lumitype : une fenêtre seule pour regarder ses images (et celles d'un dossier), lancée par
//! `lumitype --visionneuse [fichier]`, par l'exécutable `lumitype-visionneuse` (lien vers le même programme, créé par
//! l'installateur Windows) ou par le raccourci « Lumitype Visionneuse ». Voir main.rs (`viewer_mode`) et
//! ui/src/viewer/.
//!
//! Le même programme que l'éditeur, sans l'éditeur : pas de document, pas de disque de travail, pas d'instance unique
//! (chaque image ouverte depuis l'explorateur a sa fenêtre, comme dans la plupart des visionneuses). « Modifier dans
//! Lumitype » relance l'exécutable de l'éditeur avec le fichier : l'éditeur déjà ouvert le reçoit.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::ipc::Response;

/// Images de la visionneuse : celles que Lumitype sait lire (navigateur, moteur, RAW).
const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "jfif", "webp", "avif", "gif", "bmp", "tif", "tiff", "heic", "heif"];
/// Affichées par la webview telles quelles (le fichier lui-même est envoyé).
const WEB_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "jfif", "webp", "avif", "gif", "bmp"];
/// Côté long maximal de l'image en pleine résolution décodée par le moteur (RAW, HEIC, TIFF).
const DISPLAY_SIDE: u32 = 16384;

fn failed(what: impl std::fmt::Display) -> String {
    what.to_string()
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase()
}

/// Image que la visionneuse sait montrer.
pub fn is_viewable(path: &Path) -> bool {
    let ext = extension(path);
    EXTENSIONS.contains(&ext.as_str()) || engine::io::RAW_EXTENSIONS.contains(&ext.as_str())
}

/// Fichier donné au lancement (`lumitype --visionneuse photo.jpg`), s'il y en a un.
#[tauri::command]
pub fn viewer_start() -> Option<PathBuf> {
    crate::commands::files_among(std::env::args().skip(1)).into_iter().next()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    path: PathBuf,
    name: String,
    size: u64,
    /// Millisecondes depuis 1970.
    modified: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    folder: PathBuf,
    items: Vec<Entry>,
}

/// Comparaison « naturelle » des noms : photo2 avant photo10, sans tenir compte de la casse.
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    x.next();
                }
                let mut m = String::new();
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    m.push(d);
                    y.next();
                }
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let order = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
            (Some(c), Some(d)) => {
                let order = c.to_lowercase().cmp(d.to_lowercase());
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
                x.next();
                y.next();
            }
        }
    }
}

fn list(folder: &Path) -> Result<Vec<Entry>, String> {
    let entries = std::fs::read_dir(folder).map_err(|e| failed(format!("{} : {e}", folder.display())))?;
    let mut items: Vec<Entry> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let meta = entry.metadata().ok()?;
            if !meta.is_file() || !is_viewable(&path) {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_millis() as f64);
            Some(Entry { path, name, size: meta.len(), modified })
        })
        .collect();
    items.sort_by(|a, b| natural(&a.name, &b.name));
    Ok(items)
}

/// Les images du dossier d'une image (ou d'un dossier), dans l'ordre des noms.
#[tauri::command]
pub async fn viewer_folder(path: PathBuf) -> Result<Folder, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let folder = if path.is_dir() { path } else { path.parent().map(Path::to_path_buf).ok_or("dossier introuvable")? };
        Ok(Folder { items: list(&folder)?, folder })
    })
    .await
    .map_err(failed)?
}

/// Image RGBA (alpha droit) d'un fichier que la webview ne sait pas lire, réduite à `side` au plus.
/// `preview` : un RAW peut se contenter de l'aperçu JPEG embarqué par l'appareil s'il atteint ce côté (vignette,
/// première image montrée) ; `None` : le capteur est développé en pleine définition (6024 × 4024 pour un Sony α6400,
/// quand l'aperçu embarqué n'en fait que 1616 × 1080).
fn decode(path: &Path, side: u32, preview: Option<u32>) -> Result<image::RgbaImage, String> {
    let bytes = std::fs::read(path).map_err(|e| failed(format!("{} : {e}", path.display())))?;
    let image = match preview.and_then(|min| engine::io::raw_preview(&bytes, path, min)) {
        Some(preview) => preview,
        None => {
            let (width, height, mut rgba) = engine::io::decode_premultiplied(&bytes, path).map_err(failed)?;
            for px in rgba.chunks_exact_mut(4) {
                let p = engine::pixel::unpremultiply([px[0], px[1], px[2], px[3]]);
                px.copy_from_slice(&p);
            }
            image::RgbaImage::from_raw(width, height, rgba).ok_or("image de taille inattendue")?
        }
    };
    let (w, h) = image.dimensions();
    if w.max(h) <= side {
        return Ok(image);
    }
    let scale = side as f32 / w.max(h) as f32;
    Ok(image::imageops::thumbnail(&image, ((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1)))
}

fn encode(image: &image::RgbaImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    if image.pixels().all(|p| p[3] == 255) {
        let rgb = image::DynamicImage::ImageRgba8(image.clone()).to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode(&rgb, rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8).map_err(failed)?;
    } else {
        image::codecs::png::PngEncoder::new_with_quality(&mut out, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Adaptive)
            .write_image(image.as_raw(), image.width(), image.height(), image::ExtendedColorType::Rgba8)
            .map_err(failed)?;
    }
    Ok(out)
}

use image::ImageEncoder;

/// Image à afficher : le fichier lui-même si la webview sait le lire, sinon décodé par le moteur (JPEG, ou PNG avec
/// transparence).
#[tauri::command]
pub async fn viewer_image(path: PathBuf) -> Result<Response, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if WEB_EXTENSIONS.contains(&extension(&path).as_str()) {
            return std::fs::read(&path).map(Response::new).map_err(|e| failed(format!("{} : {e}", path.display())));
        }
        Ok(Response::new(encode(&decode(&path, DISPLAY_SIDE, None)?, 92)?))
    })
    .await
    .map_err(failed)?
}

/// Formats que la bibliothèque `image` décode ici (réductions rapides, orientation EXIF appliquée).
const DECODED_HERE: &[&str] = &["png", "jpg", "jpeg", "jfif", "webp"];
/// Formats laissés tels quels à la webview, même réduits : AVIF (que le moteur ne lit pas), GIF (animé), BMP.
const PASS_THROUGH: &[&str] = &["avif", "gif", "bmp"];

/// Image réduite à `side` (côté long) pour l'affichage : la pellicule et la première image montrée, avant la pleine
/// résolution. Le fichier lui-même s'il est déjà assez petit et que la webview le lit, ou s'il est d'un format laissé
/// tel quel ; sinon JPEG (PNG s'il y a de la transparence).
fn reduced(path: &Path, side: u32, quality: u8) -> Result<Vec<u8>, String> {
    let ext = extension(path);
    if PASS_THROUGH.contains(&ext.as_str()) {
        return std::fs::read(path).map_err(failed);
    }
    let image = if DECODED_HERE.contains(&ext.as_str()) {
        let bytes = std::fs::read(path).map_err(|e| failed(format!("{} : {e}", path.display())))?;
        let decoded = image::load_from_memory(&bytes).map_err(failed)?;
        if decoded.width().max(decoded.height()) <= side {
            return Ok(bytes); // déjà à la bonne taille : rien à recompresser
        }
        let decoded = orient(decoded.thumbnail(side, side), exif_orientation(&bytes));
        decoded.to_rgba8()
    } else {
        // RAW : l'aperçu embarqué suffit s'il couvre au moins la moitié du côté demandé (l'interface charge ensuite la
        // pleine définition dès que l'affichage en demande plus).
        decode(path, side, Some((side / 2).clamp(160, 1600)))?
    };
    encode(&image, quality)
}

/// Vignette de la pellicule (côté long `side`).
#[tauri::command]
pub async fn viewer_thumb(path: PathBuf, side: u32) -> Result<Response, String> {
    tauri::async_runtime::spawn_blocking(move || reduced(&path, side.clamp(32, 512), 82).map(Response::new)).await.map_err(failed)?
}

/// Version « écran » d'une image (côté long `side`, de 512 à 4096 px) : montrée tout de suite, la pleine résolution
/// n'étant chargée qu'au zoom (voir ui/src/viewer/Viewer.svelte).
#[tauri::command]
pub async fn viewer_screen(path: PathBuf, side: u32) -> Result<Response, String> {
    tauri::async_runtime::spawn_blocking(move || reduced(&path, side.clamp(512, 4096), 90).map(Response::new)).await.map_err(failed)?
}

/// Orientation EXIF (1 à 8) d'un fichier, 1 sans EXIF.
fn exif_orientation(bytes: &[u8]) -> u32 {
    exif::Reader::new()
        .read_from_container(&mut std::io::Cursor::new(bytes))
        .ok()
        .and_then(|e| e.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0)))
        .unwrap_or(1)
}

fn orient(image: image::DynamicImage, orientation: u32) -> image::DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate270().fliph(),
        8 => image.rotate270(),
        _ => image,
    }
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    width: u32,
    height: u32,
    format: String,
    /// « 8 bits », « 16 bits »…
    depth: Option<String>,
    /// « RVB », « RVBA », « Gris »…
    color: Option<String>,
    /// Description du profil ICC incorporé.
    profile: Option<String>,
    size: u64,
    modified: f64,
    camera: Option<String>,
    lens: Option<String>,
    exposure: Option<String>,
    taken: Option<String>,
    orientation: u32,
    /// Pivoter enregistre-t-il le fichier sans perte ?
    rotatable: bool,
}

fn format_name(ext: &str) -> String {
    match ext {
        "jpg" | "jpeg" | "jfif" => "JPEG".into(),
        "tif" | "tiff" => "TIFF".into(),
        "heic" | "heif" => "HEIF".into(),
        other if engine::io::RAW_EXTENSIONS.contains(&other) => format!("RAW ({})", other.to_ascii_uppercase()),
        other => other.to_ascii_uppercase(),
    }
}

fn icc_name(icc: &[u8]) -> Option<String> {
    engine::cmyk::icc_description(icc)
}

/// Taille, format, profondeur, profil, et prise de vue (EXIF) d'une image.
#[tauri::command]
pub async fn viewer_info(path: PathBuf) -> Result<Info, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let meta = std::fs::metadata(&path).map_err(|e| failed(format!("{} : {e}", path.display())))?;
        let ext = extension(&path);
        let mut info = Info {
            size: meta.len(),
            modified: meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_millis() as f64),
            format: format_name(&ext),
            orientation: 1,
            ..Info::default()
        };
        let bytes = std::fs::read(&path).map_err(|e| failed(format!("{} : {e}", path.display())))?;
        // Dimensions, profondeur et profil : par l'en-tête (sans décoder l'image) pour les formats de `image`.
        if let Ok(reader) = image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format() {
            if let Ok(mut decoder) = reader.into_decoder() {
                use image::ImageDecoder;
                let (w, h) = decoder.dimensions();
                (info.width, info.height) = (w, h);
                let kind = decoder.color_type();
                info.depth = Some(format!("{} bits", kind.bits_per_pixel() / u16::from(kind.channel_count())));
                info.color = Some(
                    match kind.channel_count() {
                        1 => "Gris",
                        2 => "Gris + alpha",
                        3 => "RVB",
                        _ => "RVBA",
                    }
                    .into(),
                );
                info.profile = decoder.icc_profile().ok().flatten().and_then(|icc| icc_name(&icc));
            }
        }
        if info.width == 0 {
            // RAW : définition réelle de l'image développée, lue sans décoder le capteur ; HEIC… : décodée.
            if let Some((w, h)) = engine::io::raw_dimensions(&bytes, &path) {
                (info.width, info.height) = (w, h);
            } else if let Ok(image) = decode(&path, u32::MAX, None) {
                (info.width, info.height) = image.dimensions();
            }
        }
        if let Ok(exif) = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(&bytes)) {
            let text = |tag| exif.get_field(tag, exif::In::PRIMARY).map(|f| f.display_value().with_unit(&exif).to_string().trim_matches('"').trim().to_owned()).filter(|s| !s.is_empty());
            let make = text(exif::Tag::Make).unwrap_or_default();
            let model = text(exif::Tag::Model).unwrap_or_default();
            let camera = if model.starts_with(&make) { model } else { format!("{make} {model}").trim().to_owned() };
            info.camera = (!camera.is_empty()).then_some(camera);
            info.lens = text(exif::Tag::LensModel);
            let parts: Vec<String> = [text(exif::Tag::ExposureTime), text(exif::Tag::FNumber), text(exif::Tag::PhotographicSensitivity).map(|iso| format!("ISO {iso}")), text(exif::Tag::FocalLength)].into_iter().flatten().collect();
            info.exposure = (!parts.is_empty()).then(|| parts.join(" · "));
            info.taken = text(exif::Tag::DateTimeOriginal);
            info.orientation = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0)).unwrap_or(1);
        }
        if (5..=8).contains(&info.orientation) && WEB_EXTENSIONS.contains(&ext.as_str()) {
            (info.width, info.height) = (info.height, info.width);
        }
        info.rotatable = matches!(ext.as_str(), "jpg" | "jpeg" | "jfif" | "png" | "tif" | "tiff") || (ext == "webp" && is_lossless_webp(&bytes));
        Ok(info)
    })
    .await
    .map_err(failed)?
}

fn is_lossless_webp(bytes: &[u8]) -> bool {
    bytes.len() > 16 && &bytes[12..16] == b"VP8L"
}

/// Orientation EXIF après un quart de tour dans le sens des aiguilles d'une montre (`quarters` positif) ou dans
/// l'autre sens (négatif). Les orientations miroir (2, 4, 5, 7) gardent leur miroir.
fn turned(orientation: u32, quarters: i32) -> u32 {
    const PLAIN: [u32; 4] = [1, 6, 3, 8];
    const MIRROR: [u32; 4] = [2, 7, 4, 5];
    let (cycle, at) = match PLAIN.iter().position(|&o| o == orientation) {
        Some(i) => (PLAIN, i),
        None => (MIRROR, MIRROR.iter().position(|&o| o == orientation).unwrap_or(0)),
    };
    cycle[(at as i32 + quarters).rem_euclid(4) as usize]
}

/// Réécrit l'orientation EXIF d'un JPEG sur place (aucun pixel recompressé). `None` : pas de balise d'orientation à
/// modifier (l'appelant ajoute alors un bloc EXIF).
fn jpeg_patch_orientation(bytes: &mut [u8], orientation: u16) -> Option<()> {
    let mut at = 2;
    while at + 4 <= bytes.len() && bytes[at] == 0xFF {
        let marker = bytes[at + 1];
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        if marker == 0xDA {
            return None;
        }
        if marker == 0xE1 && bytes.get(at + 4..at + 10) == Some(b"Exif\0\0") {
            let tiff = at + 10;
            let little = bytes.get(tiff..tiff + 2)? == b"II";
            let read16 = |b: &[u8], i: usize| -> Option<u16> { let s = b.get(i..i + 2)?; Some(if little { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) }) };
            let read32 = |b: &[u8], i: usize| -> Option<u32> { let s = b.get(i..i + 4)?; Some(if little { u32::from_le_bytes([s[0], s[1], s[2], s[3]]) } else { u32::from_be_bytes([s[0], s[1], s[2], s[3]]) }) };
            let ifd = tiff + read32(bytes, tiff + 4)? as usize;
            let count = usize::from(read16(bytes, ifd)?);
            for i in 0..count {
                let entry = ifd + 2 + i * 12;
                if read16(bytes, entry)? == 0x0112 {
                    let value = if little { orientation.to_le_bytes() } else { orientation.to_be_bytes() };
                    bytes.get_mut(entry + 8..entry + 10)?.copy_from_slice(&value);
                    return Some(());
                }
            }
            return None;
        }
        at += 2 + length;
    }
    None
}

/// Ajoute un bloc EXIF minimal (orientation seule) juste après le début d'un JPEG qui n'en a pas.
fn jpeg_insert_orientation(bytes: &[u8], orientation: u16) -> Option<Vec<u8>> {
    if bytes.get(..2) != Some(&[0xFF, 0xD8]) {
        return None;
    }
    // Un bloc EXIF déjà présent mais sans orientation : on ne le réécrit pas (risque d'abîmer ses décalages).
    let mut at = 2;
    while at + 4 <= bytes.len() && bytes[at] == 0xFF && bytes[at + 1] != 0xDA {
        if bytes[at + 1] == 0xE1 && bytes.get(at + 4..at + 10) == Some(b"Exif\0\0") {
            return None;
        }
        at += 2 + usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
    }
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"MM\0\x2A\0\0\0\x08"); // gros-boutiste, IFD0 à l'octet 8
    tiff.extend_from_slice(&1u16.to_be_bytes()); // une entrée
    tiff.extend_from_slice(&0x0112u16.to_be_bytes());
    tiff.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&orientation.to_be_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_be_bytes()); // pas d'IFD suivant
    let mut segment = vec![0xFF, 0xE1];
    segment.extend_from_slice(&((2 + 6 + tiff.len()) as u16).to_be_bytes());
    segment.extend_from_slice(b"Exif\0\0");
    segment.extend_from_slice(&tiff);
    let mut out = Vec::with_capacity(bytes.len() + segment.len());
    out.extend_from_slice(&bytes[..2]);
    out.extend_from_slice(&segment);
    out.extend_from_slice(&bytes[2..]);
    Some(out)
}

/// Écrit dans `<fichier>.tmp` puis remplace le fichier : jamais de fichier à moitié écrit.
fn replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_extension(format!("{}.lumitype-tmp", extension(path)));
    std::fs::write(&temp, bytes).map_err(|e| failed(format!("{} : {e}", temp.display())))?;
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        failed(format!("{} : {e}", path.display()))
    })
}

/// Pivote une image d'un ou plusieurs quarts de tour, en l'enregistrant sans perte : JPEG par son orientation EXIF
/// (aucun pixel recompressé), PNG, TIFF et WebP sans perte réencodés tels quels. Les autres formats sont refusés.
pub fn rotate_file(path: &Path, quarters: i32) -> Result<(), String> {
    let quarters = quarters.rem_euclid(4);
    if quarters == 0 {
        return Ok(());
    }
    let mut bytes = std::fs::read(path).map_err(|e| failed(format!("{} : {e}", path.display())))?;
    let ext = extension(path);
    match ext.as_str() {
        "jpg" | "jpeg" | "jfif" => {
            let next = turned(exif_orientation(&bytes), quarters) as u16;
            if jpeg_patch_orientation(&mut bytes, next).is_some() {
                return replace(path, &bytes);
            }
            let out = jpeg_insert_orientation(&bytes, next).ok_or("ce JPEG ne peut pas être pivoté sans perte")?;
            replace(path, &out)
        }
        "png" | "tif" | "tiff" | "webp" => {
            if ext == "webp" && !is_lossless_webp(&bytes) {
                return Err("WebP avec perte : pivoter le recompresserait (ouvrez-le dans Lumitype pour l'exporter)".into());
            }
            let image = image::load_from_memory(&bytes).map_err(failed)?;
            let image = match quarters {
                1 => image.rotate90(),
                2 => image.rotate180(),
                _ => image.rotate270(),
            };
            let format = match ext.as_str() {
                "png" => image::ImageFormat::Png,
                "webp" => image::ImageFormat::WebP,
                _ => image::ImageFormat::Tiff,
            };
            let mut out = std::io::Cursor::new(Vec::new());
            image.write_to(&mut out, format).map_err(failed)?;
            replace(path, out.get_ref())
        }
        _ => Err("ce format ne peut pas être pivoté sans perte (ouvrez-le dans Lumitype)".into()),
    }
}

#[tauri::command]
pub async fn viewer_rotate(path: PathBuf, quarters: i32) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || rotate_file(&path, quarters)).await.map_err(failed)?
}

/// Met le fichier à la corbeille du système (récupérable).
#[tauri::command]
pub async fn viewer_trash(path: PathBuf) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || trash::delete(&path).map_err(|e| failed(format!("mise à la corbeille impossible : {e}")))).await.map_err(failed)?
}

/// Copie l'image (pixels) dans le presse-papiers du système.
#[tauri::command]
pub async fn viewer_copy(path: PathBuf) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let image = if WEB_EXTENSIONS.contains(&extension(&path).as_str()) && extension(&path) != "avif" {
            let bytes = std::fs::read(&path).map_err(failed)?;
            orient(image::load_from_memory(&bytes).map_err(failed)?, exif_orientation(&bytes)).to_rgba8()
        } else {
            decode(&path, 8192, None)?
        };
        let (width, height) = (image.width() as usize, image.height() as usize);
        arboard::Clipboard::new()
            .and_then(|mut c| c.set_image(arboard::ImageData { width, height, bytes: std::borrow::Cow::Owned(image.into_raw()) }))
            .map_err(|e| failed(format!("presse-papiers indisponible : {e}")))
    })
    .await
    .map_err(failed)?
}

/// Exécutable de l'éditeur : celui-ci, ou `lumitype` à côté quand la visionneuse est lancée par son lien.
fn editor_executable() -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(failed)?;
    let stem = current.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    if stem.ends_with("visionneuse") {
        let sibling = current.with_file_name(format!("lumitype{}", std::env::consts::EXE_SUFFIX));
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    Ok(current)
}

/// « Modifier dans Lumitype » : ouvre le fichier dans l'éditeur (celui déjà ouvert le reçoit).
#[tauri::command]
pub fn viewer_edit(path: PathBuf) -> Result<(), String> {
    std::process::Command::new(editor_executable()?).arg(path).spawn().map(|_| ()).map_err(|e| failed(format!("Lumitype ne démarre pas : {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order_puts_numbers_in_sequence() {
        let mut names = vec!["IMG10.jpg", "img2.jpg", "IMG1.jpg", "a.png"];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(names, ["a.png", "IMG1.jpg", "img2.jpg", "IMG10.jpg"]);
    }

    #[test]
    fn orientation_turns_and_keeps_mirroring() {
        assert_eq!(turned(1, 1), 6);
        assert_eq!(turned(6, 1), 3);
        assert_eq!(turned(8, 1), 1);
        assert_eq!(turned(1, -1), 8);
        assert_eq!(turned(2, 1), 7);
        assert_eq!(turned(5, 1), 2);
        assert_eq!(turned(3, 2), 1);
    }

    #[test]
    fn jpeg_rotation_is_lossless_and_png_rotation_turns_the_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let image = image::RgbImage::from_fn(4, 2, |x, _| image::Rgb([x as u8 * 60, 0, 0]));
        let jpeg = dir.path().join("photo.jpg");
        image.save(&jpeg).unwrap();
        let before = std::fs::read(&jpeg).unwrap();
        rotate_file(&jpeg, 1).unwrap();
        let after = std::fs::read(&jpeg).unwrap();
        assert_eq!(exif_orientation(&after), 6);
        assert!(after.ends_with(&before[2..]), "pixels inchangés : seul un bloc EXIF est ajouté");
        rotate_file(&jpeg, 1).unwrap();
        assert_eq!(exif_orientation(&std::fs::read(&jpeg).unwrap()), 3, "balise réécrite sur place");
        assert_eq!(std::fs::read(&jpeg).unwrap().len(), after.len());

        let png = dir.path().join("dessin.png");
        image.save(&png).unwrap();
        rotate_file(&png, 1).unwrap();
        let turned = image::open(&png).unwrap();
        assert_eq!((turned.width(), turned.height()), (2, 4));
        assert!(rotate_file(&dir.path().join("x.heic"), 1).is_err());
    }

    #[test]
    fn folder_lists_images_only_in_natural_order() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["b10.png", "b2.png", "notes.txt", ".cache.png"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let names: Vec<String> = list(dir.path()).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["b2.png", "b10.png"]);
    }
}
