//! Sélections : un masque 8 bits de la taille du document (0 = hors sélection,
//! 255 = sélectionné, valeurs intermédiaires = bords lissés ou contour progressif).
//!
//! La sélection fait partie du [`Document`](crate::Document) : elle est donc
//! annulable comme le reste, sans mécanisme particulier.

use std::collections::HashSet;

use serde::Deserialize;

use crate::pixel::{mul_div_255, unpremultiply};
use crate::tiles::{MaskStore, MaskTile, Sampler, TileCoord, TileStore, TILE_SIZE};

/// Combinaison avec la sélection existante (Maj = ajouter, Alt = soustraire, Maj+Alt = intersection).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CombineMode {
    Replace,
    Add,
    Subtract,
    Intersect,
}

/// Forme à sélectionner, en pixels du document.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    Rect { x: f32, y: f32, width: f32, height: f32 },
    Ellipse { x: f32, y: f32, width: f32, height: f32 },
    /// Lasso libre ou polygonal ; le contour est fermé automatiquement.
    Polygon { points: Vec<[f32; 2]> },
    /// Plusieurs contours (tracé de la plume) : règle pair-impair, un contour dans un autre fait un trou.
    Contours { contours: Vec<Vec<[f32; 2]>> },
}

/// Remplit un masque ligne par ligne : `row(y, tampon)` écrit la couverture `0..=1`
/// des pixels `x0..x1` de la ligne `y`.
fn rasterize_rows(
    (x0, y0, x1, y1): (u32, u32, u32, u32),
    mut row: impl FnMut(u32, &mut [f32]),
) -> MaskStore {
    let mut store = MaskStore::with_background([0]);
    if x0 >= x1 || y0 >= y1 {
        return store;
    }
    let mut coverage = vec![0.0f32; (x1 - x0) as usize];
    for y in y0..y1 {
        coverage.fill(0.0);
        row(y, &mut coverage);
        for tx in x0 / TILE_SIZE..=(x1 - 1) / TILE_SIZE {
            let (from, to) = ((tx * TILE_SIZE).max(x0), ((tx + 1) * TILE_SIZE).min(x1));
            let values = &coverage[(from - x0) as usize..(to - x0) as usize];
            if values.iter().all(|v| *v <= 0.0) {
                continue;
            }
            let tile = store.get_mut(TileCoord::new(tx as i32, (y / TILE_SIZE) as i32));
            for (x, value) in (from..to).zip(values) {
                tile.set_pixel(x % TILE_SIZE, y % TILE_SIZE, [(value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8]);
            }
        }
    }
    store
}

/// Boîte englobante entière d'un rectangle flottant, bornée au document.
fn clamp_box(x: f32, y: f32, w: f32, h: f32, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let (x, y, w, h) = (x.min(x + w), y.min(y + h), w.abs(), h.abs());
    let lo = |v: f32, max: u32| (v.floor().max(0.0) as u32).min(max);
    let hi = |v: f32, max: u32| (v.ceil().max(0.0) as u32).min(max);
    (lo(x, width), lo(y, height), hi(x + w, width), hi(y + h, height))
}

impl Shape {
    /// Masque de la forme seule. Le rectangle est aligné sur les pixels (comme dans
    /// Photoshop) ; ellipse et polygone ont des bords lissés.
    pub fn rasterize(&self, width: u32, height: u32) -> MaskStore {
        match self {
            Self::Rect { x, y, width: w, height: h } => {
                let (x, y, w, h) = (x.min(x + w).round(), y.min(y + h).round(), w.abs().round(), h.abs().round());
                rasterize_rows(clamp_box(x, y, w, h, width, height), |_, row| row.fill(1.0))
            }
            Self::Ellipse { x, y, width: w, height: h } => {
                let bounds = clamp_box(*x, *y, *w, *h, width, height);
                let (rx, ry) = (w.abs() / 2.0, h.abs() / 2.0);
                let (cx, cy) = (x.min(x + w) + rx, y.min(y + h) + ry);
                if rx < 0.5 || ry < 0.5 {
                    return MaskStore::with_background([0]);
                }
                rasterize_rows(bounds, |py, row| {
                    let dy = py as f32 + 0.5 - cy;
                    for (i, value) in row.iter_mut().enumerate() {
                        let dx = (bounds.0 + i as u32) as f32 + 0.5 - cx;
                        // Distance signée au premier ordre : (k - 1) / |grad k|, k = équation implicite de l'ellipse.
                        let k = (dx / rx).powi(2) + (dy / ry).powi(2);
                        let gradient = 2.0 * ((dx / (rx * rx)).powi(2) + (dy / (ry * ry)).powi(2)).sqrt();
                        let distance = if gradient > 0.0 { (k - 1.0) / gradient } else { -rx.min(ry) };
                        *value = 0.5 - distance;
                    }
                })
            }
            Self::Polygon { points } => rasterize_polygon(std::slice::from_ref(points), width, height),
            Self::Contours { contours } => rasterize_polygon(contours, width, height),
        }
    }
}

/// Remplissage pair-impair par lignes de balayage, 4 sous-lignes par pixel et
/// couverture horizontale exacte : bords lissés sans suréchantillonner toute la forme.
fn rasterize_polygon(contours: &[Vec<[f32; 2]>], width: u32, height: u32) -> MaskStore {
    const SUBLINES: u32 = 4;
    let contours: Vec<&Vec<[f32; 2]>> = contours.iter().filter(|c| c.len() >= 3).collect();
    if contours.is_empty() {
        return MaskStore::with_background([0]);
    }
    let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in contours.iter().flat_map(|c| c.iter()) {
        for axis in 0..2 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    let bounds = clamp_box(min[0], min[1], max[0] - min[0], max[1] - min[1], width, height);
    let mut crossings: Vec<f32> = Vec::new();

    rasterize_rows(bounds, |py, row| {
        for sub in 0..SUBLINES {
            let y = py as f32 + (sub as f32 + 0.5) / SUBLINES as f32;
            crossings.clear();
            for points in &contours {
                for (i, a) in points.iter().enumerate() {
                    let b = points[(i + 1) % points.len()];
                    if (a[1] <= y) != (b[1] <= y) {
                        crossings.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                    }
                }
            }
            crossings.sort_unstable_by(f32::total_cmp);
            for span in crossings.chunks_exact(2) {
                let (start, end) = (span[0] - bounds.0 as f32, span[1] - bounds.0 as f32);
                let first = start.floor().max(0.0) as usize;
                let last = (end.ceil().max(0.0) as usize).min(row.len());
                for (x, value) in row.iter_mut().enumerate().take(last).skip(first) {
                    let covered = (end.min(x as f32 + 1.0) - start.max(x as f32)).max(0.0);
                    *value += covered / SUBLINES as f32;
                }
            }
        }
    })
}

/// Applique `op(existant, nouveau)` tuile par tuile, fonds compris.
fn zip_with(a: &MaskStore, b: &MaskStore, op: impl Fn(u8, u8) -> u8) -> MaskStore {
    let mut out = MaskStore::with_background([op(a.background()[0], b.background()[0])]);
    let coords: HashSet<TileCoord> = a.iter().chain(b.iter()).map(|(coord, _)| coord).collect();
    for coord in coords {
        let mut tile = MaskTile::filled(a.background());
        if let Some(existing) = a.get(coord) {
            tile = (**existing).clone();
        }
        match b.get(coord) {
            Some(other) => {
                for (value, new) in tile.bytes_mut().iter_mut().zip(other.bytes()) {
                    *value = op(*value, *new);
                }
            }
            None => {
                let new = b.background()[0];
                for value in tile.bytes_mut() {
                    *value = op(*value, new);
                }
            }
        }
        out.insert(coord, tile);
    }
    out
}

fn is_empty(mask: &MaskStore) -> bool {
    mask.background() == [0] && mask.tile_count() == 0
}

/// Combine une nouvelle forme avec la sélection existante. `None` = plus rien de sélectionné.
pub fn combine(existing: Option<&MaskStore>, new: MaskStore, mode: CombineMode) -> Option<MaskStore> {
    let result = match (existing, mode) {
        (None, CombineMode::Subtract | CombineMode::Intersect) => return None,
        (None, _) | (_, CombineMode::Replace) => new,
        (Some(old), CombineMode::Add) => zip_with(old, &new, u8::max),
        (Some(old), CombineMode::Subtract) => zip_with(old, &new, |a, b| mul_div_255(a, 255 - b)),
        (Some(old), CombineMode::Intersect) => zip_with(old, &new, u8::min),
    };
    (!is_empty(&result)).then_some(result)
}

pub fn select_all() -> MaskStore {
    MaskStore::with_background([255])
}

/// Ctrl+Maj+I. Sans sélection, tout est sélectionné.
pub fn invert(existing: Option<&MaskStore>) -> Option<MaskStore> {
    let Some(mask) = existing else { return Some(select_all()) };
    let inverted = zip_with(mask, &MaskStore::with_background([0]), |a, _| 255 - a);
    (!is_empty(&inverted)).then_some(inverted)
}

/// Contour progressif : flou gaussien approché par trois flous en boîte successifs.
/// `width`, `height` : taille du document, hors de laquelle la sélection n'a pas de sens.
pub fn feather(mask: &MaskStore, radius: f32, width: u32, height: u32) -> MaskStore {
    let reach = (radius * 3.0).ceil().max(1.0) as i32;
    let (dw, dh) = (width as i32, height as i32);
    // Zone de travail : les tuiles stockées, élargies de la portée du flou. Un fond
    // blanc (sélection inversée) oblige à traiter tout le document.
    let area = if mask.background() == [255] {
        Some((0, 0, dw, dh))
    } else {
        mask.bounds().map(|(x0, y0, x1, y1)| ((x0 - reach).max(0), (y0 - reach).max(0), (x1 + reach).min(dw), (y1 + reach).min(dh)))
    };
    let Some((x0, y0, x1, y1)) = area.filter(|(x0, y0, x1, y1)| x0 < x1 && y0 < y1) else { return mask.clone() };
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut sampler = Sampler::new(mask);
    let mut buffer: Vec<u8> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| sampler.get(x, y)[0]).collect();

    // Trois passes d'une boîte de rayon r ≈ sigma : approximation classique d'une gaussienne.
    let box_radius = (radius.max(0.5)).round() as usize;
    let mut scratch = vec![0u8; w * h];
    for _ in 0..3 {
        box_blur(&buffer, &mut scratch, w, h, box_radius, true);
        box_blur(&scratch, &mut buffer, w, h, box_radius, false);
    }

    let mut out = MaskStore::with_background(mask.background());
    let (first, _, _) = TileCoord::of_pixel(x0, y0);
    let (last, _, _) = TileCoord::of_pixel(x1 - 1, y1 - 1);
    let size = TILE_SIZE as i32;
    for ty in first.y..=last.y {
        for tx in first.x..=last.x {
            let mut tile = MaskTile::filled(mask.background());
            let (ox, oy) = (tx * size, ty * size);
            for y in oy.max(y0)..(oy + size).min(y1) {
                for x in ox.max(x0)..(ox + size).min(x1) {
                    tile.set_pixel((x - ox) as u32, (y - oy) as u32, [buffer[(y - y0) as usize * w + (x - x0) as usize]]);
                }
            }
            out.insert(TileCoord::new(tx, ty), tile);
        }
    }
    out
}

/// Flou en boîte à somme glissante, bords prolongés. `horizontal` choisit l'axe.
fn box_blur(src: &[u8], dst: &mut [u8], w: usize, h: usize, radius: usize, horizontal: bool) {
    let (lines, length) = if horizontal { (h, w) } else { (w, h) };
    let index = |line: usize, i: usize| if horizontal { line * w + i } else { i * w + line };
    let window = (2 * radius + 1) as u32;
    for line in 0..lines {
        let at = |i: isize| u32::from(src[index(line, i.clamp(0, length as isize - 1) as usize)]);
        let mut sum: u32 = (-(radius as isize)..=radius as isize).map(at).sum();
        for i in 0..length {
            dst[index(line, i)] = ((sum + window / 2) / window) as u8;
            sum += at(i as isize + radius as isize + 1);
            sum -= at(i as isize - radius as isize);
        }
    }
}

/// Réglages de la baguette magique.
#[derive(Clone, Copy, Debug)]
pub struct Wand {
    /// 0 à 255, sur une échelle perceptuelle : 32 sépare deux couleurs que l'œil juge « proches ».
    pub tolerance: u8,
    pub contiguous: bool,
    /// Bord lissé : les pixels à la limite de la tolérance sont sélectionnés en partie.
    pub anti_alias: bool,
    /// Côté du carré dont on fait la moyenne pour la couleur de référence : 1 (le pixel cliqué), 3 ou 5.
    pub sample: u32,
}

impl Wand {
    pub fn plain(tolerance: u8, contiguous: bool) -> Self {
        Self { tolerance, contiguous, anti_alias: true, sample: 1 }
    }
}

/// Couleur dans l'espace Oklab (clarté, axe vert-rouge, axe bleu-jaune) + alpha : les distances y
/// correspondent à ce que l'œil perçoit, contrairement aux écarts rouge/vert/bleu.
fn oklab(px: [u8; 4]) -> [f32; 4] {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(px[0]), lin(px[1]), lin(px[2]));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        f32::from(px[3]) / 255.0,
    ]
}

