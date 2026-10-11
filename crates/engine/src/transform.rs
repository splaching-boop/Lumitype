//! Transformation libre : homographie 3x3 et rééchantillonnage d'un stockage en tuiles.
//!
//! Une seule représentation couvre translation, échelle, rotation, inclinaison et
//! déformation en perspective. Le rééchantillonnage parcourt les pixels de
//! **destination** et va chercher leur antécédent par la matrice inverse : aucun trou.
//!
//! Qualité :
//! - aperçu : bilinéaire, pour suivre la souris ;
//! - final : bicubique (Catmull-Rom) ; en réduction, plusieurs échantillons bilinéaires
//!   sont moyennés par pixel pour éviter le crénelage.
//!
//! Les calculs se font en alpha prémultiplié, ce qui évite les franges sombres aux bords.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::tiles::{Sampler, Store, TileBuf, TileCoord, MAX_DIMENSION, TILE_SIZE};

/// Matrice 3x3 en ligne : `[a, b, c, d, e, f, g, h, i]`, appliquée aux vecteurs colonne `(x, y, 1)`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Matrix(pub [f64; 9]);

impl Default for Matrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Matrix {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    pub fn scaling(factor: f64) -> Self {
        Self([factor, 0.0, 0.0, 0.0, factor, 0.0, 0.0, 0.0, 1.0])
    }

    /// Facteur d'agrandissement moyen autour de l'origine : racine du déterminant de la partie affine.
    pub fn mean_scale(&self) -> f64 {
        let m = &self.0;
        let w = if m[8].abs() > 1e-12 { m[8] } else { 1.0 };
        ((m[0] * m[4] - m[1] * m[3]) / (w * w)).abs().sqrt()
    }

    pub fn translation(dx: f64, dy: f64) -> Self {
        Self([1.0, 0.0, dx, 0.0, 1.0, dy, 0.0, 0.0, 1.0])
    }

    /// `self * other` : applique `other` d'abord, puis `self`.
    pub fn then_after(&self, other: &Self) -> Self {
        let (a, b) = (&self.0, &other.0);
        let mut out = [0.0; 9];
        for row in 0..3 {
            for col in 0..3 {
                out[row * 3 + col] = (0..3).map(|k| a[row * 3 + k] * b[k * 3 + col]).sum();
            }
        }
        Self(out)
    }

    /// Image d'un point, ou `None` s'il part à l'infini (derrière le plan de perspective).
    pub fn apply(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let m = &self.0;
        let w = m[6] * x + m[7] * y + m[8];
        (w > 1e-9).then(|| ((m[0] * x + m[1] * y + m[2]) / w, (m[3] * x + m[4] * y + m[5]) / w))
    }

