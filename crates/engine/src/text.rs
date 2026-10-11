//! Calques de texte : le modèle éditable et son rendu.
//!
//! Un calque de texte conserve son [`TextData`] ; ses pixels ne sont qu'un cache recalculé
//! à chaque modification. Le texte reste donc éditable indéfiniment, et c'est la seule
//! chose que l'utilisateur perd en « pixellisant » le calque.
//!
//! Mise en forme et rendu : `cosmic-text` (mise en page, césure des lignes, polices de
//! repli, bidi) qui s'appuie sur `swash` pour dessiner les glyphes avec antialiasing.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use cosmic_text::{Align, Attrs, Buffer, Color, Fallback, Family, FontSystem, Metrics, PlatformFallback, Shaping, Style, SwashCache, Weight};
use unicode_script::{Script, UnicodeScript};
use serde::{Deserialize, Serialize};

use crate::error::EngineError;

use crate::pixel::{over, premultiply};

/// Taquets de tabulation : tous les 8 espaces depuis le début de la ligne (la touche Tab de l'éditeur insère une
/// tabulation ; le texte saute au taquet suivant, comme dans Photoshop — 4 espaces ne décalaient presque rien).
pub const TAB_WIDTH: u16 = 8;
use crate::tiles::TileStore;
use crate::transform::{transform_store, Matrix, Quality};

/// Plus grand côté d'un bloc de texte rendu, en pixels.
const MAX_BLOCK: f32 = 16_384.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Tout ce qui définit un calque de texte. Le point d'ancrage est la position du calque :
/// bord gauche, centre ou bord droit de la première ligne selon l'alignement, comme le
/// « texte de point » de Photoshop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextData {
    pub content: String,
    /// Nom de famille ; une police absente est remplacée par une police de repli.
    pub font_family: String,
    /// Corps en pixels.
    pub size: f32,
    /// Interlignage, en multiple du corps (1,2 = 120 %).
    pub line_height: f32,
    /// Approche (espacement des lettres), en millièmes de cadratin comme dans Photoshop.
    pub tracking: f32,
    pub align: TextAlign,
    pub color: [u8; 3],
    /// Graisse de la police, de 100 (Thin) à 900 (Black) ; 400 = Regular, 700 = Bold.
    #[serde(default = "regular_weight")]
    pub weight: u16,
    /// Conservé pour relire les fichiers écrits avant l'existence de `weight` (voir [`TextData::upgraded`]).
    #[serde(default, skip_serializing)]
    pub bold: bool,
    pub italic: bool,
    /// Portions du texte dont le style diffère de celui du calque (un mot dans une autre
    /// police, une autre couleur…). En cas de recouvrement, la dernière l'emporte.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<StyleRun>,
}

/// Style propre à une portion du texte. `start..end` en unités UTF-16, comme la sélection
/// d'un champ de saisie ; un champ absent reprend le réglage du calque.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleRun {
    pub start: usize,
    pub end: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
}

fn regular_weight() -> u16 {
    400
}

impl TextData {
    /// Ancien fichier : « gras » devient une graisse de 700.
    pub fn upgraded(mut self) -> Self {
        if self.bold && self.weight == 400 {
            self.weight = 700;
        }
        self.bold = false;
        self
    }
}

/// Une variante installée d'une famille : « Light Italic », « Bold »…
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct FontStyle {
    pub weight: u16,
    pub italic: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FontFamily {
    pub family: String,
    /// Variantes réellement installées, de la plus fine à la plus grasse, romain avant italique.
    pub styles: Vec<FontStyle>,
    /// Police livrée avec Lumitype (dossier des polices incluses) : présente sur toute machine, à l'identique.
    pub bundled: bool,
}

/// Familles de repli, dans l'ordre où elles sont essayées pour un caractère que la police demandée n'a pas.
/// D'abord les polices INCLUSES (présentes partout, à l'identique, et dans toutes les graisses : Inter va de 100 à
/// 900, en romain et en italique), puis des familles système courantes, puis la liste de cosmic-text pour la
/// plateforme (symboles, émojis). Une famille absente de la machine est simplement sautée.
const BUNDLED_FALLBACK: [&str; 2] = ["Inter 24pt", "IBM Plex Serif"];
const SYSTEM_FALLBACK: [&str; 7] = ["Noto Sans", "DejaVu Sans", "Liberation Sans", "Segoe UI", "Arial", "Helvetica Neue", "Helvetica"];

/// Liste de repli de Lumitype pour cosmic-text : la courte liste ci-dessus en tête des familles communes ; les
/// familles par écriture (arabe, CJK…) et les interdites restent celles de la plateforme.
struct LumitypeFallback {
    common: Vec<&'static str>,
}

impl LumitypeFallback {
    fn new() -> Self {
        let mut common: Vec<&'static str> = Vec::new();
        for name in BUNDLED_FALLBACK.iter().chain(&SYSTEM_FALLBACK).chain(PlatformFallback.common_fallback()) {
            if !common.contains(name) {
                common.push(name);
            }
        }
        Self { common }
    }
}

impl Fallback for LumitypeFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &self.common
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        PlatformFallback.forbidden_fallback()
    }

    fn script_fallback(&self, script: Script, locale: &str) -> &[&'static str] {
        PlatformFallback.script_fallback(script, locale)
    }
}

