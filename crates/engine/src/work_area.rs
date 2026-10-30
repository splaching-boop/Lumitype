//! Plan de travail isolé (la liste en haut du panneau Calques) : seul ce plan doit réagir.
//!
//! L'interface n'affiche plus que le plan choisi, mais les autres plans restent là, à côté : sans garde, un clic, un
//! trait ou Suppr juste hors du plan visible modifiait un plan caché. La session reçoit donc le plan isolé
//! ([`crate::Session::set_work_area`]) et, après chaque modification :
//! - les calques qui n'appartiennent PAS au plan isolé ne peuvent changer que DANS son cadre : leurs pixels et
//!   leur masque hors du cadre reprennent leur état d'avant (un calque hors plan visible à travers le plan, s'il y
//!   en a, se retouche donc normalement là où on le voit) ;
//! - les calques du plan isolé sont libres (ils sont découpés par le plan, et un contenu qui déborde du cadre doit
//!   pouvoir être déplacé ou transformé sans perte) ;
//! - la sélection est limitée au cadre du plan.
//!
//! La règle est appliquée par la session elle-même, après coup : elle vaut pour tous les outils et toutes les
//! commandes, présents et à venir, sans qu'aucun n'ait à y penser.

use std::sync::Arc;

use crate::document::{Document, LayerId};
use crate::selection::{combine, CombineMode, Shape};
use crate::tiles::{Store, TileBuf, TILE_SIZE};

impl Document {
    /// Cadre du plan de travail `board` en pixels du document `[x0, y0, x1, y1]` (bornes hautes exclues).
    pub fn artboard_rect(&self, board: LayerId) -> Option<[i32; 4]> {
        let a = self.layer(board).ok()?.artboard.as_ref()?;
        Some([a.x, a.y, a.x + a.width as i32, a.y + a.height as i32])
    }

    /// `id` est-il le plan `board` ou l'un de ses descendants ?
    pub fn is_inside(&self, id: LayerId, board: LayerId) -> bool {
        let mut at = Some(id);
        while let Some(current) = at {
            if current == board {
                return true;
            }
            at = self.layer(current).ok().and_then(|l| l.parent);
        }
        false
    }

    /// Ramène `self` (l'état après une modification) à la règle du plan isolé, par rapport à `before`.
    /// Sans effet si `board` n'est pas (ou plus) un plan de travail.
    pub fn confine_to_board(&mut self, before: &Document, board: LayerId) {
        let Some(rect) = before.artboard_rect(board).or_else(|| self.artboard_rect(board)) else { return };
        let ids: Vec<LayerId> = self.nodes().map(|l| l.id).collect();
        for id in ids {
            if self.is_inside(id, board) {
                continue;
            }
            let (Ok(old), Ok(new)) = (before.layer(id), self.layer(id)) else { continue };
            if old.offset != new.offset {
                continue; // déplacé : un geste voulu sur ce calque, pas une retouche de pixels
            }
            let offset = new.offset;
            let local = [rect[0] - offset.0, rect[1] - offset.1, rect[2] - offset.0, rect[3] - offset.1];
            let old_pixels = old.pixels().cloned();
            let old_mask = old.mask.as_ref().map(|m| m.pixels.clone());
            let new_pixels_changed = match (&old_pixels, new.pixels()) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                _ => false,
            };
            let new_mask_changed = match (&old_mask, new.mask.as_ref()) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, &b.pixels),
                _ => false,
            };
            let paintable = new.text().is_none() && new.shape().is_none() && new.smart().is_none() && new.fill_content().is_none() && new.filters.is_none();
            if new_pixels_changed && paintable {
                if let (Some(old_pixels), Some(slot)) = (old_pixels, self.pixels_slot(id)) {
                    keep_inside(&old_pixels, slot, local);
                }
            }
            if new_mask_changed {
                if let (Some(old_mask), Some(slot)) = (old_mask, self.mask_slot(id)) {
                    keep_inside(&old_mask, slot, local);
                }
            }
        }
        // Sélection : limitée au cadre du plan.
        let changed = match (before.selection(), self.selection()) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        if changed {
            if let Some(selection) = self.selection().cloned() {
                let (w, h) = (self.width(), self.height());
                let frame = Shape::Rect { x: rect[0] as f32, y: rect[1] as f32, width: (rect[2] - rect[0]) as f32, height: (rect[3] - rect[1]) as f32 }.rasterize(w, h);
                let clipped = combine(Some(&selection), frame, CombineMode::Intersect);
                self.set_selection(clipped.filter(|s| s.tile_count() > 0 || s.background() != [0]));
            }
        }
    }
}

