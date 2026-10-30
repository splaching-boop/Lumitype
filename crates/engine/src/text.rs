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
    /// Justifié : les lignes d'un texte de paragraphe remplissent la largeur du cadre (la dernière ligne de chaque
    /// paragraphe reste alignée à gauche, comme dans Photoshop). Un texte de point justifié s'aligne à gauche.
    Justify,
}

/// Cadre d'un texte de paragraphe (« texte en bloc »), en pixels du repère texte : le texte revient à la ligne dans
/// sa largeur ; ce qui dépasse sa hauteur n'est pas dessiné (voir [`overflows`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextFrame {
    pub width: f32,
    pub height: f32,
}

/// Casse d'un texte : telle que tapée, tout en capitales, ou en petites capitales (fonction `smcp` de la police).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextCaps {
    #[default]
    Normal,
    AllCaps,
    SmallCaps,
}

impl TextCaps {
    fn is_normal(&self) -> bool {
        *self == Self::Normal
    }
}

/// Tout ce qui définit un calque de texte. Le point d'ancrage est la position du calque :
/// bord gauche, centre ou bord droit de la première ligne selon l'alignement, comme le
/// « texte de point » de Photoshop. Un texte de paragraphe (`frame`) a pour origine le coin
/// haut gauche de son cadre.
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
    /// Texte de paragraphe : son cadre. Absent (fichiers écrits avant son existence compris) : texte de point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<TextFrame>,
    /// Texte dynamique (texte de paragraphe seulement) : chaque ligne est mise à la taille qui remplit la largeur du
    /// cadre, les mots répartis pour en remplir la hauteur. Absent : texte ordinaire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<DynamicText>,
    /// Casse du calque (les portions peuvent avoir la leur).
    #[serde(default, skip_serializing_if = "TextCaps::is_normal")]
    pub caps: TextCaps,
    /// Style de caractère appliqué au calque entier (identifiant dans `Document::text_styles`). Ses réglages sont
    /// recopiés dans le texte : le rendu n'en dépend pas, l'identifiant sert à le retrouver et à le mettre à jour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<u32>,
}

/// Réglages d'un texte dynamique (voir [`TextData::dynamic`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DynamicText {
    /// Nombre de lignes : 0 = automatique (celui qui remplit le mieux le cadre).
    #[serde(default)]
    pub lines: u16,
    /// Les retours à la ligne tapés font les lignes (`lines` est alors ignoré).
    #[serde(default)]
    pub manual: bool,
    /// Espace entre deux lignes, en part de la hauteur moyenne d'une ligne (0,12 = 12 %).
    #[serde(default = "dynamic_gap")]
    pub gap: f32,
}

fn dynamic_gap() -> f32 {
    0.12
}