/// Polices du moteur de texte.
///
/// # Pourquoi deux bases
///
/// Pour un caractère qu'aucune police de repli connue n'a, cosmic-text essaie TOUTES les polices de sa base, une à
/// une : chaque fichier est ouvert, analysé, gardé en mémoire pour toujours, et le mot remis en forme avec — sous
/// le verrou global des polices. Avec une police sans lettres latines (« Noto Sans Cherokee ») et quelques milliers
/// de polices installées, le premier aperçu prenait des secondes.
///
/// La base de cosmic-text (`system`) ne contient donc que l'utile : les familles de repli, les polices à chasse
/// fixe, les familles réellement employées par un texte (ajoutées à la demande) et, pour un caractère que rien de
/// cela ne couvre, la famille trouvée par UNE recherche dans `all` (mémorisée : jamais deux fois pour un même
/// caractère, et sans garder les fichiers ouverts). Le parcours exhaustif de cosmic-text reste borné à cette
/// petite base. `all` recense toutes les polices (métadonnées seulement) : c'est elle que liste le sélecteur.
struct Fonts {
    all: cosmic_text::fontdb::Database,
    system: FontSystem,
    cache: SwashCache,
    /// Faces de `all` déjà recopiées dans `system`.
    pushed: HashSet<cosmic_text::fontdb::ID>,
    /// Familles (en minuscules) déjà recopiées dans `system`.
    loaded: HashSet<String>,
    /// Écritures dont les familles de repli propres ont été recopiées.
    scripts: HashSet<Script>,
    /// Caractères dont on sait qu'une police de `system` les a…
    covered: HashSet<char>,
    /// … et ceux déjà cherchés dans `all` (trouvés ou non) : on ne les cherche plus.
    searched: HashSet<char>,
}

/// Dossier des polices livrées avec Lumitype, fixé par l'application avant le premier usage des polices.
static BUNDLED_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Polices incluses : chargées AVANT celles du système, pour qu'un projet s'affiche pareil sur toute machine
/// (une police système de même nom ne prend pas le pas). Si les polices ont déjà été recensées, elles sont
/// ajoutées après coup (sans la priorité sur les homonymes du système).
pub fn set_bundled_fonts_dir(dir: PathBuf) {
    if BUNDLED_DIR.set(dir).is_err() {
        return;
    }
    if let Some(fonts) = FONTS.get() {
        let mut fonts = fonts.lock().unwrap_or_else(PoisonError::into_inner);
        let mut added = Vec::new();
        for path in bundled_files(BUNDLED_DIR.get().expect("fixé ci-dessus")) {
            added.extend(fonts.all.load_font_source(cosmic_text::fontdb::Source::File(path)));
        }
        for id in added {
            let wanted = fonts.all.face(id).is_some_and(|face| face.families.iter().any(|(name, _)| fonts.loaded.contains(&name.to_lowercase())));
            if wanted {
                fonts.push_face(id);
            }
        }
        // De nouvelles polices peuvent couvrir ce qu'on n'avait pas trouvé.
        fonts.searched.clear();
    }
}

static FONTS: OnceLock<Mutex<Fonts>> = OnceLock::new();

fn bundled_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf")) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// Familles par défaut de cosmic-text (pour « monospace », « sans-serif »…), fixées sur les deux bases.
fn set_generic_families(db: &mut cosmic_text::fontdb::Database) {
    db.set_monospace_family("Noto Sans Mono");
    db.set_sans_serif_family("Open Sans");
    db.set_serif_family("DejaVu Serif");
}

/// Les polices (incluses, puis du système) sont recensées une seule fois, au premier usage (quelques
/// centaines de millisecondes), puis partagées par tous les documents.
fn fonts() -> &'static Mutex<Fonts> {
    FONTS.get_or_init(|| {
        // Base construite à la main : les polices incluses d'abord, celles du système ensuite (cosmic-text ferait
        // l'inverse), puis les mêmes familles génériques que `FontSystem::new`.
        let mut all = cosmic_text::fontdb::Database::new();
        if let Some(dir) = BUNDLED_DIR.get() {
            for path in bundled_files(dir) {
                all.load_font_source(cosmic_text::fontdb::Source::File(path));
            }
        }
        all.load_system_fonts();
        set_generic_families(&mut all);
        let fallback = LumitypeFallback::new();
        // Base de travail de départ : familles de repli, famille « monospace » par défaut, et toutes les polices à
        // chasse fixe (cosmic-text ne les recense qu'à la construction, pour la famille générique « monospace »).
        // Recopiées dans l'ordre de `all` : à égalité, les incluses passent toujours avant le système.
        let mut initial: HashSet<String> = fallback.common.iter().map(|name| name.to_lowercase()).collect();
        initial.insert("noto sans mono".into());
        let mut work = cosmic_text::fontdb::Database::new();
        set_generic_families(&mut work);
        let mut pushed = HashSet::new();
        for face in all.faces() {
            if face.monospaced || face.families.iter().any(|(name, _)| initial.contains(&name.to_lowercase())) {
                work.push_face_info(face.clone());
                pushed.insert(face.id);
            }
        }
        let locale = sys_locale::get_locale().unwrap_or_else(|| "en-US".to_string());
        let system = FontSystem::new_with_locale_and_db_and_fallback(locale, work, fallback);
        Mutex::new(Fonts {
            all,
            system,
            cache: SwashCache::new(),
            pushed,
            loaded: initial,
            scripts: HashSet::new(),
            covered: HashSet::new(),
            searched: HashSet::new(),
        })
    })
}

/// Vrai si la face `id` de `db` a un glyphe pour `ch` (lecture de sa table cmap, sans garder le fichier ouvert).
fn face_has(db: &cosmic_text::fontdb::Database, id: cosmic_text::fontdb::ID, chars: &[char]) -> Vec<bool> {
    db.with_face_data(id, |data, index| match cosmic_text::ttf_parser::Face::parse(data, index) {
        Ok(face) => chars.iter().map(|ch| face.glyph_index(*ch).is_some()).collect(),
        Err(_) => vec![false; chars.len()],
    })
    .unwrap_or_else(|| vec![false; chars.len()])
}

/// Écriture d'un caractère, au sens de cosmic-text : commun, hérité, latin et inconnu n'ont pas de repli propre.
fn own_script(ch: char) -> Option<Script> {
    match ch.script() {
        Script::Common | Script::Inherited | Script::Latin | Script::Unknown => None,
        script => Some(script),
    }
}

