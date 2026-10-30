//! Pinceaux importés : fichiers Photoshop (.abr), GIMP (.gbr) et simples images.
//!
//! Une pointe est une carte de couverture en niveaux de gris (255 = dépose toute la couleur). Un fichier ABR récent
//! (version 6 et suivantes) contient en plus ses préréglages — nom, diamètre, espacement, angle, rondeur, dynamiques,
//! pinceau double, texture — décrits dans le bloc `desc`, et les textures elles-mêmes dans le bloc `patt` : tout est
//! repris (voir `BrushPreset`), y compris les pinceaux « calculés » (ronds paramétriques), rendus par le rond natif.

use std::path::Path;

use crate::error::EngineError;

/// Plus grand côté conservé pour une pointe ; au-delà elle est réduite (elle sera de toute façon mise à l'échelle du pinceau).
const MAX_TIP_SIDE: u32 = 1024;

/// Version réduite d'une pointe (moitié de la précédente, par moyenne de blocs 2 × 2).
#[derive(Clone, Debug, PartialEq)]
pub struct TipLevel {
    pub width: u32,
    pub height: u32,
    pub mask: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BrushTip {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Couverture par pixel, ligne par ligne.
    pub mask: Vec<u8>,
    /// Espacement des empreintes, en fraction du diamètre (0,25 = 25 %).
    pub spacing: f32,
    /// Identifiant de la pointe dans son fichier ABR (les préréglages la désignent ainsi) ; vide sinon.
    pub uid: String,
    /// Versions réduites (1/2, 1/4…) : une grande pointe posée petite est lue dans la version la plus proche de sa
    /// taille à l'écran. Lue en pleine définition, un pixel du trait ne voyait qu'un pixel de la pointe sur des
    /// dizaines : bord en escalier et grain qui scintille d'une empreinte à l'autre.
    pub levels: Vec<TipLevel>,
}

impl BrushTip {
    /// Pointe à partir d'une carte de couverture ; `None` si elle est vide ou incohérente.
    pub fn from_mask(name: String, width: u32, height: u32, mask: Vec<u8>, spacing: f32) -> Option<Self> {
        if width == 0 || height == 0 || mask.len() != (width as usize) * (height as usize) || mask.iter().all(|v| *v == 0) {
            return None;
        }
        let spacing = if spacing.is_finite() { spacing.clamp(0.01, 10.0) } else { 0.25 };
        let tip = Self { name, width, height, mask, spacing, uid: String::new(), levels: Vec::new() };
        Some(tip.limited().with_levels())
    }

    fn new(name: String, width: u32, height: u32, mask: Vec<u8>, spacing: f32) -> Option<Self> {
        Self::from_mask(name, width, height, mask, spacing)
    }

    /// Réduit une pointe démesurée par moyenne de blocs.
    fn limited(self) -> Self {
        let factor = self.width.max(self.height).div_ceil(MAX_TIP_SIDE).max(1);
        if factor == 1 {
            return self;
        }
        let (w, h) = (self.width.div_ceil(factor), self.height.div_ceil(factor));
        let mut mask = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let (mut sum, mut count) = (0u32, 0u32);
                for sy in y * factor..((y + 1) * factor).min(self.height) {
                    for sx in x * factor..((x + 1) * factor).min(self.width) {
                        sum += u32::from(self.mask[(sy * self.width + sx) as usize]);
                        count += 1;
                    }
                }
                mask.push((sum / count.max(1)) as u8);
            }
        }
        Self { width: w, height: h, mask, ..self }
    }

    /// Calcule les versions réduites, jusqu'à 2 pixels de côté.
    fn with_levels(mut self) -> Self {
        let (mut w, mut h, mut mask) = (self.width, self.height, self.mask.clone());
        let mut levels = Vec::new();
        while w.max(h) > 2 {
            let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
            let at = |x: u32, y: u32| u32::from(mask[(y.min(h - 1) * w + x.min(w - 1)) as usize]);
            let next: Vec<u8> = (0..nh)
                .flat_map(|y| (0..nw).map(move |x| (x, y)))
                .map(|(x, y)| ((at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1) + 2) / 4) as u8)
                .collect();
            levels.push(TipLevel { width: nw, height: nh, mask: next.clone() });
            (w, h, mask) = (nw, nh, next);
        }
        self.levels = levels;
        self
    }

    /// Version à lire pour une empreinte de `side` pixels (plus grand côté à l'écran) : la plus petite qui garde au
    /// moins un pixel de pointe par pixel du trait. 0 = pleine définition.
    pub fn level_for(&self, side: f32) -> usize {
        let mut level = 0;
        while let Some(next) = self.levels.get(level) {
            if (next.width.max(next.height) as f32) < side {
                break;
            }
            level += 1;
        }
        level
    }

    /// Couverture interpolée en `(u, v)` dans la version `level` (voir `level_for`).
    pub fn sample_level(&self, level: usize, u: f32, v: f32) -> f32 {
        match level.checked_sub(1).and_then(|i| self.levels.get(i)) {
            Some(lv) => bilinear(&lv.mask, lv.width, lv.height, u, v),
            None => self.sample(u, v),
        }
    }

    /// Couverture interpolée en `(u, v)`, coordonnées de 0 à 1 dans la pointe ; 0 en dehors.
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        bilinear(&self.mask, self.width, self.height, u, v)
    }
}

/// Lecture bilinéaire d'une carte `w × h` en `(u, v)` (0 à 1) ; 0 en dehors.
fn bilinear(mask: &[u8], width: u32, height: u32, u: f32, v: f32) -> f32 {
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return 0.0;
    }
    let (fx, fy) = ((u * width as f32 - 0.5).max(0.0), (v * height as f32 - 0.5).max(0.0));
    let (x0, y0) = ((fx as u32).min(width - 1), (fy as u32).min(height - 1));
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let at = |x: u32, y: u32| f32::from(mask[(y * width + x) as usize]);
    ((at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty) / 255.0
}

