//! Développement RAW paramétré (fenêtre « Développement RAW »).
//!
//! Le capteur est décodé **une fois** ([`RawSession::open`]) : dématriçage, rognage et orientation, mais ni balance
//! des blancs ni matrice couleur. On garde ainsi la lumière **linéaire dans l'espace de l'appareil**, en `f32`, à pleine
//! taille et en version réduite (~1600 px) pour l'aperçu. Chaque développement ([`RawSession::develop`]) repart de là :
//!
//! 1. balance des blancs (coefficients par couche de l'appareil), matrice appareil -> sRGB linéaire — exactement le
//!    calcul de `rawler`, si bien que sans réglage le résultat est celui du décodage habituel (`io::decode_raw`) ;
//! 2. vignetage (gain radial) et exposition, en linéaire, AVANT toute coupure : une valeur au-delà de 1 (haute lumière
//!    « brûlée » à l'exposition de la prise de vue) se récupère en baissant l'exposition ;
//! 3. compression des hautes lumières (celle de `rawler` : la teinte est gardée), gamma sRGB ;
//! 4. réduction du bruit : filtre guidé sur la luminance (lisse les aplats, garde les bords), flou de la chrominance ;
//! 5. tonalité et présence : le réglage `Develop` d'`adjust.rs`, tel quel (mêmes curseurs que le calque Développement) ;
//! 6. netteté : masque flou sur la luminance.
//!
//! La température et la teinte suivent un modèle physique : l'illuminant (lieu de Planck, décalé perpendiculairement
//! pour la teinte) est ramené dans l'espace de l'appareil par sa matrice XYZ -> appareil. Les coefficients sont pris
//! **relativement** à ceux de la prise de vue : à la température « telle quelle » on retombe exactement sur eux.

use std::path::Path;
use std::sync::Arc;

use rawler::imgop::develop::{Intermediate, ProcessingStep, RawDevelop};
use rawler::imgop::matrix::{multiply, normalize, pseudo_inverse, transform_1d};
use rawler::imgop::srgb::srgb_apply_gamma;
use rawler::imgop::xyz::{Illuminant, SRGB_TO_XYZ_D65};
use rawler::Orientation;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::adjust::{Adjustment, Prepared};
use crate::error::EngineError;

/// Côté long visé de l'aperçu.
pub const PREVIEW_SIDE: usize = 1600;
/// Bornes de la température (K) : celles du lieu de Planck approché ci-dessous.
pub const TEMPERATURE_MIN: f32 = 2000.0;
pub const TEMPERATURE_MAX: f32 = 25000.0;
/// Bornes de la teinte (vert -> magenta), comme Lightroom.
pub const TINT_MAX: f32 = 150.0;

// ------------------------------------------------------------------ paramètres

/// Origine de la balance des blancs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WhiteBalance {
    /// Coefficients enregistrés par l'appareil (`temperature`/`tint` ignorés).
    #[default]
    AsShot,
    /// Gris moyen de l'image, calculé pour chaque fichier (utile pour « Appliquer à tous les suivants »).
    Auto,
    /// `temperature` et `tint` (préréglages Lumière du jour, Tungstène… compris).
    Custom,
}

/// Réglages de développement. Tous neutres par défaut ; mêmes plages que l'interface Développement (-100 à 100 ;
/// exposition en IL de -5 à +5), sauf le détail (0 à 100) et le vignetage (-100 à 100).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RawParams {
    pub white_balance: WhiteBalance,
    pub temperature: f32,
    pub tint: f32,
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub clarity: f32,
    pub vibrance: f32,
    pub saturation: f32,
    /// Réduction du bruit de luminance et de couleur, 0 à 100.
    pub noise_luma: f32,
    pub noise_color: f32,
    /// Netteté, 0 à 100.
    pub sharpen: f32,
    /// Correction du vignetage : positif éclaircit les coins (+100 : +1 IL dans les angles), négatif les assombrit.
    pub vignette: f32,
}

impl Default for RawParams {
    fn default() -> Self {
        Self {
            white_balance: WhiteBalance::AsShot,
            temperature: 5500.0,
            tint: 0.0,
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            clarity: 0.0,
            vibrance: 0.0,
            saturation: 0.0,
            noise_luma: 0.0,
            noise_color: 0.0,
            sharpen: 0.0,
            vignette: 0.0,
        }
    }
}

/// Visualisation des zones écrêtées (Alt pendant un réglage).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipView {
    #[default]
    None,
    /// Fond noir ; les couches à 255 apparaissent (blanc : les trois).
    Highlights,
    /// Fond blanc ; les couches à 0 apparaissent (noir : les trois).
    Shadows,
}

/// Tonalité automatique (bouton « Auto ») : valeurs des curseurs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoTone {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
}

/// En-tête affiché par la fenêtre : appareil et prise de vue.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawInfo {
    /// Nom du fichier, extension comprise.
    pub file_name: String,
    /// « Sony ILCE-7M4 ».
    pub camera: String,
    pub width: u32,
    pub height: u32,
    pub iso: Option<u32>,
    /// Ouverture (f/2,8 -> 2.8).
    pub aperture: Option<f32>,
    /// Durée d'exposition en secondes.
    pub shutter: Option<f32>,
    /// Focale en mm.
    pub focal_length: Option<f32>,
    /// Température et teinte de la balance « telle quelle » (lues d'après les coefficients de l'appareil).
    pub as_shot_temperature: f32,
    pub as_shot_tint: f32,
    /// Balance « Auto » (gris moyen).
    pub auto_temperature: f32,
    pub auto_tint: f32,
    /// Capteur monochrome : pas de balance des blancs.
    pub monochrome: bool,
}

// ------------------------------------------------------------------ image linéaire

/// Image linéaire, couches entrelacées (1, 3 ou 4 couches de l'appareil).
#[derive(Clone, Debug)]
pub struct Linear {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub data: Vec<f32>,
}

impl Linear {
    fn pixel(&self, x: usize, y: usize) -> &[f32] {
        let at = (y * self.width + x) * self.channels;
        &self.data[at..at + self.channels]
    }

    /// Orientation EXIF appliquée (même convention que `io::orient_raw`).
    fn oriented(self, orientation: Orientation) -> Self {
        let (w, h, c) = (self.width, self.height, self.channels);
        let swap = matches!(orientation, Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Transpose | Orientation::Transverse);
        if matches!(orientation, Orientation::Normal | Orientation::Unknown) {
            return self;
        }
        let (ow, oh) = if swap { (h, w) } else { (w, h) };
        // Pixel source de chaque pixel de sortie.
        let source = |x: usize, y: usize| -> (usize, usize) {
            match orientation {
                Orientation::HorizontalFlip => (w - 1 - x, y),
                Orientation::Rotate180 => (w - 1 - x, h - 1 - y),
                Orientation::VerticalFlip => (x, h - 1 - y),
                Orientation::Rotate90 => (y, h - 1 - x),
                Orientation::Rotate270 => (w - 1 - y, x),
                Orientation::Transpose => (y, x),
                Orientation::Transverse => (w - 1 - y, h - 1 - x),
                Orientation::Normal | Orientation::Unknown => (x, y),
            }
        };
        let mut data = vec![0.0f32; ow * oh * c];
        data.par_chunks_exact_mut(ow * c).enumerate().for_each(|(y, row)| {
            for x in 0..ow {
                let (sx, sy) = source(x, y);
                let at = (sy * w + sx) * c;
                row[x * c..x * c + c].copy_from_slice(&self.data[at..at + c]);
            }
        });
        Self { width: ow, height: oh, channels: c, data }
    }

    /// Réduction par moyenne de blocs `factor` x `factor` (lumière linéaire : moyenne exacte).
    fn reduced(&self, factor: usize) -> Self {
        if factor <= 1 {
            return self.clone();
        }
        let (w, h, c) = (self.width, self.height, self.channels);
        let (ow, oh) = (w.div_ceil(factor), h.div_ceil(factor));
        let mut data = vec![0.0f32; ow * oh * c];
        data.par_chunks_exact_mut(ow * c).enumerate().for_each(|(oy, row)| {
            let (y0, y1) = (oy * factor, ((oy + 1) * factor).min(h));
            let mut sum = vec![0.0f32; c];
            for ox in 0..ow {
                let (x0, x1) = (ox * factor, ((ox + 1) * factor).min(w));
                sum.iter_mut().for_each(|s| *s = 0.0);
                for y in y0..y1 {
                    for x in x0..x1 {
                        for (s, v) in sum.iter_mut().zip(self.pixel(x, y)) {
                            *s += v;
                        }
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)) as f32;
                for (k, s) in sum.iter().enumerate() {
                    row[ox * c + k] = s / n;
                }
            }
        });
        Self { width: ow, height: oh, channels: c, data }
    }

    /// Rectangle `(x, y, w, h)` (déjà borné à l'image).
    fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Self {
        let c = self.channels;
        let mut data = Vec::with_capacity(w * h * c);
        for row in y..y + h {
            let at = (row * self.width + x) * c;
            data.extend_from_slice(&self.data[at..at + w * c]);
        }
        Self { width: w, height: h, channels: c, data }
    }

    fn bytes(&self) -> usize {
        self.data.len() * std::mem::size_of::<f32>()
    }
}

/// Zone de l'image pleine taille couverte par un développement partiel (zoom de la fenêtre).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Place des pixels traités dans l'image entière, pour le vignetage : le pixel `(i, j)` couvre, en pixels pleine
/// taille, `x0 + i x step` à `x0 + (i + 1) x step`.
#[derive(Clone, Copy)]
struct Frame {
    x0: f32,
    y0: f32,
    step: f32,
    width: f32,
    height: f32,
}

// ------------------------------------------------------------------ calibration

/// Ce qui relie l'appareil au sRGB : matrices et coefficients de la prise de vue.
#[derive(Clone, Debug)]
struct Calibration {
    channels: usize,
    /// XYZ (D65) -> appareil, lignes = couches de l'appareil.
    xyz2cam: [[f32; 3]; 4],
    /// Appareil -> sRGB linéaire, calculé comme `rawler` (`map_3ch_to_rgb`).
    cam2rgb: [[f32; 4]; 3],
    /// Coefficients de balance des blancs de la prise de vue.
    as_shot: [f32; 4],
    /// Température et teinte correspondant à `as_shot`.
    as_shot_tt: (f32, f32),
}

impl Calibration {
    fn new(channels: usize, xyz2cam: [[f32; 3]; 4], as_shot: [f32; 4]) -> Self {
        let rgb2cam = normalize(multiply(&xyz2cam, &SRGB_TO_XYZ_D65));
        let cam2rgb = pseudo_inverse(rgb2cam);
        let mut calibration = Self { channels, xyz2cam, cam2rgb, as_shot, as_shot_tt: (5500.0, 0.0) };
        calibration.as_shot_tt = calibration.fit(as_shot);
        calibration
    }

