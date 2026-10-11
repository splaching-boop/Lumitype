//! Plafond de mémoire vive, comme le réglage « Utilisation de la mémoire » de Photoshop.
//!
//! Lumitype ne réserve rien d'avance : il prend ce dont il a besoin et ne dépasse jamais le plafond fixé
//! par les préférences (une part de la mémoire vive de la machine). La mesure de base est
//! [`crate::tiles::resident_bytes`], le compte exact des octets de toutes les tuiles vivantes, où qu'elles
//! soient : documents ouverts, états d'annulation (qui ne possèdent en propre que ce qui a changé) et
//! caches de rendu.
//!
//! Quand on s'approche du plafond, deux leviers, dans cet ordre :
//! 1. le **cache de rendu** est dynamique : il n'a droit qu'à ce que le plafond laisse une fois comptées
//!    les tuiles qui ne sont pas du cache, avec un minimum de [`MIN_CACHE_BUDGET`] (voir
//!    `render::cache_budget_for`). Vidé, il se reconstruit à la demande : rien n'est perdu. Les caches de
//!    tous les documents ouverts sont inscrits ici ([`register_caches`]) : une session qui grossit serre
//!    aussi ceux des documents qui ne sont pas à l'écran ([`trim_caches`]) ;
//! 2. si les documents et leur historique dépassent **à eux seuls** le plafond (caches mis à part :
//!    [`cache_bytes_total`]), la session qui vient d'être modifiée envoie ses états d'annulation anciens sur le
//!    **disque de travail** (module `scratch`) : leurs tuiles quittent la mémoire et seront relues si l'on y revient ;
//! 3. seulement si le disque de travail est désactivé, plein ou en erreur, elle abandonne ses plus anciens états,
//!    jamais moins de [`MIN_HISTORY`] et jamais l'état courant (voir `Session::enforce_memory_cap`).
//!
//! Rien ne tourne en boucle : le cache se vérifie à chaque tuile ajoutée, tout le reste après chaque édition.
//! Le bilan [`memory_status`] est calculé à la demande (l'interface l'affiche une fois par seconde, boîte des
//! préférences ouverte).

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use crate::document::Document;
use crate::render::Caches;
use crate::session::Session;
use crate::tiles::resident_bytes;

/// Plafond en octets ; 0 = aucun plafond (le cache garde alors son budget fixe, s'il y en a un).
static MEMORY_CAP: AtomicU64 = AtomicU64::new(0);

/// Ce que le cache de rendu garde au minimum, même sous le plafond : en dessous, l'affichage recalculerait
/// sans cesse les mêmes tuiles.
pub const MIN_CACHE_BUDGET: u64 = 128 * 1024 * 1024;
/// États d'annulation toujours conservés, quel que soit le plafond.
pub const MIN_HISTORY: usize = 5;

pub fn set_memory_cap(bytes: u64) {
    MEMORY_CAP.store(bytes, Ordering::Relaxed);
}

pub fn memory_cap() -> u64 {
    MEMORY_CAP.load(Ordering::Relaxed)
}

/// Caches de rendu de tous les documents ouverts (références faibles : un document fermé en sort tout seul).
/// Grâce à lui, une session qui grossit peut serrer les caches des AUTRES documents avant de toucher à son
/// historique, et l'élagage de l'historique ne compte jamais un cache comme de l'historique.
static CACHES: Mutex<Vec<Weak<Caches>>> = Mutex::new(Vec::new());

/// Inscrit les caches d'un nouveau `TileRenderer`. Les entrées mortes sont balayées au passage.
pub(crate) fn register_caches(caches: &Arc<Caches>) {
    let mut registry = CACHES.lock().unwrap_or_else(PoisonError::into_inner);
    registry.retain(|weak| weak.strong_count() > 0);
    registry.push(Arc::downgrade(caches));
}

/// Les caches encore vivants, hors verrou : serrer un cache prend ses propres verrous, jamais celui du registre.
fn live_caches() -> Vec<Arc<Caches>> {
    CACHES.lock().unwrap_or_else(PoisonError::into_inner).iter().filter_map(Weak::upgrade).collect()
}

/// Octets de tous les caches de rendu du processus (même estimation que `TileRenderer::cache_bytes`).
pub fn cache_bytes_total() -> u64 {
    live_caches().iter().map(|caches| caches.bytes()).sum()
}