impl Default for DynamicText {
    fn default() -> Self {
        Self { lines: 0, manual: false, gap: dynamic_gap() }
    }
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
    /// Corps de la portion, en pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    /// Approche de la portion, en millièmes de cadratin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<TextCaps>,
    /// Style de caractère appliqué à la portion (voir [`TextData::style`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<u32>,
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
/// Texte de paragraphe : le bloc est le cadre, l'ancre son bord gauche (`anchor = 0`).
struct Layout {
    buffer: Buffer,
    width: f32,
    height: f32,
    anchor: f32,
    /// Bas de la zone dessinée : la hauteur du cadre d'un texte de paragraphe ; l'infini pour un texte de point.
    clip: f32,
}

/// Une ligne mise en page est dessinée si elle tient EN ENTIER dans la hauteur du cadre (comme dans Photoshop : une
/// ligne à moitié visible est masquée, pas coupée). Un demi-pixel de tolérance absorbe les arrondis.
fn line_shown(run: &cosmic_text::LayoutRun<'_>, clip: f32) -> bool {
    run.line_top + run.line_height <= clip + 0.5
}

/// Alignement de cosmic-text pour un réglage du calque ; `None` : alignement naturel (gauche, ou droite en arabe…).
fn cosmic_align(align: TextAlign, paragraph: bool) -> Option<Align> {
    match align {
        TextAlign::Left => None,
        TextAlign::Center => Some(Align::Center),
        TextAlign::Right => Some(Align::Right),
        // Justifier n'a de sens que dans une largeur imposée : un texte de point justifié reste aligné à gauche.
        TextAlign::Justify if paragraph => Some(Align::Justified),
        TextAlign::Justify => None,
    }
}

/// Abscisse (repère de mise en page) d'une ligne vide selon l'alignement : là où le curseur s'y place.
fn empty_line_x(align: TextAlign, width: f32) -> f32 {
    match align {
        TextAlign::Left | TextAlign::Justify => 0.0,
        TextAlign::Center => width / 2.0,
        TextAlign::Right => width,
    }
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

/// Petites capitales : la fonction OpenType `smcp` de la police (sans effet si elle ne l'a pas).
fn with_caps(attrs: Attrs<'_>, caps: TextCaps) -> Attrs<'_> {
    if caps != TextCaps::SmallCaps {
        return attrs;
    }
    let mut features = cosmic_text::FontFeatures::new();
    features.enable(cosmic_text::FeatureTag::SMALL_CAPS);
    attrs.font_features(features)
}

/// Texte tel qu'affiché : les caractères « tout en capitales » passent en majuscule quand leur majuscule tient dans
/// le même nombre d'octets (les positions du curseur et des portions restent valables), sinon ils restent tels quels.
fn shown_content(text: &TextData) -> std::borrow::Cow<'_, str> {
    let any = text.caps == TextCaps::AllCaps || text.runs.iter().any(|r| r.caps == Some(TextCaps::AllCaps));
    if !any {
        return std::borrow::Cow::Borrowed(&text.content);
    }
    let mut out = String::with_capacity(text.content.len());
    let mut utf16 = 0;
    for ch in text.content.chars() {
        let caps = text.runs.iter().rev().find(|run| utf16 >= run.start && utf16 < run.end).and_then(|run| run.caps).unwrap_or(text.caps);
        let mut upper = ch.to_uppercase();
        match (caps, upper.next(), upper.next()) {
            (TextCaps::AllCaps, Some(u), None) if u.len_utf8() == ch.len_utf8() => out.push(u),
            _ => out.push(ch),
        }
        utf16 += ch.len_utf16();
    }
    std::borrow::Cow::Owned(out)
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
    fonts.prepare(&shown_content(text));
    let system = &mut fonts.system;
    let line_height = text.size * text.line_height.clamp(0.5, 5.0);
    let mut buffer = Buffer::new(system, Metrics::new(text.size, line_height));
    let attrs = with_caps(attributes(&family, weight, text.italic, text.tracking), text.caps);
    let content = shown_content(text);
    if let Some(frame) = text.frame {
        let valid = |v: f32| v.is_finite() && (1.0..=MAX_BLOCK).contains(&v);
        if !(valid(frame.width) && valid(frame.height)) {
            return Err(EngineError::InvalidOperation("cadre de texte hors limites".into()));
        }
    }
    // Texte de paragraphe : retour à la ligne automatique dans la largeur du cadre, par mots, et à l'intérieur d'un mot
    // seulement s'il est plus long qu'une ligne. Texte de point : aucune largeur, aucun retour automatique.
    buffer.set_wrap(system, cosmic_text::Wrap::WordOrGlyph);
    buffer.set_size(system, text.frame.map(|frame| frame.width), None);
    // Tabulation : taquets tous les quatre espaces (Tab dans l'éditeur insère le caractère, comme dans Photoshop).
    buffer.set_tab_width(system, TAB_WIDTH);
    if text.runs.is_empty() {
        buffer.set_text(system, &content, &attrs, Shaping::Advanced);
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
            let tracking = run.and_then(|r| r.tracking).unwrap_or(text.tracking);
            let mut span = with_caps(attributes(family, weight, italic, tracking), run.and_then(|r| r.caps).unwrap_or(text.caps));
            if let Some([r, g, b]) = run.and_then(|r| r.color) {
                span = span.color(Color::rgb(r, g, b));
            }
            if let Some(size) = run.and_then(|r| r.size).filter(|v| v.is_finite() && (1.0..=4_000.0).contains(v)) {
                span = span.metrics(Metrics::new(size, size * text.line_height.clamp(0.5, 5.0)));
            }
            spans.push((&content[start..end], span));
            from = i;
        }
        buffer.set_rich_text(system, spans, &attrs, Shaping::Advanced, None);
    }

    if let Some(frame) = text.frame {
        // Texte de paragraphe : les lignes s'alignent dans la largeur du cadre, dès la première mise en page.
        let align = cosmic_align(text.align, true);
        for line in &mut buffer.lines {
            line.set_align(align);
        }
        buffer.shape_until_scroll(system, false);
        return Ok(Layout { buffer, width: frame.width, height: frame.height, anchor: 0.0, clip: frame.height });
    }
    buffer.shape_until_scroll(system, false);

    // Première passe : largeur du bloc. L'alignement des lignes entre elles se fait ensuite dans cette largeur.
    let width = buffer.layout_runs().map(|run| run.line_w).fold(0.0f32, f32::max).ceil();
    let height = buffer.layout_runs().map(|run| run.line_top + run.line_height).fold(line_height, f32::max).ceil();
    if width > MAX_BLOCK || height > MAX_BLOCK {
        return Err(EngineError::InvalidOperation("bloc de texte trop grand".into()));
    }
    if let Some(align) = cosmic_align(text.align, false) {
        for line in &mut buffer.lines {
            line.set_align(Some(align));
        }
        buffer.set_size(system, Some(width.max(1.0)), None);
        buffer.shape_until_scroll(system, false);
    }
    let anchor = match text.align {
        TextAlign::Left | TextAlign::Justify => 0.0,
        TextAlign::Center => (width / 2.0).round(),
        TextAlign::Right => width,
    };
    Ok(Layout { buffer, width, height, anchor, clip: f32::INFINITY })
}

/// Vrai si une partie d'un texte de paragraphe ne tient pas dans la hauteur de son cadre (elle n'est pas dessinée,
/// comme dans Photoshop, et l'interface le signale). Toujours faux pour un texte de point.
pub fn overflows(text: &TextData) -> Result<bool, EngineError> {
    if text.frame.is_none() || text.content.is_empty() || dynamic_of(text).is_some() {
        return Ok(false);
    }
    let mut guard = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    let Layout { buffer, clip, .. } = layout(&mut guard, text)?;
    Ok(overflow_of(&buffer, clip))
}

/// Une ligne masquée qui contient quelque chose (des lignes vides sous le cadre ne cachent rien).
fn overflow_of(buffer: &Buffer, clip: f32) -> bool {
    buffer.layout_runs().any(|run| !line_shown(&run, clip) && !run.glyphs.is_empty())
}

/// Rend le texte en pixels, en repère texte : l'origine `(0, 0)` est le point d'ancrage.
pub fn render(text: &TextData) -> Result<TileStore, EngineError> {
    if text.content.trim().is_empty() {
        return Ok(TileStore::new());
    }
    let mut guard = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((dynamic, frame)) = dynamic_of(text) {
        return render_dynamic(&mut guard, text, dynamic, frame);
    }
    let Layout { buffer, width, height, anchor, clip } = layout(&mut guard, text)?;
    let Fonts { system, cache, .. } = &mut *guard;

    // Marge : italiques, accents et jambages peuvent dépasser de la boîte de mise en page.
    let pad = text.size.ceil() as i32;
    let (w, h) = ((width.ceil() as i32 + 2 * pad) as usize, (height.ceil() as i32 + 2 * pad) as usize);
    let mut rgba = vec![0u8; w * h * 4];
    let [r, g, b] = text.color;
    // Comme `Buffer::draw`, mais seulement les lignes qui tiennent dans le cadre d'un texte de paragraphe (toutes
    // pour un texte de point). Les lignes viennent de haut en bas : la première qui dépasse arrête le dessin.
    for run in buffer.layout_runs() {
        if !line_shown(&run, clip) {
            break;
        }
        for glyph in run.glyphs {
            let physical = glyph.physical((0.0, 0.0), 1.0);
            let base = glyph.color_opt.unwrap_or(Color::rgb(r, g, b));
            cache.with_pixels(system, physical.cache_key, base, |x, y, color| {
                let (px, py) = (physical.x + x + pad, run.line_y as i32 + physical.y + y + pad);
                if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                    return;
                }
                // `color.a()` est la couverture du glyphe ; les glyphes colorés (émojis) gardent leur couleur.
                let ink = premultiply([color.r(), color.g(), color.b(), color.a()]);
                let i = (py as usize * w + px as usize) * 4;
                let under = [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]];
                rgba[i..i + 4].copy_from_slice(&over(ink, under));
            });
        }
    }

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
    // Le cadre d'un texte de paragraphe grandit avec le corps : les lignes reviennent à la ligne aux mêmes mots.
    let frame = text.frame.map(|frame| TextFrame { width: (f64::from(frame.width) * factor) as f32, height: (f64::from(frame.height) * factor) as f32 });
    let scaled = TextData { size: (f64::from(text.size) * factor) as f32, frame, ..text.clone() };
    let residual = matrix.then_after(&Matrix::scaling(1.0 / factor));
    transform_store(&render(&scaled)?, &residual, quality)
}