    /// Coefficients (inverses du neutre de l'appareil) sous l'illuminant `temperature`/`tint`, vert à 1.
    fn model(&self, temperature: f32, tint: f32) -> [f32; 4] {
        let xyz = illuminant_xyz(temperature, tint);
        let mut m = [1.0f32; 4];
        for (i, slot) in m.iter_mut().enumerate().take(self.channels.max(1)) {
            let row = self.xyz2cam[i];
            let neutral = row[0] * xyz[0] + row[1] * xyz[1] + row[2] * xyz[2];
            *slot = 1.0 / neutral.max(1e-6);
        }
        let green = m[1.min(self.channels.saturating_sub(1))];
        m.map(|v| v / green)
    }

    /// Température et teinte dont le modèle donne des coefficients proportionnels à `target` (au mieux, en écart
    /// logarithmique). Grille en mireds puis affinages successifs.
    fn fit(&self, target: [f32; 4]) -> (f32, f32) {
        let n = self.channels.clamp(1, 4);
        if n < 3 || target.iter().take(n).any(|v| !v.is_finite() || *v <= 0.0) {
            return (5500.0, 0.0);
        }
        let reference = target[1];
        let goal: Vec<f32> = target.iter().take(n).map(|v| (v / reference).ln()).collect();
        let error = |mired: f32, tint: f32| -> f32 {
            let m = self.model(1.0e6 / mired, tint);
            goal.iter().enumerate().map(|(i, g)| ((m[i] / m[1]).ln() - g).powi(2)).sum()
        };
        let (lo, hi) = (1.0e6 / TEMPERATURE_MAX, 1.0e6 / TEMPERATURE_MIN);
        let (mut best_m, mut best_t, mut best_e) = (1.0e6 / 5500.0, 0.0, f32::INFINITY);
        let (mut step_m, mut step_t) = ((hi - lo) / 96.0, 5.0);
        for i in 0..=96 {
            for j in 0..=60 {
                let (m, t) = (lo + i as f32 * step_m, -TINT_MAX + j as f32 * step_t);
                let e = error(m, t);
                if e < best_e {
                    (best_m, best_t, best_e) = (m, t, e);
                }
            }
        }
        for _ in 0..6 {
            let (cm, ct) = (best_m, best_t);
            for i in -4..=4 {
                for j in -4..=4 {
                    let m = (cm + i as f32 * step_m / 4.0).clamp(lo, hi);
                    let t = (ct + j as f32 * step_t / 4.0).clamp(-TINT_MAX, TINT_MAX);
                    let e = error(m, t);
                    if e < best_e {
                        (best_m, best_t, best_e) = (m, t, e);
                    }
                }
            }
            step_m /= 4.0;
            step_t /= 4.0;
        }
        (1.0e6 / best_m, best_t)
    }

    /// Coefficients pour une température et une teinte : ceux de la prise de vue, déplacés comme le modèle.
    fn coefficients_for(&self, temperature: f32, tint: f32) -> [f32; 4] {
        let (t0, n0) = self.as_shot_tt;
        let (m, m0) = (self.model(temperature, tint), self.model(t0, n0));
        [0, 1, 2, 3].map(|i| self.as_shot[i] * m[i] / m0[i])
    }

    /// Température et teinte qui donnent les coefficients `wanted` (pipette, Auto).
    fn temperature_for(&self, wanted: [f32; 4]) -> (f32, f32) {
        let (t0, n0) = self.as_shot_tt;
        let m0 = self.model(t0, n0);
        self.fit([0, 1, 2, 3].map(|i| wanted[i] * m0[i] / self.as_shot[i]))
    }
}

/// Lieu de Planck (Kim et al., 2002), en chromaticité xy, de 1667 à 25 000 K.
fn planck_xy(temperature: f32) -> (f64, f64) {
    let t = f64::from(temperature.clamp(1667.0, 25000.0));
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.266_123_9e9 / t3 - 0.234_358_9e6 / t2 + 0.877_695_6e3 / t + 0.179_910
    } else {
        -3.025_846_9e9 / t3 + 2.107_037_9e6 / t2 + 0.222_634_7e3 / t + 0.240_390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.106_381_4 * x3 - 1.348_110_20 * x2 + 2.185_558_32 * x - 0.202_196_83
    } else if t <= 4000.0 {
        -0.954_947_6 * x3 - 1.374_185_93 * x2 + 2.091_370_15 * x - 0.167_488_67
    } else {
        3.081_758_0 * x3 - 5.873_386_70 * x2 + 3.751_129_97 * x - 0.370_014_83
    };
    (x, y)
}

/// Blanc de l'illuminant (XYZ, Y = 1) : lieu de Planck à `temperature`, décalé perpendiculairement (plan uv CIE 1960)
/// de `tint` / 3000 (Duv) — positif vers le vert : l'image compense en tirant vers le magenta, comme Lightroom.
fn illuminant_xyz(temperature: f32, tint: f32) -> [f32; 3] {
    let uv = |(x, y): (f64, f64)| {
        let d = -2.0 * x + 12.0 * y + 3.0;
        (4.0 * x / d, 6.0 * y / d)
    };
    let (u, v) = uv(planck_xy(temperature));
    let (u1, v1) = uv(planck_xy(temperature * 1.01));
    let (du, dv) = (u1 - u, v1 - v);
    let length = du.hypot(dv).max(1e-12);
    // Normale au lieu, orientée vers le vert (v croissant).
    let (mut nu, mut nv) = (-dv / length, du / length);
    if nv < 0.0 {
        (nu, nv) = (-nu, -nv);
    }
    let duv = f64::from(tint.clamp(-TINT_MAX, TINT_MAX)) / 3000.0;
    let (u, v) = (u + nu * duv, v + nv * duv);
    let d = 2.0 * u - 8.0 * v + 4.0;
    let (x, y) = (3.0 * u / d, 2.0 * v / d);
    [(x / y) as f32, 1.0, ((1.0 - x - y) / y) as f32]
}

/// Capteur décodé : en-tête, lumière linéaire de l'appareil (orientée), matrice XYZ -> appareil et balance de la
/// prise de vue.
type Sensor = (RawInfo, Linear, [[f32; 3]; 4], [f32; 4]);

/// Décode le capteur. Erreur si le fichier n'est pas un RAW que `rawler` sait dématricer (l'appelant peut alors
/// retomber sur l'aperçu embarqué).
fn decode_sensor(bytes: &[u8], origin: &Path) -> Result<Sensor, EngineError> {
    use rawler::{decoders::RawDecodeParams, rawsource::RawSource};
    let name = origin.file_name().and_then(|n| n.to_str()).unwrap_or("image").to_owned();
    let corrupt = |what: String| EngineError::Corrupt(format!("RAW « {name} » : {what}"));
    let source = RawSource::new_from_slice(bytes);
    let decoder = rawler::get_decoder(&source).map_err(|e| corrupt(format!("appareil ou format non reconnu ({e})")))?;
    let params = RawDecodeParams::default();
    let raw = decoder.raw_image(&source, &params, false).map_err(|e| corrupt(e.to_string()))?;
    let metadata = decoder.raw_metadata(&source, &params).ok();

    // Mêmes étapes que `RawDevelop::default()`, sans balance des blancs, matrice ni gamma : on s'arrête à la
    // lumière linéaire de l'appareil, dématricée et rognée.
    let steps = [ProcessingStep::Rescale, ProcessingStep::Demosaic, ProcessingStep::FujiRotate, ProcessingStep::CropActiveArea, ProcessingStep::CropDefault];
    let intermediate = RawDevelop::new_with(&steps).develop_intermediate(&raw).map_err(|e| corrupt(e.to_string()))?;
    let linear = match intermediate {
        Intermediate::Monochrome(p) => Linear { width: p.width, height: p.height, channels: 1, data: p.data },
        Intermediate::ThreeColor(p) => Linear { width: p.width, height: p.height, channels: 3, data: p.data.into_iter().flatten().collect() },
        Intermediate::FourColor(p) => Linear { width: p.width, height: p.height, channels: 4, data: p.data.into_iter().flatten().collect() },
    };
    if linear.width == 0 || linear.height == 0 {
        return Err(corrupt("image vide".into()));
    }
    let (w, h) = (linear.width as u32, linear.height as u32);
    let max = crate::tiles::MAX_DIMENSION;
    if w > max || h > max {
        return Err(EngineError::InvalidDimensions { width: w, height: h, max });
    }

    // Matrice XYZ -> appareil : celle de D65, sinon une autre adaptée par Bradford (comme `rawler`).
    let xyz2cam = if linear.channels == 1 {
        [[0.0; 3]; 4]
    } else {
        let (illuminant, matrix) = raw
            .color_matrix_find_first([Illuminant::D65, Illuminant::A, Illuminant::B, Illuminant::C, Illuminant::D50, Illuminant::D55, Illuminant::D75, Illuminant::Daylight, Illuminant::Flash])
            .unwrap_or_else(|| (Illuminant::D65, rawler::imgop::matrix::IDENTITY_MATRIX_3.as_flattened().to_vec()));
        let matrix = if illuminant == Illuminant::D65 {
            matrix
        } else if matrix.len() == 9 {
            let m = transform_1d::<3, 3>(&matrix).ok_or_else(|| corrupt("matrice couleur illisible".into()))?;
            rawler::imgop::chromatic_adaption::adapt_bradford(&illuminant, &Illuminant::D65, &m).as_flattened().to_vec()
        } else {
            return Err(corrupt("matrice couleur à quatre couches sans illuminant D65".into()));
        };
        if matrix.len() % 3 != 0 || matrix.len() / 3 > 4 {
            return Err(corrupt("matrice couleur de taille inattendue".into()));
        }
        let mut xyz2cam = [[0.0f32; 3]; 4];
        for (i, row) in matrix.chunks_exact(3).enumerate() {
            xyz2cam[i] = [row[0], row[1], row[2]];
        }
        xyz2cam
    };
    // Certains vieux fichiers n'ont pas de coefficients : 1 partout, comme `rawler`.
    let as_shot = if raw.wb_coeffs[0].is_nan() { [1.0; 4] } else { raw.wb_coeffs };

    let full = linear.oriented(raw.orientation);
    let exif = metadata.as_ref().map(|m| &m.exif);
    let rational = |r: Option<rawler::formats::tiff::Rational>| r.filter(|r| r.d != 0 && r.n != 0).map(|r| r.n as f32 / r.d as f32);
    let camera = format!("{} {}", raw.clean_make, raw.clean_model).trim().to_owned();
    let info = RawInfo {
        file_name: name,
        camera,
        width: full.width as u32,
        height: full.height as u32,
        iso: exif.and_then(|e| e.iso_speed_ratings.map(u32::from).or(e.iso_speed).or(e.recommended_exposure_index)).filter(|v| *v > 0),
        aperture: exif.and_then(|e| rational(e.fnumber)),
        shutter: exif.and_then(|e| rational(e.exposure_time)),
        focal_length: exif.and_then(|e| rational(e.focal_length)),
        monochrome: full.channels == 1,
        ..RawInfo::default()
    };
    Ok((info, full, xyz2cam, as_shot))
}

