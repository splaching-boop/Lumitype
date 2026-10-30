//! Calques de remplissage (Photoshop : Calque > Nouveau calque de remplissage) : une couleur unie, un
//! dégradé ou un motif qui couvre toute la zone de travail et reste modifiable.
//!
//! Le calque (`LayerKind::Fill`) garde la description du remplissage et un rendu en cache, régénéré
//! dès que la description, la position du calque ou la taille du document change : composition,
//! export et carte graphique n'y voient qu'un calque de pixels ordinaire.
//!
//! Le remplissage est défini dans le repère du calque, sur la « boîte » `(0, 0)`–`(largeur, hauteur)`
//! du document : déplacer le calque déplace donc le dégradé ou le motif (comme Photoshop), tandis que
//! le rendu, lui, couvre toujours exactement le document.

use std::sync::Arc;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::adjust::gradient_map_table;
use crate::document::GradientStop;
use crate::error::EngineError;
use crate::patterns;
use crate::pixel::premultiply;
use crate::tiles::{Tile, TileCoord, TileStore, TILE_SIZE};

/// Style d'un dégradé de remplissage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GradientStyle {
    #[default]
    Linear,
    Radial,
    Angle,
    Reflected,
    Diamond,
}

/// Ce qui remplit le calque.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FillContent {
    /// Couleur unie (RVB).
    SolidColor { color: [u8; 3] },
    /// Fond en dégradé : arrêts (RGBA à alpha droit), style, angle en degrés (sens trigonométrique,
    /// 90 = de bas en haut), échelle en % (10 à 150), sens inversé, et décalage du centre en pixels
    /// par rapport au centre du document (glisser dans l'image pendant le réglage).
    #[serde(rename_all = "camelCase")]
    Gradient {
        stops: Vec<GradientStop>,
        #[serde(default)]
        style: GradientStyle,
        #[serde(default = "default_angle")]
        angle: f32,
        #[serde(default = "hundred")]
        scale: f32,
        #[serde(default)]
        reverse: bool,
        #[serde(default)]
        offset: [f32; 2],
    },
    /// Motif de la bibliothèque (`crate::patterns`) : identifiant, échelle en % (10 à 1000), couleur du
    /// motif et couleur du fond.
    #[serde(rename_all = "camelCase")]
    Pattern {
        pattern: String,
        #[serde(default = "hundred")]
        scale: f32,
        #[serde(default)]
        color: [u8; 3],
        #[serde(default = "white")]
        background: [u8; 3],
    },
}

fn default_angle() -> f32 {
    90.0
}

fn hundred() -> f32 {
    100.0
}

fn white() -> [u8; 3] {
    [255, 255, 255]
}

impl FillContent {
    /// Nom donné au nouveau calque (suivi d'un numéro).
    pub fn label(&self) -> &'static str {
        match self {
            Self::SolidColor { .. } => "Couleur unie",
            Self::Gradient { .. } => "Dégradé",
            Self::Pattern { .. } => "Motif",
        }
    }

    /// Refuse une description inutilisable (motif inconnu, valeurs non finies).
    pub fn validate(&self) -> Result<(), EngineError> {
        match self {
            Self::SolidColor { .. } => Ok(()),
            Self::Gradient { angle, scale, offset, .. } => {
                if [*angle, *scale, offset[0], offset[1]].iter().all(|v| v.is_finite()) {
                    Ok(())
                } else {
                    Err(EngineError::InvalidOperation("dégradé de remplissage invalide".into()))
                }
            }
            Self::Pattern { pattern, scale, .. } => {
                if patterns::find(pattern).is_none() {
                    return Err(EngineError::InvalidOperation(format!("motif inconnu : {pattern}")));
                }
                if !scale.is_finite() {
                    return Err(EngineError::InvalidOperation("échelle de motif invalide".into()));
                }
                Ok(())
            }
        }
    }
}

/// Rendu du remplissage sur le rectangle `(x, y, largeur, hauteur)` du repère du calque (prémultiplié),
/// pour une boîte (le document) de `size` pixels posée à l'origine de ce repère.
pub fn render(fill: &FillContent, size: (u32, u32), rect: (i32, i32, u32, u32)) -> Result<TileStore, EngineError> {
    render_reusing(fill, size, rect, None)
}

