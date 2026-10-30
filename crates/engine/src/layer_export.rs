//! Export d'un calque ou d'un groupe seul (clic droit › Exporter) : un document isolé, recadré, puis exporté par le
//! chemin habituel (`io::export_image_with_progress`). Effets, masques, filtres dynamiques et réglages du groupe sont
//! donc rendus exactement comme dans le document ; le reste du document est simplement masqué.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::document::{Document, LayerId};
use crate::effects::reach_enabled;
use crate::error::EngineError;
use crate::io::flatten_for_export;

/// Zone exportée.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportZone {
    /// Le contenu visible du calque, effets compris, au pixel près.
    #[default]
    Content,
    /// Le cadre du document.
    Document,
    /// Le rectangle de la sélection active.
    Selection,
    /// Le cadre du plan de travail exporté (le premier calque doit en être un).
    Artboard,
}

/// Document où seuls `ids` (et leur contenu) restent visibles, recadré à `zone` plus `margin` pixels de chaque côté.
pub fn isolate(doc: &Document, ids: &[LayerId], zone: ExportZone, margin: u32) -> Result<Document, EngineError> {
    isolate_at(doc, ids, zone, margin).map(|(d, _)| d)
}

/// Comme [`isolate`], avec la position (repère document) du coin haut gauche de la zone gardée.
pub fn isolate_at(doc: &Document, ids: &[LayerId], zone: ExportZone, margin: u32) -> Result<(Document, (i32, i32)), EngineError> {
    if ids.is_empty() {
        return Err(EngineError::InvalidOperation("aucun calque à exporter".into()));
    }
    let selection_rect = doc.selection().and_then(|s| s.pixel_bounds(|px| px[0] > 0));
    let mut out = doc.clone();
    out.set_selection(None);

    // Gardés : les calques demandés, leurs descendants (dessinés) et leurs ancêtres (qui les portent).
    let mut keep: HashSet<LayerId> = HashSet::new();
    let mut stack: Vec<LayerId> = ids.to_vec();
    while let Some(id) = stack.pop() {
        if keep.insert(id) {
            stack.extend_from_slice(out.layer(id)?.children());
        }
    }
    let wanted: HashSet<LayerId> = keep.clone();
    for &id in ids {
        let mut parent = out.layer(id)?.parent;
        while let Some(p) = parent {
            keep.insert(p);
            parent = out.layer(p)?.parent;
        }
    }
    let all: Vec<LayerId> = out.walk_top_down().into_iter().map(|(_, l)| l.id).collect();
    for id in all {
        if !keep.contains(&id) {
            out.edit_props(id, |p| p.visible = false)?;
        } else if ids.contains(&id) {
            // Un calque masqué qu'on exporte explicitement est rendu quand même.
            out.edit_props(id, |p| p.visible = true)?;
        }
    }
    // Un calque écrêté dont la base n'est pas exportée : sans base, il serait invisible ; on le montre tel quel.
    for &id in ids {
        if out.layer(id)?.props.clipped && !base_kept(&out, id, &wanted) {
            out.edit_props(id, |p| p.clipped = false)?;
        }
    }

    let (w, h) = (doc.width() as i32, doc.height() as i32);
    let m = margin.min(4096) as i32;
    let rect = match zone {
        ExportZone::Document => (0, 0, w, h),
        ExportZone::Selection => selection_rect.ok_or_else(|| EngineError::InvalidOperation("aucune sélection active".into()))?,
        ExportZone::Content => content_rect(&out, ids, &wanted)?,
        ExportZone::Artboard => {
            let board = doc.layer(ids[0])?.artboard.clone().ok_or_else(|| EngineError::InvalidOperation("ce calque n'est pas un plan de travail".into()))?;
            (board.x, board.y, board.x + board.width as i32, board.y + board.height as i32)
        }
    };
    let (x0, y0, x1, y1) = rect;
    out.crop(x0 - m, y0 - m, (x1 - x0 + 2 * m).max(1) as u32, (y1 - y0 + 2 * m).max(1) as u32)?;
    Ok((out, (x0 - m, y0 - m)))
}

/// La base d'écrêtage de `id` (premier calque non écrêté sous lui, dans le même parent) est-elle exportée ?
fn base_kept(doc: &Document, id: LayerId, wanted: &HashSet<LayerId>) -> bool {
    let Ok(layer) = doc.layer(id) else { return false };
    let Ok(siblings) = doc.children_of(layer.parent) else { return false };
    let Some(at) = siblings.iter().position(|s| *s == id) else { return false };
    siblings[..at].iter().rev().find(|s| doc.layer(**s).is_ok_and(|l| !l.props.clipped)).is_some_and(|base| wanted.contains(base))
}

