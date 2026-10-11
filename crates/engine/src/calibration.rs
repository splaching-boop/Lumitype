//! Calage du panneau Développement sur Lightroom.
//!
//! Chaque curseur a été mesuré : une mire chromatique a été passée dans Lightroom avec ce seul
//! curseur poussé à une valeur connue, et la transformation couleur a été ajustée sur les
//! 460 000 pixels du couple avant/après. Ce module rejoue ces transformations mesurées, au lieu
//! des formules écrites à la main qui les approchaient de loin (écart moyen 31 sur 255 ;
//! ici 3,3, pour un plancher de bruit de mesure à 1,5).
//!
//! Les pixels proches du neutre pèsent sept fois plus que les autres dans l'ajustement du premier
//! étage. Sans cela la mire, faite à 80 % de couleurs saturées, sacrifiait les gris : sur
//! `Rouge primaire -100`, un gris à 63 ressortait avec 60 niveaux d'écart entre ses canaux.
//!
//! Un modèle se lit en quatre étages, dans cet ordre :
//!
//! 1. une matrice 3 x 3, ou un polynôme racine à six termes quand il mesure plus juste,
//!    appliquée en lumière linéaire ou directement sur les valeurs affichées ;
//! 2. une courbe de 129 points par canal, qui porte le rendu et l'écrêtage ;
//! 3. une correction résiduelle de 65 points indexée par la luminance, **réservée aux neutres** :
//!    elle s'efface dès que la couleur sature ;
//! 4. une correction indexée par la teinte d'entrée, sur 96 secteurs, **pondérée par la chroma** —
//!    elle vaut donc zéro sur un gris.
//!
//! Les étages 3 et 4 sont complémentaires, et c'est voulu. Un premier calage les avait fondus en
//! un seul, ajusté sur tous les pixels : la mire étant à 80 % faite de couleurs saturées, les gris
//! se faisaient écraser et partaient en couleur — jusqu'à 19,7 d'écart sur les gris quand la
//! moyenne affichait 5,3. Or un gris qui vire se voit immédiatement, contrairement à une teinte
//! saturée légèrement fausse.
//!
//! Les courbes de l'étage 2 sont lissées et **strictement croissantes**. C'est indispensable : une
//! courbe de transfert qui redescend, même de deux niveaux, se voit immédiatement comme une bande
//! verticale dans un dégradé.

use std::sync::OnceLock;

/// Les treize curseurs dont la transformation a été mesurée.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slider {
    Temperature,
    Tint,
    Exposure,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Dehaze,
    Vibrance,
    Saturation,
    RedHue,
    GreenHue,
    BlueHue,
}

pub const LUT: usize = 129;
pub const SECTORS: usize = 96;
pub const LUMA_STEPS: usize = 65;
/// Taille d'un modèle dans la table : drapeaux, matrice, courbes, teintes, luminance.
pub const BLOCK: usize = 1 + 18 + 3 * LUT + SECTORS * 3 + LUMA_STEPS * 3;
/// Chroma à partir de laquelle la correction des neutres est complètement éteinte.
pub const CHROMA_FULL: f32 = 0.25;

/// Table mesurée, rangée en flottants little-endian. Un modèle tous les `BLOCK` flottants.
static TABLE_BYTES: &[u8] = include_bytes!("develop.bin");

fn table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| TABLE_BYTES.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

/// Curseur, valeur mesurée, et position du modèle dans la table.
#[rustfmt::skip]
const INDEX: &[(Slider, f32, usize)] = &[
    (Slider::Temperature, -100.0, 0), (Slider::Temperature, 100.0, 1),
    (Slider::Tint, -100.0, 2), (Slider::Tint, 100.0, 3),
    (Slider::Exposure, -5.0, 4), (Slider::Exposure, -3.24, 5), (Slider::Exposure, 1.32, 6), (Slider::Exposure, 5.0, 7),
    (Slider::Highlights, -100.0, 8), (Slider::Highlights, 100.0, 9),
    (Slider::Shadows, -100.0, 10), (Slider::Shadows, 100.0, 11),
    (Slider::Whites, -100.0, 12), (Slider::Whites, 100.0, 13),
    (Slider::Blacks, -100.0, 14), (Slider::Blacks, 100.0, 15),
    (Slider::Dehaze, -100.0, 16), (Slider::Dehaze, 100.0, 17),
    (Slider::Vibrance, -100.0, 18), (Slider::Vibrance, 100.0, 19),
    (Slider::Saturation, -100.0, 20), (Slider::Saturation, 100.0, 21),
    (Slider::RedHue, -100.0, 22), (Slider::RedHue, 100.0, 23),
    (Slider::GreenHue, -100.0, 24), (Slider::GreenHue, 100.0, 25),
    (Slider::BlueHue, -100.0, 26), (Slider::BlueHue, 100.0, 27),
];

/// Un modèle mesuré, emprunté à la table.
#[derive(Clone, Copy)]
pub struct Model {
    data: &'static [f32],
}

impl Model {
    fn at(slot: usize) -> Self {
        Self { data: &table()[slot * BLOCK..(slot + 1) * BLOCK] }
    }

