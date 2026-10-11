//! Modes de fusion de Photoshop : formules de référence.
//!
//! Notations : `b` = fond (backdrop), `s` = source, valeurs de canal dans `0..=1`,
//! couleurs **non prémultipliées**, espace sRGB non linéaire (comme Photoshop en
//! 8 bits). `B(b, s)` est la fonction du mode. La composition générale est celle
//! de la spécification W3C *Compositing and Blending* / PDF 1.7 § 11.3 :
//!
//! ```text
//! co = (1 - ab) * cs  +  as * ab * B(Cb, Cs)  +  (1 - as) * cb      (prémultiplié)
//! ao = as + ab * (1 - as)
//! ```
//!
//! | Mode                      | B(b, s)                                                  |
//! |---------------------------|----------------------------------------------------------|
//! | Normal                    | s                                                        |
//! | Dissoudre                 | s, alpha binarisé par un bruit (voir `compose`)          |
//! | Obscurcir                 | min(b, s)                                                |
//! | Produit                   | b * s                                                    |
//! | Densité couleur +         | s = 0 : 0 (1 si b = 1) ; sinon 1 - min(1, (1 - b) / s)   |
//! | Densité linéaire +        | max(0, b + s - 1)                                        |
//! | Éclaircir                 | max(b, s)                                                |
//! | Superposition             | b + s - b * s                                            |
//! | Densité couleur -         | s = 1 : 1 (0 si b = 0) ; sinon min(1, b / (1 - s))       |
//! | Densité linéaire - (ajout)| min(1, b + s)                                            |
//! | Incrustation              | Lumière crue avec b et s échangés                        |
//! | Lumière tamisée           | s <= ½ : 2bs + b²(1 - 2s) ; sinon 2b(1 - s) + √b (2s - 1)|
//! | Lumière crue              | s <= ½ : 2bs ; sinon 1 - 2(1 - b)(1 - s)                 |
//! | Différence                | abs(b - s)                                               |
//! | Exclusion                 | b + s - 2bs                                              |
//! | Soustraire                | max(0, b - s)                                            |
//! | Diviser                   | b = 0 : 0 ; s = 0 : 1 ; sinon min(1, b / s)              |
//! | Teinte                    | SetLum(SetSat(s, Sat(b)), Lum(b))                        |
//! | Saturation                | SetLum(SetSat(b, Sat(s)), Lum(b))                        |
//! | Couleur                   | SetLum(s, Lum(b))                                        |
//! | Luminosité                | SetLum(b, Lum(s))                                        |
//!
//! `Lum(c) = 0,30 R + 0,59 G + 0,11 B`, `Sat(c) = max - min`, `SetLum` et `SetSat`
//! selon la spécification PDF. La formule de Lumière tamisée est celle
//! couramment attribuée à Photoshop ; elle diffère légèrement de celle du W3C.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    /// « Transfert » : réservé aux groupes, les enfants fusionnent directement avec le fond.
    PassThrough,
    #[default]
    Normal,
    Dissolve,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    /// Modes applicables à tout calque, dans l'ordre du menu de Photoshop.
    pub const LAYER_MODES: [Self; 21] = [
        Self::Normal,
        Self::Dissolve,
        Self::Darken,
        Self::Multiply,
        Self::ColorBurn,
        Self::LinearBurn,
        Self::Lighten,
        Self::Screen,
        Self::ColorDodge,
        Self::LinearDodge,
        Self::Overlay,
        Self::SoftLight,
        Self::HardLight,
        Self::Difference,
        Self::Exclusion,
        Self::Subtract,
        Self::Divide,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
    ];

    /// Nom affiché, identique à Photoshop en français.
    pub fn label_fr(self) -> &'static str {
        match self {
            Self::PassThrough => "Transfert",
            Self::Normal => "Normal",
            Self::Dissolve => "Dissoudre",
            Self::Darken => "Obscurcir",
            Self::Multiply => "Produit",
            Self::ColorBurn => "Densité couleur +",
            Self::LinearBurn => "Densité linéaire +",
            Self::Lighten => "Éclaircir",
            Self::Screen => "Superposition",
            Self::ColorDodge => "Densité couleur -",
            Self::LinearDodge => "Densité linéaire - (ajout)",
            Self::Overlay => "Incrustation",
            Self::SoftLight => "Lumière tamisée",
            Self::HardLight => "Lumière crue",
            Self::Difference => "Différence",
            Self::Exclusion => "Exclusion",
            Self::Subtract => "Soustraire",
            Self::Divide => "Diviser",
            Self::Hue => "Teinte",
            Self::Saturation => "Saturation",
            Self::Color => "Couleur",
            Self::Luminosity => "Luminosité",
        }
    }
}

type Rgb = [f32; 3];

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        2.0 * b * s
    } else {
        1.0 - 2.0 * (1.0 - b) * (1.0 - s)
    }
}