/// Pointes fournies avec le logiciel : le pack de pinceaux Krita choisi pour Lumitype (licence CC-BY-SA, voir
/// `assets/brushes/CREDITS.md`), converti en masques par `tools/krita-brushes.py`. `brushes.tsv` donne, dans
/// l'ordre, le fichier, le nom affiché et l'espacement de chacun ; les masques sont en niveaux de gris
/// (0 = rien, 255 = encre). Les ronds « Net » et « Flou » ne sont pas des pointes : ils sont calculés.
const BUILTIN_LIST: &str = include_str!("../assets/brushes/brushes.tsv");
const BUILTIN_MASKS: [&[u8]; 18] = [
    include_bytes!("../assets/brushes/01.png"),
    include_bytes!("../assets/brushes/02.png"),
    include_bytes!("../assets/brushes/03.png"),
    include_bytes!("../assets/brushes/04.png"),
    include_bytes!("../assets/brushes/05.png"),
    include_bytes!("../assets/brushes/06.png"),
    include_bytes!("../assets/brushes/07.png"),
    include_bytes!("../assets/brushes/08.png"),
    include_bytes!("../assets/brushes/09.png"),
    include_bytes!("../assets/brushes/10.png"),
    include_bytes!("../assets/brushes/11.png"),
    include_bytes!("../assets/brushes/12.png"),
    include_bytes!("../assets/brushes/13.png"),
    include_bytes!("../assets/brushes/14.png"),
    include_bytes!("../assets/brushes/15.png"),
    include_bytes!("../assets/brushes/16.png"),
    include_bytes!("../assets/brushes/17.png"),
    include_bytes!("../assets/brushes/18.png"),
];

pub fn builtin_tips() -> Vec<BrushTip> {
    BUILTIN_LIST
        .lines()
        .filter(|line| !line.trim().is_empty())
        .zip(BUILTIN_MASKS)
        .filter_map(|(line, png)| {
            let mut fields = line.split('\t');
            let (_file, name, spacing) = (fields.next()?, fields.next()?, fields.next()?.trim().parse::<f32>().ok()?);
            let mask = image::load_from_memory(png).ok()?.into_luma8();
            BrushTip::new(name.to_owned(), mask.width(), mask.height(), mask.into_raw(), spacing)
        })
        .collect()
}

/// Lit toutes les pointes d'un fichier, d'après son extension.
pub fn load_tips(bytes: &[u8], origin: &Path) -> Result<Vec<BrushTip>, EngineError> {
    let stem = origin.file_stem().and_then(|s| s.to_str()).unwrap_or("Pinceau").to_owned();
    let extension = origin.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let tips = match extension.as_str() {
        "abr" => parse_abr(bytes, &stem),
        "gbr" => parse_gbr(bytes, &stem).into_iter().collect(),
        _ => vec![tip_from_image(bytes, &stem)?],
    };
    if tips.is_empty() {
        return Err(EngineError::InvalidOperation(format!(
            "aucune pointe exploitable dans « {} » (seuls les pinceaux dessinés d'un fichier ABR sont repris, pas les pinceaux ronds paramétriques)",
            origin.display()
        )));
    }
    Ok(tips)
}

/// Image ordinaire : sa transparence sert de pointe ; si elle est opaque, c'est sa noirceur (noir = encre, blanc = rien).
fn tip_from_image(bytes: &[u8], name: &str) -> Result<BrushTip, EngineError> {
    let image = image::load_from_memory(bytes).map_err(|e| EngineError::Corrupt(format!("image de pinceau illisible : {e}")))?.into_rgba8();
    let (width, height) = image.dimensions();
    let opaque = image.pixels().all(|px| px.0[3] == 255);
    let mask = image
        .pixels()
        .map(|px| {
            let [r, g, b, a] = px.0;
            if opaque { 255 - ((u32::from(r) * 30 + u32::from(g) * 59 + u32::from(b) * 11) / 100) as u8 } else { a }
        })
        .collect();
    BrushTip::new(name.to_owned(), width, height, mask, 0.25).ok_or_else(|| EngineError::InvalidOperation("cette image ne contient aucune forme de pinceau".into()))
}

/// Lecteur grand-boutiste qui ne panique jamais : toute lecture hors limites rend `None`.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let slice = self.bytes.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(slice)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.take(n).map(|_| ())
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// Pixels d'une pointe ABR : bruts, ou compressés ligne par ligne en PackBits (comme dans un PSD).
fn abr_pixels(reader: &mut Reader, width: u32, height: u32, depth: u16, compressed: bool) -> Option<Vec<u8>> {
    let bytes_per_sample = usize::from(depth / 8).max(1);
    let (w, h) = (width as usize, height as usize);
    let row_bytes = w * bytes_per_sample;
    let mut raw = Vec::with_capacity(row_bytes * h);
    if compressed {
        let lengths: Vec<usize> = (0..h).map(|_| reader.u16().map(usize::from)).collect::<Option<_>>()?;
        for length in lengths {
            let mut line = Reader { bytes: reader.take(length)?, at: 0 };
            let start = raw.len();
            while raw.len() - start < row_bytes {
                let control = line.u8()? as i8;
                if control >= 0 {
                    raw.extend_from_slice(line.take(control as usize + 1)?);
                } else if control != -128 {
                    let value = line.u8()?;
                    // Répétition : 1 - n fois (n négatif). La conversion vient APRÈS la soustraction.
                    raw.extend(std::iter::repeat_n(value, (1 - isize::from(control)) as usize));
                }
            }
            raw.truncate(start + row_bytes);
        }
    } else {
        raw.extend_from_slice(reader.take(row_bytes * h)?);
    }
    // 16 bits : l'octet de poids fort suffit.
    Some(raw.chunks_exact(bytes_per_sample).map(|sample| sample[0]).collect())
}

