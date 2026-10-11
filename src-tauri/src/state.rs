//! État partagé entre les commandes.
//!
//! Chaque document ouvert est une [`Session`] du moteur derrière un `RwLock` :
//! - les demandes de tuiles prennent le verrou en **lecture** et se calculent en parallèle ;
//! - une modification prend le verrou en **écriture**, donc jamais au milieu d'un rendu.
//!
//! L'ordre des modifications (indispensable pour les points d'un trait) est garanti
//! côté interface, qui attend la réponse d'une modification avant d'envoyer la suivante.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use engine::brushes::BrushTip;
use engine::document::Clip;
use engine::{Document, Session};

pub type DocId = u32;

/// Document ouvert par « Modifier le contenu » : le contenu `content` d'un objet dynamique du document `parent`.
/// L'enregistrer (`smart_content_commit`) remplace ce contenu dans le parent, pour toutes ses instances.
#[derive(Clone, Debug)]
pub struct SmartLink {
    pub parent: DocId,
    pub content: engine::smart::ContentId,
    /// Nom du calque objet dynamique au moment de l'ouverture (titre, messages).
    pub name: String,
}

pub struct OpenDocument {
    session: RwLock<Session>,
    /// Chemin du projet, absent pour un nouveau document ou une image importée.
    project_path: RwLock<Option<PathBuf>>,
    /// Présent pour le contenu d'un objet dynamique ouvert comme document.
    smart_parent: RwLock<Option<SmartLink>>,
}

impl OpenDocument {
    // Un verrou empoisonné reste utilisable : une session n'est modifiée que par
    // des opérations qui la laissent cohérente même en cas d'erreur.
    pub fn read(&self) -> RwLockReadGuard<'_, Session> {
        self.session.read().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, Session> {
        self.session.write().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn project_path(&self) -> Option<PathBuf> {
        self.project_path.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn set_project_path(&self, path: PathBuf) {
        *self.project_path.write().unwrap_or_else(PoisonError::into_inner) = Some(path);
    }

    pub fn smart_parent(&self) -> Option<SmartLink> {
        self.smart_parent.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn set_smart_parent(&self, link: SmartLink) {
        *self.smart_parent.write().unwrap_or_else(PoisonError::into_inner) = Some(link);
    }
}

#[derive(Default)]
pub struct Inner {
    documents: RwLock<HashMap<DocId, Arc<OpenDocument>>>,
    next_id: AtomicU32,
    /// Presse-papiers interne, commun à tous les documents : on copie dans l'un, on colle dans l'autre.
    clipboard: RwLock<Option<Clip>>,
    /// Calques copiés (Ctrl+C dans le panneau Calques), collables dans n'importe quel document.
    layer_clipboard: RwLock<Option<engine::document::LayerClip>>,
    /// Pointes de pinceau importées (ABR, GBR, images), par identifiant.
    brushes: RwLock<Vec<(u32, Arc<BrushTip>)>>,
}

/// Clonable à faible coût pour être déplacé dans les tâches bloquantes.
#[derive(Clone, Default)]
pub struct AppState(Arc<Inner>);

impl AppState {
    pub fn insert(&self, document: Document, label: &str, project_path: Option<PathBuf>) -> (DocId, Arc<OpenDocument>) {
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let entry = Arc::new(OpenDocument {
            session: RwLock::new(Session::new(document, label)),
            project_path: RwLock::new(project_path),
            smart_parent: RwLock::new(None),
        });
        self.0.documents.write().unwrap_or_else(PoisonError::into_inner).insert(id, Arc::clone(&entry));
        (id, entry)
    }

    /// Document reconstitué avec tout son historique (récupération après une fermeture imprévue).
    pub fn insert_session(&self, session: Session, project_path: Option<PathBuf>) -> (DocId, Arc<OpenDocument>) {
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let entry = Arc::new(OpenDocument { session: RwLock::new(session), project_path: RwLock::new(project_path), smart_parent: RwLock::new(None) });
        self.0.documents.write().unwrap_or_else(PoisonError::into_inner).insert(id, Arc::clone(&entry));
        (id, entry)
    }

    pub fn get(&self, id: DocId) -> Option<Arc<OpenDocument>> {
        self.0.documents.read().unwrap_or_else(PoisonError::into_inner).get(&id).cloned()
    }

    /// Document déjà ouvert pour le contenu `content` de l'objet dynamique du document `parent`, s'il y en a un.
    pub fn smart_child(&self, parent: DocId, content: engine::smart::ContentId) -> Option<(DocId, Arc<OpenDocument>)> {
        let documents = self.0.documents.read().unwrap_or_else(PoisonError::into_inner);
        documents
            .iter()
            .find(|(_, entry)| entry.smart_parent().is_some_and(|link| link.parent == parent && link.content == content))
            .map(|(id, entry)| (*id, Arc::clone(entry)))
    }

    /// Ajoute des pointes à la bibliothèque et retourne leurs identifiants.
    pub fn add_brushes(&self, tips: Vec<BrushTip>) -> Vec<(u32, Arc<BrushTip>)> {
        let mut library = self.0.brushes.write().unwrap_or_else(PoisonError::into_inner);
        let mut next = library.last().map_or(1, |(id, _)| id + 1);
        let added: Vec<(u32, Arc<BrushTip>)> = tips
            .into_iter()
            .map(|tip| {
                let entry = (next, Arc::new(tip));
                next += 1;
                entry
            })
            .collect();
        library.extend(added.iter().cloned());
        added
    }

    pub fn brushes(&self) -> Vec<(u32, Arc<BrushTip>)> {
        self.0.brushes.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn brush(&self, id: u32) -> Option<Arc<BrushTip>> {
        self.brushes().into_iter().find(|(tip_id, _)| *tip_id == id).map(|(_, tip)| tip)
    }

    pub fn set_layer_clipboard(&self, clip: engine::document::LayerClip) {
        *self.0.layer_clipboard.write().unwrap_or_else(PoisonError::into_inner) = Some(clip);
    }

    pub fn layer_clipboard(&self) -> Option<engine::document::LayerClip> {
        self.0.layer_clipboard.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Tous les documents ouverts.
    pub fn all(&self) -> Vec<Arc<OpenDocument>> {
        self.0.documents.read().unwrap_or_else(PoisonError::into_inner).values().cloned().collect()
    }

    pub fn clipboard(&self) -> Option<Clip> {
        self.0.clipboard.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn set_clipboard(&self, clip: Clip) {
        *self.0.clipboard.write().unwrap_or_else(PoisonError::into_inner) = Some(clip);
    }

    pub fn remove(&self, id: DocId) {
        self.0.documents.write().unwrap_or_else(PoisonError::into_inner).remove(&id);
    }
}