/// Développement par défaut d'un RAW (balance de la prise de vue, aucun réglage), en RGBA 8 bits : ce qu'ouvre
/// Lumitype sans passer par la fenêtre de développement et ce qu'affiche la visionneuse — exactement ce que la fenêtre
/// montre en ouvrant le fichier.
pub fn develop_default(bytes: &[u8], origin: &Path) -> Result<(u32, u32, Vec<u8>), EngineError> {
    let (_, full, xyz2cam, as_shot) = decode_sensor(bytes, origin)?;
    let calibration = Calibration::new(full.channels, xyz2cam, as_shot);
    let developed = develop(&full, &calibration, calibration.as_shot, &RawParams::default(), None, 1.0, ClipView::None, None);
    Ok((developed.width, developed.height, developed.to_rgba()))
}

// ------------------------------------------------------------------ session

/// Un RAW ouvert dans la fenêtre de développement : lumière linéaire de l'appareil, en pleine taille et réduite.
pub struct RawSession {
    pub info: RawInfo,
    full: Linear,
    preview: Linear,
    calibration: Calibration,
    /// Dernière table de tonalité de l'aperçu : un glissement d'exposition ou de balance ne la recalcule pas.
    tone_cache: std::sync::Mutex<Option<Arc<ToneLut>>>,
    /// Même chose pour les zones développées en pleine définition (table plus fine) : aperçu et zone alternent sans
    /// se chasser l'un l'autre.
    region_tone_cache: std::sync::Mutex<Option<Arc<ToneLut>>>,
    /// Le fichier RAW lui-même : un objet dynamique « à redévelopper » le garde à l'octet près.
    original: Arc<Vec<u8>>,
}

impl RawSession {
    /// Décode le capteur. Erreur si le fichier n'est pas un RAW que `rawler` sait dématricer (l'appelant peut alors
    /// retomber sur l'ouverture ordinaire, qui sait se contenter de l'aperçu embarqué).
    pub fn open(bytes: Arc<Vec<u8>>, origin: &Path) -> Result<Self, EngineError> {
        let (info, full, xyz2cam, as_shot) = decode_sensor(&bytes, origin)?;
        let mut session = Self::with_linear(info, full, xyz2cam, as_shot);
        session.original = bytes;
        Ok(session)
    }

    /// Session à partir d'une image linéaire déjà prête (aussi pour les tests).
    fn with_linear(info: RawInfo, full: Linear, xyz2cam: [[f32; 3]; 4], as_shot: [f32; 4]) -> Self {
        let calibration = Calibration::new(full.channels, xyz2cam, as_shot);
        Self::with_calibration(info, full, calibration)
    }

    /// Filtre › Filtre Camera Raw : session construite depuis les pixels d'un calque (RGBA8 prémultiplié, `width` x
    /// `height`), et non depuis un capteur. Voir [`layer_calibration`] pour la balance des blancs : « telle quelle »
    /// vaut 6 500 K / 0 et ne change rien ; l'en-tête ne porte que le nom du calque (pas d'EXIF).
    pub fn from_layer(name: &str, width: u32, height: u32, premultiplied: &[u8]) -> Result<Self, EngineError> {
        let full = linear_from_premultiplied(width as usize, height as usize, premultiplied)?;
        let info = RawInfo { file_name: name.to_owned(), ..RawInfo::default() };
        Ok(Self::with_calibration(info, full, layer_calibration()))
    }

    fn with_calibration(mut info: RawInfo, full: Linear, calibration: Calibration) -> Self {
        let factor = full.width.max(full.height).div_ceil(PREVIEW_SIDE).max(1);
        let preview = full.reduced(factor);
        let auto = gray_world(&preview, &calibration);
        let auto_tt = calibration.temperature_for(auto);
        (info.as_shot_temperature, info.as_shot_tint) = calibration.as_shot_tt;
        (info.auto_temperature, info.auto_tint) = auto_tt;
        info.width = full.width as u32;
        info.height = full.height as u32;
        Self { info, full, preview, calibration, tone_cache: std::sync::Mutex::new(None), region_tone_cache: std::sync::Mutex::new(None), original: Arc::default() }
    }

    /// Octets du fichier RAW (partagés, sans copie).
    pub fn original(&self) -> &Arc<Vec<u8>> {
        &self.original
    }

    /// Mémoire occupée (pleine taille et aperçu).
    pub fn bytes(&self) -> usize {
        self.full.bytes() + self.preview.bytes()
    }

    /// Taille de l'aperçu.
    pub fn preview_size(&self) -> (u32, u32) {
        (self.preview.width as u32, self.preview.height as u32)
    }

    /// Coefficients de balance des blancs des réglages.
    fn coefficients(&self, params: &RawParams) -> [f32; 4] {
        match params.white_balance {
            WhiteBalance::AsShot => self.calibration.as_shot,
            // « Auto » passe par sa température : ce que la fenêtre affiche est exactement ce qui est rendu.
            WhiteBalance::Auto => self.calibration.coefficients_for(self.info.auto_temperature, self.info.auto_tint),
            WhiteBalance::Custom => self.calibration.coefficients_for(
                params.temperature.clamp(TEMPERATURE_MIN, TEMPERATURE_MAX),
                params.tint.clamp(-TINT_MAX, TINT_MAX),
            ),
        }
    }

    /// Table de tonalité de l'aperçu, reprise du cache si les curseurs de tonalité n'ont pas bougé.
    fn tone_lut(&self, params: &RawParams, size: usize) -> Option<Arc<ToneLut>> {
        Self::cached_lut(&self.tone_cache, params, size)
    }

    fn cached_lut(cache: &std::sync::Mutex<Option<Arc<ToneLut>>>, params: &RawParams, size: usize) -> Option<Arc<ToneLut>> {
        let key = ToneLut::key(params)?;
        let mut cache = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(lut) = cache.as_ref().filter(|lut| lut.key == key && lut.size == size) {
            return Some(Arc::clone(lut));
        }
        let lut = Arc::new(ToneLut::new(params, size)?);
        *cache = Some(Arc::clone(&lut));
        Some(lut)
    }

    /// Développe l'aperçu réduit : RVB 8 bits.
    pub fn develop_preview(&self, params: &RawParams, clip: ClipView) -> Developed {
        let scale = self.full.width as f32 / self.preview.width as f32;
        let tone = self.tone_lut(params, PREVIEW_LUT);
        develop(&self.preview, &self.calibration, self.coefficients(params), params, tone.as_deref(), scale, clip, None)
    }

    /// Zoom de la fenêtre : développe la zone `(x, y, width, height)` de l'image pleine taille, réduite d'un facteur
    /// `factor` (1 = pixels réels ; 2 = un pixel pour 2 x 2, quand l'écran n'en montre pas plus). Même rendu que le
    /// développement final : le bruit et la netteté sont calculés avec une marge autour de la zone (pas de bord
    /// artificiel), le vignetage d'après la position dans l'image entière. Renvoie la zone réellement couverte (en
    /// pixels pleine taille, bornée à l'image et alignée sur `factor`) et ses pixels. L'histogramme n'est pas calculé
    /// sur la zone (celui de l'aperçu vaut pour l'image entière).
    #[allow(clippy::too_many_arguments)]
    pub fn develop_region(&self, params: &RawParams, clip: ClipView, x: u32, y: u32, width: u32, height: u32, factor: u32) -> (Region, Developed) {
        let (fw, fh) = (self.full.width, self.full.height);
        let factor = (factor as usize).clamp(1, 16);
        // Début aligné sur le facteur : la marge retirée tombe sur un nombre entier de pixels réduits.
        let x0 = (x as usize).min(fw - 1) / factor * factor;
        let y0 = (y as usize).min(fh - 1) / factor * factor;
        let x1 = (x as usize).saturating_add(width.max(1) as usize).clamp(x0 + 1, fw);
        let y1 = (y as usize).saturating_add(height.max(1) as usize).clamp(y0 + 1, fh);
        // Rayons maximaux en pixels pleine taille : bruit couleur 2 x 11, bruit lum. 2 x 4, netteté 2.
        let margin = (32usize).max(4 * factor).div_ceil(factor) * factor;
        let (mx0, my0) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
        let (mx1, my1) = ((x1 + margin).min(fw), (y1 + margin).min(fh));
        let source = self.full.crop(mx0, my0, mx1 - mx0, my1 - my0).reduced(factor);
        let frame = Frame { x0: mx0 as f32, y0: my0 as f32, step: factor as f32, width: fw as f32, height: fh as f32 };
        let tone = Self::cached_lut(&self.region_tone_cache, params, FINAL_LUT);
        let developed = develop(&source, &self.calibration, self.coefficients(params), params, tone.as_deref(), factor as f32, clip, Some(frame));
        // Marge retirée.
        let (ox, oy) = ((x0 - mx0) / factor, (y0 - my0) / factor);
        let ow = (x1 - x0).div_ceil(factor).min(developed.width as usize - ox);
        let oh = (y1 - y0).div_ceil(factor).min(developed.height as usize - oy);
        let stride = developed.width as usize * 3;
        let mut rgb = Vec::with_capacity(ow * oh * 3);
        for row in developed.rgb.chunks_exact(stride).skip(oy).take(oh) {
            rgb.extend_from_slice(&row[ox * 3..(ox + ow) * 3]);
        }
        let region = Region { x: x0 as u32, y: y0 as u32, width: (ow * factor).min(fw - x0) as u32, height: (oh * factor).min(fh - y0) as u32 };
        (region, Developed { width: ow as u32, height: oh as u32, rgb, histogram: Vec::new() })
    }

    /// Développe l'image en pleine taille : RVB 8 bits.
    pub fn develop_full(&self, params: &RawParams) -> Developed {
        let tone = ToneLut::new(params, FINAL_LUT);
        develop(&self.full, &self.calibration, self.coefficients(params), params, tone.as_ref(), 1.0, ClipView::None, None)
    }

