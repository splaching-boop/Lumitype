//! Bibliothèque de motifs : des motifs bicolores, raccordables à l'infini, dessinés par le code.
//!
//! Aucun fichier d'image : chaque motif est une forme définie sur une **cellule** (sa période, en
//! pixels à 100 %), décrite par sa distance signée au bord du dessin (négative à l'intérieur). La
//! couverture d'un pixel s'en déduit avec un antialiasing exact à toute échelle, et la cellule se
//! répète sans couture dans les deux directions.
//!
//! Le module ne connaît ni calques ni documents : il répond à « quelle part du motif couvre ce point
//! du plan ? » (`Pattern::coverage`) et sait remplir un rectangle de pixels (`render_rgba`). Il sert au
//! calque de remplissage Motif (`crate::fill`) et pourra servir tel quel à un effet de calque
//! « Incrustation de motif » : même identifiant, même échelle, mêmes couleurs.

use serde::Serialize;

/// Dessin d'une cellule : distance signée au bord du motif (en pixels de la cellule, négative à
/// l'intérieur), ou grain aléatoire (une valeur par pixel de la cellule).
#[derive(Clone, Copy)]
enum Drawing {
    Distance(fn(f32, f32) -> f32),
    Grain,
}

/// Un motif de la bibliothèque.
pub struct Pattern {
    /// Identifiant stable, enregistré dans les fichiers : ne jamais le changer.
    pub id: &'static str,
    /// Nom affiché.
    pub name: &'static str,
    /// Taille de la cellule à 100 %, en pixels.
    pub width: f32,
    pub height: f32,
    drawing: Drawing,
}

/// Description d'un motif pour l'interface.
#[derive(Clone, Debug, Serialize)]
pub struct PatternInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub width: f32,
    pub height: f32,
}

const SQRT_2: f32 = std::f32::consts::SQRT_2;

/// Distance (signée, 0 au bord) à une ligne de période `period` passant par 0, d'épaisseur `width`.
fn line(position: f32, period: f32, width: f32) -> f32 {
    let p = position.rem_euclid(period);
    p.min(period - p) - width / 2.0
}

fn points(x: f32, y: f32) -> f32 {
    ((x - 5.0).powi(2) + (y - 5.0).powi(2)).sqrt() - 1.6
}

fn polka(x: f32, y: f32) -> f32 {
    let a = ((x - 6.0).powi(2) + (y - 6.0).powi(2)).sqrt();
    let b = ((x - 18.0).powi(2) + (y - 18.0).powi(2)).sqrt();
    a.min(b) - 4.5
}

fn grid(x: f32, y: f32) -> f32 {
    line(x, 16.0, 2.0).min(line(y, 16.0, 2.0))
}

fn stripes(_x: f32, y: f32) -> f32 {
    (y - 6.0).abs() - 2.5
}

fn diagonals(x: f32, y: f32) -> f32 {
    let s = (x + y).rem_euclid(12.0);
    (s - 6.0).abs() / SQRT_2 - 1.1
}

fn crosshatch(x: f32, y: f32) -> f32 {
    let a = ((x + y).rem_euclid(12.0) - 6.0).abs() / SQRT_2;
    let b = ((x - y).rem_euclid(12.0) - 6.0).abs() / SQRT_2;
    a.min(b) - 0.8
}

fn checker(x: f32, y: f32) -> f32 {
    let (cx, cy) = (x.rem_euclid(8.0), y.rem_euclid(8.0));
    let edge = cx.min(8.0 - cx).min(cy).min(8.0 - cy);
    let inside = ((x / 8.0).floor() as i32 + (y / 8.0).floor() as i32).rem_euclid(2) == 0;
    if inside { -edge } else { edge }
}

fn chevrons(x: f32, y: f32) -> f32 {
    let t = (x.rem_euclid(16.0) - 8.0).abs();
    let s = (y + t).rem_euclid(16.0);
    (s - 8.0).abs() / SQRT_2 - 1.6
}

fn bricks(x: f32, y: f32) -> f32 {
    let row = (y / 8.0).floor() as i32;
    let shift = if row.rem_euclid(2) == 1 { 12.0 } else { 0.0 };
    line(x + shift, 24.0, 2.0).min(line(y, 8.0, 2.0))
}

