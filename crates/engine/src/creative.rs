//! Filtres créatifs de la galerie : flous et verres (flou radial, verre cannelé), lumière et matière
//! (vignettage, film d'essence, chrome, relief, grain argentique), impression (demi-teinte, risographie,
//! pointillisme), rétro (écran CRT, lignes de balayage, pixellisation), glitch (distorsion numérique,
//! datamosh, tri de pixels) et artistique (peinture à l'huile, postérisation, contours, Truchet).
//!
//! Les calculs se font sur la couleur « droite » (non prémultipliée, de 0 à 1) ; l'alpha du calque est
//! conservé, sauf pour les filtres qui déplacent la matière (flou radial, verre cannelé), qui travaillent
//! directement en prémultiplié comme les flous du module `filters`. Tout tirage au sort est déterministe
//! (graine) : le même réglage redonne la même image, dans l'aperçu comme à l'application.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::filters::{gaussian, hash, normal, value_noise};

// ---------------------------------------------------------------- Choix proposés par les filtres

/// Motif du filtre Truchet : la façon dont chaque case relie ses voisines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TruchetPattern {
    /// Quarts de cercle reliant les milieux des côtés : les chemins serpentent.
    #[default]
    Arcs,
    /// Deux voies parallèles par arc.
    Double,
    /// Trois voies.
    Triple,
    /// Arcs concentriques depuis un seul coin, tourné au hasard.
    Fan,
    /// Une diagonale par case : le labyrinthe « 10 PRINT ».
    Diagonals,
    /// Arcs larges dont seuls les deux bords sont tracés.
    Ribbons,
    /// Coins coupés : chemins octogonaux.
    Bevels,
    /// Un anneau par case, de taille variable.
    Rings,
    /// Arcs, ou croisement droit dans un tiers des cases.
    Mixed,
    /// Traits horizontaux ou verticaux : vannerie.
    Weave,
    /// Un mur sur le bord haut ou gauche de chaque case.
    Maze,
    /// Arcs, et un point à chaque coin.
    DotsArcs,
    /// Quarts de disque pleins.
    FilledQuarters,
    /// Demi-cases pleines.
    Triangles,
    /// Arcs, avec des cases fines dans les zones claires et larges dans les sombres.
    MultiScale,
}

/// Couleur du filtre Truchet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TruchetColor {
    /// Chaque tube prend la couleur de l'image sous lui.
    #[default]
    Image,
    /// Blanc sur noir, l'éclat suit la luminosité.
    Mono,
    /// Dégradé violet, rose, jaune selon la luminosité.
    Neon,
    /// Traits foncés sur papier clair.
    Paper,
}

/// Masque de l'écran CRT.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CrtMask {
    /// Points ronds rouges, verts, bleus.
    #[default]
    Triads,
    /// Bandes verticales continues (Trinitron).
    Grille,
    /// Barres hautes, décalées d'une demi-hauteur d'une triade à l'autre.
    Slots,
}

/// Forme des points du pointillisme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StippleShape {
    #[default]
    Round,
    Square,
    Stroke,
}

/// Motif de la demi-teinte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum HalftonePattern {
    /// Points ronds sur une grille inclinée (la presse).
    #[default]
    Dots,
    /// Cercles concentriques autour d'un centre, plus épais dans les sombres.
    Circles,
    /// Lignes parallèles, plus épaisses dans les sombres (gravure).
    Lines,
    /// Quatre trames CMJN, chacune à son angle d'imprimerie, en couleur.
    Cmyk,
}

/// Jeu d'encres de la risographie.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RisoInks {
    /// Bleu, rose fluo, jaune, noir.
    #[default]
    Quad,
    /// Bleu et rose fluo.
    Duo,
    /// Vert et orange.
    GreenOrange,
    /// Une seule encre, rouge fluo.
    MonoRed,
}

/// Sens du tri de pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SortDirection {
    #[default]
    Right,
    Left,
    Down,
    Up,
}

/// Teinte du métal du filtre Chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ChromeTint {
    #[default]
    Silver,
    Gold,
    Copper,
}

/// Palette du datamosh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MoshPalette {
    /// Violet, orange, crème.
    #[default]
    Violet,
    /// Les couleurs de l'image.
    Original,
    /// Vert sombre, vert acide, jaune.
    Acid,
}

// ---------------------------------------------------------------- Outils communs

/// Image de travail : couleur droite (0 à 1) et alpha (0 à 1).
pub(crate) struct Straight {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<[f32; 3]>,
    pub alpha: Vec<f32>,
}

impl Straight {
    /// Depuis le tampon prémultiplié du module `filters` (valeurs de 0 à 255).
    pub fn from_premultiplied(source: &[f32], w: usize, h: usize) -> Self {
        let (rgb, alpha): (Vec<[f32; 3]>, Vec<f32>) = source
            .par_chunks_exact(4)
            .map(|px| {
                let a = px[3] / 255.0;
                if a <= 0.0 {
                    ([0.0; 3], 0.0)
                } else {
                    ([(px[0] / 255.0 / a).min(1.0), (px[1] / 255.0 / a).min(1.0), (px[2] / 255.0 / a).min(1.0)], a)
                }
            })
            .unzip();
        Self { w, h, rgb, alpha }
    }