    /// Pipette : température et teinte qui rendent neutre le point `(x, y)` de l'image (0 à 1 sur chaque axe),
    /// moyenné sur 5 x 5 pixels de l'aperçu. `None` sur un capteur monochrome ou un point noir.
    pub fn pick_white(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let p = &self.preview;
        if p.channels < 3 {
            return None;
        }
        let cx = ((x.clamp(0.0, 1.0) * p.width as f32) as usize).min(p.width - 1);
        let cy = ((y.clamp(0.0, 1.0) * p.height as f32) as usize).min(p.height - 1);
        let mut sum = [0.0f32; 4];
        for yy in cy.saturating_sub(2)..=(cy + 2).min(p.height - 1) {
            for xx in cx.saturating_sub(2)..=(cx + 2).min(p.width - 1) {
                for (s, v) in sum.iter_mut().zip(p.pixel(xx, yy)) {
                    *s += v;
                }
            }
        }
        neutralizing(sum, p.channels).map(|wanted| self.calibration.temperature_for(wanted))
    }

    /// Tonalité automatique, d'après l'histogramme de l'aperçu à la balance des blancs donnée.
    pub fn auto_tone(&self, params: &RawParams) -> AutoTone {
        let coeffs = self.coefficients(params);
        let luminance = linear_luminance(&self.preview, &self.calibration, coeffs);
        auto_tone(&luminance)
    }
}

// ------------------------------------------------------------------ calques (Filtre Camera Raw)

/// Température et teinte « telles quelles » d'un calque : le blanc D65 du sRGB.
pub const LAYER_AS_SHOT: (f32, f32) = (6500.0, 0.0);

/// Calibration d'un calque déjà en sRGB : un « appareil » dont les couches sont celles du sRGB linéaire (matrice
/// appareil -> sRGB = identité), coefficients de prise de vue neutres (1, 1, 1). Il n'y a pas de matrice d'appareil
/// à connaître : la température et la teinte restent relatives (modèle de [`Calibration::coefficients_for`]),
/// rapportées à 6 500 K / 0 posés comme « telle quelle ». 6 500 K / 0 rend donc les pixels tels quels ; 5 000 K
/// réchauffe comme un développement réglé 1 500 K plus bas que la prise de vue, etc.
fn layer_calibration() -> Calibration {
    let m = rawler::imgop::xyz::XYZ_TO_SRGB_D65;
    let mut calibration = Calibration::new(3, [m[0], m[1], m[2], [0.0; 3]], [1.0; 4]);
    // Identité exacte : la matrice recalculée ne l'est qu'à 1e-7 près, et un rouge pur à 1,000 0001 passerait par la
    // compression des hautes lumières (qui le désature franchement).
    calibration.cam2rgb = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
    calibration.as_shot_tt = LAYER_AS_SHOT;
    calibration
}