/// Comme [`render`], en reprenant de `previous` (le rendu du même remplissage sur un autre rectangle du même repère)
/// les tuiles entièrement couvertes par les deux rectangles : le remplissage ne dépend que de la position dans le
/// repère du calque, ces tuiles sont donc identiques. Un calque de remplissage déplacé ne recalcule ainsi que ses
/// tuiles de bord, et les autres restent les mêmes (même mémoire, rien à recomposer ni à recharger pour elles).
/// Rectangle `(x, y, largeur, hauteur)` en pixels du document.
pub type Rect = (i32, i32, u32, u32);

pub fn render_reusing(fill: &FillContent, size: (u32, u32), rect: Rect, previous: Option<(&TileStore, Rect)>) -> Result<TileStore, EngineError> {
    let reuse = |coord: TileCoord| -> Option<Arc<Tile>> {
        let (store, before) = previous?;
        let inside = |(x, y, w, h): (i32, i32, u32, u32)| {
            let (ox, oy) = coord.origin();
            let size = TILE_SIZE as i32;
            ox >= x && oy >= y && ox + size <= x + w as i32 && oy + size <= y + h as i32
        };
        if inside(rect) && inside(before) { store.get(coord).cloned() } else { None }
    };
    fill.validate()?;
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    match fill {
        FillContent::SolidColor { color } => {
            let px = premultiply([color[0], color[1], color[2], 255]);
            Ok(paint(rect, |_, _| px, reuse))
        }
        FillContent::Gradient { stops, style, angle, scale, reverse, offset } => {
            let table = gradient_map_table(stops, *reverse, 0.0);
            let geometry = GradientGeometry::new(*style, *angle, *scale, (w, h), *offset);
            Ok(paint(rect, |x, y| {
                let position = geometry.position(x, y) * 255.0;
                let index = (position as usize).min(255);
                let next = (index + 1).min(255);
                let k = position - index as f32;
                let channel = |c: usize| {
                    let v = table[index][c] + (table[next][c] - table[index][c]) * k;
                    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
                };
                premultiply([channel(0), channel(1), channel(2), channel(3)])
            }, reuse))
        }
        FillContent::Pattern { pattern, scale, color, background } => {
            let pattern = patterns::find(pattern).ok_or_else(|| EngineError::InvalidOperation("motif inconnu".into()))?;
            let scale = scale.clamp(10.0, 1000.0) / 100.0;
            let (fg, bg) = ([color[0], color[1], color[2], 255], [background[0], background[1], background[2], 255]);
            Ok(paint(rect, |x, y| premultiply(patterns::mix(fg, bg, pattern.coverage(x, y, scale))), reuse))
        }
    }
}

/// Géométrie d'un dégradé de remplissage : où se trouve un pixel le long du dégradé (0 à 1).
pub struct GradientGeometry {
    style: GradientStyle,
    center: (f32, f32),
    /// Direction du dégradé (vers la fin) et sa perpendiculaire, en repère image (y vers le bas).
    direction: (f32, f32),
    across: (f32, f32),
    angle: f32,
    /// Longueur du dégradé linéaire ; la moitié pour les styles partant du centre.
    length: f32,
}

impl GradientGeometry {
    /// `angle` en degrés (sens trigonométrique à l'écran), `scale` en %, `size` la boîte, `offset` le
    /// décalage du centre. La longueur à 100 % est l'étendue de la boîte dans la direction du dégradé :
    /// un dégradé linéaire va d'un bord à l'autre, quel que soit l'angle.
    pub fn new(style: GradientStyle, angle: f32, scale: f32, (w, h): (f32, f32), offset: [f32; 2]) -> Self {
        let radians = angle.to_radians();
        let (sin, cos) = radians.sin_cos();
        let span = (w * cos).abs() + (h * sin).abs();
        let length = (span * scale.clamp(10.0, 150.0) / 100.0).max(1e-3);
        Self { style, center: (w / 2.0 + offset[0], h / 2.0 + offset[1]), direction: (cos, -sin), across: (sin, cos), angle: radians, length }
    }

