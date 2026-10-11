//! Les calques de réglage doivent modifier l'image composée (panneaux Réglages et Développement).
use engine::adjust::Adjustment;
use engine::blend::BlendMode;
use engine::document::{Background, Document};
use engine::render::TileRenderer;
use engine::tiles::TileCoord;

const ORIGIN: TileCoord = TileCoord::new(0, 0);

fn rendered(doc: &Document) -> [u8; 4] {
    TileRenderer::new().tile(doc, 0, ORIGIN).map_or([0; 4], |t| t.pixel(10, 10))
}

#[test]
fn an_adjustment_layer_changes_the_picture() {
    let mut doc = Document::with_background("x", 256, 256, 72.0, Background::White).expect("document");
    assert_eq!(rendered(&doc), [255; 4], "fond blanc");
    // Inverser laisse le papier (blanc pur) blanc : c'est Négatif qui le rend noir.
    let paper = doc.add_adjustment_layer(Adjustment::Invert, None, 1, false).expect("réglage");
    assert_eq!(rendered(&doc), [255; 4], "Inverser épargne le blanc pur");
    doc.remove_layers(&[paper]).expect("retrait");
    let layer = doc.add_adjustment_layer(Adjustment::Negative, None, 1, false).expect("réglage");
    assert_eq!(rendered(&doc), [0, 0, 0, 255], "négatif du blanc");

    doc.edit_props(layer, |p| p.opacity = 0.5).expect("opacité");
    assert_eq!(rendered(&doc), [128, 128, 128, 255], "à moitié");

    doc.edit_props(layer, |p| (p.opacity, p.visible) = (1.0, false)).expect("visibilité");
    assert_eq!(rendered(&doc), [255; 4], "invisible : rien");

    doc.set_adjustment(layer, Adjustment::Negative).expect("même type");
    doc.edit_props(layer, |p| (p.visible, p.blend) = (true, BlendMode::Multiply)).expect("mode");
    assert_eq!(rendered(&doc), [0, 0, 0, 255], "blanc x noir = noir");
}

#[test]
fn a_develop_layer_changes_the_picture() {
    // -3,24 IL est une des valeurs mesurées sur Lightroom : à celles-là, le rendu est exact.
    // Entre deux mesures il est interpolé, et l'écart y monte — d'où le choix d'une valeur mesurée.
    let exposure = |ev: f32| Adjustment::Develop {
        temperature: 0.0, tint: 0.0, exposure: ev, contrast: 0.0, highlights: 0.0, shadows: 0.0, whites: 0.0, blacks: 0.0,
        clarity: 0.0, dehaze: 0.0, vibrance: 0.0, saturation: 0.0, red_hue: 0.0, green_hue: 0.0, blue_hue: 0.0,
    };
    let mut doc = Document::with_background("x", 256, 256, 72.0, Background::White).expect("document");
    let layer = doc.add_adjustment_layer(exposure(-3.24), None, 1, false).expect("réglage");
    let px = rendered(&doc);
    assert!(px[0] < 140, "-3,24 IL assombrit nettement le blanc : {px:?}");
    assert!(px[0].abs_diff(px[2]) <= 2, "et ne le teinte pas : {px:?}");

    // Plus on descend, plus c'est sombre : l'interpolation ne doit jamais s'inverser.
    let mut last = 255u8;
    for ev in [-0.5, -1.0, -2.0, -3.24, -5.0] {
        doc.set_adjustment(layer, exposure(ev)).expect("même type");
        let value = rendered(&doc)[0];
        assert!(value <= last, "{ev} IL éclaircit alors qu'il devrait assombrir : {value} après {last}");
        last = value;
    }
}