// ---- Texte dynamique ---------------------------------------------------------------------------------------------
//
// Les mots sont répartis en lignes (nombre choisi, automatique, ou retours tapés), puis chaque ligne est composée
// comme un petit texte de point, au corps qui remplit exactement la largeur du cadre (bords de l'encre, pas de la
// chasse). Les lignes s'empilent avec l'interligne demandé ; si le tout dépasse la hauteur du cadre, tout est réduit
// d'un même facteur et centré. Le curseur et le clic se placent dans la ligne visée, à son échelle.

/// Corps de mesure : les lignes sont mesurées à ce corps, puis mises à l'échelle.
const DYNAMIC_REFERENCE: f32 = 100.0;
/// Nombre de lignes essayé au plus en mode automatique.
const DYNAMIC_MAX_LINES: usize = 24;

/// Texte dynamique effectif : un cadre et des réglages.
fn dynamic_of(text: &TextData) -> Option<(DynamicText, TextFrame)> {
    Some((text.dynamic?, text.frame?))
}

/// Une ligne d'un texte dynamique : sa portion du contenu (octets), le texte de point qui la dessine et la position
/// de son ancre dans le repère du texte, avec son étendue verticale.
struct DynamicLine {
    start: usize,
    end: usize,
    text: TextData,
    offset: (f32, f32),
    top: f32,
    bottom: f32,
}

/// Mots du contenu : plages d'octets sans espace.
fn words_of(content: &str) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut from: Option<usize> = None;
    for (i, ch) in content.char_indices() {
        if ch.is_whitespace() {
            if let Some(start) = from.take() {
                words.push((start, i));
            }
        } else if from.is_none() {
            from = Some(i);
        }
    }
    if let Some(start) = from {
        words.push((start, content.len()));
    }
    words
}

/// Portion `start..end` (octets) du texte, en texte de point au corps `size` : retours remplacés par des espaces
/// (même longueur : les positions restent les mêmes), portions stylées recalées.
fn line_text(text: &TextData, start: usize, end: usize, size: f32) -> TextData {
    let content: String = text.content[start..end].chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    let (u0, u1) = (text.content[..start].encode_utf16().count(), text.content[..end].encode_utf16().count());
    let runs = text
        .runs
        .iter()
        .filter(|r| r.end > u0 && r.start < u1)
        .map(|r| StyleRun { start: r.start.max(u0) - u0, end: r.end.min(u1) - u0, size: r.size.map(|v| v * size / text.size.max(1.0)), ..r.clone() })
        .collect();
    TextData { content, size, align: TextAlign::Left, frame: None, dynamic: None, runs, line_height: 1.0, ..text.clone() }
}

/// Étendue de l'encre d'un texte de point (repère texte), ou rien s'il n'a aucun pixel.
fn ink_bounds(fonts: &mut Fonts, text: &TextData) -> Result<Option<(f32, f32, f32, f32)>, EngineError> {
    let Layout { buffer, .. } = layout(fonts, text)?;
    let Fonts { system, cache, .. } = fonts;
    let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let physical = glyph.physical((0.0, 0.0), 1.0);
            cache.with_pixels(system, physical.cache_key, Color::rgb(0, 0, 0), |x, y, color| {
                if color.a() == 0 {
                    return;
                }
                let (px, py) = ((physical.x + x) as f32, run.line_y + (physical.y + y) as f32);
                b = (b.0.min(px), b.1.min(py), b.2.max(px + 1.0), b.3.max(py + 1.0));
            });
        }
    }
    Ok((b.0 < b.2).then_some(b))
}

/// Répartit des mots de largeurs `widths` (espace `space` entre deux) en `n` lignes de largeurs aussi proches que
/// possible (moindres carrés) : plages d'indices de mots.
fn balance(widths: &[f32], space: f32, n: usize) -> Vec<(usize, usize)> {
    let count = widths.len();
    let n = n.clamp(1, count.max(1));
    if count == 0 {
        return Vec::new();
    }
    let mut prefix = vec![0.0f32; count + 1];
    for (i, w) in widths.iter().enumerate() {
        prefix[i + 1] = prefix[i] + w;
    }
    let total = prefix[count] + space * (count as f32 - n as f32).max(0.0);
    let target = total / n as f32;
    let cost = |i: usize, j: usize| {
        let w = prefix[j] - prefix[i] + space * (j - i - 1) as f32;
        (w - target) * (w - target)
    };
    // best[k][j] : coût minimal des j premiers mots en k lignes.
    let mut best = vec![vec![f32::INFINITY; count + 1]; n + 1];
    let mut cut = vec![vec![0usize; count + 1]; n + 1];
    best[0][0] = 0.0;
    for k in 1..=n {
        for j in k..=count {
            for i in (k - 1)..j {
                let c = best[k - 1][i] + cost(i, j);
                if c < best[k][j] {
                    best[k][j] = c;
                    cut[k][j] = i;
                }
            }
        }
    }
    let mut out = Vec::with_capacity(n);
    let mut j = count;
    for k in (1..=n).rev() {
        let i = cut[k][j];
        out.push((i, j));
        j = i;
    }
    out.reverse();
    out
}