/// Rectangle exact (repère document) de ce que rendent les calques isolés, effets compris.
fn content_rect(isolated: &Document, ids: &[LayerId], wanted: &HashSet<LayerId>) -> Result<(i32, i32, i32, i32), EngineError> {
    let Some((x0, y0, x1, y1)) = isolated.content_bounds(ids)? else {
        return Err(EngineError::InvalidOperation("le calque est vide".into()));
    };
    // Les effets (ombre, lueur…) débordent du contenu : on compose sur un cadre élargi d'autant, puis on mesure.
    let reach = wanted.iter().filter_map(|id| isolated.layer(*id).ok()).map(|l| reach_enabled(&l.effects)).max().unwrap_or(0) + 2;
    let (cx, cy) = (x0 - reach, y0 - reach);
    let (cw, ch) = ((x1 - x0 + 2 * reach).max(1) as u32, (y1 - y0 + 2 * reach).max(1) as u32);
    let mut probe = isolated.clone();
    probe.crop(cx, cy, cw, ch)?;
    let flat = flatten_for_export(&probe, 1.0)?;
    let (fw, fh) = (flat.width as usize, flat.height as usize);
    let (mut minx, mut miny, mut maxx, mut maxy) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..fh {
        let row = &flat.rgba[y * fw * 4..(y + 1) * fw * 4];
        for (x, px) in row.chunks_exact(4).enumerate() {
            if px[3] > 0 {
                minx = minx.min(x);
                maxx = maxx.max(x + 1);
                miny = miny.min(y);
                maxy = maxy.max(y + 1);
            }
        }
    }
    if minx == usize::MAX {
        return Err(EngineError::InvalidOperation("le calque est vide".into()));
    }
    Ok((cx + minx as i32, cy + miny as i32, cx + maxx as i32, cy + maxy as i32))
}

/// Nom de fichier tiré du nom d'un calque : sans caractère interdit, jamais vide.
pub fn file_stem(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() || r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() { "calque".into() } else { trimmed.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::effects::LayerEffect;
    use crate::tiles::TileStore;

    fn square_layer(doc: &mut Document, name: &str, x: i32, y: i32, color: [u8; 4]) -> LayerId {
        let mut rgba = vec![0u8; 40 * 40 * 4];
        for px in rgba.chunks_exact_mut(4) {
            px.copy_from_slice(&color);
        }
        let id = doc.add_pixel_layer(name, TileStore::from_premultiplied(40, 40, &rgba).expect("tampon"), None, usize::MAX).expect("calque");
        doc.translate_layers(&[id], x, y).expect("position");
        id
    }

    #[test]
    fn exports_only_the_layer_cropped_to_its_content_and_effects() {
        let mut doc = Document::new("t", 300, 200, 72.0).expect("document");
        let red = square_layer(&mut doc, "Rouge", 20, 20, [255, 0, 0, 255]);
        let _blue = square_layer(&mut doc, "Bleu", 200, 100, [0, 0, 255, 255]);
        let alone = isolate(&doc, &[red], ExportZone::Content, 0).expect("isolé");
        assert_eq!((alone.width(), alone.height()), (40, 40));
        let flat = flatten_for_export(&alone, 1.0).expect("aplat");
        assert!(flat.rgba.chunks_exact(4).all(|p| p == [255, 0, 0, 255]), "le bleu est absent, le rouge plein cadre");
        // Marge : du transparent autour.
        let padded = isolate(&doc, &[red], ExportZone::Content, 5).expect("marge");
        assert_eq!((padded.width(), padded.height()), (50, 50));
        // Document entier : le cadre du document, le bleu masqué.
        let whole = isolate(&doc, &[red], ExportZone::Document, 0).expect("document");
        assert_eq!((whole.width(), whole.height()), (300, 200));
        // Effet : une ombre décalée agrandit la zone.
        let mut shadowed = doc.clone();
        let shadow = LayerEffect::DropShadow { enabled: true, color: [0, 0, 0], opacity: 1.0, angle: -45.0, distance: 10.0, size: 4.0, blend: crate::blend::BlendMode::Normal };
        shadowed.set_effects(red, vec![shadow]).expect("effets");
        let with_shadow = isolate(&shadowed, &[red], ExportZone::Content, 0).expect("ombre");
        assert!(with_shadow.width() > 40 && with_shadow.height() > 40, "{}×{}", with_shadow.width(), with_shadow.height());
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_stem("Logo / V2*"), "Logo _ V2_");
        assert_eq!(file_stem("  ..  "), "calque");
    }
}