/// Fichier de pinceaux Photoshop. Versions 1 et 2 : liste de pinceaux ; versions 6 et suivantes :
/// blocs `8BIM`, les pointes étant dans le bloc `samp`. Une pointe illisible est ignorée, pas le fichier.
fn parse_abr(bytes: &[u8], stem: &str) -> Vec<BrushTip> {
    let mut reader = Reader { bytes, at: 0 };
    let Some(version) = reader.u16() else { return Vec::new() };
    let mut tips = Vec::new();
    let push = |tips: &mut Vec<BrushTip>, name: Option<String>, width, height, mask, spacing| {
        let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("{stem} {}", tips.len() + 1));
        tips.extend(BrushTip::new(name, width, height, mask, spacing));
    };
    match version {
        1 | 2 => {
            let count = reader.u16().unwrap_or(0);
            for _ in 0..count {
                let (Some(kind), Some(size)) = (reader.u16(), reader.u32()) else { break };
                let next = reader.at + size as usize;
                if kind == 2 {
                    let mut read = || -> Option<()> {
                        reader.skip(4)?; // divers
                        let spacing = f32::from(reader.u16()?) / 100.0;
                        let name = if version == 2 {
                            let length = reader.u32()? as usize;
                            let units: Vec<u16> = (0..length).map(|_| reader.u16()).collect::<Option<_>>()?;
                            Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned())
                        } else {
                            None
                        };
                        reader.skip(1 + 8)?; // antialiasing, bornes courtes
                        let (top, left, bottom, right) = (reader.u32()?, reader.u32()?, reader.u32()?, reader.u32()?);
                        let depth = reader.u16()?;
                        let compressed = reader.u8()? != 0;
                        let (width, height) = (right.checked_sub(left)?, bottom.checked_sub(top)?);
                        if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
                            return None;
                        }
                        let mask = abr_pixels(&mut reader, width, height, depth, compressed)?;
                        push(&mut tips, name, width, height, mask, spacing);
                        Some(())
                    };
                    let _ = read();
                }
                reader.at = next.min(bytes.len());
            }
        }
        6..=10 => {
            let subversion = reader.u16().unwrap_or(0);
            while let (Some(signature), Some(key), Some(length)) = (reader.take(4), reader.take(4), reader.u32()) {
                let end = (reader.at + length as usize).min(bytes.len());
                if signature == b"8BIM" && key == b"samp" {
                    while reader.at + 4 <= end {
                        let Some(size) = reader.u32() else { break };
                        let next = reader.at + ((size as usize + 3) & !3); // chaque pinceau est aligné sur 4 octets
                        let mut read = || -> Option<()> {
                            // Identifiant du pinceau (chaîne Pascal de 37 octets), puis un bloc de taille fixe selon la sous-version.
                            let start = reader.at;
                            let id_length = reader.u8()? as usize;
                            let uid = String::from_utf8_lossy(reader.take(id_length.min(36))?).into_owned();
                            reader.at = start + if subversion == 1 { 47 } else { 301 };
                            let (top, left, bottom, right) = (reader.u32()?, reader.u32()?, reader.u32()?, reader.u32()?);
                            let depth = reader.u16()?;
                            let compressed = reader.u8()? != 0;
                            let (width, height) = (right.checked_sub(left)?, bottom.checked_sub(top)?);
                            if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
                                return None;
                            }
                            let mask = abr_pixels(&mut reader, width, height, depth, compressed)?;
                            push(&mut tips, None, width, height, mask, 0.25);
                            if let Some(tip) = tips.last_mut() {
                                tip.uid = uid;
                            }
                            Some(())
                        };
                        let _ = read();
                        reader.at = next.min(end);
                    }
                }
                reader.at = end;
            }
        }
        _ => {}
    }
    tips
}

/// Mode de combinaison du pinceau double ou de la texture avec la pointe (modes de Photoshop).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TipBlend {
    #[default]
    Multiply,
    Darken,
    Lighten,
    Overlay,
    ColorDodge,
    ColorBurn,
    LinearBurn,
    HardMix,
    Subtract,
    Height,
    LinearHeight,
    Screen,
}

impl TipBlend {
    fn from_code(code: &str) -> Self {
        match code {
            "Drkn" => Self::Darken,
            "Lghn" => Self::Lighten,
            "Ovrl" => Self::Overlay,
            "CDdg" => Self::ColorDodge,
            "CBrn" => Self::ColorBurn,
            "LnBr" | "linearBurn" => Self::LinearBurn,
            "HrdM" => Self::HardMix,
            "Sbtr" => Self::Subtract,
            "height" => Self::Height,
            "linearHeight" => Self::LinearHeight,
            "Scrn" => Self::Screen,
            _ => Self::Multiply,
        }
    }

    /// Couverture `base` (la pointe) combinée à `other` (pinceau double ou texture), toutes deux de 0 à 1.
    pub fn apply(self, base: f32, other: f32) -> f32 {
        let (a, b) = (base.clamp(0.0, 1.0), other.clamp(0.0, 1.0));
        let out = match self {
            Self::Multiply => a * b,
            Self::Darken => a.min(b),
            // Modes « éclaircissants » : limités à la pointe, sans quoi le pinceau double peindrait hors d'elle.
            Self::Lighten => if a > 0.0 { a.max(b) } else { 0.0 },
            Self::Screen => if a > 0.0 { 1.0 - (1.0 - a) * (1.0 - b) } else { 0.0 },
            Self::Overlay => if a < 0.5 { 2.0 * a * b } else { 1.0 - 2.0 * (1.0 - a) * (1.0 - b) },
            Self::ColorDodge => if b >= 1.0 { if a > 0.0 { 1.0 } else { 0.0 } } else { (a / (1.0 - b)).min(1.0) },
            Self::ColorBurn => if b <= 0.0 { 0.0 } else { (1.0 - (1.0 - a) / b).max(0.0) },
            Self::LinearBurn => (a + b - 1.0).max(0.0),
            Self::Subtract => (a - (1.0 - b)).max(0.0),
            Self::HardMix => if a + b > 1.0 { a } else { 0.0 },
            Self::Height | Self::LinearHeight => ((a - (1.0 - b)) * 2.0).clamp(0.0, 1.0).min(a),
        };
        out.clamp(0.0, 1.0)
    }
}

/// Pinceau double (Photoshop) : une seconde pointe tamponnée le long du même trait, qui découpe la première.
#[derive(Clone, Debug, PartialEq)]
pub struct DualTip {
    pub tip_uid: String,
    /// Diamètre en pixels, à la taille de référence du préréglage (il suit la taille du pinceau en proportion).
    pub diameter: f32,
    pub spacing: f32,
    /// Dispersion en fraction du diamètre ; `both_axes` : dans toutes les directions, sinon perpendiculaire au trait.
    pub scatter: f32,
    pub both_axes: bool,
    pub count: u32,
    pub mode: TipBlend,
    /// Retournement aléatoire de chaque empreinte.
    pub flip: bool,
}

