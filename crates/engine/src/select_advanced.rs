//! Sélection avancée : détourer les sujets difficiles (arbres, cheveux, fourrure) comme le font les graphistes, en
//! partant de la couche où le sujet contraste le plus avec le fond.
//!
//! 1. Une **carte de départ** en niveaux de gris : une couche (rouge, vert, bleu, luminosité ou un mélange), poussée
//!    en noir et blanc par des niveaux (noir, gamma, blanc : le « seuil doux »), ou une plage de couleurs, ou une plage
//!    de tons.
//! 2. **Affinage des contours** par un filtre guidé (He, Sun et Tang, 2010) : près du contour, la carte suit les
//!    détails de l'image elle-même (mèches, feuilles), dans un rayon donné (« rayon intelligent ») et, plus large, là
//!    où le pinceau Affiner est passé. Puis lissage, décalage du contour et contour progressif.
//! 3. Pinceaux **+ / −** : sujet ou fond forcés.
//! 4. **Décontamination** (pour un nouveau calque) : la couleur du fond restée dans les pixels du bord est retirée en
//!    estimant la couleur du sujet seul (équation de composition `I = αF + (1 − α)B`).
//!
//! Tout se calcule sur une image RGBA droite (alpha non prémultiplié) en repère document : l'aperçu travaille sur une
//! version réduite (réglages mis à l'échelle), le résultat final sur l'image entière.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::document::{Document, LayerId};
use crate::error::EngineError;
use crate::tiles::{MaskStore, TILE_SIZE, TileBuf, TileCoord, tiles_for};

/// Méthode de la carte de départ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    /// Une couche (ou un mélange de couches), puis des niveaux.
    #[default]
    Channel,
    /// Les couleurs prélevées, avec une tolérance.
    Color,
    /// Une plage de luminosité (ombres, tons moyens, hautes lumières).
    Tones,
}

/// Action d'un passage de pinceau.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StrokeMode {
    /// Recalcule le contour sous le pinceau, avec un grand rayon (cheveux, bords flous).
    Refine,
    /// Force le sujet.
    Add,
    /// Force le fond.
    Remove,
}

/// Un point de pinceau, en pixels du document.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dab {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub mode: StrokeMode,
}

/// Tous les réglages de la fenêtre.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Params {
    pub method: Method,
    /// Mélange des couches, en % (rouge, vert, bleu) ; luminosité : 30 / 59 / 11.
    pub mix: [f32; 3],
    /// Niveaux de la couche (0 à 255) et gamma (0,1 à 10).
    pub black: f32,
    pub gamma: f32,
    pub white: f32,
    /// Inverser la carte (sujet sombre sur fond clair).
    pub invert: bool,
    /// Couleurs prélevées (méthode Couleur) et tolérance (0 à 200).
    pub colors: Vec<[u8; 3]>,
    pub fuzziness: f32,
    /// Plage de tons : luminosités gardées (0 à 255) et adoucissement de ses bords.
    pub tone_low: f32,
    pub tone_high: f32,
    pub tone_soft: f32,
    /// Rayon intelligent (px), lissage (0 à 10), contour progressif (px), décalage du contour (−100 à 100 %).
    pub radius: f32,
    pub smooth: f32,
    pub feather: f32,
    pub shift: f32,
    /// Décontaminer les couleurs (nouveau calque) et intensité (0 à 1).
    pub decontaminate: bool,
    pub amount: f32,
    pub dabs: Vec<Dab>,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            method: Method::Channel,
            mix: [0.0, 0.0, 100.0],
            black: 0.0,
            gamma: 1.0,
            white: 255.0,
            invert: false,
            colors: Vec::new(),
            fuzziness: 40.0,
            tone_low: 0.0,
            tone_high: 85.0,
            tone_soft: 20.0,
            radius: 4.0,
            smooth: 0.0,
            feather: 0.0,
            shift: 0.0,
            decontaminate: false,
            amount: 0.7,
            dabs: Vec::new(),
        }
    }
}

/// Analyse d'une couche : contraste sujet / fond (0 à 1), seuil et réglages proposés.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelScore {
    /// Part de la variance expliquée par la séparation en deux classes (critère d'Otsu) : 1 = deux tons purs.
    pub score: f32,
    pub threshold: f32,
    pub black: f32,
    pub white: f32,
    /// Le fond (le bord de l'image) est clair : le sujet est la partie sombre, la carte doit être inversée.
    pub invert: bool,
}

