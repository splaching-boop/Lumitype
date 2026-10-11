//! Calques de forme : rectangle (à coins arrondis) et ellipse, avec fond et contour.
//!
//! Comme pour le texte, le calque conserve sa description ([`ShapeData`]) ; ses pixels ne sont
//! qu'un cache, redessiné à chaque modification. La forme reste donc réglable indéfiniment :
//! taille, arrondi, couleurs, épaisseur du contour.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::pixel::premultiply;
use crate::tiles::TileStore;
use crate::transform::Matrix;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    #[default]
    Rect,
    Ellipse,
    /// Polygone régulier à `sides` côtés, pointe en haut, étiré pour remplir la boîte : 3 = triangle.
    Polygon,
}

/// Forme placée par la position de son calque (coin haut gauche de sa boîte).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeData {
    pub kind: ShapeKind,
    pub width: f32,
    pub height: f32,
    /// Rayon des coins d'un rectangle, en pixels ; limité à la moitié du plus petit côté.
    pub radius: f32,
    /// Couleur de fond RGBA à alpha droit ; `None` = pas de fond.
    pub fill: Option<[u8; 4]>,
    /// Couleur du contour ; `None` = pas de contour.
    pub stroke: Option<[u8; 4]>,
    /// Épaisseur du contour, tracé vers l'intérieur : la forme garde exactement sa taille.
    pub stroke_width: f32,
    /// Nombre de côtés d'un polygone (3 à 64) ; sans objet pour les autres formes.
    #[serde(default = "three")]
    pub sides: u32,
    /// Rotation, inclinaison ou perspective de la forme : envoie sa boîte `[0, largeur] x [0, hauteur]`
    /// dans le repère du calque. Identité pour une forme droite. La forme reste un vecteur : elle est
    /// redessinée exactement à chaque changement, sans rééchantillonnage. `normalized` garde la boîte
    /// englobante du dessin calée en (0, 0) : la position du calque reste le coin haut gauche visible.
    #[serde(default)]
    pub transform: Matrix,
}

impl ShapeData {
    /// Coins de la boîte de la forme, dans le repère du calque (après transformation).
    pub fn corners(&self) -> [(f64, f64); 4] {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| self.transform.apply(x, y).unwrap_or((x, y)))
    }

    /// Boîte englobante du dessin dans le repère du calque : `(x0, y0, x1, y1)` en pixels entiers.
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        if self.transform == Matrix::IDENTITY {
            return (0, 0, self.width.ceil() as i32, self.height.ceil() as i32);
        }
        let corners = self.corners();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (x, y) in corners {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        // Un pixel de marge : l'antialiasing déborde légèrement de la boîte exacte.
        ((x0 - 1.0).floor() as i32, (y0 - 1.0).floor() as i32, (x1 + 1.0).ceil() as i32, (y1 + 1.0).ceil() as i32)
    }

    /// La même forme, sa transformation décalée pour que le dessin commence en (0, 0) ; renvoie le
    /// décalage `(dx, dy)` à ajouter à la position du calque pour que rien ne bouge à l'écran.
    pub fn normalized(&self) -> (ShapeData, (i32, i32)) {
        if self.transform == Matrix::IDENTITY {
            return (self.clone(), (0, 0));
        }
        let (x0, y0, _, _) = self.bounds();
        let shifted = ShapeData { transform: Matrix::translation(-f64::from(x0), -f64::from(y0)).then_after(&self.transform), ..self.clone() };
        (shifted, (x0, y0))
    }
}

fn three() -> u32 {
    3
}

/// Sommets du polygone régulier, pointe en haut, mis à l'échelle pour toucher les quatre côtés de la boîte.
fn polygon_vertices(shape: &ShapeData) -> Vec<(f32, f32)> {
    let n = shape.sides.clamp(3, 64) as usize;
    let unit: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::TAU / n as f32;
            (angle.cos(), angle.sin())
        })
        .collect();
    let (min_x, max_x) = unit.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.0), hi.max(v.0)));
    let (min_y, max_y) = unit.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.1), hi.max(v.1)));
    unit.iter().map(|(x, y)| ((x - min_x) / (max_x - min_x) * shape.width, (y - min_y) / (max_y - min_y) * shape.height)).collect()
}

