//! Correcteur localisé (le « pansement ») : on peint sur un défaut, il disparaît.
//!
//! Principe, celui du correcteur de Photoshop en mode « similarité des couleurs » :
//!   1. la **matière** vient d'une zone voisine de même forme, choisie automatiquement : celle dont le
//!      pourtour ressemble le plus au pourtour de la zone à corriger ;
//!   2. la **lumière et la couleur** viennent de la destination : l'écart entre les deux pourtours est
//!      diffusé vers l'intérieur (équation de Laplace), puis ajouté à la matière copiée. Le raccord est
//!      invisible même si la zone source est plus claire ou d'une autre teinte.

use rayon::prelude::*;

use crate::error::EngineError;
use crate::tiles::{MaskStore, TileStore};

/// Plus grande zone corrigeable d'un coup, en pixels (boîte englobante, marge comprise).
const MAX_AREA: i64 = 16_000_000;

/// Corrige dans `pixels` (repère local) la zone désignée par `mask` (même repère ; 255 = à corriger).
pub fn heal(pixels: &TileStore, mask: &MaskStore) -> Result<TileStore, EngineError> {
    let Some((mx0, my0, mx1, my1)) = mask.pixel_bounds(|v| v[0] > 0) else { return Ok(pixels.clone()) };
    let (zone_w, zone_h) = (mx1 - mx0, my1 - my0);
    // Fenêtre de travail : la zone, plus de quoi chercher une source tout autour.
    let reach = (zone_w.max(zone_h) * 3).clamp(24, 600);
    let (left, top) = (mx0 - reach, my0 - reach);
    let (w, h) = ((zone_w + 2 * reach) as usize, (zone_h + 2 * reach) as usize);
    if (w as i64) * (h as i64) > MAX_AREA {
        return Err(EngineError::InvalidOperation("zone trop grande pour le correcteur : procédez par touches plus petites".into()));
    }
    let image: Vec<f32> = pixels.translated(-left, -top).to_premultiplied(w as u32, h as u32).iter().map(|v| f32::from(*v)).collect();
    let cover: Vec<f32> = mask.translated(-left, -top).to_bytes(w as u32, h as u32).iter().map(|v| f32::from(*v) / 255.0).collect();
    let inside = |x: usize, y: usize| cover[y * w + x] > 0.0;

    // Pourtour : pixels hors zone qui touchent la zone (à 2 px près), là où la comparaison a un sens.
    let mut ring = Vec::new();
    for y in 2..h - 2 {
        for x in 2..w - 2 {
            if !inside(x, y) && image[(y * w + x) * 4 + 3] > 0.0 && [(-2i32, 0i32), (2, 0), (0, -2), (0, 2)].iter().any(|(dx, dy)| inside((x as i32 + dx) as usize, (y as i32 + dy) as usize)) {
                ring.push((x, y));
            }
        }
    }
    if ring.is_empty() {
        return Err(EngineError::InvalidOperation("rien autour de la zone pour la reconstruire".into()));
    }

    // Recherche de la source : 24 directions, trois distances. La source ne doit ni recouvrir la zone ni sortir de l'image.
    let zone: Vec<(usize, usize)> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|(x, y)| inside(*x, *y)).collect();
    let span = zone_w.max(zone_h) as f32;
    let candidates: Vec<(i32, i32)> = (0..24)
        .flat_map(|step| {
            let angle = step as f32 / 24.0 * std::f32::consts::TAU;
            [1.15f32, 1.7, 2.5].map(move |k| ((angle.cos() * span * k).round() as i32, (angle.sin() * span * k).round() as i32))
        })
        .collect();
    let valid = |dx: i32, dy: i32, x: usize, y: usize| -> Option<usize> {
        let (sx, sy) = (x as i32 + dx, y as i32 + dy);
        (sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h).then(|| sy as usize * w + sx as usize)
    };
    let best = candidates
        .par_iter()
        .filter_map(|(dx, dy)| {
            // Toute la zone source doit être de la matière saine : dans l'image, opaque, hors de la zone à corriger.
            for (x, y) in &zone {
                let i = valid(*dx, *dy, *x, *y)?;
                if cover[i] > 0.0 || image[i * 4 + 3] <= 0.0 {
                    return None;
                }
            }
            let mut cost = 0.0f64;
            for (x, y) in &ring {
                let i = valid(*dx, *dy, *x, *y)?;
                let o = (y * w + x) * 4;
                cost += (0..3).map(|c| f64::from(image[o + c] - image[i * 4 + c]).powi(2)).sum::<f64>();
            }
            Some((cost, (*dx, *dy)))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, offset)| offset)
        .ok_or_else(|| EngineError::InvalidOperation("aucune zone saine à proximité pour reconstruire celle-ci".into()))?;

    // Écart destination - source : connu sur le pourtour, diffusé dans la zone (relaxation de Laplace).
    let source_at = |x: usize, y: usize| valid(best.0, best.1, x, y).unwrap_or(y * w + x);
    let mut delta = vec![[0.0f32; 4]; w * h];
    let mut mean = [0.0f32; 4];
    for (x, y) in &ring {
        let (o, i) = ((y * w + x) * 4, source_at(*x, *y) * 4);
        let d = [0, 1, 2, 3].map(|c| image[o + c] - image[i + c]);
        delta[y * w + x] = d;
        for c in 0..4 {
            mean[c] += d[c] / ring.len() as f32;
        }
    }
    for (x, y) in &zone {
        delta[y * w + x] = mean; // point de départ : l'écart moyen, la relaxation fait le reste
    }
    // Hors zone, l'écart exact est connu partout : ce sont les conditions aux limites.
    for y in 0..h {
        for x in 0..w {
            if !inside(x, y) {
                let (o, i) = ((y * w + x) * 4, source_at(x, y) * 4);
                delta[y * w + x] = [0, 1, 2, 3].map(|c| image[o + c] - image[i + c]);
            }
        }
    }
    let sweeps = (span as usize * 3).clamp(60, 1200);
    for _ in 0..sweeps {
        for (x, y) in &zone {
            let (x, y) = (*x, *y);
            if x == 0 || y == 0 || x + 1 >= w || y + 1 >= h {
                continue;
            }
            let i = y * w + x;
            let around = [delta[i - 1], delta[i + 1], delta[i - w], delta[i + w]];
            // Sur-relaxation : converge bien plus vite qu'une simple moyenne des voisins.
            delta[i] = [0, 1, 2, 3].map(|c| delta[i][c] + 1.85 * ((around[0][c] + around[1][c] + around[2][c] + around[3][c]) / 4.0 - delta[i][c]));
        }
    }

    // Résultat : matière de la source + écart diffusé, fondu selon la couverture du pinceau.
    let mut out = image.clone();
    for (x, y) in &zone {
        let (o, i) = ((y * w + x) * 4, source_at(*x, *y) * 4);
        let k = cover[y * w + x];
        let alpha = (image[i + 3] + delta[y * w + x][3]).clamp(0.0, 255.0);
        for c in 0..4 {
            let healed = if c == 3 { alpha } else { (image[i + c] + delta[y * w + x][c]).clamp(0.0, alpha) };
            out[o + c] = image[o + c] + (healed - image[o + c]) * k;
        }
    }
    let bytes: Vec<u8> = out.iter().map(|v| (v + 0.5).clamp(0.0, 255.0) as u8).collect();
    let patch = TileStore::from_premultiplied(w as u32, h as u32, &bytes).ok_or_else(|| EngineError::Corrupt("tampon de correction de taille inattendue".into()))?;
    // Seule la fenêtre de travail est remplacée ; le reste du calque n'est pas touché (tuiles partagées).
    let mut result = pixels.clone();
    result.paste(&patch.translated(left, top), (mx0 - 1, my0 - 1, mx1 + 1, my1 + 1));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::{Sampler, TileCoord};

    /// Dégradé horizontal (de sombre à clair) à fines rayures verticales, avec une tache rouge de 30 px.
    fn blemished() -> (TileStore, MaskStore) {
        let (mut pixels, mut mask) = (TileStore::new(), MaskStore::with_background([0]));
        for y in 0..400 {
            for x in 0..600 {
                let v = (60 + x / 4 + if x % 4 < 2 { 12 } else { 0 }) as u8;
                let spot = ((x - 300) * (x - 300) + (y - 200) * (y - 200)) < 15 * 15;
                let (coord, px, py) = TileCoord::of_pixel(x, y);
                pixels.get_mut(coord).set_pixel(px, py, if spot { [230, 20, 20, 255] } else { [v, v, v, 255] });
                if ((x - 300) * (x - 300) + (y - 200) * (y - 200)) < 22 * 22 {
                    mask.get_mut(coord).set_pixel(px, py, [255]);
                }
            }
        }
        (pixels, mask)
    }

    #[test]
    fn the_blemish_disappears_and_the_patch_blends_in() {
        let (pixels, mask) = blemished();
        let healed = heal(&pixels, &mask).expect("correction");
        let mut sampler = Sampler::new(&healed);
        let expected = |x: i32| (60 + x / 4 + if x % 4 < 2 { 12 } else { 0 }) as i32;
        let mut worst = 0;
        for y in 185..215 {
            for x in 285..315 {
                let px = sampler.get(x, y);
                assert!(px[0].abs_diff(px[1]) <= 6 && px[0].abs_diff(px[2]) <= 6, "plus de rouge en ({x}, {y}) : {px:?}");
                worst = worst.max((i32::from(px[0]) - expected(x)).abs());
            }
        }
        // La matière (les rayures) est reprise d'à côté et la lumière suit le dégradé : l'écart à l'image idéale reste faible.
        assert!(worst <= 16, "écart maximal à l'image sans défaut : {worst}");
        assert_eq!(sampler.get(100, 100), Sampler::new(&pixels).get(100, 100), "rien ne change hors de la zone");
        assert_eq!(sampler.get(340, 200), Sampler::new(&pixels).get(340, 200));
    }

    #[test]
    fn an_empty_mask_changes_nothing_and_an_isolated_speck_is_refused() {
        let (pixels, _) = blemished();
        let untouched = heal(&pixels, &MaskStore::with_background([0])).expect("rien à faire");
        assert_eq!(untouched.to_premultiplied(600, 400), pixels.to_premultiplied(600, 400));

        // Un point seul sur du transparent : rien autour pour le reconstruire.
        let (mut speck, mut mask) = (TileStore::new(), MaskStore::with_background([0]));
        speck.get_mut(TileCoord::new(0, 0)).set_pixel(50, 50, [255, 255, 255, 255]);
        mask.get_mut(TileCoord::new(0, 0)).set_pixel(50, 50, [255]);
        assert!(heal(&speck, &mask).is_err());
    }
}