impl Fonts {
    /// Recopie une face de `all` dans la base de travail (une seule fois).
    fn push_face(&mut self, id: cosmic_text::fontdb::ID) {
        if !self.pushed.insert(id) {
            return;
        }
        if let Some(face) = self.all.face(id) {
            let face = face.clone();
            self.system.db_mut().push_face_info(face);
        }
    }

    /// Recopie toutes les faces d'une famille (noms comparés sans tenir compte de la casse).
    fn ensure_family(&mut self, family: &str) {
        let key = family.to_lowercase();
        if key.is_empty() || !self.loaded.insert(key) {
            return;
        }
        let ids: Vec<_> = self.all.faces().filter(|face| face.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family))).map(|face| face.id).collect();
        for id in ids {
            self.push_face(id);
        }
    }

    /// Famille et graisse à employer pour `family` : le nom exact de la famille installée et, si cette graisse
    /// n'existe pas dans ce style, la plus proche (cosmic-text ne retient la famille demandée qu'à graisse égale ;
    /// sinon il prenait n'importe quelle police de la bonne graisse). Une famille vide ou absente de la machine
    /// prend, comme avant, la première famille de repli du système présente à cette graisse ; à défaut, Inter.
    fn resolve(&mut self, family: &str, weight: u16, italic: bool) -> (String, u16) {
        let weight = weight.clamp(100, 900);
        if family == "monospace" {
            return (family.to_owned(), weight);
        }
        let style = if italic { cosmic_text::fontdb::Style::Italic } else { cosmic_text::fontdb::Style::Normal };
        let normal = cosmic_text::fontdb::Stretch::Normal;
        let named = |face: &cosmic_text::fontdb::FaceInfo, family: &str| face.families.iter().find(|(name, _)| name.eq_ignore_ascii_case(family)).map(|(name, _)| name.clone());
        let known = !family.trim().is_empty() && self.all.faces().any(|face| named(face, family).is_some());
        if !known {
            let system_default = PlatformFallback.common_fallback().iter().find(|name| {
                self.all.faces().any(|face| face.style == style && face.stretch == normal && face.weight.0 == weight && face.families.iter().any(|(n, _)| n == *name))
            });
            let chosen = match system_default {
                Some(name) => (*name).to_owned(),
                None if self.all.faces().any(|face| named(face, BUNDLED_FALLBACK[0]).is_some()) => BUNDLED_FALLBACK[0].to_owned(),
                None => return (family.to_owned(), weight),
            };
            return self.resolve(&chosen, weight, italic);
        }
        let best = self
            .all
            .faces()
            .filter(|face| face.style == style && face.stretch == normal)
            .filter_map(|face| named(face, family).map(|name| (face.weight.0.abs_diff(weight), face.weight.0, name)))
            .min_by_key(|(distance, face_weight, _)| (*distance, *face_weight));
        let (name, weight) = match best {
            Some((_, face_weight, name)) => (name, face_weight),
            // La famille existe, mais pas dans ce style (pas d'italique) : cosmic-text passe aux polices de repli.
            None => (family.to_owned(), weight),
        };
        self.ensure_family(&name);
        (name, weight)
    }

    /// Prépare la base de travail pour `content` : familles de repli propres à ses écritures (arabe, CJK…), puis,
    /// pour tout caractère qu'aucune police de la base n'a, UNE recherche dans toutes les polices (mémorisée).
    fn prepare(&mut self, content: &str) {
        let locale = self.system.locale().to_owned();
        let mut chars: Vec<char> = content.chars().filter(|ch| !ch.is_control() && !self.covered.contains(ch) && !self.searched.contains(ch)).collect();
        chars.sort_unstable();
        chars.dedup();
        if chars.is_empty() {
            return;
        }
        for script in chars.iter().filter_map(|ch| own_script(*ch)) {
            if self.scripts.insert(script) {
                for name in PlatformFallback.script_fallback(script, &locale) {
                    self.ensure_family(name);
                }
            }
        }
        // Ce que la base de travail couvre déjà.
        let ids: Vec<_> = self.system.db().faces().map(|face| face.id).collect();
        let mut missing = chars;
        for id in ids {
            if missing.is_empty() {
                break;
            }
            let has = face_has(self.system.db(), id, &missing);
            let mut index = 0;
            missing.retain(|ch| {
                let keep = !has[index];
                index += 1;
                if !keep {
                    self.covered.insert(*ch);
                }
                keep
            });
        }
        if missing.is_empty() {
            return;
        }
        // Le reste : une recherche dans toutes les polices, dans l'ordre (incluses d'abord), la première famille
        // qui a le caractère rejoint la base de travail.
        self.searched.extend(missing.iter().copied());
        let candidates: Vec<_> = self.all.faces().filter(|face| !self.pushed.contains(&face.id)).map(|face| face.id).collect();
        for id in candidates {
            if missing.is_empty() {
                break;
            }
            let has = face_has(&self.all, id, &missing);
            if !has.contains(&true) {
                continue;
            }
            let family = self.all.face(id).and_then(|face| face.families.first()).map(|(name, _)| name.clone());
            if let Some(family) = family {
                self.ensure_family(&family);
            }
            let mut index = 0;
            missing.retain(|ch| {
                let keep = !has[index];
                index += 1;
                if !keep {
                    self.covered.insert(*ch);
                }
                keep
            });
        }
    }
}

fn is_bundled(source: &cosmic_text::fontdb::Source) -> bool {
    match (source, BUNDLED_DIR.get()) {
        (cosmic_text::fontdb::Source::File(path), Some(dir)) => path.starts_with(dir),
        _ => false,
    }
}

/// Familles de polices disponibles, triées, chacune avec ses variantes ; les incluses sont signalées.
pub fn font_families() -> Vec<FontFamily> {
    let fonts = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    let mut families: std::collections::BTreeMap<String, (String, Vec<FontStyle>, bool)> = std::collections::BTreeMap::new();
    for face in fonts.all.faces() {
        let Some((name, _)) = face.families.first() else { continue };
        let style = FontStyle { weight: face.weight.0, italic: face.style != cosmic_text::fontdb::Style::Normal };
        let entry = families.entry(name.to_lowercase()).or_insert_with(|| (name.clone(), Vec::new(), false));
        entry.1.push(style);
        entry.2 |= is_bundled(&face.source);
    }
    families
        .into_values()
        .map(|(family, mut styles, bundled)| {
            styles.sort_unstable();
            styles.dedup();
            FontFamily { family, styles, bundled }
        })
        .collect()
}

