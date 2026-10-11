//! Mesures des opérations lourdes de l'étape 1, sur une image 8000 x 8000.
//! Lancer avec : `cargo bench -p engine`.

use criterion::{criterion_group, criterion_main, Criterion};
use engine::document::Background;
use engine::{BlendMode, Document, TileCoord, TileRenderer, TileStore};

const SIDE: u32 = 8000;

fn bench_tiling(c: &mut Criterion) {
    let rgba: Vec<u8> = (0..SIDE * SIDE).flat_map(|i| [(i % 255) as u8, (i % 13) as u8, 7, 255]).collect();
    c.bench_function("decoupage_8000x8000", |b| b.iter(|| TileStore::from_premultiplied(SIDE, SIDE, &rgba)));
}

fn bench_pyramid(c: &mut Criterion) {
    let doc = match Document::with_background("bench", SIDE, SIDE, 300.0, Background::White) {
        Ok(doc) => doc,
        Err(error) => panic!("document de test invalide : {error}"),
    };
    let top = TileRenderer::max_level(&doc);
    c.bench_function("pyramide_complete_8000x8000", |b| {
        b.iter(|| TileRenderer::new().tile(&doc, top, TileCoord::new(0, 0)))
    });
}

/// Coût du compositeur CPU de référence : une tuile, dix calques translucides en Incrustation.
/// C'est ce chiffre, multiplié par le nombre de tuiles visibles, qui justifiera le passage sur GPU.
fn bench_composition(c: &mut Criterion) {
    let mut doc = match Document::with_background("bench", 256, 256, 72.0, Background::White) {
        Ok(doc) => doc,
        Err(error) => panic!("document de test invalide : {error}"),
    };
    for index in 1..=10 {
        let mut pixels = TileStore::new();
        pixels.fill(256, 256, [90, 60, 30, 128]);
        let added = doc
            .add_pixel_layer("calque", pixels, None, index)
            .and_then(|id| doc.edit_props(id, |p| p.blend = BlendMode::Overlay));
        if let Err(error) = added {
            panic!("calque de test invalide : {error}");
        }
    }
    c.bench_function("composition_tuile_10_calques", |b| {
        b.iter(|| TileRenderer::new().tile(&doc, 0, TileCoord::new(0, 0)))
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_tiling, bench_pyramid, bench_composition
}
criterion_main!(benches);