/// Mélanges des quatre couches proposées : rouge, vert, bleu, luminosité.
pub const CHANNELS: [[f32; 3]; 4] = [[100.0, 0.0, 0.0], [0.0, 100.0, 0.0], [0.0, 0.0, 100.0], [30.0, 59.0, 11.0]];

/// Couche mélangée (0 à 255) d'une image RGBA droite ; un pixel transparent compte comme noir.
pub fn mixed(rgba: &[u8], mix: [f32; 3]) -> Vec<f32> {
    let k = mix.map(|v| v / 100.0);
    rgba.par_chunks_exact(4)
        .map(|p| {
            let a = f32::from(p[3]) / 255.0;
            ((f32::from(p[0]) * k[0] + f32::from(p[1]) * k[1] + f32::from(p[2]) * k[2]) * a).clamp(0.0, 255.0)
        })
        .collect()
}

/// Score de contraste d'une couche (voir [`ChannelScore`]).
pub fn score(channel: &[f32], width: usize, height: usize) -> ChannelScore {
    let mut histogram = [0u64; 256];
    for v in channel {
        histogram[(*v as usize).min(255)] += 1;
    }
    let total = channel.len().max(1) as f64;
    let mean: f64 = histogram.iter().enumerate().map(|(i, n)| i as f64 * *n as f64).sum::<f64>() / total;
    let variance: f64 = histogram.iter().enumerate().map(|(i, n)| (i as f64 - mean).powi(2) * *n as f64).sum::<f64>() / total;
    // Otsu : le seuil qui maximise la variance entre les deux classes.
    let mut between = [0.0f64; 256];
    let (mut w0, mut sum0) = (0.0f64, 0.0f64);
    let sum: f64 = mean * total;
    for t in 0..256 {
        w0 += histogram[t] as f64;
        sum0 += t as f64 * histogram[t] as f64;
        let w1 = total - w0;
        if w0 == 0.0 || w1 == 0.0 {
            continue;
        }
        let (m0, m1) = (sum0 / w0, (sum - sum0) / w1);
        between[t] = w0 * w1 * (m0 - m1).powi(2) / (total * total);
    }
    let best = between.iter().copied().fold(0.0f64, f64::max);
    // Plusieurs seuils également bons (un creux vide entre deux pics) : le milieu du creux.
    let ties: Vec<usize> = (0..256).filter(|t| best > 0.0 && between[*t] >= best * (1.0 - 1e-9)).collect();
    let best_t = if ties.is_empty() { 128 } else { (ties[0] + ties[ties.len() - 1]) / 2 };
    let threshold = best_t as f32 + 0.5;
    let (mut n0, mut s0, mut n1, mut s1) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (i, n) in histogram.iter().enumerate() {
        if (i as f32) < threshold {
            n0 += *n as f64;
            s0 += i as f64 * *n as f64;
        } else {
            n1 += *n as f64;
            s1 += i as f64 * *n as f64;
        }
    }
    let (m0, m1) = ((s0 / n0.max(1.0)) as f32, (s1 / n1.max(1.0)) as f32);
    // Le fond est ce qui touche le bord de l'image.
    let mut bright_border = 0usize;
    let mut border = 0usize;
    for x in 0..width {
        for y in [0, height.saturating_sub(1)] {
            border += 1;
            bright_border += usize::from(channel[y * width + x] >= threshold);
        }
    }
    for y in 0..height {
        for x in [0, width.saturating_sub(1)] {
            border += 1;
            bright_border += usize::from(channel[y * width + x] >= threshold);
        }
    }
    // Séparation (Otsu : 1 = deux classes nettes) pondérée par l'écart entre les deux classes : deux tons purs mais
    // proches séparent moins bien, à l'œil comme au seuil, que deux tons éloignés.
    let separation = if variance > 0.0 { (best / variance) as f32 } else { 0.0 };
    ChannelScore {
        score: separation * ((m1 - m0) / 255.0).clamp(0.0, 1.0),
        threshold,
        black: (threshold - (threshold - m0) * 0.45).clamp(0.0, 254.0),
        white: (threshold + (m1 - threshold) * 0.45).clamp(1.0, 255.0),
        invert: bright_border * 2 > border,
    }
}