    /// 3 ou 6 termes.
    fn terms(&self) -> usize {
        (self.data[0] as usize) / 10
    }

    /// Vrai si la matrice a été ajustée sur les valeurs affichées plutôt qu'en lumière linéaire.
    fn in_srgb(&self) -> bool {
        (self.data[0] as usize) % 10 == 1
    }

    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let d = self.data;
        let terms = self.terms();
        let srgb_space = self.in_srgb();
        let base = if srgb_space { rgb.map(|v| v.clamp(0.0, 1.0)) } else { rgb.map(|v| to_linear(v.clamp(0.0, 1.0))) };
        let mut f = [base[0], base[1], base[2], 0.0, 0.0, 0.0];
        if terms == 6 {
            f[3] = (base[0] * base[1]).max(0.0).sqrt();
            f[4] = (base[1] * base[2]).max(0.0).sqrt();
            f[5] = (base[0] * base[2]).max(0.0).sqrt();
        }

        let mut out = [0.0f32; 3];
        for (c, slot) in out.iter_mut().enumerate() {
            let row = &d[1 + c * 6..1 + c * 6 + 6];
            let t: f32 = (0..terms).map(|k| row[k] * f[k]).sum();
            let u = if srgb_space { t.clamp(0.0, 1.0) } else { to_srgb(t.clamp(0.0, 1.0)) } * (LUT - 1) as f32;
            let curve = &d[19 + c * LUT..19 + (c + 1) * LUT];
            *slot = lerp_table(curve, u);
        }

        let (hue, chroma) = hue_and_chroma(rgb);

        // Correction des neutres : pleine sur un gris, éteinte dès que la couleur sature.
        let neutral = (1.0 - chroma / CHROMA_FULL).max(0.0);
        if neutral > 0.0 {
            let base = 19 + 3 * LUT + SECTORS * 3;
            let position = luma(rgb).clamp(0.0, 1.0) * (LUMA_STEPS - 1) as f32;
            let k = (position.floor() as usize).min(LUMA_STEPS - 2);
            let fract = position - k as f32;
            for (c, slot) in out.iter_mut().enumerate() {
                let low = d[base + k * 3 + c];
                let high = d[base + (k + 1) * 3 + c];
                *slot = (*slot + (low * (1.0 - fract) + high * fract) * neutral).clamp(0.0, 1.0);
            }
        }

        // Correction par teinte, proportionnelle à la chroma : nulle sur un gris.
        if chroma > 1e-6 {
            let base = 19 + 3 * LUT;
            let position = hue * SECTORS as f32;
            let k = (position.floor() as usize) % SECTORS;
            let fract = position - position.floor();
            let next = (k + 1) % SECTORS;
            for (c, slot) in out.iter_mut().enumerate() {
                let low = d[base + k * 3 + c];
                let high = d[base + next * 3 + c];
                *slot = (*slot + (low * (1.0 - fract) + high * fract) * chroma).clamp(0.0, 1.0);
            }
        }
        out
    }

    /// Les flottants du modèle, pour les envoyer tels quels au shader.
    pub fn raw(&self) -> &'static [f32] {
        self.data
    }
}

/// Un curseur poussé à `value` : le modèle mesuré juste en dessous, celui juste au-dessus, et
/// la part de l'un vers l'autre.
///
/// Entre deux mesures — et entre zéro et la première — on interpole. C'est une approximation
/// assumée : seules les valeurs mesurées sont exactes.
pub struct Step {
    pub below: Option<Model>,
    pub above: Model,
    pub mix: f32,
}

pub fn step(slider: Slider, value: f32) -> Option<Step> {
    if value == 0.0 {
        return None;
    }
    let same_sign = |v: f32| (v > 0.0) == (value > 0.0);
    let mut known: Vec<(f32, usize)> =
        INDEX.iter().filter(|(s, v, _)| *s == slider && same_sign(*v)).map(|(_, v, i)| (v.abs(), *i)).collect();
    if known.is_empty() {
        return None;
    }
    known.sort_by(|a, b| a.0.total_cmp(&b.0));
    let target = value.abs();
    let below = known.iter().rev().find(|(v, _)| *v <= target).copied();
    let above = known.iter().find(|(v, _)| *v >= target).copied().unwrap_or(*known.last().unwrap());
    let low = below.map(|(v, _)| v).unwrap_or(0.0);
    let mix = if above.0 > low { ((target - low) / (above.0 - low)).clamp(0.0, 1.0) } else { 1.0 };
    Some(Step { below: below.map(|(_, i)| Model::at(i)), above: Model::at(above.1), mix })
}

/// Vrai si ce curseur, dans ce sens, a été mesuré. `Contraste`, `Texture` et `Clarté` ne l'ont
/// jamais été, et `Rouge primaire` seulement vers le négatif : ceux-là gardent l'ancienne formule.
pub fn is_measured(slider: Slider, value: f32) -> bool {
    value != 0.0 && INDEX.iter().any(|(s, v, _)| *s == slider && (*v > 0.0) == (value > 0.0))
}

pub fn luma([r, g, b]: [f32; 3]) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

