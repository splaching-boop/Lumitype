//! Effets de calque (« fx ») : ombre portée, ombre interne, lueurs, contour, incrustation de couleur.
//!
//! Non destructifs : le calque garde ses pixels et la liste de ses effets. Le compositeur ne voit
//! que le résultat, un « calque stylé » — effets du dessous, pixels, effets du dessus — calculé une
//! fois puis gardé en cache tant que ni les pixels ni les effets ne changent.
//!
//! Simplification assumée par rapport à Photoshop : les effets sont posés en mode Normal à
//! l'intérieur du calque stylé, qui est ensuite fusionné d'un bloc avec le mode du calque. Une ombre
//! noire ou une lueur claire donnent le même résultat ; seuls des cas rares (ombre colorée en
//! Produit sur un fond vif) diffèrent.

use crate::blend::BlendMode;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::filters::gaussian;
use crate::tiles::TileStore;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokePosition {
    #[default]
    Outside,
    Inside,
    Center,
}

/// Un effet. `opacity` de 0 à 1 ; longueurs en pixels ; `angle` en degrés : direction d'où vient la
/// lumière (120° = en haut à gauche, l'ombre part donc en bas à droite).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LayerEffect {
    // `blend` : mode de fusion de l'effet (Normal pour les fichiers d'avant son apparition). Ombre portée et lueur
    // externe se fondent dans ce qui est SOUS le calque ; les autres, dans le calque lui-même (comme Photoshop).
    DropShadow { enabled: bool, color: [u8; 3], opacity: f32, angle: f32, distance: f32, size: f32, #[serde(default)] blend: BlendMode },
    InnerShadow { enabled: bool, color: [u8; 3], opacity: f32, angle: f32, distance: f32, size: f32, #[serde(default)] blend: BlendMode },
    OuterGlow { enabled: bool, color: [u8; 3], opacity: f32, size: f32, #[serde(default)] blend: BlendMode },
    InnerGlow { enabled: bool, color: [u8; 3], opacity: f32, size: f32, #[serde(default)] blend: BlendMode },
    Stroke { enabled: bool, color: [u8; 3], opacity: f32, width: f32, position: StrokePosition, #[serde(default)] blend: BlendMode },
    ColorOverlay { enabled: bool, color: [u8; 3], opacity: f32, #[serde(default)] blend: BlendMode },
    /// Incrustation de dégradé : de `from` à `to` à travers le contenu du calque, dans la direction
    /// `angle` (0° = vers la droite, 90° = vers le haut), ou du centre vers les bords si `radial`.
    GradientOverlay { enabled: bool, from: [u8; 3], to: [u8; 3], opacity: f32, angle: f32, radial: bool, #[serde(default)] blend: BlendMode },
}

impl LayerEffect {
    pub fn blend(&self) -> BlendMode {
        match *self {
            Self::DropShadow { blend, .. }
            | Self::InnerShadow { blend, .. }
            | Self::OuterGlow { blend, .. }
            | Self::InnerGlow { blend, .. }
            | Self::Stroke { blend, .. }
            | Self::ColorOverlay { blend, .. }
            | Self::GradientOverlay { blend, .. } => blend,
        }
    }

    /// Effet posé sous le calque, qui se fond dans ce qui est dessous avec un mode autre que Normal : il est
    /// rendu à part, et le compositeur le fond dans l'image (voir `Styled::under`).
    fn blends_into_backdrop(&self) -> bool {
        matches!(self, Self::DropShadow { .. } | Self::OuterGlow { .. }) && !matches!(self.blend(), BlendMode::Normal | BlendMode::PassThrough)
    }

    pub(crate) fn enabled(&self) -> bool {
        match *self {
            Self::DropShadow { enabled, .. } | Self::InnerShadow { enabled, .. } | Self::OuterGlow { enabled, .. } | Self::InnerGlow { enabled, .. } | Self::Stroke { enabled, .. } | Self::ColorOverlay { enabled, .. } | Self::GradientOverlay { enabled, .. } => enabled,
        }
    }

    /// Marge de calcul nécessaire autour du contenu du calque.
    pub(crate) fn reach(&self) -> f32 {
        match *self {
            // Les effets internes ne débordent pas, mais leur calcul a besoin de « voir » le vide autour du
            // calque sur la même distance : sans cette marge, le bord du tampon passerait pour de la matière.
            Self::DropShadow { distance, size, .. } | Self::InnerShadow { distance, size, .. } => distance.abs() + size * 1.6 + 2.0,
            Self::OuterGlow { size, .. } | Self::InnerGlow { size, .. } => size * 1.6 + 2.0,
            Self::Stroke { width, position, .. } if position != StrokePosition::Inside => width + 2.0,
            _ => 0.0,
        }
    }
}

/// Vrai si au moins un effet est actif : sinon le calque se compose tel quel, sans aucun surcoût.
pub fn any_enabled(effects: &[LayerEffect]) -> bool {
    effects.iter().any(LayerEffect::enabled)
}

const MAX_AREA: i64 = 80_000_000;

/// Flou d'une carte à un canal (on réutilise le flou RGBA du module des filtres).
fn blur(map: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma < 0.3 {
        return map.to_vec();
    }
    let packed: Vec<f32> = map.iter().flat_map(|v| [*v, 0.0, 0.0, 0.0]).collect();
    gaussian(&packed, w, h, sigma).chunks_exact(4).map(|px| px[0]).collect()
}

/// Carte décalée de `(dx, dy)` pixels ; ce qui entre par les bords vaut `outside`.
fn shifted(map: &[f32], w: usize, h: usize, dx: i32, dy: i32, outside: f32) -> Vec<f32> {
    let mut out = vec![outside; map.len()];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let sy = y as i32 - dy;
        if sy < 0 || sy >= h as i32 {
            return;
        }
        for (x, slot) in row.iter_mut().enumerate() {
            let sx = x as i32 - dx;
            if sx >= 0 && sx < w as i32 {
                *slot = map[sy as usize * w + sx as usize];
            }
        }
    });
    out
}

/// Distance euclidienne (en pixels) de chaque pixel au plus proche pixel « plein » (`inside` vrai),
/// par la transformée exacte en deux passes de Felzenszwalb et Huttenlocher.
pub(crate) fn distance_to(inside: &[bool], w: usize, h: usize) -> Vec<f32> {
    const FAR: f32 = 1e12;
    // Passe d'une ligne : enveloppe inférieure de paraboles.
    fn pass(f: &[f32], out: &mut [f32]) {
        let n = f.len();
        let (mut v, mut z) = (vec![0usize; n], vec![0.0f32; n + 1]);
        let mut k = 0;
        (z[0], z[1]) = (f32::NEG_INFINITY, f32::INFINITY);
        for q in 1..n {
            loop {
                let p = v[k];
                let s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * (q as f32 - p as f32));
                if s <= z[k] && k > 0 {
                    k -= 1;
                } else {
                    k += 1;
                    v[k] = q;
                    z[k] = s;
                    z[k + 1] = f32::INFINITY;
                    break;
                }
            }
        }
        k = 0;
        for (q, slot) in out.iter_mut().enumerate() {
            while z[k + 1] < q as f32 {
                k += 1;
            }
            let p = v[k];
            *slot = (q as f32 - p as f32).powi(2) + f[p];
        }
    }
    // Colonnes d'abord…
    let mut columns = vec![0.0f32; w * h];
    let seeds: Vec<f32> = inside.iter().map(|full| if *full { 0.0 } else { FAR }).collect();
    let by_column: Vec<Vec<f32>> = (0..w)
        .into_par_iter()
        .map(|x| {
            let f: Vec<f32> = (0..h).map(|y| seeds[y * w + x]).collect();
            let mut out = vec![0.0; h];
            pass(&f, &mut out);
            out
        })
        .collect();
    for (x, column) in by_column.iter().enumerate() {
        for (y, value) in column.iter().enumerate() {
            columns[y * w + x] = *value;
        }
    }
    // …puis lignes.
    let mut squared = vec![0.0f32; w * h];
    squared.par_chunks_mut(w).zip(columns.par_chunks(w)).for_each(|(out, row)| pass(row, out));
    squared.into_iter().map(f32::sqrt).collect()
}

/// Pose `color` avec la couverture `coverage` (0..1) sur un tampon prémultiplié 0..255 (« par-dessus »).
fn paint_over(target: &mut [f32], coverage: &[f32], color: [u8; 3], opacity: f32) {
    target.par_chunks_mut(4).zip(coverage.par_iter()).for_each(|(px, c)| {
        let a = (c * opacity).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        for i in 0..3 {
            px[i] = f32::from(color[i]) * a + px[i] * (1.0 - a);
        }
        px[3] = 255.0 * a + px[3] * (1.0 - a);
    });
}

/// Pose `color` avec la couverture `coverage` sur un tampon prémultiplié 0..255, selon le mode de fusion `mode`
/// (contre ce que le tampon contient déjà : le calque et ses effets posés avant).
fn paint_blend(target: &mut [f32], coverage: &[f32], color: [u8; 3], opacity: f32, mode: BlendMode) {
    if matches!(mode, BlendMode::Normal | BlendMode::PassThrough) {
        return paint_over(target, coverage, color, opacity);
    }
    target.par_chunks_mut(4).zip(coverage.par_iter()).for_each(|(px, c)| {
        let a = (c * opacity).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let src = [f32::from(color[0]) / 255.0 * a, f32::from(color[1]) / 255.0 * a, f32::from(color[2]) / 255.0 * a, a];
        let dst = [px[0] / 255.0, px[1] / 255.0, px[2] / 255.0, px[3] / 255.0];
        let out = crate::blend::blend_premultiplied(mode, src, dst);
        for i in 0..4 {
            px[i] = out[i] * 255.0;
        }
    });
}

/// Liste des effets rendus à part, si cet effet doit l'être : mode autre que Normal, et rendu en parties.
fn apart<'a>(blend: BlendMode, over: &'a mut Option<&mut Vec<(BlendMode, TileStore)>>) -> Option<&'a mut Vec<(BlendMode, TileStore)>> {
    if matches!(blend, BlendMode::Normal | BlendMode::PassThrough) {
        return None;
    }
    over.as_deref_mut()
}

/// Pose un effet du dessus : dans le calque en mode Normal ; seul, dans son propre tampon, dans un autre mode.
fn place(
    out: &mut [f32],
    over: &mut Option<&mut Vec<(BlendMode, TileStore)>>,
    coverage: &[f32],
    color: [u8; 3],
    opacity: f32,
    blend: BlendMode,
    to_store: &dyn Fn(&[f32]) -> Result<TileStore, EngineError>,
) -> Result<(), EngineError> {
    match apart(blend, over) {
        Some(list) => {
            let mut alone = vec![0.0f32; out.len()];
            paint_over(&mut alone, coverage, color, opacity);
            list.push((blend, to_store(&alone)?));
        }
        None => paint_blend(out, coverage, color, opacity, blend),
    }
    Ok(())
}

/// Calque stylé : le calque et ses effets (`main`), plus à part les effets du dessous qui se fondent dans l'image
/// sous le calque avec un mode autre que Normal (`under`, de bas en haut), en repère local.
pub struct Styled {
    pub main: TileStore,
    pub under: Vec<(BlendMode, TileStore)>,
    /// Effets du dessus (contour, incrustations, lueur et ombre internes) dans un mode autre que Normal : rendus
    /// seuls, ils se fondent dans TOUT ce qui est dessous (le calque et l'image), là où ils sont dessinés.
    pub over: Vec<(BlendMode, TileStore)>,
}

/// Calque stylé en un seul tampon (effets du dessous inclus en mode Normal) : aperçus, tests.
pub fn styled(pixels: &TileStore, effects: &[LayerEffect]) -> Result<TileStore, EngineError> {
    let mut under = Vec::new();
    styled_into(pixels, effects, &mut under, None)
}

/// Calque stylé : pixels du calque entourés de ses effets, en repère local.
pub fn styled_parts(pixels: &TileStore, effects: &[LayerEffect]) -> Result<Styled, EngineError> {
    let (mut under, mut over) = (Vec::new(), Vec::new());
    let main = styled_into(pixels, effects, &mut under, Some(&mut over))?;
    Ok(Styled { main, under, over })
}

/// `over` : `None` = tous les effets du dessus fondus dans le calque (rendu en un seul tampon).
fn styled_into(pixels: &TileStore, effects: &[LayerEffect], under: &mut Vec<(BlendMode, TileStore)>, mut over: Option<&mut Vec<(BlendMode, TileStore)>>) -> Result<TileStore, EngineError> {
    let Some((x0, y0, x1, y1)) = pixels.pixel_bounds(|px| px[3] > 0) else { return Ok(TileStore::new()) };
    let margin = effects.iter().filter(|e| e.enabled()).map(LayerEffect::reach).fold(0.0f32, f32::max).ceil() as i32 + 1;
    let (left, top) = (x0 - margin, y0 - margin);
    let (w, h) = ((x1 - x0 + 2 * margin) as usize, (y1 - y0 + 2 * margin) as usize);
    if (w as i64) * (h as i64) > MAX_AREA {
        return Err(EngineError::InvalidOperation("calque trop grand pour des effets de calque".into()));
    }
    let source: Vec<f32> = pixels.translated(-left, -top).to_premultiplied(w as u32, h as u32).iter().map(|v| f32::from(*v)).collect();
    let alpha: Vec<f32> = source.chunks_exact(4).map(|px| px[3] / 255.0).collect();
    // Direction de l'ombre : à l'opposé de la lumière (l'axe vertical de l'écran pointe vers le bas).
    let offset = |angle: f32, distance: f32| ((-angle.to_radians().cos() * distance).round() as i32, (angle.to_radians().sin() * distance).round() as i32);
    // Distances au bord, calculées seulement si un contour le demande.
    let needs_distance = effects.iter().any(|e| matches!(e, LayerEffect::Stroke { enabled: true, .. }));
    let solid: Vec<bool> = alpha.iter().map(|a| *a >= 0.5).collect();
    let (outside, inside) = if needs_distance { (distance_to(&solid, w, h), distance_to(&solid.iter().map(|s| !s).collect::<Vec<_>>(), w, h)) } else { (Vec::new(), Vec::new()) };

    // Dessous du calque : ombre portée, lueur externe. En mode Normal, elles vont dans le tampon du calque ; dans un
    // autre mode, chacune est rendue dans son propre tampon, que le compositeur fondra dans l'image dessous.
    let to_store = |buffer: &[f32]| -> Result<TileStore, EngineError> {
        let bytes: Vec<u8> = buffer
            .chunks_exact(4)
            .flat_map(|px| {
                let a = px[3].clamp(0.0, 255.0);
                [px[0].clamp(0.0, a), px[1].clamp(0.0, a), px[2].clamp(0.0, a), a].map(|v| (v + 0.5) as u8)
            })
            .collect();
        let store = TileStore::from_premultiplied(w as u32, h as u32, &bytes).ok_or_else(|| EngineError::Corrupt("tampon d'effets de taille inattendue".into()))?;
        Ok(store.translated(left, top))
    };
    let mut out = vec![0.0f32; source.len()];
    for effect in effects.iter().filter(|e| e.enabled()) {
        let (coverage, color, opacity) = match *effect {
            LayerEffect::DropShadow { color, opacity, angle, distance, size, .. } => {
                let (dx, dy) = offset(angle, distance);
                (blur(&shifted(&alpha, w, h, dx, dy, 0.0), w, h, size / 2.0), color, opacity)
            }
            // Halo renforcé près du bord, comme une source de lumière derrière le calque.
            LayerEffect::OuterGlow { color, opacity, size, .. } => (blur(&alpha, w, h, size / 2.0).iter().map(|v| (v * 1.8).min(1.0)).collect(), color, opacity),
            _ => continue,
        };
        if effect.blends_into_backdrop() {
            let mut alone = vec![0.0f32; source.len()];
            paint_over(&mut alone, &coverage, color, opacity);
            under.push((effect.blend(), to_store(&alone)?));
        } else {
            paint_over(&mut out, &coverage, color, opacity);
        }
    }
    // Le calque lui-même.
    out.par_chunks_mut(4).zip(source.par_chunks(4)).for_each(|(px, layer)| {
        let keep = 1.0 - layer[3] / 255.0;
        for i in 0..4 {
            px[i] = layer[i] + px[i] * keep;
        }
    });
    // Dessus du calque, limités à sa forme, dans l'ordre de Photoshop quel que soit celui de la liste :
    // incrustation de couleur, puis lueur interne, puis ombre interne ; le contour vient en dernier.
    let empty: Vec<f32> = alpha.iter().map(|a| 1.0 - a).collect();
    for pass in 0..3 {
        for effect in effects.iter().filter(|e| e.enabled()) {
            match (pass, *effect) {
                (0, LayerEffect::ColorOverlay { color, opacity, blend, .. }) => {
                    if let Some(list) = apart(blend, &mut over) {
                        let mut alone = vec![0.0f32; source.len()];
                        paint_over(&mut alone, &alpha, color, opacity);
                        list.push((blend, to_store(&alone)?));
                    } else {
                        paint_blend(&mut out, &alpha, color, opacity, blend);
                    }
                }
                (0, LayerEffect::GradientOverlay { from, to, opacity, angle, radial, blend, .. }) => {
                    let separate = apart(blend, &mut over).is_some();
                    let mut alone = if separate { vec![0.0f32; source.len()] } else { Vec::new() };
                    let target: &mut Vec<f32> = if separate { &mut alone } else { &mut out };
                    let blend = if separate { BlendMode::Normal } else { blend };
                    // Le dégradé couvre exactement le contenu du calque (sans la marge de calcul).
                    let (cw, ch) = ((x1 - x0) as f32, (y1 - y0) as f32);
                    let (cx, cy) = (margin as f32 + cw / 2.0, margin as f32 + ch / 2.0);
                    let (dx, dy) = (angle.to_radians().cos(), -angle.to_radians().sin());
                    let half = (dx.abs() * cw + dy.abs() * ch) / 2.0;
                    target.par_chunks_mut(4).enumerate().for_each(|(i, px)| {
                        let a = (alpha[i] * opacity).clamp(0.0, 1.0);
                        if a <= 0.0 {
                            return;
                        }
                        let (x, y) = ((i % w) as f32 + 0.5 - cx, (i / w) as f32 + 0.5 - cy);
                        let t = if radial { (x / (cw / 2.0)).hypot(y / (ch / 2.0)) } else { (x * dx + y * dy) / half.max(1.0) * 0.5 + 0.5 }.clamp(0.0, 1.0);
                        let colour: [f32; 3] = std::array::from_fn(|c| f32::from(from[c]) + (f32::from(to[c]) - f32::from(from[c])) * t);
                        let src = [colour[0] / 255.0 * a, colour[1] / 255.0 * a, colour[2] / 255.0 * a, a];
                        let dst = [px[0] / 255.0, px[1] / 255.0, px[2] / 255.0, px[3] / 255.0];
                        let mode = if blend == BlendMode::PassThrough { BlendMode::Normal } else { blend };
                        let mixed = crate::blend::blend_premultiplied(mode, src, dst);
                        for i in 0..4 {
                            px[i] = mixed[i] * 255.0;
                        }
                    });
                    if separate {
                        let store = to_store(&alone)?;
                        if let (LayerEffect::GradientOverlay { blend, .. }, Some(list)) = (*effect, over.as_deref_mut()) {
                            list.push((blend, store));
                        }
                    }
                }
                (1, LayerEffect::InnerGlow { color, opacity, size, blend, .. }) => {
                    let glow: Vec<f32> = blur(&empty, w, h, size / 2.0).iter().zip(&alpha).map(|(g, a)| (g * 1.8).min(1.0) * a).collect();
                    place(&mut out, &mut over, &glow, color, opacity, blend, &to_store)?;
                }
                (2, LayerEffect::InnerShadow { color, opacity, angle, distance, size, blend, .. }) => {
                    let (dx, dy) = offset(angle, distance);
                    let shadow: Vec<f32> = blur(&shifted(&empty, w, h, dx, dy, 1.0), w, h, size / 2.0).iter().zip(&alpha).map(|(s, a)| s * a).collect();
                    place(&mut out, &mut over, &shadow, color, opacity, blend, &to_store)?;
                }
                _ => {}
            }
        }
    }
    // Plusieurs contours : le premier de la liste est dessiné en dernier, donc au-dessus (comme dans
    // Photoshop). Pour un contour double, on liste donc le plus fin d'abord, le plus large ensuite.
    for effect in effects.iter().rev().filter(|e| e.enabled()) {
        if let LayerEffect::Stroke { color, opacity, width, position, blend, .. } = *effect {
            let (out_w, in_w) = match position {
                StrokePosition::Outside => (width, 0.0),
                StrokePosition::Inside => (0.0, width),
                StrokePosition::Center => (width / 2.0, width / 2.0),
            };
            // Distance signée au bord : négative dedans. Un pixel d'antialiasing de chaque côté.
            let band: Vec<f32> = (0..w * h)
                .into_par_iter()
                .map(|i| {
                    let d = if solid[i] { 0.5 - inside[i] } else { outside[i] - 0.5 };
                    ((out_w - d + 0.5).clamp(0.0, 1.0)).min((d + in_w + 0.5).clamp(0.0, 1.0))
                })
                .collect();
            place(&mut out, &mut over, &band, color, opacity, blend, &to_store)?;
        }
    }

    to_store(&out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::Sampler;

    /// Carré rouge de 100 px, de (100, 100) à (200, 200).
    fn square() -> TileStore {
        let mut store = TileStore::new();
        for y in 100..200 {
            for x in 100..200 {
                let (coord, px, py) = crate::tiles::TileCoord::of_pixel(x, y);
                store.get_mut(coord).set_pixel(px, py, [255, 0, 0, 255]);
            }
        }
        store
    }

    fn at(store: &TileStore, x: i32, y: i32) -> [u8; 4] {
        Sampler::new(store).get(x, y)
    }

    #[test]
    fn no_enabled_effect_means_no_work() {
        let off = [LayerEffect::OuterGlow { enabled: false, color: [255; 3], opacity: 1.0, size: 20.0, blend: crate::blend::BlendMode::Normal }];
        assert!(!any_enabled(&off) && !any_enabled(&[]));
        let styled = styled(&square(), &off).expect("rendu");
        assert_eq!((at(&styled, 150, 150), at(&styled, 90, 150)), ([255, 0, 0, 255], [0; 4]));
    }

    #[test]
    fn drop_shadow_falls_away_from_the_light_and_stays_under_the_layer() {
        // Lumière en haut à gauche (135°) : l'ombre part en bas à droite.
        let shadow = [LayerEffect::DropShadow { enabled: true, color: [0, 0, 0], opacity: 1.0, angle: 135.0, distance: 20.0, size: 0.0, blend: crate::blend::BlendMode::Normal }];
        let styled = styled(&square(), &shadow).expect("rendu");
        assert_eq!(at(&styled, 150, 150), [255, 0, 0, 255], "le calque recouvre son ombre");
        assert_eq!(at(&styled, 208, 208), [0, 0, 0, 255], "ombre visible en bas à droite");
        assert_eq!(at(&styled, 92, 92), [0; 4], "rien en haut à gauche");
        let soft = [LayerEffect::DropShadow { enabled: true, color: [0, 0, 0], opacity: 0.5, angle: 135.0, distance: 20.0, size: 16.0, blend: crate::blend::BlendMode::Normal }];
        let edge = at(&styled2(&soft), 214, 160)[3];
        assert!((40..=90).contains(&edge), "bord flouté, à demi-opacité : {edge}");
    }

    fn styled2(effects: &[LayerEffect]) -> TileStore {
        styled(&square(), effects).expect("rendu")
    }

    #[test]
    fn strokes_follow_the_edge_at_the_requested_position() {
        let stroke = |position| styled2(&[LayerEffect::Stroke { enabled: true, color: [0, 0, 255], opacity: 1.0, width: 10.0, position, blend: crate::blend::BlendMode::Normal }]);
        let outside = stroke(StrokePosition::Outside);
        assert_eq!((at(&outside, 95, 150), at(&outside, 105, 150), at(&outside, 88, 150)[3]), ([0, 0, 255, 255], [255, 0, 0, 255], 0));
        // Angles arrondis : à 10 px en diagonale du coin, on est hors du contour (il faudrait 14 px pour un angle droit).
        assert_eq!(at(&outside, 91, 91)[3], 0, "angle arrondi, pas carré");
        let inside = stroke(StrokePosition::Inside);
        assert_eq!((at(&inside, 105, 150), at(&inside, 115, 150), at(&inside, 95, 150)[3]), ([0, 0, 255, 255], [255, 0, 0, 255], 0));
        let center = stroke(StrokePosition::Center);
        assert_eq!((at(&center, 97, 150), at(&center, 103, 150), at(&center, 108, 150)), ([0, 0, 255, 255], [0, 0, 255, 255], [255, 0, 0, 255]));
    }

    #[test]
    fn inner_effects_never_leave_the_shape_and_glows_surround_it() {
        let inner = styled2(&[
            LayerEffect::InnerShadow { enabled: true, color: [0, 0, 0], opacity: 1.0, angle: 135.0, distance: 12.0, size: 0.0, blend: crate::blend::BlendMode::Normal },
            LayerEffect::InnerGlow { enabled: true, color: [255, 255, 0], opacity: 1.0, size: 12.0, blend: crate::blend::BlendMode::Normal },
        ]);
        assert_eq!(inner.pixel_bounds(|px| px[3] > 0), Some((100, 100, 200, 200)), "rien ne déborde");
        assert_eq!(at(&inner, 105, 105), [0, 0, 0, 255], "ombre interne contre les bords éclairés (haut et gauche)");
        assert_eq!(at(&inner, 150, 150), [255, 0, 0, 255], "le centre garde sa couleur");
        assert!(at(&inner, 197, 150)[1] > 80, "lueur interne jaune le long du bord opposé : {:?}", at(&inner, 197, 150));

        let glow = styled2(&[LayerEffect::OuterGlow { enabled: true, color: [0, 255, 255], opacity: 1.0, size: 24.0, blend: crate::blend::BlendMode::Normal }]);
        let (near, far) = (at(&glow, 96, 150), at(&glow, 80, 150));
        assert!(near[3] > far[3] && far[3] > 0 && near[0] == 0 && near[1] > 0, "halo cyan qui s'éteint en s'éloignant : {near:?} puis {far:?}");
        assert_eq!(at(&glow, 150, 150), [255, 0, 0, 255]);

        let overlay = styled2(&[LayerEffect::ColorOverlay { enabled: true, color: [0, 255, 0], opacity: 1.0, blend: crate::blend::BlendMode::Normal }]);
        assert_eq!((at(&overlay, 150, 150), at(&overlay, 90, 150)[3]), ([0, 255, 0, 255], 0));
    }

    #[test]
    fn effects_stack_several_strokes_and_a_gradient_overlay() {
        // Double contour : 4 px blanc listé d'abord (au-dessus), 10 px noir ensuite (dessous, il dépasse).
        let stroke = |color, width| LayerEffect::Stroke { enabled: true, color, opacity: 1.0, width, position: StrokePosition::Outside, blend: crate::blend::BlendMode::Normal };
        let double = styled2(&[stroke([255, 255, 255], 4.0), stroke([0, 0, 0], 10.0)]);
        assert_eq!((at(&double, 98, 150), at(&double, 93, 150), at(&double, 88, 150)[3]), ([255, 255, 255, 255], [0, 0, 0, 255], 0));

        let gradient = styled2(&[LayerEffect::GradientOverlay { enabled: true, from: [0, 0, 255], to: [255, 255, 0], opacity: 1.0, angle: 0.0, radial: false, blend: crate::blend::BlendMode::Normal }]);
        let (left, right) = (at(&gradient, 102, 150), at(&gradient, 197, 150));
        assert!(left[2] > 240 && left[0] < 15 && right[0] > 240 && right[2] < 15, "bleu à gauche, jaune à droite : {left:?} {right:?}");
        assert_eq!(gradient.pixel_bounds(|px| px[3] > 0), Some((100, 100, 200, 200)), "limité à la forme du calque");
        let up = styled2(&[LayerEffect::GradientOverlay { enabled: true, from: [0, 0, 0], to: [255, 255, 255], opacity: 1.0, angle: 90.0, radial: false, blend: crate::blend::BlendMode::Normal }]);
        assert!(at(&up, 150, 103)[0] > 240 && at(&up, 150, 197)[0] < 15, "90° : vers le haut");
    }

    #[test]
    fn exact_distance_transform() {
        // Un seul pixel plein au centre d'une grille 9 x 9.
        let mut grid = vec![false; 81];
        grid[4 * 9 + 4] = true;
        let d = distance_to(&grid, 9, 9);
        assert_eq!(d[4 * 9 + 4], 0.0);
        assert!((d[4 * 9 + 7] - 3.0).abs() < 1e-4 && (d[0] - 32.0f32.sqrt()).abs() < 1e-4 && (d[7 * 9 + 8] - 5.0).abs() < 1e-4);
    }
}