/// Carte de départ (0 à 1).
fn base_map(rgba: &[u8], p: &Params) -> Vec<f32> {
    let mut map: Vec<f32> = match p.method {
        Method::Channel => {
            let (black, white) = (p.black.clamp(0.0, 254.0), p.white.clamp(1.0, 255.0).max(p.black + 1.0));
            let gamma = p.gamma.clamp(0.1, 10.0);
            mixed(rgba, p.mix).into_par_iter().map(|v| ((v - black) / (white - black)).clamp(0.0, 1.0).powf(1.0 / gamma)).collect()
        }
        Method::Color => {
            let tolerance = p.fuzziness.clamp(1.0, 200.0);
            rgba.par_chunks_exact(4)
                .map(|px| {
                    p.colors
                        .iter()
                        .map(|c| {
                            let d = ((f32::from(px[0]) - f32::from(c[0])).powi(2) + (f32::from(px[1]) - f32::from(c[1])).powi(2) + (f32::from(px[2]) - f32::from(c[2])).powi(2)).sqrt();
                            (1.0 - d / tolerance).clamp(0.0, 1.0)
                        })
                        .fold(0.0f32, f32::max)
                        * f32::from(px[3])
                        / 255.0
                })
                .collect()
        }
        Method::Tones => {
            let soft = p.tone_soft.max(0.5);
            let (low, high) = (p.tone_low.min(p.tone_high), p.tone_high.max(p.tone_low));
            mixed(rgba, CHANNELS[3])
                .into_par_iter()
                .map(|v| {
                    let inside = ((v - (low - soft)) / soft).clamp(0.0, 1.0).min(((high + soft) - v) / soft).clamp(0.0, 1.0);
                    if (low <= 0.5 && v <= low) || (high >= 254.5 && v >= high) { 1.0 } else { inside }
                })
                .collect()
        }
    };
    if p.invert {
        map.par_iter_mut().for_each(|v| *v = 1.0 - *v);
    }
    map
}

/// Flou en boîte de rayon `r` (somme glissante), lignes puis colonnes.
fn box_blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    if r == 0 || w == 0 || h == 0 {
        return src.to_vec();
    }
    let horizontal = blur_rows(src, w, r);
    let vertical = blur_rows(&transpose(&horizontal, w, h), h, r);
    transpose(&vertical, h, w)
}

/// Flou en boîte de chaque ligne (largeur `w`) ; au bord, la moyenne porte sur les pixels présents.
fn blur_rows(data: &[f32], w: usize, r: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; data.len()];
    out.par_chunks_exact_mut(w).zip(data.par_chunks_exact(w)).for_each(|(o, row)| {
        let mut acc: f64 = row[..(r + 1).min(w)].iter().map(|v| f64::from(*v)).sum();
        for x in 0..w {
            let (lo, hi) = (x.saturating_sub(r), (x + r).min(w - 1));
            o[x] = (acc / (hi - lo + 1) as f64) as f32;
            if x + r + 1 < w {
                acc += f64::from(row[x + r + 1]);
            }
            if x >= r {
                acc -= f64::from(row[x - r]);
            }
        }
    });
    out
}

fn transpose(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    out.par_chunks_exact_mut(h).enumerate().for_each(|(x, column)| {
        for (y, v) in column.iter_mut().enumerate() {
            *v = src[y * w + x];
        }
    });
    out
}

/// Flou d'allure gaussienne d'écart type `sigma` (trois flous en boîte).
fn gaussian(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma < 0.3 {
        return src.to_vec();
    }
    let r = ((sigma * sigma * 4.0 + 1.0).sqrt() / 2.0).round().max(1.0) as usize;
    let a = box_blur(src, w, h, r);
    let b = box_blur(&a, w, h, r);
    box_blur(&b, w, h, r)
}

/// Filtre guidé : `p` suit les contours de `guide` dans un rayon `r`. `eps` : lissage (variance du guide en 0..1²).
fn guided(guide: &[f32], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let ii: Vec<f32> = guide.par_iter().map(|v| v * v).collect();
    let ip: Vec<f32> = guide.par_iter().zip(p).map(|(a, b)| a * b).collect();
    let (mean_i, mean_p, corr_i, corr_ip) = (box_blur(guide, w, h, r), box_blur(p, w, h, r), box_blur(&ii, w, h, r), box_blur(&ip, w, h, r));
    let (a, b): (Vec<f32>, Vec<f32>) = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let var = corr_i[i] - mean_i[i] * mean_i[i];
            let cov = corr_ip[i] - mean_i[i] * mean_p[i];
            let a = cov / (var + eps);
            (a, mean_p[i] - a * mean_i[i])
        })
        .unzip();
    let (ma, mb) = (box_blur(&a, w, h, r), box_blur(&b, w, h, r));
    (0..w * h).into_par_iter().map(|i| (ma[i] * guide[i] + mb[i]).clamp(0.0, 1.0)).collect()
}