    /// Position (0 à 1) du pixel dont le coin haut gauche est `(x, y)`.
    pub fn position(&self, x: f32, y: f32) -> f32 {
        let (px, py) = (x + 0.5 - self.center.0, y + 0.5 - self.center.1);
        let along = px * self.direction.0 + py * self.direction.1;
        let half = self.length / 2.0;
        let t = match self.style {
            GradientStyle::Linear => along / self.length + 0.5,
            GradientStyle::Reflected => along.abs() / half,
            GradientStyle::Radial => (px * px + py * py).sqrt() / half,
            GradientStyle::Diamond => (along.abs() + (px * self.across.0 + py * self.across.1).abs()) / half,
            GradientStyle::Angle => {
                // Angle du point à l'écran (y vers le haut), compté depuis la direction du dégradé.
                let theta = (-py).atan2(px);
                ((theta - self.angle) / std::f32::consts::TAU).rem_euclid(1.0)
            }
        };
        t.clamp(0.0, 1.0)
    }
}

/// Tuiles couvrant exactement le rectangle `(x, y, largeur, hauteur)`, chaque pixel valant `color(x, y)`
/// (RGBA prémultiplié, `(x, y)` son coin haut gauche). Calculées en parallèle ; celles que `reuse` fournit sont
/// reprises telles quelles (voir [`render_reusing`]).
fn paint(rect: (i32, i32, u32, u32), color: impl Fn(f32, f32) -> [u8; 4] + Sync, reuse: impl Fn(TileCoord) -> Option<Arc<Tile>> + Sync) -> TileStore {
    let (x0, y0, w, h) = rect;
    let mut store = TileStore::new();
    if w == 0 || h == 0 {
        return store;
    }
    let (x1, y1) = (x0 + w as i32, y0 + h as i32);
    let (first, _, _) = TileCoord::of_pixel(x0, y0);
    let (last, _, _) = TileCoord::of_pixel(x1 - 1, y1 - 1);
    let coords: Vec<TileCoord> = (first.y..=last.y).flat_map(|ty| (first.x..=last.x).map(move |tx| TileCoord::new(tx, ty))).collect();
    let size = TILE_SIZE as i32;
    let tiles: Vec<(TileCoord, Option<Arc<Tile>>)> = coords
        .into_par_iter()
        .map(|coord| {
            if let Some(tile) = reuse(coord) {
                return (coord, Some(tile));
            }
            let (ox, oy) = coord.origin();
            let mut tile = Tile::transparent();
            let bytes = tile.bytes_mut();
            for y in oy.max(y0)..(oy + size).min(y1) {
                for x in ox.max(x0)..(ox + size).min(x1) {
                    let at = (((y - oy) * size + (x - ox)) * 4) as usize;
                    bytes[at..at + 4].copy_from_slice(&color(x as f32, y as f32));
                }
            }
            // Tuile entièrement transparente : jamais stockée (comme `TileStore::insert`).
            (coord, (!tile.is_transparent()).then(|| Arc::new(tile)))
        })
        .collect();
    for (coord, tile) in tiles {
        if tile.is_some() {
            store.restore(coord, tile);
        }
    }
    store
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(position: f32, color: [u8; 4]) -> GradientStop {
        GradientStop { position, color }
    }

    fn pixel(store: &TileStore, x: i32, y: i32) -> [u8; 4] {
        let (coord, px, py) = TileCoord::of_pixel(x, y);
        store.get(coord).map_or([0; 4], |t| t.pixel(px, py))
    }

    #[test]
    fn a_solid_colour_covers_exactly_the_rectangle() {
        let store = render(&FillContent::SolidColor { color: [10, 200, 30] }, (300, 200), (-20, -10, 300, 200)).expect("rendu");
        assert_eq!(pixel(&store, -20, -10), [10, 200, 30, 255]);
        assert_eq!(pixel(&store, 279, 189), [10, 200, 30, 255]);
        assert_eq!(pixel(&store, 280, 0)[3], 0, "rien au-delà du rectangle");
        assert_eq!(pixel(&store, -21, 0)[3], 0);
    }

    #[test]
    fn a_linear_gradient_runs_edge_to_edge_along_its_angle() {
        let fill = |angle, reverse, style| FillContent::Gradient {
            stops: vec![stop(0.0, [0, 0, 0, 255]), stop(1.0, [255, 255, 255, 255])],
            style,
            angle,
            scale: 100.0,
            reverse,
            offset: [0.0, 0.0],
        };
        // 90° : du noir en bas au blanc en haut.
        let up = render(&fill(90.0, false, GradientStyle::Linear), (100, 100), (0, 0, 100, 100)).expect("rendu");
        assert!(pixel(&up, 50, 0)[0] > 250 && pixel(&up, 50, 99)[0] < 5);
        assert!((i32::from(pixel(&up, 50, 50)[0]) - 128).abs() < 4, "milieu gris");
        // 0° : de gauche à droite ; inversé : de droite à gauche.
        let right = render(&fill(0.0, false, GradientStyle::Linear), (100, 100), (0, 0, 100, 100)).expect("rendu");
        assert!(pixel(&right, 0, 50)[0] < 5 && pixel(&right, 99, 50)[0] > 250);
        let reversed = render(&fill(0.0, true, GradientStyle::Linear), (100, 100), (0, 0, 100, 100)).expect("rendu");
        assert!(pixel(&reversed, 0, 50)[0] > 250);
        // Radial : le centre prend le premier arrêt ; réfléchi : symétrique.
        let radial = render(&fill(90.0, false, GradientStyle::Radial), (100, 100), (0, 0, 100, 100)).expect("rendu");
        assert!(pixel(&radial, 50, 50)[0] < 5 && pixel(&radial, 50, 0)[0] > 250);
        let reflected = render(&fill(0.0, false, GradientStyle::Reflected), (100, 100), (0, 0, 100, 100)).expect("rendu");
        assert_eq!(pixel(&reflected, 10, 3), pixel(&reflected, 89, 3));
        for style in [GradientStyle::Angle, GradientStyle::Diamond] {
            let store = render(&fill(30.0, false, style), (64, 48), (0, 0, 64, 48)).expect("rendu");
            assert_eq!(pixel(&store, 63, 47)[3], 255);
        }
    }

    #[test]
    fn a_fill_layer_covers_the_document_and_stays_editable() {
        use crate::document::{Background, Document};
        use crate::render::TileRenderer;
        let mut doc = Document::with_background("x", 300, 200, 72.0, Background::White).expect("document");
        let id = doc.add_fill_layer(FillContent::SolidColor { color: [255, 0, 0] }, "Couleur unie 1", None, 1).expect("remplissage");
        let layer = doc.layer(id).expect("calque");
        assert!(layer.mask.as_ref().is_some_and(|m| m.pixels.background() == [255]), "masque blanc : tout visible");
        assert_eq!(pixel(layer.pixels().expect("rendu"), 299, 199), [255, 0, 0, 255]);
        let composed = TileRenderer::new().tile(&doc, 0, TileCoord::new(0, 0)).expect("tuile");
        assert_eq!(composed.pixel(10, 10), [255, 0, 0, 255]);

        // Modifiable : un autre remplissage, d'un autre type.
        doc.set_fill(id, FillContent::Pattern { pattern: "rayures".into(), scale: 100.0, color: [0, 0, 0], background: [255, 255, 255] }).expect("motif");
        assert!(matches!(doc.layer(id).expect("calque").fill_content(), Some(FillContent::Pattern { .. })));

        // Non peignable, mais pixellisable ; un filtre le pixellise d'abord.
        assert!(doc.pixels_mut(id).is_err() && doc.fill_layer(id, [0, 0, 0]).is_err());

        // Déplacé : le rendu couvre toujours le document.
        doc.set_fill(id, FillContent::SolidColor { color: [0, 255, 0] }).expect("couleur");
        doc.translate_layers(&[id], 40, -25).expect("déplacement");
        let layer = doc.layer(id).expect("calque");
        assert_eq!(pixel(layer.pixels().expect("rendu"), -40, 25 + 199), [0, 255, 0, 255], "coin bas gauche du document couvert");
        assert_eq!(pixel(layer.pixels().expect("rendu"), -41, 30)[3], 0);

        // Zone de travail agrandie : le remplissage la couvre entière, position remise à zéro.
        doc.crop(-50, -50, 400, 300).expect("agrandissement");
        let layer = doc.layer(id).expect("calque");
        assert_eq!(layer.offset, (0, 0));
        assert_eq!(pixel(layer.pixels().expect("rendu"), 399, 299), [0, 255, 0, 255]);

        doc.rasterize_layer(id).expect("pixellisé");
        assert!(doc.layer(id).expect("calque").fill_content().is_none() && doc.pixels_mut(id).is_ok());
    }

    #[test]
    fn a_moved_fill_layer_reuses_its_inner_tiles_and_matches_a_fresh_render() {
        use crate::document::{Background, Document};
        let mut doc = Document::with_background("x", 1300, 900, 72.0, Background::White).expect("document");
        let fill = FillContent::Gradient { stops: vec![stop(0.0, [255, 0, 0, 255]), stop(1.0, [0, 0, 255, 200])], style: GradientStyle::Radial, angle: 30.0, scale: 80.0, reverse: false, offset: [5.0, -3.0] };
        let id = doc.add_fill_layer(fill.clone(), "Dégradé 1", None, 1).expect("remplissage");
        let before = Arc::clone(doc.layer(id).expect("calque").pixels().expect("rendu"));
        doc.translate_layers(&[id], 300, -280).expect("déplacement");
        let after = doc.layer(id).expect("calque").pixels().expect("rendu");
        let shared = after.iter().filter(|(c, t)| before.get(*c).is_some_and(|b| Arc::ptr_eq(b, t))).count();
        assert!(shared >= 3, "tuiles intérieures reprises : {shared}");
        let fresh = render(&fill, (1300, 900), (-300, 280, 1300, 900)).expect("rendu");
        assert_eq!(fresh.tile_count(), after.tile_count());
        for (coord, tile) in fresh.iter() {
            assert!(after.get(coord).is_some_and(|t| t.bytes() == tile.bytes()), "tuile {coord:?}");
        }
        // Remis en place : identique au rendu de départ.
        doc.translate_layers(&[id], -300, 280).expect("retour");
        let back = doc.layer(id).expect("calque").pixels().expect("rendu");
        for (coord, tile) in before.iter() {
            assert!(back.get(coord).is_some_and(|t| t.bytes() == tile.bytes()), "tuile {coord:?}");
        }
    }

    #[test]
    fn a_fill_layer_survives_a_project_round_trip() {
        use crate::document::{Background, Document};
        let mut doc = Document::with_background("x", 120, 80, 72.0, Background::White).expect("document");
        let fill = FillContent::Gradient { stops: vec![stop(0.0, [255, 0, 0, 255]), stop(1.0, [0, 0, 255, 255])], style: GradientStyle::Radial, angle: 30.0, scale: 80.0, reverse: true, offset: [5.0, -3.0] };
        let id = doc.add_fill_layer(fill.clone(), "Dégradé 1", None, 1).expect("remplissage");
        doc.translate_layers(&[id], 7, 3).expect("déplacement");
        let path = std::env::temp_dir().join(format!("lumitype-fill-{}.graf", std::process::id()));
        crate::io::save_project(&doc, &path).expect("enregistré");
        let back = crate::io::load_project(&path).expect("relu");
        let _ = std::fs::remove_file(&path);
        let (a, b) = (doc.layer(id).expect("calque"), back.layer(id).expect("calque relu"));
        assert_eq!(b.fill_content(), Some(&fill));
        assert_eq!(a.offset, b.offset);
        for (x, y) in [(-7, -3), (50, 40), (112, 76)] {
            assert_eq!(pixel(a.pixels().expect("rendu"), x, y), pixel(b.pixels().expect("rendu relu"), x, y), "({x}, {y})");
        }
    }

    #[test]
    fn a_pattern_fill_uses_its_two_colours() {
        let store = render(&FillContent::Pattern { pattern: "damier".into(), scale: 100.0, color: [200, 0, 0], background: [0, 0, 255] }, (64, 64), (0, 0, 64, 64)).expect("rendu");
        assert_eq!(pixel(&store, 2, 2), [200, 0, 0, 255]);
        assert_eq!(pixel(&store, 10, 2), [0, 0, 255, 255]);
        assert!(render(&FillContent::Pattern { pattern: "inconnu".into(), scale: 100.0, color: [0; 3], background: [0; 3] }, (8, 8), (0, 0, 8, 8)).is_err());
        // Fichier ou interface minimale : valeurs par défaut.
        let minimal: FillContent = serde_json::from_str(r#"{"type":"gradient","stops":[]}"#).expect("relu");
        assert!(matches!(minimal, FillContent::Gradient { style: GradientStyle::Linear, scale, .. } if (scale - 100.0).abs() < 1e-6));
    }
}