/// Mise en page d'un texte dynamique : ses lignes, de haut en bas.
fn dynamic_lines(fonts: &mut Fonts, text: &TextData, dynamic: DynamicText, frame: TextFrame) -> Result<Vec<DynamicLine>, EngineError> {
    let valid = |v: f32| v.is_finite() && (1.0..=MAX_BLOCK).contains(&v);
    if !(valid(frame.width) && valid(frame.height)) {
        return Err(EngineError::InvalidOperation("cadre de texte hors limites".into()));
    }
    let content = &text.content;
    // Lignes : plages d'octets du contenu (du premier au dernier mot de la ligne).
    let groups: Vec<(usize, usize)> = if dynamic.manual {
        let mut out = Vec::new();
        let mut at = 0;
        for line in content.split('\n') {
            let words = words_of(line);
            if let (Some(first), Some(last)) = (words.first(), words.last()) {
                out.push((at + first.0, at + last.1));
            }
            at += line.len() + 1;
        }
        out
    } else {
        let words = words_of(content);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        // Chasse de chaque mot au corps de mesure (une seule mise en page : un mot par ligne).
        let mut probe: String = content.clone();
        // SAFETY de longueur : chaque espace ASCII devient un retour, d'un octet aussi.
        probe = probe.chars().map(|c| if c == ' ' || c == '\t' { '\n' } else { c }).collect();
        let measure = TextData { content: probe, ..line_text(text, 0, 0, DYNAMIC_REFERENCE) };
        let measure = TextData { runs: text.runs.clone(), ..measure };
        let Layout { buffer, .. } = layout(fonts, &measure)?;
        let starts: Vec<usize> = std::iter::once(0).chain(measure.content.match_indices('\n').map(|(i, _)| i + 1)).collect();
        let mut widths = vec![0.0f32; words.len()];
        for run in buffer.layout_runs() {
            let Some(line_start) = starts.get(run.line_i) else { continue };
            if let Some(w) = words.iter().position(|(a, _)| a == line_start) {
                widths[w] = widths[w].max(run.line_w);
            }
        }
        let space = DYNAMIC_REFERENCE * 0.28;
        let height_of = |parts: &[(usize, usize)]| -> f32 {
            let lines: Vec<f32> = parts
                .iter()
                .map(|&(i, j)| {
                    let w = widths[i..j].iter().sum::<f32>() + space * (j - i - 1) as f32;
                    frame.width / w.max(1.0) * DYNAMIC_REFERENCE * 0.74
                })
                .collect();
            let mean = lines.iter().sum::<f32>() / lines.len().max(1) as f32;
            lines.iter().sum::<f32>() + dynamic.gap.clamp(-0.2, 2.0) * mean * (lines.len() as f32 - 1.0)
        };
        let to_bytes = |parts: Vec<(usize, usize)>| parts.iter().map(|&(i, j)| (words[i].0, words[j - 1].1)).collect::<Vec<_>>();
        if dynamic.lines > 0 {
            to_bytes(balance(&widths, space, usize::from(dynamic.lines)))
        } else {
            // Automatique : le nombre de lignes dont la hauteur approche le mieux celle du cadre. L'estimation (chasse
            // des mots) désigne un nombre ; ses voisins sont ensuite mesurés pour de bon (encre réelle) et le
            // meilleur l'emporte.
            let most = words.len().min(DYNAMIC_MAX_LINES);
            let miss = |h: f32| (h / frame.height).ln().abs();
            let guess = (1..=most).min_by(|a, b| miss(height_of(&balance(&widths, space, *a))).total_cmp(&miss(height_of(&balance(&widths, space, *b))))).unwrap_or(1);
            let mut best: Option<(f32, Vec<(usize, usize)>)> = None;
            for n in guess.saturating_sub(2).max(1)..=(guess + 2).min(most) {
                let groups = to_bytes(balance(&widths, space, n));
                let (_, total) = measure_lines(fonts, text, &groups, dynamic, frame)?;
                if best.as_ref().is_none_or(|(m, _)| miss(total) < *m) {
                    best = Some((miss(total), groups));
                }
            }
            best.map(|(_, g)| g).unwrap_or_default()
        }
    };

    // Chaque ligne mesurée au corps de référence, puis mise à la largeur du cadre.
    let (measured, total) = measure_lines(fonts, text, &groups, dynamic, frame)?;
    if measured.is_empty() {
        return Ok(Vec::new());
    }
    let scale_of = |b: &(f32, f32, f32, f32)| frame.width / (b.2 - b.0).max(1.0);
    let heights: Vec<f32> = measured.iter().map(|(_, _, b)| (b.3 - b.1) * scale_of(b)).collect();
    let mean = heights.iter().sum::<f32>() / heights.len() as f32;
    let gap = dynamic.gap.clamp(-0.2, 2.0) * mean;
    let fit = (frame.height / total.max(1.0)).min(1.0);
    let mut y = (frame.height - total * fit) / 2.0;
    let mut lines = Vec::with_capacity(measured.len());
    for (start, end, b) in measured {
        let size = (DYNAMIC_REFERENCE * scale_of(&b) * fit).clamp(1.0, 4_000.0);
        let k = size / DYNAMIC_REFERENCE;
        let width = (b.2 - b.0) * k;
        let height = (b.3 - b.1) * k;
        let x = (frame.width - width) / 2.0;
        lines.push(DynamicLine { start, end, text: line_text(text, start, end, size), offset: (x - b.0 * k, y - b.1 * k), top: y, bottom: y + height });
        y += height + gap * fit;
    }
    Ok(lines)
}

/// Encre de chaque ligne au corps de référence, et hauteur totale une fois chaque ligne à la largeur du cadre.
#[allow(clippy::type_complexity)]
fn measure_lines(fonts: &mut Fonts, text: &TextData, groups: &[(usize, usize)], dynamic: DynamicText, frame: TextFrame) -> Result<(Vec<(usize, usize, (f32, f32, f32, f32))>, f32), EngineError> {
    let mut measured = Vec::new();
    for &(start, end) in groups {
        if let Some(b) = ink_bounds(fonts, &line_text(text, start, end, DYNAMIC_REFERENCE))? {
            measured.push((start, end, b));
        }
    }
    let heights: Vec<f32> = measured.iter().map(|(_, _, b)| (b.3 - b.1) * frame.width / (b.2 - b.0).max(1.0)).collect();
    let mean = heights.iter().sum::<f32>() / heights.len().max(1) as f32;
    let total = heights.iter().sum::<f32>() + dynamic.gap.clamp(-0.2, 2.0) * mean * (heights.len() as f32 - 1.0).max(0.0);
    Ok((measured, total))
}

/// Rendu d'un texte dynamique, en repère texte (origine : coin haut gauche du cadre).
fn render_dynamic(fonts: &mut Fonts, text: &TextData, dynamic: DynamicText, frame: TextFrame) -> Result<TileStore, EngineError> {
    let lines = dynamic_lines(fonts, text, dynamic, frame)?;
    let pad = 8i32;
    let (w, h) = ((frame.width.ceil() as i32 + 2 * pad) as usize, (frame.height.ceil() as i32 + 2 * pad) as usize);
    let mut rgba = vec![0u8; w * h * 4];
    let [r, g, b] = text.color;
    for line in &lines {
        let Layout { buffer, .. } = layout(fonts, &line.text)?;
        let Fonts { system, cache, .. } = &mut *fonts;
        let (ox, oy) = (line.offset.0.round() as i32 + pad, line.offset.1.round() as i32 + pad);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let base = glyph.color_opt.unwrap_or(Color::rgb(r, g, b));
                cache.with_pixels(system, physical.cache_key, base, |x, y, color| {
                    let (px, py) = (physical.x + x + ox, run.line_y as i32 + physical.y + y + oy);
                    if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                        return;
                    }
                    let ink = premultiply([color.r(), color.g(), color.b(), color.a()]);
                    let i = (py as usize * w + px as usize) * 4;
                    let under = [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]];
                    rgba[i..i + 4].copy_from_slice(&over(ink, under));
                });
            }
        }
    }
    let store = TileStore::from_premultiplied(w as u32, h as u32, &rgba).ok_or_else(|| EngineError::Corrupt("tampon de texte de taille inattendue".into()))?;
    Ok(store.translated(-pad, -pad))
}

