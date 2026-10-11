//! Tracés vectoriels de la plume : courbes de Bézier cubiques, en pixels du document.
//!
//! Un tracé est une liste de sous-tracés ; chaque point d'ancrage porte ses deux poignées en
//! coordonnées absolues (une poignée confondue avec son point = pas de courbure de ce côté).
//! Le moteur ne fait que convertir : tracé -> sélection, fond ou contour. L'édition interactive
//! (poser, déplacer, convertir des points) vit dans l'interface, qui renvoie le tracé complet.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub point: [f32; 2],
    /// Poignée du côté du segment précédent.
    pub handle_in: [f32; 2],
    /// Poignée du côté du segment suivant.
    pub handle_out: [f32; 2],
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SubPath {
    pub anchors: Vec<Anchor>,
    pub closed: bool,
}

/// Point de la cubique `p0, p1, p2, p3` au paramètre `t`.
fn cubic(p: [[f32; 2]; 4], t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    [0, 1].map(|axis| (0..4).map(|i| p[i][axis] * w[i]).sum())
}

impl SubPath {
    /// Approximation polygonale. Le nombre de segments suit la longueur du polygone de contrôle :
    /// une grande courbe est découpée finement (écart bien inférieur au pixel), une petite ne coûte rien.
    pub fn flatten(&self) -> Vec<[f32; 2]> {
        let Some(first) = self.anchors.first() else { return Vec::new() };
        let mut points = vec![first.point];
        let count = self.anchors.len();
        let segments = if self.closed && count > 1 { count } else { count - 1 };
        for i in 0..segments {
            let (a, b) = (self.anchors[i], self.anchors[(i + 1) % count]);
            let control = [a.point, a.handle_out, b.handle_in, b.point];
            if a.handle_out == a.point && b.handle_in == b.point {
                points.push(b.point); // segment droit
                continue;
            }
            let length: f32 = control.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
            let steps = ((length / 3.0).ceil() as usize).clamp(8, 400);
            points.extend((1..=steps).map(|s| cubic(control, s as f32 / steps as f32)));
        }
        points
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corner(x: f32, y: f32) -> Anchor {
        Anchor { point: [x, y], handle_in: [x, y], handle_out: [x, y] }
    }

    #[test]
    fn straight_segments_are_not_subdivided_and_closing_returns_to_start() {
        let open = SubPath { anchors: vec![corner(0.0, 0.0), corner(10.0, 0.0), corner(10.0, 10.0)], closed: false };
        assert_eq!(open.flatten(), vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
        let closed = SubPath { closed: true, ..open };
        assert_eq!(closed.flatten().last(), Some(&[0.0, 0.0]));
        assert!(SubPath::default().flatten().is_empty());
    }

    #[test]
    fn curves_pass_through_anchors_and_bulge_towards_handles() {
        // Demi-cercle approché : de (0,0) à (100,0), poignées tirées vers le haut.
        let arc = SubPath {
            anchors: vec![
                Anchor { point: [0.0, 0.0], handle_in: [0.0, 0.0], handle_out: [0.0, -66.0] },
                Anchor { point: [100.0, 0.0], handle_in: [100.0, -66.0], handle_out: [100.0, 0.0] },
            ],
            closed: false,
        };
        let points = arc.flatten();
        assert_eq!((points[0], points[points.len() - 1]), ([0.0, 0.0], [100.0, 0.0]));
        let top = points.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert!((top + 49.5).abs() < 1.0, "sommet de la courbe : {top}");
        // Assez de segments pour que deux points consécutifs restent proches.
        assert!(points.windows(2).all(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]) < 6.0));
    }
}