/// Fichier d'une police incluse (variante la plus proche de « normal, 400 ») : l'interface s'en sert pour
/// afficher l'échantillon du sélecteur dans la police elle-même. `None` pour une police du système (la webview y
/// accède déjà par son nom) ou inconnue.
pub fn bundled_font_file(family: &str) -> Option<PathBuf> {
    let fonts = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    let mut best: Option<(u32, PathBuf)> = None;
    for face in fonts.all.faces() {
        if !face.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family)) || !is_bundled(&face.source) {
            continue;
        }
        let cosmic_text::fontdb::Source::File(path) = &face.source else { continue };
        let distance = (i32::from(face.weight.0) - 400).unsigned_abs() + if face.style == cosmic_text::fontdb::Style::Normal { 0 } else { 1000 };
        if best.as_ref().is_none_or(|(d, _)| distance < *d) {
            best = Some((distance, path.clone()));
        }
    }
    best.map(|(_, path)| path)
}

/// Mise en page d'un texte : tampon `cosmic-text`, largeur du bloc et abscisse de l'ancre.
/// Les coordonnées de mise en page ont leur origine en haut à gauche du bloc ; le repère
/// « texte » utilisé partout ailleurs a son origine à l'ancre : `x_texte = x_page - anchor`.
struct Layout {
    buffer: Buffer,
    width: f32,
    height: f32,
    anchor: f32,
}

fn attributes(family: &str, weight: u16, italic: bool, tracking: f32) -> Attrs<'_> {
    // « monospace » désigne la police à chasse fixe du système, quelle qu'elle soit (filtre ASCII).
    let family = if family == "monospace" { Family::Monospace } else { Family::Name(family) };
    Attrs::new()
        .family(family)
        .weight(Weight(weight.clamp(100, 900)))
        .style(if italic { Style::Italic } else { Style::Normal })
        .letter_spacing(tracking / 1000.0)
}

fn layout(fonts: &mut Fonts, text: &TextData) -> Result<Layout, EngineError> {
    if !(text.size.is_finite() && text.size >= 1.0 && text.size <= 4_000.0) {
        return Err(EngineError::InvalidOperation("corps de texte hors limites".into()));
    }
    // Familles réellement employées (nom installé, graisse existante), puis polices de repli du texte : la base de
    // travail est prête AVANT la mise en forme, qui ne parcourt donc jamais toutes les polices de la machine.
    let (family, weight) = fonts.resolve(&text.font_family, text.weight, text.italic);
    let resolved: Vec<(String, u16)> = text
        .runs
        .iter()
        .map(|run| fonts.resolve(run.font_family.as_deref().unwrap_or(&text.font_family), run.weight.unwrap_or(text.weight), run.italic.unwrap_or(text.italic)))
        .collect();
    fonts.prepare(&text.content);
    let system = &mut fonts.system;
    let line_height = text.size * text.line_height.clamp(0.5, 5.0);
    let mut buffer = Buffer::new(system, Metrics::new(text.size, line_height));
    let attrs = attributes(&family, weight, text.italic, text.tracking);
    buffer.set_size(system, None, None);
    // Tabulation : taquets tous les quatre espaces (Tab dans l'éditeur insère le caractère, comme dans Photoshop).
    buffer.set_tab_width(system, TAB_WIDTH);
    if text.runs.is_empty() {
        buffer.set_text(system, &text.content, &attrs, Shaping::Advanced);
    } else {
        // Style de chaque caractère (le dernier passage qui le couvre gagne), puis regroupement
        // des caractères voisins de même style en segments.
        let chars: Vec<(usize, char)> = text.content.char_indices().collect();
        let mut owner: Vec<Option<usize>> = vec![None; chars.len()];
        let mut utf16 = 0;
        for (slot, (_, ch)) in owner.iter_mut().zip(&chars) {
            *slot = text.runs.iter().rposition(|run| utf16 >= run.start && utf16 < run.end);
            utf16 += ch.len_utf16();
        }
        let mut spans: Vec<(&str, Attrs)> = Vec::new();
        let mut from = 0;
        for i in 1..=chars.len() {
            if i < chars.len() && owner[i] == owner[from] {
                continue;
            }
            let start = chars[from].0;
            let end = chars.get(i).map_or(text.content.len(), |c| c.0);
            let run = owner[from].map(|index| &text.runs[index]);
            let (family, weight) = owner[from].map_or((family.as_str(), weight), |index| (resolved[index].0.as_str(), resolved[index].1));
            let italic = run.and_then(|r| r.italic).unwrap_or(text.italic);
            let mut span = attributes(family, weight, italic, text.tracking);
            if let Some([r, g, b]) = run.and_then(|r| r.color) {
                span = span.color(Color::rgb(r, g, b));
            }
            spans.push((&text.content[start..end], span));
            from = i;
        }
        buffer.set_rich_text(system, spans, &attrs, Shaping::Advanced, None);
    }
    buffer.shape_until_scroll(system, false);

    // Première passe : largeur du bloc. L'alignement des lignes entre elles se fait ensuite dans cette largeur.
    let width = buffer.layout_runs().map(|run| run.line_w).fold(0.0f32, f32::max).ceil();
    let height = buffer.layout_runs().map(|run| run.line_top + run.line_height).fold(line_height, f32::max).ceil();
    if width > MAX_BLOCK || height > MAX_BLOCK {
        return Err(EngineError::InvalidOperation("bloc de texte trop grand".into()));
    }
    if text.align != TextAlign::Left {
        let align = if text.align == TextAlign::Center { Align::Center } else { Align::Right };
        for line in &mut buffer.lines {
            line.set_align(Some(align));
        }
        buffer.set_size(system, Some(width.max(1.0)), None);
        buffer.shape_until_scroll(system, false);
    }
    let anchor = match text.align {
        TextAlign::Left => 0.0,
        TextAlign::Center => (width / 2.0).round(),
        TextAlign::Right => width,
    };
    Ok(Layout { buffer, width, height, anchor })
}

