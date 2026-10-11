//! Documents de référence pour vérifier la composition WebGL de l'interface (ui/tests/gpu-parity.mjs).
//!
//! Pour chaque document : sa description (comme l'interface la reçoit), les pixels et masques de ses calques (comme
//! `layer_pixels` et `layer_mask` les envoient), ses réglages mis à plat (comme `adjustment_gpu`), et
//! l'image composée par le moteur, à pleine résolution. Le test rejoue la composition dans un vrai WebGL2 et compare.
//!
//!     cargo run -p engine --release --example webgl_parity -- <dossier de sortie>

use std::fs;
use std::path::{Path, PathBuf};

use engine::adjust::Adjustment;
use engine::blend::BlendMode;
use engine::document::{Background, Document, GradientStop, LayerId};
use engine::effects::{LayerEffect, StrokePosition};
use engine::render::TileRenderer;
use engine::tiles::{TILE_SIZE, TileCoord, TileStore};
use serde_json::{Value, json};

/// Bruit reproductible : une image colorée variée, avec des bords transparents (même motif que les tests `gpu`).
fn painted(width: u32, height: u32, seed: u32) -> TileStore {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            let n = (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503) ^ seed.wrapping_mul(97)) >> 24;
            let alpha = if x < 4 || y < 4 { 0 } else { (60 + (n % 196)) as u8 };
            let straight = [(x * 255 / width.max(1)) ^ n, ((y * 255 / height.max(1)) + n / 2).min(255), ((x + y) * 255 / (width + height).max(1)) ^ (seed * 40 % 256)];
            for (channel, value) in straight.iter().enumerate() {
                rgba[i + channel] = ((*value).min(255) * u32::from(alpha) / 255) as u8;
            }
            rgba[i + 3] = alpha;
        }
    }
    TileStore::from_premultiplied(width, height, &rgba).expect("pixels")
}

fn add_painted(doc: &mut Document, name: &str, seed: u32, parent: Option<LayerId>) -> LayerId {
    let index = doc.children_of(parent).expect("parent").len();
    let (w, h) = (doc.width(), doc.height());
    doc.add_pixel_layer(name, painted(w, h, seed), parent, index).expect("calque")
}

/// Masque en dégradé ; `partial` : seulement une bande peinte, le reste valant le fond (blanc).
fn gradient_mask(doc: &mut Document, id: LayerId, partial: bool) {
    doc.add_mask(id, true).expect("masque");
    let (w, h) = (doc.width(), doc.height());
    let mask = doc.mask_mut(id).expect("masque");
    for y in 0..h {
        for x in 0..w {
            if partial && !(x > w / 3 && x < w * 2 / 3) {
                continue;
            }
            let coord = TileCoord::new(x as i32 / TILE_SIZE as i32, y as i32 / TILE_SIZE as i32);
            mask.get_mut(coord).set_pixel(x % TILE_SIZE, y % TILE_SIZE, [((x + y) % 256) as u8]);
        }
    }
}

