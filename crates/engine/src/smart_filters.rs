//! Filtres dynamiques : les filtres d'un calque gardés en liste, non destructifs.
//!
//! Le calque garde ses pixels d'origine (`LayerFilters::source`) ; ses pixels affichés sont le résultat de la liste
//! appliquée du bas vers le haut (le premier filtre de la liste agit le premier), chacun avec son mode et son opacité
//! sur ce qu'il reçoit, puis limité par le masque de filtre. Tout ce qui lit les pixels d'un calque (composition,
//! carte graphique, vignettes, export) voit donc le résultat sans rien savoir des filtres ; les modifier, les masquer
//! ou les retirer recalcule ce résultat depuis l'origine.

use std::sync::Arc;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::blend::{BlendMode, blend_premultiplied};
use crate::document::Mask;
use crate::error::EngineError;
use crate::filters::{self, Filter};
use crate::tiles::{MaskStore, Sampler, Tile, TileCoord, TileStore, TILE_SIZE};

/// Un filtre de la liste.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartFilter {
    pub filter: Filter,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Options de fusion du filtre : comment son résultat se pose sur ce qu'il a reçu.
    #[serde(default)]
    pub blend: BlendMode,
    #[serde(default = "one")]
    pub opacity: f32,
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

impl SmartFilter {
    pub fn new(filter: Filter) -> Self {
        Self { filter, enabled: true, blend: BlendMode::Normal, opacity: 1.0 }
    }

    /// Le filtre se pose tel quel (Normal à 100 %) : son résultat remplace ce qu'il a reçu.
    fn plain(&self) -> bool {
        matches!(self.blend, BlendMode::Normal | BlendMode::PassThrough) && self.opacity >= 1.0
    }
}

/// Filtres dynamiques d'un calque de pixels.
#[derive(Clone, Debug)]
pub struct LayerFilters {
    pub list: Vec<SmartFilter>,
    /// Œil de « Filtres dynamiques » : éteint, le calque montre son origine.
    pub enabled: bool,
    /// Pixels d'origine, dans le repère du calque.
    pub source: Arc<TileStore>,
    /// Masque de filtre (repère du calque) : là où il est noir, l'origine reste visible.
    pub mask: Option<Mask>,
}

/// Les filtres actifs de la liste appliqués à `input`, du premier au dernier. `scale` : réglages mis à l'échelle d'un
/// aperçu réduit (1 pour le calque lui-même).
pub fn apply_list(input: &TileStore, list: &[SmartFilter], scale: f32) -> Result<TileStore, EngineError> {
    let mut current = input.clone();
    for item in list.iter().filter(|f| f.enabled) {
        let filter = if (scale - 1.0).abs() > f32::EPSILON { item.filter.scaled(scale) } else { item.filter.clone() };
        let out = filters::apply(&current, &filter)?;
        current = if item.plain() { out } else { mix(&current, &out, |before, after, _, _| fused(item, before, after)) };
    }
    Ok(current)
}

/// Pixels affichés d'un calque à filtres dynamiques.
pub fn render(filters: &LayerFilters) -> Result<TileStore, EngineError> {
    if !filters.enabled || filters.list.iter().all(|f| !f.enabled) {
        return Ok(TileStore::clone(&filters.source));
    }
    let filtered = apply_list(&filters.source, &filters.list, 1.0)?;
    match filters.mask.as_ref().filter(|m| m.enabled) {
        None => Ok(filtered),
        Some(mask) => Ok(mix_with_mask(&filters.source, &filtered, &mask.pixels)),
    }
}

/// Résultat d'un filtre posé sur son entrée avec son mode et son opacité (RGBA prémultiplié, octets).
fn fused(item: &SmartFilter, before: [u8; 4], after: [u8; 4]) -> [u8; 4] {
    let unit = |p: [u8; 4]| p.map(|v| f32::from(v) / 255.0);
    let (b, a) = (unit(before), unit(after));
    let mode = if item.blend == BlendMode::PassThrough { BlendMode::Normal } else { item.blend };
    // Normal : le résultat du filtre lui-même ; un autre mode : le résultat fondu dans l'entrée.
    let full = if mode == BlendMode::Normal { a } else { blend_premultiplied(mode, a, b) };
    let k = item.opacity.clamp(0.0, 1.0);
    [0, 1, 2, 3].map(|c| ((b[c] + (full[c] - b[c]) * k) * 255.0 + 0.5).clamp(0.0, 255.0) as u8)
}