/// Écart entre deux couleurs, ramené à l'échelle 0..255 de la tolérance. Un pixel transparent n'a
/// pas de couleur : seul son alpha compte.
fn difference(a: [f32; 4], b: [f32; 4]) -> f32 {
    let alpha = (a[3] - b[3]).abs() * 255.0;
    if a[3] <= 0.0 || b[3] <= 0.0 {
        return alpha;
    }
    let colour = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt() * 290.0;
    colour.max(alpha)
}

/// Baguette magique sur les pixels d'un calque placé en `offset`. Les coordonnées et
/// le masque produit sont en repère document. `None` si le point est hors document.
pub fn magic_wand(pixels: &TileStore, offset: (i32, i32), size: (u32, u32), (x, y): (u32, u32), tolerance: u8, contiguous: bool) -> Option<MaskStore> {
    wand_select(pixels, offset, size, (x, y), Wand::plain(tolerance, contiguous))
}

pub fn wand_select(pixels: &TileStore, offset: (i32, i32), (width, height): (u32, u32), (x, y): (u32, u32), wand: Wand) -> Option<MaskStore> {
    if x >= width || y >= height {
        return None;
    }
    // Couleur de référence : le pixel cliqué, ou la moyenne d'un petit carré (insensible au bruit).
    let reach = (wand.sample.clamp(1, 9) / 2) as i32;
    let mut sampler = Sampler::new(pixels);
    let (mut sum, mut count) = ([0.0f32; 4], 0.0f32);
    // Tout le carré échantillonné sert de point de départ : si le pixel cliqué est lui-même aberrant
    // (bruit, poussière), la sélection part quand même de ses voisins.
    let mut seeds = Vec::new();
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let (sx, sy) = (x as i32 + dx, y as i32 + dy);
            if sx >= 0 && sy >= 0 && (sx as u32) < width && (sy as u32) < height {
                seeds.push((sx as u32, sy as u32));
                let lab = oklab(unpremultiply(sampler.get(sx - offset.0, sy - offset.1)));
                for c in 0..4 {
                    sum[c] += lab[c];
                }
                count += 1.0;
            }
        }
    }
    let reference = sum.map(|v| v / count.max(1.0));
    Some(grow(pixels, offset, (width, height), &seeds, &[reference], wand, None))
}