/// Fait respecter leur budget du moment à tous les caches de rendu, ceux des documents à l'écran comme
/// ceux des autres : appelé après chaque édition et quand le plafond change (voir `Session::enforce_memory_cap`).
pub fn trim_caches() {
    for caches in live_caches() {
        caches.trim();
    }
}

/// Répartition de la mémoire occupée par les tuiles, en octets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryStatus {
    /// Tuiles de l'état courant de chaque session ouverte (les tuiles partagées entre documents comptent une fois).
    pub documents_bytes: u64,
    /// Ce que les états d'annulation possèdent en propre : le reste, une fois documents et caches retirés.
    pub history_bytes: u64,
    /// Caches de rendu de toutes les sessions (estimation par comptage de leurs entrées).
    pub cache_bytes: u64,
    /// Total exact : toutes les tuiles vivantes du processus.
    pub resident_bytes: u64,
    /// Octets des fichiers d'échange (états d'historique partis sur le disque de travail), hors mémoire vive.
    pub scratch_bytes: u64,
}

/// Bilan à cet instant pour les sessions ouvertes. Le coût est proportionnel au nombre de tuiles des documents
/// courants (un ensemble d'adresses), pas à la taille de l'historique : c'est fait pour un appel par seconde.
pub fn memory_status<'a>(sessions: impl IntoIterator<Item = &'a Session>) -> MemoryStatus {
    let mut seen = HashSet::new();
    let (mut documents_bytes, mut cache_bytes) = (0, 0);
    for session in sessions {
        documents_bytes += document_tile_bytes(session.document(), &mut seen);
        cache_bytes += session.renderer().cache_bytes();
    }
    let resident = resident_bytes();
    // Les caches peuvent contenir des tuiles du document lui-même (niveau 0, calque unique sans effet) :
    // l'estimation de l'historique est donc bornée par zéro plutôt que laissée négative.
    MemoryStatus {
        documents_bytes,
        history_bytes: resident.saturating_sub(documents_bytes + cache_bytes),
        cache_bytes,
        resident_bytes: resident,
        scratch_bytes: crate::scratch::scratch_bytes(),
    }
}

/// Octets des tuiles d'un document (pixels, masques, sélection) absentes de `seen`, qui s'enrichit au passage.
/// Le contenu d'un objet dynamique (son document, son aplat et ses réductions) compte avec le document qui
/// l'utilise, une seule fois quel que soit le nombre de ses instances : les tuiles sont reconnues à leur adresse,
/// et le contenu lui-même aussi (son adresse rejoint `seen`, il n'est donc parcouru qu'une fois).
pub(crate) fn document_tile_bytes(doc: &Document, seen: &mut HashSet<usize>) -> u64 {
    let mut bytes = 0;
    for layer in doc.nodes() {
        if let Some(pixels) = layer.pixels() {
            bytes += pixels.unique_tile_bytes(seen);
        }
        if let Some(mask) = &layer.mask {
            bytes += mask.pixels.unique_tile_bytes(seen);
        }
        if let Some(content) = layer.smart() {
            if seen.insert(std::sync::Arc::as_ptr(content) as usize) {
                bytes += document_tile_bytes(&content.document, seen);
                bytes += content.flattened.unique_tile_bytes(seen);
                for level in content.cached_levels() {
                    bytes += level.unique_tile_bytes(seen);
                }
            }
        }
    }
    if let Some(selection) = doc.selection() {
        bytes += selection.unique_tile_bytes(seen);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Background;
    use crate::tiles::{Tile, TileStore};

    #[test]
    fn the_status_counts_current_documents_once_and_puts_the_rest_in_history() {
        let document = Document::with_background("x", 512, 256, 72.0, Background::White).expect("document");
        let mut session = Session::new(document, "Nouveau");
        // Le fond blanc : une seule tuile pleine partagée par les deux positions.
        let status = memory_status([&session]);
        assert_eq!(status.documents_bytes, Tile::BYTES);
        // Un calque de plus (une tuile propre) : l'ancien état ne possède rien que le nouveau n'ait aussi.
        let mut layer = TileStore::new();
        layer.get_mut(crate::tiles::TileCoord::new(0, 0)).set_pixel(1, 1, [255; 4]);
        session.apply("Calque", None, |doc| doc.add_pixel_layer("a", layer, None, 1)).expect("calque");
        let status = memory_status([&session]);
        assert_eq!(status.documents_bytes, 2 * Tile::BYTES);
        assert!(status.history_bytes <= status.resident_bytes, "l'historique est ce qui reste du total");
    }
}