/// Rend le texte en pixels, en repère texte : l'origine `(0, 0)` est le point d'ancrage.
pub fn render(text: &TextData) -> Result<TileStore, EngineError> {
    if text.content.trim().is_empty() {
        return Ok(TileStore::new());
    }
    let mut guard = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    let Layout { buffer, width, height, anchor } = layout(&mut guard, text)?;
    let Fonts { system, cache, .. } = &mut *guard;

    // Marge : italiques, accents et jambages peuvent dépasser de la boîte de mise en page.
    let pad = text.size.ceil() as i32;
    let (w, h) = ((width as i32 + 2 * pad) as usize, (height as i32 + 2 * pad) as usize);
    let mut rgba = vec![0u8; w * h * 4];
    let [r, g, b] = text.color;
    buffer.draw(system, cache, Color::rgb(r, g, b), |x, y, rect_w, rect_h, color| {
        // `color.a()` est la couverture du glyphe ; les glyphes colorés (émojis) gardent leur couleur.
        let ink = premultiply([color.r(), color.g(), color.b(), color.a()]);
        for py in y + pad..y + pad + rect_h as i32 {
            for px in x + pad..x + pad + rect_w as i32 {
                if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                    continue;
                }
                let i = (py as usize * w + px as usize) * 4;
                let under = [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]];
                rgba[i..i + 4].copy_from_slice(&over(ink, under));
            }
        }
    });

    let store = TileStore::from_premultiplied(w as u32, h as u32, &rgba)
        .ok_or_else(|| EngineError::Corrupt("tampon de texte de taille inattendue".into()))?;
    Ok(store.translated(-pad - anchor as i32, -pad))
}

/// Rendu d'un texte transformé (Ctrl+T sur un calque de texte), en repère du calque.
///
/// Pour rester net, le texte est d'abord dessiné **à sa taille d'arrivée** (corps multiplié
/// par l'agrandissement moyen de la matrice), puis seul le reste de la transformation
/// (rotation, inclinaison, perspective) est appliqué par rééchantillonnage. Un texte agrandi
/// trois fois est donc composé en grand, pas étiré depuis un petit rendu.
pub fn render_transformed(text: &TextData, matrix: &Matrix, quality: Quality) -> Result<TileStore, EngineError> {
    if *matrix == Matrix::IDENTITY {
        return render(text);
    }
    let factor = matrix.mean_scale();
    if !(factor.is_finite() && factor > 1e-4) {
        return Err(EngineError::InvalidOperation("transformation de texte impossible : forme aplatie".into()));
    }
    // Le corps est borné ; au-delà, le reste de l'agrandissement passe par le rééchantillonnage.
    let factor = factor.clamp(1.0 / f64::from(text.size), 4_000.0 / f64::from(text.size));
    let scaled = TextData { size: (f64::from(text.size) * factor) as f32, ..text.clone() };
    let residual = matrix.then_after(&Matrix::scaling(1.0 / factor));
    transform_store(&render(&scaled)?, &residual, quality)
}

/// Position d'un curseur dans le texte, en unités UTF-16 comme dans un champ de saisie du navigateur.
fn cursor_at(content: &str, utf16: usize) -> cosmic_text::Cursor {
    let mut units = 0;
    let (mut line, mut line_start) = (0, 0);
    let mut byte = content.len();
    for (index, ch) in content.char_indices() {
        if units >= utf16 {
            byte = index;
            break;
        }
        units += ch.len_utf16();
        if ch == '\n' {
            line += 1;
            line_start = index + 1;
        }
    }
    cosmic_text::Cursor::new(line, byte.saturating_sub(line_start).min(content.len()))
}

/// Inverse de [`cursor_at`].
fn utf16_of(content: &str, cursor: cosmic_text::Cursor) -> usize {
    let line_start: usize = content.split('\n').take(cursor.line).map(|l| l.len() + 1).sum();
    let byte = (line_start + cursor.index).min(content.len());
    content[..byte].encode_utf16().count()
}

/// Abscisse (repère de mise en page) d'un curseur sur sa ligne.
fn caret_x(run: &cosmic_text::LayoutRun<'_>, index: usize, empty_line_x: f32) -> f32 {
    let mut x = run.glyphs.first().map_or(empty_line_x, |g| g.x);
    for glyph in run.glyphs {
        if index <= glyph.start {
            return glyph.x;
        }
        if index < glyph.end {
            // Au milieu d'une ligature : répartition proportionnelle.
            let part = (index - glyph.start) as f32 / (glyph.end - glyph.start).max(1) as f32;
            return glyph.x + glyph.w * part;
        }
        x = glyph.x + glyph.w;
    }
    x
}

/// Géométrie d'édition en repère texte : le trait du curseur (haut, bas) et les rectangles
/// de la sélection `(x, y, largeur, hauteur)`. `start` et `end` sont en unités UTF-16.
pub struct CaretGeometry {
    pub caret: ((f32, f32), (f32, f32)),
    pub highlights: Vec<(f32, f32, f32, f32)>,
}