/// sRGB 8 bits (prémultiplié) -> lumière linéaire, couleur droite ; le transparent devient noir (il n'a pas de couleur).
fn linear_from_premultiplied(width: usize, height: usize, premultiplied: &[u8]) -> Result<Linear, EngineError> {
    if width == 0 || height == 0 || premultiplied.len() != width * height * 4 {
        return Err(EngineError::InvalidOperation("ce calque est vide".into()));
    }
    let table: Vec<f32> = (0..=255u8).map(|v| rawler::imgop::srgb::srgb_invert_gamma(f32::from(v) / 255.0)).collect();
    let mut data = vec![0.0f32; width * height * 3];
    data.par_chunks_exact_mut(3).zip(premultiplied.par_chunks_exact(4)).for_each(|(out, px)| {
        let a = px[3];
        if a == 0 {
            return;
        }
        for k in 0..3 {
            // Couleur droite arrondie au niveau le plus proche : un calque opaque est relu à l'identique.
            let straight = if a == 255 { px[k] } else { ((u32::from(px[k]) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8 };
            out[k] = table[usize::from(straight)];
        }
    });
    Ok(Linear { width, height, channels: 3, data })
}

/// Filtre Camera Raw appliqué à une image : couleur droite sRGB (0 à 1), rendue développée (0 à 1, au niveau 8 bits
/// près). Même chaîne que la fenêtre (balance relative, vignetage et exposition en linéaire, bruit, tonalité, netteté),
/// en pleine taille. `Auto` : le gris moyen de toute l'image.
pub fn develop_image(rgb: &[[f32; 3]], width: usize, height: usize, params: &RawParams) -> Vec<[f32; 3]> {
    if rgb.len() != width * height || rgb.is_empty() {
        return rgb.to_vec();
    }
    let data: Vec<f32> = rgb.par_iter().flat_map_iter(|c| c.map(|v| rawler::imgop::srgb::srgb_invert_gamma(v.clamp(0.0, 1.0)))).collect();
    let linear = Linear { width, height, channels: 3, data };
    let calibration = layer_calibration();
    let coeffs = match params.white_balance {
        WhiteBalance::AsShot => calibration.as_shot,
        WhiteBalance::Auto => {
            let (t, n) = calibration.temperature_for(gray_world(&linear, &calibration));
            calibration.coefficients_for(t, n)
        }
        WhiteBalance::Custom => calibration.coefficients_for(params.temperature.clamp(TEMPERATURE_MIN, TEMPERATURE_MAX), params.tint.clamp(-TINT_MAX, TINT_MAX)),
    };
    let tone = ToneLut::new(params, FINAL_LUT);
    let developed = develop(&linear, &calibration, coeffs, params, tone.as_ref(), 1.0, ClipView::None, None);
    developed.rgb.par_chunks_exact(3).map(|px| [px[0], px[1], px[2]].map(|v| f32::from(v) / 255.0)).collect()
}

/// Coefficients qui rendent neutre une couleur de l'appareil (vert à 1).
fn neutralizing(color: [f32; 4], channels: usize) -> Option<[f32; 4]> {
    if color.iter().take(channels).any(|v| *v <= 1e-6) {
        return None;
    }
    let green = color[1];
    let mut wanted = [1.0f32; 4];
    for (i, w) in wanted.iter_mut().enumerate().take(channels) {
        *w = green / color[i];
    }
    Some(wanted)
}

/// Balance « Auto » : moyenne de l'image (hors pixels proches de la saturation, qui ne disent plus rien de la couleur).
fn gray_world(lin: &Linear, calibration: &Calibration) -> [f32; 4] {
    let c = lin.channels;
    if c < 3 {
        return calibration.as_shot;
    }
    let (sum, count) = lin
        .data
        .par_chunks_exact(c)
        .filter(|px| px.iter().all(|v| *v < 0.95) && px.iter().any(|v| *v > 0.002))
        .fold(|| ([0.0f64; 4], 0usize), |(mut s, n), px| {
            for (k, v) in px.iter().enumerate() {
                s[k] += f64::from(*v);
            }
            (s, n + 1)
        })
        .reduce(|| ([0.0f64; 4], 0), |(a, n), (b, m)| ([a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]], n + m));
    if count == 0 {
        return calibration.as_shot;
    }
    neutralizing(sum.map(|v| v as f32), c).unwrap_or(calibration.as_shot)
}

// ------------------------------------------------------------------ développement

/// Image développée : RVB 8 bits et histogramme (256 cases par couche : rouge, vert, bleu).
pub struct Developed {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    pub histogram: Vec<u32>,
}

impl Developed {
    /// RGBA opaque.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(self.rgb.len() / 3 * 4);
        for px in self.rgb.chunks_exact(3) {
            rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
        rgba
    }

    /// JPEG (qualité `quality`) pour l'aperçu.
    pub fn to_jpeg(&self, quality: u8) -> Result<Vec<u8>, EngineError> {
        crate::io::encode_jpeg_rgb(&self.rgb, self.width, self.height, quality, None)
    }
}

/// Étapes 1 à 3 du module, pour un pixel : sRGB gamma (0 à 1).
#[inline]
fn to_display(px: &[f32], coeffs: &[f32; 4], m: &[[f32; 4]; 3], gain: f32, knee: f32) -> [f32; 3] {
    if px.len() == 1 {
        // Monochrome : ni balance ni matrice (comme `rawler`).
        let v = (px[0] * gain).max(0.0);
        return [srgb_apply_gamma(if v > knee { shoulder(v, knee) } else { v }); 3];
    }
    let srgb = if px.len() == 3 {
        // Même calcul, dans le même ordre, que `rawler::imgop::raw::map_3ch_to_rgb`.
        let (r, g, b) = (px[0] * coeffs[0], px[1] * coeffs[1], px[2] * coeffs[2]);
        [
            m[0][0] * r + m[0][1] * g + m[0][2] * b,
            m[1][0] * r + m[1][1] * g + m[1][2] * b,
            m[2][0] * r + m[2][1] * g + m[2][2] * b,
        ]
    } else {
        let c = [px[0] * coeffs[0], px[1] * coeffs[1], px[2] * coeffs[2], px[3] * coeffs[3]];
        [0, 1, 2].map(|i| m[i][0] * c[0] + m[i][1] * c[1] + m[i][2] * c[2] + m[i][3] * c[3])
    };
    let srgb = if gain == 1.0 { srgb } else { srgb.map(|v| v * gain) };
    roll_off(srgb, knee).map(srgb_apply_gamma)
}

/// Épaule des hautes lumières (lumière linéaire) : identité jusqu'au genou `knee`, puis une courbe de pente 1 au genou
/// qui tend vers 1 sans jamais le franchir — pas de cassure, donc pas de plage plate à bord net. Genou à 1 : coupure
/// simple au blanc.
#[inline]
fn shoulder(v: f32, knee: f32) -> f32 {
    if v <= knee {
        return v;
    }
    if knee >= 1.0 {
        return 1.0;
    }
    let room = 1.0 - knee;
    knee + room * (1.0 - (-(v - knee) / room).exp())
}

/// Hautes lumières d'un pixel sRGB linéaire. La plus forte couche passe par [`shoulder`] et les trois sont mises à la
/// même échelle : la teinte est gardée (pas de peau qui vire à l'orange quand le rouge sature seul, comme avec une
/// coupure couche par couche). Au-delà du blanc, la couleur s'éteint progressivement vers un blanc de même intensité,
/// comme sur un film : un aplat « brûlé » reste lisse, sans plaques. Tout est continu : un curseur qui pousse une zone
/// au-delà du blanc ne fait jamais apparaître de frontière.
#[inline]
fn roll_off(srgb: [f32; 3], knee: f32) -> [f32; 3] {
    let p = srgb.map(|v| v.max(0.0));
    let peak = p[0].max(p[1]).max(p[2]);
    if peak <= knee {
        return p;
    }
    let target = shoulder(peak, knee);
    let ratio = target / peak;
    // 0 au blanc, 1 à 2,5 fois le blanc (lissé).
    let t = ((peak - 1.0) / 1.5).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    p.map(|v| {
        let scaled = v * ratio;
        scaled + (target - scaled) * t
    })
}

/// Genou de l'épaule pour ces réglages : 1 (coupure au blanc) quand rien ne pousse l'image au-delà de ce qu'elle était
/// — développement par défaut, calque filtré sans réglage, laissé intact — puis de plus en plus bas à mesure que
/// l'exposition, la balance des blancs ou le vignetage relèvent les valeurs : ce qui aurait été coupé est compressé.
/// `headroom` : facteur maximal appliqué aux valeurs d'origine (2 = +1 IL).
fn knee_for(headroom: f32) -> f32 {
    (1.5 - 0.5 * headroom).clamp(0.5, 1.0)
}

/// Tramage au passage en 8 bits : une valeur dans (-0,49 ; 0,49) niveau, fixée par la position dans l'image entière
/// (une zone de zoom tombe sur la même trame que le développement final). Les dégradés poussés par les curseurs ne
/// montrent plus de marches ; un niveau exact (calque sans réglage) reste exactement lui-même.
#[inline]
fn dither(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ 0x27d4_eb2f;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    ((h >> 8) as f32 / 16_777_216.0 - 0.5) * 0.98
}

/// Gain du vignetage au pixel `(x, y)` : 2^(force x r²), r = distance au centre rapportée à la demi-diagonale.
#[inline]
fn vignette_gain(amount: f32, x: usize, y: usize, frame: &Frame) -> f32 {
    if amount == 0.0 {
        return 1.0;
    }
    let (cx, cy) = (frame.width / 2.0, frame.height / 2.0);
    let (dx, dy) = (frame.x0 + (x as f32 + 0.5) * frame.step - cx, frame.y0 + (y as f32 + 0.5) * frame.step - cy);
    (amount * (dx * dx + dy * dy) / (cx * cx + cy * cy)).exp2()
}

/// `frame` : place de `lin` dans l'image entière (zone d'un zoom) ; `None` = `lin` est l'image entière.
#[allow(clippy::too_many_arguments)]
fn develop(lin: &Linear, calibration: &Calibration, coeffs: [f32; 4], params: &RawParams, tone: Option<&ToneLut>, scale: f32, clip: ClipView, frame: Option<Frame>) -> Developed {
    let (w, h) = (lin.width, lin.height);
    let frame = frame.unwrap_or(Frame { x0: 0.0, y0: 0.0, step: 1.0, width: w as f32, height: h as f32 });
    let c = lin.channels;
    let exposure = 2f32.powf(params.exposure.clamp(-5.0, 5.0));
    let vignette = params.vignette.clamp(-100.0, 100.0) / 100.0;
    let m = &calibration.cam2rgb;
    // Ce que les réglages ajoutent au plus aux valeurs d'origine : exposition, vignetage (coins), balance des blancs
    // (rapport le plus fort aux coefficients de la prise de vue).
    let white = if c >= 3 { (0..c.min(4)).map(|k| coeffs[k] / calibration.as_shot[k].max(1e-6)).fold(1.0f32, f32::max) } else { 1.0 };
    let knee = knee_for(exposure * vignette.max(0.0).exp2() * white);

    // 1-3 : linéaire -> sRGB gamma.
    let mut rgb = vec![0.0f32; w * h * 3];
    rgb.par_chunks_exact_mut(w * 3).enumerate().for_each(|(y, row)| {
        let source = &lin.data[y * w * c..(y + 1) * w * c];
        for (x, (out, px)) in row.chunks_exact_mut(3).zip(source.chunks_exact(c)).enumerate() {
            let gain = exposure * vignette_gain(vignette, x, y, &frame);
            out.copy_from_slice(&to_display(px, &coeffs, m, gain, knee));
        }
    });

    // 4 : réduction du bruit.
    denoise(&mut rgb, w, h, params.noise_luma, params.noise_color, scale);

    // 5 : tonalité et présence, par le réglage Développement existant, mis en table 3D (voir `ToneLut`).
    if let Some(lut) = tone {
        rgb.par_chunks_exact_mut(3 * 1024).for_each(|chunk| {
            for px in chunk.chunks_exact_mut(3) {
                let out = lut.apply([px[0], px[1], px[2]]);
                px.copy_from_slice(&out);
            }
        });
    }

    // 6 : netteté.
    sharpen(&mut rgb, w, h, params.sharpen, scale);

    // 8 bits, histogramme, et zones écrêtées si demandé.
    let mut bytes = vec![0u8; w * h * 3];
    let histogram = bytes
        .par_chunks_exact_mut(w * 3)
        .zip(rgb.par_chunks_exact(w * 3))
        .enumerate()
        .fold(
            || vec![0u32; 768],
            |mut hist, (y, (out, row))| {
                let ay = (frame.y0 + y as f32 * frame.step) as u32;
                for (x, (o, v)) in out.chunks_exact_mut(3).zip(row.chunks_exact(3)).enumerate() {
                    let d = dither((frame.x0 + x as f32 * frame.step) as u32, ay);
                    for k in 0..3 {
                        let b = (v[k].clamp(0.0, 1.0) * 255.0 + d).round().clamp(0.0, 255.0) as u8;
                        o[k] = b;
                        hist[k * 256 + usize::from(b)] += 1;
                    }
                    match clip {
                        ClipView::None => {}
                        ClipView::Highlights => o.iter_mut().for_each(|b| *b = if *b >= 254 { 255 } else { 0 }),
                        ClipView::Shadows => o.iter_mut().for_each(|b| *b = if *b <= 1 { 0 } else { 255 }),
                    }
                }
                hist
            },
        )
        .reduce(|| vec![0u32; 768], |mut a, b| {
            a.iter_mut().zip(b).for_each(|(x, y)| *x += y);
            a
        });
    Developed { width: w as u32, height: h as u32, rgb: bytes, histogram }
}

/// Luminance linéaire (avant toute coupure) à la balance donnée, exposition 0 : de quoi régler l'Auto.
fn linear_luminance(lin: &Linear, calibration: &Calibration, coeffs: [f32; 4]) -> Vec<f32> {
    let c = lin.channels;
    let m = &calibration.cam2rgb;
    lin.data
        .par_chunks_exact(c)
        .map(|px| {
            if c == 1 {
                return px[0];
            }
            let s = [0, 1, 2].map(|i| (0..c).map(|k| m[i][k] * px[k] * coeffs[k]).sum::<f32>());
            0.2126 * s[0] + 0.7152 * s[1] + 0.0722 * s[2]
        })
        .collect()
}

/// Tonalité automatique : exposition qui amène la moyenne géométrique au gris moyen (18 %) sans brûler plus de
/// 1 % de l'image, puis blancs et noirs étirés vers les extrémités, hautes lumières baissées s'il reste des zones
/// brûlées et ombres relevées si l'image est très sombre.
fn auto_tone(luminance: &[f32]) -> AutoTone {
    if luminance.is_empty() {
        return AutoTone::default();
    }
    let mut sorted: Vec<f32> = luminance.iter().map(|v| v.max(0.0)).collect();
    sorted.par_sort_unstable_by(f32::total_cmp);
    let at = |q: f32| sorted[((sorted.len() - 1) as f32 * q) as usize];
    let log_mean = (sorted.iter().map(|v| f64::from(v.max(1e-4)).ln()).sum::<f64>() / sorted.len() as f64).exp() as f32;
    let mut exposure = (0.18 / log_mean.max(1e-4)).log2();
    // Pas plus de 1 % de l'image au-delà de 1,2 (récupérable par les hautes lumières).
    let p99 = at(0.99).max(1e-4);
    exposure = exposure.min((1.2 / p99).log2()).clamp(-3.0, 3.0);
    let gain = exposure.exp2();
    let gamma = |v: f32| srgb_apply_gamma((v * gain).clamp(0.0, 1.0));
    let (hi, lo) = (gamma(at(0.995)), gamma(at(0.005)));
    let whites = if hi < 0.95 { ((0.95 - hi) / 0.2 * 100.0).clamp(0.0, 50.0) } else { 0.0 };
    let blacks = if lo > 0.02 { -((lo - 0.02) / 0.12 * 100.0).clamp(0.0, 50.0) } else { 0.0 };
    let burnt = sorted.iter().rev().take_while(|v| **v * gain > 1.0).count() as f32 / sorted.len() as f32;
    let highlights = -(burnt * 600.0).clamp(0.0, 60.0);
    let dark = sorted.iter().take_while(|v| gamma(**v) < 0.15).count() as f32 / sorted.len() as f32;
    let shadows = ((dark - 0.2) * 120.0).clamp(0.0, 50.0);
    let round = |v: f32| v.round();
    AutoTone { exposure: (exposure * 20.0).round() / 20.0, contrast: 0.0, highlights: round(highlights), shadows: round(shadows), whites: round(whites), blacks: round(blacks) }
}

/// Côté de la table de tonalité : aperçu (vite construite) et développement final (plus fine).
const PREVIEW_LUT: usize = 33;
const FINAL_LUT: usize = 65;

/// Le réglage `Develop` (tonalité et présence) ne dépend que de la couleur du pixel : il est évalué une fois aux nœuds
/// d'une grille RVB (`size`³), puis interpolé (trilinéaire). Avec ses étages mesurés, l'évaluer pour chaque pixel
/// coûterait ~100 ns par étage et par pixel : plus d'une seconde par aperçu avec tous les curseurs.
struct ToneLut {
    key: [u32; 8],
    size: usize,
    data: Vec<[f32; 3]>,
}

impl ToneLut {
    /// Curseurs de tonalité et de présence (bits des flottants), ou rien s'ils sont tous neutres.
    fn key(p: &RawParams) -> Option<[u32; 8]> {
        let values = [p.contrast, p.highlights, p.shadows, p.whites, p.blacks, p.clarity, p.vibrance, p.saturation];
        values.iter().any(|v| *v != 0.0).then(|| values.map(f32::to_bits))
    }

    fn new(p: &RawParams, size: usize) -> Option<Self> {
        let key = Self::key(p)?;
        let prepared: Prepared = Adjustment::Develop {
            temperature: 0.0,
            tint: 0.0,
            exposure: 0.0,
            contrast: p.contrast,
            highlights: p.highlights,
            shadows: p.shadows,
            whites: p.whites,
            blacks: p.blacks,
            clarity: p.clarity,
            dehaze: 0.0,
            vibrance: p.vibrance,
            saturation: p.saturation,
            red_hue: 0.0,
            green_hue: 0.0,
            blue_hue: 0.0,
        }
        .prepare();
        let step = 1.0 / (size - 1) as f32;
        let mut data = vec![[0.0f32; 3]; size * size * size];
        // Indice (r * size + g) * size + b.
        data.par_chunks_exact_mut(size).enumerate().for_each(|(rg, row)| {
            let (r, g) = ((rg / size) as f32 * step, (rg % size) as f32 * step);
            for (b, slot) in row.iter_mut().enumerate() {
                *slot = prepared.apply([r, g, b as f32 * step]);
            }
        });
        Some(Self { key, size, data })
    }

    #[inline]
    fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        let last = (n - 1) as f32;
        let mut base = [0usize; 3];
        let mut frac = [0.0f32; 3];
        for k in 0..3 {
            let x = rgb[k].clamp(0.0, 1.0) * last;
            let i = (x as usize).min(n - 2);
            base[k] = i;
            frac[k] = x - i as f32;
        }
        let at = |r: usize, g: usize, b: usize| &self.data[(r * n + g) * n + b];
        let mut out = [0.0f32; 3];
        for (dr, wr) in [(0, 1.0 - frac[0]), (1, frac[0])] {
            for (dg, wg) in [(0, 1.0 - frac[1]), (1, frac[1])] {
                for (db, wb) in [(0, 1.0 - frac[2]), (1, frac[2])] {
                    let w = wr * wg * wb;
                    let v = at(base[0] + dr, base[1] + dg, base[2] + db);
                    out[0] += v[0] * w;
                    out[1] += v[1] * w;
                    out[2] += v[2] * w;
                }
            }
        }
        out
    }
}

