//! Calques de réglage : corrections non destructives appliquées à tout ce qui se trouve dessous.
//!
//! Un réglage est une fonction pure `couleur -> couleur` sur des composantes RVB droites
//! (non prémultipliées) dans `[0, 1]`. Le compositeur l'applique à l'image accumulée sous le
//! calque, puis mélange le résultat selon l'opacité, le masque et le mode de fusion du calque.

use crate::calibration::{self, luma, to_linear, to_srgb, Slider, Step};
use crate::document::GradientStop;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Adjustment {
    /// `brightness` et `contrast` de -100 à 100.
    BrightnessContrast { brightness: f32, contrast: f32 },
    /// `hue` en degrés (-180 à 180), `saturation` et `lightness` de -100 à 100.
    /// `range_width` > 0 : seules les teintes à moins de `range_width` degrés de `range_center` sont
    /// modifiées (avec une transition douce), comme le menu « Rouges, Jaunes… » de Photoshop, la plage
    /// étant ici choisie à la pipette. 0 = toutes les couleurs.
    // Sans ce `rename_all`, les champs partiraient en `range_start` alors que l'interface envoie
    // `rangeStart` : serde les remplacerait silencieusement par zéro, et la plage n'agirait jamais.
    #[serde(rename_all = "camelCase")]
    HueSaturation {
        hue: f32,
        saturation: f32,
        lightness: f32,
        /// Début et fin de la plage pleine, en degrés sur le cercle des teintes, parcourue de
        /// `range_start` vers `range_end` dans le sens croissant. Égaux : toutes les couleurs.
        #[serde(default)]
        range_start: f32,
        #[serde(default)]
        range_end: f32,
        /// Largeur des transitions, avant le début et après la fin, en degrés.
        #[serde(default)]
        feather_in: f32,
        #[serde(default)]
        feather_out: f32,
        /// Ancienne plage symétrique (centre et demi-largeur), d'avant les quatre poignées. Elle
        /// n'agit que si les quatre poignées sont à zéro, et l'interface la remet à zéro dès qu'on
        /// touche à la plage. Toujours enregistrée, pour qu'un aller-retour par le fichier ne
        /// change rien à l'image.
        #[serde(default)]
        range_center: f32,
        #[serde(default)]
        range_width: f32,
    },
    /// Niveaux d'entrée : `black` et `white` de 0 à 255, `gamma` de 0,1 à 10 (1 = neutre).
    Levels { black: f32, gamma: f32, white: f32 },
    /// Inverser (Ctrl+I) : les couleurs opposées, avec des tons moyens légèrement assombris (voir
    /// `invert`), **le papier restant blanc** : le blanc pur n'est pas retourné (voir `paper_weight`).
    /// C'est ce qui le distingue de Négatif sur les références : les bandes y sont identiques au
    /// niveau près, seul le fond blanc change — conservé par Inverser, noir sous Négatif.
    /// En calque de réglage, il est créé écrêté : il ne retourne que le calque visé.
    Invert,
    /// Négatif : la même courbe qu'Inverser, mais sur tout — le blanc devient noir. Le calque est créé
    /// libre, sur tout ce qui est dessous : l'image entière devient un négatif, comme une pellicule.
    Negative,
    /// Transfert de dégradé (Photoshop : Courbe de transfert de dégradé) : la luminosité de chaque
    /// pixel choisit une couleur le long du dégradé — le noir prend l'arrêt de gauche, le blanc
    /// celui de droite. L'opacité d'un arrêt dose le mélange avec la couleur d'origine.
    /// `reverse` : dégradé parcouru à l'envers ; `dither` : léger bruit qui casse les paliers ;
    /// `smoothness` (0 à 100 %) : transitions adoucies entre arrêts, comme le Lissage de Photoshop.
    #[serde(rename_all = "camelCase")]
    GradientMap {
        stops: Vec<GradientStop>,
        #[serde(default)]
        reverse: bool,
        #[serde(default)]
        dither: bool,
        #[serde(default = "default_smoothness")]
        smoothness: f32,
    },
    /// Développement façon Lightroom : tous les curseurs de -100 à 100, sauf `exposure` (-5 à +5 IL).
    #[serde(rename_all = "camelCase")]
    Develop {
        temperature: f32,
        tint: f32,
        exposure: f32,
        contrast: f32,
        highlights: f32,
        shadows: f32,
        whites: f32,
        blacks: f32,
        clarity: f32,
        dehaze: f32,
        vibrance: f32,
        saturation: f32,
        /// Décalage de teinte des rouges, des verts et des bleus primaires.
        red_hue: f32,
        green_hue: f32,
        blue_hue: f32,
    },
}

impl Adjustment {
    pub fn label(&self) -> &'static str {
        match self {
            Self::BrightnessContrast { .. } => "Luminosité/Contraste",
            Self::HueSaturation { .. } => "Teinte/Saturation",
            Self::Levels { .. } => "Niveaux",
            Self::Invert => "Inverser",
            Self::Negative => "Négatif",
            Self::GradientMap { .. } => "Transfert de dégradé",
            Self::Develop { .. } => "Développement",
        }
    }

    /// Prépare le réglage pour l'appliquer à beaucoup de pixels : les réglages par composante
    /// deviennent une simple table de 256 valeurs.
    pub fn prepare(&self) -> Prepared {
        let curve = |f: &dyn Fn(f32) -> f32| {
            let mut table = [0.0f32; 256];
            for (i, slot) in table.iter_mut().enumerate() {
                *slot = f(i as f32 / 255.0).clamp(0.0, 1.0);
            }
            Prepared::Curve(Box::new(table))
        };
        match self.clone() {
            Self::BrightnessContrast { brightness, contrast } => {
                let offset = brightness.clamp(-100.0, 100.0) / 100.0 * 0.5;
                // Pente autour du gris moyen : 0 à -100, 1 à 0, forte à +100.
                let c = contrast.clamp(-100.0, 100.0) / 100.0;
                let slope = if c >= 0.0 { 1.0 / (1.0 - c * 0.9) } else { 1.0 + c };
                curve(&|v| (v - 0.5) * slope + 0.5 + offset)
            }
            Self::Levels { black, gamma, white } => {
                let lo = black.clamp(0.0, 254.0) / 255.0;
                let hi = (white.clamp(1.0, 255.0) / 255.0).max(lo + 1.0 / 255.0);
                let exponent = 1.0 / gamma.clamp(0.1, 10.0);
                curve(&|v| ((v - lo) / (hi - lo)).clamp(0.0, 1.0).powf(exponent))
            }
            Self::Invert => Prepared::Invert { table: Box::new(invert_curve()), keep_paper: true },
            Self::Negative => Prepared::Invert { table: Box::new(invert_curve()), keep_paper: false },
            Self::GradientMap { stops, reverse, dither, smoothness } => Prepared::GradientMap(Box::new(gradient_map_table(&stops, reverse, smoothness)), dither),
            Self::Develop { temperature, tint, exposure, contrast, highlights, shadows, whites, blacks, clarity, dehaze, vibrance, saturation, red_hue, green_hue, blue_hue } => {
                // Valeurs telles que l'interface les affiche : la calibration cherche ses modèles
                // à ces valeurs-là (-100, +100, et l'exposition en IL).
                Prepared::Develop(Box::new(Develop::new(
                    temperature, tint, exposure, contrast, highlights, shadows, whites, blacks,
                    clarity, dehaze, vibrance, saturation, [red_hue, green_hue, blue_hue],
                )))
            }
            Self::HueSaturation { hue, saturation, lightness, range_start, range_end, feather_in, feather_out, range_center, range_width } => Prepared::HueSaturation {
                hue: hue.clamp(-180.0, 180.0) / 360.0,
                saturation: saturation.clamp(-100.0, 100.0) / 100.0,
                lightness: lightness.clamp(-100.0, 100.0) / 100.0,
                range: HueRange::of(range_start, range_end, feather_in, feather_out, range_center, range_width),
            },
        }
    }
}