    pub fn inverse(&self) -> Option<Self> {
        let m = &self.0;
        let cofactor = |a: usize, b: usize, c: usize, d: usize| m[a] * m[d] - m[b] * m[c];
        let adjugate = [
            cofactor(4, 5, 7, 8),
            -cofactor(1, 2, 7, 8),
            cofactor(1, 2, 4, 5),
            -cofactor(3, 5, 6, 8),
            cofactor(0, 2, 6, 8),
            -cofactor(0, 2, 3, 5),
            cofactor(3, 4, 6, 7),
            -cofactor(0, 1, 6, 7),
            cofactor(0, 1, 3, 4),
        ];
        let det = m[0] * adjugate[0] + m[1] * adjugate[3] + m[2] * adjugate[6];
        (det.is_finite() && det.abs() > 1e-12).then(|| Self(adjugate.map(|v| v / det)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Preview,
    Final,
}

/// Poids de Catmull-Rom pour une position fractionnaire `t` : voisins -1, 0, +1, +2.
fn catmull_rom(t: f64) -> [f64; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        -0.5 * t3 + t2 - 0.5 * t,
        1.5 * t3 - 2.5 * t2 + 1.0,
        -1.5 * t3 + 2.0 * t2 + 0.5 * t,
        0.5 * t3 - 0.5 * t2,
    ]
}

fn bilinear<const C: usize>(sampler: &mut Sampler<'_, C>, x: f64, y: f64) -> [f64; C] {
    // Les centres de pixels sont en (i + 0,5) : on se ramène à des indices entiers.
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);
    let corners = [sampler.get(ix, iy), sampler.get(ix + 1, iy), sampler.get(ix, iy + 1), sampler.get(ix + 1, iy + 1)];
    let weights = [(1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty];
    let mut out = [0.0; C];
    for (corner, weight) in corners.iter().zip(weights) {
        for c in 0..C {
            out[c] += f64::from(corner[c]) * weight;
        }
    }
    out
}

fn bicubic<const C: usize>(sampler: &mut Sampler<'_, C>, x: f64, y: f64) -> [f64; C] {
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (wx, wy) = (catmull_rom(fx - x0), catmull_rom(fy - y0));
    let mut out = [0.0; C];
    for (j, weight_y) in wy.iter().enumerate() {
        for (i, weight_x) in wx.iter().enumerate() {
            let px = sampler.get(x0 as i32 + i as i32 - 1, y0 as i32 + j as i32 - 1);
            for c in 0..C {
                out[c] += f64::from(px[c]) * weight_x * weight_y;
            }
        }
    }
    out
}

/// Transforme un stockage : `matrix` envoie les coordonnées de la source vers celles du résultat.
pub fn transform_store<const C: usize>(source: &Store<C>, matrix: &Matrix, quality: Quality) -> Result<Store<C>, EngineError> {
    let invalid = || EngineError::InvalidOperation("transformation impossible : forme aplatie ou retournée à l'infini".into());
    let inverse = matrix.inverse().ok_or_else(invalid)?;
    let mut out = Store::with_background(source.background());
    let Some((sx0, sy0, sx1, sy1)) = source.bounds() else { return Ok(out) };

    // Étendue du résultat : image des quatre coins de l'étendue source.
    let corners = [(sx0, sy0), (sx1, sy0), (sx1, sy1), (sx0, sy1)];
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in corners {
        let (tx, ty) = matrix.apply(f64::from(x), f64::from(y)).ok_or_else(invalid)?;
        (x0, y0, x1, y1) = (x0.min(tx), y0.min(ty), x1.max(tx), y1.max(ty));
    }
    let limit = f64::from(MAX_DIMENSION) * 2.0;
    if !(x1 - x0).is_finite() || x1 - x0 > limit || y1 - y0 > limit || x0.abs() > 1e7 || y0.abs() > 1e7 {
        return Err(EngineError::InvalidOperation("transformation impossible : résultat démesuré".into()));
    }
    let (first, _, _) = TileCoord::of_pixel(x0.floor() as i32, y0.floor() as i32);
    let (last, _, _) = TileCoord::of_pixel(x1.ceil() as i32, y1.ceil() as i32);
    let coords: Vec<TileCoord> =
        (first.y..=last.y).flat_map(|ty| (first.x..=last.x).map(move |tx| TileCoord::new(tx, ty))).collect();

    let size = TILE_SIZE as i32;
    let tiles: Vec<(TileCoord, TileBuf<C>)> = coords
        .into_par_iter()
        .filter_map(|coord| {
            let (ox, oy) = coord.origin();
            // Rejet rapide : si l'antécédent de la tuile ne touche pas la source, elle reste vide.
            let mut touches = false;
            let (mut bx0, mut by0, mut bx1, mut by1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for (x, y) in [(ox, oy), (ox + size, oy), (ox + size, oy + size), (ox, oy + size)] {
                match inverse.apply(f64::from(x), f64::from(y)) {
                    Some((px, py)) => (bx0, by0, bx1, by1) = (bx0.min(px), by0.min(py), bx1.max(px), by1.max(py)),
                    None => touches = true, // perspective extrême : pas de raccourci
                }
            }
            let margin = 2.0;
            touches |= bx0 - margin < f64::from(sx1) && bx1 + margin > f64::from(sx0) && by0 - margin < f64::from(sy1) && by1 + margin > f64::from(sy0);
            if !touches {
                return None;
            }

            // Pixels source couverts par un pixel de destination, estimés au centre de la tuile.
            let center = (f64::from(ox) + 128.0, f64::from(oy) + 128.0);
            let shrink = match (inverse.apply(center.0, center.1), inverse.apply(center.0 + 1.0, center.1), inverse.apply(center.0, center.1 + 1.0)) {
                (Some(o), Some(a), Some(b)) => ((a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)).abs().sqrt(),
                _ => 1.0,
            };
            let taps = if quality == Quality::Final && shrink > 1.5 { (shrink.ceil() as usize).min(4) } else { 1 };

            let mut sampler = Sampler::new(source);
            let mut tile = TileBuf::<C>::filled(source.background());
            for y in 0..size {
                for x in 0..size {
                    let mut sum = [0.0; C];
                    let mut count = 0.0;
                    for j in 0..taps {
                        for i in 0..taps {
                            let dx = f64::from(ox + x) + (i as f64 + 0.5) / taps as f64;
                            let dy = f64::from(oy + y) + (j as f64 + 0.5) / taps as f64;
                            let Some((px, py)) = inverse.apply(dx, dy) else { continue };
                            let sample = if quality == Quality::Final && taps == 1 { bicubic(&mut sampler, px, py) } else { bilinear(&mut sampler, px, py) };
                            for c in 0..C {
                                sum[c] += sample[c];
                            }
                            count += 1.0;
                        }
                    }
                    if count == 0.0 {
                        continue;
                    }
                    let mut px = [0u8; C];
                    for c in 0..C {
                        px[c] = (sum[c] / count + 0.5).clamp(0.0, 255.0) as u8;
                    }
                    // Le bicubique peut dépasser : en prémultiplié, une couleur ne doit jamais excéder son alpha.
                    if C == 4 {
                        let alpha = px[3];
                        for channel in px.iter_mut().take(3) {
                            *channel = (*channel).min(alpha);
                        }
                    }
                    tile.set_pixel(x as u32, y as u32, px);
                }
            }
            (!tile.is_uniform(source.background())).then_some((coord, tile))
        })
        .collect();
    for (coord, tile) in tiles {
        out.insert(coord, tile);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::TileStore;

    fn square() -> TileStore {
        let mut store = TileStore::new();
        store.fill(100, 100, [255, 0, 0, 255]);
        store
    }

    fn at(store: &TileStore, x: i32, y: i32) -> [u8; 4] {
        Sampler::new(store).get(x, y)
    }

    #[test]
    fn matrix_algebra() {
        let m = Matrix([2.0, 0.0, 10.0, 0.0, 3.0, -5.0, 0.0, 0.0, 1.0]);
        assert_eq!(m.apply(1.0, 1.0), Some((12.0, -2.0)));
        let back = m.inverse().expect("inversible").then_after(&m);
        for (value, expected) in back.0.iter().zip(Matrix::IDENTITY.0) {
            assert!((value - expected).abs() < 1e-12);
        }
        assert!(Matrix([1.0, 2.0, 0.0, 2.0, 4.0, 0.0, 0.0, 0.0, 1.0]).inverse().is_none(), "matrice singulière");
        let order = Matrix::translation(5.0, 0.0).then_after(&m);
        assert_eq!(order.apply(0.0, 0.0), Some((15.0, -5.0)), "m d'abord, translation ensuite");
    }

    #[test]
    fn identity_and_integer_translation_are_lossless() {
        let source = square();
        for quality in [Quality::Preview, Quality::Final] {
            let same = transform_store(&source, &Matrix::IDENTITY, quality).expect("identité");
            assert_eq!(same.to_premultiplied(100, 100), source.to_premultiplied(100, 100));
            let moved = transform_store(&source, &Matrix::translation(-300.0, 40.0), quality).expect("translation");
            assert_eq!((at(&moved, -300, 40), at(&moved, -201, 139), at(&moved, -200, 139)), ([255, 0, 0, 255], [255, 0, 0, 255], [0; 4]));
        }
    }

    #[test]
    fn scaling_up_and_down_keeps_a_solid_interior_and_clean_edges() {
        let source = square();
        let double = transform_store(&source, &Matrix([2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0]), Quality::Final).expect("x2");
        assert_eq!((at(&double, 100, 100), at(&double, 198, 198), at(&double, 201, 100)), ([255, 0, 0, 255], [255, 0, 0, 255], [0; 4]));
        let quarter = transform_store(&source, &Matrix([0.25, 0.0, 0.0, 0.0, 0.25, 0.0, 0.0, 0.0, 1.0]), Quality::Final).expect("x0,25");
        assert_eq!((at(&quarter, 12, 12), at(&quarter, 24, 24), at(&quarter, 26, 12)), ([255, 0, 0, 255], [255, 0, 0, 255], [0; 4]));
        for px in quarter.to_premultiplied(40, 40).chunks_exact(4) {
            assert!(px[0] <= px[3], "prémultiplié invalide : {px:?}");
        }
    }

    #[test]
    fn rotation_by_90_degrees_maps_corners() {
        // (x, y) -> (-y, x) : le carré [0,100)² devient (-100,0] x [0,100).
        let rotated = transform_store(&square(), &Matrix([0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]), Quality::Final).expect("rotation");
        assert_eq!((at(&rotated, -50, 50), at(&rotated, -99, 1), at(&rotated, 1, 50)), ([255, 0, 0, 255], [255, 0, 0, 255], [0; 4]));
    }

    #[test]
    fn perspective_and_degenerate_cases() {
        // Perspective douce : le résultat existe et reste borné.
        let tilted = transform_store(&square(), &Matrix([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.001, 0.0, 1.0]), Quality::Preview).expect("perspective");
        assert_eq!(at(&tilted, 40, 40), [255, 0, 0, 255]);
        assert!(transform_store(&square(), &Matrix([0.0; 9]), Quality::Final).is_err(), "matrice nulle refusée");
        assert!(transform_store(&square(), &Matrix([1e6, 0.0, 0.0, 0.0, 1e6, 0.0, 0.0, 0.0, 1.0]), Quality::Final).is_err(), "résultat démesuré refusé");
        assert_eq!(transform_store(&TileStore::new(), &Matrix::IDENTITY, Quality::Final).expect("vide").tile_count(), 0);
    }
}
