//! Mise au point : l'aperçu « Tous les calques » montre-t-il bien le filtre ?
use engine::document::{Background, Document};
use engine::filters::Filter;
use engine::session::Session;
use engine::tiles::TileStore;

fn photo(width: u32, height: u32) -> TileStore {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            // Damier contrasté : un flou se voit tout de suite.
            let dark = ((x / 16) + (y / 16)) % 2 == 0;
            let value = if dark { 0 } else { 255 };
            rgba[i..i + 4].copy_from_slice(&[value, value / 2, 255 - value, 255]);
        }
    }
    TileStore::from_premultiplied(width, height, &rgba).expect("pixels")
}

#[test]
fn the_all_layers_preview_shows_the_filter() {
    let (w, h) = (4000u32, 3000u32);
    let mut doc = Document::with_background("photo", w, h, 72.0, Background::White).expect("document");
    let layer = doc.add_pixel_layer("image", photo(w, h), None, 1).expect("calque");
    let session = Session::new(doc, "ouverture");

    let filter = Filter::GaussianBlur { radius: 20.0 };
    let (cw, ch, filtered) = session.filter_preview_in_context(layer, &filter, 1024).expect("aperçu en contexte");
    let (nw, nh, plain) = session.filter_preview_in_context(layer, &Filter::GaussianBlur { radius: 0.0 }, 1024).expect("aperçu sans flou");
    assert_eq!((cw, ch), (nw, nh));

    let changed = filtered.chunks_exact(4).zip(plain.chunks_exact(4)).filter(|(a, b)| a[0].abs_diff(b[0]) > 8).count();
    let total = (cw * ch) as usize;
    println!("aperçu {cw} x {ch} : {changed} pixels changés sur {total}");
    assert!(changed * 10 > total, "le flou doit se voir sur l'aperçu en contexte : {changed} / {total}");
}

#[test]
fn the_single_layer_preview_shows_the_filter_too() {
    let (w, h) = (4000u32, 3000u32);
    let mut doc = Document::with_background("photo", w, h, 72.0, Background::White).expect("document");
    let layer = doc.add_pixel_layer("image", photo(w, h), None, 1).expect("calque");

    let filter = Filter::GaussianBlur { radius: 20.0 };
    let (cw, ch, filtered) = doc.filter_preview(layer, &filter, 1024).expect("aperçu du calque");
    let (_, _, plain) = doc.filter_preview(layer, &Filter::GaussianBlur { radius: 0.0 }, 1024).expect("aperçu sans flou");
    let changed = filtered.chunks_exact(4).zip(plain.chunks_exact(4)).filter(|(a, b)| a[0].abs_diff(b[0]) > 8).count();
    println!("aperçu du calque {cw} x {ch} : {changed} pixels changés sur {}", cw * ch);
    assert!(changed * 10 > (cw * ch) as usize, "le flou doit se voir sur l'aperçu du calque seul");
}