/// Nid d'abeilles : hexagones à pointe en haut, de rayon 9, dont on trace les arêtes.
fn hexagons(x: f32, y: f32) -> f32 {
    const R: f32 = 9.0;
    let r = R * 3f32.sqrt() / 2.0;
    let (w, h) = (2.0 * r, 3.0 * R);
    let (x, y) = (x.rem_euclid(w), y.rem_euclid(h));
    let centers = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h), (r, 1.5 * R)];
    let n = centers
        .iter()
        .map(|(cx, cy)| {
            let (qx, qy) = ((x - cx).abs(), (y - cy).abs());
            qx.max(0.5 * qx + 0.866_025_4 * qy)
        })
        .fold(f32::INFINITY, f32::min);
    (n - r).abs() - 0.8
}

fn waves(x: f32, y: f32) -> f32 {
    let k = std::f32::consts::TAU / 24.0;
    let s = (y - 2.5 * (k * x).sin()).rem_euclid(12.0);
    let slope = 2.5 * k * (k * x).cos();
    (s - 6.0).abs() / (1.0 + slope * slope).sqrt() - 1.3
}

/// Valeur pseudo-aléatoire reproductible dans [0, 1) pour un pixel de cellule.
fn hash(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Les motifs livrés, dans l'ordre de la grille de choix.
pub const PATTERNS: &[Pattern] = &[
    Pattern { id: "points", name: "Points", width: 10.0, height: 10.0, drawing: Drawing::Distance(points) },
    Pattern { id: "pois", name: "Pois", width: 24.0, height: 24.0, drawing: Drawing::Distance(polka) },
    Pattern { id: "quadrillage", name: "Quadrillage", width: 16.0, height: 16.0, drawing: Drawing::Distance(grid) },
    Pattern { id: "rayures", name: "Rayures", width: 12.0, height: 12.0, drawing: Drawing::Distance(stripes) },
    Pattern { id: "diagonales", name: "Diagonales", width: 12.0, height: 12.0, drawing: Drawing::Distance(diagonals) },
    Pattern { id: "croisillons", name: "Croisillons", width: 12.0, height: 12.0, drawing: Drawing::Distance(crosshatch) },
    Pattern { id: "damier", name: "Damier", width: 16.0, height: 16.0, drawing: Drawing::Distance(checker) },
    Pattern { id: "chevrons", name: "Chevrons", width: 16.0, height: 16.0, drawing: Drawing::Distance(chevrons) },
    Pattern { id: "briques", name: "Briques", width: 24.0, height: 16.0, drawing: Drawing::Distance(bricks) },
    Pattern { id: "hexagones", name: "Hexagones", width: 15.588_457, height: 27.0, drawing: Drawing::Distance(hexagons) },
    Pattern { id: "vagues", name: "Vagues", width: 24.0, height: 12.0, drawing: Drawing::Distance(waves) },
    Pattern { id: "grain", name: "Grain", width: 64.0, height: 64.0, drawing: Drawing::Grain },
];

/// Motif d'identifiant `id`.
pub fn find(id: &str) -> Option<&'static Pattern> {
    PATTERNS.iter().find(|p| p.id == id)
}

/// Le catalogue, pour l'interface.
pub fn catalog() -> Vec<PatternInfo> {
    PATTERNS.iter().map(|p| PatternInfo { id: p.id, name: p.name, width: p.width, height: p.height }).collect()
}

impl Pattern {
    /// Couverture du motif (0 : fond, 1 : motif) sur le pixel dont le coin haut gauche est `(x, y)`
    /// dans le plan, le motif étant posé à l'échelle `scale` (1 = 100 %) avec l'origine d'une
    /// cellule en `(0, 0)`. Antialiasé ; réduit sous 100 %, le pixel est suréchantillonné.
    pub fn coverage(&self, x: f32, y: f32, scale: f32) -> f32 {
        let scale = scale.max(0.01);
        // Sous 100 %, une cellule tient en peu de pixels : plusieurs échantillons par pixel.
        let n = if scale >= 1.0 { 1 } else { (1.0 / scale).ceil().min(4.0) as usize };
        let mut total = 0.0;
        for j in 0..n {
            for i in 0..n {
                let (sx, sy) = (x + (i as f32 + 0.5) / n as f32, y + (j as f32 + 0.5) / n as f32);
                total += self.sample(sx / scale, sy / scale, scale * n as f32);
            }
        }
        total / (n * n) as f32
    }