/// Texture (Photoshop) : un motif fixé à la toile, qui module la couverture du trait.
#[derive(Clone, Debug, PartialEq)]
pub struct TipTexture {
    pub pattern_uid: String,
    /// Échelle du motif (1 = taille réelle).
    pub scale: f32,
    /// Profondeur (0 à 1).
    pub depth: f32,
    pub mode: TipBlend,
    pub invert: bool,
    /// Luminosité (-150 à 150) et contraste (-50 à 100) du motif, comme dans Photoshop.
    pub brightness: f32,
    pub contrast: f32,
}

/// Forme d'un préréglage : une pointe dessinée du fichier, ou un rond paramétrique.
#[derive(Clone, Debug, PartialEq)]
pub enum PresetShape {
    Sampled(String),
    Round { hardness: f32 },
}

/// Préréglage d'un fichier ABR : la forme et tous les réglages qui font le rendu du pinceau dans Photoshop.
#[derive(Clone, Debug, PartialEq)]
pub struct BrushPreset {
    pub name: String,
    pub shape: PresetShape,
    pub diameter: f32,
    /// Espacement en fraction du diamètre.
    pub spacing: f32,
    /// Angle (degrés) et rondeur (0 à 1) de la pointe.
    pub angle: f32,
    pub roundness: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    pub size_pressure: bool,
    /// Diamètre minimal sous faible pression (fraction du diamètre).
    pub min_diameter: f32,
    pub size_jitter: f32,
    /// Variation d'angle (fraction d'un tour) ; `follow_direction` : la pointe suit le sens du trait.
    pub angle_jitter: f32,
    pub follow_direction: bool,
    pub roundness_jitter: f32,
    pub min_roundness: f32,
    /// Dispersion (fraction du diamètre), dans les deux axes ou perpendiculaire au trait, et nombre d'empreintes.
    pub scatter: f32,
    pub scatter_both: bool,
    pub count: u32,
    pub opacity_pressure: bool,
    pub opacity_jitter: f32,
    pub flow_pressure: bool,
    pub flow_jitter: f32,
    /// Opacité et flux enregistrés avec l'outil (0 à 1).
    pub opacity: f32,
    pub flow: f32,
    pub dual: Option<DualTip>,
    pub texture: Option<TipTexture>,
}

/// Motif d'un fichier ABR (bloc `patt`), en niveaux de gris.
#[derive(Clone, Debug, PartialEq)]
pub struct BrushPattern {
    pub name: String,
    pub uid: String,
    pub width: u32,
    pub height: u32,
    pub gray: Vec<u8>,
}

impl BrushPattern {
    /// Valeur du motif (0 à 1) au point `(x, y)` de la toile, motif répété et mis à l'échelle `scale`.
    pub fn at(&self, x: f32, y: f32, scale: f32) -> f32 {
        let scale = scale.max(0.01);
        let (px, py) = ((x / scale).floor() as i64, (y / scale).floor() as i64);
        let (w, h) = (i64::from(self.width), i64::from(self.height));
        f32::from(self.gray[(py.rem_euclid(h) * w + px.rem_euclid(w)) as usize]) / 255.0
    }
}

/// Contenu d'un fichier de pinceaux : ses pointes, ses préréglages (fichiers ABR récents) et ses motifs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrushPack {
    pub tips: Vec<BrushTip>,
    pub presets: Vec<BrushPreset>,
    pub patterns: Vec<BrushPattern>,
}

/// Lit un fichier de pinceaux complet, d'après son extension. Les pointes d'un ABR prennent le nom du premier
/// préréglage qui les utilise.
pub fn load_pack(bytes: &[u8], origin: &Path) -> Result<BrushPack, EngineError> {
    let extension = origin.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let mut tips = load_tips(bytes, origin).or_else(|error| if extension == "abr" { Ok(Vec::new()) } else { Err(error) })?;
    if extension != "abr" {
        return Ok(BrushPack { tips, ..BrushPack::default() });
    }
    let presets = abr_section(bytes, b"desc").and_then(parse_presets).unwrap_or_default();
    let patterns = abr_section(bytes, b"patt").map(parse_patterns).unwrap_or_default();
    for tip in &mut tips {
        if let Some(preset) = presets.iter().find(|p| matches!(&p.shape, PresetShape::Sampled(uid) if *uid == tip.uid)) {
            tip.name.clone_from(&preset.name);
        }
    }
    if tips.is_empty() && presets.is_empty() {
        return Err(EngineError::InvalidOperation(format!("aucun pinceau exploitable dans « {} »", origin.display())));
    }
    Ok(BrushPack { tips, presets, patterns })
}

/// Bloc `8BIM` d'un fichier ABR de version 6 et suivantes.
fn abr_section<'a>(bytes: &'a [u8], wanted: &[u8; 4]) -> Option<&'a [u8]> {
    let mut reader = Reader { bytes, at: 0 };
    if !(6..=10).contains(&reader.u16()?) {
        return None;
    }
    reader.skip(2)?;
    while let (Some(signature), Some(key), Some(length)) = (reader.take(4), reader.take(4), reader.u32()) {
        let end = (reader.at + length as usize).min(bytes.len());
        if signature == b"8BIM" && key == wanted {
            return bytes.get(reader.at..end);
        }
        reader.at = end;
    }
    None
}

/// Valeur d'un descripteur Photoshop (« ActionDescriptor »).
#[derive(Clone, Debug)]
enum Value {
    Object(Descriptor),
    List(Vec<Value>),
    Number(f64),
    Text(String),
    Enum(String),
    Bool(bool),
    Other,
}

#[derive(Clone, Debug, Default)]
struct Descriptor {
    items: Vec<(String, Value)>,
}

impl Descriptor {
    fn get(&self, key: &str) -> Option<&Value> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    fn number(&self, key: &str) -> Option<f32> {
        match self.get(key)? {
            Value::Number(v) => Some(*v as f32),
            _ => None,
        }
    }
    fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Some(Value::Bool(true)))
    }
    fn object(&self, key: &str) -> Option<&Descriptor> {
        match self.get(key)? {
            Value::Object(d) => Some(d),
            _ => None,
        }
    }
    fn text(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }
    fn enumeration(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Enum(e) => Some(e),
            _ => None,
        }
    }
    /// Commande d'une dynamique (`brVr`) : 2 = pression du stylet, 6 et 7 = direction ; et sa variation (0 à 1).
    fn control(&self, key: &str) -> (i64, f32, f32) {
        self.object(key).map_or((0, 0.0, 0.0), |d| {
            (d.number("bVTy").unwrap_or(0.0) as i64, d.number("jitter").unwrap_or(0.0) / 100.0, d.number("Mnm ").unwrap_or(0.0) / 100.0)
        })
    }
}