/// Réglage « Développement », calé sur le panneau Réglages de base de Lightroom.
///
/// Treize curseurs sur quinze ont été **mesurés** : voir `crate::calibration`. Chacun devient un
/// étage appliqué dans l'ordre du panneau. Ne restent en formule écrite à la main que `Contraste`
/// et `Clarté`, les deux seuls qui n'ont jamais été capturés.
pub struct Develop {
    /// Étages mesurés, dans l'ordre du panneau.
    steps: Vec<Step>,
    /// Faux quand rien d'autre que les étages mesurés n'agit : on saute alors tout l'ancien chemin.
    legacy: bool,
    /// Étalonnage puis balance des blancs, réunis en une matrice 3 x 3 appliquée en lumière linéaire.
    matrix: [[f32; 3]; 3],
    exposure: f32,
    /// Courbe de tonalité : luminance d'entrée (0..1, espace perceptuel) -> luminance de sortie.
    tone: Box<[f32; 1025]>,
    clarity: f32,
    dehaze: f32,
    vibrance: f32,
    saturation: f32,
}

/// Épaule douce : au-delà de 0,8 la valeur est comprimée vers 1 au lieu d'être coupée net, comme
/// la récupération des hautes lumières de Lightroom quand on pousse l'exposition.
fn shoulder(v: f32) -> f32 {
    const KNEE: f32 = 0.8;
    if v <= KNEE { v } else { KNEE + (1.0 - KNEE) * (1.0 - (-(v - KNEE) / (1.0 - KNEE)).exp()) }
}

/// Cloche lisse centrée en `center`, nulle au-delà de `width` : zone d'action d'un curseur de tonalité.
fn bell(x: f32, center: f32, width: f32) -> f32 {
    let t = ((x - center) / width).abs();
    if t >= 1.0 { 0.0 } else { (1.0 - t * t).powi(2) }
}

#[allow(clippy::too_many_arguments)]
impl Develop {
    /// Toutes les valeurs sont celles de l'interface : de -100 à 100, et l'exposition en IL.
    fn new(temperature: f32, tint: f32, exposure: f32, contrast: f32, highlights: f32, shadows: f32, whites: f32, blacks: f32, clarity: f32, dehaze: f32, vibrance: f32, saturation: f32, primaries: [f32; 3]) -> Self {
        // Ce qui a été mesuré devient un étage ; ce qui ne l'a pas été retombe sur l'ancienne
        // formule, plus bas. Un curseur mesuré est donc mis à zéro pour l'ancien chemin.
        let mut steps = Vec::new();
        let mut keep = |slider: Slider, value: f32, raw: f32| -> f32 {
            match calibration::step(slider, value) {
                Some(step) => {
                    steps.push(step);
                    0.0
                }
                None => raw,
            }
        };
        // L'ordre est celui du panneau Lightroom : balance des blancs, tonalité, présence, couleur.
        let temperature = keep(Slider::Temperature, temperature, temperature);
        let tint = keep(Slider::Tint, tint, tint);
        let exposure_left = keep(Slider::Exposure, exposure, exposure);
        let highlights = keep(Slider::Highlights, highlights, highlights);
        let shadows = keep(Slider::Shadows, shadows, shadows);
        let whites = keep(Slider::Whites, whites, whites);
        let blacks = keep(Slider::Blacks, blacks, blacks);
        let dehaze = keep(Slider::Dehaze, dehaze, dehaze);
        let vibrance = keep(Slider::Vibrance, vibrance, vibrance);
        let saturation = keep(Slider::Saturation, saturation, saturation);
        let primaries = [
            keep(Slider::RedHue, primaries[0], primaries[0]),
            keep(Slider::GreenHue, primaries[1], primaries[1]),
            keep(Slider::BlueHue, primaries[2], primaries[2]),
        ];
        // Ce qui reste est réduit à -1..1, comme l'attend l'ancien chemin.
        let unit = |v: f32| v.clamp(-100.0, 100.0) / 100.0;
        let (contrast, clarity) = (unit(contrast), unit(clarity));
        let (temperature, tint) = (unit(temperature), unit(tint));
        let (highlights, shadows) = (unit(highlights), unit(shadows));
        let (whites, blacks) = (unit(whites), unit(blacks));
        let (dehaze, vibrance, saturation) = (unit(dehaze), unit(vibrance), unit(saturation));
        let primaries = primaries.map(unit);
        let legacy = contrast != 0.0
            || clarity != 0.0
            || exposure_left != 0.0
            || [temperature, tint, highlights, shadows, whites, blacks, dehaze, vibrance, saturation].iter().any(|v| *v != 0.0)
            || primaries.iter().any(|v| *v != 0.0);

        // Étalonnage : chaque primaire glisse vers l'une de ses voisines (rouge -> orange ou magenta…).
        // Toute couleur qui contient cette primaire bouge avec elle, contrairement à un réglage TSL.
        let mut matrix = [[0.0f32; 3]; 3];
        for (primary, shift) in primaries.iter().enumerate() {
            let k = shift.clamp(-1.0, 1.0) * 0.35;
            let (toward, amount) = if k >= 0.0 { ((primary + 1) % 3, k) } else { ((primary + 2) % 3, -k) };
            matrix[primary][primary] += 1.0 - amount;
            matrix[toward][primary] += amount;
        }
        // Balance des blancs : gains rouge/bleu opposés (température) et vert (nuance), normalisés pour
        // que la luminance d'un gris ne change pas — on réchauffe sans éclaircir.
        let (t, n) = (temperature.clamp(-1.0, 1.0), tint.clamp(-1.0, 1.0));
        let gains = [1.0 + 0.45 * t - 0.12 * n.min(0.0), 1.0, 1.0 - 0.45 * t + 0.20 * n.max(0.0)];
        let gains = [gains[0].max(0.05), (1.0 - 0.30 * n).max(0.05), gains[2].max(0.05)];
        let norm = 1.0 / luma(gains);
        for (row, gain) in matrix.iter_mut().zip(gains) {
            for cell in row.iter_mut() {
                *cell *= gain * norm;
            }
        }

        // Courbe de tonalité. Chaque curseur a sa zone, comme l'histogramme de Lightroom : noirs tout en
        // bas, ombres vers le quart, hautes lumières vers les trois quarts, blancs tout en haut.
        let mut tone = Box::new([0.0f32; 1025]);
        for (i, slot) in tone.iter_mut().enumerate() {
            let x = i as f32 / 1024.0;
            let mut y = x;
            y += shadows * 0.28 * bell(x, 0.25, 0.42) * (x * 6.0).min(1.0);
            y += highlights * 0.28 * bell(x, 0.75, 0.42) * ((1.0 - x) * 6.0).min(1.0);
            // Blancs et noirs déplacent les points de coupure : +100 de blancs amène 0,8 à 1 ; -100 de noirs amène 0,12 à 0.
            let (white, black) = (1.0 - whites * 0.2, -blacks * 0.12);
            y = (y - black) / (white - black).max(0.05);
            // Contraste : sigmoïde autour du gris moyen ; négatif, la courbe s'aplatit vers ce gris.
            let y0 = y.clamp(0.0, 1.0);
            let c = contrast.clamp(-1.0, 1.0);
            y = if c >= 0.0 {
                if y0 < 0.5 { 0.5 * (2.0 * y0).powf(1.0 + c * 1.6) } else { 1.0 - 0.5 * (2.0 * (1.0 - y0)).powf(1.0 + c * 1.6) }
            } else {
                y0 + (0.5 - y0) * (-c) * 0.65
            };
            *slot = y.clamp(0.0, 1.0);
        }
        Self { steps, legacy, matrix, exposure: 2f32.powf(exposure_left.clamp(-5.0, 5.0)), tone, clarity, dehaze, vibrance, saturation }
    }

