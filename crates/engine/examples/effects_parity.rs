//! Références pour l'aperçu des effets de calque sur carte graphique (ui/src/viewport/gpu-effects.ts).
//!
//! Pour chaque cas : les pixels seuls du calque (comme `layer_pixels` partie `raw`), les effets, et le calque stylé
//! calculé par le moteur (`styled_parts` : `main`, puis les effets fondus à part `under_<n>` et `over_<n>`), au même
//! format (16 octets d'en-tête x, y, largeur, hauteur, puis RGBA8 prémultiplié). Le test rejoue le calcul dans un
//! vrai WebGL2 et compare.
//!
//!     cargo run -p engine --release --example effects_parity -- <dossier de sortie> [côté du grand calque]

use std::fs;
use std::path::{Path, PathBuf};

use engine::blend::BlendMode;
use engine::effects::{LayerEffect, StrokePosition, styled_parts};
use engine::tiles::TileStore;
use serde_json::json;

/// Forme de test : un disque au bord adouci, un rectangle, une bande à demi transparente et un trou, en couleurs.
fn shape(width: u32, height: u32, at: (i32, i32)) -> TileStore {
    let (w, h) = (width as f32, height as f32);
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let disc = (w * 0.24 - ((fx - w * 0.32).hypot(fy - h * 0.4))).clamp(0.0, 1.0);
            let hole = (w * 0.05 - ((fx - w * 0.32).hypot(fy - h * 0.4))).clamp(0.0, 1.0);
            let rect = if fx > w * 0.5 && fx < w * 0.92 && fy > h * 0.3 && fy < h * 0.85 { 1.0 } else { 0.0 };
            let band = if fx > w * 0.1 && fx < w * 0.9 && fy > h * 0.86 && fy < h * 0.95 { 0.45 } else { 0.0 };
            let alpha = (disc * (1.0 - hole)).max(rect).max(band);
            let i = ((y * width + x) * 4) as usize;
            let straight = [x * 255 / width, 120, y * 255 / height];
            for c in 0..3 {
                rgba[i + c] = (straight[c] as f32 * alpha + 0.5) as u8;
            }
            rgba[i + 3] = (alpha * 255.0 + 0.5) as u8;
        }
    }
    TileStore::from_premultiplied(width, height, &rgba).expect("pixels").translated(at.0, at.1)
}

fn write_store(path: &Path, store: &TileStore) {
    let (x, y, w, h) = store.pixel_bounds(|px| px[3] > 0).map_or((0, 0, 0, 0), |(x0, y0, x1, y1)| (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32));
    let mut body = [x.to_le_bytes(), y.to_le_bytes(), w.to_le_bytes(), h.to_le_bytes()].concat();
    body.extend_from_slice(&store.read_rect(x, y, w, h));
    fs::write(path, body).expect("écriture");
}

