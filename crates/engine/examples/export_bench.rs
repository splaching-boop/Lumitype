//! Mesure de l'export d'une image 8K (7680 × 4320) à plusieurs calques : temps de chaque étape.
//! `cargo run --release -p engine --example export_bench`
use std::time::Instant;

use engine::blend::BlendMode;
use engine::document::{Background, Document};
use engine::io::{ExportFormat, ExportOptions, export_image_with_progress, flatten_for_export};
use engine::tiles::TileStore;

fn main() {
    let (w, h) = (7680u32, 4320u32);
    let mut doc = Document::with_background("banc", w, h, 72.0, Background::White).expect("document");
    // Calques pleins et variés : un dégradé bruité, un calque en Produit, un calque à 60 % d'opacité.
    for (index, blend) in [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen].into_iter().enumerate() {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let n = (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503) ^ (index as u32 * 97)) >> 24;
                rgba[i] = ((x * 255 / w) as u8).wrapping_add(n as u8 / 8);
                rgba[i + 1] = ((y * 255 / h) as u8).wrapping_add(n as u8 / 6);
                rgba[i + 2] = (((x + y) * 255 / (w + h)) as u8) ^ (index as u8 * 40);
                rgba[i + 3] = 255;
            }
        }
        let pixels = TileStore::from_premultiplied(w, h, &rgba).expect("pixels");
        let id = doc.add_pixel_layer(format!("calque {index}"), pixels, None, index + 1).expect("calque");
        doc.edit_props(id, |p| {
            p.blend = blend;
            p.opacity = if index == 2 { 0.6 } else { 1.0 };
        })
        .expect("propriétés");
    }
    // Une vraie photo (la bannière de l'auteur) agrandie en 8K, sous les autres : un contenu réaliste, peu compressible.
    let banner = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/assets/splash/banner.jpg");
    if let Ok(photo) = image::open(&banner) {
        let photo = photo.resize_exact(w, h, image::imageops::FilterType::Triangle).into_rgba8();
        let pixels = TileStore::from_premultiplied(w, h, photo.as_raw()).expect("photo");
        let id = doc.add_pixel_layer("photo", pixels, None, 4).expect("calque photo");
        doc.edit_props(id, |p| p.opacity = 0.85).expect("opacité");
    }
    let dir = std::env::temp_dir();
    for (label, format, scale) in [("PNG 8K", ExportFormat::Png, 1.0), ("JPEG 8K", ExportFormat::Jpeg, 1.0), ("PNG réduit à 50 %", ExportFormat::Png, 0.5)] {
        let start = Instant::now();
        let last = std::cell::Cell::new((0.0f32, String::from("début")));
        let steps = std::cell::RefCell::new(Vec::<(String, f32)>::new());
        let mark = std::cell::Cell::new(Instant::now());
        let report = |_: f32, step: &str| {
            let previous = last.replace((0.0, step.to_owned()));
            if previous.1 != step {
                steps.borrow_mut().push((previous.1, mark.replace(Instant::now()).elapsed().as_secs_f32()));
            }
        };
        let path = dir.join(format!("banc-export.{}", if matches!(format, ExportFormat::Png) { "png" } else { "jpg" }));
        export_image_with_progress(&doc, &path, ExportOptions { format, quality: 90, scale, metadata: None }, &report).expect("export");
        let total = start.elapsed().as_secs_f32();
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        println!("{label} : {total:.2} s, {:.1} Mo", size as f64 / 1_048_576.0);
        for (step, seconds) in steps.borrow().iter().filter(|(s, _)| s != "début") {
            println!("    {step:<28} {seconds:.2} s");
        }
        let _ = std::fs::remove_file(&path);
    }
    // Aperçu de l'export : composition une fois, puis vue d'ensemble, zone visible et poids estimé.
    let start = Instant::now();
    let flat = flatten_for_export(&doc, 1.0).expect("composition");
    println!("Aperçu — composition (une seule fois) : {:.2} s", start.elapsed().as_secs_f32());
    for (label, format) in [("PNG", ExportFormat::Png), ("JPEG 85", ExportFormat::Jpeg)] {
        let t = Instant::now();
        let (_, _, overview, _) = flat.overview(1600, format, 85).expect("vue d'ensemble");
        let overview_time = t.elapsed().as_secs_f32();
        let t = Instant::now();
        let (_, _, region, _) = flat.region([3000, 1500, 1400, 900], format, 85).expect("zone");
        let region_time = t.elapsed().as_secs_f32();
        let t = Instant::now();
        let (estimate, _) = flat.estimate_size(format, 85).expect("estimation");
        let estimate_time = t.elapsed().as_secs_f32();
        let t = Instant::now();
        let exact = flat.exact_size(format, 85).expect("poids exact");
        let exact_time = t.elapsed().as_secs_f32();
        println!(
            "  {label:<8} vue d'ensemble {overview_time:.2} s ({} Ko) · zone visible {region_time:.2} s ({} Ko) · poids estimé {:.1} Mo en {estimate_time:.2} s, réel {:.1} Mo ({:+.1} %) en {exact_time:.2} s",
            overview.len() / 1024,
            region.len() / 1024,
            estimate as f64 / 1_048_576.0,
            exact as f64 / 1_048_576.0,
            (estimate as f64 / exact as f64 - 1.0) * 100.0
        );
    }
}
