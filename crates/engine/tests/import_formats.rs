//! Formats d'import : chaque format accepté ouvre bien un document, avec les bonnes dimensions et
//! les bonnes couleurs. Les fixtures HEIC et DNG sont synthétiques (dégradé, mire de barres),
//! fabriquées hors de Rust ; JPEG, WebP, TIFF et PNG sont encodés ici même.

use std::io::Cursor;
use std::path::Path;

use engine::document::Document;
use engine::io::import_image_bytes;
use engine::tiles::Sampler;
use image::{ImageEncoder, ImageFormat};

/// Couleur droite d'un pixel du seul calque du document importé.
fn pixel(doc: &Document, x: i32, y: i32) -> [u8; 4] {
    let layer = doc.layer(doc.children_of(None).expect("racine")[0]).expect("calque");
    let px = Sampler::new(layer.pixels().expect("pixels")).get(x, y);
    engine::pixel::unpremultiply(px)
}

fn close(a: [u8; 4], b: [u8; 4], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(p, q)| p.abs_diff(q) <= tolerance)
}

#[test]
fn heic_photos_open_with_their_colours() {
    let bytes = include_bytes!("fixtures/degrade.heic");
    let doc = import_image_bytes(bytes, Path::new("degrade.heic")).expect("HEIC décodé");
    assert_eq!((doc.width(), doc.height()), (320, 240));
    assert_eq!(doc.name, "degrade", "le document prend le nom du fichier");
    // Rouge croissant vers la droite, vert vers le bas, bleu constant à 40 (à la tolérance du HEVC près).
    assert!(close(pixel(&doc, 10, 10), [8, 10, 40, 255], 8), "{:?}", pixel(&doc, 10, 10));
    assert!(close(pixel(&doc, 300, 230), [239, 244, 40, 255], 8), "{:?}", pixel(&doc, 300, 230));
    assert!(close(pixel(&doc, 300, 10), [239, 10, 40, 255], 8), "{:?}", pixel(&doc, 300, 10));
}

#[test]
fn dng_negatives_are_developed_and_oriented() {
    let bytes = include_bytes!("fixtures/mire.dng");
    let doc = import_image_bytes(bytes, Path::new("mire.dng")).expect("DNG développé");
    // Capteur 640 × 480 moins le rognage par défaut de 8 px, orientation 6 (à tourner de 90°) :
    // l'image ouverte est en portrait.
    assert_eq!((doc.width(), doc.height()), (464, 624));
    // La mire : huit barres verticales (blanc, jaune, cyan, vert, magenta, rouge, bleu, gris sombre),
    // éclaircies vers le bas du capteur. Tournée de 90° horaire, le bas du capteur est à gauche,
    // et la première barre (blanc) en haut. La balance des blancs de la prise de vue a été défaite :
    // la barre blanche ressort neutre, la barre rouge, rouge. On lit du côté sombre (à droite) :
    // la mire est surexposée du côté clair, et les couleurs y écrêtent en pastel, comme sur un vrai raw.
    let white = pixel(&doc, 300, 30);
    assert!(white[0] > 120 && white[0].abs_diff(white[1]) < 20 && white[1].abs_diff(white[2]) < 20, "barre blanche neutre : {white:?}");
    let red = pixel(&doc, 440, 430);
    assert!(red[0] > 150 && red[1] < 90 && red[2] < 90, "barre rouge : {red:?}");
    let blue = pixel(&doc, 300, 510);
    assert!(blue[2] > 150 && blue[0] < 90 && blue[1] < 90, "barre bleue : {blue:?}");
    // Le haut du capteur (à droite une fois tourné) est plus sombre que le bas (à gauche).
    assert!(pixel(&doc, 444, 30)[1] < pixel(&doc, 20, 30)[1], "rampe de luminosité conservée");
}

#[test]
fn jpeg_webp_tiff_and_png_still_open() {
    let mut source = image::RgbaImage::new(64, 48);
    for (x, y, px) in source.enumerate_pixels_mut() {
        *px = image::Rgba([(x * 4) as u8, (y * 5) as u8, 200, 255]);
    }
    for (format, name, tolerance) in [(ImageFormat::Jpeg, "photo.jpg", 6), (ImageFormat::WebP, "photo.webp", 0), (ImageFormat::Tiff, "photo.tif", 0), (ImageFormat::Png, "photo.png", 0)] {
        let mut bytes = Vec::new();
        match format {
            ImageFormat::WebP => image::codecs::webp::WebPEncoder::new_lossless(&mut bytes).write_image(source.as_raw(), 64, 48, image::ExtendedColorType::Rgba8).expect("webp"),
            // Le JPEG n'a pas d'alpha.
            ImageFormat::Jpeg => image::DynamicImage::ImageRgba8(source.clone()).to_rgb8().write_to(&mut Cursor::new(&mut bytes), format).expect("jpeg"),
            _ => source.write_to(&mut Cursor::new(&mut bytes), format).expect("encodé"),
        }
        let doc = import_image_bytes(&bytes, Path::new(name)).unwrap_or_else(|e| panic!("{name} : {e}"));
        assert_eq!((doc.width(), doc.height()), (64, 48), "{name}");
        assert!(close(pixel(&doc, 40, 20), [160, 100, 200, 255], tolerance), "{name} : {:?}", pixel(&doc, 40, 20));
    }
}

#[test]
fn a_plain_tiff_is_not_mistaken_for_a_dng() {
    let source = image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
    let mut bytes = Vec::new();
    source.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Tiff).expect("tiff");
    let doc = import_image_bytes(&bytes, Path::new("sans-extension")).expect("TIFF ordinaire");
    assert_eq!(pixel(&doc, 3, 3), [10, 20, 30, 255]);
}