    /// Les étages mesurés, pour les envoyer au shader.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    pub fn has_legacy(&self) -> bool {
        self.legacy
    }

    fn tone_at(&self, x: f32) -> f32 {
        let position = x.clamp(0.0, 1.0) * 1024.0;
        let i = (position as usize).min(1023);
        self.tone[i] + (self.tone[i + 1] - self.tone[i]) * (position - i as f32)
    }

    fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        // 0. Les étages mesurés sur Lightroom, chacun fondu vers la mesure voisine.
        let mut rgb = rgb;
        for step in &self.steps {
            let above = step.above.apply(rgb);
            let below = match step.below {
                Some(model) => model.apply(rgb),
                None => rgb,
            };
            rgb = [0, 1, 2].map(|i| below[i] + (above[i] - below[i]) * step.mix);
        }
        if !self.legacy {
            return rgb;
        }

        // 1. Lumière linéaire : étalonnage, balance des blancs, exposition, puis retour en sRGB avec une épaule douce.
        let linear = rgb.map(|v| to_linear(v.clamp(0.0, 1.0)));
        let m = &self.matrix;
        // L'épaule n'intervient que si l'exposition pousse la lumière : à 0 IL, l'image n'est pas touchée (un blanc reste blanc).
        let rolloff = ((self.exposure - 1.0) * 2.0).clamp(0.0, 1.0);
        let mut c = [0, 1, 2].map(|row| {
            let v = (m[row][0] * linear[0] + m[row][1] * linear[1] + m[row][2] * linear[2]).max(0.0) * self.exposure;
            to_srgb((v + (shoulder(v) - v) * rolloff).min(1.0))
        });

        // 2. Tonalité : la courbe agit sur la luminance, les trois couches suivent dans le même rapport (la teinte ne dérive pas).
        let l = luma(c);
        if l > 1e-5 {
            let mut target = self.tone_at(l);
            // Correction du voile (approximation par pixel) : noirs redescendus, contraste et saturation renforcés.
            target = ((target - self.dehaze * 0.10 * (1.0 - target)) / (1.0 - self.dehaze * 0.10).max(0.3)).clamp(0.0, 1.0);
            // Clarté (approximation par pixel) : contraste réservé aux tons moyens.
            let mid = 1.0 - (2.0 * target - 1.0).abs();
            let pushed = if target < 0.5 { 0.5 * (2.0 * target).powf(1.0 + self.clarity.abs() * 0.9) } else { 1.0 - 0.5 * (2.0 * (1.0 - target)).powf(1.0 + self.clarity.abs() * 0.9) };
            target += (pushed - target) * mid * self.clarity.signum();
            let gain = target.max(0.0) / l;
            c = c.map(|v| v * gain);
        }

        // 3. Couleur, autour de la luminance (comme Lightroom) et non en TSL : saturation à -100 = noir et blanc fidèle aux valeurs.
        let l = luma(c);
        let chroma = c.map(|v| v - l);
        let (max, min) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        let current = if max > 1e-5 { (max - min) / max } else { 0.0 };
        // Vibrance : forte sur les couleurs ternes, presque nulle sur les couleurs vives, et retenue sur les tons chair (orangés).
        let skin = if c[0] > c[1] && c[1] > c[2] { 1.0 - 0.5 * bell((c[1] - c[2]) / (c[0] - c[2]).max(1e-5), 0.55, 0.45) } else { 1.0 };
        let vibrance = if self.vibrance >= 0.0 { self.vibrance * (1.0 - current).powi(2) * skin * 1.4 } else { self.vibrance * (1.0 - current * 0.5) };
        let factor = ((1.0 + self.saturation) * (1.0 + vibrance) * (1.0 + self.dehaze.max(0.0) * 0.2)).max(0.0);
        [0, 1, 2].map(|i| (l + chroma[i] * factor).clamp(0.0, 1.0))
    }
}

/// Plage de teintes visée par Teinte/Saturation, en tours (0 à 1), comme la barre à quatre poignées
/// de l'interface : `start` et `end` bornent la zone corrigée en entier, `feather_in` et
/// `feather_out` la largeur des transitions de part et d'autre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HueRange {
    pub start: f32,
    pub end: f32,
    pub feather_in: f32,
    pub feather_out: f32,
}

impl HueRange {
    /// Construit la plage à partir des degrés de l'interface. `center`/`width` sont l'ancienne
    /// plage symétrique : ils ne servent que si les quatre poignées sont absentes du fichier.
    fn of(start: f32, end: f32, feather_in: f32, feather_out: f32, center: f32, width: f32) -> Option<Self> {
        let turns = |degrees: f32| degrees.rem_euclid(360.0) / 360.0;
        let span = (end - start).rem_euclid(360.0);
        if span > 0.0 {
            return Some(Self {
                start: turns(start),
                end: turns(end),
                feather_in: feather_in.clamp(0.0, 180.0) / 360.0,
                feather_out: feather_out.clamp(0.0, 180.0) / 360.0,
            });
        }
        // Fichier d'avant les quatre poignées : la plage symétrique reprend son allure d'origine,
        // transition sur le dernier tiers comprise.
        if width > 0.0 {
            let half = width.clamp(5.0, 180.0);
            let full = half * 0.67;
            let fade = (half - full).max(0.0);
            return Some(Self {
                start: turns(center - full),
                end: turns(center + full),
                feather_in: fade / 360.0,
                feather_out: fade / 360.0,
            });
        }
        None
    }