/// Calcule la carte finale (0 à 255) d'une image `w` × `h` (RGBA droit). `scale` : rapport à l'image entière (les
/// rayons et les pinceaux, donnés en pixels du document, sont mis à l'échelle). Renvoie aussi la couche mélangée
/// (aperçu « Couche »).
pub fn compute(rgba: &[u8], w: usize, h: usize, p: &Params, scale: f32) -> (Vec<u8>, Vec<u8>) {
    let base = base_map(rgba, p);
    let guide: Vec<f32> = match p.method {
        Method::Channel => mixed(rgba, p.mix).into_par_iter().map(|v| v / 255.0).collect(),
        _ => mixed(rgba, CHANNELS[3]).into_par_iter().map(|v| v / 255.0).collect(),
    };
    let channel_view: Vec<u8> = base.par_iter().map(|v| (v * 255.0).round() as u8).collect();
    let mut map = base;

    // Rayon intelligent : seulement dans la bande du contour (là où la carte n'est ni 0 ni 1 à cette distance).
    let radius = (p.radius * scale).round() as usize;
    if radius >= 1 {
        let refined = guided(&guide, &map, w, h, radius, 1e-3);
        let spread = box_blur(&map, w, h, radius);
        map.par_iter_mut().zip(refined.par_iter().zip(spread.par_iter())).for_each(|(m, (r, s))| {
            if *s > 0.002 && *s < 0.998 {
                *m = *r;
            }
        });
    }
    // Pinceau Affiner : grand rayon sous le pinceau, fondu sur son bord.
    let refine: Vec<&Dab> = p.dabs.iter().filter(|d| d.mode == StrokeMode::Refine).collect();
    if !refine.is_empty() {
        let big = ((refine.iter().map(|d| d.radius).fold(0.0f32, f32::max) * scale * 0.5).round() as usize).clamp(2, 64).max(radius * 2);
        let refined = guided(&guide, &map, w, h, big, 2e-3);
        let zone = coverage(&refine, w, h, scale);
        map.par_iter_mut().zip(refined.par_iter().zip(zone.par_iter())).for_each(|(m, (r, z))| *m += (*r - *m) * *z);
    }
    if p.smooth > 0.0 {
        let blurred = gaussian(&map, w, h, p.smooth * scale * 0.8);
        // Lisser sans adoucir : le flou arrondit le contour, la courbe remet le bord net.
        let k = 1.0 + p.smooth * 0.35;
        map = blurred.into_par_iter().map(|v| ((v - 0.5) * k + 0.5).clamp(0.0, 1.0)).collect();
    }
    if p.shift.abs() > 0.5 {
        let blurred = gaussian(&map, w, h, (2.0 * scale).max(0.8));
        let centre = 0.5 - p.shift.clamp(-100.0, 100.0) / 220.0;
        map = blurred.into_par_iter().map(|v| ((v - centre) * 2.2 + 0.5).clamp(0.0, 1.0)).collect();
    }
    if p.feather > 0.0 {
        map = gaussian(&map, w, h, p.feather * scale);
    }
    let add: Vec<&Dab> = p.dabs.iter().filter(|d| d.mode == StrokeMode::Add).collect();
    let remove: Vec<&Dab> = p.dabs.iter().filter(|d| d.mode == StrokeMode::Remove).collect();
    if !add.is_empty() {
        let zone = coverage(&add, w, h, scale);
        map.par_iter_mut().zip(zone.par_iter()).for_each(|(m, z)| *m = m.max(*z));
    }
    if !remove.is_empty() {
        let zone = coverage(&remove, w, h, scale);
        map.par_iter_mut().zip(zone.par_iter()).for_each(|(m, z)| *m = m.min(1.0 - *z));
    }
    (map.par_iter().map(|v| (v * 255.0).round() as u8).collect(), channel_view)
}