/// Polygone convexe rentré de `inset` pixels : chaque côté recule vers l'intérieur, les sommets sont recalculés.
fn inset_polygon(vertices: &[(f32, f32)], inset: f32) -> Vec<(f32, f32)> {
    let n = vertices.len();
    // Droite de chaque côté, décalée vers l'intérieur : point + direction.
    let lines: Vec<((f32, f32), (f32, f32))> = (0..n)
        .map(|i| {
            let (a, b) = (vertices[i], vertices[(i + 1) % n]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let length = dx.hypot(dy).max(1e-6);
            // Sommets dans le sens horaire à l'écran (y vers le bas) : l'intérieur est à droite du sens de parcours.
            let normal = (-dy / length, dx / length);
            ((a.0 + normal.0 * inset, a.1 + normal.1 * inset), (dx, dy))
        })
        .collect();
    (0..n)
        .map(|i| {
            let ((p, r), (q, s)) = (lines[(i + n - 1) % n], lines[i]);
            let cross = r.0 * s.1 - r.1 * s.0;
            if cross.abs() < 1e-6 {
                return q;
            }
            let t = ((q.0 - p.0) * s.1 - (q.1 - p.1) * s.0) / cross;
            (p.0 + r.0 * t, p.1 + r.1 * t)
        })
        .collect()
}

/// Distance signée à un polygone convexe (négative à l'intérieur), exacte y compris autour des sommets.
fn polygon_distance(vertices: &[(f32, f32)], x: f32, y: f32) -> f32 {
    let n = vertices.len();
    let (mut nearest, mut inside) = (f32::MAX, true);
    for i in 0..n {
        let (a, b) = (vertices[i], vertices[(i + 1) % n]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let (px, py) = (x - a.0, y - a.1);
        let t = ((px * dx + py * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
        nearest = nearest.min((px - dx * t).hypot(py - dy * t));
        if dx * py - dy * px < 0.0 {
            inside = false;
        }
    }
    if inside { -nearest } else { nearest }
}

/// Plus grand côté d'une forme, en pixels.
const MAX_SIDE: f32 = 16_384.0;

/// Distance signée au bord (négative à l'intérieur), en pixels.
fn distance(shape: &ShapeData, polygon: &[(f32, f32)], radius: f32, x: f32, y: f32) -> f32 {
    if shape.kind == ShapeKind::Polygon {
        // Coins arrondis : distance au polygone rentré de `radius`, diminuée de `radius`.
        return polygon_distance(polygon, x, y) - radius;
    }
    let (hw, hh) = (shape.width / 2.0, shape.height / 2.0);
    let (px, py) = ((x - hw).abs(), (y - hh).abs());
    match shape.kind {
        ShapeKind::Rect => {
            let r = shape.radius.clamp(0.0, hw.min(hh));
            let (qx, qy) = (px - (hw - r), py - (hh - r));
            qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
        }
        ShapeKind::Ellipse => {
            // Approximation au premier ordre : valeur implicite divisée par la norme de son gradient.
            let (nx, ny) = (px / hw, py / hh);
            let f = nx * nx + ny * ny - 1.0;
            let gradient = 2.0 * (nx / hw).hypot(ny / hh);
            if gradient < 1e-6 { -hw.min(hh) } else { f / gradient }
        }
        ShapeKind::Polygon => unreachable!("traité plus haut"),
    }
}

/// Dessine la forme en repère local : sa boîte occupe `[0, largeur] x [0, hauteur]`.
pub fn render(shape: &ShapeData) -> Result<TileStore, EngineError> {
    let valid = |v: f32| v.is_finite() && (1.0..=MAX_SIDE).contains(&v);
    if !valid(shape.width) || !valid(shape.height) {
        return Err(EngineError::InvalidOperation("taille de forme hors limites (1 à 16384 px)".into()));
    }
    // Forme transformée : chaque pixel du dessin est ramené dans la boîte droite par la transformation
    // inverse, où la distance au bord se calcule comme pour une forme droite ; la distance est remise à
    // l'échelle du dessin pour que l'antialiasing et le contour gardent leur épaisseur à l'écran.
    let (x0, y0, x1, y1) = shape.bounds();
    let (w, h) = ((x1 - x0).max(1) as usize, (y1 - y0).max(1) as usize);
    if (w as f32) > MAX_SIDE * 2.0 || (h as f32) > MAX_SIDE * 2.0 {
        return Err(EngineError::InvalidOperation("forme transformée trop grande".into()));
    }
    let inverse = if shape.transform == Matrix::IDENTITY { None } else { Some(shape.transform.inverse().ok_or_else(|| EngineError::InvalidOperation("transformation de forme impossible : forme aplatie".into()))?) };
    let scale = if inverse.is_some() { shape.transform.mean_scale().max(1e-3) as f32 } else { 1.0 };
    let stroke_width = if shape.stroke.is_some() { shape.stroke_width.clamp(0.0, shape.width.min(shape.height) / 2.0) } else { 0.0 };
    let straight = |c: Option<[u8; 4]>| c.map(|c| [f32::from(c[0]), f32::from(c[1]), f32::from(c[2]), f32::from(c[3]) / 255.0]);
    let (fill, stroke) = (straight(shape.fill), straight(shape.stroke));
    // Polygone : l'arrondi ne peut pas dépasser le rayon du cercle inscrit (au-delà, la forme rentrée se retournerait).
    let outline = if shape.kind == ShapeKind::Polygon { polygon_vertices(shape) } else { Vec::new() };
    let inscribed = if outline.is_empty() { 0.0 } else { -polygon_distance(&outline, shape.width / 2.0, shape.height * if shape.sides == 3 { 0.62 } else { 0.5 }) };
    let polygon_radius = shape.radius.clamp(0.0, (inscribed * 0.9).max(0.0));
    let polygon = if polygon_radius > 0.0 { inset_polygon(&outline, polygon_radius) } else { outline };
    let mut rgba = vec![0u8; w * h * 4];
    rgba.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5 + x0 as f32, y as f32 + 0.5 + y0 as f32);
            let d = match &inverse {
                None => distance(shape, &polygon, polygon_radius, px, py),
                Some(inverse) => match inverse.apply(f64::from(px), f64::from(py)) {
                    Some((lx, ly)) => distance(shape, &polygon, polygon_radius, lx as f32, ly as f32) * scale,
                    None => continue,
                },
            };
            // Couverture de la forme entière, puis de sa partie intérieure au contour (un pixel d'antialiasing).
            let outer = (0.5 - d).clamp(0.0, 1.0);
            if outer <= 0.0 {
                continue;
            }
            let inner = (0.5 - (d + stroke_width)).clamp(0.0, 1.0);
            let mut px = [0.0f32; 4]; // prémultiplié, 0..255 pour la couleur, 0..1 pour l'alpha
            for (color, coverage) in [(fill, if stroke_width > 0.0 { inner } else { outer }), (stroke, if stroke_width > 0.0 { outer - inner } else { 0.0 })] {
                let Some(c) = color else { continue };
                let a = c[3] * coverage;
                for i in 0..3 {
                    px[i] += c[i] * a;
                }
                px[3] += a;
            }
            let alpha = px[3].min(1.0);
            if alpha <= 0.0 {
                continue;
            }
            let out = premultiply([(px[0] / px[3] + 0.5) as u8, (px[1] / px[3] + 0.5) as u8, (px[2] / px[3] + 0.5) as u8, (alpha * 255.0 + 0.5) as u8]);
            row[x * 4..x * 4 + 4].copy_from_slice(&out);
        }
    });
    let store = TileStore::from_premultiplied(w as u32, h as u32, &rgba).ok_or_else(|| EngineError::Corrupt("tampon de forme de taille inattendue".into()))?;
    Ok(if (x0, y0) == (0, 0) { store } else { store.translated(x0, y0) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::Sampler;

    fn rect() -> ShapeData {
        ShapeData { kind: ShapeKind::Rect, width: 200.0, height: 100.0, radius: 0.0, fill: Some([255, 0, 0, 255]), stroke: None, stroke_width: 0.0, sides: 3, transform: Matrix::IDENTITY }
    }

    #[test]
    fn a_plain_rectangle_fills_its_box_exactly() {
        let store = render(&rect()).expect("rendu");
        assert_eq!(store.pixel_bounds(|px| px[3] > 0), Some((0, 0, 200, 100)));
        let mut sampler = Sampler::new(&store);
        assert_eq!((sampler.get(0, 0), sampler.get(199, 99), sampler.get(200, 50)), ([255, 0, 0, 255], [255, 0, 0, 255], [0; 4]));
    }

    #[test]
    fn a_rotated_rectangle_stays_a_vector_shape() {
        // Quart de tour autour de l'origine : la boîte 200 x 100 devient 100 x 200, en x négatif.
        let rotated = ShapeData { transform: Matrix([0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]), ..rect() };
        let (x0, y0, x1, y1) = rotated.bounds();
        assert!((x0, y0) <= (-100, 0) && (x1, y1) >= (0, 200), "boîte englobante : {:?}", rotated.bounds());
        let store = render(&rotated).expect("rendu");
        let mut sampler = Sampler::new(&store);
        assert_eq!(sampler.get(-50, 100), [255, 0, 0, 255], "au cœur de la forme tournée");
        assert_eq!(sampler.get(50, 100), [0; 4], "hors de la forme tournée");
        // Normalisée : le dessin commence en (0, 0) et le décalage rend la position du calque.
        let (normal, (dx, dy)) = rotated.normalized();
        assert!(dx <= -100 && dy <= 0);
        let bounds = render(&normal).expect("rendu").pixel_bounds(|px| px[3] > 0).expect("pixels");
        assert!(bounds.0 >= 0 && bounds.1 >= 0 && bounds.2 <= 102 && bounds.3 <= 202, "{bounds:?}");
        // Rotation de 45° : le bord reste net (antialiasing sur un pixel), le contour garde son épaisseur.
        let c = std::f64::consts::FRAC_1_SQRT_2;
        let tilted = ShapeData { stroke: Some([0, 0, 255, 255]), stroke_width: 10.0, transform: Matrix([c, -c, 300.0, c, c, 0.0, 0.0, 0.0, 1.0]), ..rect() };
        let (tilted, _) = tilted.normalized();
        let store = render(&tilted).expect("rendu");
        let (bx0, by0, bx1, by1) = store.pixel_bounds(|px| px[3] > 0).expect("pixels");
        let (width, height) = (bx1 - bx0, by1 - by0);
        assert!((width - 213).abs() <= 3 && (height - 213).abs() <= 3, "diagonale de 200 x 100 à 45° : {width} x {height}");
        let mut blues = 0;
        for y in by0..by1 {
            for x in bx0..bx1 {
                let px = Sampler::new(&store).get(x, y);
                if px[3] == 255 && px[2] > 200 && px[0] < 50 {
                    blues += 1;
                }
            }
        }
        // Contour de 10 px sur le périmètre 600 : environ 6 000 px pleins, moins les coins.
        assert!((4_500..7_000).contains(&blues), "contour épais tourné : {blues} px de bleu");
    }

    #[test]
    fn rounded_corners_are_cut_and_smooth() {
        let store = render(&ShapeData { radius: 40.0, ..rect() }).expect("rendu");
        let mut sampler = Sampler::new(&store);
        assert_eq!(sampler.get(2, 2)[3], 0, "le coin est arrondi");
        assert_eq!(sampler.get(40, 40)[3], 255, "centre du quart de cercle : plein");
        assert_eq!((sampler.get(100, 0)[3], sampler.get(0, 50)[3]), (255, 255), "les côtés droits touchent la boîte");
        let soft = (0..40).flat_map(|y| (0..40).map(move |x| (x, y))).filter(|(x, y)| (1..255).contains(&sampler.get(*x, *y)[3])).count();
        assert!(soft > 20, "bord lissé le long de l'arrondi : {soft} pixels partiels");
        // Un rayon démesuré est ramené à la moitié du petit côté : la forme devient une gélule.
        let capsule = render(&ShapeData { radius: 9999.0, ..rect() }).expect("rendu");
        assert_eq!((Sampler::new(&capsule).get(50, 50)[3], Sampler::new(&capsule).get(3, 3)[3]), (255, 0));
    }

    #[test]
    fn the_stroke_is_drawn_inside_in_its_own_colour() {
        let shape = ShapeData { stroke: Some([0, 0, 255, 255]), stroke_width: 10.0, ..rect() };
        let store = render(&shape).expect("rendu");
        let mut sampler = Sampler::new(&store);
        assert_eq!(store.pixel_bounds(|px| px[3] > 0), Some((0, 0, 200, 100)), "le contour ne fait pas grossir la forme");
        assert_eq!((sampler.get(4, 50), sampler.get(100, 4), sampler.get(100, 50)), ([0, 0, 255, 255], [0, 0, 255, 255], [255, 0, 0, 255]));
        // Contour seul : le centre est transparent.
        let hollow = render(&ShapeData { fill: None, ..shape }).expect("rendu");
        assert_eq!((Sampler::new(&hollow).get(100, 50)[3], Sampler::new(&hollow).get(4, 50)[3]), (0, 255));
    }

    #[test]
    fn polygons_fill_their_box_point_up_and_can_be_rounded() {
        let triangle = ShapeData { kind: ShapeKind::Polygon, sides: 3, width: 200.0, height: 160.0, ..rect() };
        let store = render(&triangle).expect("rendu");
        let mut sampler = Sampler::new(&store);
        assert_eq!(store.pixel_bounds(|px| px[3] > 0), Some((0, 0, 200, 160)), "le triangle touche les quatre côtés de sa boîte");
        assert_eq!((sampler.get(100, 80)[3], sampler.get(100, 150)[3]), (255, 255), "plein au centre et à la base");
        assert_eq!((sampler.get(10, 10)[3], sampler.get(190, 10)[3]), (0, 0), "vide de part et d'autre de la pointe");
        assert!(sampler.get(100, 3)[3] > 0 && sampler.get(5, 157)[3] > 0 && sampler.get(195, 157)[3] > 0, "pointe en haut, deux sommets en bas");

        // Hexagone : six côtés, plus large au milieu qu'en haut.
        let hexagon = render(&ShapeData { kind: ShapeKind::Polygon, sides: 6, width: 200.0, height: 200.0, ..rect() }).expect("rendu");
        let mut sampler = Sampler::new(&hexagon);
        assert_eq!((sampler.get(3, 100)[3], sampler.get(3, 20)[3], sampler.get(100, 3)[3]), (255, 0, 255));

        // Coins arrondis : les sommets sont coupés, les côtés restent en place.
        let rounded = render(&ShapeData { radius: 20.0, ..triangle.clone() }).expect("rendu");
        let mut sampler = Sampler::new(&rounded);
        assert_eq!((sampler.get(100, 3)[3], sampler.get(100, 157)[3], sampler.get(100, 80)[3]), (0, 255, 255), "la pointe est arrondie, la base est intacte");
        // Contour intérieur, comme pour les autres formes.
        let outlined = render(&ShapeData { stroke: Some([0, 0, 255, 255]), stroke_width: 8.0, ..triangle }).expect("rendu");
        assert_eq!((Sampler::new(&outlined).get(100, 156), Sampler::new(&outlined).get(100, 100)), ([0, 0, 255, 255], [255, 0, 0, 255]));
    }

    #[test]
    fn ellipses_touch_the_middle_of_each_side() {
        let store = render(&ShapeData { kind: ShapeKind::Ellipse, ..rect() }).expect("rendu");
        let mut sampler = Sampler::new(&store);
        assert!(sampler.get(100, 1)[3] > 200 && sampler.get(1, 50)[3] > 200 && sampler.get(100, 50)[3] == 255);
        assert_eq!(sampler.get(8, 8)[3], 0, "les coins de la boîte sont vides");
        assert!(render(&ShapeData { width: 0.0, ..rect() }).is_err());
    }
}