/// Ligne d'un texte dynamique qui contient la position `byte` (entre deux lignes : la fin de la précédente).
fn line_at(lines: &[DynamicLine], byte: usize) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    Some(lines.iter().position(|l| byte <= l.end).unwrap_or(lines.len() - 1))
}

/// Octet du contenu à la position `utf16`.
fn byte_at(content: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, ch) in content.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += ch.len_utf16();
    }
    content.len()
}

fn dynamic_caret(fonts: &mut Fonts, text: &TextData, dynamic: DynamicText, frame: TextFrame, start: usize, end: usize) -> Result<CaretGeometry, EngineError> {
    let lines = dynamic_lines(fonts, text, dynamic, frame)?;
    let local = |line: &DynamicLine, byte: usize| text.content[line.start..byte.clamp(line.start, line.end)].encode_utf16().count();
    let focus = byte_at(&text.content, end);
    let Some(at) = line_at(&lines, focus) else {
        let size = text.size.min(frame.height);
        return Ok(CaretGeometry { caret: ((0.0, 0.0), (0.0, size)), highlights: Vec::new(), overflow: false });
    };
    let line = &lines[at];
    let here = caret_in(fonts, &line.text, local(line, focus), local(line, focus))?;
    let shift = |(x, y): (f32, f32)| (x + line.offset.0, y + line.offset.1);
    let caret = (shift(here.caret.0), shift(here.caret.1));
    let (from, to) = (byte_at(&text.content, start.min(end)), byte_at(&text.content, start.max(end)));
    let mut highlights = Vec::new();
    if from < to {
        for line in lines.iter().filter(|l| l.end > from && l.start < to) {
            let part = caret_in(fonts, &line.text, local(line, from), local(line, to))?;
            highlights.extend(part.highlights.into_iter().map(|(x, y, w, h)| (x + line.offset.0, y + line.offset.1, w, h)));
        }
    }
    Ok(CaretGeometry { caret, highlights, overflow: false })
}

fn dynamic_hit(fonts: &mut Fonts, text: &TextData, dynamic: DynamicText, frame: TextFrame, x: f32, y: f32) -> Result<usize, EngineError> {
    let lines = dynamic_lines(fonts, text, dynamic, frame)?;
    // La ligne la plus proche verticalement.
    let distance = |l: &DynamicLine| if y < l.top { l.top - y } else if y > l.bottom { y - l.bottom } else { 0.0 };
    let Some(line) = lines.iter().min_by(|a, b| distance(a).total_cmp(&distance(b))) else { return Ok(text.content.encode_utf16().count()) };
    let local = hit_in(fonts, &line.text, x - line.offset.0, y - line.offset.1)?;
    Ok(text.content[..line.start].encode_utf16().count() + local)
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
    /// Texte de paragraphe dont une partie dépasse du cadre (voir [`overflows`]).
    pub overflow: bool,
}

/// Début (octet dans la ligne) de ce qu'affiche une ligne visuelle ; une ligne sans glyphe commence à 0.
fn run_start(run: &cosmic_text::LayoutRun<'_>) -> usize {
    run.glyphs.iter().map(|g| g.start).min().unwrap_or(0)
}

pub fn caret_geometry(text: &TextData, start: usize, end: usize) -> Result<CaretGeometry, EngineError> {
    let mut guard = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((dynamic, frame)) = dynamic_of(text) {
        return dynamic_caret(&mut guard, text, dynamic, frame, start, end);
    }
    caret_in(&mut guard, text, start, end)
}

fn caret_in(fonts: &mut Fonts, text: &TextData, start: usize, end: usize) -> Result<CaretGeometry, EngineError> {
    let Layout { buffer, width, anchor, clip, .. } = layout(fonts, text)?;
    let (from, to) = (cursor_at(&text.content, start.min(end)), cursor_at(&text.content, start.max(end)));
    let focus = cursor_at(&text.content, end);
    let empty_line_x = empty_line_x(text.align, width);
    let runs: Vec<_> = buffer.layout_runs().collect();

    let line_height = text.size * text.line_height.clamp(0.5, 5.0);
    // Ligne sans mise en page (ligne vide en fin de texte, juste après Entrée) : le curseur
    // se place quand même sur SA ligne, à l'ancre, sous la dernière ligne mise en page qui la précède.
    let above = runs.iter().filter(|run| run.line_i < focus.line).map(|run| (run.line_i, run.line_top + run.line_height)).fold((None, 0.0f32), |acc, (i, b)| (Some(i), acc.1.max(b)));
    let top = match above {
        (Some(last), bottom) => bottom + (focus.line - last - 1) as f32 * line_height,
        (None, _) => focus.line as f32 * line_height,
    };
    let mut caret = ((-anchor + empty_line_x, top), (-anchor + empty_line_x, top + line_height));
    // Une ligne du texte peut occuper plusieurs lignes visuelles (retour à la ligne d'un paragraphe) : le curseur va
    // sur celle qui contient sa position ; à la jonction de deux, au début de la suivante, comme dans un traitement
    // de texte (taper l'espace qui fait passer le mot à la ligne y emmène le curseur).
    // L'espace de coupure, elle, n'est pas toujours mise en page : juste avant elle, c'est la fin de la ligne.
    let same: Vec<_> = runs.iter().filter(|run| run.line_i == focus.line).collect();
    let chosen = (0..same.len()).find(|&k| same.get(k + 1).is_none_or(|next| focus.index < run_start(next)));
    if let Some(run) = chosen.and_then(|k| same.get(k)) {
        let x = caret_x(run, focus.index, empty_line_x) - anchor;
        caret = ((x, run.line_top), (x, run.line_top + run.line_height));
    }
    let mut highlights = Vec::new();
    if from != to {
        for run in &runs {
            if let Some((x, w)) = run.highlight(from, to) {
                highlights.push((x - anchor, run.line_top, w.max(text.size * 0.25), run.line_height));
            }
        }
    }
    let overflow = text.frame.is_some() && overflow_of(&buffer, clip);
    Ok(CaretGeometry { caret, highlights, overflow })
}

/// Position du curseur (en unités UTF-16) la plus proche du point `(x, y)` du repère texte.
pub fn hit_test(text: &TextData, x: f32, y: f32) -> Result<usize, EngineError> {
    let mut guard = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((dynamic, frame)) = dynamic_of(text) {
        return dynamic_hit(&mut guard, text, dynamic, frame, x, y);
    }
    hit_in(&mut guard, text, x, y)
}