// ------------------------------------------------------------------ bruit et netteté

/// Moyenne glissante horizontale de rayon `r` (bords prolongés), lignes en parallèle.
fn box_rows(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    let n = (2 * r + 1) as f32;
    out.par_chunks_exact_mut(w).zip(src.par_chunks_exact(w)).for_each(|(o, row)| {
        let at = |i: isize| row[i.clamp(0, w as isize - 1) as usize];
        let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
        for (x, slot) in o.iter_mut().enumerate() {
            *slot = sum / n;
            let x = x as isize;
            sum += at(x + r as isize + 1) - at(x - r as isize);
        }
    });
    out
}

/// Transposition par blocs, en parallèle.
fn transpose(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    const B: usize = 32;
    out.par_chunks_mut(h * B).enumerate().for_each(|(bx, block)| {
        // `block` : lignes de sortie x = bx*B .. bx*B+B (colonnes de la source).
        let x0 = bx * B;
        let rows = block.len() / h;
        for y0 in (0..h).step_by(B) {
            for y in y0..(y0 + B).min(h) {
                for dx in 0..rows {
                    block[dx * h + y] = src[y * w + x0 + dx];
                }
            }
        }
    });
    out
}

/// Flou en boîte de rayon `r`, séparable.
fn box_blur(plane: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    if r == 0 {
        return plane.to_vec();
    }
    let rows = box_rows(plane, w, h, r);
    let t = box_rows(&transpose(&rows, w, h), h, w, r);
    transpose(&t, h, w)
}

const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Réduction du bruit, en sRGB gamma : filtre guidé (He et al.) sur la luminance — un aplat bruité est lissé, un bord
/// franc (forte variance locale) est gardé — et double flou en boîte de la chrominance. Les rayons sont donnés en
/// pixels de l'image pleine taille, ramenés à l'échelle de l'aperçu (`scale` pixels pleins par pixel traité).
fn denoise(rgb: &mut [f32], w: usize, h: usize, luma: f32, color: f32, scale: f32) {
    let (sl, sc) = (luma.clamp(0.0, 100.0) / 100.0, color.clamp(0.0, 100.0) / 100.0);
    if sl == 0.0 && sc == 0.0 {
        return;
    }
    let radius = |full: f32| ((full / scale).round() as usize).max(1);
    let n = w * h;
    let mut y = vec![0.0f32; n];
    let mut cb = vec![0.0f32; n];
    let mut cr = vec![0.0f32; n];
    y.par_iter_mut().zip(cb.par_iter_mut()).zip(cr.par_iter_mut()).enumerate().for_each(|(i, ((l, b), r))| {
        let p = &rgb[i * 3..i * 3 + 3];
        *l = LUMA[0] * p[0] + LUMA[1] * p[1] + LUMA[2] * p[2];
        *b = p[2] - *l;
        *r = p[0] - *l;
    });
    if sl > 0.0 {
        let r = radius(1.0 + 3.0 * sl);
        let eps = (0.002 + 0.03 * sl).powi(2);
        let squares: Vec<f32> = y.par_iter().map(|v| v * v).collect();
        let (mean, mean_sq) = (box_blur(&y, w, h, r), box_blur(&squares, w, h, r));
        let (a, b): (Vec<f32>, Vec<f32>) = mean
            .par_iter()
            .zip(mean_sq.par_iter())
            .map(|(m, s)| {
                let var = (s - m * m).max(0.0);
                let a = var / (var + eps);
                (a, m * (1.0 - a))
            })
            .unzip();
        let (ma, mb) = (box_blur(&a, w, h, r), box_blur(&b, w, h, r));
        y.par_iter_mut().zip(ma.par_iter().zip(mb.par_iter())).for_each(|(v, (a, b))| *v = a * *v + b);
    }
    if sc > 0.0 {
        let r = radius(1.0 + 10.0 * sc);
        cb = box_blur(&box_blur(&cb, w, h, r), w, h, r);
        cr = box_blur(&box_blur(&cr, w, h, r), w, h, r);
    }
    rgb.par_chunks_exact_mut(3).enumerate().for_each(|(i, p)| {
        let (l, b, r) = (y[i], cb[i], cr[i]);
        let (red, blue) = (l + r, l + b);
        let green = (l - LUMA[0] * red - LUMA[2] * blue) / LUMA[1];
        p.copy_from_slice(&[red, green, blue]);
    });
}

