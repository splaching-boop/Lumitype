//! Fabrique les illustrations de l'aide : onglet Réglages, chaque curseur de Développement, et les
//! deux masques. Comme pour les vignettes de filtres, la moitié gauche est corrigée et la moitié
//! droite montre l'image d'origine — la comparaison se fait d'un coup d'œil. Les corrections sont
//! celles du moteur : l'illustration montre exactement ce que le curseur fera. À relancer quand un
//! réglage change ou s'ajoute :
//!
//!     cargo run -p engine --release --example adjust_previews -- ui/src/assets/filters/reference.jpg ui/src/assets/help

use engine::adjust::Adjustment;
use engine::document::GradientStop;
use image::imageops::FilterType;

/// Deux fois la taille affichée dans la bulle (250 × 100) : net sur un écran à forte densité.
const SIZE: (u32, u32) = (500, 200);

/// Curseurs de Développement. Tout est neutre par défaut : chaque illustration ne bouge qu'un curseur.
#[derive(Clone, Copy, Default)]
struct Dev {
    temperature: f32,
    tint: f32,
    exposure: f32,
    contrast: f32,
    highlights: f32,
    shadows: f32,
    whites: f32,
    blacks: f32,
    clarity: f32,
    dehaze: f32,
    vibrance: f32,
    saturation: f32,
    red_hue: f32,
    green_hue: f32,
    blue_hue: f32,
}

impl From<Dev> for Adjustment {
    fn from(d: Dev) -> Self {
        Adjustment::Develop {
            temperature: d.temperature,
            tint: d.tint,
            exposure: d.exposure,
            contrast: d.contrast,
            highlights: d.highlights,
            shadows: d.shadows,
            whites: d.whites,
            blacks: d.blacks,
            clarity: d.clarity,
            dehaze: d.dehaze,
            vibrance: d.vibrance,
            saturation: d.saturation,
            red_hue: d.red_hue,
            green_hue: d.green_hue,
            blue_hue: d.blue_hue,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (reference, out_dir) = (args.next().ok_or("chemin de l'image de référence manquant")?, args.next().ok_or("dossier de sortie manquant")?);
    let out_dir = std::path::PathBuf::from(out_dir);
    std::fs::create_dir_all(&out_dir)?;
    let source = image::open(&reference)?.resize_to_fill(SIZE.0, SIZE.1, FilterType::Lanczos3).into_rgb8();

    // Valeurs de démonstration : franches, pour que l'effet se lise à 250 px de large.
    let shots: [(&str, Adjustment); 29] = [
        // ---- Onglet Réglages : l'effet d'ensemble de chaque calque de réglage.
        ("develop", Dev { exposure: 0.5, contrast: 30.0, shadows: 40.0, clarity: 40.0, vibrance: 55.0, ..Dev::default() }.into()),
        ("brightnessContrast", Adjustment::BrightnessContrast { brightness: 45.0, contrast: 45.0 }),
        ("levels", Adjustment::Levels { black: 40.0, gamma: 0.7, white: 235.0 }),
        ("hueSaturation", Adjustment::HueSaturation { hue: 60.0, saturation: 40.0, lightness: 0.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 0.0, range_width: 0.0 }),
        ("invert", Adjustment::Invert),
        ("gradientMap", Adjustment::GradientMap {
            stops: vec![
                GradientStop { position: 0.0, color: [253, 194, 216, 255] },
                GradientStop { position: 0.36, color: [252, 166, 94, 255] },
                GradientStop { position: 0.67, color: [255, 121, 131, 255] },
                GradientStop { position: 1.0, color: [4, 18, 130, 255] },
            ],
            reverse: false,
            dither: false,
            smoothness: 100.0,
        }),
        // ---- Curseurs de Luminosité/Contraste, Niveaux, Teinte/Saturation.
        ("brightnessContrast-brightness", Adjustment::BrightnessContrast { brightness: 70.0, contrast: 0.0 }),
        ("brightnessContrast-contrast", Adjustment::BrightnessContrast { brightness: 0.0, contrast: 80.0 }),
        ("levels-black", Adjustment::Levels { black: 110.0, gamma: 1.0, white: 255.0 }),
        ("levels-gamma", Adjustment::Levels { black: 0.0, gamma: 0.45, white: 255.0 }),
        ("levels-white", Adjustment::Levels { black: 0.0, gamma: 1.0, white: 130.0 }),
        ("hueSaturation-hue", Adjustment::HueSaturation { hue: 110.0, saturation: 0.0, lightness: 0.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 0.0, range_width: 0.0 }),
        ("hueSaturation-saturation", Adjustment::HueSaturation { hue: 0.0, saturation: 85.0, lightness: 0.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 0.0, range_width: 0.0 }),
        ("hueSaturation-lightness", Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 45.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 0.0, range_width: 0.0 }),
        // ---- Curseurs de Développement, un par un.
        ("develop-temperature", Dev { temperature: 80.0, ..Dev::default() }.into()),
        ("develop-tint", Dev { tint: 100.0, ..Dev::default() }.into()),
        ("develop-exposure", Dev { exposure: 1.5, ..Dev::default() }.into()),
        ("develop-contrast", Dev { contrast: 80.0, ..Dev::default() }.into()),
        ("develop-highlights", Dev { highlights: -100.0, ..Dev::default() }.into()),
        ("develop-shadows", Dev { shadows: 100.0, ..Dev::default() }.into()),
        ("develop-whites", Dev { whites: 90.0, ..Dev::default() }.into()),
        ("develop-blacks", Dev { blacks: 100.0, ..Dev::default() }.into()),
        ("develop-clarity", Dev { clarity: 100.0, ..Dev::default() }.into()),
        ("develop-dehaze", Dev { dehaze: 90.0, ..Dev::default() }.into()),
        ("develop-vibrance", Dev { vibrance: 100.0, ..Dev::default() }.into()),
        ("develop-saturation", Dev { saturation: 100.0, ..Dev::default() }.into()),
        ("develop-redHue", Dev { red_hue: 100.0, ..Dev::default() }.into()),
        ("develop-greenHue", Dev { green_hue: 100.0, ..Dev::default() }.into()),
        ("develop-blueHue", Dev { blue_hue: 100.0, ..Dev::default() }.into()),
    ];

    for (name, adjustment) in shots {
        let prepared = adjustment.prepare();
        let mut out = source.clone();
        for (x, _y, px) in out.enumerate_pixels_mut() {
            if x >= SIZE.0 / 2 {
                continue; // moitié droite : l'image d'origine, pour comparer.
            }
            let rgb = prepared.apply([f32::from(px[0]) / 255.0, f32::from(px[1]) / 255.0, f32::from(px[2]) / 255.0]);
            *px = image::Rgb([to_byte(rgb[0]), to_byte(rgb[1]), to_byte(rgb[2])]);
        }
        write(&out_dir, name, &out)?;
    }


    // ---- Les deux masques, composés à la main : ce sont des illustrations, pas des réglages.
    write(&out_dir, "mask-layer", &layer_mask(&source))?;
    write(&out_dir, "mask-clip", &clipping_mask(&source))?;
    Ok(())
}

fn to_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Trait clair au milieu : sans lui, une correction douce (la nuance, les hautes lumières) se lit mal,
/// parce qu'on ne sait plus où s'arrête la moitié corrigée.
fn draw_divider(image: &mut image::RgbImage) {
    let x = SIZE.0 / 2;
    for y in 0..SIZE.1 {
        for (column, force) in [(x.saturating_sub(1), 0.25), (x, 0.55), (x + 1, 0.25)] {
            if column >= SIZE.0 {
                continue;
            }
            let px = image.get_pixel_mut(column, y);
            *px = image::Rgb([0, 1, 2].map(|c| to_byte((f32::from(px[c]) * (1.0 - force) + 255.0 * force) / 255.0)));
        }
    }
}

fn write(dir: &std::path::Path, name: &str, image: &image::RgbImage) -> Result<(), Box<dyn std::error::Error>> {
    let mut image = image.clone();
    draw_divider(&mut image);
    let image = &image;
    let path = dir.join(format!("{name}.jpg"));
    let mut file = std::io::BufWriter::new(std::fs::File::create(&path)?);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 88).encode_image(image)?;
    println!("{}", path.display());
    Ok(())
}