    /// Couleur `rgb` (droite) recomposée avec l'alpha d'origine, en prémultiplié 8 bits.
    pub fn compose(&self, rgb: &[[f32; 3]]) -> Vec<u8> {
        rgb.par_iter()
            .zip(self.alpha.par_iter())
            .flat_map_iter(|(c, &a)| {
                let alpha = (a * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                let q = |v: f32| (v.clamp(0.0, 1.0) * a * 255.0 + 0.5).clamp(0.0, f32::from(alpha)) as u8;
                [q(c[0]), q(c[1]), q(c[2]), alpha]
            })
            .collect()
    }

    /// Couleur floutée (écart-type `sigma`), pondérée par l'alpha : le transparent ne noircit pas les bords.
    pub fn blurred(&self, sigma: f32) -> Vec<[f32; 3]> {
        if sigma < 0.3 {
            return self.rgb.clone();
        }
        let packed: Vec<f32> = self.rgb.iter().zip(&self.alpha).flat_map(|(c, &a)| [c[0] * a, c[1] * a, c[2] * a, a]).collect();
        let soft = gaussian(&packed, self.w, self.h, sigma);
        soft.chunks_exact(4).map(|p| if p[3] > 1e-5 { [p[0] / p[3], p[1] / p[3], p[2] / p[3]] } else { [0.0; 3] }).collect()
    }

    pub fn luma(&self) -> Vec<f32> {
        self.rgb.par_iter().map(|c| luma(*c)).collect()
    }
}

pub(crate) fn luma(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// Valeur scalaire floutée, normalisée sur les bords (la moyenne ne tient compte que de l'image).
fn blur_scalar(values: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma < 0.3 {
        return values.to_vec();
    }
    let packed: Vec<f32> = values.iter().flat_map(|&v| [v, 0.0, 0.0, 1.0]).collect();
    gaussian(&packed, w, h, sigma).chunks_exact(4).map(|p| if p[3] > 1e-5 { p[0] / p[3] } else { 0.0 }).collect()
}

/// Échantillon bilinéaire d'une grille scalaire, bords prolongés.
fn sample1(values: &[f32], w: usize, h: usize, x: f32, y: f32) -> f32 {
    let (fx, fy) = ((x - 0.5).clamp(0.0, w as f32 - 1.0), (y - 0.5).clamp(0.0, h as f32 - 1.0));
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let at = |ix: usize, iy: usize| values[iy * w + ix];
    (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty
}

/// Échantillon bilinéaire d'une image en couleur droite, bords prolongés.
fn sample3(values: &[[f32; 3]], w: usize, h: usize, x: f32, y: f32) -> [f32; 3] {
    let (fx, fy) = ((x - 0.5).clamp(0.0, w as f32 - 1.0), (y - 0.5).clamp(0.0, h as f32 - 1.0));
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let mut out = [0.0; 3];
    for (c, slot) in out.iter_mut().enumerate() {
        let at = |ix: usize, iy: usize| values[iy * w + ix][c];
        *slot = (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty;
    }
    out
}

/// Échantillon bilinéaire d'un tampon RGBA prémultiplié (0 à 255), bords prolongés.
fn sample4(source: &[f32], w: usize, h: usize, x: f32, y: f32) -> [f32; 4] {
    let (fx, fy) = ((x - 0.5).clamp(0.0, w as f32 - 1.0), (y - 0.5).clamp(0.0, h as f32 - 1.0));
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let mut out = [0.0; 4];
    for (c, slot) in out.iter_mut().enumerate() {
        let at = |ix: usize, iy: usize| source[(iy * w + ix) * 4 + c];
        *slot = (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty;
    }
    out
}

/// Prémultiplié flottant (0 à 255) vers 8 bits, sans qu'une couleur dépasse son alpha.
fn quantize4(values: &[f32]) -> Vec<u8> {
    values
        .par_chunks_exact(4)
        .flat_map_iter(|p| {
            let a = p[3].clamp(0.0, 255.0);
            [(p[0].clamp(0.0, a) + 0.5) as u8, (p[1].clamp(0.0, a) + 0.5) as u8, (p[2].clamp(0.0, a) + 0.5) as u8, (a + 0.5) as u8]
        })
        .collect()
}

/// Générateur pseudo-aléatoire (SplitMix64) : tirages déterministes pour une graine donnée.
struct Rng(u64);

impl Rng {
    fn new(seed: u32, salt: u64) -> Self {
        Self(u64::from(seed).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ salt.wrapping_mul(0xD1B5_4A32_D192_ED03))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Dans `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// Entier dans `[low, high)` (`high > low`).
    fn range(&mut self, low: usize, high: usize) -> usize {
        low + (self.next() % (high.max(low + 1) - low) as u64) as usize
    }
    fn normal(&mut self) -> f32 {
        let (u, v) = (self.unit().max(1e-7), self.unit());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
}

/// Bruit lisse centré sur 0 (environ -1 à 1), de période `cell` pixels.
fn smooth(x: f32, y: f32, cell: f32, salt: u32) -> f32 {
    (value_noise(x, y, cell.max(1.0), salt) - 0.5) * 2.0
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Couleur d'un dégradé à étapes `(position, couleur)`, positions croissantes de 0 à 1.
fn gradient(stops: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    for pair in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (pair[0], pair[1]);
        if t <= t1 {
            return mix3(c0, c1, ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0));
        }
    }
    stops.last().map_or([0.0; 3], |s| s.1)
}

/// Moyenne de la couleur (pondérée par l'alpha) dans chaque case `cell_w x cell_h` : la grille réduite,
/// rangée ligne par ligne, et ses dimensions.
fn cell_means(img: &Straight, cell_w: usize, cell_h: usize) -> (Vec<[f32; 3]>, usize, usize) {
    let (gw, gh) = (img.w.div_ceil(cell_w.max(1)), img.h.div_ceil(cell_h.max(1)));
    let rows: Vec<Vec<[f32; 3]>> = (0..gh)
        .into_par_iter()
        .map(|gy| {
            let mut sums = vec![[0.0f32; 4]; gw];
            for y in gy * cell_h..((gy + 1) * cell_h).min(img.h) {
                for x in 0..img.w {
                    let (c, a) = (img.rgb[y * img.w + x], img.alpha[y * img.w + x]);
                    let s = &mut sums[x / cell_w];
                    s[0] += c[0] * a;
                    s[1] += c[1] * a;
                    s[2] += c[2] * a;
                    s[3] += a;
                }
            }
            sums.into_iter().map(|s| if s[3] > 1e-6 { [s[0] / s[3], s[1] / s[3], s[2] / s[3]] } else { [0.0; 3] }).collect()
        })
        .collect();
    (rows.concat(), gw, gh)
}

/// Luminosité « haute » de l'image (99e centile) : sert à étaler les tons d'une image sombre.
fn bright_point(values: &[f32]) -> f32 {
    let mut histogram = [0usize; 256];
    for &v in values {
        histogram[(v.clamp(0.0, 1.0) * 255.0) as usize] += 1;
    }
    let target = values.len() as f32 * 0.99;
    let mut seen = 0usize;
    for (i, count) in histogram.iter().enumerate() {
        seen += count;
        if seen as f32 >= target {
            return (i as f32 / 255.0).max(0.2);
        }
    }
    1.0
}

// ---------------------------------------------------------------- Flou radial

/// Flou en zoom : chaque pixel est la moyenne des pixels sur le rayon qui le relie au centre `(cx, cy)` ;
/// le centre reste net. `strength` (0 à 1) : fraction du rayon parcourue ; `fringe` (0 à 1) : le rouge
/// porte plus loin et le bleu moins, ce qui donne des franges colorées.
pub(crate) fn radial_blur(source: &[f32], w: usize, h: usize, center: (f32, f32), strength: f32, fringe: f32) -> Vec<u8> {
    // Deux passes, chacune sur la moitié du chemin (en échelle) : même étendue qu'une seule passe, un flou
    // plus doux (noyau en triangle), pour trois fois moins d'échantillons.
    let step = 1.0 - (1.0 - strength.clamp(0.0, 0.99)).sqrt();
    let first = zoom_pass(source, w, h, center, step);
    let blurred = zoom_pass(&first, w, h, center, step);
    if fringe <= 0.0 {
        return quantize4(&blurred);
    }
    // Franges : le rouge est lu un peu plus près du centre (il paraît plus grand), le bleu un peu plus loin.
    let (cx, cy) = center;
    let extra = fringe * strength * 0.12;
    let mut out = blurred.clone();
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let at = |s: f32| sample4(&blurred, w, h, cx + (px - cx) * s, cy + (py - cy) * s);
            row[x * 4] = at(1.0 - extra)[0];
            row[x * 4 + 2] = at(1.0 + extra)[2];
        }
    });
    quantize4(&out)
}

/// Une passe de flou en zoom : moyenne des pixels entre le point et sa position rapprochée du centre de
/// la fraction `k` (prémultiplié, 0 à 255).
fn zoom_pass(source: &[f32], w: usize, h: usize, (cx, cy): (f32, f32), k: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; source.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let samples = (((px - cx).hypot(py - cy) * k / 1.5).ceil() as usize).clamp(1, 32);
            let mut acc = [0.0f32; 4];
            for i in 0..samples {
                let t = if samples == 1 { 0.0 } else { i as f32 / (samples - 1) as f32 };
                let s = 1.0 - k * t;
                for (sum, v) in acc.iter_mut().zip(sample4(source, w, h, cx + (px - cx) * s, cy + (py - cy) * s)) {
                    *sum += v;
                }
            }
            for (slot, sum) in row[x * 4..x * 4 + 4].iter_mut().zip(acc) {
                *slot = sum / samples as f32;
            }
        }
    });
    out
}

// ---------------------------------------------------------------- Grain argentique

/// Grain de film : un bruit en amas (de taille `size` pixels), plus fort dans les tons moyens, comme
/// l'argent d'une pellicule ; `amount` de 0 à 1.
pub(crate) fn film_grain(img: &Straight, amount: f32, size: f32, seed: u32) -> Vec<u8> {
    let size = size.max(0.5);
    let rgb: Vec<[f32; 3]> = (0..img.w * img.h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % img.w) as f32, (i / img.w) as f32);
            let c = img.rgb[i];
            let n = if size <= 1.0 {
                normal(i % img.w, i / img.w, seed) * 0.5
            } else {
                smooth(x, y, size, seed) * 0.7 + smooth(x, y, size * 0.5, seed.wrapping_add(1)) * 0.5 + normal(i % img.w, i / img.w, seed.wrapping_add(2)) * 0.12
            };
            let l = luma(c);
            let weight = 0.25 + 3.0 * l * (1.0 - l);
            let d = n * amount * 0.32 * weight;
            [c[0] + d, c[1] + d, c[2] + d]
        })
        .collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Verre cannelé

/// L'image lue à travers un verre cannelé : des cannelures de `rib` pixels. Chacune agit comme une lentille
/// cylindrique : elle reprend une tranche plus large de l'image, retournée (`refraction`), plus floue sur
/// ses flancs ; le verre ondule un peu (`wave` pixels) et un liseré sombre marque le creux entre deux.
pub(crate) fn fluted_glass(source: &[f32], w: usize, h: usize, rib: f32, refraction: f32, wave: f32, horizontal: bool) -> Vec<u8> {
    let rib = rib.max(2.0);
    let sharp = gaussian(source, w, h, 0.8);
    let soft = gaussian(source, w, h, (rib * 0.5).max(1.0));
    let mut out = vec![0.0f32; source.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            // `a` : position le long de laquelle les cannelures se succèdent ; `b` : le long d'une cannelure.
            let (a, b) = if horizontal { (y as f32 + 0.5, x as f32 + 0.5) } else { (x as f32 + 0.5, y as f32 + 0.5) };
            let center = ((a / rib).floor() + 0.5) * rib;
            let u = (a - center) / (rib / 2.0);
            let wobble = (b / (rib * 3.1) + center * 0.37).sin() * wave;
            let sa = center - u * rib * 0.5 * refraction + wobble;
            let (sx, sy) = if horizontal { (b, sa) } else { (sa, b) };
            let (p, q) = (sample4(&sharp, w, h, sx, sy), sample4(&soft, w, h, sx, sy));
            let blend = u.abs().min(1.0) * 0.6;
            let edge = (1.0 - 0.12 * u.abs().powi(6) - 0.06 * u.max(0.0).powi(2)).clamp(0.0, 1.0);
            let alpha = p[3] + (q[3] - p[3]) * blend;
            let highlight = (1.0 - (u + 0.55).abs() * 4.0).max(0.0) * 0.06 * alpha;
            for c in 0..3 {
                // `alpha` est sur 0..255 : le reflet aussi.
                row[x * 4 + c] = (p[c] + (q[c] - p[c]) * blend) * edge + highlight;
            }
            row[x * 4 + 3] = alpha;
        }
    });
    quantize4(&out)
}

// ---------------------------------------------------------------- Vignettage

/// Bords assombris (`amount` > 0) ou éclaircis (< 0), de -1 à 1 ; `size` (0 à 1) : étendue de la zone
/// épargnée ; `softness` (0 à 1) : douceur du passage.
pub(crate) fn vignette(img: &Straight, amount: f32, size: f32, softness: f32) -> Vec<u8> {
    let (cx, cy) = (img.w as f32 / 2.0, img.h as f32 / 2.0);
    let inner = 0.2 + size * 0.9;
    let soft = 0.05 + softness * 1.1;
    let rgb: Vec<[f32; 3]> = (0..img.w * img.h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % img.w) as f32 + 0.5, (i / img.w) as f32 + 0.5);
            let d = ((x - cx) / cx).hypot((y - cy) / cy) / std::f32::consts::SQRT_2 * 1.4;
            let f = smoothstep(inner, inner + soft, d);
            let c = img.rgb[i];
            if amount >= 0.0 {
                c.map(|v| v * (1.0 - amount * f))
            } else {
                c.map(|v| v + (1.0 - v) * -amount * f)
            }
        })
        .collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Film d'essence

/// Interférences en couche mince : l'épaisseur du film varie lentement (bruit lisse de `scale` pixels) et
/// suit un peu la luminosité ; chaque épaisseur donne une teinte d'arc-en-ciel (`bands` : nombre de
/// bandes). La teinte du film remplace en partie celle de l'image, luminosité conservée, surtout dans
/// les tons moyens et les zones planes. `strength` de 0 à 1.
pub(crate) fn petrol_sheen(img: &Straight, strength: f32, bands: f32, scale: f32, seed: u32) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let soft = img.blurred(2.0);
    let l: Vec<f32> = soft.iter().map(|c| luma(*c)).collect();
    let flat = blur_scalar(&l, w, h, 3.0);
    let rgb: Vec<[f32; 3]> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let (fx, fy) = (x as f32, y as f32);
            let thick = smooth(fx, fy, scale, seed) * 0.9 + smooth(fx, fy, scale / 2.5, seed.wrapping_add(1)) * 0.3 + l[i] * 0.8;
            let phase = thick * bands;
            let film = [0.0f32, 0.36, 0.68].map(|o| 0.45 + 0.8 * (0.5 + 0.5 * (std::f32::consts::TAU * (phase + o)).cos()));
            let fl = luma(film).max(1e-3);
            let c = img.rgb[i];
            let la = luma(c);
            let tinted = [0, 1, 2].map(|k| (c[k] * 0.25 + la * 0.75) * film[k] / fl);
            let gx = flat[y * w + (x + 1).min(w - 1)] - flat[y * w + x.saturating_sub(1)];
            let gy = flat[(y + 1).min(h - 1) * w + x] - flat[y.saturating_sub(1) * w + x];
            let detail = (gx.hypot(gy) * 0.5 * 25.0).clamp(0.0, 1.0);
            let lv = l[i];
            let amount = strength * (1.0 - 0.5 * detail) * (2.6 * lv.powf(0.45) * (1.0 - lv).max(0.0).powf(1.2)).clamp(0.0, 1.0);
            mix3(c, tinted, amount)
        })
        .collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Chrome

/// Métal liquide : la luminosité adoucie (`softness` pixels) devient une hauteur dont les lignes de niveau
/// (`rings` anneaux sur toute la plage) dessinent des reflets serrés ; le métal est éclairé d'en haut à
/// gauche. Les zones sombres deviennent un fond nuit.
pub(crate) fn chrome(img: &Straight, rings: f32, softness: f32, tint: ChromeTint) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let l = img.luma();
    let field = blur_scalar(&l, w, h, softness.max(0.5));
    let detail = blur_scalar(&l, w, h, 2.5);
    let lit = blur_scalar(&detail, w, h, 2.0);
    let (metal_tint, background) = match tint {
        ChromeTint::Silver => ([0.74, 0.80, 0.93], [0.07, 0.10, 0.19]),
        ChromeTint::Gold => ([0.98, 0.80, 0.45], [0.12, 0.07, 0.03]),
        ChromeTint::Copper => ([0.95, 0.62, 0.55], [0.10, 0.04, 0.05]),
    };
    let rgb: Vec<[f32; 3]> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let f = field[i] * rings;
            let frac = f - f.floor();
            let line = (-(frac - 0.06).powi(2) / 0.0025).exp();
            let groove = (-frac.powi(2) / 0.0012).exp();
            let gx = (lit[y * w + (x + 1).min(w - 1)] - lit[y * w + x.saturating_sub(1)]) * 0.5;
            let gy = (lit[(y + 1).min(h - 1) * w + x] - lit[y.saturating_sub(1) * w + x]) * 0.5;
            let light = (0.55 + (-gx - gy) * 9.0).clamp(0.0, 1.0);
            let metal = 0.42 + 0.25 * detail[i].max(0.0).powf(0.8) + 0.28 * light + line * 0.5 - groove * 0.7;
            let presence = ((field[i] - 0.05) * 7.0).clamp(0.0, 1.0);
            let shine = metal.clamp(0.0, 1.0);
            let polished = [0, 1, 2].map(|k| shine * metal_tint[k] + shine.powi(8) * 0.3);
            mix3(background, polished, presence)
        })
        .collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Relief

/// Estampage : la différence de luminosité de part et d'autre de chaque pixel, dans la direction `angle`
/// (degrés) à `height` pixels, devient un relief gris (ou posé sur la couleur si `color`). `amount` de 0 à 5.
pub(crate) fn emboss(img: &Straight, angle: f32, height: f32, amount: f32, color: bool) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let l = img.luma();
    let (dx, dy) = (angle.to_radians().cos() * height, -angle.to_radians().sin() * height);
    let rgb: Vec<[f32; 3]> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            let relief = (sample1(&l, w, h, x - dx, y - dy) - sample1(&l, w, h, x + dx, y + dy)) * amount;
            if color {
                img.rgb[i].map(|v| v + relief)
            } else {
                let v = 0.5 + relief;
                [v; 3]
            }
        })
        .collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Demi-teinte

/// Trame de points ronds sur une grille de `cell` pixels inclinée de `angle` degrés : chaque point couvre la
/// part de sa case qui doit être encrée. Noir sur blanc, ou dans la couleur de l'image si `color`.
/// Réglages de la demi-teinte (voir `filters::Filter::Halftone`).
pub(crate) struct HalftoneStyle {
    pub pattern: HalftonePattern,
    /// -1 à 1.
    pub contrast: f32,
    pub ink: [f32; 3],
    pub paper: [f32; 3],
    /// Centre des cercles, en pixels du tampon.
    pub center: (f32, f32),
    /// Ancien réglage « En couleur » : points de la couleur de l'image sur blanc.
    pub color: bool,
}

/// Encre que demande un pixel (0 = papier, 1 = encre pleine), contraste appliqué.
fn tone_of(c: [f32; 3], contrast: f32) -> f32 {
    let t = 1.0 - luma(c);
    ((t - 0.5) * (1.0 + contrast * 1.5) + 0.5).clamp(0.0, 1.0)
}

pub(crate) fn halftone(img: &Straight, cell: f32, angle: f32, style: &HalftoneStyle) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let cell = cell.max(2.0);
    let (cos, sin) = (angle.to_radians().cos(), angle.to_radians().sin());
    let contrast = style.contrast.clamp(-1.0, 1.0);
    match style.pattern {
        HalftonePattern::Dots => {
            let soft = img.blurred(cell * 0.35);
            let rgb: Vec<[f32; 3]> = (0..w * h)
                .into_par_iter()
                .map(|i| {
                    let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                    let (u, v) = (x * cos + y * sin, -x * sin + y * cos);
                    let (cu, cv) = (((u / cell).floor() + 0.5) * cell, ((v / cell).floor() + 0.5) * cell);
                    // Centre de la case, ramené dans le repère de l'image : la couleur de toute la case vient de là.
                    let (sx, sy) = (cu * cos - cv * sin, cu * sin + cv * cos);
                    let ink = sample3(&soft, w, h, sx, sy);
                    let tone = tone_of(ink, contrast);
                    let d = ((u - cu) / cell).hypot((v - cv) / cell);
                    let radius = (tone / std::f32::consts::PI).sqrt();
                    let cover = ((radius - d) * cell * 1.3 + 0.5).clamp(0.0, 1.0);
                    if style.color {
                        mix3([1.0; 3], ink.map(|c| c * 0.85), cover)
                    } else {
                        mix3(style.paper, style.ink, cover)
                    }
                })
                .collect();
            img.compose(&rgb)
        }
        HalftonePattern::Lines | HalftonePattern::Circles => {
            let soft = img.blurred(cell * 0.25);
            let (cx, cy) = style.center;
            let rgb: Vec<[f32; 3]> = (0..w * h)
                .into_par_iter()
                .map(|i| {
                    let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                    let tone = tone_of(soft[i], contrast);
                    // Position dans la période (0 au milieu d'un trait, 0,5 entre deux traits).
                    let r = if style.pattern == HalftonePattern::Lines { -x * sin + y * cos } else { (x - cx).hypot(y - cy) };
                    let p = (r / cell).rem_euclid(1.0);
                    let dist = p.min(1.0 - p);
                    // Trait d'épaisseur proportionnelle à l'encre demandée, bord adouci sur un pixel.
                    let cover = ((tone * 0.5 - dist) * cell + 0.5).clamp(0.0, 1.0);
                    mix3(style.paper, style.ink, cover)
                })
                .collect();
            img.compose(&rgb)
        }
        HalftonePattern::Cmyk => {
            let soft = img.blurred(cell * 0.3);
            const INKS: [([f32; 3], f32); 4] = [([0.0, 0.62, 0.86], 15.0), ([0.88, 0.10, 0.48], 75.0), ([1.0, 0.92, 0.10], 0.0), ([0.10, 0.10, 0.10], 45.0)];
            let rgb: Vec<[f32; 3]> = (0..w * h)
                .into_par_iter()
                .map(|i| {
                    let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                    let c = soft[i].map(|v| ((v - 0.5) * (1.0 + contrast * 1.5) + 0.5).clamp(0.0, 1.0));
                    let k = 1.0 - c[0].max(c[1]).max(c[2]);
                    let room = (1.0 - k).max(1e-4);
                    let parts = [(1.0 - c[0] - k) / room, (1.0 - c[1] - k) / room, (1.0 - c[2] - k) / room, k];
                    let mut out = style.paper;
                    for (amount, (ink, a)) in parts.iter().zip(INKS) {
                        // L'angle du filtre tourne toutes les trames ensemble.
                        let cover = dot_cover(x, y, amount.clamp(0.0, 1.0), cell, a + angle - 45.0);
                        // Encres surimprimées : elles se multiplient.
                        out = [0, 1, 2].map(|j| out[j] * (1.0 - cover * (1.0 - ink[j])));
                    }
                    out
                })
                .collect();
            img.compose(&rgb)
        }
    }
}

/// Impression rétro : une seule encre sur papier coloré, posée en grains irréguliers (bruit gaussien de
/// taille `grain`), plus denses dans les sombres. `density` 0 à 1 (moins ou plus d'encre), `contrast`
/// -1 à 1, `bleed` 0 à 1 : l'encre bave et les grains se soudent en taches.
#[allow(clippy::too_many_arguments)]
pub(crate) fn retro_print(img: &Straight, ink: [f32; 3], paper: [f32; 3], density: f32, grain: f32, contrast: f32, bleed: f32, seed: u32) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let salt = seed.wrapping_mul(7919).wrapping_add(17);
    // Bruit gaussien par pixel, flouté à la taille du grain puis renormalisé.
    let white: Vec<f32> = (0..w * h).into_par_iter().map(|i| normal(i % w, i / w, salt)).collect();
    let sigma = (grain * 0.5).max(0.0);
    let noise = blur_scalar(&white, w, h, sigma);
    let (mean, var) = {
        let n = noise.len().max(1) as f64;
        let m = noise.par_iter().map(|v| f64::from(*v)).sum::<f64>() / n;
        (m as f32, (noise.par_iter().map(|v| (f64::from(*v) - m).powi(2)).sum::<f64>() / n) as f32)
    };
    let sd = var.sqrt().max(1e-5);
    let soft = img.blurred(0.6);
    let gain = 0.5 + density.clamp(0.0, 1.0);
    // Probabilité d'encre par pixel, puis tirage par le bruit (rang approché par une logistique).
    let cover: Vec<f32> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let tone = (tone_of(soft[i], contrast) * gain).clamp(0.0, 1.0);
            let z = (noise[i] - mean) / sd;
            let u = 1.0 / (1.0 + (-1.7 * z).exp());
            if u < tone { 1.0 } else { 0.0 }
        })
        .collect();
    // Bavure : les grains voisins se soudent (flou puis seuil), bord adouci.
    let cover = if bleed > 0.01 {
        let spread = blur_scalar(&cover, w, h, 0.4 + bleed * (1.0 + grain));
        spread.par_iter().map(|v| smoothstep(0.42, 0.58, *v)).collect()
    } else {
        blur_scalar(&cover, w, h, 0.35)
    };
    let rgb: Vec<[f32; 3]> = cover.par_iter().map(|c| mix3(paper, ink, *c)).collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Risographie

fn riso_ink(name: &str) -> [f32; 3] {
    match name {
        "pink" => [1.0, 0.28, 0.69],
        "yellow" => [1.0, 0.91, 0.0],
        "blue" => [0.0, 0.50, 0.92],
        "black" => [0.12, 0.10, 0.12],
        "red" => [1.0, 0.20, 0.24],
        "green" => [0.0, 0.60, 0.42],
        _ => [1.0, 0.45, 0.10],
    }
}

/// Part d'une case de trame couverte par un point de taille `tone`, au pixel `(x, y)` (grille de `cell`
/// pixels inclinée de `angle` degrés).
fn dot_cover(x: f32, y: f32, tone: f32, cell: f32, angle: f32) -> f32 {
    let t = angle.to_radians();
    let (u, v) = (x * t.cos() + y * t.sin(), -x * t.sin() + y * t.cos());
    let (fu, fv) = ((u / cell).rem_euclid(1.0) - 0.5, (v / cell).rem_euclid(1.0) - 0.5);
    let radius = (tone.clamp(0.0, 1.0) / std::f32::consts::PI).sqrt();
    ((radius - fu.hypot(fv)) * cell * 1.3).clamp(0.0, 1.0)
}

/// Impression Riso : séparation en encres, chacune tramée à son angle et légèrement décalée (repérage
/// imparfait, `shift` pixels), surimprimées en multiplication sur papier crème, avec du grain (0 à 1).
pub(crate) fn riso(img: &Straight, cell: f32, shift: f32, grain: f32, inks: RisoInks, seed: u32) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let cell = cell.max(2.0);
    // Le Riso est franc : couleurs saturées, contraste relevé.
    let separations: Vec<Vec<f32>> = {
        let per_pixel: Vec<[f32; 4]> = img
            .rgb
            .par_iter()
            .map(|c| {
                let g = luma(*c);
                let a = c.map(|v| ((g + (v - g) * 2.2 - 0.5) * 1.2 + 0.5).clamp(0.0, 1.0));
                let (cy, mg, ye) = (1.0 - a[0], 1.0 - a[1], 1.0 - a[2]);
                let k = cy.min(mg).min(ye);
                let room = (1.0 - k).max(1e-3);
                let blue = ((cy - k - 0.25 * (mg - k)) / room).clamp(0.0, 1.0);
                let pink = ((mg - k - 0.6 * (cy - k)) / room).clamp(0.0, 1.0);
                let yellow = ((ye - k) / room).clamp(0.0, 1.0);
                let black = (k * 1.1 - 0.15).clamp(0.0, 1.0);
                match inks {
                    RisoInks::Quad => [blue, pink, yellow, black],
                    RisoInks::Duo => [blue, (pink + yellow * 0.5).min(1.0), 0.0, 0.0],
                    RisoInks::GreenOrange => [(blue * 0.8 + black * 0.6).min(1.0), (pink + yellow * 0.7).min(1.0), 0.0, 0.0],
                    RisoInks::MonoRed => [((1.0 - luma(a)) * 1.1).clamp(0.0, 1.0), 0.0, 0.0, 0.0],
                }
            })
            .collect();
        (0..4).map(|k| per_pixel.iter().map(|p| p[k]).collect()).collect()
    };
    let plates: Vec<(&str, f32, (f32, f32))> = match inks {
        RisoInks::Quad => vec![("blue", 15.0, (0.0, 0.0)), ("pink", 75.0, (shift, -shift / 2.0)), ("yellow", 0.0, (-shift / 2.0, shift)), ("black", 45.0, (shift * 0.5, shift * 0.5))],
        RisoInks::Duo => vec![("blue", 15.0, (0.0, 0.0)), ("pink", 75.0, (shift, -shift / 2.0))],
        RisoInks::GreenOrange => vec![("green", 15.0, (0.0, 0.0)), ("orange", 75.0, (shift, -shift / 2.0))],
        RisoInks::MonoRed => vec![("red", 45.0, (0.0, 0.0))],
    };
    let paper = [0.96, 0.94, 0.90];
    let rgb: Vec<[f32; 3]> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut out = paper;
            for (k, (name, angle, (dx, dy))) in plates.iter().enumerate() {
                let tone = sample1(&separations[k], w, h, fx - dx, fy - dy);
                let mut cover = dot_cover(fx, fy, (tone * 1.15).clamp(0.0, 1.0).powf(0.85), cell, *angle);
                // L'encre Riso n'est pas parfaitement couvrante : un grain l'éclaircit par endroits.
                cover *= 1.0 - grain * hash(x, y, seed.wrapping_add(k as u32)) * 0.6;
                let ink = riso_ink(name);
                for c in 0..3 {
                    out[c] *= 1.0 - cover * (1.0 - ink[c]);
                }
            }
            let noise = normal(x, y, seed.wrapping_add(17)) * 0.025 * grain;
            out.map(|v| v + noise)
        })
        .collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Pointillisme

/// Un point du pointillisme : position, rayon, couleur.
type Dot = (f32, f32, f32, [f32; 3]);

/// Points de couleur posés au hasard, un candidat par case de `spacing` pixels, gardé d'autant plus
/// souvent que l'image est claire (ou sombre, sur papier). `size` : rayon des points en pixels.
pub(crate) fn stipple(img: &Straight, spacing: f32, size: f32, shape: StippleShape, paper: bool, seed: u32) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let spacing = spacing.max(2.0);
    let soft = img.blurred(spacing * 0.22);
    let tone: Vec<f32> = img.blurred(spacing * 0.17).iter().map(|c| luma(*c)).collect();
    let background: [f32; 3] = if paper { [0.95, 0.93, 0.88] } else { [0.06, 0.045, 0.04] };
    let (gw, gh) = ((w as f32 / spacing).ceil() as usize + 1, (h as f32 / spacing).ceil() as usize + 1);
    // Les points, case par case : position, rayon et couleur (ou rien).
    let dots: Vec<Option<Dot>> = (0..gw * gh)
        .into_par_iter()
        .map(|cell| {
            let (gx, gy) = (cell % gw, cell / gw);
            let x = (gx as f32 + hash(gx, gy, seed)) * spacing;
            let y = (gy as f32 + hash(gx, gy, seed.wrapping_add(1))) * spacing;
            if x >= w as f32 || y >= h as f32 {
                return None;
            }
            let i = (y as usize).min(h - 1) * w + (x as usize).min(w - 1);
            let l = tone[i];
            let p = if paper { (1.0 - l) * 1.3 } else { (l - 0.1) * 1.9 };
            if hash(gx, gy, seed.wrapping_add(2)) > p.clamp(0.0, 1.0).powf(0.9) {
                return None;
            }
            let r = size * (0.85 + 0.3 * hash(gx, gy, seed.wrapping_add(3)));
            let c = soft[i];
            let color = if paper { c.map(|v| v * 0.8) } else { c.map(|v| v * 1.08 + 0.03) };
            Some((x, y, r, color))
        })
        .collect();
    let reach = ((size * 1.15 * 1.5 + 1.0) / spacing).ceil() as isize + 1;
    let rgb: Vec<[f32; 3]> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (px, py) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            let (cx, cy) = ((px / spacing) as isize, (py / spacing) as isize);
            let mut out = background;
            for gy in (cy - reach).max(0)..=(cy + reach).min(gh as isize - 1) {
                for gx in (cx - reach).max(0)..=(cx + reach).min(gw as isize - 1) {
                    let Some((x, y, r, color)) = dots[gy as usize * gw + gx as usize] else { continue };
                    let (dx, dy) = (px - x, py - y);
                    let cover = match shape {
                        StippleShape::Round => r - dx.hypot(dy) + 0.5,
                        StippleShape::Square => r * 0.85 - dx.abs().max(dy.abs()) + 0.5,
                        StippleShape::Stroke => {
                            // Trait court en diagonale montante, d'épaisseur 0,7 r.
                            let along = ((dx - dy) / std::f32::consts::SQRT_2).clamp(-r * 2.0, r * 2.0);
                            let (nx, ny) = (along / std::f32::consts::SQRT_2, -along / std::f32::consts::SQRT_2);
                            r * 0.35 - (dx - nx).hypot(dy - ny) + 0.5
                        }
                    }
                    .clamp(0.0, 1.0);
                    if cover > 0.0 {
                        out = mix3(out, color, cover);
                    }
                }
            }
            out
        })
        .collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Écran CRT

/// Masque de luminophores : chaque pixel de l'écran est une triade rouge, vert, bleu de `pitch` pixels de
/// large, dont la brillance suit la couleur ; un halo (`glow`, 0 à 1) fond les points ; les bords du verre
/// bombé sont plus sombres.
pub(crate) fn crt(img: &Straight, pitch: f32, glow: f32, mask: CrtMask) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let cell = pitch.max(3.0);
    let dot = cell / 3.0;
    let (means, gw, gh) = cell_means(img, cell.round().max(1.0) as usize, dot.round().max(1.0) as usize);
    let (cw, ch) = (cell.round().max(1.0), dot.round().max(1.0));
    let lit: Vec<f32> = (0..w * h)
        .into_par_iter()
        .flat_map_iter(|i| {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            let column = ((x / cw) as usize).min(gw - 1);
            let value = means[((y / ch) as usize).min(gh - 1) * gw + column].map(|v| v.max(0.0).powf(0.8));
            let sub = (((x % cw) / (cw / 3.0)) as usize).min(2);
            let fx = (x % (cw / 3.0)) / (cw / 3.0) - 0.5 + 0.5 / (cw / 3.0);
            let fy = (y % ch) / ch - 0.5 + 0.5 / ch;
            let shape = match mask {
                CrtMask::Triads => ((0.42 - fx.hypot(fy)) * 9.0).clamp(0.0, 1.0),
                CrtMask::Grille => ((0.36 - fx.abs()) * 9.0).clamp(0.0, 1.0),
                CrtMask::Slots => {
                    let fy2 = ((y + (column % 2) as f32 * ch) % (2.0 * ch)) / (2.0 * ch) - 0.5;
                    ((0.34 - fx.abs()) * 9.0).clamp(0.0, 1.0) * ((0.44 - fy2.abs()) * 12.0).clamp(0.0, 1.0)
                }
            };
            let mut px = [0.0, 0.0, 0.0, 1.0];
            px[sub] = value[sub] * shape * 2.7;
            px
        })
        .collect();
    let near = gaussian(&lit, w, h, (dot * 0.9).max(0.5));
    let far = gaussian(&lit, w, h, (dot * 3.0).max(0.5));
    let rgb: Vec<[f32; 3]> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as f32 / w as f32 - 0.5, (i / w) as f32 / h as f32 - 0.5);
            let shade = (1.05 - 0.22 * ((x * 2.0).powi(4) + (y * 2.0).powi(4))).clamp(0.0, 1.0);
            let at = |b: &[f32], c: usize| if b[i * 4 + 3] > 1e-5 { b[i * 4 + c] / b[i * 4 + 3] } else { 0.0 };
            [0, 1, 2].map(|c| (lit[i * 4 + c] + (at(&near, c) * 0.6 + at(&far, c) * 0.6) * glow) * shade)
        })
        .collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Lignes de balayage

/// Signal vidéo : fines lignes sombres toutes les `period` pixels (profondeur `depth`, 0 à 1), contraste
/// relevé (`contrast`, 1 = neutre), et un décrochage horizontal ligne par ligne (`jitter` pixels), plus
/// marqué dans les ombres.
pub(crate) fn scanlines(img: &Straight, period: f32, depth: f32, jitter: f32, contrast: f32, seed: u32) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let period = period.max(1.5);
    let l = img.luma();
    let rgb: Vec<[f32; 3]> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            let row = smooth(0.0, y, 2.5, seed) * smooth(0.0, y, 40.0, seed.wrapping_add(1));
            let shift = row * jitter * (1.3 - l[i]);
            let c = sample3(&img.rgb, w, h, x + shift, y);
            let c = c.map(|v| ((v - 0.5) * contrast + 0.5).clamp(0.0, 1.0).powf(1.1));
            let phase = (y % period) / period;
            let line = 1.0 - depth * ((std::f32::consts::TAU * phase).cos() * 0.5 + 0.5).clamp(0.0, 1.0).powi(2);
            c.map(|v| v * line * (1.0 + depth * 0.45))
        })
        .collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Pixellisation

/// Grands pixels carrés de `cell` pixels, couleur et couverture moyennes de la case.
pub(crate) fn pixelate(source: &[f32], w: usize, h: usize, cell: usize) -> Vec<u8> {
    let cell = cell.max(1);
    let (gw, gh) = (w.div_ceil(cell), h.div_ceil(cell));
    let means: Vec<[f32; 4]> = (0..gw * gh)
        .into_par_iter()
        .map(|g| {
            let (gx, gy) = (g % gw, g / gw);
            let mut sum = [0.0f32; 4];
            let mut n = 0.0;
            for y in gy * cell..((gy + 1) * cell).min(h) {
                for x in gx * cell..((gx + 1) * cell).min(w) {
                    for c in 0..4 {
                        sum[c] += source[(y * w + x) * 4 + c];
                    }
                    n += 1.0;
                }
            }
            sum.map(|v| v / n)
        })
        .collect();
    let out: Vec<f32> = (0..w * h).into_par_iter().flat_map_iter(|i| means[(i / w / cell) * gw + (i % w) / cell]).collect();
    quantize4(&out)
}

// ---------------------------------------------------------------- Distorsion numérique

/// Le signal numérique qui se déchire : image délavée, lignes de balayage, bandes horizontales décalées,
/// canaux séparés, traînées arc-en-ciel, blocs de couleur saturée et macroblocs figés. `intensity` de 0 à 1.
pub(crate) fn glitch(img: &Straight, intensity: f32, seed: u32) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let mut rng = Rng::new(seed, 1);
    let mut out: Vec<[f32; 3]> = img
        .rgb
        .par_iter()
        .enumerate()
        .map(|(i, c)| {
            let g = luma(*c);
            let faded = c.map(|v| v * 0.45 + g * 0.55);
            if (i / w) % 2 == 0 {
                faded.map(|v| v * 0.94)
            } else {
                faded
            }
        })
        .collect();
    let roll = |row: &mut [[f32; 3]], shift: isize| {
        let n = row.len() as isize;
        if n > 0 {
            row.rotate_right(shift.rem_euclid(n) as usize);
        }
    };
    // Bandes décalées.
    let mut y = 0;
    while y < h {
        let band = rng.range(3, (h / 18).max(4));
        if rng.unit() < 0.45 * intensity {
            let shift = (rng.normal() * w as f32 * 0.06 * intensity) as isize;
            for row in y..(y + band).min(h) {
                roll(&mut out[row * w..(row + 1) * w], shift);
            }
        }
        y += band;
    }
    // Canaux séparés.
    let split = ((w as f32 * 0.006 * intensity).round() as usize).max(1);
    let snapshot = out.clone();
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            px[0] = snapshot[y * w + (x + w - split % w) % w][0];
            px[2] = snapshot[y * w + (x + split) % w][2];
        }
    });
    // Traînées arc-en-ciel.
    for _ in 0..(22.0 * intensity) as usize {
        let y0 = rng.range(0, h.saturating_sub(6).max(1));
        let hh = rng.range(2, (h / 60).max(3) + 9);
        let x0 = if rng.unit() < 0.7 { rng.range(0, (w / 3).max(1)) } else { 0 };
        let (phase, alpha) = (rng.unit(), 0.25 + rng.unit() * 0.35);
        for y in y0..(y0 + hh).min(h) {
            for x in x0..w {
                let hue = ((x - x0) as f32 / (w - x0).max(1) as f32 * 1.3 + phase).fract();
                let rainbow = [((hue * 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0), (2.0 - (hue * 6.0 - 2.0).abs()).clamp(0.0, 1.0), (2.0 - (hue * 6.0 - 4.0).abs()).clamp(0.0, 1.0)];
                out[y * w + x] = mix3(out[y * w + x], rainbow, alpha);
            }
        }
    }
    // Blocs de couleur saturée, texturés par l'image.
    let palette = [[0.15, 0.85, 0.25], [0.85, 0.1, 0.85], [0.2, 0.9, 0.9]];
    for _ in 0..(5.0 * intensity) as usize {
        let (bw, bh) = (rng.range(w / 6, (w / 3).max(w / 6 + 1)), rng.range(h / 12, (h / 6).max(h / 12 + 1)));
        let (x0, y0) = (rng.range(0, w.saturating_sub(bw).max(1)), rng.range(h / 4, h.saturating_sub(bh).max(h / 4 + 1)));
        let col = palette[rng.range(0, 3)];
        for y in y0..(y0 + bh).min(h) {
            for x in x0..(x0 + bw).min(w) {
                let g = luma(img.rgb[y * w + x]);
                out[y * w + x] = col.map(|v| v * (0.75 + 0.35 * g));
            }
        }
    }
    // Macroblocs figés : une ligne recopiée sur toute la hauteur du bloc.
    for _ in 0..(4.0 * intensity) as usize {
        let (bw, bh) = (rng.range(w / 10, (w / 4).max(w / 10 + 1)), rng.range(8.min(h), (h / 10).max(9)));
        let (x0, y0) = (rng.range(0, w.saturating_sub(bw).max(1)), rng.range(0, h.saturating_sub(bh).max(1)));
        for y in y0 + 1..(y0 + bh).min(h) {
            for x in x0..(x0 + bw).min(w) {
                out[y * w + x] = out[y0 * w + x];
            }
        }
    }
    img.compose(&out)
}

// ---------------------------------------------------------------- Datamosh

/// Vidéo corrompue : l'image en macroblocs de `block` pixels, recolorée par une palette, des blocs qui
/// « bavent » vers le bas (`bleed`, 0 à 1), des blocs recopiés ailleurs (`glitches`, 0 à 1), des plaques
/// sombres, du grain (`grain`, 0 à 1) et quelques cadres en pointillés.
pub(crate) fn datamosh(img: &Straight, block: usize, bleed: f32, glitches: f32, grain: f32, palette: MoshPalette, seed: u32) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let block = block.max(2);
    let (means, gw, gh) = cell_means(img, block, block);
    let lums: Vec<f32> = means.iter().map(|c| luma(*c)).collect();
    let (low, high) = lums.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let stops: &[(f32, [f32; 3])] = match palette {
        MoshPalette::Violet => &[(0.0, [0.07, 0.02, 0.10]), (0.22, [0.20, 0.07, 0.32]), (0.40, [0.42, 0.20, 0.66]), (0.55, [0.90, 0.42, 0.28]), (0.85, [0.98, 0.62, 0.45]), (1.0, [1.0, 0.88, 0.78])],
        MoshPalette::Acid => &[(0.0, [0.02, 0.08, 0.06]), (0.35, [0.05, 0.35, 0.30]), (0.6, [0.55, 0.85, 0.25]), (1.0, [1.0, 0.97, 0.6])],
        MoshPalette::Original => &[],
    };
    let mut grid: Vec<[f32; 3]> = means
        .iter()
        .zip(&lums)
        .map(|(c, &l)| if stops.is_empty() { *c } else { gradient(stops, (l - low) / (high - low).max(1e-3)) })
        .collect();
    let mut rng = Rng::new(seed, 2);
    // Bavure : une colonne reprend par endroits le bloc du dessus sur plusieurs rangées.
    for x in 0..gw {
        let mut y = 1;
        while y < gh {
            if rng.unit() < bleed * 0.25 {
                let n = rng.range(1, 5);
                let above = grid[(y - 1) * gw + x];
                for yy in y..(y + n).min(gh) {
                    grid[yy * gw + x] = above;
                }
                y += n;
            }
            y += 1;
        }
    }
    // Blocs recopiés ailleurs, parfois en 2 x 2.
    let count = (glitches * (gw * gh) as f32 * 0.025).round() as usize;
    for _ in 0..count {
        let source = grid[rng.range(0, gw * gh)];
        let (tx, ty, s) = (rng.range(0, gw.max(2) - 1), rng.range(0, gh.max(2) - 1), rng.range(1, 3));
        for yy in ty..(ty + s).min(gh) {
            for xx in tx..(tx + s).min(gw) {
                grid[yy * gw + xx] = source;
            }
        }
    }
    // Plaques sombres translucides (blocs de compression ratés).
    for _ in 0..count / 2 {
        let (tx, ty, s) = (rng.range(0, gw.max(3) - 2), rng.range(0, gh.max(3) - 2), rng.range(2, 4));
        for yy in ty..(ty + s).min(gh) {
            for xx in tx..(tx + s).min(gw) {
                grid[yy * gw + xx] = grid[yy * gw + xx].map(|v| v * 0.72);
            }
        }
    }
    let mut rgb: Vec<[f32; 3]> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let n = normal(x, y, seed.wrapping_add(5)) * grain * 0.1;
            grid[(y / block) * gw + x / block].map(|v| v + n)
        })
        .collect();
    // Cadres en pointillés, calés sur les blocs.
    let dash = (block / 7).max(2);
    for _ in 0..3 {
        if gw < 6 || gh < 5 {
            break;
        }
        let (x0, y0) = (rng.range(0, gw - 4) * block, rng.range(0, gh - 3) * block);
        let (x1, y1) = ((x0 + rng.range(3, 5) * block).min(w - 1), (y0 + rng.range(2, 4) * block).min(h - 1));
        let line = [0.92, 0.88, 0.96];
        for x in (x0..=x1).filter(|x| (x - x0) % (dash * 2) < dash) {
            rgb[y0 * w + x] = line;
            rgb[y1 * w + x] = line;
        }
        for y in (y0..=y1).filter(|y| (y - y0) % (dash * 2) < dash) {
            rgb[y * w + x0] = line;
            rgb[y * w + x1] = line;
        }
    }
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Tri de pixels

/// Dans chaque ligne (ou colonne), les suites de pixels dont la luminosité est entre `low` et `high`
/// (0 à 1) sont triées par luminosité : les zones claires s'étirent en traînées.
pub(crate) fn pixel_sort(source: &[f32], w: usize, h: usize, low: f32, high: f32, direction: SortDirection) -> Vec<u8> {
    let vertical = matches!(direction, SortDirection::Down | SortDirection::Up);
    let descending = matches!(direction, SortDirection::Right | SortDirection::Down);
    let (lines, length) = if vertical { (w, h) } else { (h, w) };
    let index = move |line: usize, k: usize| if vertical { k * w + line } else { line * w + k };
    let brightness = |p: &[f32; 4]| if p[3] > 0.0 { (0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]) / p[3] } else { 0.0 };
    let sorted: Vec<Vec<[f32; 4]>> = (0..lines)
        .into_par_iter()
        .map(|line| {
            let mut pixels: Vec<[f32; 4]> = (0..length).map(|k| {
                let i = index(line, k) * 4;
                [source[i], source[i + 1], source[i + 2], source[i + 3]]
            }).collect();
            let inside: Vec<bool> = pixels.iter().map(|p| p[3] > 0.0 && (low..=high).contains(&brightness(p))).collect();
            let mut k = 0;
            while k < length {
                if !inside[k] {
                    k += 1;
                    continue;
                }
                let start = k;
                while k < length && inside[k] {
                    k += 1;
                }
                let run = &mut pixels[start..k];
                run.sort_by(|a, b| brightness(a).total_cmp(&brightness(b)));
                if descending {
                    run.reverse();
                }
            }
            pixels
        })
        .collect();
    let mut out = vec![0.0f32; source.len()];
    for (line, pixels) in sorted.iter().enumerate() {
        for (k, p) in pixels.iter().enumerate() {
            out[index(line, k) * 4..index(line, k) * 4 + 4].copy_from_slice(p);
        }
    }
    quantize4(&out)
}

// ---------------------------------------------------------------- Peinture à l'huile

/// Moyennes glissantes de côté `2q+1` de plusieurs couches (rangées couche par couche), bords normalisés.
fn box_means(layers: &[Vec<f32>], w: usize, h: usize, q: usize) -> Vec<Vec<f32>> {
    layers
        .par_iter()
        .map(|values| {
            // Lignes puis colonnes, en sommes cumulées.
            let mut rows = vec![0.0f32; w * h];
            rows.par_chunks_mut(w).enumerate().for_each(|(y, out)| {
                let line = &values[y * w..(y + 1) * w];
                let mut prefix = vec![0.0f64; w + 1];
                for x in 0..w {
                    prefix[x + 1] = prefix[x] + f64::from(line[x]);
                }
                for (x, slot) in out.iter_mut().enumerate() {
                    let (a, b) = (x.saturating_sub(q), (x + q + 1).min(w));
                    *slot = ((prefix[b] - prefix[a]) / (b - a) as f64) as f32;
                }
            });
            let mut out = vec![0.0f32; w * h];
            let columns: Vec<Vec<f32>> = (0..w)
                .into_par_iter()
                .map(|x| {
                    let mut prefix = vec![0.0f64; h + 1];
                    for y in 0..h {
                        prefix[y + 1] = prefix[y] + f64::from(rows[y * w + x]);
                    }
                    (0..h).map(|y| {
                        let (a, b) = (y.saturating_sub(q), (y + q + 1).min(h));
                        ((prefix[b] - prefix[a]) / (b - a) as f64) as f32
                    }).collect()
                })
                .collect();
            for (x, column) in columns.iter().enumerate() {
                for (y, v) in column.iter().enumerate() {
                    out[y * w + x] = *v;
                }
            }
            out
        })
        .collect()
}

/// Filtre de Kuwahara : chaque pixel prend la couleur moyenne du quart de voisinage (rayon `radius`) le
/// plus uniforme. Les contours restent nets et les aplats se forment, comme des touches de peinture.
pub(crate) fn oil_paint(img: &Straight, radius: f32) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let q = ((radius / 2.0).round() as usize).max(1);
    let l = img.luma();
    let layers = vec![img.rgb.iter().map(|c| c[0]).collect(), img.rgb.iter().map(|c| c[1]).collect(), img.rgb.iter().map(|c| c[2]).collect(), l.clone(), l.iter().map(|v| v * v).collect()];
    let means = box_means(&layers, w, h, q);
    let rgb: Vec<[f32; 3]> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            let mut best = (f32::MAX, img.rgb[i]);
            for (ox, oy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                let (sx, sy) = ((x + ox * q as isize).clamp(0, w as isize - 1) as usize, (y + oy * q as isize).clamp(0, h as isize - 1) as usize);
                let j = sy * w + sx;
                let variance = means[4][j] - means[3][j] * means[3][j];
                if variance < best.0 {
                    best = (variance, [means[0][j], means[1][j], means[2][j]]);
                }
            }
            best.1
        })
        .collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Postérisation et contours

pub(crate) fn posterize(img: &Straight, levels: u32) -> Vec<u8> {
    let n = levels.max(2) as f32 - 1.0;
    let rgb: Vec<[f32; 3]> = img.rgb.par_iter().map(|c| c.map(|v| (v * n).round() / n)).collect();
    img.compose(&rgb)
}

/// Contours (Sobel) d'intensité `amount` (1 = normal) : traits foncés sur blanc, ou de la couleur de
/// l'image sur blanc si `color`.
pub(crate) fn contours(img: &Straight, amount: f32, color: bool) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let l = blur_scalar(&img.luma(), w, h, 0.8);
    let at = |x: isize, y: isize| l[(y.clamp(0, h as isize - 1) as usize) * w + x.clamp(0, w as isize - 1) as usize];
    let rgb: Vec<[f32; 3]> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            let gx = at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x - 1, y) - at(x - 1, y + 1);
            let gy = at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1) - at(x - 1, y - 1) - 2.0 * at(x, y - 1) - at(x + 1, y - 1);
            let edge = (gx.hypot(gy) * amount * 1.6).clamp(0.0, 1.0);
            let ink = if color { img.rgb[i].map(|v| v * 0.8) } else { [0.08; 3] };
            mix3([1.0; 3], ink, edge)
        })
        .collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Truchet

/// Distance au tracé du motif dans une case (`u`, `v` de 0 à 1) ; `pick` : tirage de la case (0 à 1).
/// Pour les motifs pleins, 0 à l'intérieur et 1 dehors (et `true`).
fn truchet_field(u: f32, v: f32, pick: f32, pattern: TruchetPattern) -> (f32, bool) {
    let flip = pick < 0.5;
    let uf = if flip { 1.0 - u } else { u };
    let corners = |uu: f32, vv: f32, radii: &[f32]| radii.iter().fold(f32::MAX, |m, &r| m.min((uu.hypot(vv) - r).abs()).min(((1.0 - uu).hypot(1.0 - vv) - r).abs()));
    let d = match pattern {
        TruchetPattern::Arcs | TruchetPattern::MultiScale => corners(uf, v, &[0.5]),
        TruchetPattern::Double => corners(uf, v, &[1.0 / 3.0, 2.0 / 3.0]),
        TruchetPattern::Triple => corners(uf, v, &[0.25, 0.5, 0.75]),
        TruchetPattern::Fan => {
            let (uu, vv) = match (pick * 4.0) as u32 % 4 {
                0 => (u, v),
                1 => (1.0 - u, v),
                2 => (1.0 - u, 1.0 - v),
                _ => (u, 1.0 - v),
            };
            [0.2f32, 0.45, 0.7, 0.95].iter().fold(f32::MAX, |m, &r| m.min((uu.hypot(vv) - r).abs()))
        }
        TruchetPattern::Diagonals => (uf - v).abs() / std::f32::consts::SQRT_2,
        TruchetPattern::Ribbons => (corners(uf, v, &[0.5]) - 0.16).abs(),
        TruchetPattern::Bevels => ((uf + v - 0.5).abs()).min(((1.0 - uf) + (1.0 - v) - 0.5).abs()) / std::f32::consts::SQRT_2,
        TruchetPattern::Rings => ((u - 0.5).hypot(v - 0.5) - (0.18 + 0.2 * pick)).abs(),
        TruchetPattern::Mixed => {
            if pick > 0.66 {
                (u - 0.5).abs().min((v - 0.5).abs())
            } else {
                corners(if pick < 0.33 { 1.0 - u } else { u }, v, &[0.5])
            }
        }
        TruchetPattern::Weave => {
            if flip {
                (v - 0.5).abs()
            } else {
                (u - 0.5).abs()
            }
        }
        TruchetPattern::Maze => {
            if flip {
                v.min(1.0 - v)
            } else {
                u.min(1.0 - u)
            }
        }
        TruchetPattern::DotsArcs => {
            let dots = [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].iter().fold(f32::MAX, |m, &(cx, cy)| m.min((u - cx).hypot(v - cy))) - 0.12;
            corners(uf, v, &[0.5]).min(dots.abs())
        }
        TruchetPattern::FilledQuarters => return (if uf.hypot(v) < 0.5 || (1.0 - uf).hypot(1.0 - v) < 0.5 { 0.0 } else { 1.0 }, true),
        TruchetPattern::Triangles => return (if uf + v < 1.0 { 0.0 } else { 1.0 }, true),
    };
    (d, false)
}

/// Tuiles de Truchet de `cell` pixels : des tubes lumineux (épaisseur `width`, en part de la case ; halo
/// `glow`, 0 à 1) dont la couleur et l'éclat viennent de l'image. Une image sombre est d'abord étalée,
/// pour que le motif reste visible.
#[allow(clippy::too_many_arguments)]
pub(crate) fn truchet(img: &Straight, cell: f32, width: f32, glow: f32, pattern: TruchetPattern, color: TruchetColor, seed: u32) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let cell = cell.max(3.0);
    let soft = img.blurred(cell * 0.8);
    let lums: Vec<f32> = soft.iter().map(|c| luma(*c)).collect();
    let top = bright_point(&lums);
    // Couche du motif en un point, pour une taille de case : cœur du tube et halo.
    let layer = |x: f32, y: f32, size: f32| -> (f32, f32) {
        let (gx, gy) = ((x / size).floor(), (y / size).floor());
        let (u, v) = (x / size - gx, y / size - gy);
        let pick = hash((gx + 65536.0) as usize, (gy + 65536.0) as usize, seed);
        let (d, filled) = truchet_field(u, v, pick, pattern);
        if filled {
            ((1.0 - d) * 0.85, 0.0)
        } else {
            ((1.0 - (d / width).powi(2)).clamp(0.0, 1.0).powf(0.6), (-(d / (width * 2.6)).powi(2)).exp() * glow)
        }
    };
    let rgb: Vec<[f32; 3]> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            let s = soft[i].map(|v| (v / top).min(1.0));
            let l = luma(s);
            let (core, halo) = if pattern == TruchetPattern::MultiScale {
                let (c1, h1) = layer(x, y, cell / 0.6);
                let (c2, h2) = layer(x, y, cell / 1.6);
                let m = ((l - 0.3) * 3.0).clamp(0.0, 1.0);
                (c1 + (c2 - c1) * m, h1 + (h2 - h1) * m)
            } else {
                layer(x, y, cell)
            };
            let shape = core + halo * 0.35;
            match color {
                TruchetColor::Mono => [(0.1 + 1.1 * l.powf(1.4)).min(1.0) * shape; 3],
                TruchetColor::Neon => {
                    let c = gradient(&[(0.0, [0.15, 0.0, 0.35]), (0.35, [0.55, 0.1, 0.9]), (0.65, [1.0, 0.2, 0.6]), (1.0, [1.0, 0.95, 0.5])], l * 1.3);
                    c.map(|v| v * shape * 1.15)
                }
                TruchetColor::Paper => mix3([0.96, 0.94, 0.90], s.map(|v| v * 0.55), core),
                TruchetColor::Image => {
                    let peak = s[0].max(s[1]).max(s[2]).max(1e-4);
                    let tone = 0.12 + 1.05 * l.powf(1.8);
                    let white = (l - 0.82).max(0.0) * 2.0;
                    [0, 1, 2].map(|k| ((0.35 * s[k] / peak + 0.65 * s[k] / l.max(0.05)) * tone + white) * shape)
                }
            }
        })
        .collect();
    img.compose(&rgb)
}

// ---------------------------------------------------------------- Aberration organique

/// Aberration « organique » : le rouge et le bleu (magenta) partent d'un côté, le vert de l'autre, de
/// `amount` pixels, le long d'un champ de déformation lisse (`warp` pixels) qui grandit vers les bords ;
/// chaque couche est la moyenne de copies échelonnées (images fantômes), et un bruit par ligne étire le
/// tout en traînées horizontales (`streak` pixels).
pub(crate) fn organic_aberration(source: &[f32], w: usize, h: usize, amount: f32, warp: f32, streak: f32) -> Vec<f32> {
    let scale = (w.max(h) as f32 * 0.17).max(8.0);
    let ghosts = 4;
    let mut out = vec![0.0f32; source.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let fy = y as f32 + 0.5;
        let lines = smooth(0.0, fy, 3.0, 31);
        for x in 0..w {
            let fx = x as f32 + 0.5;
            let (dx, dy) = (smooth(fx, fy, scale, 11), smooth(fx, fy, scale, 12));
            let rows = lines * smooth(fx, fy, scale * 1.5, 13);
            let edge = 0.35 + (fx / w as f32 - 0.5).hypot(fy / h as f32 - 0.5) * 1.8;
            let mut alpha = 0.0f32;
            for (c, k) in [(0usize, 1.0f32), (1, -1.0), (2, 0.75)] {
                let mut acc = 0.0;
                for g in 0..ghosts {
                    let t = (g as f32 + 0.5) / ghosts as f32;
                    let ox = dx * warp + k * amount * edge * t * (0.7 + 0.5 * dy) + rows * streak * t;
                    let oy = dy * warp * 0.4 + k * amount * 0.3 * edge * t * dx;
                    let p = sample4(source, w, h, fx + ox, fy + oy);
                    acc += p[c];
                    if c == 1 {
                        alpha += p[3];
                    }
                }
                row[x * 4 + c] = acc / ghosts as f32;
            }
            row[x * 4 + 3] = alpha / ghosts as f32;
            // Franges vives : la saturation monte un peu, comme dans un vrai objectif.
            let a = row[x * 4 + 3].max(1e-3);
            let px = [row[x * 4] / a, row[x * 4 + 1] / a, row[x * 4 + 2] / a];
            let g = luma(px);
            for c in 0..3 {
                row[x * 4 + c] = ((g + (px[c] - g) * 1.4) * 0.92).clamp(0.0, 255.0) * a;
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient_image(w: usize, h: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let (r, g, b) = (x as f32 / w as f32 * 255.0, y as f32 / h as f32 * 255.0, 128.0);
                out.extend_from_slice(&[r, g, b, 255.0]);
            }
        }
        out
    }

    #[test]
    fn every_pattern_draws_something_and_is_deterministic() {
        let (w, h) = (96, 64);
        let img = Straight::from_premultiplied(&gradient_image(w, h), w, h);
        let patterns = [
            TruchetPattern::Arcs,
            TruchetPattern::Double,
            TruchetPattern::Triple,
            TruchetPattern::Fan,
            TruchetPattern::Diagonals,
            TruchetPattern::Ribbons,
            TruchetPattern::Bevels,
            TruchetPattern::Rings,
            TruchetPattern::Mixed,
            TruchetPattern::Weave,
            TruchetPattern::Maze,
            TruchetPattern::DotsArcs,
            TruchetPattern::FilledQuarters,
            TruchetPattern::Triangles,
            TruchetPattern::MultiScale,
        ];
        for pattern in patterns {
            let a = truchet(&img, 12.0, 0.1, 0.5, pattern, TruchetColor::Mono, 3);
            let b = truchet(&img, 12.0, 0.1, 0.5, pattern, TruchetColor::Mono, 3);
            assert_eq!(a, b, "{pattern:?} : même graine, même image");
            let lit = a.chunks_exact(4).filter(|p| p[0] > 60).count();
            assert!(lit > w * h / 50 && lit < w * h * 9 / 10, "{pattern:?} : motif visible ({lit} pixels allumés)");
        }
    }

    #[test]
    fn color_filters_keep_alpha_and_size() {
        let (w, h) = (40, 30);
        let mut source = gradient_image(w, h);
        // Une moitié transparente.
        for px in source.chunks_exact_mut(4).skip(w * h / 2) {
            px.copy_from_slice(&[0.0; 4]);
        }
        let img = Straight::from_premultiplied(&source, w, h);
        let outputs = [
            vignette(&img, 0.6, 0.5, 0.5),
            petrol_sheen(&img, 0.9, 1.6, 50.0, 1),
            chrome(&img, 9.0, 3.0, ChromeTint::Gold),
            emboss(&img, 45.0, 2.0, 2.0, false),
            halftone(&img, 6.0, 45.0, &HalftoneStyle { pattern: HalftonePattern::Dots, contrast: 0.0, ink: [0.0; 3], paper: [1.0; 3], center: (8.0, 8.0), color: true }),
            halftone(&img, 6.0, 45.0, &HalftoneStyle { pattern: HalftonePattern::Circles, contrast: 0.3, ink: [0.1; 3], paper: [1.0; 3], center: (8.0, 8.0), color: false }),
            halftone(&img, 6.0, 45.0, &HalftoneStyle { pattern: HalftonePattern::Lines, contrast: 0.0, ink: [0.1; 3], paper: [1.0; 3], center: (8.0, 8.0), color: false }),
            halftone(&img, 6.0, 45.0, &HalftoneStyle { pattern: HalftonePattern::Cmyk, contrast: 0.0, ink: [0.1; 3], paper: [1.0; 3], center: (8.0, 8.0), color: false }),
            retro_print(&img, [0.1; 3], [0.95; 3], 0.5, 1.0, 0.0, 0.2, 3),
            riso(&img, 4.0, 2.0, 0.3, RisoInks::Quad, 1),
            stipple(&img, 5.0, 2.0, StippleShape::Stroke, true, 1),
            crt(&img, 6.0, 0.8, CrtMask::Slots),
            scanlines(&img, 3.0, 0.5, 6.0, 1.1, 1),
            glitch(&img, 0.8, 4),
            datamosh(&img, 6, 0.5, 0.5, 0.3, MoshPalette::Violet, 2),
            oil_paint(&img, 4.0),
            posterize(&img, 4),
            contours(&img, 1.0, true),
            film_grain(&img, 0.5, 2.0, 1),
        ];
        for (k, out) in outputs.iter().enumerate() {
            assert_eq!(out.len(), w * h * 4, "filtre {k} : taille");
            for (i, px) in out.chunks_exact(4).enumerate() {
                let expected = if i < w * h / 2 { 255 } else { 0 };
                assert_eq!(px[3], expected, "filtre {k} : alpha conservé au pixel {i}");
                assert!(px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3], "filtre {k} : prémultiplié valide");
            }
        }
    }

    #[test]
    fn pixel_sort_only_reorders_the_pixels_of_a_line() {
        let (w, h) = (32, 4);
        let source = gradient_image(w, h);
        let sorted = pixel_sort(&source, w, h, 0.0, 1.0, SortDirection::Right);
        // Toute la ligne est dans la plage : elle est triée du plus clair au plus sombre.
        let first: Vec<u8> = sorted.chunks_exact(4).take(w).map(|p| p[0]).collect();
        assert!(first.windows(2).all(|p| p[0] >= p[1]), "{first:?}");
        let mut a: Vec<u8> = sorted.chunks_exact(4).take(w).map(|p| p[0]).collect();
        let mut b: Vec<u8> = quantize4(&source).chunks_exact(4).take(w).map(|p| p[0]).collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b, "les mêmes pixels, dans un autre ordre");
    }

    #[test]
    fn radial_blur_keeps_the_center_sharp() {
        let (w, h) = (64, 64);
        let source = gradient_image(w, h);
        let out = radial_blur(&source, w, h, (32.0, 32.0), 0.4, 0.0);
        let at = |x: usize, y: usize| out[(y * w + x) * 4] as i32;
        let original = |x: usize, y: usize| (source[(y * w + x) * 4] + 0.5) as i32;
        assert!((at(32, 32) - original(32, 32)).abs() <= 2, "le centre reste net");
        assert!(out.chunks_exact(4).all(|p| p[3] == 255), "image opaque : reste opaque");
    }
}
