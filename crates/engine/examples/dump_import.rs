//! Outil de mise au point : ouvre une image par le chemin d'import du moteur et l'écrit en PNG.
//!
//!     cargo run -p engine --release --example dump_import -- photo.heic sortie.png
use engine::io::import_image_bytes;
use engine::tiles::Sampler;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (input, output) = (args.next().ok_or("fichier d'entrée manquant")?, args.next().ok_or("fichier de sortie manquant")?);
    let doc = import_image_bytes(&std::fs::read(&input)?, std::path::Path::new(&input))?;
    let layer = doc.layer(doc.children_of(None)?[0])?;
    let mut sampler = Sampler::new(layer.pixels().ok_or("pas de pixels")?);
    let mut out = image::RgbaImage::new(doc.width(), doc.height());
    for (x, y, px) in out.enumerate_pixels_mut() {
        *px = image::Rgba(engine::pixel::unpremultiply(sampler.get(x as i32, y as i32)));
    }
    out.save(&output)?;
    println!("{} × {} → {output}", doc.width(), doc.height());
    Ok(())
}