/// Couverture (0 à 1) de pinceaux ronds au bord adouci.
fn coverage(dabs: &[&Dab], w: usize, h: usize, scale: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    out.par_chunks_exact_mut(w).enumerate().for_each(|(y, row)| {
        let py = y as f32 + 0.5;
        for d in dabs {
            let (cx, cy, r) = (d.x * scale, d.y * scale, (d.radius * scale).max(0.5));
            if (py - cy).abs() > r {
                continue;
            }
            let x0 = ((cx - r).floor().max(0.0)) as usize;
            let x1 = ((cx + r).ceil() as usize).min(w);
            for (x, v) in row.iter_mut().enumerate().take(x1).skip(x0) {
                let d2 = ((x as f32 + 0.5 - cx).powi(2) + (py - cy).powi(2)).sqrt();
                let k = ((r - d2) / (r * 0.25).max(0.5)).clamp(0.0, 1.0);
                *v = v.max(k);
            }
        }
    });
    out
}

/// Décontamination : la couleur du sujet seul, estimée là où la carte n'est ni 0 ni 1 (RGBA droit, en place).
/// `amount` : 0 = couleurs d'origine, 1 = couleurs estimées.
pub fn decontaminate(rgba: &mut [u8], mask: &[u8], w: usize, h: usize, amount: f32, scale: f32) {
    let r = ((12.0 * scale).round() as usize).max(2);
    let alpha: Vec<f32> = mask.par_iter().map(|v| f32::from(*v) / 255.0).collect();
    let mut planes = Vec::new();
    for c in 0..3 {
        let channel: Vec<f32> = rgba.par_chunks_exact(4).map(|p| f32::from(p[c])).collect();
        // Couleur moyenne du fond (pondérée par 1 − α) et du sujet (pondérée par α), dans le voisinage.
        let bg_num: Vec<f32> = channel.par_iter().zip(&alpha).map(|(v, a)| v * (1.0 - a)).collect();
        let fg_num: Vec<f32> = channel.par_iter().zip(&alpha).map(|(v, a)| v * a).collect();
        planes.push((box_blur(&bg_num, w, h, r), box_blur(&fg_num, w, h, r)));
    }
    let inverse: Vec<f32> = alpha.par_iter().map(|a| 1.0 - a).collect();
    let (bg_den, fg_den) = (box_blur(&inverse, w, h, r), box_blur(&alpha, w, h, r));
    let amount = amount.clamp(0.0, 1.0);
    rgba.par_chunks_exact_mut(4).enumerate().for_each(|(i, px)| {
        let a = alpha[i];
        if a <= 0.004 || a >= 0.996 {
            return;
        }
        for c in 0..3 {
            let background = planes[c].0[i] / bg_den[i].max(1e-4);
            let neighbour = planes[c].1[i] / fg_den[i].max(1e-4);
            // I = αF + (1 − α)B ⇒ F = (I − (1 − α)B) / α, tenu près de la couleur du sujet voisin.
            let solved = ((f32::from(px[c]) - (1.0 - a) * background) / a).clamp(0.0, 255.0);
            let estimate = solved * a + neighbour * (1.0 - a);
            px[c] = (f32::from(px[c]) + (estimate - f32::from(px[c])) * amount).round().clamp(0.0, 255.0) as u8;
        }
    });
}

/// Carte d'octets `w` × `h` (repère document) en masque à tuiles ; le fond vaut 0.
pub fn mask_from_bytes(bytes: &[u8], w: u32, h: u32) -> MaskStore {
    let mut out = MaskStore::with_background([0]);
    let size = TILE_SIZE;
    for ty in 0..tiles_for(h) {
        for tx in 0..tiles_for(w) {
            let mut tile = TileBuf::<1>::filled([0]);
            let mut any = false;
            for py in 0..size {
                let y = ty * size + py;
                if y >= h {
                    break;
                }
                for px in 0..size {
                    let x = tx * size + px;
                    if x >= w {
                        break;
                    }
                    let v = bytes[(y * w + x) as usize];
                    if v != 0 {
                        any = true;
                        tile.set_pixel(px, py, [v]);
                    }
                }
            }
            if any {
                out.restore(TileCoord::new(tx as i32, ty as i32), Some(std::sync::Arc::new(tile)));
            }
        }
    }
    out
}