impl Reader<'_> {
    fn unicode(&mut self) -> Option<String> {
        let length = self.u32()? as usize;
        if length > 1 << 20 {
            return None;
        }
        let units: Vec<u16> = (0..length).map(|_| self.u16()).collect::<Option<_>>()?;
        Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned())
    }
    fn key(&mut self) -> Option<String> {
        let length = self.u32()? as usize;
        let length = if length == 0 { 4 } else { length };
        if length > 1 << 16 {
            return None;
        }
        Some(String::from_utf8_lossy(self.take(length)?).into_owned())
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }
    fn descriptor(&mut self, depth: u32) -> Option<Descriptor> {
        if depth > 16 {
            return None;
        }
        self.unicode()?;
        self.key()?;
        let count = self.u32()?;
        let mut items = Vec::new();
        for _ in 0..count.min(4096) {
            let key = self.key()?;
            let value = self.value(depth)?;
            items.push((key, value));
        }
        Some(Descriptor { items })
    }
    fn value(&mut self, depth: u32) -> Option<Value> {
        let kind = self.take(4)?;
        Some(match kind {
            b"Objc" | b"GlbO" => Value::Object(self.descriptor(depth + 1)?),
            b"VlLs" => {
                let count = self.u32()?;
                Value::List((0..count.min(1 << 16)).map(|_| self.value(depth + 1)).collect::<Option<_>>()?)
            }
            b"doub" => Value::Number(self.f64()?),
            b"UntF" => {
                self.skip(4)?;
                Value::Number(self.f64()?)
            }
            b"long" => Value::Number(f64::from(i32::from_be_bytes(self.take(4)?.try_into().ok()?))),
            b"comp" => Value::Number(i64::from_be_bytes(self.take(8)?.try_into().ok()?) as f64),
            b"bool" => Value::Bool(self.u8()? != 0),
            b"TEXT" => Value::Text(self.unicode()?),
            b"enum" => {
                self.key()?;
                Value::Enum(self.key()?)
            }
            b"type" | b"GlbC" => {
                self.unicode()?;
                self.key()?;
                Value::Other
            }
            b"tdta" | b"alis" | b"Pth " => {
                let length = self.u32()? as usize;
                self.skip(length)?;
                Value::Other
            }
            _ => return None,
        })
    }
}

/// Préréglages du bloc `desc` : une liste « Brsh » de descripteurs.
fn parse_presets(section: &[u8]) -> Option<Vec<BrushPreset>> {
    let mut reader = Reader { bytes: section, at: 0 };
    reader.skip(4)?; // version du descripteur (16)
    let root = reader.descriptor(0)?;
    let Some(Value::List(list)) = root.get("Brsh") else { return None };
    Some(list.iter().filter_map(|v| if let Value::Object(d) = v { preset(d) } else { None }).collect())
}

fn preset(d: &Descriptor) -> Option<BrushPreset> {
    let tip = d.object("Brsh")?;
    let shape = match tip.text("sampledData") {
        Some(uid) => PresetShape::Sampled(uid.to_owned()),
        None => PresetShape::Round { hardness: tip.number("Hrdn").unwrap_or(100.0) / 100.0 },
    };
    let percent = |desc: &Descriptor, key: &str, default: f32| desc.number(key).map_or(default, |v| v / 100.0);
    let spacing = if tip.get("Intr").is_none() || tip.flag("Intr") { percent(tip, "Spcn", 0.25) } else { 0.1 };
    let use_tip = d.flag("useTipDynamics");
    let paint = d.flag("usePaintDynamics");
    let (size_control, size_jitter, _) = if use_tip { d.control("szVr") } else { (0, 0.0, 0.0) };
    let (angle_control, angle_jitter, _) = if use_tip { d.control("angleDynamics") } else { (0, 0.0, 0.0) };
    let (_, roundness_jitter, _) = if use_tip { d.control("roundnessDynamics") } else { (0, 0.0, 0.0) };
    let (opacity_control, opacity_jitter, _) = if paint { d.control("opVr") } else { (0, 0.0, 0.0) };
    let (flow_control, flow_jitter, _) = if paint { d.control("prVr") } else { (0, 0.0, 0.0) };
    let scatter = d.flag("useScatter");
    let options = d.object("toolOptions");
    let dual = d.object("dualBrush").filter(|b| b.flag("useDualBrush")).and_then(|b| {
        let second = b.object("Brsh")?;
        Some(DualTip {
            tip_uid: second.text("sampledData")?.to_owned(),
            diameter: second.number("Dmtr").unwrap_or(25.0),
            spacing: percent(second, "Spcn", 0.25).max(0.01),
            // La quantité de dispersion est la « variation » de `scatterDynamics` (jusqu'à 1000 %) ; `Spcn` vaut toujours 100.
            scatter: if b.flag("useScatter") { b.control("scatterDynamics").1 } else { 0.0 },
            both_axes: b.flag("bothAxes"),
            count: b.number("Cnt ").unwrap_or(1.0).clamp(1.0, 16.0) as u32,
            mode: TipBlend::from_code(b.enumeration("BlnM").unwrap_or("Mltp")),
            flip: b.flag("Flip"),
        })
    });
    let texture = d.flag("useTexture").then(|| d.object("Txtr")).flatten().and_then(|pattern| {
        Some(TipTexture {
            pattern_uid: pattern.text("Idnt")?.to_owned(),
            scale: percent(d, "textureScale", 1.0).max(0.01),
            depth: percent(d, "textureDepth", 1.0).clamp(0.0, 1.0),
            mode: TipBlend::from_code(d.enumeration("textureBlendMode").unwrap_or("Mltp")),
            invert: d.flag("InvT"),
            brightness: d.number("textureBrightness").unwrap_or(0.0),
            contrast: d.number("textureContrast").unwrap_or(0.0),
        })
    });
    Some(BrushPreset {
        name: d.text("Nm  ").unwrap_or("Pinceau").to_owned(),
        shape,
        diameter: tip.number("Dmtr").unwrap_or(30.0).max(1.0),
        spacing: spacing.clamp(0.01, 10.0),
        angle: tip.number("Angl").unwrap_or(0.0),
        roundness: percent(tip, "Rndn", 1.0).clamp(0.01, 1.0),
        flip_x: tip.flag("flipX") || d.flag("flipX"),
        flip_y: tip.flag("flipY") || d.flag("flipY"),
        size_pressure: size_control == 2,
        min_diameter: if use_tip { percent(d, "minimumDiameter", 0.0) } else { 0.0 },
        size_jitter,
        angle_jitter,
        follow_direction: matches!(angle_control, 6 | 7),
        roundness_jitter,
        min_roundness: if use_tip { percent(d, "minimumRoundness", 0.0) } else { 0.0 },
        scatter: if scatter { d.control("scatterDynamics").1 } else { 0.0 },
        scatter_both: d.flag("bothAxes"),
        count: if scatter { d.number("Cnt ").unwrap_or(1.0).clamp(1.0, 16.0) as u32 } else { 1 },
        opacity_pressure: opacity_control == 2,
        opacity_jitter,
        flow_pressure: flow_control == 2,
        flow_jitter,
        opacity: options.and_then(|o| o.number("Opct")).map_or(1.0, |v| (v / 100.0).clamp(0.01, 1.0)),
        flow: options.and_then(|o| o.number("flow")).map_or(1.0, |v| (v / 100.0).clamp(0.01, 1.0)),
        dual,
        texture,
    })
}