/// L'origine et le résultat mêlés par le masque de filtre.
fn mix_with_mask(source: &TileStore, filtered: &TileStore, mask: &MaskStore) -> TileStore {
    let mut coords = source.sorted_coords();
    coords.extend(filtered.sorted_coords());
    coords.sort_unstable();
    coords.dedup();
    let tiles: Vec<(TileCoord, Option<Arc<Tile>>)> = coords
        .par_iter()
        .map(|&coord| {
            let (mut before, mut after, mut m) = (Sampler::new(source), Sampler::new(filtered), Sampler::new(mask));
            let (ox, oy) = coord.origin();
            let mut tile = Tile::transparent();
            for py in 0..TILE_SIZE {
                for px in 0..TILE_SIZE {
                    let (x, y) = (ox + px as i32, oy + py as i32);
                    let k = u16::from(m.get(x, y)[0]);
                    let (o, f) = (before.get(x, y), after.get(x, y));
                    let mixed = [0, 1, 2, 3].map(|c| ((u16::from(f[c]) * k + u16::from(o[c]) * (255 - k) + 127) / 255) as u8);
                    tile.set_pixel(px, py, mixed);
                }
            }
            (coord, (!tile.is_transparent()).then(|| Arc::new(tile)))
        })
        .collect();
    let mut out = TileStore::new();
    for (coord, tile) in tiles {
        out.restore(coord, tile);
    }
    out
}

/// Deux calques mêlés pixel par pixel, sur l'union de leurs tuiles.
fn mix(a: &TileStore, b: &TileStore, f: impl Fn([u8; 4], [u8; 4], i32, i32) -> [u8; 4] + Sync) -> TileStore {
    let mut coords = a.sorted_coords();
    coords.extend(b.sorted_coords());
    coords.sort_unstable();
    coords.dedup();
    let tiles: Vec<(TileCoord, Option<Arc<Tile>>)> = coords
        .par_iter()
        .map(|&coord| {
            let (mut sa, mut sb) = (Sampler::new(a), Sampler::new(b));
            let (ox, oy) = coord.origin();
            let mut tile = Tile::transparent();
            for py in 0..TILE_SIZE {
                for px in 0..TILE_SIZE {
                    let (x, y) = (ox + px as i32, oy + py as i32);
                    tile.set_pixel(px, py, f(sa.get(x, y), sb.get(x, y), x, y));
                }
            }
            (coord, (!tile.is_transparent()).then(|| Arc::new(tile)))
        })
        .collect();
    let mut out = TileStore::new();
    for (coord, tile) in tiles {
        out.restore(coord, tile);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::Sampler;

    fn square() -> TileStore {
        let mut rgba = vec![0u8; 64 * 64 * 4];
        for y in 16..48 {
            for x in 16..48 {
                rgba[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4].copy_from_slice(&[200, 40, 40, 255]);
            }
        }
        TileStore::from_premultiplied(64, 64, &rgba).expect("tampon")
    }

    #[test]
    fn filters_apply_in_order_and_respect_eye_opacity_and_mask() {
        let blur = SmartFilter::new(Filter::GaussianBlur { radius: 4.0 });
        let source = Arc::new(square());
        let mut filters = LayerFilters { list: vec![blur.clone()], enabled: true, source: Arc::clone(&source), mask: None };
        let blurred = render(&filters).expect("rendu");
        assert!(Sampler::new(&blurred).get(14, 32)[3] > 0, "le flou déborde");
        // Œil éteint : l'origine.
        filters.list[0].enabled = false;
        assert_eq!(Sampler::new(&render(&filters).expect("rendu")).get(14, 32)[3], 0);
        filters.list[0].enabled = true;
        filters.enabled = false;
        assert_eq!(Sampler::new(&render(&filters).expect("rendu")).get(14, 32)[3], 0);
        filters.enabled = true;
        // Opacité 50 % : à mi-chemin.
        filters.list[0].opacity = 0.5;
        let half = Sampler::new(&render(&filters).expect("rendu")).get(14, 32)[3];
        let full = Sampler::new(&blurred).get(14, 32)[3];
        assert!((i32::from(half) - i32::from(full) / 2).abs() <= 2, "{half} ≈ {full} / 2");
        filters.list[0].opacity = 1.0;
        // Masque noir : l'origine partout.
        filters.mask = Some(Mask { pixels: Arc::new(MaskStore::with_background([0])), enabled: true, linked: true });
        assert_eq!(Sampler::new(&render(&filters).expect("rendu")).get(14, 32)[3], 0);
    }
}