/// Image réduite pour l'aperçu : côté le plus long ≤ `max_side` (moyenne par boîte, RGBA droit).
pub fn downscale(rgba: &[u8], w: usize, h: usize, max_side: usize) -> (Vec<u8>, usize, usize, f32) {
    let k = (max_side as f32 / w.max(h) as f32).min(1.0);
    if k >= 1.0 {
        return (rgba.to_vec(), w, h, 1.0);
    }
    let (nw, nh) = (((w as f32 * k).round() as usize).max(1), ((h as f32 * k).round() as usize).max(1));
    let mut out = vec![0u8; nw * nh * 4];
    out.par_chunks_exact_mut(nw * 4).enumerate().for_each(|(y, row)| {
        let (y0, y1) = (y * h / nh, ((y + 1) * h / nh).max(y * h / nh + 1));
        for x in 0..nw {
            let (x0, x1) = (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1));
            let mut acc = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = &rgba[(sy * w + sx) * 4..(sy * w + sx) * 4 + 4];
                    for c in 0..4 {
                        acc[c] += u32::from(p[c]);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            for c in 0..4 {
                row[x * 4 + c] = (acc[c] / n.max(1)) as u8;
            }
        }
    });
    (out, nw, nh, nw as f32 / w as f32)
}

// ------------------------------------------------------------------ dans le document

/// Ce que produit la fenêtre.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Output {
    /// Une sélection.
    #[default]
    Selection,
    /// Le masque de fusion du calque (remplacé s'il en avait un).
    LayerMask,
    /// Un nouveau calque détouré (couleurs décontaminées sur demande) avec son masque, le calque d'origine masqué.
    NewLayer,
}

/// Image analysée : pixels RVBA, largeur, hauteur, et coin haut-gauche dans le document.
pub type Source = (Vec<u8>, u32, u32, (i32, i32));

/// Image analysée, en RGBA droit, et position (repère document) de son coin haut gauche. Un calque : son contenu
/// seul, recadré à son étendue (effets compris) ; un plan de travail, ou son fond géré : le cadre du plan, tout son
/// contenu visible. Sans calque : tout le document.
pub fn source(doc: &Document, layer: Option<LayerId>) -> Result<Source, EngineError> {
    use crate::layer_export::{isolate_at, ExportZone};
    let Some(id) = layer else {
        let flat = crate::io::flatten_for_export(doc, 1.0)?;
        return Ok((flat.rgba, flat.width, flat.height, (0, 0)));
    };
    let l = doc.layer(id)?;
    let board = if l.artboard.is_some() {
        Some(id)
    } else {
        l.parent.filter(|p| doc.layer(*p).is_ok_and(|g| g.artboard.as_ref().is_some_and(|b| b.background_layer == Some(id))))
    };
    let (isolated, origin) = match board {
        Some(b) => isolate_at(doc, &[b], ExportZone::Artboard, 0)?,
        None => isolate_at(doc, &[id], ExportZone::Content, 0)?,
    };
    let flat = crate::io::flatten_for_export(&isolated, 1.0)?;
    Ok((flat.rgba, flat.width, flat.height, origin))
}