    /// Couverture au point `(u, v)` de la cellule (en pixels de cellule), un pixel de sortie valant
    /// `1 / pixel` pixel de cellule.
    fn sample(&self, u: f32, v: f32, pixel: f32) -> f32 {
        let (u, v) = (u.rem_euclid(self.width), v.rem_euclid(self.height));
        match self.drawing {
            Drawing::Distance(distance) => (0.5 - distance(u, v) * pixel).clamp(0.0, 1.0),
            Drawing::Grain => {
                if hash(u.floor() as i32, v.floor() as i32) < 0.38 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// Remplit un rectangle du plan avec le motif : `(x0, y0)` coin haut gauche, `width` × `height` pixels,
/// en RGBA à alpha droit, ligne par ligne. `color` est la couleur du motif, `background` celle du fond
/// (RGBA à alpha droit tous deux, mélangés selon la couverture).
pub fn render_rgba(pattern: &Pattern, scale: f32, color: [u8; 4], background: [u8; 4], (x0, y0): (i32, i32), width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0u8; width as usize * height as usize * 4];
    for (row, line) in out.chunks_exact_mut(width as usize * 4).enumerate() {
        for (col, px) in line.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&mix(color, background, pattern.coverage((x0 + col as i32) as f32, (y0 + row as i32) as f32, scale)));
        }
    }
    out
}

/// Mélange de deux couleurs RGBA droites selon `k` (1 : `color`).
pub fn mix(color: [u8; 4], background: [u8; 4], k: f32) -> [u8; 4] {
    [0, 1, 2, 3].map(|c| (f32::from(background[c]) + (f32::from(color[c]) - f32::from(background[c])) * k + 0.5) as u8)
}

/// Vignette carrée de `size` pixels d'un motif (RGBA à alpha droit), noir sur blanc, à l'échelle
/// `scale` ; `None` si le motif n'existe pas.
pub fn swatch(id: &str, size: u32, scale: f32) -> Option<Vec<u8>> {
    let pattern = find(id)?;
    Some(render_rgba(pattern, scale, [0, 0, 0, 255], [255, 255, 255, 255], (0, 0), size, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_is_two_toned_and_seamless() {
        let mut ids = std::collections::HashSet::new();
        for pattern in PATTERNS {
            assert!(ids.insert(pattern.id), "identifiant en double : {}", pattern.id);
            // Le motif et le fond sont tous deux présents sur quelques cellules.
            let cover: Vec<f32> = (0..64 * 64).map(|i| pattern.coverage((i % 64) as f32, (i / 64) as f32, 1.0)).collect();
            assert!(cover.iter().any(|c| *c > 0.9) && cover.iter().any(|c| *c < 0.1), "{} : deux tons", pattern.id);
            // Raccord : une cellule plus loin, le même dessin (au flottant près).
            for (x, y) in [(0.3f32, 0.7f32), (3.2, 9.1), (7.5, 2.5)] {
                let here = pattern.sample(x, y, 1.0);
                let there = pattern.sample(x + pattern.width * 3.0, y + pattern.height * 2.0, 1.0);
                assert!((here - there).abs() < 0.02, "{} : raccord en ({x}, {y}) : {here} / {there}", pattern.id);
            }
        }
        assert_eq!(PATTERNS.len(), 12);
    }

    #[test]
    fn scale_enlarges_the_cell() {
        let stripes = find("rayures").expect("rayures");
        // À 100 % la bande est centrée sur y = 6 ; à 200 %, sur y = 12.
        assert!(stripes.coverage(0.0, 5.5, 1.0) > 0.9 && stripes.coverage(0.0, 11.5, 2.0) > 0.9);
        assert!(stripes.coverage(0.0, 0.0, 1.0) < 0.1 && stripes.coverage(0.0, 1.0, 2.0) < 0.1);
        let thumb = swatch("damier", 16, 1.0).expect("vignette");
        assert_eq!(thumb.len(), 16 * 16 * 4);
        assert!(swatch("inconnu", 8, 1.0).is_none());
    }
}