/// Netteté : masque flou de la luminance (rayon ~1 px de l'image pleine taille), seuil doux contre le bruit.
fn sharpen(rgb: &mut [f32], w: usize, h: usize, amount: f32, scale: f32) {
    let s = amount.clamp(0.0, 100.0) / 100.0;
    if s == 0.0 {
        return;
    }
    let y: Vec<f32> = rgb.par_chunks_exact(3).map(|p| LUMA[0] * p[0] + LUMA[1] * p[1] + LUMA[2] * p[2]).collect();
    // Deux passes de rayon 1 (presque une gaussienne) en pleine taille ; une seule sur un aperçu réduit.
    let blurred = if scale > 1.5 { box_blur(&y, w, h, 1) } else { box_blur(&box_blur(&y, w, h, 1), w, h, 1) };
    let strength = s * 1.5 * if scale > 1.5 { 0.6 } else { 1.0 };
    rgb.par_chunks_exact_mut(3).enumerate().for_each(|(i, p)| {
        let detail = y[i] - blurred[i];
        // Seuil doux : les variations de moins de ~1 niveau (bruit) sont à peine renforcées.
        let weight = (detail.abs() / 0.004).min(1.0);
        let delta = detail * strength * weight;
        p.iter_mut().for_each(|v| *v += delta);
    });
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
    }

    /// Matrice XYZ -> appareil d'un appareil « idéal » dont les couches sont celles du sRGB linéaire.
    fn srgb_camera() -> [[f32; 3]; 4] {
        let m = rawler::imgop::xyz::XYZ_TO_SRGB_D65;
        [m[0], m[1], m[2], [0.0; 3]]
    }

    /// Session synthétique : `pixel(x, y)` donne la lumière linéaire de l'appareil.
    fn synthetic(w: usize, h: usize, as_shot: [f32; 4], pixel: impl Fn(usize, usize) -> [f32; 3]) -> RawSession {
        let mut data = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&pixel(x, y));
            }
        }
        RawSession::with_linear(RawInfo::default(), Linear { width: w, height: h, channels: 3, data }, srgb_camera(), as_shot)
    }

    fn at(d: &Developed, x: usize, y: usize) -> [u8; 3] {
        let i = (y * d.width as usize + x) * 3;
        [d.rgb[i], d.rgb[i + 1], d.rgb[i + 2]]
    }

    fn inverse_gamma(v: u8) -> f32 {
        rawler::imgop::srgb::srgb_invert_gamma(f32::from(v) / 255.0)
    }

    /// Développement de `rawler` seul (ce que faisait l'ouverture des RAW avant), orienté, en RGBA.
    fn plain_rawler(bytes: &[u8], path: &Path) -> Vec<u8> {
        use rawler::{decoders::RawDecodeParams, rawsource::RawSource};
        let source = RawSource::new_from_slice(bytes);
        let decoder = rawler::get_decoder(&source).unwrap();
        let raw = decoder.raw_image(&source, &RawDecodeParams::default(), false).unwrap();
        let session = RawSession::open(Arc::new(bytes.to_vec()), path).unwrap();
        let image = RawDevelop::default().develop_intermediate(&raw).unwrap().to_dynamic_image().unwrap().to_rgba8();
        let (w, h) = (image.width() as usize, image.height() as usize);
        let lin = Linear { width: w, height: h, channels: 4, data: image.into_raw().into_iter().map(f32::from).collect() }.oriented(raw.orientation);
        assert_eq!((lin.width as u32, lin.height as u32), (session.info.width, session.info.height));
        lin.data.into_iter().map(|v| v as u8).collect()
    }

    #[test]
    fn default_development_matches_the_plain_decoder() {
        for name in ["mire.dng", "mire-lineaire.dng"] {
            let path = fixture(name);
            let bytes = std::fs::read(&path).unwrap();
            let session = RawSession::open(Arc::new(bytes.clone()), &path).unwrap();
            let developed = session.develop_full(&RawParams::default());
            // Ouvrir le RAW sans la fenêtre donne exactement la même image.
            let (w, h, opened) = crate::io::decode_premultiplied(&bytes, &path).unwrap();
            assert_eq!((developed.width, developed.height), (w, h), "{name} : mêmes dimensions (orientation comprise)");
            let rgba = developed.to_rgba();
            assert_eq!(rgba, opened, "{name} : même développement par défaut");
            // Et c'est le rendu de `rawler` (couleurs, matrice, balance), au tramage près, partout où rien n'atteint le
            // blanc (au-delà, l'épaule des hautes lumières remplace la coupure de `rawler`).
            let reference = plain_rawler(&bytes, &path);
            let mut worst = 0;
            for (a, b) in rgba.chunks_exact(4).zip(reference.chunks_exact(4)) {
                if a[..3].iter().all(|v| *v < 250) {
                    worst = worst.max((0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap());
                }
            }
            assert!(worst <= 2, "{name} : écart maximal {worst} niveaux");
            // L'en-tête lit l'appareil et la balance telle quelle donne une température plausible.
            assert!(!session.info.camera.is_empty());
            assert!((TEMPERATURE_MIN..=TEMPERATURE_MAX).contains(&session.info.as_shot_temperature));
        }
    }

    #[test]
    fn as_shot_temperature_reproduces_the_camera_coefficients() {
        // Choisir « Personnalisée » à la température « telle quelle » ne doit rien changer.
        let path = fixture("mire.dng");
        let session = RawSession::open(Arc::new(std::fs::read(&path).unwrap()), &path).unwrap();
        let custom = RawParams { white_balance: WhiteBalance::Custom, temperature: session.info.as_shot_temperature, tint: session.info.as_shot_tint, ..RawParams::default() };
        let (a, b) = (session.develop_preview(&RawParams::default(), ClipView::None), session.develop_preview(&custom, ClipView::None));
        let worst = a.rgb.iter().zip(&b.rgb).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        assert!(worst <= 1, "écart {worst}");
    }

    #[test]
    fn plus_one_stop_doubles_linear_values() {
        let session = synthetic(64, 64, [1.0; 4], |x, _| { let v = 0.02 + 0.2 * x as f32 / 64.0; [v, v, v] });
        let base = session.develop_full(&RawParams::default());
        let brighter = session.develop_full(&RawParams { exposure: 1.0, ..RawParams::default() });
        for x in [4, 20, 40, 60] {
            let (a, b) = (inverse_gamma(at(&base, x, 10)[1]), inverse_gamma(at(&brighter, x, 10)[1]));
            assert!((b / a - 2.0).abs() < 0.06, "x={x} : {a} -> {b}");
        }
    }

    #[test]
    fn white_balance_picker_neutralises_a_gray() {
        // Gris photographié sous une lumière chaude : l'appareil voit plus de rouge, moins de bleu.
        let session = synthetic(40, 40, [1.0; 4], |_, _| [0.30, 0.20, 0.12]);
        let (temperature, tint) = session.pick_white(0.5, 0.5).unwrap();
        let params = RawParams { white_balance: WhiteBalance::Custom, temperature, tint, ..RawParams::default() };
        let px = at(&session.develop_full(&params), 20, 20);
        let spread = px.iter().max().unwrap() - px.iter().min().unwrap();
        assert!(spread <= 3, "gris neutre attendu : {px:?} ({temperature} K, teinte {tint})");
        // Lumière chaude : la pipette trouve une température basse.
        assert!(temperature < 5000.0, "{temperature}");
    }

    #[test]
    fn temperature_moves_the_right_way() {
        let session = synthetic(16, 16, [1.0; 4], |_, _| [0.18, 0.18, 0.18]);
        let (t0, n0) = (session.info.as_shot_temperature, session.info.as_shot_tint);
        let with = |temperature: f32| at(&session.develop_full(&RawParams { white_balance: WhiteBalance::Custom, temperature, tint: n0, ..RawParams::default() }), 8, 8);
        let (warm, cold) = (with(t0 * 1.6), with(t0 * 0.6));
        assert!(warm[0] > warm[2] + 10, "plus chaud : plus jaune {warm:?}");
        assert!(cold[2] > cold[0] + 10, "plus froid : plus bleu {cold:?}");
        // Teinte positive : magenta (vert en retrait).
        let magenta = at(&session.develop_full(&RawParams { white_balance: WhiteBalance::Custom, temperature: t0, tint: n0 + 60.0, ..RawParams::default() }), 8, 8);
        assert!(magenta[1] + 5 < magenta[0] && magenta[1] + 5 < magenta[2], "{magenta:?}");
    }

    #[test]
    fn highlights_beyond_white_are_recoverable() {
        // Zone « brûlée » : de 1,2 à 1,8 en linéaire. À 0 IL elle est plate (255) ; à -1 IL elle retrouve son relief.
        let session = synthetic(64, 8, [1.0; 4], |x, _| { let v = 1.2 + 0.6 * x as f32 / 63.0; [v, v, v] });
        let flat = session.develop_full(&RawParams::default());
        assert!((0..64).all(|x| at(&flat, x, 4)[1] == 255));
        let recovered = session.develop_full(&RawParams { exposure: -1.0, ..RawParams::default() });
        let (left, right) = (at(&recovered, 2, 4)[1], at(&recovered, 61, 4)[1]);
        assert!(right > left + 20 && right < 255, "relief retrouvé : {left} -> {right}");
    }

    #[test]
    fn histogram_counts_every_pixel_per_channel() {
        let session = synthetic(30, 20, [1.0; 4], |x, _| if x < 10 { [0.0; 3] } else { [1.0; 3] });
        let developed = session.develop_preview(&RawParams::default(), ClipView::None);
        assert_eq!(developed.histogram.len(), 768);
        for k in 0..3 {
            let channel = &developed.histogram[k * 256..(k + 1) * 256];
            assert_eq!(channel.iter().sum::<u32>(), 600);
            assert_eq!(channel[0], 200);
            assert_eq!(channel[255], 400);
        }
        // Zones écrêtées : fond noir, seules les zones à 255 restent blanches.
        let clipped = session.develop_preview(&RawParams::default(), ClipView::Highlights);
        assert_eq!(at(&clipped, 2, 2), [0, 0, 0]);
        assert_eq!(at(&clipped, 20, 2), [255, 255, 255]);
    }

    #[test]
    fn auto_tone_brightens_a_dark_image_and_tone_sliders_act() {
        let session = synthetic(64, 64, [1.0; 4], |x, y| { let v = 0.005 + 0.03 * ((x + y) as f32 / 128.0); [v, v, v] });
        let auto = session.auto_tone(&RawParams::default());
        assert!(auto.exposure > 1.0, "{auto:?}");
        // Les curseurs de tonalité passent bien par le réglage Développement.
        let base = at(&session.develop_full(&RawParams::default()), 32, 32);
        let shadows = at(&session.develop_full(&RawParams { shadows: 80.0, ..RawParams::default() }), 32, 32);
        assert!(shadows[1] > base[1], "{base:?} -> {shadows:?}");
    }

    #[test]
    fn noise_reduction_smooths_flat_areas_and_sharpening_adds_contrast() {
        // Aplat gris bruité (bruit déterministe) avec un bord net au milieu.
        let noise = |x: usize, y: usize| ((x * 7919 + y * 104_729) % 97) as f32 / 97.0 - 0.5;
        let session = synthetic(80, 80, [1.0; 4], |x, y| { let v = if x < 40 { 0.1 } else { 0.4 } + 0.02 * noise(x, y); [v, v, v] });
        let spread = |d: &Developed| {
            let values: Vec<f32> = (5..30).flat_map(|y| (5..30).map(move |x| (x, y))).map(|(x, y)| f32::from(at(d, x, y)[1])).collect();
            let mean = values.iter().sum::<f32>() / values.len() as f32;
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32).sqrt()
        };
        let base = session.develop_full(&RawParams::default());
        let smooth = session.develop_full(&RawParams { noise_luma: 60.0, noise_color: 60.0, ..RawParams::default() });
        assert!(spread(&smooth) < spread(&base) * 0.6, "{} -> {}", spread(&base), spread(&smooth));
        // Le bord reste franc.
        assert!(at(&smooth, 36, 40)[1] + 40 < at(&smooth, 44, 40)[1]);
        let sharp = session.develop_full(&RawParams { sharpen: 80.0, ..RawParams::default() });
        let edge = |d: &Developed| i32::from(at(d, 40, 40)[1]) - i32::from(at(d, 39, 40)[1]);
        assert!(edge(&sharp) > edge(&base), "{} -> {}", edge(&base), edge(&sharp));
    }

    #[test]
    fn vignette_correction_brightens_corners_only() {
        let session = synthetic(101, 101, [1.0; 4], |_, _| [0.1, 0.1, 0.1]);
        let base = session.develop_full(&RawParams::default());
        let fixed = session.develop_full(&RawParams { vignette: 100.0, ..RawParams::default() });
        assert!(at(&fixed, 50, 50)[1].abs_diff(at(&base, 50, 50)[1]) <= 1);
        let ratio = inverse_gamma(at(&fixed, 0, 0)[1]) / inverse_gamma(at(&base, 0, 0)[1]);
        assert!((ratio - 2.0).abs() < 0.15, "coin : +1 IL, {ratio}");
    }

    #[test]
    fn tone_table_follows_the_develop_adjustment() {
        let p = RawParams { contrast: 30.0, highlights: -60.0, shadows: 40.0, whites: 10.0, blacks: -15.0, clarity: 20.0, vibrance: 25.0, saturation: -10.0, ..RawParams::default() };
        let lut = ToneLut::new(&p, FINAL_LUT).unwrap();
        let exact = Adjustment::Develop { temperature: 0.0, tint: 0.0, exposure: 0.0, contrast: 30.0, highlights: -60.0, shadows: 40.0, whites: 10.0, blacks: -15.0, clarity: 20.0, dehaze: 0.0, vibrance: 25.0, saturation: -10.0, red_hue: 0.0, green_hue: 0.0, blue_hue: 0.0 }.prepare();
        let mut errors: Vec<f32> = (0..20000u32)
            .map(|i| {
                let c = [i.wrapping_mul(2654435761) % 1000, i.wrapping_mul(40503) % 1000, i.wrapping_mul(97) % 1000].map(|v| v as f32 / 999.0);
                let (a, b) = (lut.apply(c), exact.apply(c));
                (0..3).map(|k| (a[k] - b[k]).abs() * 255.0).fold(0.0, f32::max)
            })
            .collect();
        errors.sort_by(f32::total_cmp);
        let (p99, worst) = (errors[errors.len() * 99 / 100], errors[errors.len() - 1]);
        println!("table de tonalité : 99 % à {p99} niveaux, pire {worst}");
        // `Develop` a des ruptures franches (protection des tons chair de la vibrance) : seules elles dépassent 1 niveau.
        assert!(p99 <= 1.0 && worst <= 10.0, "99 % : {p99}, pire : {worst}");
        assert!(ToneLut::key(&RawParams::default()).is_none(), "neutre : pas de table");
    }

    #[test]
    fn orientation_matches_the_exif_convention() {
        let lin = Linear { width: 3, height: 2, channels: 1, data: vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0] };
        // 0 1 2 / 3 4 5, tournée de 90° dans le sens horaire : 3 0 / 4 1 / 5 2.
        let r = lin.clone().oriented(Orientation::Rotate90);
        assert_eq!((r.width, r.height, r.data), (2, 3, vec![3.0, 0.0, 4.0, 1.0, 5.0, 2.0]));
        let r = lin.oriented(Orientation::Rotate270);
        assert_eq!(r.data, vec![2.0, 5.0, 1.0, 4.0, 0.0, 3.0]);
    }

    #[test]
    fn box_blur_is_exact_on_a_constant_and_transpose_round_trips() {
        let (w, h) = (37, 53);
        let plane: Vec<f32> = (0..w * h).map(|i| (i % 11) as f32).collect();
        assert_eq!(transpose(&transpose(&plane, w, h), h, w), plane);
        let flat = vec![0.5f32; w * h];
        assert!(box_blur(&flat, w, h, 4).iter().all(|v| (v - 0.5).abs() < 1e-5));
    }

    // ---------------------------------------------------------- Filtre Camera Raw (calques)

    /// Calque de test : dégradé coloré opaque (toutes les teintes, du sombre au clair), et une bande semi-transparente.
    fn layer_pixels(w: u32, h: u32) -> Vec<u8> {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let (r, g, b) = ((x * 255 / (w - 1)) as u8, (y * 255 / (h - 1)) as u8, ((x + y) * 127 / (w + h)) as u8);
                let a: u8 = if y < 4 { 128 } else { 255 };
                let pre = |v: u8| crate::pixel::mul_div_255(v, a);
                rgba.extend_from_slice(&[pre(r), pre(g), pre(b), a]);
            }
        }
        rgba
    }

    fn worst_difference(a: &[u8], b: &[u8]) -> u8 {
        a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
    }

    #[test]
    fn camera_raw_filter_with_neutral_settings_leaves_the_layer_unchanged() {
        use crate::filters::{apply, Filter};
        let (w, h) = (64u32, 48u32);
        let rgba = layer_pixels(w, h);
        let store = crate::tiles::TileStore::from_premultiplied(w, h, &rgba).unwrap();
        // Réglages neutres, puis « Personnalisée » à la température telle quelle (6 500 K / 0) : rien ne bouge.
        for params in [RawParams::default(), RawParams { white_balance: WhiteBalance::Custom, temperature: LAYER_AS_SHOT.0, tint: LAYER_AS_SHOT.1, ..RawParams::default() }] {
            let out = apply(&store, &Filter::CameraRaw { params }).unwrap().to_premultiplied(w, h);
            let worst = worst_difference(&out, &rgba);
            assert!(worst <= 1, "{params:?} : écart {worst}");
        }
        // La session de la fenêtre (aperçu, histogramme) voit le même calque : « telle quelle » = 6 500 K / 0.
        let session = RawSession::from_layer("Calque 1", w, h, &rgba).unwrap();
        assert_eq!((session.info.as_shot_temperature, session.info.as_shot_tint), LAYER_AS_SHOT);
        assert_eq!(session.info.file_name, "Calque 1");
        let preview = session.develop_preview(&RawParams::default(), ClipView::None);
        // Pixels opaques (y >= 4) : identiques au niveau près.
        let opaque: Vec<u8> = rgba.chunks_exact(4).skip(4 * w as usize).flat_map(|px| [px[0], px[1], px[2]]).collect();
        assert!(worst_difference(&preview.rgb[(4 * w * 3) as usize..], &opaque) <= 1);
    }

    #[test]
    fn camera_raw_filter_exposure_brightens_and_temperature_is_relative() {
        use crate::filters::{apply, Filter};
        let (w, h) = (32u32, 32u32);
        let gray = vec![[100u8, 100, 100, 255]; (w * h) as usize].concat();
        let store = crate::tiles::TileStore::from_premultiplied(w, h, &gray).unwrap();
        let run = |params: RawParams| apply(&store, &Filter::CameraRaw { params }).unwrap().to_premultiplied(w, h);
        let brighter = run(RawParams { exposure: 1.0, ..RawParams::default() });
        let (before, after) = (inverse_gamma(100), inverse_gamma(brighter[(16 * w as usize + 16) * 4 + 1]));
        assert!((after / before - 2.0).abs() < 0.05, "+1 IL : lumière doublée, {before} -> {after}");
        assert_eq!(brighter[3], 255, "alpha conservé");
        // Plus chaud que la référence (6 500 K) : le gris tire vers le jaune ; plus froid : vers le bleu.
        let warm = run(RawParams { white_balance: WhiteBalance::Custom, temperature: 9000.0, ..RawParams::default() });
        let cold = run(RawParams { white_balance: WhiteBalance::Custom, temperature: 4000.0, ..RawParams::default() });
        assert!(warm[0] > warm[2] + 8 && cold[2] > cold[0] + 8, "chaud {:?}, froid {:?}", &warm[..3], &cold[..3]);
    }

    #[test]
    fn camera_raw_filter_respects_the_selection() {
        use crate::document::{Background, Document};
        use crate::filters::Filter;
        use crate::selection::Shape;
        use crate::tiles::{Sampler, TileStore};
        let mut doc = Document::with_background("x", 120, 80, 72.0, Background::White).unwrap();
        let gray = vec![[90u8, 110, 130, 255]; 120 * 80].concat();
        let layer = doc.add_pixel_layer("Photo", TileStore::from_premultiplied(120, 80, &gray).unwrap(), None, 1).unwrap();
        doc.set_selection(Some(Shape::Rect { x: 0.0, y: 0.0, width: 60.0, height: 80.0 }.rasterize(120, 80)));
        doc.apply_filter(layer, &Filter::CameraRaw { params: RawParams { exposure: 1.5, ..RawParams::default() } }).unwrap();
        let mut at = Sampler::new(doc.layer(layer).unwrap().pixels().unwrap());
        let (inside, outside) = (at.get(20, 40), at.get(100, 40));
        assert!(inside[1] > 140, "sélection éclaircie : {inside:?}");
        assert_eq!(outside, [90, 110, 130, 255], "hors sélection : inchangé");
    }

    /// Mesure (lancer avec `--ignored --nocapture`) : ouverture, aperçu et développement complet.
    #[test]
    #[ignore]
    fn timings() {
        let mut paths = vec![fixture("mire.dng")];
        if let Ok(extra) = std::env::var("RAW_BENCH") {
            paths.extend(extra.split(':').map(PathBuf::from));
        }
        let heavy = RawParams { white_balance: WhiteBalance::Custom, temperature: 4800.0, exposure: 0.4, contrast: 10.0, highlights: -60.0, shadows: 35.0, whites: 8.0, blacks: -12.0, clarity: 15.0, vibrance: 20.0, noise_luma: 25.0, noise_color: 25.0, sharpen: 40.0, vignette: 15.0, ..RawParams::default() };
        for path in paths {
            let bytes = std::fs::read(&path).unwrap();
            let t = std::time::Instant::now();
            let session = RawSession::open(Arc::new(bytes.clone()), &path).unwrap();
            let open = t.elapsed();
            let _ = session.develop_preview(&heavy, ClipView::None);
            let t = std::time::Instant::now();
            let preview = session.develop_preview(&heavy, ClipView::None);
            let jpeg = preview.to_jpeg(85).unwrap();
            let preview_time = t.elapsed();
            let t = std::time::Instant::now();
            let plain = session.develop_preview(&RawParams::default(), ClipView::None);
            let plain_time = t.elapsed();
            let t = std::time::Instant::now();
            let full = session.develop_full(&heavy);
            let full_time = t.elapsed();
            let t = std::time::Instant::now();
            let _ = crate::io::decode_premultiplied(&bytes, &path).unwrap();
            let decode = t.elapsed();
            println!(
                "{} : {}x{}, ouverture {open:?}, aperçu {}x{} réglé {preview_time:?} (JPEG {} Ko compris), sans réglage {plain_time:?}, final {full_time:?} ; decode_raw {decode:?}",
                path.display(), full.width, full.height, plain.width, plain.height, jpeg.len() / 1024
            );
        }
    }
    /// Zoom de la fenêtre : une zone en pixels réels est le morceau correspondant du développement final
    /// (bruit, netteté et vignetage compris) ; une zone réduite commence alignée sur le facteur et reste dans l'image.
    /// Une peau réchauffée au-delà du blanc ne fait plus de plaques : le rendu varie continûment (pas de saut entre
    /// voisins) et garde sa teinte, et sans réglage un calque reste intact.
    #[test]
    fn pushed_highlights_stay_smooth() {
        // Dégradé de peau (rouge dominant) de 0,5 à 1 en linéaire.
        let session = synthetic(256, 4, [1.0; 4], |x, _| {
            let v = 0.5 + 0.5 * x as f32 / 255.0;
            [v, v * 0.62, v * 0.45]
        });
        for params in [RawParams { exposure: 1.0, ..RawParams::default() }, RawParams { exposure: 0.6, white_balance: WhiteBalance::Custom, temperature: 9000.0, tint: 10.0, ..RawParams::default() }] {
            let out = session.develop_full(&params);
            let row: Vec<[u8; 3]> = (0..256).map(|x| at(&out, x, 2)).collect();
            for pair in row.windows(2) {
                for k in 0..3 {
                    assert!(pair[0][k].abs_diff(pair[1][k]) <= 4, "saut {:?} -> {:?} ({params:?})", pair[0], pair[1]);
                }
            }
            // Le haut du dégradé reste plus clair que le bas, et le rouge domine encore (pas de plaque blanche nette).
            let (low, high) = (row[0], row[255]);
            assert!(u32::from(high[1]) > u32::from(low[1]), "{low:?} -> {high:?}");
            assert!(high[0] >= high[2], "teinte gardée : {high:?}");
        }
        // Les valeurs juste sous le blanc ne bougent pas sans réglage.
        let neutral = session.develop_full(&RawParams::default());
        let plain: Vec<[u8; 3]> = (0..256).map(|x| at(&neutral, x, 2)).collect();
        assert!(plain[255][0] >= 254);
        assert!(shoulder(0.7, 1.0) == 0.7 && shoulder(1.3, 1.0) == 1.0);
        assert!(shoulder(3.0, 0.6) < 1.0 && shoulder(3.0, 0.6) > 0.99);
    }

    #[test]
    fn region_matches_the_full_develop() {
        let session = synthetic(300, 200, [1.0; 4], |x, y| {
            let n = ((x * 7919 + y * 104_729) % 97) as f32 / 97.0;
            [0.1 + 0.3 * n, 0.2 + 0.1 * ((x / 13) % 2) as f32, 0.05 + 0.4 * (y as f32 / 200.0)]
        });
        let params = RawParams { exposure: 0.4, contrast: 20.0, shadows: 30.0, noise_luma: 60.0, noise_color: 80.0, sharpen: 70.0, vignette: -50.0, ..RawParams::default() };
        let full = session.develop_full(&params);
        let (region, part) = session.develop_region(&params, ClipView::None, 101, 57, 120, 90, 1);
        assert_eq!(region, Region { x: 101, y: 57, width: 120, height: 90 });
        assert_eq!((part.width, part.height), (120, 90));
        // Au niveau 8 bits près : les flous en boîte cumulent leurs sommes depuis un autre bord (arrondis flottants).
        for y in 0..90 {
            let a = ((57 + y) * 300 + 101) * 3;
            for (k, (p, q)) in full.rgb[a..a + 120 * 3].iter().zip(&part.rgb[y * 360..(y + 1) * 360]).enumerate() {
                assert!(p.abs_diff(*q) <= 1, "ligne {y}, octet {k} : {p} contre {q}");
            }
        }
        let (region, part) = session.develop_region(&params, ClipView::None, 251, 151, 500, 500, 2);
        assert_eq!(region, Region { x: 250, y: 150, width: 50, height: 50 });
        assert_eq!((part.width, part.height), (25, 25));
    }
}