/// Applique la sélection avancée au document (image entière, réglages tels quels).
pub fn apply(doc: &mut Document, layer: Option<LayerId>, params: &Params, output: Output) -> Result<(), EngineError> {
    let (mut rgba, w, h, (ox, oy)) = source(doc, layer)?;
    let (mask, _) = compute(&rgba, w as usize, h as usize, params, 1.0);
    // La carte est calculée sur la zone analysée : elle est replacée à sa position dans le document.
    let store = mask_from_bytes(&mask, w, h).translated(ox, oy);
    match output {
        Output::Selection => {
            doc.set_selection(Some(store));
        }
        Output::LayerMask => {
            let id = layer.ok_or_else(|| EngineError::InvalidOperation("choisissez un calque pour en faire le masque".into()))?;
            if doc.layer(id)?.mask.is_some() {
                doc.remove_mask(id)?;
            }
            doc.set_selection(Some(store));
            doc.add_mask(id, true)?;
        }
        Output::NewLayer => {
            if params.decontaminate {
                decontaminate(&mut rgba, &mask, w as usize, h as usize, params.amount, 1.0);
            }
            rgba.par_chunks_exact_mut(4).for_each(|px| {
                let p = crate::pixel::premultiply([px[0], px[1], px[2], px[3]]);
                px.copy_from_slice(&p);
            });
            let pixels = crate::tiles::TileStore::from_premultiplied(w, h, &rgba).ok_or_else(|| EngineError::InvalidOperation("image incomplète".into()))?;
            let (parent, index, name) = match layer {
                Some(id) => {
                    let l = doc.layer(id)?;
                    let parent = l.parent;
                    let index = doc.children_of(parent)?.iter().position(|c| *c == id).map_or(usize::MAX, |i| i + 1);
                    (parent, index, format!("{} détouré", l.props.name))
                }
                None => (None, usize::MAX, "Détourage".to_string()),
            };
            let created = doc.add_pixel_layer(name, pixels, parent, index)?;
            doc.translate_layers(&[created], ox, oy)?;
            doc.set_selection(Some(store));
            doc.add_mask(created, true)?;
            if let Some(id) = layer {
                doc.edit_props(id, |p| p.visible = false)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ciel bleu clair, « arbre » sombre au centre avec des branches fines d'un pixel.
    fn tree(w: usize, h: usize) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let trunk = (x as i32 - w as i32 / 2).abs() < 4 && y > h / 3;
                let crown = ((x as f32 - w as f32 / 2.0).powi(2) + (y as f32 - h as f32 / 3.0).powi(2)).sqrt() < w as f32 / 5.0;
                let twig = y == h / 3 && x > w / 8 && x < w * 7 / 8;
                rgba.extend_from_slice(if trunk || crown || twig { &[40, 70, 30, 255] } else { &[150, 190, 240, 255] });
            }
        }
        rgba
    }

    #[test]
    fn the_blue_channel_wins_and_the_twigs_survive() {
        let (w, h) = (120, 90);
        let rgba = tree(w, h);
        let scores: Vec<ChannelScore> = CHANNELS.iter().map(|mix| score(&mixed(&rgba, *mix), w, h)).collect();
        let best = scores.iter().enumerate().max_by(|a, b| a.1.score.total_cmp(&b.1.score)).map(|(i, _)| i).expect("couche");
        assert_eq!(best, 2, "le bleu sépare le mieux le ciel du feuillage : {scores:?}");
        assert!(scores[2].invert, "le ciel clair touche les bords : le sujet est sombre");
        let s = scores[2];
        let params = Params { mix: CHANNELS[2], black: s.black, white: s.white, invert: s.invert, radius: 2.0, ..Params::default() };
        let (mask, _) = compute(&rgba, w, h, &params, 1.0);
        assert!(mask[(h / 3) * w + w / 2] > 200, "la couronne est sélectionnée");
        assert!(mask[5 * w + 5] < 30, "le ciel ne l'est pas");
        assert!(mask[(h / 3) * w + w / 8 + 3] > 100, "la brindille d'un pixel est gardée");
        // Pinceau − : le fond est forcé.
        let erase = Params { dabs: vec![Dab { x: (w / 2) as f32, y: (h / 3) as f32, radius: 6.0, mode: StrokeMode::Remove }], ..params.clone() };
        assert!(compute(&rgba, w, h, &erase, 1.0).0[(h / 3) * w + w / 2] < 10);
    }

    #[test]
    fn decontamination_removes_the_sky_from_edges() {
        let (w, h) = (40, 20);
        // Moitié gauche verte (sujet), droite bleue (fond), une colonne de bord mélangée à 50 %.
        let mut rgba = Vec::new();
        let mut mask = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                let (px, m) = if x < 20 { ([0u8, 200, 0, 255], 255u8) } else if x == 20 { ([0, 100, 120, 255], 128) } else { ([0, 0, 240, 255], 0) };
                rgba.extend_from_slice(&px);
                mask.push(m);
            }
        }
        decontaminate(&mut rgba, &mask, w, h, 1.0, 1.0);
        let edge = &rgba[(10 * w + 20) * 4..(10 * w + 20) * 4 + 4];
        assert!(edge[2] < 60 && edge[1] > 150, "le bord a repris le vert du sujet : {edge:?}");
    }

    #[test]
    fn masks_round_trip_through_tiles() {
        let (w, h) = (300u32, 140u32);
        let bytes: Vec<u8> = (0..w * h).map(|i| (i % 251) as u8).collect();
        let store = mask_from_bytes(&bytes, w, h);
        assert_eq!(store.to_bytes(w, h), bytes);
    }
}