fn cases() -> Vec<(String, Vec<LayerEffect>)> {
    let shadow = |size: f32, blend| LayerEffect::DropShadow { enabled: true, color: [10, 0, 40], opacity: 0.75, angle: 120.0, distance: 15.0, size, blend };
    let stroke = |width: f32, position, blend| LayerEffect::Stroke { enabled: true, color: [255, 200, 0], opacity: 0.9, width, position, blend };
    let mut out: Vec<(String, Vec<LayerEffect>)> = Vec::new();
    for size in [0.0, 5.0, 40.0] {
        out.push((format!("ombre-{size}"), vec![shadow(size, BlendMode::Normal)]));
    }
    out.push(("ombre-16-produit".into(), vec![shadow(16.0, BlendMode::Multiply)]));
    for width in [1.0, 10.0] {
        for (name, position) in [("exterieur", StrokePosition::Outside), ("interieur", StrokePosition::Inside), ("centre", StrokePosition::Center)] {
            out.push((format!("contour-{width}-{name}"), vec![stroke(width, position, BlendMode::Normal)]));
        }
    }
    out.push(("contour-6-incrustation".into(), vec![stroke(6.0, StrokePosition::Outside, BlendMode::Overlay)]));
    out.push(("lueur-24".into(), vec![LayerEffect::OuterGlow { enabled: true, color: [255, 236, 170], opacity: 0.75, size: 24.0, blend: BlendMode::Normal }]));
    out.push(("lueur-24-ecran".into(), vec![LayerEffect::OuterGlow { enabled: true, color: [255, 236, 170], opacity: 0.75, size: 24.0, blend: BlendMode::Screen }]));
    out.push(("lueur-interne-14".into(), vec![LayerEffect::InnerGlow { enabled: true, color: [255, 255, 255], opacity: 0.6, size: 14.0, blend: BlendMode::Normal }]));
    out.push(("ombre-interne-10".into(), vec![LayerEffect::InnerShadow { enabled: true, color: [0, 0, 0], opacity: 0.5, angle: 120.0, distance: 6.0, size: 10.0, blend: BlendMode::Normal }]));
    out.push(("incrustations".into(), vec![
        LayerEffect::ColorOverlay { enabled: true, color: [255, 0, 80], opacity: 0.5, blend: BlendMode::Normal },
        LayerEffect::GradientOverlay { enabled: true, from: [255, 80, 120], to: [80, 120, 255], opacity: 0.6, angle: 30.0, radial: false, blend: BlendMode::Normal },
    ]));
    out.push(("degrade-radial-produit".into(), vec![LayerEffect::GradientOverlay { enabled: true, from: [255, 255, 255], to: [0, 0, 80], opacity: 1.0, angle: 0.0, radial: true, blend: BlendMode::Multiply }]));
    // Les réglages par défaut de la boîte de dialogue, tous ensemble.
    out.push(("tous".into(), vec![
        LayerEffect::DropShadow { enabled: true, color: [0, 0, 0], opacity: 0.55, angle: 120.0, distance: 12.0, size: 16.0, blend: BlendMode::Multiply },
        LayerEffect::InnerShadow { enabled: true, color: [0, 0, 0], opacity: 0.5, angle: 120.0, distance: 6.0, size: 10.0, blend: BlendMode::Multiply },
        LayerEffect::OuterGlow { enabled: true, color: [255, 236, 170], opacity: 0.75, size: 24.0, blend: BlendMode::Screen },
        LayerEffect::InnerGlow { enabled: true, color: [255, 255, 255], opacity: 0.6, size: 14.0, blend: BlendMode::Screen },
        stroke(4.0, StrokePosition::Outside, BlendMode::Normal),
        stroke(10.0, StrokePosition::Center, BlendMode::Normal),
        LayerEffect::ColorOverlay { enabled: true, color: [255, 0, 80], opacity: 0.3, blend: BlendMode::Normal },
    ]));
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().expect("dossier de sortie"));
    let big: u32 = args.next().map_or(0, |v| v.parse().expect("côté"));
    let layer = shape(520, 400, (37, -21));
    let mut names = Vec::new();
    for (name, effects) in cases() {
        let dir = out.join(&name);
        fs::create_dir_all(&dir).expect("dossier");
        write_store(&dir.join("raw.bin"), &layer);
        let started = std::time::Instant::now();
        let parts = styled_parts(&layer, &effects).expect("effets");
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        write_store(&dir.join("main.bin"), &parts.main);
        for (n, (_, store)) in parts.under.iter().enumerate() {
            write_store(&dir.join(format!("under_{n}.bin")), store);
        }
        for (n, (_, store)) in parts.over.iter().enumerate() {
            write_store(&dir.join(format!("over_{n}.bin")), store);
        }
        let info = json!({ "effects": effects, "under": parts.under.iter().map(|(m, _)| *m).collect::<Vec<_>>(), "over": parts.over.iter().map(|(m, _)| *m).collect::<Vec<_>>(), "engineMs": elapsed });
        fs::write(dir.join("case.json"), info.to_string()).expect("cas");
        names.push(name);
    }
    // Grand calque (mesure de temps seulement) : pixels seuls, et le temps du moteur pour quelques effets.
    if big > 0 {
        let dir = out.join("grand");
        fs::create_dir_all(&dir).expect("dossier");
        let large = shape(big, big, (0, 0));
        write_store(&dir.join("raw.bin"), &large);
        let mut timings = serde_json::Map::new();
        for (name, effects) in cases().into_iter().filter(|(n, _)| ["ombre-5", "ombre-40", "contour-10-exterieur", "lueur-24", "tous"].contains(&n.as_str())) {
            let started = std::time::Instant::now();
            styled_parts(&large, &effects).expect("effets");
            timings.insert(name, json!(started.elapsed().as_secs_f64() * 1000.0));
        }
        fs::write(dir.join("timings.json"), serde_json::Value::Object(timings).to_string()).expect("temps");
    }
    fs::write(out.join("index.json"), serde_json::to_string(&names).expect("json")).expect("index");
    println!("{} cas écrits dans {}", names.len(), out.display());
}