    /// Part du réglage reçue par une teinte (en tours) : entière au cœur, dégressive dans les
    /// transitions, nulle au-delà. Même calcul dans le shader (`composite.wgsl`).
    pub fn coverage(&self, hue: f32) -> f32 {
        let span = (self.end - self.start).rem_euclid(1.0);
        let inside = (hue - self.start).rem_euclid(1.0);
        if inside <= span {
            return 1.0;
        }
        let before = (self.start - hue).rem_euclid(1.0);
        if before <= self.feather_in {
            return smooth(1.0 - before / self.feather_in.max(1e-4));
        }
        let after = (hue - self.end).rem_euclid(1.0);
        if after <= self.feather_out {
            return smooth(1.0 - after / self.feather_out.max(1e-4));
        }
        0.0
    }
}

/// Lissage en S : une transition qui démarre et s'arrête en douceur.
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Inversion d'une composante (Ctrl+I), calée sur des références mesurées : ce n'est **pas**
/// le négatif `1 - v`. Sur deux rampes de gris inversées dans Photoshop, les tons moyens
/// ressortent plus sombres d'environ 12 niveaux sur 255 (128 → 116, 64 → 185 au lieu de 191),
/// les extrêmes restant exacts. Cet affaissement, symétrique, est reproduit par une cloche en
/// sinus ; l'écart aux mesures est de 1,5 niveau, sous le bruit JPEG des références. La même
/// courbe s'applique aux trois composantes, ce que l'arc-en-ciel des références confirme.
pub fn invert(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    const SAG: f32 = 11.5 / 255.0;
    // Le sinus est borné à zéro : en 1,0 il vaut -1e-7, dont la puissance non entière serait NaN.
    let bell = (std::f32::consts::PI * v).sin().max(0.0).powf(2.75);
    (1.0 - v - SAG * bell).clamp(0.0, 1.0)
}

/// La courbe `invert` en table de 256 valeurs, comme les autres réglages par composante.
pub fn invert_curve() -> [f32; 256] {
    let mut table = [0.0f32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        *slot = invert(i as f32 / 255.0);
    }
    table
}

/// Part d'un pixel qui est « du papier » : du blanc pur, que l'inversion laisse blanc. Un pixel dont
/// la composante la plus faible est à 254 ou plus est du papier ; à 248 ou moins, de la couleur ;
/// entre les deux, une transition douce qui absorbe le bruit JPEG au bord des zones blanches.
/// Même formule dans le shader (`paper_weight`).
pub fn paper_weight(rgb: [f32; 3]) -> f32 {
    let low = rgb[0].min(rgb[1]).min(rgb[2]);
    smooth((low - 248.0 / 255.0) / (6.0 / 255.0))
}

/// La même inversion, en table sur 8 bits, pour l'opération destructive sur les pixels.
pub fn invert_table() -> [u8; 256] {
    let mut table = [0u8; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        *slot = (invert(i as f32 / 255.0) * 255.0 + 0.5) as u8;
    }
    table
}

fn default_smoothness() -> f32 {
    100.0
}

/// Luminosité qui indexe le transfert de dégradé : la pondération vidéo classique sur les
/// composantes telles qu'elles sont affichées, celle du passage en niveaux de gris de Photoshop.
/// (Pas celle de `calibration::luma`, qui sert au développement en lumière linéaire.)
pub fn gradient_luma([r, g, b]: [f32; 3]) -> f32 {
    0.299 * r + 0.587 * g + 0.114 * b
}

/// Couleur du transfert pour un pixel : la luminosité (décalée du bruit de tramage, en fraction
/// de 0 à 1) choisit l'entrée de la table, interpolée entre deux niveaux.
fn gradient_lookup(table: &[[f32; 4]; 256], rgb: [f32; 3], noise: f32) -> [f32; 3] {
    let position = (gradient_luma(rgb) + noise).clamp(0.0, 1.0) * 255.0;
    let index = position as usize;
    let next = (index + 1).min(255);
    let k = position - index as f32;
    let mut out = [0.0f32; 3];
    let alpha = table[index][3] + (table[next][3] - table[index][3]) * k;
    for c in 0..3 {
        let mapped = table[index][c] + (table[next][c] - table[index][c]) * k;
        // L'opacité d'un arrêt dose le mélange avec la couleur d'origine.
        out[c] = rgb[c] + (mapped - rgb[c]) * alpha;
    }
    out
}

/// Lecture d'une table par composante, interpolée entre deux entrées : pas de paliers visibles sur
/// des dégradés doux.
fn curve_lookup(table: &[f32; 256], v: f32) -> f32 {
    let position = v.clamp(0.0, 1.0) * 255.0;
    let index = position as usize;
    let next = (index + 1).min(255);
    table[index] + (table[next] - table[index]) * (position - index as f32)
}

/// Bruit de tramage dans [0, 1) pour le pixel `index` d'une tuile (même formule dans le shader).
pub fn dither_noise(index: usize) -> f32 {
    let (x, y) = ((index % 256) as f32, (index / 256) as f32);
    let v = (x * 12.9898 + y * 78.233).sin() * 43758.547;
    v - v.floor()
}

/// Table du transfert de dégradé : 256 couleurs RGBA (alpha droit), une par niveau de luminosité.
/// Entre deux arrêts, la transition est linéaire ; le lissage la rapproche d'une marche douce
/// (smoothstep) sans en changer les extrémités — à 100 %, c'est celle du Lissage de Photoshop.
pub fn gradient_map_table(stops: &[GradientStop], reverse: bool, smoothness: f32) -> [[f32; 4]; 256] {
    let mut sorted: Vec<GradientStop> = stops.to_vec();
    sorted.sort_by(|a, b| a.position.total_cmp(&b.position));
    if sorted.is_empty() {
        sorted.push(GradientStop { position: 0.0, color: [0, 0, 0, 255] });
    }
    if sorted.len() == 1 {
        sorted.push(sorted[0]);
        sorted[1].position = 1.0;
    }
    let ease = smoothness.clamp(0.0, 100.0) / 100.0;
    let mut table = [[0.0f32; 4]; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut t = i as f32 / 255.0;
        if reverse {
            t = 1.0 - t;
        }
        let after = sorted.iter().position(|s| s.position >= t).unwrap_or(sorted.len() - 1);
        let (a, b) = (sorted[after.saturating_sub(1)], sorted[after]);
        let span = b.position - a.position;
        let u = if span > 1e-6 { ((t - a.position) / span).clamp(0.0, 1.0) } else { 1.0 };
        let u = u + (smooth(u) - u) * ease;
        for c in 0..4 {
            slot[c] = (f32::from(a.color[c]) + (f32::from(b.color[c]) - f32::from(a.color[c])) * u) / 255.0;
        }
    }
    table
}

/// Réglage mis à plat, prêt à être appliqué pixel par pixel.
pub enum Prepared {
    Develop(Box<Develop>),
    Curve(Box<[f32; 256]>),
    /// Inverser / Négatif : la courbe `invert`, et si le papier (blanc pur) reste blanc.
    Invert { table: Box<[f32; 256]>, keep_paper: bool },
    /// Couleur RGBA (alpha droit) par niveau de luminosité, et tramage ou non.
    GradientMap(Box<[[f32; 4]; 256]>, bool),
    /// `range` : plage de teintes visée, ou `None` pour toutes les couleurs.
    HueSaturation { hue: f32, saturation: f32, lightness: f32, range: Option<HueRange> },
}