/// Motifs du bloc `patt` (version 1 : RVB, niveaux de gris ou couleurs indexées), ramenés en niveaux de gris.
fn parse_patterns(section: &[u8]) -> Vec<BrushPattern> {
    let mut reader = Reader { bytes: section, at: 0 };
    let mut patterns = Vec::new();
    while reader.at + 4 <= section.len() {
        let Some(length) = reader.u32() else { break };
        let next = reader.at + ((length as usize + 3) & !3);
        let mut read = || -> Option<BrushPattern> {
            let (_version, mode) = (reader.u32()?, reader.u32()?);
            let (_height, _width) = (reader.u16()?, reader.u16()?);
            let name = reader.unicode()?;
            let id_length = reader.u8()? as usize;
            let uid = String::from_utf8_lossy(reader.take(id_length)?).into_owned();
            if mode == 2 {
                reader.skip(768)?; // palette
            }
            reader.skip(8)?; // version et longueur de la liste
            reader.skip(16)?; // rectangle
            let channels = reader.u32()?;
            let wanted = if mode == 3 { 3 } else { 1 };
            let mut planes: Vec<(u32, u32, Vec<u8>)> = Vec::new();
            for _ in 0..channels.min(64) {
                if planes.len() == wanted {
                    break;
                }
                if reader.u32()? == 0 {
                    continue; // canal absent
                }
                let channel_length = reader.u32()? as usize;
                let channel_end = reader.at + channel_length;
                let _depth = reader.u32()?;
                let (top, left, bottom, right) = (reader.u32()?, reader.u32()?, reader.u32()?, reader.u32()?);
                let depth = reader.u16()?;
                let compressed = reader.u8()? != 0;
                let (w, h) = (right.checked_sub(left)?, bottom.checked_sub(top)?);
                if w == 0 || h == 0 || w > 8192 || h > 8192 {
                    return None;
                }
                planes.push((w, h, abr_pixels(&mut reader, w, h, depth, compressed)?));
                reader.at = channel_end;
            }
            let (w, h) = planes.first().map(|(w, h, _)| (*w, *h))?;
            if planes.iter().any(|(pw, ph, _)| (*pw, *ph) != (w, h)) {
                return None;
            }
            let gray = (0..(w * h) as usize).map(|i| (planes.iter().map(|(_, _, p)| u32::from(p[i])).sum::<u32>() / planes.len() as u32) as u8).collect();
            Some(BrushPattern { name, uid, width: w, height: h, gray })
        };
        if let Some(pattern) = read() {
            patterns.push(pattern);
        }
        reader.at = next.min(section.len());
    }
    patterns
}