/// Damier gris clair / gris foncé : la transparence, comme sur le canevas.
fn checker(x: u32, y: u32) -> [u8; 3] {
    if ((x / 10) + (y / 10)) % 2 == 0 {
        [0x3a, 0x3a, 0x3c]
    } else {
        [0x2b, 0x2b, 0x2d]
    }
}

fn over(image: &image::RgbImage, x: u32, y: u32, alpha: f32) -> image::Rgb<u8> {
    let (px, back) = (image.get_pixel(x, y), checker(x, y));
    image::Rgb([0, 1, 2].map(|c| to_byte((f32::from(px[c]) * alpha + f32::from(back[c]) * (1.0 - alpha)) / 255.0)))
}

/// Masque de fusion : un dégradé noir → blanc peint dans le masque efface progressivement le calque.
fn layer_mask(source: &image::RgbImage) -> image::RgbImage {
    let mut out = source.clone();
    for (x, y, px) in out.enumerate_pixels_mut() {
        // Le dégradé ne couvre que la moitié gauche ; la droite reste le calque intact.
        let alpha = if x >= SIZE.0 / 2 { 1.0 } else { (x as f32 / (SIZE.0 as f32 / 2.0)).clamp(0.0, 1.0) };
        *px = over(source, x, y, alpha);
    }
    out
}

/// Masque d'écrêtage : le calque du dessus n'apparaît que là où le calque du dessous a des pixels
/// — ici un disque, dont le bord adouci se retrouve tel quel sur l'image écrêtée.
fn clipping_mask(source: &image::RgbImage) -> image::RgbImage {
    let (cx, cy, radius) = (SIZE.0 as f32 * 0.25, SIZE.1 as f32 * 0.5, SIZE.1 as f32 * 0.38);
    let mut out = source.clone();
    for (x, y, px) in out.enumerate_pixels_mut() {
        if x >= SIZE.0 / 2 {
            continue;
        }
        let distance = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
        let alpha = ((radius - distance) / 3.0).clamp(0.0, 1.0);
        *px = over(source, x, y, alpha);
    }
    out
}