/// Sélection rapide : on peint sur le sujet, la sélection s'étend aux couleurs voisines de celles
/// qu'on a peintes. `brushed` est la zone peinte (repère document), `limit` la zone au-delà de
/// laquelle la sélection ne s'étend pas (quelques diamètres de pinceau autour du trait).
pub fn quick_select(pixels: &TileStore, offset: (i32, i32), (width, height): (u32, u32), brushed: &MaskStore, limit: &MaskStore, tolerance: u8) -> MaskStore {
    let Some((x0, y0, x1, y1)) = brushed.pixel_bounds(|v| v[0] > 127) else { return MaskStore::with_background([0]) };
    let (mut pixels_at, mut painted) = (Sampler::new(pixels), Sampler::new(brushed));
    // Couleurs de référence : celles rencontrées sous le pinceau, regroupées (au plus 8 groupes).
    let (mut seeds, mut references): (Vec<(u32, u32)>, Vec<[f32; 4]>) = (Vec::new(), Vec::new());
    for py in y0.max(0)..y1.min(height as i32) {
        for px in x0.max(0)..x1.min(width as i32) {
            if painted.get(px, py)[0] <= 127 {
                continue;
            }
            seeds.push((px as u32, py as u32));
            // Un pixel sur quatre suffit à connaître les couleurs présentes.
            if (px + py) % 4 == 0 {
                let lab = oklab(unpremultiply(pixels_at.get(px - offset.0, py - offset.1)));
                if references.len() < 8 && references.iter().all(|r| difference(*r, lab) > f32::from(tolerance.max(8))) {
                    references.push(lab);
                }
            }
        }
    }
    if references.is_empty() {
        return MaskStore::with_background([0]);
    }
    let wand = Wand { tolerance, contiguous: true, anti_alias: true, sample: 1 };
    let mut out = grow(pixels, offset, (width, height), &seeds, &references, wand, Some(limit));
    // Ce qu'on a peint est sélectionné quoi qu'il arrive : le geste prime sur la couleur.
    for (px, py) in seeds {
        let (coord, tx, ty) = TileCoord::of_pixel(px as i32, py as i32);
        out.get_mut(coord).set_pixel(tx, ty, [255]);
    }
    out
}

/// Demi-largeur de la zone de transition du bord lissé (0 = bord net).
fn softness(wand: Wand) -> f32 {
    if wand.anti_alias && wand.tolerance > 0 { (f32::from(wand.tolerance) * 0.2).max(1.5) } else { 0.0 }
}

