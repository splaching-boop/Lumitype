//! Fabrique les vignettes du menu Filtre : l'image de référence, moitié gauche filtrée, moitié
//! droite d'origine. Les filtres sont ceux du moteur : la vignette montre exactement ce que le
//! filtre fera. À relancer quand un filtre change ou s'ajoute :
//!
//!     cargo run -p engine --release --example filter_previews -- ui/src/assets/filters/reference.jpg ui/src/assets/filters

use engine::filters::{apply, BlurPin, Filter};
use engine::pixel::premultiply;
use engine::tiles::TileStore;
use image::imageops::FilterType;

const SIZE: (u32, u32) = (420, 280);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (reference, out_dir) = (args.next().ok_or("chemin de l'image de référence manquant")?, args.next().ok_or("dossier de sortie manquant")?);
    let source = image::open(&reference)?.resize_to_fill(SIZE.0, SIZE.1, FilterType::Lanczos3).into_rgba8();
    let mut rgba = source.clone().into_raw();
    for px in rgba.chunks_exact_mut(4) {
        let p = premultiply([px[0], px[1], px[2], px[3]]);
        px.copy_from_slice(&p);
    }
    let store = TileStore::from_premultiplied(SIZE.0, SIZE.1, &rgba).ok_or("tampon de taille inattendue")?;

    // Réglages de démonstration, choisis pour que l'effet se lise d'un coup d'œil à cette taille.
    let filters = [
        ("gaussianBlur", Filter::GaussianBlur { radius: 5.0 }),
        // Net sur la fougère (à droite de la moitié filtrée), de plus en plus flou vers la gauche.
        ("fieldBlur", Filter::FieldBlur { pins: vec![BlurPin { x: 0.42, y: 0.5, blur: 0.0 }, BlurPin { x: 0.04, y: 0.5, blur: 26.0 }] }),
        ("motionBlur", Filter::MotionBlur { angle: 20.0, distance: 36.0 }),
        ("lensBlur", Filter::LensBlur { radius: 9.0, highlights: 70.0 }),
        ("sharpen", Filter::Sharpen { radius: 1.5, amount: 220.0 }),
        ("noise", Filter::Noise { amount: 35.0, monochrome: false }),
        ("dither", Filter::Dither { scale: 2, levels: 2, monochrome: true }),
        ("glass", Filter::Glass { distortion: 9.0, smoothness: 3.0, texture: engine::filters::GlassTexture::Blocks, scale: 100.0, invert: false }),
        ("presence", Filter::Presence { texture: 60.0, clarity: 70.0, dehaze: 35.0 }),
        ("gaussianNoise", Filter::GaussianNoise { radius: 10.0, scatter: 7.0, amount: 28.0, grain: 1, monochrome: true }),
        ("colorDither", Filter::ColorDither { scale: 2, contrast: 120.0, threshold: 42.0, strength: 100.0 }),
        ("chromaticAberration", Filter::ChromaticAberration { amount: 9.0, angle: 0.0, radial: true }),
        ("ascii", Filter::Ascii { cell: 9, color: true }),
    ];
    for (name, filter) in filters {
        let filtered = apply(&store, &filter)?.to_premultiplied(SIZE.0, SIZE.1);
        let mut out = image::RgbImage::new(SIZE.0, SIZE.1);
        for (x, y, px) in out.enumerate_pixels_mut() {
            let i = ((y * SIZE.0 + x) * 4) as usize;
            // L'image de référence est opaque : le prémultiplié vaut la couleur.
            *px = if x < SIZE.0 / 2 { image::Rgb([filtered[i], filtered[i + 1], filtered[i + 2]]) } else { image::Rgb([rgba[i], rgba[i + 1], rgba[i + 2]]) };
        }
        let path = std::path::Path::new(&out_dir).join(format!("{name}.jpg"));
        let mut file = std::io::BufWriter::new(std::fs::File::create(&path)?);
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 86).encode_image(&out)?;
        println!("{}", path.display());
    }
    Ok(())
}