/// Type de réglage tel que le lit un shader (mêmes valeurs dans `composite.wgsl`).
pub const GPU_CURVE: u32 = 1;
pub const GPU_HUE_SATURATION: u32 = 2;
pub const GPU_DEVELOP: u32 = 3;
pub const GPU_GRADIENT_MAP: u32 = 4;
pub const GPU_INVERT: u32 = 5;

/// Réglage mis à plat pour la carte graphique : un type, puis la suite des valeurs (tables comprises).
/// Le shader lit exactement les mêmes nombres que `Prepared::apply` et fait les mêmes calculs.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuAdjustment {
    pub kind: u32,
    pub values: Vec<f32>,
}

impl Prepared {
    /// Description destinée à un shader.
    ///
    /// - `GPU_CURVE` : les 256 entrées de la table.
    /// - `GPU_HUE_SATURATION` : teinte, saturation, luminosité, plage active (0 ou 1), début, fin,
    ///   transition avant le début, transition après la fin (tout en tours).
    /// - `GPU_GRADIENT_MAP` : tramage (0 ou 1), puis les 256 couleurs RGBA de la table (1024 valeurs).
/// - `GPU_INVERT` : papier conservé (0 ou 1), puis les 256 entrées de la courbe.
/// - `GPU_DEVELOP` : les 9 cases de la matrice (ligne par ligne), exposition, clarté, voile, vibrance,
    ///   saturation, puis les 1025 entrées de la courbe de tonalité.
    pub fn to_gpu(&self) -> GpuAdjustment {
        match self {
            Self::Curve(table) => GpuAdjustment { kind: GPU_CURVE, values: table.to_vec() },
            Self::Invert { table, keep_paper } => {
                let mut values = Vec::with_capacity(1 + 256);
                values.push(f32::from(u8::from(*keep_paper)));
                values.extend_from_slice(&table[..]);
                GpuAdjustment { kind: GPU_INVERT, values }
            }
            Self::GradientMap(table, dither) => {
                let mut values = Vec::with_capacity(1 + 256 * 4);
                values.push(f32::from(u8::from(*dither)));
                values.extend(table.iter().flatten().copied());
                GpuAdjustment { kind: GPU_GRADIENT_MAP, values }
            }
            Self::HueSaturation { hue, saturation, lightness, range } => {
                let plage = range.unwrap_or(HueRange { start: 0.0, end: 0.0, feather_in: 0.0, feather_out: 0.0 });
                GpuAdjustment {
                    kind: GPU_HUE_SATURATION,
                    values: vec![*hue, *saturation, *lightness, f32::from(u8::from(range.is_some())), plage.start, plage.end, plage.feather_in, plage.feather_out],
                }
            }
            Self::Develop(develop) => {
                // En-tête : nombre d'étages mesurés, présence de l'ancien chemin, et où il commence.
                let mut values = vec![develop.steps().len() as f32, f32::from(u8::from(develop.has_legacy())), 0.0];
                for step in develop.steps() {
                    values.push(f32::from(u8::from(step.below.is_some())));
                    values.push(step.mix);
                    if let Some(model) = step.below {
                        values.extend_from_slice(model.raw());
                    }
                    values.extend_from_slice(step.above.raw());
                }
                values[2] = values.len() as f32;
                if develop.has_legacy() {
                    values.extend(develop.matrix.iter().flatten().copied());
                    values.extend([develop.exposure, develop.clarity, develop.dehaze, develop.vibrance, develop.saturation]);
                    values.extend(develop.tone.iter().copied());
                }
                GpuAdjustment { kind: GPU_DEVELOP, values }
            }
        }
    }
}

impl Prepared {
    /// Le réglage pour le pixel `index` d'une tuile : seul le transfert de dégradé tramé s'en sert,
    /// pour tirer un bruit propre à chaque pixel. Les autres réglages ignorent la position.
    pub fn apply_at(&self, rgb: [f32; 3], index: usize) -> [f32; 3] {
        match self {
            Self::GradientMap(table, true) => {
                // Bruit à ±1 niveau sur la luminosité : les paliers d'un dégradé pauvre se fondent.
                let noise = (dither_noise(index) - 0.5) * 2.0 / 255.0;
                gradient_lookup(table, rgb, noise)
            }
            _ => self.apply(rgb),
        }
    }

    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Develop(develop) => develop.apply(rgb),
            Self::Curve(table) => rgb.map(|v| curve_lookup(table, v)),
            Self::Invert { table, keep_paper } => {
                let inverted = rgb.map(|v| curve_lookup(table, v));
                if !keep_paper {
                    return inverted;
                }
                let paper = paper_weight(rgb);
                [0, 1, 2].map(|c| inverted[c] + (rgb[c] - inverted[c]) * paper)
            }
            Self::GradientMap(table, _) => gradient_lookup(table, rgb, 0.0),
            Self::HueSaturation { hue, saturation, lightness, range } => {
                let (h, s, l) = rgb_to_hsl(rgb);
                // Part du réglage reçue par ce pixel : celle de la plage à quatre poignées, et nulle
                // pour un gris, qui n'a pas de teinte à viser.
                let weight = range.map_or(1.0, |range| range.coverage(h) * (s * 8.0).min(1.0));
                if weight <= 0.0 {
                    return rgb;
                }
                let (saturation, lightness) = (saturation * weight, lightness * weight);
                let s = if saturation >= 0.0 { s + (1.0 - s) * saturation } else { s * (1.0 + saturation) };
                let l = if lightness >= 0.0 { l + (1.0 - l) * lightness } else { l * (1.0 + lightness) };
                hsl_to_rgb(((h + hue * weight).rem_euclid(1.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0)))
            }
        }
    }
}

fn rgb_to_hsl([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6);
    let h = if max == r { ((g - b) / d).rem_euclid(6.0) } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    (h / 6.0, s.min(1.0), l)
}