/// Valeur de masque d'un pixel à l'écart `d` des couleurs de référence : pleine dans la tolérance,
/// dégradée sur la zone de transition.
#[inline]
fn mask_value(d: f32, tolerance: f32, soft: f32) -> u8 {
    if soft <= 0.0 {
        return if d <= tolerance { 255 } else { 0 };
    }
    (((tolerance + soft - d) / (2.0 * soft)).clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Écart d'un pixel du calque aux couleurs de référence, avec une petite table à adressage direct
/// indexée par la couleur RGBA : sur un aplat, la conversion Oklab (trois `powf`, trois `cbrt`)
/// n'est faite qu'une fois pour toute la zone au lieu d'une fois par pixel.
struct Distance<'a> {
    sampler: Sampler<'a, 4>,
    /// Zone hors de laquelle tout pixel est à l'infini (sélection rapide).
    allowed: Option<Sampler<'a, 1>>,
    offset: (i32, i32),
    references: &'a [[f32; 4]],
    /// Clé = couleur RGBA tassée sur 32 bits, bit 32 levé (0 = case vide) ; valeur = écart.
    cache: Vec<(u64, f32)>,
}

impl<'a> Distance<'a> {
    const SLOTS: usize = 4096;

    fn new(pixels: &'a TileStore, offset: (i32, i32), references: &'a [[f32; 4]], limit: Option<&'a MaskStore>) -> Self {
        Self { sampler: Sampler::new(pixels), allowed: limit.map(Sampler::new), offset, references, cache: vec![(0, 0.0); Self::SLOTS] }
    }

    #[inline]
    fn at(&mut self, x: u32, y: u32) -> f32 {
        if self.allowed.as_mut().is_some_and(|mask| mask.get(x as i32, y as i32)[0] == 0) {
            return f32::INFINITY;
        }
        let px = self.sampler.get(x as i32 - self.offset.0, y as i32 - self.offset.1);
        let key = u64::from(u32::from_le_bytes(px)) | (1 << 32);
        // Multiplication de Fibonacci : les couleurs voisines (bruit léger) tombent dans des cases différentes.
        let slot = (u32::from_le_bytes(px).wrapping_mul(0x9E37_79B1) >> 20) as usize;
        let entry = &mut self.cache[slot];
        if entry.0 == key {
            return entry.1;
        }
        let lab = oklab(unpremultiply(px));
        let d = self.references.iter().map(|r| difference(*r, lab)).fold(f32::INFINITY, f32::min);
        *entry = (key, d);
        d
    }
}

/// Écrit dans `out` un masque dont chaque pixel du rectangle `(x0, y0, x1, y1)` (bornes hautes
/// exclues, repère document) vaut `value(x, y)`. Les tuiles sont remplies d'un bloc, puis
/// insérées : une tuile restée vide n'est pas stockée, ni cherchée dans la table à chaque pixel.
fn write_rect(out: &mut MaskStore, (x0, y0, x1, y1): (u32, u32, u32, u32), mut value: impl FnMut(u32, u32) -> u8) {
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for ty in y0 / TILE_SIZE..=(y1 - 1) / TILE_SIZE {
        for tx in x0 / TILE_SIZE..=(x1 - 1) / TILE_SIZE {
            let (ox, oy) = (tx * TILE_SIZE, ty * TILE_SIZE);
            let mut tile = MaskTile::filled([0]);
            let mut any = false;
            for y in oy.max(y0)..(oy + TILE_SIZE).min(y1) {
                let row = ((y - oy) * TILE_SIZE) as usize;
                for x in ox.max(x0)..(ox + TILE_SIZE).min(x1) {
                    let v = value(x, y);
                    if v > 0 {
                        tile.bytes_mut()[row + (x - ox) as usize] = v;
                        any = true;
                    }
                }
            }
            if any {
                out.insert(TileCoord::new(tx as i32, ty as i32), tile);
            }
        }
    }
}

/// Croissance de région commune à la baguette et à la sélection rapide : part des `seeds`, s'étend
/// aux pixels proches d'une des couleurs de `references`, sans sortir de `limit`.
///
/// Chaque pixel n'est lu et converti qu'une fois ; `grow_reference` (tests) est la version
/// d'origine, plus simple, qui sert à vérifier que le masque produit est identique.
fn grow(pixels: &TileStore, offset: (i32, i32), (width, height): (u32, u32), seeds: &[(u32, u32)], references: &[[f32; 4]], wand: Wand, limit: Option<&MaskStore>) -> MaskStore {
    let tolerance = f32::from(wand.tolerance);
    let soft = softness(wand);
    let mut distance = Distance::new(pixels, offset, references, limit);
    let mut out = MaskStore::with_background([0]);

    if !wand.contiguous {
        write_rect(&mut out, (0, 0, width, height), |x, y| mask_value(distance.at(x, y), tolerance, soft));
        return out;
    }

    // Remplissage par segments horizontaux. État de chaque pixel : 0 = pas encore examiné,
    // OUTSIDE = examiné et hors tolérance, sinon la valeur de masque (128..=255) d'un pixel de la région.
    // Un pixel est classé au moment où on le teste, jamais deux fois.
    const OUTSIDE: u8 = 1;
    let w = width as usize;
    let mut state = vec![0u8; w * height as usize];
    // Rectangle englobant de la région (bornes hautes exclues), pour limiter la passe finale.
    let (mut bx0, mut by0, mut bx1, mut by1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    // Vrai si `(x, y)` entre dans la région à cet instant : pas encore examiné et dans la tolérance.
    // Faux s'il est hors tolérance ou déjà dans la région (son segment s'en occupe).
    let mut claim = |x: u32, y: u32| -> bool {
        let slot = &mut state[y as usize * w + x as usize];
        if *slot != 0 {
            return false;
        }
        let d = distance.at(x, y);
        if d > tolerance {
            *slot = OUTSIDE;
            return false;
        }
        *slot = mask_value(d, tolerance, soft).max(128);
        bx0 = bx0.min(x);
        by0 = by0.min(y);
        bx1 = bx1.max(x + 1);
        by1 = by1.max(y + 1);
        true
    };
    // Segments `(gauche, droite, ligne)` dont tous les pixels sont déjà dans la région.
    let mut stack: Vec<(u32, u32, u32)> = Vec::new();
    for &(x, y) in seeds {
        if x < width && y < height && claim(x, y) {
            stack.push((x, x, y));
        }
    }
    while let Some((mut left, mut right, y)) = stack.pop() {
        while left > 0 && claim(left - 1, y) {
            left -= 1;
        }
        while right + 1 < width && claim(right + 1, y) {
            right += 1;
        }
        for ny in [y.checked_sub(1), (y + 1 < height).then_some(y + 1)].into_iter().flatten() {
            let mut px = left;
            while px <= right {
                if claim(px, ny) {
                    // Un seul segment par suite contiguë de la ligne voisine.
                    let start = px;
                    px += 1;
                    while px <= right && claim(px, ny) {
                        px += 1;
                    }
                    stack.push((start, px - 1, ny));
                } else {
                    px += 1;
                }
            }
        }
    }
    if bx0 >= bx1 {
        return out;
    }
    // Valeurs : pleines au cœur de la région, dégradées à sa limite et sur les pixels qui la bordent.
    // Seul le rectangle de la région, élargi d'un pixel, peut contenir des pixels non nuls.
    let rect = (bx0.saturating_sub(1), by0.saturating_sub(1), (bx1 + 1).min(width), (by1 + 1).min(height));
    write_rect(&mut out, rect, |px, py| {
        let i = py as usize * w + px as usize;
        let inside = state[i] >= 128;
        if inside {
            return state[i];
        }
        let borders = soft > 0.0
            && ((px > 0 && state[i - 1] >= 128) || (px + 1 < width && state[i + 1] >= 128) || (py > 0 && state[i - w] >= 128) || (py + 1 < height && state[i + w] >= 128));
        if borders { mask_value(distance.at(px, py), tolerance, soft).min(127) } else { 0 }
    });
    out
}

/// Version d'origine de [`grow`], conservée comme référence : même résultat, mais chaque pixel est
/// relu et reconverti plusieurs fois et la passe finale parcourt tout le document.
#[cfg(test)]
fn grow_reference(pixels: &TileStore, offset: (i32, i32), (width, height): (u32, u32), seeds: &[(u32, u32)], references: &[[f32; 4]], wand: Wand, limit: Option<&MaskStore>) -> MaskStore {
    let tolerance = f32::from(wand.tolerance);
    let soft = softness(wand);
    let mut sampler = Sampler::new(pixels);
    let mut allowed = limit.map(Sampler::new);
    let mut distance = |x: u32, y: u32| -> f32 {
        if allowed.as_mut().is_some_and(|mask| mask.get(x as i32, y as i32)[0] == 0) {
            return f32::INFINITY;
        }
        let lab = oklab(unpremultiply(sampler.get(x as i32 - offset.0, y as i32 - offset.1)));
        references.iter().map(|r| difference(*r, lab)).fold(f32::INFINITY, f32::min)
    };
    let value = |d: f32| mask_value(d, tolerance, soft);
    let mut out = MaskStore::with_background([0]);
    let mut put = |x: u32, y: u32, v: u8| {
        if v > 0 {
            let (coord, px, py) = TileCoord::of_pixel(x as i32, y as i32);
            out.get_mut(coord).set_pixel(px, py, [v]);
        }
    };

    if !wand.contiguous {
        for py in 0..height {
            for px in 0..width {
                put(px, py, value(distance(px, py)));
            }
        }
        return out;
    }

    // Remplissage par segments horizontaux : chaque ligne est parcourue une seule fois. Seuls les pixels
    // DANS la tolérance propagent la sélection ; ceux de la zone de transition la bordent sans la faire fuir.
    let w = width as usize;
    let mut visited = vec![false; w * height as usize];
    let mut stack: Vec<(u32, u32)> = seeds.iter().copied().filter(|(x, y)| *x < width && *y < height).collect();
    while let Some((sx, sy)) = stack.pop() {
        let row = sy as usize * w;
        if visited[row + sx as usize] || distance(sx, sy) > tolerance {
            continue;
        }
        let mut left = sx;
        while left > 0 && !visited[row + left as usize - 1] && distance(left - 1, sy) <= tolerance {
            left -= 1;
        }
        let mut right = sx;
        while right + 1 < width && !visited[row + right as usize + 1] && distance(right + 1, sy) <= tolerance {
            right += 1;
        }
        for px in left..=right {
            visited[row + px as usize] = true;
        }
        for ny in [sy.checked_sub(1), (sy + 1 < height).then_some(sy + 1)].into_iter().flatten() {
            let neighbour = ny as usize * w;
            let mut px = left;
            while px <= right {
                if !visited[neighbour + px as usize] && distance(px, ny) <= tolerance {
                    stack.push((px, ny));
                    // Un seul départ par segment contigu de la ligne voisine.
                    while px <= right && !visited[neighbour + px as usize] && distance(px, ny) <= tolerance {
                        px += 1;
                    }
                } else {
                    px += 1;
                }
            }
        }
    }
    // Valeurs : pleines au cœur de la région, dégradées à sa limite et sur les pixels qui la bordent.
    for py in 0..height {
        for px in 0..width {
            let i = py as usize * w + px as usize;
            let inside = visited[i];
            let borders = !inside
                && soft > 0.0
                && ((px > 0 && visited[i - 1]) || (px + 1 < width && visited[i + 1]) || (py > 0 && visited[i - w]) || (py + 1 < height && visited[i + w]));
            if inside || borders {
                let v = value(distance(px, py));
                put(px, py, if inside { v.max(128) } else { v.min(127) });
            }
        }
    }
    out
}

/// Dilate (`amount` > 0) ou contracte (`amount` < 0) une sélection de tant de pixels, puis arrondit
/// son contour (`smooth`, en pixels). Travaille sur le rectangle du document.
pub fn modify(mask: &MaskStore, (width, height): (u32, u32), amount: i32, smooth: u32) -> Option<MaskStore> {
    let (w, h) = (width as usize, height as usize);
    let mut values: Vec<f32> = mask.to_bytes(width, height).iter().map(|v| f32::from(*v) / 255.0).collect();
    if amount != 0 {
        let solid: Vec<bool> = values.iter().map(|v| *v >= 0.5).collect();
        // Distance au bord, mesurée du côté où l'on se déplace.
        let reference: Vec<bool> = if amount > 0 { solid.clone() } else { solid.iter().map(|s| !s).collect() };
        let distance = crate::effects::distance_to(&reference, w, h);
        let reach = amount.unsigned_abs() as f32;
        for (i, slot) in values.iter_mut().enumerate() {
            let cover = (reach - distance[i] + 0.5).clamp(0.0, 1.0);
            *slot = if amount > 0 { slot.max(cover) } else { slot.min(1.0 - cover) };
        }
    }
    if smooth > 0 {
        // Flou puis resserrement autour du seuil : les angles s'arrondissent, le bord reste net.
        let packed: Vec<f32> = values.iter().flat_map(|v| [*v, 0.0, 0.0, 0.0]).collect();
        let blurred = crate::filters::gaussian(&packed, w, h, smooth as f32);
        for (slot, px) in values.iter_mut().zip(blurred.chunks_exact(4)) {
            *slot = ((px[0] - 0.5) * (smooth as f32).max(2.0) + 0.5).clamp(0.0, 1.0);
        }
    }
    let mut out = MaskStore::with_background([0]);
    let mut any = false;
    for (i, v) in values.iter().enumerate() {
        let byte = (v * 255.0 + 0.5) as u8;
        if byte > 0 {
            any = true;
            let (coord, px, py) = TileCoord::of_pixel((i % w) as i32, (i / w) as i32);
            out.get_mut(coord).set_pixel(px, py, [byte]);
        }
    }
    any.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(mask: &MaskStore, x: i32, y: i32) -> u8 {
        Sampler::new(mask).get(x, y)[0]
    }

    /// Image `w x h` dont chaque pixel est donné par `color(x, y)`.
    fn image(w: u32, h: u32, color: impl Fn(u32, u32) -> [u8; 4]) -> TileStore {
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| crate::pixel::premultiply(color(i % w, i / w))).collect();
        TileStore::from_premultiplied(w, h, &rgba).expect("tampon")
    }

    #[test]
    fn the_wand_sees_colours_like_the_eye_does() {
        // Ciel en dégradé de bleu (clair en haut, plus dense en bas) avec un nuage gris de même clarté au milieu.
        let sky = image(200, 200, |x, y| {
            if (60..140).contains(&x) && (80..120).contains(&y) { [150, 150, 150, 255] } else { [70 + (y / 4) as u8, 130 + (y / 5) as u8, 235, 255] }
        });
        let mask = wand_select(&sky, (0, 0), (200, 200), (10, 100), Wand { tolerance: 40, contiguous: true, anti_alias: true, sample: 1 }).expect("sélection");
        assert_eq!((value(&mask, 10, 5), value(&mask, 190, 195)), (255, 255), "tout le dégradé de ciel est pris");
        assert_eq!(value(&mask, 100, 100), 0, "le nuage gris ne l'est pas, bien que sa clarté soit la même");
    }

    #[test]
    fn the_edge_is_soft_and_a_noisy_click_is_harmless() {
        // Bord en trois marches : blanc, gris très clair (dans la tolérance), gris clair (à la limite), puis noir.
        let steps = image(100, 40, |x, _| match x {
            0..=39 => [255, 255, 255, 255],
            40 => [236, 236, 236, 255],
            41 => [226, 226, 226, 255],
            _ => [0, 0, 0, 255],
        });
        let soft = wand_select(&steps, (0, 0), (100, 40), (5, 20), Wand { tolerance: 30, contiguous: true, anti_alias: true, sample: 1 }).expect("sélection");
        let edge = value(&soft, 41, 20);
        assert_eq!((value(&soft, 20, 20), value(&soft, 60, 20)), (255, 0));
        assert!((1..255).contains(&edge), "pixel à la limite de la tolérance : sélectionné en partie ({edge})");
        let hard = wand_select(&steps, (0, 0), (100, 40), (5, 20), Wand { tolerance: 30, contiguous: true, anti_alias: false, sample: 1 }).expect("sélection");
        assert!((0..100).all(|x| matches!(value(&hard, x, 20), 0 | 255)), "sans lissage : tout ou rien");

        // Un pixel aberrant sous le clic : avec une moyenne 5 x 5, la sélection reste celle de l'aplat.
        let noisy = image(100, 40, |x, y| if (x, y) == (20, 20) { [140, 140, 140, 255] } else if x < 50 { [250, 250, 250, 255] } else { [0, 0, 0, 255] });
        let point = wand_select(&noisy, (0, 0), (100, 40), (20, 20), Wand { tolerance: 20, contiguous: true, anti_alias: true, sample: 1 }).expect("sélection");
        let averaged = wand_select(&noisy, (0, 0), (100, 40), (20, 20), Wand { tolerance: 20, contiguous: true, anti_alias: true, sample: 5 }).expect("sélection");
        assert_eq!(value(&point, 5, 5), 0, "échantillon ponctuel : le clic sur le pixel aberrant ne prend que lui");
        assert_eq!((value(&averaged, 5, 5), value(&averaged, 80, 5)), (255, 0), "moyenne 5 x 5 : tout l'aplat clair");
    }

    #[test]
    fn quick_selection_grows_from_the_stroke_to_the_object_edges() {
        // Disque rouge ombré (deux rouges) sur fond vert ; on peint un court trait au centre.
        let scene = image(300, 300, |x, y| {
            let (dx, dy) = (x as f32 - 150.0, y as f32 - 150.0);
            if dx.hypot(dy) < 80.0 { if x < 150 { [200, 40, 40, 255] } else { [170, 30, 35, 255] } } else { [40, 160, 60, 255] }
        });
        let brushed = crate::paint::stroke_mask(&[(135.0, 150.0), (165.0, 150.0)], 16.0, 1.0);
        let limit = crate::paint::stroke_mask(&[(135.0, 150.0), (165.0, 150.0)], 400.0, 1.0);
        let mask = quick_select(&scene, (0, 0), (300, 300), &brushed, &limit, 24);
        let inside = [value(&mask, 90, 150), value(&mask, 215, 150), value(&mask, 150, 85)];
        assert!(inside.iter().all(|v| *v > 240), "tout le disque, ses deux rouges compris : {inside:?}");
        assert_eq!((value(&mask, 40, 150), value(&mask, 150, 250)), (0, 0), "pas le fond");
        // Limite de portée : avec un rayon d'action réduit, la sélection ne va pas plus loin.
        let near = crate::paint::stroke_mask(&[(135.0, 150.0), (165.0, 150.0)], 60.0, 1.0);
        let bounded = quick_select(&scene, (0, 0), (300, 300), &brushed, &near, 24);
        assert_eq!((value(&bounded, 150, 150), value(&bounded, 90, 150)), (255, 0));
    }

    #[test]
    fn selections_can_be_grown_shrunk_and_smoothed() {
        let square = Shape::Rect { x: 100.0, y: 100.0, width: 100.0, height: 100.0 }.rasterize(300, 300);
        let grown = modify(&square, (300, 300), 10, 0).expect("dilatation");
        assert_eq!((value(&grown, 92, 150), value(&grown, 88, 150), value(&grown, 94, 94)), (255, 0, 255));
        assert_eq!(value(&grown, 91, 91), 0, "la dilatation arrondit les angles (distance euclidienne)");
        let shrunk = modify(&square, (300, 300), -10, 0).expect("contraction");
        assert_eq!((value(&shrunk, 112, 150), value(&shrunk, 106, 150)), (255, 0));
        let smooth = modify(&square, (300, 300), 0, 12).expect("lissage");
        assert_eq!((value(&smooth, 150, 150), value(&smooth, 101, 101), value(&smooth, 150, 102)), (255, 0, 255), "angles arrondis, côtés en place");
        assert!(modify(&square, (300, 300), -60, 0).is_none(), "contractée jusqu'à disparaître");
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> MaskStore {
        Shape::Rect { x, y, width: w, height: h }.rasterize(600, 400)
    }

    #[test]
    fn rectangle_is_pixel_aligned_clamped_and_accepts_negative_sizes() {
        let mask = rect(250.4, 10.0, 20.2, 5.0);
        assert_eq!((value(&mask, 249, 12), value(&mask, 250, 12), value(&mask, 269, 12), value(&mask, 270, 12)), (0, 255, 255, 0));
        assert_eq!(mask.tile_count(), 2, "à cheval sur deux tuiles");
        let dragged_backwards = rect(270.0, 15.0, -20.0, -5.0);
        assert_eq!(value(&dragged_backwards, 255, 12), 255);
        let overflowing = rect(-50.0, -50.0, 100.0, 100.0);
        assert_eq!((value(&overflowing, 0, 0), value(&overflowing, 49, 49), value(&overflowing, 50, 50)), (255, 255, 0));
    }

    #[test]
    fn ellipse_is_antialiased() {
        let mask = Shape::Ellipse { x: 100.5, y: 100.0, width: 100.0, height: 61.0 }.rasterize(600, 400);
        assert_eq!(value(&mask, 150, 130), 255, "centre");
        assert_eq!(value(&mask, 101, 101), 0, "coin de la boîte englobante");
        let edge = value(&mask, 100, 130);
        assert!(edge > 0 && edge < 255, "bord lissé : {edge}");
    }

    #[test]
    fn polygon_fills_with_even_odd_rule_and_smooth_edges() {
        let triangle = Shape::Polygon { points: vec![[10.0, 10.0], [110.0, 10.0], [10.0, 110.0]] }.rasterize(600, 400);
        assert_eq!(value(&triangle, 20, 20), 255);
        assert_eq!(value(&triangle, 100, 100), 0);
        let diagonal = value(&triangle, 60, 59);
        assert!(diagonal > 60 && diagonal < 200, "hypoténuse lissée : {diagonal}");
        assert!(Shape::Polygon { points: vec![[0.0, 0.0], [5.0, 5.0]] }.rasterize(600, 400).tile_count() == 0);

        // Deux contours emboîtés : le second perce un trou dans le premier.
        let square = |a: f32, b: f32| vec![[a, a], [b, a], [b, b], [a, b]];
        let ring = Shape::Contours { contours: vec![square(10.0, 110.0), square(40.0, 80.0)] }.rasterize(600, 400);
        assert_eq!((value(&ring, 20, 20), value(&ring, 60, 60), value(&ring, 100, 100)), (255, 0, 255));
    }

    #[test]
    fn combine_modes() {
        let a = rect(0.0, 0.0, 100.0, 100.0);
        let b = || rect(50.0, 50.0, 100.0, 100.0);
        let at = |m: &Option<MaskStore>, x, y| m.as_ref().map_or(0, |m| value(m, x, y));

        let added = combine(Some(&a), b(), CombineMode::Add);
        assert_eq!((at(&added, 10, 10), at(&added, 75, 75), at(&added, 140, 140), at(&added, 140, 10)), (255, 255, 255, 0));
        let subtracted = combine(Some(&a), b(), CombineMode::Subtract);
        assert_eq!((at(&subtracted, 10, 10), at(&subtracted, 75, 75), at(&subtracted, 140, 140)), (255, 0, 0));
        let intersected = combine(Some(&a), b(), CombineMode::Intersect);
        assert_eq!((at(&intersected, 10, 10), at(&intersected, 75, 75), at(&intersected, 140, 140)), (0, 255, 0));
        assert!(combine(Some(&a), rect(0.0, 0.0, 100.0, 100.0), CombineMode::Subtract).is_none(), "tout soustraire = désélectionner");
        assert!(combine(None, b(), CombineMode::Subtract).is_none());
    }

    #[test]
    fn invert_twice_is_identity_and_handles_no_selection() {
        let a = rect(10.0, 10.0, 20.0, 20.0);
        let inverted = invert(Some(&a)).expect("inverse");
        assert_eq!((value(&inverted, 15, 15), value(&inverted, 300, 300), value(&inverted, 599, 399)), (0, 255, 255));
        let back = invert(Some(&inverted)).expect("double inverse");
        assert_eq!((value(&back, 15, 15), value(&back, 300, 300)), (255, 0));
        assert_eq!(invert(None).expect("tout").background(), [255]);
        assert!(invert(Some(&select_all())).is_none(), "inverser « tout » = rien");
    }

    #[test]
    fn feather_softens_edges_and_preserves_the_inside() {
        let mask = feather(&rect(200.0, 100.0, 200.0, 200.0), 8.0, 600, 400);
        assert_eq!(value(&mask, 300, 200), 255, "cœur intact");
        let edge = value(&mask, 200, 200);
        assert!((100..=155).contains(&edge), "environ la moitié sur l'ancien bord : {edge}");
        assert!(value(&mask, 190, 200) > 0, "la sélection déborde en s'estompant");
        assert_eq!(value(&mask, 100, 200), 0);
    }

    /// Image 4000 x 4000 : fond blanc plat, un disque rouge et une bande noire (zones planes, comme un aplat
    /// de dessin). Sert à mesurer le pot de peinture : `cargo test -p engine --release --lib bench_bucket -- --ignored --nocapture`.
    fn flat_scene(side: u32) -> TileStore {
        let mut pixels = TileStore::new();
        pixels.fill(side, side, [255, 255, 255, 255]);
        let (cx, cy, r) = (side as i32 / 2, side as i32 / 2, side as i32 / 5);
        let size = TILE_SIZE as i32;
        for ty in 0..crate::tiles::tiles_for(side) as i32 {
            for tx in 0..crate::tiles::tiles_for(side) as i32 {
                let coord = TileCoord::new(tx, ty);
                let mut tile = (**pixels.get(coord).expect("tuile")).clone();
                let mut changed = false;
                for y in 0..size {
                    for x in 0..size {
                        let (px, py) = (tx * size + x, ty * size + y);
                        let colour = if (px - cx).pow(2) + (py - cy).pow(2) < r * r {
                            Some([220, 30, 30, 255])
                        } else if (px - py).abs() < 8 {
                            Some([0, 0, 0, 255])
                        } else {
                            None
                        };
                        if let Some(colour) = colour {
                            tile.set_pixel(x as u32, y as u32, colour);
                            changed = true;
                        }
                    }
                }
                if changed {
                    pixels.insert(coord, tile);
                }
            }
        }
        pixels
    }

    /// Masques identiques : mêmes tuiles stockées, mêmes octets.
    fn same_mask(a: &MaskStore, b: &MaskStore) -> bool {
        a.background() == b.background()
            && a.sorted_coords() == b.sorted_coords()
            && a.iter().all(|(coord, tile)| b.get(coord).is_some_and(|other| other.bytes() == tile.bytes()))
    }

    /// Générateur déterministe (xorshift) : les tests sont reproductibles sans dépendance.
    fn random(seed: &mut u64) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 32) as u32
    }

    #[test]
    fn grow_matches_the_reference_implementation() {
        // Image 700 x 500 à cheval sur plusieurs tuiles : damier de grands aplats, bruit léger sur certains,
        // un disque semi-transparent, un trou transparent et quelques pixels isolés très différents.
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let palette = [[255, 255, 255, 255], [250, 250, 250, 255], [40, 60, 200, 255], [40, 60, 210, 255], [0, 0, 0, 255], [200, 200, 200, 128], [0, 0, 0, 0]];
        let (w, h) = (700u32, 500u32);
        let mut cells = vec![[0u8; 4]; 7 * 5];
        for cell in cells.iter_mut() {
            *cell = palette[random(&mut seed) as usize % palette.len()];
        }
        let mut noise = vec![0u8; (w * h) as usize];
        for v in noise.iter_mut() {
            *v = (random(&mut seed) % 5) as u8;
        }
        let pixels = image(w, h, |x, y| {
            let cell = cells[(y / 100 * 7 + x / 100) as usize];
            let (dx, dy) = (x as f32 - 350.0, y as f32 - 250.0);
            if dx.hypot(dy) < 60.0 {
                return [120, 200, 90, 160];
            }
            if (x * 7 + y * 13) % 311 == 0 {
                return [255, 0, 0, 255];
            }
            let n = noise[(y * w + x) as usize];
            // Bruit sur les cellules à alpha plein uniquement, et seulement une cellule sur deux.
            if cell[3] == 255 && (x / 100 + y / 100) % 2 == 0 { [cell[0].saturating_sub(n), cell[1].saturating_sub(n), cell[2].saturating_sub(n), 255] } else { cell }
        });
        let limit = Shape::Ellipse { x: 100.0, y: 50.0, width: 500.0, height: 400.0 }.rasterize(w, h);
        let mut cases = 0;
        for offset in [(0, 0), (-37, 23)] {
            let size = ((w as i32 + offset.0) as u32, (h as i32 + offset.1) as u32);
            for &(x, y) in &[(10u32, 10u32), (350, 250), (150, 150), (620, 450), (0, 0)] {
                for tolerance in [0u8, 5, 24, 60] {
                    for contiguous in [true, false] {
                        for anti_alias in [true, false] {
                            let wand = Wand { tolerance, contiguous, anti_alias, sample: 1 };
                            let click = ((x as i32 + offset.0).max(0) as u32, (y as i32 + offset.1).max(0) as u32);
                            let lab = oklab(unpremultiply(Sampler::new(&pixels).get(click.0 as i32 - offset.0, click.1 as i32 - offset.1)));
                            for limit in [None, Some(&limit)] {
                                let new = grow(&pixels, offset, size, &[click], &[lab], wand, limit);
                                let old = grow_reference(&pixels, offset, size, &[click], &[lab], wand, limit);
                                assert!(same_mask(&new, &old), "masque différent : clic {click:?}, décalage {offset:?}, tolérance {tolerance}, contigu {contiguous}, lissage {anti_alias}, limite {}", limit.is_some());
                                cases += 1;
                            }
                        }
                    }
                }
            }
            // Plusieurs graines et plusieurs références (sélection rapide).
            let seeds: Vec<(u32, u32)> = (0..40).map(|_| (random(&mut seed) % size.0, random(&mut seed) % size.1)).collect();
            let references: Vec<[f32; 4]> = palette[..4].iter().map(|px| oklab(*px)).collect();
            let wand = Wand { tolerance: 20, contiguous: true, anti_alias: true, sample: 1 };
            let new = grow(&pixels, offset, size, &seeds, &references, wand, Some(&limit));
            let old = grow_reference(&pixels, offset, size, &seeds, &references, wand, Some(&limit));
            assert!(same_mask(&new, &old), "sélection rapide, décalage {offset:?}");
            assert!(new.tile_count() > 0);
        }
        assert!(cases >= 300, "{cases} cas comparés");
    }

    #[test]
    fn bucket_fill_only_touches_the_tiles_of_the_region() {
        use crate::document::{Background, Document};
        // Forme en L rouge sur fond blanc, couvrant 5 des 9 tuiles de son rectangle englobant.
        let scene = image(700, 700, |x, y| if x < 100 || y < 100 { [220, 30, 30, 255] } else { [255; 4] });
        let mut doc = Document::with_background("L", 700, 700, 72.0, Background::White).expect("document");
        let layer = doc.add_pixel_layer("forme", scene, None, 1).expect("calque");
        let before = std::sync::Arc::clone(doc.layer(layer).expect("calque").pixels().expect("pixels"));
        doc.bucket_fill(layer, (10, 10), [10, 200, 10], 10, true).expect("pot de peinture");
        let after = doc.layer(layer).expect("calque").pixels().expect("pixels");
        let mut changed = before.changed_coords(after);
        changed.sort_unstable();
        let expected = [TileCoord::new(0, 0), TileCoord::new(1, 0), TileCoord::new(2, 0), TileCoord::new(0, 1), TileCoord::new(0, 2)];
        let mut expected = expected.to_vec();
        expected.sort_unstable();
        assert_eq!(changed, expected, "seules les tuiles du L changent, pas celles du coin blanc");
        let at = |x, y| Sampler::new(after).get(x, y);
        assert_eq!((at(50, 50), at(650, 50), at(50, 650), at(400, 400)), ([10, 200, 10, 255], [10, 200, 10, 255], [10, 200, 10, 255], [255; 4]));
    }

    #[test]
    #[ignore = "mesure de temps : lancer à la main en --release avec --nocapture"]
    fn bench_bucket_fill_on_a_flat_4000_square() {
        use crate::document::{Background, Document};
        const SIDE: u32 = 4000;
        let scene = flat_scene(SIDE);
        let mut doc = Document::with_background("bench", SIDE, SIDE, 72.0, Background::White).expect("document");
        let layer = doc.add_pixel_layer("aplat", scene.clone(), None, 1).expect("calque");
        let seeds = [(SIDE / 2, SIDE / 2, 10, true, "disque rouge contigu"), (100, SIDE - 100, 10, true, "fond blanc contigu"), (SIDE / 2, SIDE / 2, 10, false, "disque rouge non contigu")];
        for (x, y, tolerance, contiguous, label) in seeds {
            let start = std::time::Instant::now();
            let mask = magic_wand(&scene, (0, 0), (SIDE, SIDE), (x, y), tolerance, contiguous).expect("baguette");
            let wand = start.elapsed();
            let lab = oklab(unpremultiply(Sampler::new(&scene).get(x as i32, y as i32)));
            let start = std::time::Instant::now();
            let reference = grow_reference(&scene, (0, 0), (SIDE, SIDE), &[(x, y)], &[lab], Wand::plain(tolerance, contiguous), None);
            let old = start.elapsed();
            assert!(same_mask(&mask, &reference), "{label} : masque différent de la référence");
            let start = std::time::Instant::now();
            doc.bucket_fill(layer, (x, y), [10, 200, 10], tolerance, contiguous).expect("pot de peinture");
            let fill = start.elapsed();
            println!("{label} : baguette {wand:?} (ancienne version {old:?}, tuiles du masque : {}), pot de peinture complet {fill:?}", mask.tile_count());
        }
    }

    #[test]
    fn magic_wand_contiguous_and_global() {
        let mut pixels = TileStore::new();
        pixels.fill(600, 300, [255, 255, 255, 255]);
        // Deux carrés rouges séparés, dont un à cheval sur deux tuiles ; le second est légèrement plus sombre.
        for (x0, color) in [(240i32, [255, 0, 0, 255]), (400, [240, 0, 0, 255])] {
            for y in 50..90 {
                for x in x0..x0 + 40 {
                    let (coord, px, py) = TileCoord::of_pixel(x, y);
                    pixels.get_mut(coord).set_pixel(px, py, color);
                }
            }
        }
        let wand = |x, y, tolerance, contiguous| magic_wand(&pixels, (0, 0), (600, 300), (x, y), tolerance, contiguous);
        let contiguous = wand(250, 60, 32, true).expect("baguette");
        assert_eq!((value(&contiguous, 241, 51), value(&contiguous, 279, 89), value(&contiguous, 239, 60)), (255, 255, 0));
        assert_eq!(value(&contiguous, 410, 60), 0, "le second carré n'est pas contigu");

        let everywhere = wand(250, 60, 32, false).expect("baguette");
        assert_eq!((value(&everywhere, 250, 60), value(&everywhere, 410, 60), value(&everywhere, 10, 10)), (255, 255, 0));
        let strict = wand(250, 60, 5, false).expect("baguette");
        assert_eq!(value(&strict, 410, 60), 0, "tolérance trop faible pour le rouge sombre");

        let background = wand(5, 5, 0, true).expect("baguette");
        assert_eq!((value(&background, 590, 290), value(&background, 250, 60)), (255, 0), "le fond contourne les carrés");
        assert!(wand(600, 0, 0, true).is_none());
    }
}