fn hit_in(fonts: &mut Fonts, text: &TextData, x: f32, y: f32) -> Result<usize, EngineError> {
    let Layout { buffer, height, anchor, clip, .. } = layout(fonts, text)?;
    // Au-dessus ou au-dessous du bloc, on vise la première ou la dernière ligne. Texte de paragraphe : la dernière
    // ligne VISIBLE (on ne place pas le curseur dans le texte masqué en cliquant sous le cadre).
    let bottom = if text.frame.is_some() {
        buffer.layout_runs().take_while(|run| line_shown(run, clip)).map(|run| run.line_top + run.line_height).fold(0.0f32, f32::max).min(height)
    } else {
        height
    };
    let cursor = buffer.hit(x + anchor, y.clamp(0.0, (bottom - 1.0).max(0.0)));
    Ok(cursor.map_or_else(|| text.content.encode_utf16().count(), |c| utf16_of(&text.content, c)))
}

/// Contenu tel qu'il s'affiche dans un texte de paragraphe, une ligne visuelle par ligne : chaque retour à la ligne
/// automatique devient un vrai retour (l'espace où la ligne se coupe est remplacée, sinon un retour est inséré).
/// Les portions stylées sont recalées. Sert à « Convertir en texte de point » sans changer l'aspect du texte.
fn hard_wrapped(text: &TextData, buffer: &Buffer) -> TextData {
    // Points de coupure, en octets du contenu : début de chaque ligne visuelle qui n'est pas la première de sa ligne.
    let line_starts: Vec<usize> = std::iter::once(0).chain(text.content.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let mut breaks: Vec<usize> = Vec::new();
    let mut previous_line = None;
    for run in buffer.layout_runs() {
        let start = run.glyphs.iter().map(|g| g.start).min();
        if previous_line == Some(run.line_i) {
            if let (Some(start), Some(line_start)) = (start, line_starts.get(run.line_i)) {
                breaks.push(line_start + start);
            }
        }
        previous_line = Some(run.line_i);
    }
    let mut content = String::with_capacity(text.content.len() + breaks.len());
    let mut runs = text.runs.clone();
    let mut copied = 0;
    let mut inserted = 0usize; // unités UTF-16 ajoutées jusqu'ici
    for at in breaks {
        let before = &text.content[copied..at];
        match before.char_indices().next_back() {
            // Coupure après une espace : l'espace devient le retour à la ligne (une unité UTF-16 l'une comme l'autre :
            // les portions stylées ne bougent pas).
            Some((i, ' ' | '\t' | '\u{3000}')) => {
                content.push_str(&before[..i]);
                content.push('\n');
            }
            // Coupure au milieu d'un mot trop long : un retour est inséré, les portions qui suivent se décalent.
            _ => {
                content.push_str(before);
                content.push('\n');
                let position = text.content[..at].encode_utf16().count() + inserted;
                for run in &mut runs {
                    if run.start >= position {
                        run.start += 1;
                    }
                    if run.end > position {
                        run.end += 1;
                    }
                }
                inserted += 1;
            }
        }
        copied = at;
    }
    content.push_str(&text.content[copied..]);
    TextData { content, runs, ..text.clone() }
}

/// « Convertir en texte de paragraphe » : le cadre est la boîte actuelle du texte, et le texte garde son aspect.
/// Rend le nouveau texte et le décalage de son origine (repère texte d'avant) : l'ancre devient le coin du cadre.
pub fn to_paragraph(text: &TextData) -> Result<(TextData, (f32, f32)), EngineError> {
    if text.frame.is_some() {
        return Ok((text.clone(), (0.0, 0.0)));
    }
    let mut guard = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    let Layout { width, height, anchor, .. } = layout(&mut guard, text)?;
    // Texte vide : un cadre d'au moins un corps de côté, pour qu'il reste saisissable.
    let frame = TextFrame { width: width.max(text.size.ceil()).max(1.0), height: height.max(1.0) };
    Ok((TextData { frame: Some(frame), ..text.clone() }, (-anchor, 0.0)))
}

/// « Convertir en texte de point » : chaque ligne visuelle du paragraphe devient une ligne du texte (retours
/// insérés), le texte masqué sous le cadre réapparaît. Rend le nouveau texte et le décalage de son origine (repère
/// texte d'avant) : le coin du cadre devient l'ancre (bord gauche, milieu ou bord droit selon l'alignement).
pub fn to_point(text: &TextData) -> Result<(TextData, (f32, f32)), EngineError> {
    let Some(frame) = text.frame else {
        return Ok((text.clone(), (0.0, 0.0)));
    };
    let mut guard = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    let Layout { buffer, .. } = layout(&mut guard, text)?;
    let mut point = hard_wrapped(text, &buffer);
    point.frame = None;
    point.dynamic = None;
    // Un texte de point justifié s'aligne à gauche : autant le dire au panneau.
    if point.align == TextAlign::Justify {
        point.align = TextAlign::Left;
    }
    let dx = match point.align {
        TextAlign::Left | TextAlign::Justify => 0.0,
        TextAlign::Center => (frame.width / 2.0).round(),
        TextAlign::Right => frame.width.round(),
    };
    Ok((point, (dx, 0.0)))
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
            frame: None,
            dynamic: None,
            caps: TextCaps::Normal,
            style: None,
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

    /// Texte dynamique : chaque ligne remplit la largeur du cadre (à quelques pixels près), le tout tient dans le
    /// cadre ; le curseur et le clic retombent sur le bon caractère ; le nombre de lignes demandé est respecté.
    #[test]
    fn dynamic_text_fills_the_frame_width_line_by_line() {
        let text = TextData {
            content: "Le texte qui remplit toute la largeur du cadre".into(),
            size: 40.0,
            frame: Some(TextFrame { width: 400.0, height: 500.0 }),
            dynamic: Some(DynamicText::default()),
            ..TextData::default()
        };
        let store = render(&text).expect("rendu");
        let (x0, y0, x1, y1) = extent(&store);
        assert!(x0 >= -3 && x1 <= 403 && x1 - x0 >= 392, "largeur remplie : {:?}", (x0, x1));
        assert!(y0 >= -3 && y1 <= 503, "dans le cadre : {:?}", (y0, y1));
        let mut guard = fonts().lock().unwrap_or_else(PoisonError::into_inner);
        let lines = dynamic_lines(&mut guard, &text, DynamicText::default(), TextFrame { width: 400.0, height: 500.0 }).expect("lignes");
        assert!(lines.len() >= 3, "le cadre haut appelle plusieurs lignes ({})", lines.len());
        let fixed = dynamic_lines(&mut guard, &text, DynamicText { lines: 2, ..DynamicText::default() }, TextFrame { width: 400.0, height: 500.0 }).expect("lignes");
        assert_eq!(fixed.len(), 2);
        let manual = TextData { content: "UN\ndeux trois".into(), ..text.clone() };
        let rows = dynamic_lines(&mut guard, &manual, DynamicText { manual: true, ..DynamicText::default() }, TextFrame { width: 400.0, height: 500.0 }).expect("lignes");
        assert_eq!(rows.iter().map(|l| &manual.content[l.start..l.end]).collect::<Vec<_>>(), ["UN", "deux trois"]);
        drop(guard);
        // Le clic au milieu de la dernière ligne vise un caractère de cette ligne.
        let last = &lines[lines.len() - 1];
        let hit = hit_test(&text, 200.0, (last.top + last.bottom) / 2.0).expect("clic");
        let first_of_last = text.content[..last.start].encode_utf16().count();
        assert!(hit >= first_of_last, "{hit} < {first_of_last}");
        let caret = caret_geometry(&text, hit, hit).expect("curseur").caret;
        assert!(caret.0.1 < last.bottom && caret.1.1 > last.top, "curseur sur la ligne visée");
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
        let run = StyleRun { start: 3, end: 5, font_family: None, weight: Some(900), italic: None, color: Some([0, 255, 0]), size: None, tracking: None, caps: None, style: None };
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

    fn paragraph(content: &str, width: f32, height: f32) -> TextData {
        TextData { frame: Some(TextFrame { width, height }), ..sample(content) }
    }

    /// Lignes visuelles d'un texte : (texte de la ligne, gauche de l'encre, droite de l'encre, haut de ligne).
    fn visual_lines(text: &TextData) -> Vec<(String, f32, f32, f32)> {
        let mut guard = fonts().lock().unwrap_or_else(PoisonError::into_inner);
        let Layout { buffer, .. } = layout(&mut guard, text).expect("mise en page");
        buffer
            .layout_runs()
            .map(|run| {
                let shown: String = run.glyphs.iter().map(|g| &run.text[g.start..g.end]).collect();
                let ink: Vec<_> = run.glyphs.iter().filter(|g| !run.text[g.start..g.end].trim().is_empty()).collect();
                let left = ink.first().map_or(0.0, |g| g.x);
                let right = ink.last().map_or(0.0, |g| g.x + g.w);
                (shown, left, right, run.line_top)
            })
            .collect()
    }

    #[test]
    fn paragraph_text_wraps_by_words_inside_its_frame() {
        let text = paragraph("Lorem ipsum dolor sit amet consectetur", 300.0, 400.0);
        let lines = visual_lines(&text);
        assert!(lines.len() >= 3, "plusieurs lignes : {lines:?}");
        for (shown, _, right, _) in &lines {
            assert!(*right <= 300.5, "« {shown} » dépasse la largeur du cadre ({right})");
        }
        // Coupure par mots : aucun mot n'est coupé, et chaque mot du texte se retrouve tel quel.
        let rebuilt: Vec<String> = lines.iter().flat_map(|(s, ..)| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>()).collect();
        assert_eq!(rebuilt.join(" "), "Lorem ipsum dolor sit amet consectetur");
        // Le rendu est celui de la mise en page, dans le cadre ; un texte de point reste sur une ligne.
        let (x0, y0, x1, y1) = extent(&render(&text).expect("rendu"));
        assert!(x0 >= -2 && x1 <= 304 && y0 >= 0 && y1 > 100, "{:?}", (x0, y0, x1, y1));
        assert_eq!(visual_lines(&sample("Lorem ipsum dolor sit amet consectetur")).len(), 1);
    }

    #[test]
    fn a_word_longer_than_the_line_is_broken_and_empty_lines_are_kept() {
        let lines = visual_lines(&paragraph("Anticonstitutionnellement", 120.0, 400.0));
        assert!(lines.len() >= 2, "mot trop long coupé : {lines:?}");
        assert!(lines.iter().all(|(_, _, right, _)| *right <= 120.5));
        assert_eq!(lines.iter().map(|(s, ..)| s.as_str()).collect::<String>(), "Anticonstitutionnellement");
        // Lignes vides (Entrée, Entrée) : elles gardent leur hauteur.
        let blank = visual_lines(&paragraph("a\n\nb", 300.0, 400.0));
        assert_eq!(blank.len(), 3, "{blank:?}");
        assert!((blank[2].3 - 2.0 * 40.0 * 1.2).abs() < 1.0, "troisième ligne à deux interlignes : {blank:?}");
    }

    #[test]
    fn paragraph_alignment_uses_the_frame_width() {
        let content = "Un deux trois quatre cinq six sept huit";
        let lines = |align| visual_lines(&TextData { align, ..paragraph(content, 300.0, 400.0) });
        let (left, center, right, justify) = (lines(TextAlign::Left), lines(TextAlign::Center), lines(TextAlign::Right), lines(TextAlign::Justify));
        assert!(left.iter().all(|(_, l, ..)| *l < 6.0), "à gauche : {left:?}");
        assert!(right.iter().all(|(_, _, r, _)| *r > 290.0 && *r <= 300.5), "à droite : {right:?}");
        assert!(center.iter().all(|(_, l, r, _)| ((l + r) / 2.0 - 150.0).abs() < 8.0), "centré : {center:?}");
        // Justifié : toutes les lignes pleines vont d'un bord à l'autre, sauf la dernière, alignée à gauche.
        let (last, full) = justify.split_last().expect("lignes");
        assert!(full.len() >= 1 && full.iter().all(|(_, l, r, _)| *l < 6.0 && *r > 290.0), "justifié : {justify:?}");
        assert!(last.2 < 290.0, "dernière ligne non étirée : {last:?}");
        // Un texte de point justifié s'aligne à gauche (rien à justifier sans largeur).
        let point = render(&TextData { align: TextAlign::Justify, ..sample("Un\nDeux lignes") }).expect("rendu");
        assert!(extent(&point).0 >= -4);
    }

    #[test]
    fn text_below_the_frame_is_hidden_and_reported() {
        let content = "Lorem ipsum dolor sit amet consectetur adipiscing elit";
        let tall = paragraph(content, 200.0, 1000.0);
        let short = paragraph(content, 200.0, 100.0);
        assert!(!overflows(&tall).expect("mesure") && overflows(&short).expect("mesure"));
        assert!(caret_geometry(&short, 0, 0).expect("curseur").overflow);
        let (_, _, _, tall_bottom) = extent(&render(&tall).expect("rendu"));
        let (_, _, _, short_bottom) = extent(&render(&short).expect("rendu"));
        assert!(tall_bottom > 150 && short_bottom <= 100 + 6, "seules les lignes entières du cadre sont dessinées : {short_bottom}");
        // Lignes vides sous le cadre : rien n'est caché.
        assert!(!overflows(&paragraph("abc\n\n\n\n\n\n", 200.0, 50.0)).expect("mesure"));
        assert!(!overflows(&sample(content)).expect("texte de point"));
    }

    #[test]
    fn resizing_the_frame_recomposes_the_text_without_scaling_it() {
        let content = "Lorem ipsum dolor sit amet consectetur";
        let (narrow, wide) = (render(&paragraph(content, 150.0, 600.0)).expect("rendu"), render(&paragraph(content, 600.0, 600.0)).expect("rendu"));
        let (n, w) = (extent(&narrow), extent(&wide));
        assert!(n.3 - n.1 > 2 * (w.3 - w.1), "plus étroit : plus de lignes ({n:?} / {w:?})");
        // Même corps : une ligne d'un « H » a la même hauteur d'encre dans les deux cadres.
        let h = |width| extent(&render(&paragraph("H", width, 100.0)).expect("rendu"));
        let (a, b) = (h(100.0), h(400.0));
        assert_eq!(a.3 - a.1, b.3 - b.1, "pas d'étirement vertical");
        assert_eq!(a.2 - a.0, b.2 - b.0, "pas d'étirement horizontal");
        assert!(render(&paragraph("a", 0.0, 10.0)).is_err() && render(&paragraph("a", f32::NAN, 10.0)).is_err(), "cadre invalide refusé");
    }

    #[test]
    fn caret_and_hit_testing_follow_the_paragraph_layout() {
        let text = paragraph("alpha beta gamma delta epsilon", 160.0, 400.0);
        let lines = visual_lines(&text);
        assert!(lines.len() >= 2, "{lines:?}");
        let second_start = text.content.find(lines[1].0.trim_start()).expect("deuxième ligne");
        let first_end = lines[0].0.trim_end().len();
        // Juste après le dernier mot d'une ligne : la fin de CETTE ligne ; après l'espace de coupure, la suivante.
        let ((end_x, end_top), _) = caret_geometry(&text, first_end, first_end).expect("curseur").caret;
        assert!(end_top < 1.0 && end_x > 40.0, "fin de la première ligne : {:?}", (end_x, end_top));
        let ((x, top), (_, bottom)) = caret_geometry(&text, second_start, second_start).expect("curseur").caret;
        assert!(x < 2.0 && top >= 40.0, "début de la deuxième ligne : {:?}", (x, top));
        assert_eq!(hit_test(&text, x + 0.5, (top + bottom) / 2.0).expect("pointage"), second_start);
        for offset in [0usize, 3, second_start + 2, 30] {
            let ((x, top), (_, bottom)) = caret_geometry(&text, offset, offset).expect("curseur").caret;
            assert_eq!(hit_test(&text, x + 0.5, (top + bottom) / 2.0).expect("pointage"), offset, "décalage {offset}");
        }
        // Une sélection sur deux lignes visuelles : deux zones.
        assert_eq!(caret_geometry(&text, 1, second_start + 2).expect("sélection").highlights.len(), 2);
        // Sous le cadre : la dernière ligne visible, pas le texte masqué.
        let hidden = paragraph("alpha beta gamma delta epsilon", 160.0, 50.0);
        let below = hit_test(&hidden, 5.0, 300.0).expect("pointage");
        assert!(below <= second_start, "le clic sous le cadre reste sur la ligne visible : {below}");
    }

    #[test]
    fn frames_survive_serialization_and_old_files_are_point_text() {
        let text = TextData { align: TextAlign::Justify, ..paragraph("abc", 120.5, 80.0) };
        let json = serde_json::to_string(&text).expect("json");
        assert!(json.contains(r#""frame":{"width":120.5,"height":80.0}"#) && json.contains(r#""align":"justify""#), "{json}");
        assert_eq!(serde_json::from_str::<TextData>(&json).expect("relu"), text);
        let old: TextData = serde_json::from_str(r#"{"content":"a","fontFamily":"","size":40,"lineHeight":1.2,"tracking":0,"align":"left","color":[0,0,0],"weight":400,"italic":false}"#).expect("ancien");
        assert!(old.frame.is_none(), "un ancien texte reste un texte de point");
        assert!(!serde_json::to_string(&sample("a")).expect("json").contains("frame"), "un texte de point n'écrit pas le champ");
    }

    #[test]
    fn converting_between_point_and_paragraph_keeps_the_look() {
        // Paragraphe -> point : un retour par ligne visuelle ; un mot coupé reçoit un retour inséré, les portions suivent.
        let run = StyleRun { start: 6, end: 10, font_family: None, weight: None, italic: None, color: Some([0, 0, 255]), size: None, tracking: None, caps: None, style: None };
        let text = TextData { runs: vec![run], ..paragraph("alpha beta gamma Anticonstitutionnellement", 200.0, 600.0) };
        let (point, (dx, dy)) = to_point(&text).expect("conversion");
        assert!(point.frame.is_none() && (dx, dy) == (0.0, 0.0));
        let lines: Vec<&str> = point.content.split('\n').collect();
        assert!(lines.len() >= 3, "{lines:?}");
        let letters = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(letters(&point.content), letters(&text.content), "aucune lettre perdue ni ajoutée");
        assert_eq!(point.content.matches('\n').count(), lines.len() - 1);
        assert!(lines.iter().all(|line| !line.ends_with(' ')), "l'espace de coupure devient le retour : {lines:?}");
        let blue: String = point.content.encode_utf16().collect::<Vec<_>>()[point.runs[0].start..point.runs[0].end].iter().map(|u| char::from_u32(u32::from(*u)).unwrap_or('?')).collect();
        assert_eq!(blue, "beta", "la portion stylée suit le texte");
        let a = extent(&render(&text).expect("paragraphe"));
        let b = extent(&render(&point).expect("point"));
        assert!([a.0 - b.0, a.1 - b.1, a.2 - b.2, a.3 - b.3].iter().all(|d| d.abs() <= 1), "même aspect : {a:?} / {b:?}");
        // Point -> paragraphe : le cadre est la boîte du texte, l'ancre devient le coin haut gauche.
        let centered = TextData { align: TextAlign::Center, ..sample("Un\nDeux lignes") };
        let (para, (dx, _)) = to_paragraph(&centered).expect("conversion");
        let frame = para.frame.expect("cadre");
        assert!(dx < 0.0 && (dx + (frame.width / 2.0).round()).abs() < 0.01, "origine au bord gauche : {dx} / {frame:?}");
        let before = extent(&render(&centered).expect("point"));
        let after = extent(&render(&para).expect("paragraphe"));
        let moved = (after.0 + dx as i32, after.1, after.2 + dx as i32, after.3);
        assert!((moved.0 - before.0).abs() <= 1 && (moved.2 - before.2).abs() <= 1 && moved.1 == before.1, "{before:?} -> {moved:?}");
        assert!(!overflows(&para).expect("mesure"), "la boîte du texte le contient en entier");
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