pub fn caret_geometry(text: &TextData, start: usize, end: usize) -> Result<CaretGeometry, EngineError> {
    let mut guard = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    let Layout { buffer, width, anchor, .. } = layout(&mut guard, text)?;
    let (from, to) = (cursor_at(&text.content, start.min(end)), cursor_at(&text.content, start.max(end)));
    let focus = cursor_at(&text.content, end);
    let empty_line_x = match text.align {
        TextAlign::Left => 0.0,
        TextAlign::Center => width / 2.0,
        TextAlign::Right => width,
    };

    let line_height = text.size * text.line_height.clamp(0.5, 5.0);
    // Ligne sans mise en page (ligne vide en fin de texte, juste après Entrée) : le curseur
    // se place quand même sur SA ligne, à l'ancre, et non au début du texte.
    let top = focus.line as f32 * line_height;
    let mut caret = ((-anchor + empty_line_x, top), (-anchor + empty_line_x, top + line_height));
    let mut highlights = Vec::new();
    for run in buffer.layout_runs() {
        if run.line_i == focus.line {
            let x = caret_x(&run, focus.index, empty_line_x) - anchor;
            caret = ((x, run.line_top), (x, run.line_top + run.line_height));
        }
        if from != to {
            if let Some((x, w)) = run.highlight(from, to) {
                highlights.push((x - anchor, run.line_top, w.max(text.size * 0.25), run.line_height));
            }
        }
    }
    Ok(CaretGeometry { caret, highlights })
}

/// Position du curseur (en unités UTF-16) la plus proche du point `(x, y)` du repère texte.
pub fn hit_test(text: &TextData, x: f32, y: f32) -> Result<usize, EngineError> {
    let mut guard = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    let Layout { buffer, height, anchor, .. } = layout(&mut guard, text)?;
    // Au-dessus ou au-dessous du bloc, on vise la première ou la dernière ligne.
    let cursor = buffer.hit(x + anchor, y.clamp(0.0, (height - 1.0).max(0.0)));
    Ok(cursor.map_or_else(|| text.content.encode_utf16().count(), |c| utf16_of(&text.content, c)))
}