fn documents() -> Vec<(String, Document)> {
    let mut out = Vec::new();

    // Chaque mode de fusion, sur un fond peint à 80 %.
    for mode in BlendMode::LAYER_MODES {
        let mut doc = Document::with_background("modes", 300, 200, 72.0, Background::White).expect("document");
        let under = add_painted(&mut doc, "fond", 1, None);
        doc.edit_props(under, |p| p.opacity = 0.8).expect("opacité");
        let top = add_painted(&mut doc, "dessus", 2, None);
        doc.edit_props(top, |p| (p.blend, p.opacity, p.fill) = (mode, 0.75, 0.9)).expect("mode");
        out.push((format!("mode-{}", serde_json::to_value(mode).unwrap().as_str().unwrap()), doc));
    }

    // Masques (plein, partiel), calque déplacé, calque masqué.
    {
        let mut doc = Document::with_background("masques", 520, 380, 72.0, Background::Transparent).expect("document");
        let base = add_painted(&mut doc, "base", 3, None);
        gradient_mask(&mut doc, base, false);
        let moved = add_painted(&mut doc, "déplacé", 4, None);
        doc.translate_layers(&[moved], 137, -61).expect("déplacement");
        doc.edit_props(moved, |p| (p.blend, p.opacity) = (BlendMode::Overlay, 0.6)).expect("propriétés");
        gradient_mask(&mut doc, moved, true);
        let hidden = add_painted(&mut doc, "invisible", 5, None);
        doc.edit_props(hidden, |p| p.visible = false).expect("visibilité");
        out.push(("masques".into(), doc));
    }

    // Groupes en Transfert (opacité + masque), isolé dans un mode, imbriqués, écrêtages dont un groupe écrêté.
    {
        let mut doc = Document::with_background("groupes", 512, 400, 72.0, Background::White).expect("document");
        add_painted(&mut doc, "fond peint", 6, None);
        let transfer = doc.add_group("transfert", None, 2).expect("groupe");
        add_painted(&mut doc, "dans transfert", 7, Some(transfer));
        let inner = add_painted(&mut doc, "produit", 8, Some(transfer));
        doc.edit_props(inner, |p| p.blend = BlendMode::Multiply).expect("mode");
        doc.edit_props(transfer, |p| p.opacity = 0.55).expect("opacité");
        gradient_mask(&mut doc, transfer, false);

        let isolated = doc.add_group("isolé", None, 3).expect("groupe");
        doc.edit_props(isolated, |p| (p.blend, p.opacity) = (BlendMode::Screen, 0.8)).expect("mode");
        let nested = doc.add_group("imbriqué", Some(isolated), 0).expect("groupe");
        add_painted(&mut doc, "dans imbriqué", 9, Some(nested));
        let clipped_in_group = add_painted(&mut doc, "écrêté", 10, Some(nested));
        doc.edit_props(clipped_in_group, |p| (p.clipped, p.blend) = (true, BlendMode::ColorDodge)).expect("écrêtage");

        let base = add_painted(&mut doc, "base d'écrêtage", 11, None);
        doc.edit_props(base, |p| (p.opacity, p.blend) = (0.9, BlendMode::SoftLight)).expect("opacité");
        let paint = add_painted(&mut doc, "peinture écrêtée", 12, None);
        doc.edit_props(paint, |p| (p.clipped, p.blend, p.opacity) = (true, BlendMode::HardLight, 0.7)).expect("écrêtage");
        gradient_mask(&mut doc, paint, false);
        let clipped_group = doc.add_group("groupe écrêté", None, 6).expect("groupe");
        doc.edit_props(clipped_group, |p| (p.clipped, p.blend) = (true, BlendMode::Difference)).expect("écrêtage");
        let in_clipped = add_painted(&mut doc, "dans le groupe écrêté", 16, Some(clipped_group));
        doc.translate_layers(&[in_clipped], -40, 25).expect("déplacement");
        let clipped_adjust = doc.add_adjustment_layer(Adjustment::Invert, None, 7, true).expect("réglage");
        doc.edit_props(clipped_adjust, |p| p.opacity = 0.6).expect("opacité");
        out.push(("groupes".into(), doc));
    }

    // Effets de calque : ombre en Produit (fondue à part, dessous), contour en Incrustation (à part, dessus), lueur
    // et ombre en Normal (fondues au calque), calque écrêté sur une base à effets, calque à effets déplacé et masqué,
    // groupe dans un mode avec un calque écrêté dessus.
    {
        let mut doc = Document::with_background("effets", 480, 400, 72.0, Background::White).expect("document");
        add_painted(&mut doc, "fond", 14, None);
        let square = doc.add_pixel_layer("carré", TileStore::new(), None, 2).expect("calque");
        {
            let pixels = doc.pixels_mut(square).expect("pixels");
            for y in 110..280 {
                for x in 90..260 {
                    pixels.get_mut(TileCoord::new(x / 256, y / 256)).set_pixel((x % 256) as u32, (y % 256) as u32, [220, 40, 90, 255]);
                }
            }
        }
        doc.set_effects(
            square,
            vec![
                LayerEffect::DropShadow { enabled: true, color: [0, 0, 0], opacity: 0.8, angle: 135.0, distance: 24.0, size: 18.0, blend: BlendMode::Multiply },
                LayerEffect::Stroke { enabled: true, color: [255, 255, 255], opacity: 0.9, width: 8.0, position: StrokePosition::Outside, blend: BlendMode::Overlay },
                LayerEffect::OuterGlow { enabled: true, color: [255, 220, 0], opacity: 0.7, size: 20.0, blend: BlendMode::Normal },
            ],
        )
        .expect("effets");
        doc.edit_props(square, |p| (p.opacity, p.fill) = (0.85, 0.7)).expect("opacité");
        let paint = add_painted(&mut doc, "peinture écrêtée", 15, None);
        doc.edit_props(paint, |p| (p.clipped, p.blend) = (true, BlendMode::Screen)).expect("écrêtage");

        let badge = add_painted(&mut doc, "badge", 17, None);
        doc.translate_layers(&[badge], 180, 120).expect("déplacement");
        gradient_mask(&mut doc, badge, true);
        doc.set_effects(badge, vec![LayerEffect::DropShadow { enabled: true, color: [20, 0, 60], opacity: 0.6, angle: 45.0, distance: 10.0, size: 8.0, blend: BlendMode::Normal }]).expect("effets");
        doc.edit_props(badge, |p| p.blend = BlendMode::HardLight).expect("mode");

        let group = doc.add_group("groupe", None, 5).expect("groupe");
        add_painted(&mut doc, "dans le groupe", 18, Some(group));
        doc.edit_props(group, |p| (p.blend, p.opacity) = (BlendMode::Multiply, 0.5)).expect("mode");
        let on_group = add_painted(&mut doc, "écrêté sur le groupe", 19, None);
        doc.edit_props(on_group, |p| p.clipped = true).expect("écrêtage");
        out.push(("effets".into(), doc));
    }

    // Réglages, en Normal et en Luminosité, avec masque.
    let adjustments = [
        Adjustment::BrightnessContrast { brightness: 35.0, contrast: 60.0 },
        Adjustment::Levels { black: 18.0, gamma: 1.6, white: 230.0 },
        Adjustment::Invert,
        Adjustment::Negative,
        Adjustment::GradientMap {
            stops: vec![
                GradientStop { position: 0.0, color: [253, 194, 216, 255] },
                GradientStop { position: 0.36, color: [252, 166, 94, 128] },
                GradientStop { position: 0.67, color: [255, 121, 131, 255] },
                GradientStop { position: 1.0, color: [4, 18, 130, 255] },
            ],
            reverse: false,
            dither: false,
            smoothness: 100.0,
        },
        Adjustment::GradientMap {
            stops: vec![GradientStop { position: 0.0, color: [0, 0, 0, 255] }, GradientStop { position: 1.0, color: [255, 255, 255, 255] }],
            reverse: true,
            dither: true,
            smoothness: 0.0,
        },
        Adjustment::HueSaturation { hue: -120.0, saturation: 60.0, lightness: 10.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 30.0, range_width: 45.0 },
        Adjustment::HueSaturation { hue: 40.0, saturation: 35.0, lightness: -15.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 0.0, range_width: 0.0 },
        Adjustment::HueSaturation { hue: 25.0, saturation: 70.0, lightness: 12.0, range_start: 340.0, range_end: 40.0, feather_in: 12.0, feather_out: 45.0, range_center: 0.0, range_width: 0.0 },
        Adjustment::Develop {
            temperature: 45.0,
            tint: -20.0,
            exposure: 0.8,
            contrast: 40.0,
            highlights: -60.0,
            shadows: 55.0,
            whites: 25.0,
            blacks: -30.0,
            clarity: 40.0,
            dehaze: 30.0,
            vibrance: 50.0,
            saturation: -20.0,
            red_hue: 25.0,
            green_hue: -15.0,
            blue_hue: 10.0,
        },
    ];
    for (index, adjustment) in adjustments.iter().enumerate() {
        for mode in [BlendMode::Normal, BlendMode::Luminosity] {
            let mut doc = Document::with_background("réglages", 300, 200, 72.0, Background::White).expect("document");
            add_painted(&mut doc, "photo", 13, None);
            let layer = doc.add_adjustment_layer(adjustment.clone(), None, 2, false).expect("réglage");
            gradient_mask(&mut doc, layer, index % 2 == 1);
            doc.edit_props(layer, |p| (p.blend, p.opacity) = (mode, 0.85)).expect("mode");
            out.push((format!("reglage-{index}-{}", serde_json::to_value(mode).unwrap().as_str().unwrap()), doc));
        }
    }
    out
}