fn separable(mode: BlendMode, b: f32, s: f32) -> f32 {
    match mode {
        BlendMode::Darken => b.min(s),
        BlendMode::Multiply => b * s,
        BlendMode::ColorBurn => {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        BlendMode::LinearBurn => (b + s - 1.0).max(0.0),
        BlendMode::Lighten => b.max(s),
        BlendMode::Screen => b + s - b * s,
        BlendMode::ColorDodge => {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        BlendMode::LinearDodge => (b + s).min(1.0),
        BlendMode::Overlay => hard_light(s, b),
        BlendMode::SoftLight => {
            if s <= 0.5 {
                2.0 * b * s + b * b * (1.0 - 2.0 * s)
            } else {
                2.0 * b * (1.0 - s) + b.sqrt() * (2.0 * s - 1.0)
            }
        }
        BlendMode::HardLight => hard_light(b, s),
        BlendMode::Difference => (b - s).abs(),
        BlendMode::Exclusion => b + s - 2.0 * b * s,
        BlendMode::Subtract => (b - s).max(0.0),
        BlendMode::Divide => {
            if b <= 0.0 {
                0.0
            } else if s <= 0.0 {
                1.0
            } else {
                (b / s).min(1.0)
            }
        }
        // Normal, Dissoudre, Transfert et les modes non séparables ne passent pas ici.
        _ => s,
    }
}

fn lum(c: Rgb) -> f32 {
    0.30 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn sat(c: Rgb) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn clip_color(c: Rgb) -> Rgb {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 {
        out = out.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 {
        out = out.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    out
}

fn set_lum(c: Rgb, l: f32) -> Rgb {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

fn set_sat(c: Rgb, s: f32) -> Rgb {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    if max <= min {
        return [0.0; 3];
    }
    // Le minimum va à 0, le maximum à `s`, le médian est interpolé.
    c.map(|v| (v - min) * s / (max - min))
}

/// `B(b, s)` sur des couleurs non prémultipliées.
pub fn blend_function(mode: BlendMode, b: Rgb, s: Rgb) -> Rgb {
    match mode {
        BlendMode::Normal | BlendMode::Dissolve | BlendMode::PassThrough => s,
        BlendMode::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
        BlendMode::Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
        BlendMode::Color => set_lum(s, lum(b)),
        BlendMode::Luminosity => set_lum(b, lum(s)),
        _ => [separable(mode, b[0], s[0]), separable(mode, b[1], s[1]), separable(mode, b[2], s[2])],
    }
}

/// Compose `src` sur `dst` (RGBA prémultipliés, `0..=1`) avec le mode donné.
#[inline]
pub fn blend_premultiplied(mode: BlendMode, src: [f32; 4], dst: [f32; 4]) -> [f32; 4] {
    let (sa, da) = (src[3], dst[3]);
    if sa <= 0.0 {
        return dst;
    }
    let inv_sa = 1.0 - sa;
    // Sans fond, ou en mode Normal, la fonction de fusion n'intervient pas.
    if da <= 0.0 || matches!(mode, BlendMode::Normal | BlendMode::Dissolve | BlendMode::PassThrough) {
        return [src[0] + dst[0] * inv_sa, src[1] + dst[1] * inv_sa, src[2] + dst[2] * inv_sa, sa + da * inv_sa];
    }
    let s = [src[0] / sa, src[1] / sa, src[2] / sa];
    let b = [dst[0] / da, dst[1] / da, dst[2] / da];
    let mixed = blend_function(mode, b, s);
    let both = sa * da;
    let inv_da = 1.0 - da;
    [
        inv_da * src[0] + both * mixed[0] + inv_sa * dst[0],
        inv_da * src[1] + both * mixed[1] + inv_sa * dst[1],
        inv_da * src[2] + both * mixed[2] + inv_sa * dst[2],
        sa + da * inv_sa,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_rgb(actual: Rgb, expected: Rgb) {
        for i in 0..3 {
            assert!((actual[i] - expected[i]).abs() < 1e-4, "{actual:?} != {expected:?}");
        }
    }

    const B: Rgb = [0.8, 0.4, 0.2];
    const S: Rgb = [0.5, 0.25, 1.0];

    #[test]
    fn separable_modes_match_reference_values() {
        let cases: [(BlendMode, Rgb); 16] = [
            (BlendMode::Normal, S),
            (BlendMode::Darken, [0.5, 0.25, 0.2]),
            (BlendMode::Multiply, [0.4, 0.1, 0.2]),
            (BlendMode::ColorBurn, [0.6, 0.0, 0.2]),
            (BlendMode::LinearBurn, [0.3, 0.0, 0.2]),
            (BlendMode::Lighten, [0.8, 0.4, 1.0]),
            (BlendMode::Screen, [0.9, 0.55, 1.0]),
            (BlendMode::ColorDodge, [1.0, 0.533_333, 1.0]),
            (BlendMode::LinearDodge, [1.0, 0.65, 1.0]),
            (BlendMode::Overlay, [0.8, 0.2, 0.4]),
            (BlendMode::HardLight, [0.8, 0.2, 1.0]),
            (BlendMode::Difference, [0.3, 0.15, 0.8]),
            (BlendMode::Exclusion, [0.5, 0.45, 0.8]),
            (BlendMode::Subtract, [0.3, 0.15, 0.0]),
            (BlendMode::Divide, [1.0, 1.0, 0.2]),
            (BlendMode::SoftLight, [0.8, 0.28, 0.447_214]),
        ];
        for (mode, expected) in cases {
            assert_rgb(blend_function(mode, B, S), expected);
        }
    }

    #[test]
    fn burn_dodge_and_divide_handle_limits_without_nan() {
        assert_eq!(separable(BlendMode::ColorBurn, 1.0, 0.0), 1.0);
        assert_eq!(separable(BlendMode::ColorBurn, 0.5, 0.0), 0.0);
        assert_eq!(separable(BlendMode::ColorDodge, 0.0, 1.0), 0.0);
        assert_eq!(separable(BlendMode::ColorDodge, 0.5, 1.0), 1.0);
        assert_eq!(separable(BlendMode::Divide, 0.0, 0.0), 0.0);
        assert_eq!(separable(BlendMode::Divide, 0.5, 0.0), 1.0);
    }

    #[test]
    fn neutral_colors_leave_backdrop_unchanged() {
        let white = [1.0; 3];
        let black = [0.0; 3];
        let gray = [0.5; 3];
        for mode in [BlendMode::Multiply, BlendMode::Darken, BlendMode::ColorBurn, BlendMode::LinearBurn, BlendMode::Divide] {
            assert_rgb(blend_function(mode, B, white), B);
        }
        for mode in [
            BlendMode::Screen,
            BlendMode::Lighten,
            BlendMode::ColorDodge,
            BlendMode::LinearDodge,
            BlendMode::Difference,
            BlendMode::Exclusion,
            BlendMode::Subtract,
        ] {
            assert_rgb(blend_function(mode, B, black), B);
        }
        for mode in [BlendMode::Overlay, BlendMode::SoftLight, BlendMode::HardLight] {
            assert_rgb(blend_function(mode, B, gray), B);
        }
    }

    #[test]
    fn non_separable_modes_preserve_the_expected_component() {
        let hue = blend_function(BlendMode::Hue, B, S);
        assert!((lum(hue) - lum(B)).abs() < 1e-4, "Teinte conserve la luminosité du fond");
        assert!((sat(hue) - sat(B)).abs() < 1e-4, "Teinte conserve la saturation du fond");

        let saturation = blend_function(BlendMode::Saturation, B, [0.5; 3]);
        assert!(sat(saturation) < 1e-4, "une source grise désature le fond");
        assert!((lum(saturation) - lum(B)).abs() < 1e-4);

        let color = blend_function(BlendMode::Color, B, S);
        assert!((lum(color) - lum(B)).abs() < 1e-4, "Couleur conserve la luminosité du fond");

        let luminosity = blend_function(BlendMode::Luminosity, B, S);
        assert!((lum(luminosity) - lum(S)).abs() < 1e-4, "Luminosité prend celle de la source");
        for channel in hue.iter().chain(&color).chain(&luminosity) {
            assert!((0.0..=1.0).contains(channel), "résultat hors gamme : {channel}");
        }
    }

    #[test]
    fn compositing_respects_alpha() {
        // Source opaque sur fond opaque : résultat = B(b, s).
        let out = blend_premultiplied(BlendMode::Multiply, [0.5, 0.25, 1.0, 1.0], [0.8, 0.4, 0.2, 1.0]);
        assert_rgb([out[0], out[1], out[2]], [0.4, 0.1, 0.2]);
        assert!((out[3] - 1.0).abs() < 1e-6);

        // Sans fond, n'importe quel mode se réduit à la source.
        let src = [0.2, 0.1, 0.05, 0.4];
        assert_eq!(blend_premultiplied(BlendMode::Difference, src, [0.0; 4]), src);

        // Source transparente : le fond est inchangé.
        let dst = [0.3, 0.2, 0.1, 0.6];
        assert_eq!(blend_premultiplied(BlendMode::Screen, [0.0; 4], dst), dst);

        // Source à 50 % en Produit sur blanc opaque : moyenne entre le fond et le produit.
        let half = blend_premultiplied(BlendMode::Multiply, [0.0, 0.0, 0.0, 0.5], [1.0, 1.0, 1.0, 1.0]);
        assert_rgb([half[0], half[1], half[2]], [0.5, 0.5, 0.5]);
    }

    #[test]
    fn every_layer_mode_has_a_distinct_french_label() {
        let labels: std::collections::HashSet<_> = BlendMode::LAYER_MODES.iter().map(|m| m.label_fr()).collect();
        assert_eq!(labels.len(), 21);
    }
}