pub fn to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

pub fn to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Teinte sur le cercle (0 à 1) et force chromatique de la couleur, telles qu'elles ont été
/// calculées pendant l'ajustement.
fn hue_and_chroma([r, g, b]: [f32; 3]) -> (f32, f32) {
    let high = r.max(g).max(b);
    let low = r.min(g).min(b);
    let chroma = high - low;
    if chroma <= 1e-6 {
        return (0.0, 0.0);
    }
    let raw = if high == r {
        ((g - b) / chroma).rem_euclid(6.0)
    } else if high == g {
        (b - r) / chroma + 2.0
    } else {
        (r - g) / chroma + 4.0
    };
    ((raw / 6.0).rem_euclid(1.0), chroma)
}

fn lerp_table(table: &[f32], position: f32) -> f32 {
    let k = (position.floor().max(0.0) as usize).min(table.len() - 2);
    let fract = (position - k as f32).clamp(0.0, 1.0);
    (table[k] * (1.0 - fract) + table[k + 1] * fract).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_holds_one_model_per_measurement() {
        assert_eq!(table().len(), INDEX.len() * BLOCK);
        assert_eq!(BLOCK, 889);
    }

    #[test]
    fn a_measured_value_uses_its_model_whole() {
        // Pile sur la mesure : le modèle s'applique entier, sans mélange.
        let full = step(Slider::Temperature, -100.0).expect("mesuré");
        assert_eq!(full.mix, 1.0);
        let colour = [0.9, 0.3, 0.1];
        assert_eq!(full.above.apply(colour), Model::at(0).apply(colour));
        // À mi-chemin, on est à mi-chemin entre rien et le modèle.
        let half = step(Slider::Temperature, -50.0).expect("mesuré");
        assert!((half.mix - 0.5).abs() < 1e-6);
    }

    #[test]
    fn exposure_interpolates_between_two_measurements() {
        // Mesures à +1,32 et +5 IL : à +3,16 IL on est pile entre les deux.
        let between = step(Slider::Exposure, 3.16).expect("mesuré");
        assert!(between.below.is_some(), "le modèle du bas doit exister");
        assert!((between.mix - 0.5).abs() < 0.01, "mix {}", between.mix);
    }

    #[test]
    fn every_primary_is_measured_both_ways() {
        assert!(is_measured(Slider::RedHue, -100.0));
        assert!(is_measured(Slider::RedHue, 100.0), "le rouge est mesuré dans les deux sens");
        assert!(!is_measured(Slider::Temperature, 0.0));
        assert!(step(Slider::Temperature, 0.0).is_none());
    }

    #[test]
    fn the_curves_never_go_back_down() {
        // Une courbe de transfert qui redescend fait des bandes dans un dégradé.
        for (slider, value, slot) in INDEX {
            let model = Model::at(*slot);
            for c in 0..3 {
                let curve = &model.raw()[19 + c * LUT..19 + (c + 1) * LUT];
                for pair in curve.windows(2) {
                    assert!(pair[1] >= pair[0] - 1e-6, "{slider:?} {value} canal {c} : {} puis {}", pair[0], pair[1]);
                }
            }
        }
    }

    #[test]
    fn calibrating_a_primary_leaves_greys_alone() {
        // L'étalonnage ne touche que la couleur : sur toute l'échelle de gris, la sortie doit
        // rester neutre et garder son niveau.
        for slider in [Slider::RedHue, Slider::GreenHue, Slider::BlueHue] {
            for value in [-100.0, 100.0] {
                let model = step(slider, value).expect("mesuré").above;
                for level in [0.1, 0.25, 0.5, 0.75, 0.9] {
                    let out = model.apply([level; 3]);
                    let spread = out[0].max(out[1]).max(out[2]) - out[0].min(out[1]).min(out[2]);
                    let shift = (out.iter().sum::<f32>() / 3.0 - level).abs();
                    assert!(spread < 0.02, "{slider:?} {value} : gris {level} parti en couleur, {out:?}");
                    assert!(shift < 0.02, "{slider:?} {value} : gris {level} a changé de niveau, {out:?}");
                }
            }
        }
    }

    #[test]
    fn a_grey_is_corrected_where_a_colour_is_not() {
        // La correction des neutres ne doit rien faire sur une couleur vive.
        let model = step(Slider::Exposure, 1.32).expect("mesuré").above;
        let grey = model.apply([0.58; 3]);
        let spread = grey[0].max(grey[1]).max(grey[2]) - grey[0].min(grey[1]).min(grey[2]);
        assert!(spread < 0.02, "un gris ne doit pas partir en couleur : {grey:?}");
    }

    #[test]
    fn a_grey_stays_grey_under_a_hue_only_model() {
        // L'étalonnage des primaires ne touche que la couleur : un gris doit rester neutre.
        let model = step(Slider::GreenHue, -100.0).expect("mesuré").above;
        let out = model.apply([0.5, 0.5, 0.5]);
        let spread = out[0].max(out[1]).max(out[2]) - out[0].min(out[1]).min(out[2]);
        assert!(spread < 0.02, "gris parti en couleur : {out:?}");
    }
}