fn header(x: i32, y: i32, w: u32, h: u32) -> Vec<u8> {
    [x.to_le_bytes(), y.to_le_bytes(), w.to_le_bytes(), h.to_le_bytes()].concat()
}

fn dump(dir: &Path, doc: &Document) {
    fs::create_dir_all(dir).expect("dossier");
    let renderer = TileRenderer::new();
    let mut layers: Vec<Value> = Vec::new();
    let mut stack = vec![None];
    while let Some(parent) = stack.pop() {
        for (index, id) in doc.children_of(parent).expect("enfants").iter().enumerate() {
            let layer = doc.layer(*id).expect("calque");
            let p = &layer.props;
            let adjustment = layer.adjustment().map(|a| serde_json::to_value(a).expect("réglage"));
            layers.push(json!({
                "id": id, "parent": parent, "index": index, "depth": 0, "offset": [layer.offset.0, layer.offset.1],
                "name": p.name, "isGroup": layer.is_group(), "expanded": true, "childCount": 0, "visible": p.visible,
                "opacity": p.opacity, "fill": p.fill, "blend": p.blend, "locks": {}, "clipped": p.clipped,
                "hasMask": layer.mask.is_some(), "maskEnabled": layer.mask.as_ref().is_some_and(|m| m.enabled),
                "text": null, "adjustment": adjustment, "shape": null, "effects": serde_json::to_value(&layer.effects).expect("effets"),
            }));
            if layer.is_group() {
                stack.push(Some(*id));
            }
            // Comme `layer_pixels` : le calque avec ses effets fondus (p), ses pixels seuls (r), ses effets à part (u, o),
            // et comme `layer_effects` : leurs modes (e).
            let store = |store: &TileStore| {
                let (x, y, w, h) = store.pixel_bounds(|px| px[3] > 0).map_or((0, 0, 0, 0), |(x0, y0, x1, y1)| (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32));
                let mut body = header(x, y, w, h);
                body.extend_from_slice(&store.read_rect(x, y, w, h));
                body
            };
            if let (Some(pixels), Some(styled)) = (layer.pixels(), renderer.styled_layer(layer)) {
                fs::write(dir.join(format!("p{id}.bin")), store(&styled.main)).expect("pixels");
                fs::write(dir.join(format!("r{id}.bin")), store(pixels)).expect("pixels seuls");
                for (n, (_, part)) in styled.under.iter().enumerate() {
                    fs::write(dir.join(format!("u{id}_{n}.bin")), store(part)).expect("effet");
                }
                for (n, (_, part)) in styled.over.iter().enumerate() {
                    fs::write(dir.join(format!("o{id}_{n}.bin")), store(part)).expect("effet");
                }
                let modes = json!({ "under": styled.under.iter().map(|(m, _)| *m).collect::<Vec<_>>(), "over": styled.over.iter().map(|(m, _)| *m).collect::<Vec<_>>() });
                fs::write(dir.join(format!("e{id}.bin")), modes.to_string()).expect("modes");
            }
            if let Some(mask) = &layer.mask {
                let (x, y, w, h) = mask.pixels.bounds().map_or((0, 0, 0, 0), |(x0, y0, x1, y1)| (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32));
                let mut body = header(x, y, w, h);
                body.extend_from_slice(&u32::from(mask.pixels.background()[0]).to_le_bytes());
                body.extend_from_slice(&mask.pixels.read_rect(x, y, w, h));
                fs::write(dir.join(format!("m{id}.bin")), body).expect("masque");
            }
            if let Some(adjustment) = layer.adjustment() {
                // Comme `adjustment_gpu` : type, puis valeurs.
                let gpu = adjustment.prepare().to_gpu();
                let mut body = gpu.kind.to_le_bytes().to_vec();
                for value in &gpu.values {
                    body.extend_from_slice(&value.to_le_bytes());
                }
                fs::write(dir.join(format!("a{id}.bin")), body).expect("réglage");
            }
        }
    }

    // Image composée par le moteur, niveau 0, tuile par tuile.
    let (w, h) = (doc.width(), doc.height());
    let mut image = vec![0u8; (w * h * 4) as usize];
    for ty in 0..h.div_ceil(TILE_SIZE) {
        for tx in 0..w.div_ceil(TILE_SIZE) {
            let Some(tile) = renderer.tile(doc, 0, TileCoord::new(tx as i32, ty as i32)) else { continue };
            for y in 0..TILE_SIZE.min(h - ty * TILE_SIZE) {
                for x in 0..TILE_SIZE.min(w - tx * TILE_SIZE) {
                    let at = (((ty * TILE_SIZE + y) * w + tx * TILE_SIZE + x) * 4) as usize;
                    image[at..at + 4].copy_from_slice(&tile.pixel(x, y));
                }
            }
        }
    }
    fs::write(dir.join("expected.bin"), image).expect("image");
    let info = json!({ "id": 1, "name": "parité", "width": w, "height": h, "tileSize": TILE_SIZE, "maxLevel": 0, "layers": layers });
    fs::write(dir.join("doc.json"), serde_json::to_string_pretty(&info).expect("json")).expect("description");
}

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).expect("dossier de sortie"));
    let docs = documents();
    for (name, doc) in &docs {
        dump(&out.join(name), doc);
    }
    let names: Vec<&str> = docs.iter().map(|(name, _)| name.as_str()).collect();
    fs::write(out.join("index.json"), serde_json::to_string(&names).expect("json")).expect("index");
    println!("{} documents écrits dans {}", docs.len(), out.display());
}
