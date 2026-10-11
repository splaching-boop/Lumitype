//! Mise au point : applique Inverser et Négatif à une image et écrit les deux résultats en PNG.
//!
//!     cargo run -p engine --release --example invert_check -- normal.jpg inverse.png negatif.png
use engine::adjust::Adjustment;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (input, out_invert, out_negative) = (args.next().ok_or("entrée")?, args.next().ok_or("sortie inverser")?, args.next().ok_or("sortie négatif")?);
    let source = image::open(&input)?.into_rgb8();
    for (adjustment, path) in [(Adjustment::Invert, out_invert), (Adjustment::Negative, out_negative)] {
        let prepared = adjustment.prepare();
        let mut out = image::RgbImage::new(source.width(), source.height());
        for (x, y, px) in out.enumerate_pixels_mut() {
            let s = source.get_pixel(x, y).0;
            let rgb = prepared.apply([f32::from(s[0]) / 255.0, f32::from(s[1]) / 255.0, f32::from(s[2]) / 255.0]);
            *px = image::Rgb(rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
        }
        out.save(&path)?;
        println!("{path}");
    }
    Ok(())
}
