// Rendu d'essai du pinceau flou (500 px), pour comparaison visuelle avec Photoshop.
use engine::document::{Background, Document};
use engine::paint::{Brush, BrushMode, Stroke, StrokePoint};

fn main() {
    let mut doc = Document::with_background("x", 1400, 900, 72.0, Background::White).expect("document");
    let layer = doc.add_pixel_layer("trait", engine::tiles::TileStore::new(), None, 1).expect("calque");
    let brush = Brush { size: 500.0, hardness: 0.0, opacity: 1.0, color: [0, 0, 0], mode: BrushMode::Paint, pressure_size: false, pressure_opacity: false };
    let point = |x: f32, y: f32| StrokePoint { x, y, pressure: 1.0 };
    // Un trait ondulé, puis un clic seul.
    let path: Vec<StrokePoint> = (0..=40).map(|i| point(150.0 + i as f32 * 22.0, 640.0 + (i as f32 * 0.25).sin() * 70.0)).collect();
    let mut stroke = Stroke::begin(&doc, layer, false, Brush { size: 300.0, ..brush.clone() }).expect("trait");
    stroke.extend(&mut doc, &path).expect("peinture");
    stroke.finish(&mut doc).expect("fin");
    // Série de clics comme la référence : 1, 2… 8 clics au même endroit, puis 40 (le maximum).
    let small = Brush { size: 150.0, ..brush.clone() };
    for (index, clicks) in [1usize, 2, 3, 4, 5, 6, 7, 8, 40].into_iter().enumerate() {
        let (x, y) = (90.0 + (index % 5) as f32 * 170.0, 90.0 + (index / 5) as f32 * 170.0);
        for _ in 0..clicks {
            let mut click = Stroke::begin(&doc, layer, false, small.clone()).expect("clic");
            click.extend(&mut doc, &[point(x, y)]).expect("clic");
            click.finish(&mut doc).expect("fin");
        }
    }
    let renderer = engine::render::TileRenderer::new();
    let mut rgba = vec![0u8; 1400 * 900 * 4];
    for ty in 0..4 {
        for tx in 0..6 {
            let Some(tile) = renderer.tile(&doc, 0, engine::tiles::TileCoord::new(tx, ty)) else { continue };
            for y in 0..256u32 {
                for x in 0..256u32 {
                    let (gx, gy) = (tx as u32 * 256 + x, ty as u32 * 256 + y);
                    if gx < 1400 && gy < 900 {
                        let i = ((gy * 1400 + gx) * 4) as usize;
                        rgba[i..i + 4].copy_from_slice(&tile.pixel(x, y));
                    }
                }
            }
        }
    }
    image::RgbaImage::from_raw(1400, 900, rgba).expect("image").save("/tmp/soft_stroke.png").expect("png");
}