fn hsl_to_rgb((h, s, l): (f32, f32, f32)) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h6 = h * 6.0;
    let x = c * (1.0 - (h6.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 0.01)
    }

    #[test]
    fn neutral_settings_change_nothing() {
        let color = [0.2, 0.55, 0.9];
        for neutral in [
            Adjustment::BrightnessContrast { brightness: 0.0, contrast: 0.0 },
            Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 0.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 0.0, range_width: 0.0 },
            Adjustment::Levels { black: 0.0, gamma: 1.0, white: 255.0 },
            develop(|_| {}),
        ] {
            assert!(close(neutral.prepare().apply(color), color), "{neutral:?}");
            assert!(close(neutral.prepare().apply([1.0; 3]), [1.0; 3]) && close(neutral.prepare().apply([0.0; 3]), [0.0; 3]), "blanc et noir intacts : {neutral:?}");
        }
    }

    fn develop(set: impl Fn(&mut [f32; 15])) -> Adjustment {
        let mut v = [0.0f32; 15];
        set(&mut v);
        Adjustment::Develop {
            temperature: v[0], tint: v[1], exposure: v[2], contrast: v[3], highlights: v[4], shadows: v[5], whites: v[6], blacks: v[7],
            clarity: v[8], dehaze: v[9], vibrance: v[10], saturation: v[11], red_hue: v[12], green_hue: v[13], blue_hue: v[14],
        }
    }

    #[test]
    fn develop_behaves_like_the_measurements() {
        // Ce test décrit ce que Lightroom fait *vraiment*, relevé sur la mire. Les seuils viennent
        // des mesures, pas d'une intuition : voir `crate::calibration`.
        let run = |index: usize, value: f32, color: [f32; 3]| develop(|v| v[index] = value).prepare().apply(color);
        let (dark, mid, light) = ([0.12; 3], [0.58; 3], [0.88; 3]);
        let neutral = |c: [f32; 3]| c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]) < 0.02;

        // Exposition : monotone, et les extrêmes butent là où Lightroom les fait buter.
        let ladder: Vec<f32> = [-5.0, -3.24, -1.0, 0.0, 1.0, 1.32, 5.0].iter().map(|ev| run(2, *ev, mid)[1]).collect();
        assert!(ladder.windows(2).all(|p| p[1] >= p[0] - 1e-4), "exposition non monotone : {ladder:?}");
        assert!(ladder[0] < 0.06, "-5 IL : presque noir, {}", ladder[0]);
        assert!(ladder[6] > 0.99, "+5 IL : écrêté au blanc, {}", ladder[6]);
        assert!(neutral(run(2, 1.0, mid)), "l'exposition ne doit pas teinter un gris");

        // Balance des blancs : le sens est mesuré, et le réglage n'est pas neutre en clarté.
        let warm = run(0, 100.0, mid);
        assert!(warm[0] > warm[2] + 0.3, "+100 : chaud, {warm:?}");
        let cool = run(0, -100.0, mid);
        assert!(cool[2] > cool[0] + 0.2, "-100 : froid, {cool:?}");
        assert!(run(1, 100.0, mid)[1] < run(1, 100.0, mid)[0], "nuance positive : vers le magenta");
        assert!(run(1, -100.0, mid)[1] > run(1, -100.0, mid)[0], "nuance négative : vers le vert");

        // Tonalité : chaque curseur agit dans sa zone, et laisse un gris neutre.
        assert!(run(5, 100.0, dark)[1] > dark[1] + 0.3, "ombres +100 : Lightroom relève très fort le noir");
        assert!(run(4, -100.0, light)[1] < light[1] - 0.1, "hautes lumières -100");
        assert!((run(4, -100.0, dark)[1] - dark[1]).abs() < 0.06, "et elles laissent les ombres tranquilles");
        assert!(run(7, -100.0, dark)[1] < 0.06, "noirs -100 : les ombres se bouchent");
        assert!(run(6, 100.0, light)[1] > 0.97, "blancs +100 : les clairs montent au blanc");
        for index in [4, 5, 6, 7] {
            assert!(neutral(run(index, 100.0, mid)) && neutral(run(index, -100.0, mid)), "le curseur {index} teinte un gris");
        }

        // Contraste et clarté n'ont pas été mesurés : ils gardent l'ancienne formule.
        let (deep, bright) = (run(3, 100.0, [0.25; 3])[0], run(3, 100.0, [0.75; 3])[0]);
        assert!(deep < 0.17 && bright > 0.83 && (run(3, 100.0, [0.5; 3])[0] - 0.5).abs() < 0.01, "{deep} {bright}");
        assert!(run(3, -100.0, [0.25; 3])[0] > 0.35, "contraste négatif : tout se rapproche du gris moyen");
        assert!(run(8, 100.0, [0.35; 3])[0] < 0.33 && run(8, 100.0, [0.65; 3])[0] > 0.67, "clarté");

        // Saturation : -100 donne un noir et blanc qui garde les valeurs.
        let (yellow, blue) = (run(11, -100.0, [0.9, 0.9, 0.1]), run(11, -100.0, [0.1, 0.1, 0.9]));
        assert!(neutral(yellow) && neutral(blue) && yellow[0] > blue[0] + 0.4, "{yellow:?} {blue:?}");
        // Vibrance et saturation ouvrent la couleur sans toucher aux gris.
        let spread = |c: [f32; 3]| c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
        let dull = [0.45, 0.50, 0.60];
        assert!(spread(run(10, 100.0, dull)) > spread(dull), "vibrance +100 ouvre une couleur terne");
        assert!(spread(run(10, -100.0, dull)) < spread(dull), "vibrance -100 la referme");
        assert!(neutral(run(10, 100.0, mid)) && neutral(run(11, 100.0, mid)), "ni l'une ni l'autre ne teinte un gris");

        // Étalonnage : mesuré pour le vert et le bleu, ancienne formule pour le rouge vers le positif.
        assert!(neutral(run(13, 100.0, mid)) && neutral(run(14, -100.0, mid)), "l'étalonnage ne touche pas les gris");
        assert!(run(12, 100.0, [1.0, 0.0, 0.0])[1] > 0.2, "rouge pur tiré vers l'orange");
        assert!(run(9, 100.0, [0.3; 3])[0] < 0.27, "correction du voile");
    }

    #[test]
    fn hue_can_be_shifted_for_one_colour_range_only() {
        // Plage centrée sur le rouge (0°), large de 40° : le rouge devient vert, le bleu et le gris ne bougent pas.
        let reds = Adjustment::HueSaturation { hue: 120.0, saturation: 0.0, lightness: 0.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 0.0, range_width: 40.0 }.prepare();
        assert!(close(reds.apply([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]), "au cœur de la plage : réglage entier");
        assert!(close(reds.apply([0.0, 0.0, 1.0]), [0.0, 0.0, 1.0]), "hors plage : intact");
        assert!(close(reds.apply([0.5; 3]), [0.5; 3]), "un gris n'a pas de teinte");
        // À la lisière (orange à 35°) : transition, ni intact ni entièrement décalé.
        let orange = [1.0, 0.583, 0.0];
        let shifted = reds.apply(orange);
        assert!(!close(shifted, orange) && shifted[0] > 0.3, "transition douce en bord de plage : {shifted:?}");
        // La plage passe par 0° : un rouge violacé (350°) est dedans.
        assert!(!close(reds.apply([1.0, 0.0, 0.17]), [1.0, 0.0, 0.17]));
    }

    #[test]
    fn the_json_field_names_match_the_interface() {
        // L'interface envoie du camelCase : si les noms divergent, serde remplace les valeurs par
        // zéro sans rien dire, et la plage de teintes cesse d'agir. Ce test l'interdit.
        let json = serde_json::to_string(&Adjustment::HueSaturation {
            hue: 10.0,
            saturation: 0.0,
            lightness: 0.0,
            range_start: 90.0,
            range_end: 150.0,
            feather_in: 30.0,
            feather_out: 10.0,
            range_center: 0.0,
            range_width: 0.0,
        })
        .expect("sérialisation");
        for name in ["rangeStart", "rangeEnd", "featherIn", "featherOut", "rangeCenter", "rangeWidth"] {
            assert!(json.contains(name), "champ {name} absent de {json}");
        }
        let back: Adjustment = serde_json::from_str(&json).expect("relecture");
        let Prepared::HueSaturation { range: Some(range), .. } = back.prepare() else { panic!("la plage doit survivre à l'aller-retour") };
        assert!((range.coverage(120.0 / 360.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_four_handles_bound_the_range_and_its_fades() {
        // Plage pleine de 90° à 150° (les verts), transition de 30° avant et de 10° après.
        let greens = Adjustment::HueSaturation {
            hue: 0.0,
            saturation: -100.0,
            lightness: 0.0,
            range_start: 90.0,
            range_end: 150.0,
            feather_in: 30.0,
            feather_out: 10.0,
            range_center: 0.0,
            range_width: 0.0,
        }
        .prepare();
        let Prepared::HueSaturation { range: Some(range), .. } = &greens else { panic!("la plage doit être active") };
        let at = |degrees: f32| range.coverage(degrees / 360.0);
        assert!((at(120.0) - 1.0).abs() < 1e-6, "au cœur : correction entière");
        assert!((at(90.0) - 1.0).abs() < 1e-6 && (at(150.0) - 1.0).abs() < 1e-6, "les bornes pleines sont dedans");
        assert!(at(75.0) > 0.0 && at(75.0) < 1.0, "dans la transition d'entrée : partielle");
        assert!(at(155.0) > 0.0 && at(155.0) < 1.0, "dans la transition de sortie : partielle");
        assert_eq!(at(59.0), 0.0, "avant la transition d'entrée : rien");
        assert_eq!(at(161.0), 0.0, "après la transition de sortie : rien");
        // Les transitions sont bien asymétriques : 10° avant le début pèsent plus que 10° après la fin.
        assert!(at(80.0) > at(160.0), "transition longue d'un côté, courte de l'autre");
        // Un vert franc est désaturé, un bleu franc ne bouge pas.
        let green = greens.apply([0.0, 1.0, 0.0]);
        assert!((green[0] - green[2]).abs() < 1e-3 && green[1] < 0.6, "vert désaturé : {green:?}");
        assert!(close(greens.apply([0.0, 0.0, 1.0]), [0.0, 0.0, 1.0]), "bleu intact");
    }

    #[test]
    fn an_old_file_keeps_its_symmetric_range() {
        // Fichier d'avant les quatre poignées : centre 0°, demi-largeur 40°.
        let legacy = Adjustment::HueSaturation { hue: 120.0, saturation: 0.0, lightness: 0.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 0.0, range_width: 40.0 }.prepare();
        let Prepared::HueSaturation { range: Some(range), .. } = &legacy else { panic!("la plage doit rester active") };
        assert!((range.coverage(0.0) - 1.0).abs() < 1e-6, "le centre reste corrigé en entier");
        assert_eq!(range.coverage(45.0 / 360.0), 0.0, "au-delà de la demi-largeur : rien");
        assert!(range.feather_in > 0.0 && (range.feather_in - range.feather_out).abs() < 1e-6, "transition symétrique");
    }

    #[test]
    fn each_adjustment_does_what_its_name_says() {
        let gray = [0.5, 0.5, 0.5];
        assert!(Adjustment::BrightnessContrast { brightness: 50.0, contrast: 0.0 }.prepare().apply(gray)[0] > 0.7);
        let contrasted = Adjustment::BrightnessContrast { brightness: 0.0, contrast: 80.0 }.prepare();
        assert!(contrasted.apply([0.3; 3])[0] < 0.1 && contrasted.apply([0.7; 3])[0] > 0.9 && close(contrasted.apply(gray), gray));

        // Inverser : extrêmes exacts, tons moyens plus sombres que le négatif strict (mesures : 128 → 116, 64 → 185).
        let inverted = Adjustment::Invert.prepare().apply([1.0, 0.0, 128.0 / 255.0]);
        assert!(close([inverted[0], inverted[1], 0.0], [0.0, 1.0, 0.0]), "blanc → noir, noir → blanc : {inverted:?}");
        assert!((inverted[2] * 255.0 - 116.0).abs() < 1.5, "gris moyen → 116 : {}", inverted[2] * 255.0);
        assert!(close(Adjustment::Negative.prepare().apply([0.2, 0.5, 0.9]), Adjustment::Invert.prepare().apply([0.2, 0.5, 0.9])), "Négatif : même courbe sur les couleurs");
        // Le papier : Inverser laisse le blanc pur blanc, Négatif le rend noir. Un blanc cassé est inversé par les deux.
        assert!(close(Adjustment::Invert.prepare().apply([1.0; 3]), [1.0; 3]), "Inverser garde le papier blanc");
        assert!(close(Adjustment::Negative.prepare().apply([1.0; 3]), [0.0; 3]), "Négatif rend le papier noir");
        let off_white = [240.0 / 255.0; 3];
        assert!(Adjustment::Invert.prepare().apply(off_white)[0] < 0.1, "un blanc cassé (240) est bien inversé");
        assert!((paper_weight([254.0 / 255.0; 3]) - 1.0).abs() < 1e-6 && paper_weight([248.0 / 255.0; 3]) < 1e-6);
        assert_eq!(Adjustment::Invert.prepare().to_gpu().kind, GPU_INVERT);
        let table = invert_table();
        assert_eq!((table[0], table[255]), (255, 0));
        assert!((i32::from(table[64]) - 185).abs() <= 2 && (i32::from(table[192]) - 57).abs() <= 2, "64 → {}, 192 → {}", table[64], table[192]);
        assert!(table.windows(2).all(|w| w[0] >= w[1]), "courbe décroissante : pas de retour en arrière");

        let levels = Adjustment::Levels { black: 51.0, gamma: 1.0, white: 204.0 }.prepare();
        assert!(close(levels.apply([0.2, 0.5, 0.8]), [0.0, 0.5, 1.0]));
        assert!(Adjustment::Levels { black: 0.0, gamma: 2.0, white: 255.0 }.prepare().apply(gray)[0] > 0.68, "gamma > 1 éclaircit les tons moyens");

        let red = [1.0, 0.0, 0.0];
        assert!(close(Adjustment::HueSaturation { hue: 120.0, saturation: 0.0, lightness: 0.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 0.0, range_width: 0.0 }.prepare().apply(red), [0.0, 1.0, 0.0]), "rouge + 120° = vert");
        assert!(close(Adjustment::HueSaturation { hue: 0.0, saturation: -100.0, lightness: 0.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 0.0, range_width: 0.0 }.prepare().apply(red), [0.5; 3]), "désaturé : gris");
        assert!(close(Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 100.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 0.0, range_width: 0.0 }.prepare().apply(red), [1.0; 3]));
    }

    #[test]
    fn a_gradient_map_reads_the_luminosity_and_blends_through_stop_opacity() {
        let stop = |position, color| GradientStop { position, color };
        let map = Adjustment::GradientMap { stops: vec![stop(0.0, [255, 0, 0, 255]), stop(1.0, [0, 0, 255, 255])], reverse: false, dither: false, smoothness: 0.0 }.prepare();
        assert!(close(map.apply([0.0; 3]), [1.0, 0.0, 0.0]), "le noir prend l'arrêt de gauche");
        assert!(close(map.apply([1.0; 3]), [0.0, 0.0, 1.0]), "le blanc prend l'arrêt de droite");
        assert!(close(map.apply([0.5; 3]), [0.5, 0.0, 0.5]), "gris moyen : à mi-chemin, sans lissage");
        // Un vert pur pèse 0,587 en luminosité : plus loin dans le dégradé qu'un rouge pur (0,299).
        assert!(map.apply([0.0, 1.0, 0.0])[2] > map.apply([1.0, 0.0, 0.0])[2]);
        // Inversé : le noir prend l'arrêt de droite.
        let reversed = Adjustment::GradientMap { stops: vec![stop(0.0, [255, 0, 0, 255]), stop(1.0, [0, 0, 255, 255])], reverse: true, dither: false, smoothness: 0.0 }.prepare();
        assert!(close(reversed.apply([0.0; 3]), [0.0, 0.0, 1.0]));
        // Lissage à 100 % : une marche douce, qui ne bouge pas les extrémités ni le milieu.
        let smooth = Adjustment::GradientMap { stops: vec![stop(0.0, [0, 0, 0, 255]), stop(1.0, [255, 255, 255, 255])], reverse: false, dither: false, smoothness: 100.0 }.prepare();
        assert!(close(smooth.apply([0.5; 3]), [0.5; 3]) && smooth.apply([0.25; 3])[0] < 0.2 && smooth.apply([0.75; 3])[0] > 0.8);
        // Un arrêt à moitié transparent laisse passer la moitié de la couleur d'origine.
        let veiled = Adjustment::GradientMap { stops: vec![stop(0.0, [255, 255, 255, 128]), stop(1.0, [255, 255, 255, 128])], reverse: false, dither: false, smoothness: 0.0 }.prepare();
        let out = veiled.apply([0.0, 0.0, 0.0]);
        assert!((out[0] - 0.502).abs() < 0.01, "{out:?}");
        // Tramé : la position bouge d'au plus un niveau, jamais plus.
        let dithered = Adjustment::GradientMap { stops: vec![stop(0.0, [0, 0, 0, 255]), stop(1.0, [255, 255, 255, 255])], reverse: false, dither: true, smoothness: 0.0 }.prepare();
        for index in [0, 1, 300, 65535] {
            assert!((dithered.apply_at([0.5; 3], index)[0] - 0.5).abs() <= 1.01 / 255.0);
        }
        // La description pour le shader : tramage, puis 256 couleurs RGBA.
        assert_eq!(dithered.to_gpu().values.len(), 1 + 256 * 4);
        assert_eq!(dithered.to_gpu().kind, GPU_GRADIENT_MAP);
        // Un aller-retour par le fichier garde tout.
        let json = serde_json::to_string(&Adjustment::GradientMap { stops: vec![stop(0.3, [1, 2, 3, 4])], reverse: true, dither: true, smoothness: 40.0 }).expect("json");
        let back: Adjustment = serde_json::from_str(&json).expect("relu");
        assert_eq!(back, Adjustment::GradientMap { stops: vec![stop(0.3, [1, 2, 3, 4])], reverse: true, dither: true, smoothness: 40.0 });
        // Sans `smoothness` (fichier plus ancien, ou interface minimale) : 100 %.
        let minimal: Adjustment = serde_json::from_str(r#"{"type":"gradientMap","stops":[]}"#).expect("relu");
        assert!(matches!(minimal, Adjustment::GradientMap { smoothness, .. } if (smoothness - 100.0).abs() < 1e-6));
    }

    /// Un « Développement » dont un seul curseur bouge.
    fn develop_with(set: impl Fn(&mut [f32; 15])) -> Adjustment {
        let mut v = [0.0f32; 15];
        set(&mut v);
        Adjustment::Develop {
            temperature: v[0], tint: v[1], exposure: v[2], contrast: v[3], highlights: v[4], shadows: v[5],
            whites: v[6], blacks: v[7], clarity: v[8], dehaze: v[9], vibrance: v[10], saturation: v[11],
            red_hue: v[12], green_hue: v[13], blue_hue: v[14],
        }
    }

    #[test]
    fn a_measured_slider_replays_its_model() {
        // Température à -100 : la sortie doit être celle du modèle mesuré, au bit près.
        let colour = [0.99, 0.09, 0.03];
        let expected = crate::calibration::step(crate::calibration::Slider::Temperature, -100.0).expect("mesuré").above.apply(colour);
        let got = develop_with(|v| v[0] = -100.0).prepare().apply(colour);
        assert!(close(got, expected), "{got:?} != {expected:?}");
        // Et ce modèle fait bien virer le rouge vers le bleu, ce que l'ancienne formule ne faisait pas.
        assert!(got[2] > 0.6, "le bleu devrait monter fort : {got:?}");
    }

    #[test]
    fn a_slider_at_half_lands_between() {
        let colour = [0.2, 0.85, 0.2];
        let none = develop_with(|_| {}).prepare().apply(colour);
        let full = develop_with(|v| v[0] = -100.0).prepare().apply(colour);
        let half = develop_with(|v| v[0] = -50.0).prepare().apply(colour);
        for i in 0..3 {
            let expected = none[i] + (full[i] - none[i]) * 0.5;
            assert!((half[i] - expected).abs() < 1e-5, "canal {i} : {} attendu {expected}", half[i]);
        }
    }

    #[test]
    fn an_untouched_develop_leaves_the_colour_alone() {
        for colour in [[0.0; 3], [0.5, 0.2, 0.9], [1.0; 3]] {
            assert!(close(develop_with(|_| {}).prepare().apply(colour), colour), "{colour:?} a bougé");
        }
    }

    #[test]
    fn what_was_never_measured_keeps_the_old_formula() {
        // Contraste et clarté n'ont pas de mesure : ils doivent quand même agir.
        let flat = [0.3, 0.3, 0.3];
        assert!(develop_with(|v| v[3] = 80.0).prepare().apply(flat)[0] < 0.25, "le contraste doit creuser les tons sombres");
        assert!(develop_with(|v| v[8] = 100.0).prepare().apply([0.45; 3])[0] != 0.45, "la clarté doit agir");
        // Les trois primaires sont mesurées dans les deux sens : plus rien ne passe par l'ancienne matrice.
        for index in [12, 13, 14] {
            let grey = develop_with(|v| v[index] = 100.0).prepare().apply([0.58; 3]);
            let spread = grey[0].max(grey[1]).max(grey[2]) - grey[0].min(grey[1]).min(grey[2]);
            assert!(spread < 0.02, "l'étalonnage {index} teinte un gris : {grey:?}");
        }
    }

    #[test]
    fn a_measured_slider_keeps_a_grey_grey_when_it_should() {
        // L'étalonnage des primaires ne touche que la couleur.
        let out = develop_with(|v| v[14] = 100.0).prepare().apply([0.58; 3]);
        let spread = out[0].max(out[1]).max(out[2]) - out[0].min(out[1]).min(out[2]);
        assert!(spread < 0.01, "gris parti en couleur : {out:?}");
    }
}