/// Pinceau GIMP (.gbr, version 2) : niveaux de gris (0 = rien… 255 = encre) ou RGBA (l'alpha sert de pointe).
fn parse_gbr(bytes: &[u8], stem: &str) -> Option<BrushTip> {
    let mut reader = Reader { bytes, at: 0 };
    let header = reader.u32()? as usize;
    let version = reader.u32()?;
    let (width, height, channels) = (reader.u32()?, reader.u32()?, reader.u32()?);
    if version < 2 || reader.take(4)? != b"GIMP" || width == 0 || height == 0 || width > 16_384 || height > 16_384 {
        return None;
    }
    let spacing = reader.u32()? as f32 / 100.0;
    let name = String::from_utf8_lossy(bytes.get(reader.at..header)?).trim_end_matches('\0').trim().to_owned();
    reader.at = header;
    let data = reader.take((width * height * channels) as usize)?;
    let mask = match channels {
        1 => data.to_vec(),
        4 => data.chunks_exact(4).map(|px| px[3]).collect(),
        _ => return None,
    };
    BrushTip::new(if name.is_empty() { stem.to_owned() } else { name }, width, height, mask, spacing)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pointe 4 x 3 : un dégradé, facile à reconnaître.
    fn pixels() -> Vec<u8> {
        (0..12).map(|i| (i * 20 + 10) as u8).collect()
    }

    fn packbits_row(row: &[u8]) -> Vec<u8> {
        let mut out = vec![(row.len() - 1) as u8]; // un seul paquet littéral
        out.extend_from_slice(row);
        out
    }

    #[test]
    fn reads_sampled_brushes_from_abr_version_6_raw_and_compressed() {
        for compressed in [false, true] {
            let mut brush = vec![0u8; 47]; // identifiant + bloc fixe (sous-version 1)
            for v in [0u32, 0, 3, 4] {
                brush.extend_from_slice(&v.to_be_bytes()); // haut, gauche, bas, droite
            }
            brush.extend_from_slice(&8u16.to_be_bytes());
            brush.push(u8::from(compressed));
            if compressed {
                let rows: Vec<Vec<u8>> = pixels().chunks(4).map(packbits_row).collect();
                rows.iter().for_each(|r| brush.extend_from_slice(&(r.len() as u16).to_be_bytes()));
                rows.iter().for_each(|r| brush.extend_from_slice(r));
            } else {
                brush.extend_from_slice(&pixels());
            }
            let mut samp = (brush.len() as u32).to_be_bytes().to_vec();
            samp.extend_from_slice(&brush);
            samp.resize(samp.len().div_ceil(4) * 4, 0);

            let mut file = vec![0, 6, 0, 1];
            file.extend_from_slice(b"8BIMdesc");
            file.extend_from_slice(&4u32.to_be_bytes());
            file.extend_from_slice(&[1, 2, 3, 4]); // un bloc inconnu, à sauter
            file.extend_from_slice(b"8BIMsamp");
            file.extend_from_slice(&(samp.len() as u32).to_be_bytes());
            file.extend_from_slice(&samp);

            let tips = load_tips(&file, Path::new("Craie.abr")).expect("pinceaux");
            assert_eq!(tips.len(), 1);
            assert_eq!((tips[0].width, tips[0].height, tips[0].name.as_str()), (4, 3, "Craie 1"));
            assert_eq!(tips[0].mask, pixels(), "compressé : {compressed}");
        }
    }

    #[test]
    fn reads_abr_version_2_with_its_name_and_spacing() {
        let mut body = vec![0u8; 4];
        body.extend_from_slice(&40u16.to_be_bytes()); // espacement 40 %
        let name: Vec<u16> = "Fusain\0".encode_utf16().collect();
        body.extend_from_slice(&(name.len() as u32).to_be_bytes());
        name.iter().for_each(|u| body.extend_from_slice(&u.to_be_bytes()));
        body.extend_from_slice(&[0; 9]);
        for v in [0u32, 0, 3, 4] {
            body.extend_from_slice(&v.to_be_bytes());
        }
        body.extend_from_slice(&8u16.to_be_bytes());
        body.push(0);
        body.extend_from_slice(&pixels());
        let mut file = vec![0, 2, 0, 2];
        // Un pinceau « calculé » (type 1), ignoré, puis le pinceau dessiné.
        file.extend_from_slice(&1u16.to_be_bytes());
        file.extend_from_slice(&6u32.to_be_bytes());
        file.extend_from_slice(&[0; 6]);
        file.extend_from_slice(&2u16.to_be_bytes());
        file.extend_from_slice(&(body.len() as u32).to_be_bytes());
        file.extend_from_slice(&body);

        let tips = load_tips(&file, Path::new("x.abr")).expect("pinceaux");
        assert_eq!(tips.len(), 1);
        assert_eq!((tips[0].name.as_str(), tips[0].spacing, tips[0].mask.clone()), ("Fusain", 0.4, pixels()));
    }

    #[test]
    fn reads_gimp_brushes_and_plain_images_and_never_panics_on_garbage() {
        let name = b"Eponge\0";
        let header = 28 + name.len() as u32;
        let mut gbr = Vec::new();
        for v in [header, 2, 4, 3, 1] {
            gbr.extend_from_slice(&v.to_be_bytes());
        }
        gbr.extend_from_slice(b"GIMP");
        gbr.extend_from_slice(&15u32.to_be_bytes());
        gbr.extend_from_slice(name);
        gbr.extend_from_slice(&pixels());
        let tip = &load_tips(&gbr, Path::new("e.gbr")).expect("gbr")[0];
        assert_eq!((tip.name.as_str(), tip.width, tip.height, tip.spacing), ("Eponge", 4, 3, 0.15));

        // Image opaque : le noir devient de l'encre, le blanc rien.
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_fn(2, 1, |x, _| image::Rgba(if x == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] })).write_to(&mut png, image::ImageFormat::Png).expect("png");
        assert_eq!(load_tips(png.get_ref(), Path::new("tampon.png")).expect("image")[0].mask, vec![255, 0]);

        for garbage in [&b""[..], &[0, 6][..], &[0, 6, 0, 1, b'8', b'B', b'I', b'M', b's', b'a', b'm', b'p', 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 9][..], &[0, 2, 0xFF, 0xFF, 0, 2, 0xFF][..]] {
            assert!(load_tips(garbage, Path::new("casse.abr")).is_err());
        }
    }

    #[test]
    fn builtin_tips_are_the_krita_pack_all_valid() {
        let tips = builtin_tips();
        assert_eq!(tips.len(), BUILTIN_MASKS.len(), "chaque masque a sa ligne dans brushes.tsv et se décode");
        assert_eq!((tips[0].name.as_str(), tips[8].name.as_str(), tips[17].name.as_str()), ("Marqueur rond", "Encre G-pen", "Gomme douce"));
        for tip in &tips {
            assert_eq!(tip.mask.len(), (tip.width * tip.height) as usize);
            assert!(tip.mask.iter().any(|v| *v > 128), "{} : la pointe encre", tip.name);
            assert_eq!(tip.mask[0], 0, "{} : les coins sont vides", tip.name);
            assert!((0.01..=1.0).contains(&tip.spacing), "{} : espacement {}", tip.name, tip.spacing);
        }
        // Le marqueur est un rond plein : encré au centre, vide hors du cercle.
        assert!(tips[0].sample(0.5, 0.5) > 0.99 && tips[0].sample(0.02, 0.02) < 0.01);
    }

    #[test]
    fn sampling_is_interpolated_and_empty_outside() {
        let tip = BrushTip::new("t".into(), 2, 1, vec![0, 255], 0.25).expect("pointe");
        assert_eq!((tip.sample(-0.1, 0.5), tip.sample(1.0, 0.5)), (0.0, 0.0));
        assert!((tip.sample(0.5, 0.5) - 0.5).abs() < 0.01 && tip.sample(0.9, 0.5) > 0.95);
        let huge = BrushTip::new("g".into(), 4096, 2, vec![200; 8192], 0.25).expect("pointe");
        assert!(huge.width <= MAX_TIP_SIDE && huge.mask.iter().all(|v| *v == 200), "réduite sans changer sa valeur");
    }
    /// Descripteur Photoshop minimal : clés et valeurs écrites comme dans un fichier ABR.
    struct Desc(Vec<u8>);
    impl Desc {
        fn new(count: u32) -> Self {
            let mut d = Desc(Vec::new());
            d.0.extend_from_slice(&0u32.to_be_bytes()); // nom vide
            d.key("null");
            d.0.extend_from_slice(&count.to_be_bytes());
            d
        }
        fn key(&mut self, k: &str) {
            if k.len() == 4 {
                self.0.extend_from_slice(&0u32.to_be_bytes());
            } else {
                self.0.extend_from_slice(&(k.len() as u32).to_be_bytes());
            }
            self.0.extend_from_slice(k.as_bytes());
        }
        fn unit(&mut self, k: &str, unit: &str, v: f64) -> &mut Self {
            self.key(k);
            self.0.extend_from_slice(b"UntF");
            self.0.extend_from_slice(unit.as_bytes());
            self.0.extend_from_slice(&v.to_be_bytes());
            self
        }
        fn text(&mut self, k: &str, v: &str) -> &mut Self {
            self.key(k);
            self.0.extend_from_slice(b"TEXT");
            let units: Vec<u16> = v.encode_utf16().collect();
            self.0.extend_from_slice(&(units.len() as u32).to_be_bytes());
            units.iter().for_each(|u| self.0.extend_from_slice(&u.to_be_bytes()));
            self
        }
        fn flag(&mut self, k: &str, v: bool) -> &mut Self {
            self.key(k);
            self.0.extend_from_slice(b"bool");
            self.0.push(u8::from(v));
            self
        }
        fn enumeration(&mut self, k: &str, v: &str) -> &mut Self {
            self.key(k);
            self.0.extend_from_slice(b"enum");
            self.key("BlnM");
            self.key(v);
            self
        }
        fn object(&mut self, k: &str, inner: &Desc) -> &mut Self {
            self.key(k);
            self.0.extend_from_slice(b"Objc");
            self.0.extend_from_slice(&inner.0);
            self
        }
    }

    #[test]
    fn reads_photoshop_presets_spacing_angle_roundness_dual_brush_and_texture() {
        let mut tip = Desc::new(5);
        tip.unit("Dmtr", "#Pxl", 60.0).unit("Spcn", "#Prc", 3.0).unit("Angl", "#Ang", -105.0).unit("Rndn", "#Prc", 70.0).text("sampledData", "pointe-a");
        let mut second = Desc::new(3);
        second.unit("Dmtr", "#Pxl", 120.0).unit("Spcn", "#Prc", 6.0).text("sampledData", "pointe-b");
        let mut dual = Desc::new(4);
        dual.flag("useDualBrush", true).object("Brsh", &second).enumeration("BlnM", "CBrn").flag("Flip", true);
        let mut pattern = Desc::new(1);
        pattern.text("Idnt", "motif-1");
        let mut preset = Desc::new(7);
        preset.text("Nm  ", "Encrage").object("Brsh", &tip).object("dualBrush", &dual).flag("useTexture", true).object("Txtr", &pattern).unit("textureDepth", "#Prc", 50.0).flag("InvT", true);
        // Racine : une liste « Brsh » d'un préréglage, plus un pinceau rond calculé.
        let mut round_tip = Desc::new(2);
        round_tip.unit("Dmtr", "#Pxl", 15.0).unit("Hrdn", "#Prc", 0.0);
        let mut round = Desc::new(2);
        round.text("Nm  ", "Rond doux").object("Brsh", &round_tip);
        let mut root = Desc::new(1);
        root.key("Brsh");
        root.0.extend_from_slice(b"VlLs");
        root.0.extend_from_slice(&2u32.to_be_bytes());
        for d in [&preset, &round] {
            root.0.extend_from_slice(b"Objc");
            root.0.extend_from_slice(&d.0);
        }
        let mut section = 16u32.to_be_bytes().to_vec();
        section.extend_from_slice(&root.0);

        let presets = parse_presets(&section).expect("préréglages");
        assert_eq!(presets.len(), 2);
        let ink = &presets[0];
        assert_eq!((ink.name.as_str(), ink.shape.clone()), ("Encrage", PresetShape::Sampled("pointe-a".into())));
        assert!((ink.diameter - 60.0).abs() < 1e-3 && (ink.spacing - 0.03).abs() < 1e-4 && (ink.angle + 105.0).abs() < 1e-3 && (ink.roundness - 0.7).abs() < 1e-4);
        let dual = ink.dual.as_ref().expect("pinceau double");
        assert_eq!((dual.tip_uid.as_str(), dual.mode, dual.flip), ("pointe-b", TipBlend::ColorBurn, true));
        assert!((dual.diameter - 120.0).abs() < 1e-3 && (dual.spacing - 0.06).abs() < 1e-4);
        let texture = ink.texture.as_ref().expect("texture");
        assert_eq!((texture.pattern_uid.as_str(), texture.invert), ("motif-1", true));
        assert!((texture.depth - 0.5).abs() < 1e-4);
        assert_eq!(presets[1].shape, PresetShape::Round { hardness: 0.0 });
        assert!(presets[1].dual.is_none() && presets[1].texture.is_none());
        // Données tronquées : rien n'est lu, sans panique.
        assert!(parse_presets(&section[..section.len() / 2]).is_none());
    }

    #[test]
    fn tip_levels_halve_down_to_two_pixels_and_keep_the_average() {
        let tip = BrushTip::from_mask("t".into(), 8, 4, vec![200; 32], 0.25).expect("pointe");
        let sizes: Vec<(u32, u32)> = tip.levels.iter().map(|l| (l.width, l.height)).collect();
        assert_eq!(sizes, vec![(4, 2), (2, 1)]);
        assert!(tip.levels.iter().all(|l| l.mask.iter().all(|v| *v == 200)));
        // Grande pointe posée petite : la version réduite la plus proche ; posée grande : la pleine définition.
        assert_eq!((tip.level_for(8.0), tip.level_for(4.0), tip.level_for(1.0)), (0, 1, 2));
        assert!((tip.sample_level(1, 0.5, 0.5) - 200.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn blend_modes_never_ink_outside_the_tip() {
        for mode in [TipBlend::Multiply, TipBlend::Darken, TipBlend::Lighten, TipBlend::Overlay, TipBlend::ColorDodge, TipBlend::ColorBurn, TipBlend::LinearBurn, TipBlend::HardMix, TipBlend::Subtract, TipBlend::Height, TipBlend::LinearHeight, TipBlend::Screen] {
            assert_eq!(mode.apply(0.0, 1.0), 0.0, "{mode:?}");
            let v = mode.apply(0.8, 0.6);
            assert!((0.0..=1.0).contains(&v), "{mode:?} : {v}");
        }
        assert!((TipBlend::Multiply.apply(0.5, 0.5) - 0.25).abs() < 1e-6);
    }

}