impl Default for TextData {
    fn default() -> Self {
        Self {
            content: String::new(),
            font_family: String::new(),
            size: 48.0,
            line_height: 1.2,
            tracking: 0.0,
            align: TextAlign::Left,
            color: [0, 0, 0],
            weight: 400,
            bold: false,
            italic: false,
            runs: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(content: &str) -> TextData {
        TextData { content: content.into(), size: 40.0, color: [255, 0, 0], ..TextData::default() }
    }

    fn extent(store: &TileStore) -> (i32, i32, i32, i32) {
        store.pixel_bounds(|px| px[3] > 0).expect("du texte a été dessiné")
    }

    #[test]
    fn text_is_rendered_near_its_anchor_in_the_requested_color() {
        let store = render(&sample("Hm")).expect("rendu");
        let (x0, y0, x1, y1) = extent(&store);
        assert!(x0 >= -4 && x0 < 12 && y0 >= 0 && y1 <= 60, "étendue inattendue : {:?}", (x0, y0, x1, y1));
        assert!(x1 - x0 > 30, "deux lettres de 40 px font plus de 30 px de large");
        let inked = store.to_premultiplied(128, 64);
        let opaque = inked.chunks_exact(4).find(|px| px[3] == 255).expect("au moins un pixel plein");
        assert_eq!(opaque, [255, 0, 0, 255]);
        assert!(inked.chunks_exact(4).any(|px| px[3] > 0 && px[3] < 255), "bords antialiasés");
    }

    /// Les polices livrées avec Lumitype (src-tauri/fonts) sont recensées, signalées « incluses », retrouvées par
    /// fichier, et bel et bien utilisées au rendu (une police de script n'a pas la même étendue qu'Inter).
    /// Ce test fixe le dossier des polices pour tout le processus de test : les autres tests en profitent.
    #[test]
    fn bundled_fonts_are_listed_and_rendered() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/fonts");
        assert!(dir.join("Berkshire_Swash").is_dir(), "dossier des polices incluses manquant");
        set_bundled_fonts_dir(dir);
        let families = font_families();
        let swash = families.iter().find(|f| f.family == "Berkshire Swash").expect("Berkshire Swash incluse");
        assert!(swash.bundled);
        let inter = families.iter().find(|f| f.family == "Inter 24pt").expect("Inter incluse");
        assert!(inter.bundled && inter.styles.iter().any(|s| s.weight == 700 && !s.italic) && inter.styles.iter().any(|s| s.italic));
        assert!(bundled_font_file("Berkshire Swash").is_some_and(|p| p.ends_with("BerkshireSwash-Regular.ttf")));
        assert!(bundled_font_file("Inter 24pt").is_some_and(|p| p.ends_with("Inter_24pt-Regular.ttf")));
        let plain = extent(&render(&TextData { font_family: "Inter 24pt".into(), ..sample("Lumitype") }).expect("rendu"));
        let script = extent(&render(&TextData { font_family: "Berkshire Swash".into(), ..sample("Lumitype") }).expect("rendu"));
        assert_ne!(plain, script, "la police incluse doit changer le rendu");
    }

    #[test]
    fn empty_text_is_empty_and_bad_sizes_are_refused() {
        assert_eq!(render(&sample("  \n ")).expect("vide").tile_count(), 0);
        assert!(render(&TextData { size: 0.0, ..sample("a") }).is_err());
        assert!(render(&TextData { size: f32::NAN, ..sample("a") }).is_err());
    }

    #[test]
    fn size_line_height_and_tracking_change_the_layout() {
        let width = |t: &TextData| {
            let (x0, _, x1, _) = extent(&render(t).expect("rendu"));
            x1 - x0
        };
        let height = |t: &TextData| {
            let (_, y0, _, y1) = extent(&render(t).expect("rendu"));
            y1 - y0
        };
        let base = sample("Hello");
        assert!(width(&TextData { size: 80.0, ..base.clone() }) > width(&base) * 3 / 2);
        assert!(width(&TextData { tracking: 300.0, ..base.clone() }) > width(&base) + 30, "approche élargie");
        let two_lines = sample("Hello\nHello");
        assert!(height(&TextData { line_height: 2.0, ..two_lines.clone() }) > height(&two_lines) + 20, "interlignage");
    }

    #[test]
    fn a_tab_indents_like_several_spaces() {
        // Tab dans un texte (touche Tab de l'éditeur) : un retrait net, plus large qu'une espace, comme dans Photoshop.
        let width = |t: &TextData| {
            let (x0, _, x1, _) = extent(&render(t).expect("rendu"));
            x1 - x0
        };
        let (space, tab) = (width(&sample("H H")), width(&sample("H\tH")));
        assert!(tab > space + 10, "tabulation ({tab}) plus large qu'une espace ({space})");
        assert!(tab <= width(&sample("H        H")) + 1, "mais pas plus qu'un taquet (8 espaces)");
    }

    #[test]
    fn alignment_moves_the_block_around_the_anchor() {
        let lines = "Un\nDeux lignes";
        let left = extent(&render(&sample(lines)).expect("gauche"));
        let center = extent(&render(&TextData { align: TextAlign::Center, ..sample(lines) }).expect("centre"));
        let right = extent(&render(&TextData { align: TextAlign::Right, ..sample(lines) }).expect("droite"));
        assert!(left.0 >= -4, "aligné à gauche : le bloc part de l'ancre");
        assert!(right.2 <= 4, "aligné à droite : le bloc finit à l'ancre");
        assert!((center.0 + center.2).abs() < 12, "centré : le bloc est à cheval sur l'ancre, {center:?}");
    }

    #[test]
    fn scaled_text_is_composed_large_instead_of_being_stretched() {
        let small = sample("Hm");
        let (x0, y0, x1, y1) = extent(&render(&small).expect("rendu"));
        let tripled = render_transformed(&small, &Matrix::scaling(3.0), Quality::Final).expect("rendu x3");
        let (tx0, ty0, tx1, ty1) = extent(&tripled);
        assert!(((tx1 - tx0) - 3 * (x1 - x0)).abs() <= 8 && ((ty1 - ty0) - 3 * (y1 - y0)).abs() <= 8, "trois fois plus grand");
        // Composé à 120 px, pas étiré : les fûts du « H » restent pleins et leurs bords étroits.
        let row = ty0 + (ty1 - ty0) / 2;
        let values: Vec<u8> = (tx0..tx1).map(|x| crate::tiles::Sampler::new(&tripled).get(x, row)[3]).collect();
        let soft = values.iter().filter(|a| **a > 0 && **a < 255).count();
        assert!(values.contains(&255) && soft <= 16, "bords trop flous pour un texte recomposé : {soft} pixels intermédiaires");
        assert!(render_transformed(&small, &Matrix([0.0; 9]), Quality::Final).is_err());
    }

    #[test]
    fn caret_and_hit_testing_agree_and_count_in_utf16() {
        let text = sample("été 😀\nab");
        let at = |offset| caret_geometry(&text, offset, offset).expect("curseur").caret.0;
        assert_eq!(at(0).0, 0.0, "début de ligne à l'ancre");
        assert!(at(1).0 > at(0).0 && at(3).0 > at(1).0, "le curseur avance avec les lettres");
        assert!(at(6).0 > at(4).0, "l'émoji occupe deux unités UTF-16 : 4 -> 6");
        assert!(at(7).1 > at(6).1 && at(7).0 == 0.0, "après le retour à la ligne : ligne suivante, à l'ancre");
        assert!(at(9).0 > at(8).0);

        // Pointer un peu à droite du milieu d'une lettre place le curseur après elle.
        for offset in [0usize, 1, 2, 3, 7, 8, 9] {
            let ((x, top), (_, bottom)) = caret_geometry(&text, offset, offset).expect("curseur").caret;
            assert_eq!(hit_test(&text, x + 0.5, (top + bottom) / 2.0).expect("pointage"), offset, "décalage {offset}");
        }
        let selection = caret_geometry(&text, 1, 8).expect("sélection");
        assert_eq!(selection.highlights.len(), 2, "une zone par ligne");
        assert_eq!(hit_test(&text, 5000.0, 5000.0).expect("pointage"), 9, "loin en bas à droite : fin du texte");

        let empty = TextData { content: String::new(), align: TextAlign::Center, ..sample("") };
        let ((x, top), (_, bottom)) = caret_geometry(&empty, 0, 0).expect("curseur").caret;
        assert!(x.abs() < 1.0 && bottom - top > 30.0, "texte vide : curseur visible à l'ancre");
    }

    #[test]
    fn a_styled_run_changes_only_its_part_of_the_text() {
        // « ab cd » : « cd » (positions 3..5) en vert et en gras, le reste en rouge.
        let run = StyleRun { start: 3, end: 5, font_family: None, weight: Some(900), italic: None, color: Some([0, 255, 0]) };
        let plain = sample("ab cd");
        let styled = TextData { runs: vec![run], ..plain.clone() };
        let store = render(&styled).expect("rendu");
        let (x0, y0, x1, y1) = extent(&store);
        let split = caret_geometry(&styled, 3, 3).expect("curseur").caret.0 .0 as i32;
        let mut sampler = crate::tiles::Sampler::new(&store);
        let (mut red_left, mut green_right, mut wrong) = (0, 0, 0);
        for y in y0..y1 {
            for x in x0..x1 {
                let px = sampler.get(x, y);
                if px[3] < 255 {
                    continue;
                }
                match (px[0] > 0, px[1] > 0, x < split) {
                    (true, false, true) => red_left += 1,
                    (false, true, false) => green_right += 1,
                    _ => wrong += 1,
                }
            }
        }
        assert!(red_left > 50 && green_right > 50 && wrong == 0, "rouge à gauche {red_left}, vert à droite {green_right}, mal placés {wrong}");
        // Un ancien fichier sans `runs` se relit, et un texte sans portion stylée n'écrit pas le champ.
        assert!(!serde_json::to_string(&plain).expect("json").contains("runs"));
    }

    #[test]
    fn installed_fonts_are_listed() {
        let families = font_families();
        let names: Vec<String> = families.iter().map(|f| f.family.to_lowercase()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted, "triées et sans doublon");
        assert!(families.iter().all(|f| !f.styles.is_empty()), "chaque famille a au moins une variante");
    }

    #[test]
    fn pressing_enter_puts_the_caret_on_the_next_line() {
        let text = sample("abc\n");
        let ((x, top), (_, bottom)) = caret_geometry(&text, 4, 4).expect("curseur").caret;
        assert!(x.abs() < 1.0, "à l'ancre");
        assert!(top >= 40.0 && bottom - top > 30.0, "sur la deuxième ligne, pas au début du texte : y = {top}");
        let third = caret_geometry(&sample("abc\n\n"), 5, 5).expect("curseur").caret.0;
        assert!(third.1 > top + 30.0, "deux retours : troisième ligne");
    }

    #[test]
    fn weight_changes_the_rendering_and_old_bold_files_are_upgraded() {
        let old: TextData = serde_json::from_str(
            r#"{"content":"a","fontFamily":"","size":40,"lineHeight":1.2,"tracking":0,"align":"left","color":[0,0,0],"bold":true,"italic":false}"#,
        )
        .expect("ancien format");
        assert_eq!(old.upgraded().weight, 700);
        let json = serde_json::to_string(&sample("a")).expect("json");
        assert!(json.contains("\"weight\":400") && !json.contains("bold"), "le champ hérité n'est plus écrit : {json}");
    }

    /// Police d'essai SANS lettres latines (chiffres et espace seulement, tirée d'Inter et renommée « Lumitype
    /// Essai Chiffres ») : le cas d'une « Noto Sans Cherokee » choisie pour un texte latin. Ajoutée à la base de
    /// toutes les polices, comme une police du système.
    fn load_digits_only_font() {
        let mut fonts = fonts().lock().unwrap_or_else(PoisonError::into_inner);
        if !fonts.all.faces().any(|f| f.families.iter().any(|(n, _)| n == "Lumitype Essai Chiffres")) {
            fonts.all.load_font_data(include_bytes!("../tests/fixtures/sans-latin.ttf").to_vec());
            fonts.searched.clear();
        }
    }

    /// Pour chaque glyphe mis en page : le texte qu'il dessine et la famille de la police employée.
    fn families_used(text: &TextData) -> Vec<(String, String)> {
        let mut guard = fonts().lock().unwrap_or_else(PoisonError::into_inner);
        let Layout { buffer, .. } = layout(&mut guard, text).expect("mise en page");
        let mut out = Vec::new();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let family = guard.system.db().face(glyph.font_id).and_then(|f| f.families.first()).map(|(n, _)| n.clone()).unwrap_or_default();
                out.push((run.text[glyph.start..glyph.end].to_owned(), family));
            }
        }
        out
    }

    #[test]
    fn a_family_without_latin_letters_falls_back_to_the_short_list() {
        set_bundled_fonts_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/fonts"));
        load_digits_only_font();
        let short: Vec<&str> = LumitypeFallback::new().common;
        for weight in [400, 500, 300] {
            let text = TextData { content: "Lumitype 2026".into(), font_family: "Lumitype Essai Chiffres".into(), weight, ..sample("") };
            let used = families_used(&text);
            assert!(!used.is_empty());
            for (part, family) in &used {
                if part.chars().all(|c| c.is_ascii_digit()) {
                    assert_eq!(family, "Lumitype Essai Chiffres", "les chiffres viennent de la police demandée (graisse {weight})");
                } else if part.chars().all(char::is_alphabetic) {
                    // Inter (incluse, toutes graisses) : la première de la courte liste, quelle que soit la graisse.
                    assert_eq!(family, "Inter 24pt", "« {part} » en graisse {weight}");
                    assert!(short.contains(&family.as_str()));
                }
            }
            let (x0, _, x1, _) = extent(&render(&text).expect("rendu"));
            assert!(x1 - x0 > 150, "le texte est bien dessiné ({})", x1 - x0);
        }
        // La base de travail de cosmic-text reste petite : jamais toutes les polices de la machine.
        let fonts = fonts().lock().unwrap_or_else(PoisonError::into_inner);
        assert!(fonts.system.db().len() < fonts.all.len(), "{} polices de travail pour {} installées", fonts.system.db().len(), fonts.all.len());
    }

    #[test]
    fn a_missing_weight_uses_the_nearest_one_of_the_same_family() {
        set_bundled_fonts_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/fonts"));
        // Berkshire Swash n'existe qu'en 400 : en 700, c'est elle quand même (et non une police quelconque en 700).
        let regular = TextData { font_family: "Berkshire Swash".into(), ..sample("Lumitype") };
        let bold = TextData { weight: 700, ..regular.clone() };
        assert!(families_used(&bold).iter().all(|(_, family)| family == "Berkshire Swash"));
        assert_eq!(extent(&render(&bold).expect("rendu")), extent(&render(&regular).expect("rendu")));
    }

    /// Mesure : `cargo test -p engine --release --lib text::tests::bench_fallback -- --ignored --nocapture`.
    /// Premier rendu d'un texte latin dans une police sans lettres latines, puis d'un caractère qu'aucune police
    /// n'a (cherokee), première fois et suivantes.
    #[test]
    #[ignore]
    fn bench_fallback() {
        set_bundled_fonts_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/fonts"));
        load_digits_only_font();
        for (label, content) in [("latin", "Lumitype éditeur d'images 2026"), ("latin (déjà vu)", "Lumitype éditeur d'images 2026"), ("cherokee", "Ꮳ Ꮑ Ꮫ 12"), ("cherokee (déjà vu)", "Ꮳ Ꮑ Ꮫ 12")] {
            let text = TextData { content: content.into(), font_family: "Lumitype Essai Chiffres".into(), weight: 500, ..sample("") };
            let start = std::time::Instant::now();
            render(&text).expect("rendu");
            eprintln!("{label:>20} : {:?}", start.elapsed());
        }
        let fonts = fonts().lock().unwrap_or_else(PoisonError::into_inner);
        eprintln!("polices de travail : {} sur {} installées", fonts.system.db().len(), fonts.all.len());
    }
}
