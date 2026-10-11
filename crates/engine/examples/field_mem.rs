//! Mesure : mémoire et temps du flou de profondeur de champ sur une grande image.
use engine::filters::{apply, BlurPin, Filter};
use engine::tiles::TileStore;

fn main() {
    let (w, h) = (4000u32, 3000u32);
    let rgba: Vec<u8> = (0..(w * h) as usize).flat_map(|i| { let v = ((i % 255) as u8).max(1); [v, v / 2, 255 - v, 255] }).collect();
    let source = TileStore::from_premultiplied(w, h, &rgba).expect("pixels");
    let pins = vec![BlurPin { x: 0.25, y: 0.5, blur: 40.0 }, BlurPin { x: 0.75, y: 0.5, blur: 0.0 }];
    let start = std::time::Instant::now();
    let out = apply(&source, &Filter::FieldBlur { pins }).expect("flou");
    println!("{w} x {h} : {:.2} s, {} tuiles", start.elapsed().as_secs_f32(), out.tile_count());
}