/// Garde dans `new` ce qui a changé DANS `[x0, y0, x1, y1]` (repère local) ; ailleurs, `old` revient. Si rien ne
/// change au final (toute la modification était hors du cadre), le stockage d'origine reprend sa place, même
/// version : l'affichage n'a rien à recharger.
fn keep_inside<const C: usize>(old: &Arc<Store<C>>, slot: &mut Arc<Store<C>>, [x0, y0, x1, y1]: [i32; 4]) {
    let size = TILE_SIZE as i32;
    let changed = slot.changed_coords(old);
    if changed.is_empty() {
        return;
    }
    let new = Arc::make_mut(slot);
    for coord in changed {
        let (tx, ty) = coord.origin();
        let (ix0, iy0, ix1, iy1) = (x0.max(tx), y0.max(ty), x1.min(tx + size), y1.min(ty + size));
        let before = old.get(coord).cloned();
        if ix0 >= ix1 || iy0 >= iy1 {
            // Tuile entièrement hors du cadre : elle revient telle quelle.
            new.restore(coord, before);
            continue;
        }
        if ix0 == tx && iy0 == ty && ix1 == tx + size && iy1 == ty + size {
            continue; // entièrement dans le cadre
        }
        let fresh: TileBuf<C> = new.get(coord).map_or_else(|| TileBuf::filled(new.background()), |t| (**t).clone());
        let mut merged: TileBuf<C> = before.as_deref().cloned().unwrap_or_else(|| TileBuf::filled(old.background()));
        for y in iy0..iy1 {
            for x in ix0..ix1 {
                let (lx, ly) = ((x - tx) as u32, (y - ty) as u32);
                merged.set_pixel(lx, ly, fresh.pixel(lx, ly));
            }
        }
        // Même contenu qu'avant (la retouche n'entrait pas dans le cadre) : la tuile d'origine revient, partagée.
        let same = match &before {
            Some(tile) => tile.bytes() == merged.bytes(),
            None => merged.is_uniform(old.background()),
        };
        if same {
            new.restore(coord, before);
        } else {
            new.insert(coord, merged);
        }
    }
    if new.changed_coords(old).is_empty() {
        *slot = Arc::clone(old);
    }
}

#[cfg(test)]
mod tests {
    use crate::document::{Artboard, Document};
    use crate::session::Session;
    use crate::tiles::{Sampler, TileStore};

    fn board(x: i32) -> Artboard {
        Artboard { x, y: 0, width: 100, height: 100, background: None, clip: true, preset: None, background_layer: None }
    }

    #[test]
    fn an_isolated_board_protects_the_hidden_neighbour() {
        let mut doc = Document::new("plans", 220, 100, 72.0).unwrap();
        let first = doc.add_artboard("Plan 1", board(0)).unwrap();
        let second = doc.add_artboard("Plan 2", board(120)).unwrap();
        // Calque du plan 2 qui déborde sous le plan 1 (comme une image posée à cheval).
        let mut pixels = TileStore::new();
        pixels.fill(220, 100, [0, 0, 255, 255]);
        let wide = doc.add_pixel_layer("plan 2 à cheval", pixels, Some(second), 0).unwrap();
        let mut pixels = TileStore::new();
        pixels.fill(100, 100, [0, 255, 0, 255]);
        let inside = doc.add_pixel_layer("dans le plan 1", pixels, Some(first), 0).unwrap();
        let mut session = Session::new(doc, "Nouveau");
        session.set_work_area(Some(first));
        // Remplir le calque commun en entier : seule la partie du plan 1 change.
        session.apply("Remplir", None, |doc| doc.fill_layer(wide, [255, 0, 0])).unwrap();
        let doc = session.document();
        let store = doc.layer(wide).unwrap().pixels().unwrap().clone();
        let mut s = Sampler::new(&store);
        assert_eq!(s.get(50, 50), [255, 0, 0, 255], "dans le plan isolé");
        assert_eq!(s.get(150, 50), [0, 0, 255, 255], "plan 2 caché : intact");
        // Modification entièrement hors du plan isolé : le calque garde son stockage d'origine (même version).
        let version = store.version();
        session.apply("Pixel", None, |doc| doc.pixels_mut(wide).map(|p| p.get_mut(crate::tiles::TileCoord::new(0, 0)).set_pixel(150, 50, [7, 7, 7, 255]))).unwrap();
        let after = session.document().layer(wide).unwrap().pixels().unwrap();
        assert_eq!(after.version(), version, "rien n'a changé dans le plan : même stockage");
        assert_eq!(Sampler::new(after).get(150, 50), [0, 0, 255, 255]);
        // Les calques du plan isolé restent libres, même hors de son cadre.
        session.apply("Remplir", None, |doc| doc.fill_layer(inside, [9, 9, 9])).unwrap();
        // Une sélection de tout le document est ramenée au plan.
        session.apply("Tout", None, |doc| {
            doc.set_selection(Some(crate::selection::Shape::Rect { x: 0.0, y: 0.0, width: 220.0, height: 100.0 }.rasterize(220, 100)));
            Ok(())
        }).unwrap();
        let bounds = session.document().selection().unwrap().pixel_bounds(|v| v[0] > 0).unwrap();
        assert_eq!(bounds.2, 100, "sélection coupée au bord du plan");
        // Sans plan isolé, plus de garde.
        session.set_work_area(None);
        session.apply("Désélectionner", None, |doc| {
            doc.set_selection(None);
            Ok(())
        })
        .unwrap();
        session.apply("Remplir", None, |doc| doc.fill_layer(wide, [1, 2, 3])).unwrap();
        let store = session.document().layer(wide).unwrap().pixels().unwrap().clone();
        assert_eq!(Sampler::new(&store).get(150, 50), [1, 2, 3, 255]);
    }

    #[test]
    fn exporting_one_artboard_crops_to_its_frame() {
        let mut doc = Document::new("plans", 220, 100, 72.0).unwrap();
        let _first = doc.add_artboard("Plan 1", board(0)).unwrap();
        let second = doc.add_artboard("Plan 2", Artboard { width: 80, height: 60, ..board(120) }).unwrap();
        let options = crate::io::ExportOptions { artboard: Some(second), ..crate::io::ExportOptions::new(crate::io::ExportFormat::Png) };
        let view = crate::io::export_view(&doc, &options).unwrap();
        assert_eq!((view.width(), view.height()), (80, 60));
        assert_eq!((doc.width(), doc.height()), (220, 100), "le document lui-même ne bouge pas");
        let missing = crate::io::ExportOptions { artboard: Some(9999), ..options };
        assert!(crate::io::export_view(&doc, &missing).is_err());
    }
}
