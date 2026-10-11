//! Commandes exposées à l'interface.
//!
//! Règle : tout travail sur des pixels ou des fichiers part dans
//! `spawn_blocking`. Ni le thread de l'interface ni la boucle asynchrone de
//! Tauri ne sont bloqués.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use engine::adjust::Adjustment;
use engine::document::{Background, Clip, GradientStop, Guide, LayerLabel, Locks, MovePlan, Orientation};
use engine::effects::LayerEffect;
use engine::filters::Filter;
use engine::io::{self, LEGACY_PROJECT_EXTENSION, PROJECT_EXTENSION};
use engine::shape::ShapeData;
use engine::paint::{Brush, BrushDynamics, BrushKind, BrushMode, StrokePoint};
use engine::path::SubPath;
use engine::selection::{self, CombineMode, Shape};
use engine::text::{self, TextData};
use engine::transform::{Matrix, Quality};
use engine::{BlendMode, Dirty, Document, EngineError, LayerId, Session, TileCoord, TileRenderer, TILE_SIZE};
use serde::{Deserialize, Serialize, Serializer};
use tauri::ipc::Response;
use tauri::State;

use crate::state::{AppState, DocId, OpenDocument, SmartLink};
use engine::document::SmartFit;
use engine::smart::{self, ContentId};

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("document {0} introuvable")]
    UnknownDocument(DocId),
    #[error("tâche interrompue")]
    Interrupted,
}

/// L'interface reçoit un message lisible, causes comprises.
impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut message = self.to_string();
        let mut cause = self.source();
        while let Some(inner) = cause {
            message.push_str(" — ");
            message.push_str(&inner.to_string());
            cause = inner.source();
        }
        serializer.serialize_str(&message)
    }
}

// ------------------------------------------------------------------ description

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerInfo {
    id: LayerId,
    parent: Option<LayerId>,
    /// Position parmi ses frères, du bas (0) vers le haut : l'indice attendu par `move`.
    index: usize,
    depth: usize,
    /// Position du calque dans le document.
    offset: (i32, i32),
    name: String,
    is_group: bool,
    expanded: bool,
    child_count: usize,
    visible: bool,
    opacity: f32,
    fill: f32,
    blend: BlendMode,
    locks: Locks,
    clipped: bool,
    has_mask: bool,
    mask_enabled: bool,
    /// Présent pour un calque de texte : son contenu éditable.
    text: Option<TextData>,
    /// Présent pour un calque de réglage : son type et ses paramètres.
    adjustment: Option<Adjustment>,
    /// Présent pour un calque de forme : sa description réglable.
    shape: Option<ShapeData>,
    /// Effets de calque (fx).
    effects: Vec<LayerEffect>,
    /// Couleur d'étiquette dans le panneau (`null` : aucune ; un dossier teinte ses enfants sans étiquette).
    label: Option<LayerLabel>,
    /// Présent pour un objet dynamique.
    smart: Option<SmartInfo>,
}

/// Objet dynamique, tel que l'interface l'affiche (panneau Calques, barre d'options de la transformation).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartInfo {
    /// Identifiant du contenu : les instances (Ctrl+J) partagent le même.
    content_id: ContentId,
    /// Contenu lié à un fichier externe (sinon incorporé au projet).
    linked: bool,
    /// Fichier lié (chemin absolu).
    path: Option<String>,
    /// Fichier lié introuvable à la dernière vérification : le dernier rendu reste affiché.
    missing: bool,
    /// Nom du contenu.
    name: String,
    /// Dimensions du contenu en pixels.
    width: u32,
    height: u32,
    /// Nombre de calques du document qui partagent ce contenu (lui compris).
    instances: u32,
    /// Image d'origine d'un contenu incorporé par « Placer », tant qu'il n'a pas été modifié : nom du fichier.
    original_name: Option<String>,
    /// Nom de fichier proposé par « Exporter le contenu » : l'image d'origine, le fichier lié, ou `<nom>.graf`.
    export_name: String,
    /// Échelles (1 = 100 %, négative : retourné), rotation en degrés (sens horaire à l'écran).
    scale_x: f64,
    scale_y: f64,
    rotation_deg: f64,
    /// Coin haut gauche du contenu dans le document.
    x: f64,
    y: f64,
    /// Coins du contenu dans le document : haut gauche, haut droit, bas droit, bas gauche.
    corners: [(f64, f64); 4],
    /// Transformation complète (repère contenu -> document), matrice 3x3 en ligne.
    transform: [f64; 9],
}

/// Document ouvert par « Modifier le contenu ».
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartParentInfo {
    /// Document qui contient l'objet dynamique.
    doc_id: DocId,
    content_id: ContentId,
    /// Nom du calque objet dynamique.
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInfo {
    id: DocId,
    name: String,
    width: u32,
    height: u32,
    dpi: f32,
    tile_size: u32,
    max_level: u8,
    project_path: Option<String>,
    revision: u64,
    /// Du calque le plus haut au plus bas, dans l'ordre d'affichage du panneau.
    layers: Vec<LayerInfo>,
    has_selection: bool,
    guides: Vec<Guide>,
    /// Repères verrouillés : l'interface refuse alors de les déplacer.
    guides_locked: bool,
    /// Tracé de travail de la plume.
    paths: Vec<SubPath>,
    history: Vec<String>,
    history_cursor: usize,
    can_undo: bool,
    can_redo: bool,
    /// Présent si ce document est le contenu d'un objet dynamique ouvert par « Modifier le contenu » : Ctrl+S doit
    /// alors appeler `smart_content_commit` (et rien d'autre).
    smart_parent: Option<SmartParentInfo>,
}

#[derive(Serialize, Default)]
pub struct DirtyInfo {
    all: bool,
    /// Tuiles de niveau 0, `[x, y]`.
    tiles: Vec<[i32; 2]>,
    layers: Vec<LayerId>,
    selection: bool,
    /// Calques dont les pixels (ou le masque) ont changé, pas seulement la position.
    repainted: Vec<LayerId>,
    /// Zones repeintes, dans le repère de chaque calque, sans limite au document.
    areas: Vec<AreaInfo>,
}

#[derive(Serialize)]
pub struct AreaInfo {
    layer: LayerId,
    mask: bool,
    rect: [i32; 4],
}

impl From<Dirty> for DirtyInfo {
    fn from(dirty: Dirty) -> Self {
        Self {
            all: dirty.all,
            tiles: dirty.tiles.iter().map(|c| [c.x, c.y]).collect(),
            layers: dirty.layers,
            selection: dirty.selection,
            repainted: dirty.repainted,
            areas: dirty.areas.into_iter().map(|a| AreaInfo { layer: a.layer, mask: a.mask, rect: a.rect }).collect(),
        }
    }
}

#[derive(Serialize)]
pub struct EditResult {
    document: DocumentInfo,
    dirty: DirtyInfo,
}

fn describe_smart(layer: &engine::Layer, instances: &std::collections::HashMap<ContentId, u32>) -> Option<SmartInfo> {
    let content = layer.smart()?;
    let full = layer.smart_transform()?;
    let geometry = smart::SmartGeometry::of(&full, content.width(), content.height());
    let original_name = content.original().map(|o| o.name.clone());
    let path = content.path().map(|p| p.to_string_lossy().into_owned());
    let linked_name = content.path().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
    let export_name = original_name.clone().or(linked_name).unwrap_or_else(|| format!("{}.{PROJECT_EXTENSION}", content.name));
    Some(SmartInfo {
        content_id: content.id,
        linked: content.is_linked(),
        path,
        missing: content.missing(),
        name: content.name.clone(),
        width: content.width(),
        height: content.height(),
        instances: instances.get(&content.id).copied().unwrap_or(1),
        original_name,
        export_name,
        scale_x: geometry.scale_x,
        scale_y: geometry.scale_y,
        rotation_deg: geometry.rotation_deg,
        x: geometry.x,
        y: geometry.y,
        corners: geometry.corners,
        transform: full.0,
    })
}

fn describe(id: DocId, entry: &OpenDocument, session: &Session) -> DocumentInfo {
    let doc = session.document();
    let (history, history_cursor) = session.history();
    let mut instances = std::collections::HashMap::<ContentId, u32>::new();
    for (_, layer) in doc.walk_top_down() {
        if let Some(content) = layer.smart() {
            *instances.entry(content.id).or_default() += 1;
        }
    }
    DocumentInfo {
        id,
        name: doc.name.clone(),
        width: doc.width(),
        height: doc.height(),
        dpi: doc.dpi,
        tile_size: TILE_SIZE,
        max_level: TileRenderer::max_level(doc),
        project_path: entry.project_path().map(|p| p.to_string_lossy().into_owned()),
        revision: session.revision(),
        layers: doc
            .walk_top_down()
            .into_iter()
            .map(|(depth, l)| {
                let siblings = doc.children_of(l.parent).unwrap_or_default();
                LayerInfo {
                    id: l.id,
                    parent: l.parent,
                    index: siblings.iter().position(|s| *s == l.id).unwrap_or_default(),
                    depth,
                    offset: l.offset,
                    name: l.props.name.clone(),
                    is_group: l.is_group(),
                    expanded: matches!(l.kind, engine::document::LayerKind::Group { expanded: true, .. }),
                    child_count: l.children().len(),
                    visible: l.props.visible,
                    opacity: l.props.opacity,
                    fill: l.props.fill,
                    blend: l.props.blend,
                    locks: l.props.locks,
                    clipped: l.props.clipped,
                    has_mask: l.mask.is_some(),
                    mask_enabled: l.mask.as_ref().is_some_and(|m| m.enabled),
                    text: l.text().cloned(),
                    adjustment: l.adjustment().cloned(),
                    shape: l.shape().cloned(),
                    effects: l.effects.clone(),
                    label: l.label,
                    smart: describe_smart(l, &instances),
                }
            })
            .collect(),
        has_selection: doc.selection().is_some(),
        guides: doc.guides.clone(),
        guides_locked: doc.guides_locked,
        paths: doc.paths.clone(),
        history: history.into_iter().map(str::to_owned).collect(),
        history_cursor,
        can_undo: session.can_undo(),
        can_redo: session.can_redo(),
        smart_parent: entry.smart_parent().map(|link| SmartParentInfo { doc_id: link.parent, content_id: link.content, name: link.name }),
    }
}

/// Groupe de threads du moteur. `None` : celui de rayon par défaut (tous les cœurs). Remplacé quand
/// l'utilisateur change le nombre de cœurs ; les calculs en cours finissent dans l'ancien groupe.
static WORKERS: std::sync::RwLock<Option<Arc<rayon::ThreadPool>>> = std::sync::RwLock::new(None);

async fn blocking<T, F>(work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CommandError> + Send + 'static,
{
    let workers = WORKERS.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let run = move || match workers {
        // Tout ce que le moteur parallélise à l'intérieur de `install` utilise ce groupe, et lui seul.
        Some(pool) => pool.install(work),
        None => work(),
    };
    tauri::async_runtime::spawn_blocking(run).await.map_err(|_| CommandError::Interrupted)?
}

/// Montre la fenêtre principale et referme la bannière de lancement. Sans effet si c'est déjà fait.
pub fn reveal_main_window(app: &crate::rt::AppHandle) {
    use tauri::Manager;
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        // La fenêtre est créée cachée (bannière) avec `maximized: true`, mais sous X11 cette demande n'est qu'un
        // message au gestionnaire de fenêtres, qui l'ignore tant que la fenêtre n'est pas affichée : on la refait
        // ici, une fois visible. Sans effet là où elle a déjà été honorée (Windows, Wayland).
        let _ = main.maximize();
        let _ = main.set_focus();
    }
    if let Some(splash) = app.get_webview_window("splashscreen") {
        let _ = splash.close();
    }
}

/// L'interface est prête (et la bannière a été vue assez longtemps) : place à la fenêtre principale.
#[tauri::command]
pub fn close_splash(app: crate::rt::AppHandle) {
    reveal_main_window(&app);
}

/// Ctrl+C dans le panneau Calques : copie les calques sélectionnés (groupes et contenu compris).
#[tauri::command]
pub async fn copy_layers(state: State<'_, AppState>, doc_id: DocId, ids: Vec<LayerId>) -> Result<usize, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    let state = state.inner().clone();
    blocking(move || {
        let clip = entry.read().document().copy_layers(&ids)?;
        state.set_layer_clipboard(clip);
        Ok(ids.len())
    })
    .await
}

/// Ctrl+V : colle les calques copiés au-dessus de `above`, dans ce document (qui peut être un autre que celui de la copie).
#[tauri::command]
pub async fn paste_layers(state: State<'_, AppState>, doc_id: DocId, above: Option<LayerId>, parent: Option<LayerId>, index: Option<usize>) -> Result<EditResult, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    let clip = state.layer_clipboard().ok_or_else(|| EngineError::InvalidOperation("aucun calque copié".into()))?;
    blocking(move || {
        let mut session = entry.write();
        let (_, dirty) = session.apply("Coller les calques", None, |doc| {
            // Emplacement précis (dépôt dans le panneau Calques), sinon au-dessus du calque actif.
            let (parent, index) = match index {
                Some(index) => (parent, index),
                None => slot(doc, above)?,
            };
            doc.paste_layers(&clip, parent, index).map(|_| ())
        })?;
        Ok(EditResult { document: describe(doc_id, &entry, &session), dirty: dirty.into() })
    })
    .await
}

/// Fichiers passés au lancement (double-clic sur un projet ou une image dans l'explorateur) : l'interface les ouvre une fois prête.
#[tauri::command]
pub fn startup_files() -> Vec<PathBuf> {
    files_among(std::env::args().skip(1))
}

/// Parmi des arguments de ligne de commande, ceux qui désignent un fichier existant.
pub fn files_among(args: impl Iterator<Item = String>) -> Vec<PathBuf> {
    args.filter(|arg| !arg.starts_with('-')).map(PathBuf::from).filter(|path| path.is_file()).collect()
}

// ------------------------------------------------------------------ presse-papiers du système

/// Empreinte du contenu actuel du presse-papiers du système (image ou texte), ou 0 s'il est vide. Sert à savoir
/// qui a copié en dernier : si l'empreinte est celle relevée juste après notre propre copie, rien n'a été copié
/// ailleurs depuis ; sinon (capture d'écran, copie dans une autre application), c'est le contenu extérieur qui
/// est le plus récent, et c'est lui qu'on colle.
#[tauri::command]
pub async fn clipboard_fingerprint() -> Result<u64, CommandError> {
    blocking(|| {
        use std::hash::{Hash, Hasher};
        let Ok(mut clipboard) = arboard::Clipboard::new() else { return Ok(0) };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        if let Ok(image) = clipboard.get_image() {
            (image.width, image.height).hash(&mut hasher);
            image.bytes.hash(&mut hasher);
            return Ok(hasher.finish() | 1);
        }
        if let Ok(text) = clipboard.get_text() {
            text.hash(&mut hasher);
            return Ok(hasher.finish() | 1);
        }
        Ok(0)
    })
    .await
}

/// Image du presse-papiers du système, en PNG ; réponse vide s'il n'y a pas d'image.
#[tauri::command]
pub async fn clipboard_image() -> Result<Response, CommandError> {
    blocking(|| {
        let image = arboard::Clipboard::new().ok().and_then(|mut clipboard| clipboard.get_image().ok());
        let Some(image) = image else { return Ok(Response::new(Vec::new())) };
        let rgba = image::RgbaImage::from_raw(image.width as u32, image.height as u32, image.bytes.into_owned())
            .ok_or_else(|| EngineError::InvalidOperation("image du presse-papiers illisible".into()))?;
        let mut png = std::io::Cursor::new(Vec::new());
        rgba.write_to(&mut png, image::ImageFormat::Png).map_err(|e| EngineError::InvalidOperation(format!("image du presse-papiers : {e}")))?;
        Ok(Response::new(png.into_inner()))
    })
    .await
}

/// Place un court texte dans le presse-papiers du système (marque d'une copie de calques).
#[tauri::command]
pub async fn clipboard_set_text(text: String) -> Result<(), CommandError> {
    blocking(move || {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(text);
        }
        Ok(())
    })
    .await
}

// ------------------------------------------------------------------ sauvegarde automatique

/// Dossier des copies de secours (sauvegarde automatique), créé au besoin.
fn recovery_dir(app: &crate::rt::AppHandle) -> Result<PathBuf, CommandError> {
    use tauri::Manager;
    let dir = app.path().app_data_dir().map_err(|e| EngineError::InvalidOperation(e.to_string()))?.join("recuperation");
    std::fs::create_dir_all(&dir).map_err(|e| EngineError::InvalidOperation(format!("{} : {e}", dir.display())))?;
    Ok(dir)
}

/// Clé d'une copie de secours : lettres, chiffres et tirets seulement (elle devient un nom de fichier).
fn recovery_path(app: &crate::rt::AppHandle, key: &str) -> Result<PathBuf, CommandError> {
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(EngineError::InvalidOperation("clé de récupération invalide".into()).into());
    }
    Ok(recovery_dir(app)?.join(format!("{key}.graf")))
}

/// Sauvegarde automatique d'un document, historique compris. L'instantané est pris tout de suite (sous verrou,
/// quelques microsecondes : les documents partagent leurs pixels) ; l'écriture se fait ensuite sans rien
/// bloquer. Les actions faites pendant l'écriture n'y figurent donc pas : elles seront dans la suivante.
/// Avancement : événement `autosave-progress` (0 à 1).
#[tauri::command]
pub async fn autosave(app: crate::rt::AppHandle, state: State<'_, AppState>, doc_id: DocId, key: String) -> Result<(), CommandError> {
    use tauri::Emitter;
    let entry = lookup(state.inner(), doc_id)?;
    let path = recovery_path(&app, &key)?;
    let (history, project) = {
        let session = entry.read();
        (session.snapshot(), entry.project_path())
    };
    blocking(move || {
        let report = |progress: f32| {
            let _ = app.emit("autosave-progress", progress);
        };
        Ok(io::save_recovery(&history, project.as_deref(), &path, &report)?)
    })
    .await
}

/// Copie de secours d'un document, trouvée au démarrage.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryEntry {
    key: String,
    /// Date de la dernière copie, en millisecondes depuis 1970.
    saved_at: u64,
    size: u64,
}

#[tauri::command]
pub fn recovery_list(app: crate::rt::AppHandle) -> Result<Vec<RecoveryEntry>, CommandError> {
    let dir = recovery_dir(&app)?;
    let mut entries: Vec<RecoveryEntry> = std::fs::read_dir(&dir)
        .map(|read| read.flatten().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|file| {
            let name = file.file_name().to_string_lossy().into_owned();
            let key = name.strip_suffix(".graf")?.to_owned();
            let meta = file.metadata().ok()?;
            let saved_at = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64;
            Some(RecoveryEntry { key, saved_at, size: meta.len() })
        })
        .collect();
    entries.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    Ok(entries)
}

/// Rouvre une copie de secours avec tout son historique ; elle reste sur le disque jusqu'à ce que l'interface la supprime.
#[tauri::command]
pub async fn recovery_open(app: crate::rt::AppHandle, state: State<'_, AppState>, key: String) -> Result<DocumentInfo, CommandError> {
    let path = recovery_path(&app, &key)?;
    let state = state.inner().clone();
    blocking(move || {
        let (history, project) = io::load_recovery(&path)?;
        let session = Session::from_history(history);
        let project = project.filter(|p| p.exists());
        let (id, entry) = state.insert_session(session, project);
        let session = entry.read();
        Ok(describe(id, &entry, &session))
    })
    .await
}

#[tauri::command]
pub fn recovery_delete(app: crate::rt::AppHandle, key: String) -> Result<(), CommandError> {
    let path = recovery_path(&app, &key)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| EngineError::InvalidOperation(format!("{} : {e}", path.display())))?;
    }
    Ok(())
}

// ------------------------------------------------------------------ écran d'accueil : aperçus des récents

/// Dossier des aperçus des fichiers récents, créé au besoin.
fn previews_dir(app: &crate::rt::AppHandle) -> Result<PathBuf, CommandError> {
    use tauri::Manager;
    let dir = app.path().app_data_dir().map_err(|e| EngineError::InvalidOperation(e.to_string()))?.join("apercus");
    std::fs::create_dir_all(&dir).map_err(|e| EngineError::InvalidOperation(format!("{} : {e}", dir.display())))?;
    Ok(dir)
}

fn preview_path(app: &crate::rt::AppHandle, key: &str) -> Result<PathBuf, CommandError> {
    if key.is_empty() || key.len() > 64 || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(EngineError::InvalidOperation("clé d'aperçu invalide".into()).into());
    }
    Ok(previews_dir(app)?.join(format!("{key}.png")))
}

/// Enregistre un petit aperçu du document (320 px) sous la clé d'un fichier récent. Le document est copié sous
/// verrou (instantané, les pixels sont partagés), l'aperçu est calculé ensuite sans rien bloquer.
#[tauri::command]
pub async fn save_preview(app: crate::rt::AppHandle, state: State<'_, AppState>, doc_id: DocId, key: String) -> Result<(), CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    let path = preview_path(&app, &key)?;
    let doc = entry.read().document().clone();
    blocking(move || {
        let png = io::preview_png(&doc, 320)?;
        std::fs::write(&path, png).map_err(|e| EngineError::InvalidOperation(format!("{} : {e}", path.display())))?;
        Ok(())
    })
    .await
}

/// Aperçu d'un fichier récent (PNG), ou réponse vide s'il n'y en a pas.
#[tauri::command]
pub async fn load_preview(app: crate::rt::AppHandle, key: String) -> Result<Response, CommandError> {
    let path = preview_path(&app, &key)?;
    blocking(move || Ok(Response::new(std::fs::read(&path).unwrap_or_default()))).await
}

#[tauri::command]
pub fn delete_preview(app: crate::rt::AppHandle, key: String) -> Result<(), CommandError> {
    let path = preview_path(&app, &key)?;
    let _ = std::fs::remove_file(path);
    Ok(())
}

// ------------------------------------------------------------------------- modèles

/// Un modèle livré avec l'application : un projet `.graf` du dossier `modeles/`, copié dans les
/// ressources à la construction (`export/sync-modeles.*`). Le nom vient du fichier, la description
/// d'un `.txt` du même nom s'il existe.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    /// Le nom de fichier sans extension : sert de clé (aperçu, ouverture).
    pub id: String,
    pub file: String,
    pub name: String,
    pub description: String,
}

fn templates_dir(app: &crate::rt::AppHandle) -> Result<PathBuf, CommandError> {
    use tauri::Manager;
    Ok(app.path().resource_dir().map_err(|e| EngineError::InvalidOperation(e.to_string()))?.join("templates"))
}

/// Tous les `.graf` du dossier des modèles, par ordre alphabétique.
fn read_templates(app: &crate::rt::AppHandle) -> Result<Vec<TemplateInfo>, CommandError> {
    let dir = templates_dir(app)?;
    let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(Vec::new()) };
    let mut list: Vec<TemplateInfo> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("graf")))
        .filter_map(|path| {
            let stem = path.file_stem()?.to_str()?.to_owned();
            let file = path.file_name()?.to_str()?.to_owned();
            let description = std::fs::read_to_string(path.with_extension("txt")).map(|t| t.trim().to_owned()).unwrap_or_default();
            Some(TemplateInfo { id: stem.clone(), file, name: stem.replace(['_', '-'], " "), description })
        })
        .collect();
    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(list)
}

fn template_file(app: &crate::rt::AppHandle, id: &str) -> Result<(TemplateInfo, PathBuf), CommandError> {
    let template = read_templates(app)?.into_iter().find(|t| t.id == id).ok_or_else(|| EngineError::InvalidOperation(format!("modèle inconnu : {id}")))?;
    let path = templates_dir(app)?.join(&template.file);
    Ok((template, path))
}

/// Les modèles disponibles.
#[tauri::command]
pub fn list_templates(app: crate::rt::AppHandle) -> Result<Vec<TemplateInfo>, CommandError> {
    read_templates(&app)
}

/// Aperçu d'un modèle (PNG, 640 px) : calculé la première fois depuis le projet lui-même, puis gardé
/// avec les aperçus des fichiers récents. Renvoie aussi les dimensions du document.
#[tauri::command]
pub async fn template_preview(app: crate::rt::AppHandle, id: String) -> Result<Response, CommandError> {
    let (_, path) = template_file(&app, &id)?;
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).take(48).collect();
    let cache = preview_path(&app, &format!("modele-{safe}"))?;
    blocking(move || {
        if let Ok(png) = std::fs::read(&cache) {
            return Ok(Response::new(png));
        }
        let doc = io::load_project(&path)?;
        let png = io::preview_png(&doc, 640)?;
        let _ = std::fs::write(&cache, &png);
        Ok(Response::new(png))
    })
    .await
}

/// Ouvre un modèle **comme copie** : un document sans chemin de projet, nommé d'après le modèle.
/// Enregistrer demandera un nouveau fichier ; le modèle livré n'est jamais modifié.
#[tauri::command]
pub async fn open_template(app: crate::rt::AppHandle, state: State<'_, AppState>, id: String) -> Result<DocumentInfo, CommandError> {
    let (template, path) = template_file(&app, &id)?;
    let state = state.inner().clone();
    blocking(move || {
        let mut document = io::load_project(&path)?;
        document.name = template.name.clone();
        let (id, entry) = state.insert(document, "Ouvrir le modèle", None);
        let info = describe(id, &entry, &entry.read());
        Ok(info)
    })
    .await
}

/// Le fichier existe-t-il encore ? (Récents : un fichier déplacé ou supprimé est signalé, pas ouvert.)
#[tauri::command]
pub fn file_exists(path: PathBuf) -> bool {
    path.is_file()
}

/// Montre le fichier dans l'explorateur du système, sélectionné quand la plateforme le permet.
///
/// Windows et macOS savent désigner le fichier lui-même ; sous Linux on passe d'abord par
/// l'interface standard des gestionnaires de fichiers, et à défaut on ouvre le dossier parent.
#[tauri::command]
pub fn reveal_path(path: PathBuf) -> Result<(), CommandError> {
    if !path.exists() {
        return Err(EngineError::InvalidOperation("ce fichier n'existe plus à cet emplacement".into()).into());
    }
    let folder = path.parent().unwrap_or(&path).to_path_buf();
    let spawned = {
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn()
        }
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open").arg("-R").arg(&path).spawn()
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let shown = std::process::Command::new("dbus-send")
                .args(["--session", "--print-reply", "--dest=org.freedesktop.FileManager1", "--type=method_call", "/org/freedesktop/FileManager1", "org.freedesktop.FileManager1.ShowItems"])
                .arg(format!("array:string:file://{}", path.display()))
                .arg("string:")
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
            if shown {
                return Ok(());
            }
            std::process::Command::new("xdg-open").arg(&folder).spawn()
        }
    };
    spawned.map(|_| ()).map_err(|error| EngineError::InvalidOperation(format!("impossible d'ouvrir {} : {error}", folder.display())).into())
}

/// Contenu brut d'un fichier choisi par l'utilisateur (images AVIF : décodées par la webview, qui sait les lire).
#[tauri::command]
pub async fn read_file_bytes(path: PathBuf) -> Result<Response, CommandError> {
    blocking(move || Ok(Response::new(std::fs::read(&path).map_err(|e| EngineError::InvalidOperation(format!("{} : {e}", path.display())))?))).await
}

/// Le dossier existe-t-il encore ? (dossier d'enregistrement par défaut, sur un disque peut-être débranché)
#[tauri::command]
pub fn directory_exists(path: PathBuf) -> bool {
    path.is_dir()
}

/// Cartes graphiques de la machine, lues dans le système (Linux) : la webview de Linux (WebKitGTK) masque le
/// nom réel de la carte et répond « Apple GPU » par protection de la vie privée. La carte de démarrage vient en
/// premier. Ailleurs, liste vide : la webview donne déjà le vrai nom.
#[tauri::command]
pub fn system_gpus() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        linux_gpus()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

/// Nom lisible d'une carte, d'après la base des identifiants PCI (format de `pci.ids`). Utile sous
/// Linux seulement — ailleurs, la webview donne déjà le vrai nom — d'où la compilation conditionnelle :
/// sans elle, Windows compilait une fonction que personne n'appelle et le signalait à chaque build.
#[cfg(any(target_os = "linux", test))]
fn pci_name(database: &str, vendor: &str, device: &str) -> String {
    let short = match vendor {
        "1002" => "AMD".to_owned(),
        "10de" => "NVIDIA".to_owned(),
        "8086" => "Intel".to_owned(),
        _ => String::new(),
    };
    let entry = |line: &str, id: &str| line.strip_prefix(id).filter(|rest| rest.starts_with(' ')).map(|rest| rest.trim().to_owned());
    let mut in_vendor = false;
    let (mut vendor_name, mut device_name) = (String::new(), String::new());
    for line in database.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            if in_vendor {
                break; // fin du bloc de ce fabricant
            }
            if let Some(name) = entry(line, vendor) {
                in_vendor = true;
                vendor_name = name;
            }
        } else if in_vendor && !line.starts_with("\t\t") {
            if let Some(name) = entry(line.trim_start(), device) {
                device_name = name;
                break;
            }
        }
    }
    let vendor = if short.is_empty() { vendor_name } else { short };
    // « Navi 31 [Radeon RX 7900 XT/7900 XTX/7900 GRE/7900M] » -> « Radeon RX 7900 XT/7900 XTX/7900 GRE/7900M (Navi 31) »
    let device = match (device_name.find('['), device_name.rfind(']')) {
        (Some(open), Some(close)) if close > open => format!("{} ({})", &device_name[open + 1..close], device_name[..open].trim()),
        _ if device_name.is_empty() => format!("carte {device}"),
        _ => device_name,
    };
    format!("{vendor} {device}").trim().to_owned()
}

#[cfg(target_os = "linux")]
fn linux_gpus() -> Vec<String> {
    let read = |path: std::path::PathBuf| std::fs::read_to_string(path).map(|s| s.trim().trim_start_matches("0x").to_ascii_lowercase()).ok();
    // Base des identifiants PCI (paquet hwdata sous Fedora, pciutils ailleurs).
    let database = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids", "/usr/share/pci.ids"].iter().find_map(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let mut cards: Vec<(bool, String)> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else { return Vec::new() };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // `card0`, `card1`… mais pas leurs sorties d'écran (`card0-DP-1`).
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let device_dir = entry.path().join("device");
        let (Some(vendor), Some(device)) = (read(device_dir.join("vendor")), read(device_dir.join("device"))) else { continue };
        let boot = read(device_dir.join("boot_vga")).is_some_and(|v| v == "1");
        let label = pci_name(&database, &vendor, &device);
        if !cards.iter().any(|(_, known)| known == &label) {
            cards.push((boot, label));
        }
    }
    cards.sort_by_key(|(boot, _)| !boot);
    cards.into_iter().map(|(_, label)| label).collect()
}

#[cfg(test)]
mod gpu_name_tests {
    use super::pci_name;

    const IDS: &str = "# extrait de pci.ids\n1000  Broadcom / LSI\n\t0001  53c810\n1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t744b  Navi 31 [Radeon Pro W7900]\n\t744c  Navi 31 [Radeon RX 7900 XT/7900 XTX/7900 GRE/7900M]\n\t\t1002 0e3b  RX 7900 XTX\n10021  Faux\n1af4  Red Hat, Inc.\n\t1050  Virtio 1.0 GPU\n";

    #[test]
    fn names_the_card_from_the_pci_database() {
        assert_eq!(pci_name(IDS, "1002", "744c"), "AMD Radeon RX 7900 XT/7900 XTX/7900 GRE/7900M (Navi 31)");
        assert_eq!(pci_name(IDS, "1af4", "1050"), "Red Hat, Inc. Virtio 1.0 GPU");
        assert_eq!(pci_name(IDS, "1002", "ffff"), "AMD carte ffff", "carte inconnue de la base : son identifiant");
        assert_eq!(pci_name("", "10de", "2684"), "NVIDIA carte 2684", "sans base du tout");
    }
}

/// Ce que la machine offre, pour la fenêtre des préférences.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    cores: usize,
    /// Mémoire vive totale, en mébioctets.
    ram_mb: u64,
    /// Mémoire vidéo de la carte graphique, en mébioctets ; `None` : inconnue (voir `gpu_info`).
    vram_mb: Option<u64>,
    /// Carte intégrée : sa mémoire vidéo est prise sur la mémoire vive (l'interface borne alors le plafond).
    vram_shared: bool,
}

fn total_ram_mb() -> u64 {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    system.total_memory() / (1024 * 1024)
}

#[tauri::command]
pub fn system_info() -> SystemInfo {
    let vram = crate::gpu_info::video_memory();
    SystemInfo {
        cores: std::thread::available_parallelism().map_or(1, usize::from),
        ram_mb: total_ram_mb(),
        vram_mb: vram.map(|memory| memory.mb),
        vram_shared: vram.is_some_and(|memory| memory.shared),
    }
}

/// Part de la mémoire vive que Lumitype s'autorise par défaut, et ses bornes (comme Photoshop).
const DEFAULT_MEMORY_PERCENT: u8 = 50;
const MEMORY_PERCENT_RANGE: std::ops::RangeInclusive<u8> = 10..=90;

/// Réglages de performance, appliqués tout de suite et sans redémarrage.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceSettings {
    /// Part de la mémoire vive (10 à 90 %) que Lumitype s'autorise, documents, historique et cache compris ;
    /// `None` = 50 %. Le cache de rendu prend ce que ce plafond laisse (voir `engine::memory`). L'ancien réglage
    /// `cacheMb` (cache fixe par document) n'est plus envoyé : s'il l'était encore, serde l'ignore.
    #[serde(default)]
    memory_percent: Option<u8>,
    /// Cœurs utilisés par le moteur ; `None` ou 0 = tous.
    cores: Option<usize>,
    history_states: usize,
    /// Disque de travail de l'historique (voir `engine::scratch`) : actif par défaut.
    #[serde(default = "enabled_by_default")]
    scratch_enabled: bool,
    /// Dossier du fichier d'échange ; `None` ou vide = `<cache>/dev.imageeditor.app/scratch/`.
    #[serde(default)]
    scratch_dir: Option<String>,
    /// Place maximale sur le disque, en Mio ; `None` = 20 Gio. Bornée de 2 à 200 Gio.
    #[serde(default)]
    scratch_quota_mb: Option<u64>,
}

fn enabled_by_default() -> bool {
    true
}

/// Bornes du quota du disque de travail, en Mio (2 à 200 Gio).
const SCRATCH_QUOTA_MB: std::ops::RangeInclusive<u64> = 2 * 1024..=200 * 1024;

#[tauri::command]
pub async fn set_performance(state: State<'_, AppState>, settings: PerformanceSettings) -> Result<(), CommandError> {
    let ram_bytes = total_ram_mb() * 1024 * 1024;
    let percent = settings.memory_percent.unwrap_or(DEFAULT_MEMORY_PERCENT).clamp(*MEMORY_PERCENT_RANGE.start(), *MEMORY_PERCENT_RANGE.end());
    // Plus de budget fixe : le cache se règle seul sous le plafond.
    engine::render::set_cache_budget(0);
    engine::set_memory_cap(ram_bytes * u64::from(percent) / 100);
    // Les filtres travaillent en flottants sur tout le calque : au-delà de la moitié de la mémoire de
    // la machine, ils refusent poliment au lieu de laisser le système tuer l'application.
    engine::filters::set_memory_budget(ram_bytes / 2);
    engine::session::set_history_states(settings.history_states);
    let quota_mb = settings.scratch_quota_mb.unwrap_or(engine::scratch::DEFAULT_QUOTA / (1024 * 1024)).clamp(*SCRATCH_QUOTA_MB.start(), *SCRATCH_QUOTA_MB.end());
    let scratch_dir = settings.scratch_dir.map(|dir| dir.trim().to_owned()).filter(|dir| !dir.is_empty()).map(PathBuf::from);
    engine::scratch::set_scratch(settings.scratch_enabled, scratch_dir, quota_mb * 1024 * 1024);
    let available = std::thread::available_parallelism().map_or(1, usize::from);
    let pool = match settings.cores.filter(|c| *c > 0 && *c < available) {
        Some(cores) => Some(Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(cores)
                .thread_name(|i| format!("lumitype-moteur-{i}"))
                .build()
                .map_err(|e| EngineError::InvalidOperation(format!("groupe de threads : {e}")))?,
        )),
        None => None,
    };
    *WORKERS.write().unwrap_or_else(std::sync::PoisonError::into_inner) = pool;
    // Le nouveau plafond s'applique aussi aux documents déjà ouverts : cache resserré, et historique élagué
    // s'il le faut (c'est le seul moment où une session qui n'est pas en cours d'édition élague).
    let state = state.inner().clone();
    blocking(move || {
        for entry in state.all() {
            entry.write().enforce_memory_cap();
        }
        Ok(())
    })
    .await
}

/// Répartition de la mémoire occupée, pour la jauge des préférences (demandée une fois par seconde tant
/// que la boîte est ouverte). Les mébioctets sont arrondis au plus proche.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStatus {
    /// Plafond en vigueur (0 tant qu'aucun réglage n'a été appliqué).
    cap_mb: u64,
    ram_mb: u64,
    documents_mb: u64,
    history_mb: u64,
    cache_mb: u64,
    /// Octets des fichiers d'échange (états d'historique partis sur le disque de travail).
    scratch_bytes: u64,
    /// Dossier où le disque de travail écrit (réglé, ou celui par défaut).
    scratch_dir: String,
}

#[tauri::command]
pub async fn memory_status(state: State<'_, AppState>) -> Result<MemoryStatus, CommandError> {
    let state = state.inner().clone();
    blocking(move || {
        let to_mb = |bytes: u64| (bytes + 512 * 1024) / (1024 * 1024);
        let entries = state.all();
        let sessions: Vec<_> = entries.iter().map(|entry| entry.read()).collect();
        let status = engine::memory_status(sessions.iter().map(|session| &**session));
        Ok(MemoryStatus {
            cap_mb: to_mb(engine::memory_cap()),
            ram_mb: total_ram_mb(),
            documents_mb: to_mb(status.documents_bytes),
            history_mb: to_mb(status.history_bytes),
            cache_mb: to_mb(status.cache_bytes),
            scratch_bytes: status.scratch_bytes,
            scratch_dir: engine::scratch::scratch_dir().to_string_lossy().into_owned(),
        })
    })
    .await
}

fn is_project(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXTENSION) || e.eq_ignore_ascii_case(LEGACY_PROJECT_EXTENSION))
}

fn lookup(state: &AppState, id: DocId) -> Result<Arc<OpenDocument>, CommandError> {
    state.get(id).ok_or(CommandError::UnknownDocument(id))
}

// -------------------------------------------------------------------- documents

#[tauri::command]
pub async fn new_document(
    state: State<'_, AppState>,
    name: String,
    width: u32,
    height: u32,
    dpi: f32,
    transparent: bool,
) -> Result<DocumentInfo, CommandError> {
    let state = state.inner().clone();
    blocking(move || {
        let background = if transparent { Background::Transparent } else { Background::White };
        let document = Document::with_background(name, width, height, dpi, background)?;
        let (id, entry) = state.insert(document, "Nouveau", None);
        let info = describe(id, &entry, &entry.read());
        Ok(info)
    })
    .await
}

/// Ouvre un projet (`.iep`) ou importe une image (PNG, JPEG, WebP).
#[tauri::command]
pub async fn open_path(state: State<'_, AppState>, path: PathBuf) -> Result<DocumentInfo, CommandError> {
    let state = state.inner().clone();
    blocking(move || {
        let (document, project_path) =
            if is_project(&path) { (io::load_project(&path)?, Some(path)) } else { (io::import_image(&path)?, None) };
        let (id, entry) = state.insert(document, "Ouvrir", project_path);
        let info = describe(id, &entry, &entry.read());
        Ok(info)
    })
    .await
}

/// Décode `%XX` (UTF-8). Une séquence invalide est conservée telle quelle.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%').then(|| text.get(i + 1..i + 3)).flatten();
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Fichier déposé dans la fenêtre. La webview ne connaît pas son chemin : son
/// contenu arrive en corps binaire, son nom (encodé URL) et l'action en en-têtes.
/// `x-doc-id` présent : import comme calque au-dessus de `x-above` ; absent : nouveau document.
#[tauri::command]
pub async fn import_dropped(state: State<'_, AppState>, request: tauri::ipc::Request<'_>) -> Result<EditResult, CommandError> {
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let name = PathBuf::from(header("x-file-name").map_or_else(|| "Sans titre".into(), |n| percent_decode(&n)));
    let target: Option<DocId> = header("x-doc-id").and_then(|v| v.parse().ok());
    let above: Option<LayerId> = header("x-above").and_then(|v| v.parse().ok());
    // `x-smart: 1` : placé comme objet dynamique incorporé (image d'origine gardée), plutôt que comme calque de pixels.
    let as_smart = header("x-smart").is_some_and(|v| v == "1");
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(EngineError::InvalidOperation("contenu du fichier manquant".into()).into());
    };
    let bytes = bytes.clone();
    let state = state.inner().clone();

    blocking(move || match target {
        Some(doc_id) => {
            let entry = lookup(&state, doc_id)?;
            let mut session = entry.write();
            let (_, dirty) = if as_smart {
                let content = smart::content_from_bytes(bytes.to_vec(), &name)?;
                session.apply("Placer un objet dynamique", None, |doc| {
                    let (parent, index) = slot(doc, above)?;
                    doc.place_smart_object(content, parent, index)
                })?
            } else {
                session.apply("Importer un calque", None, |doc| {
                    let (parent, index) = slot(doc, above)?;
                    io::import_bytes_as_layer(doc, &bytes, &name, parent, index)
                })?
            };
            Ok(EditResult { document: describe(doc_id, &entry, &session), dirty: dirty.into() })
        }
        None => {
            // Un projet déposé s'ouvre sans chemin : « Enregistrer » demandera où l'écrire.
            let document = if is_project(&name) { io::load_project_bytes(&bytes, &name)? } else { io::import_image_bytes(&bytes, &name)? };
            let (id, entry) = state.insert(document, "Ouvrir", None);
            let document = describe(id, &entry, &entry.read());
            Ok(EditResult { document, dirty: DirtyInfo::default() })
        }
    })
    .await
}

/// Image glissée depuis un navigateur qui n'a fourni que son adresse : elle est téléchargée ici
/// puis importée comme un fichier déposé (calque du document `doc_id`, ou nouveau document).
#[tauri::command]
pub async fn import_url(state: State<'_, AppState>, url: String, doc_id: Option<DocId>, above: Option<LayerId>) -> Result<EditResult, CommandError> {
    const MAX_BYTES: u64 = 80 * 1024 * 1024;
    let state = state.inner().clone();
    blocking(move || {
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(EngineError::InvalidOperation("seules les adresses http(s) peuvent être importées".into()).into());
        }
        let failed = |e: &dyn std::fmt::Display| EngineError::InvalidOperation(format!("téléchargement impossible : {e}"));
        let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(20)).build();
        let response = agent.get(&url).call().map_err(|e| failed(&e))?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(response.into_reader(), MAX_BYTES), &mut bytes).map_err(|e| failed(&e))?;
        // Nom du calque : dernier segment de l'adresse, sans ses paramètres.
        let last = url.split(['?', '#']).next().unwrap_or_default().rsplit('/').next().unwrap_or_default();
        let name = PathBuf::from(if last.is_empty() { "Image".to_owned() } else { percent_decode(last) });
        match doc_id {
            Some(doc_id) => {
                let entry = lookup(&state, doc_id)?;
                let mut session = entry.write();
                let (_, dirty) = session.apply("Importer un calque", None, |doc| {
                    let (parent, index) = slot(doc, above)?;
                    io::import_bytes_as_layer(doc, &bytes, &name, parent, index)
                })?;
                Ok(EditResult { document: describe(doc_id, &entry, &session), dirty: dirty.into() })
            }
            None => {
                let (id, entry) = state.insert(io::import_image_bytes(&bytes, &name)?, "Ouvrir", None);
                let document = describe(id, &entry, &entry.read());
                Ok(EditResult { document, dirty: DirtyInfo::default() })
            }
        }
    })
    .await
}

/// PNG des derniers pixels copiés, pour le presse-papiers du système. Vide si rien n'a été copié.
#[tauri::command]
pub async fn clipboard_png(state: State<'_, AppState>) -> Result<Response, CommandError> {
    let state = state.inner().clone();
    blocking(move || Ok(Response::new(match state.clipboard() {
        Some(clip) => io::clip_to_png(&clip)?,
        None => Vec::new(),
    })))
    .await
}

/// Masque rapide : début d'un trait qui peint la sélection. La suite passe par `stroke_points` et `stroke_end`.
#[tauri::command]
pub async fn stroke_begin_selection(state: State<'_, AppState>, doc_id: DocId, brush: BrushParams) -> Result<(), CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        entry.write().begin_selection_stroke(brush.into());
        Ok(())
    })
    .await
}

/// Fenêtre d'export : encode l'image avec les réglages courants, en mémoire. Réponse : 16 octets
/// d'en-tête (poids exact du fichier en u64, largeur et hauteur en u32, petit-boutistes) puis une
/// image que la webview sait afficher, pour voir le rendu réel du format.
#[tauri::command]
pub async fn export_preview(state: State<'_, AppState>, doc_id: DocId, options: io::ExportOptions) -> Result<Response, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let encoded = io::export_encoded(entry.read().document(), options)?;
        let mut body = Vec::with_capacity(16 + encoded.display.len());
        body.extend_from_slice(&encoded.size.to_le_bytes());
        body.extend_from_slice(&encoded.width.to_le_bytes());
        body.extend_from_slice(&encoded.height.to_le_bytes());
        body.extend_from_slice(&encoded.display);
        Ok(Response::new(body))
    })
    .await
}

// ------------------------------------------------------------------ aperçu de l'export (instantané)

/// Image composée à la taille d'export, gardée pour l'aperçu : recalculée seulement si le document (révision), le
/// document affiché ou la taille changent. Changer de format ou de qualité ne recompose rien.
struct ExportCache {
    doc_id: DocId,
    revision: u64,
    scale: u32,
    flat: Arc<io::Flattened>,
}
static EXPORT_CACHE: std::sync::Mutex<Option<ExportCache>> = std::sync::Mutex::new(None);

fn export_flattened(entry: &OpenDocument, doc_id: DocId, scale: f32) -> Result<Arc<io::Flattened>, EngineError> {
    let key = (scale.clamp(0.01, 8.0) * 10_000.0).round() as u32;
    let (revision, document) = {
        let session = entry.read();
        (session.revision(), session.document().clone())
    };
    {
        let cache = EXPORT_CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(hit) = cache.as_ref().filter(|c| c.doc_id == doc_id && c.revision == revision && c.scale == key) {
            return Ok(Arc::clone(&hit.flat));
        }
    }
    let flat = Arc::new(io::flatten_for_export(&document, scale)?);
    *EXPORT_CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ExportCache { doc_id, revision, scale: key, flat: Arc::clone(&flat) });
    Ok(flat)
}

/// Libère l'image gardée pour l'aperçu (fenêtre d'export fermée).
#[tauri::command]
pub fn export_preview_release() {
    *EXPORT_CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// Vue d'ensemble de l'export et poids estimé. Réponse : taille du fichier (u32 × 2), taille de la vue (u32 × 2),
/// poids (u64), poids exact ? (u8), 0 = PNG / 1 = JPEG (u8), puis l'image encodée. Tout en petit-boutiste.
#[tauri::command]
pub async fn export_overview(state: State<'_, AppState>, doc_id: DocId, options: io::ExportOptions, max_side: u32) -> Result<Response, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let flat = export_flattened(&entry, doc_id, options.scale)?;
        let (w, h, bytes, kind) = flat.overview(max_side.clamp(64, 4096), options.format, options.quality, options.lossless)?;
        let (size, exact) = flat.estimate_size(options.format, options.quality, options.lossless, options.metadata.as_ref())?;
        let mut body = Vec::with_capacity(26 + bytes.len());
        for value in [flat.width, flat.height, w, h] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        body.extend_from_slice(&size.to_le_bytes());
        body.extend_from_slice(&[u8::from(exact), u8::from(kind == "jpeg")]);
        body.extend_from_slice(&bytes);
        Ok(Response::new(body))
    })
    .await
}

/// Zone visible de l'aperçu, à pleine résolution et encodée comme dans le fichier. `rect` en pixels du fichier.
/// Réponse : largeur et hauteur (u32 × 2), 0 = PNG / 1 = JPEG (u8), puis l'image.
#[tauri::command]
pub async fn export_region(state: State<'_, AppState>, doc_id: DocId, options: io::ExportOptions, rect: [u32; 4]) -> Result<Response, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let flat = export_flattened(&entry, doc_id, options.scale)?;
        let (w, h, bytes, kind) = flat.region(rect, options.format, options.quality, options.lossless)?;
        let mut body = Vec::with_capacity(9 + bytes.len());
        body.extend_from_slice(&w.to_le_bytes());
        body.extend_from_slice(&h.to_le_bytes());
        body.push(u8::from(kind == "jpeg"));
        body.extend_from_slice(&bytes);
        Ok(Response::new(body))
    })
    .await
}

/// Poids exact du fichier (l'image entière encodée), demandé en coulisse après l'estimation.
#[tauri::command]
pub async fn export_exact_size(state: State<'_, AppState>, doc_id: DocId, options: io::ExportOptions) -> Result<u64, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || Ok(export_flattened(&entry, doc_id, options.scale)?.exact_size(options.format, options.quality, options.lossless, options.metadata.as_ref())?)).await
}

/// Export final. L'avancement est envoyé à l'interface (événement `export-progress`) : elle affiche une barre.
#[tauri::command]
pub async fn export_image(app: crate::rt::AppHandle, state: State<'_, AppState>, doc_id: DocId, path: PathBuf, options: io::ExportOptions) -> Result<(), CommandError> {
    use tauri::Emitter;
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        #[derive(Clone, Serialize)]
        struct Progress<'a> {
            progress: f32,
            label: &'a str,
        }
        let report = |progress: f32, label: &str| {
            let _ = app.emit("export-progress", Progress { progress, label });
        };
        Ok(io::export_image_with_progress(entry.read().document(), &path, options, &report)?)
    })
    .await
}

#[tauri::command]
pub async fn save_project(state: State<'_, AppState>, doc_id: DocId, path: PathBuf) -> Result<DocumentInfo, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let mut session = entry.write();
        session.finish_stroke();
        // Le document prend le nom du fichier : l'onglet, l'accueil (« Reprendre ») et le `.graf`
        // lui-même l'affichent ensuite, plutôt que « Sans titre ».
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            session.rename(stem);
        }
        io::save_project(session.document(), &path)?;
        entry.set_project_path(path);
        Ok(describe(doc_id, &entry, &session))
    })
    .await
}

#[tauri::command]
pub async fn export_png(state: State<'_, AppState>, doc_id: DocId, path: PathBuf) -> Result<(), CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || Ok(io::export_png(entry.read().document(), &path)?)).await
}

#[tauri::command]
pub fn close_document(state: State<'_, AppState>, doc_id: DocId) {
    state.remove(doc_id);
}

// ------------------------------------------------------------- objets dynamiques

/// « Modifier le contenu » : ouvre le contenu de l'objet dynamique `layer` comme un document à part (nouvel onglet),
/// relié à son parent. L'enregistrer (Ctrl+S) appelle `smart_content_commit` ; le fermer sans enregistrer ne change
/// rien. Déjà ouvert : le même document est rendu. Un contenu lié à un projet `.graf` s'ouvre avec ce fichier pour
/// chemin d'enregistrement.
#[tauri::command]
pub async fn smart_content_open(state: State<'_, AppState>, doc_id: DocId, layer: LayerId) -> Result<DocumentInfo, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    let state = state.inner().clone();
    blocking(move || {
        let (content, name) = {
            let session = entry.read();
            let target = session.document().layer(layer)?;
            let content = target.smart().cloned().ok_or_else(|| EngineError::InvalidOperation("ce calque n'est pas un objet dynamique".into()))?;
            (content, target.props.name.clone())
        };
        if let Some((id, child)) = state.smart_child(doc_id, content.id) {
            return Ok(describe(id, &child, &child.read()));
        }
        let mut document = (*content.document).clone();
        document.name.clone_from(&content.name);
        let project_path = content.path().filter(|p| io::is_project_path(p)).map(Path::to_path_buf);
        let (id, child) = state.insert(document, "Ouvrir", project_path);
        child.set_smart_parent(SmartLink { parent: doc_id, content: content.id, name });
        let info = describe(id, &child, &child.read());
        Ok(info)
    })
    .await
}

/// Enregistrement (Ctrl+S) d'un contenu ouvert par « Modifier le contenu » : le contenu de l'objet dynamique est
/// remplacé dans le document parent, pour toutes ses instances, en une étape d'historique « Modifier le contenu » du
/// parent (annulable). Un contenu lié à un projet `.graf` est d'abord écrit dans ce fichier. Retourne le parent mis à
/// jour (`document.id` est celui du parent).
#[tauri::command]
pub async fn smart_content_commit(state: State<'_, AppState>, doc_id: DocId) -> Result<EditResult, CommandError> {
    let child = lookup(state.inner(), doc_id)?;
    let link = child.smart_parent().ok_or_else(|| EngineError::InvalidOperation("ce document n'est pas le contenu d'un objet dynamique".into()))?;
    let parent = state.get(link.parent).ok_or_else(|| {
        EngineError::InvalidOperation("le document de l'objet dynamique a été fermé : enregistrez ce contenu sous un autre nom pour le garder".into())
    })?;
    blocking(move || {
        let document = {
            let mut session = child.write();
            session.finish_stroke();
            session.document().clone()
        };
        let current = parent
            .read()
            .document()
            .walk_top_down()
            .into_iter()
            .find_map(|(_, l)| l.smart().filter(|c| c.id == link.content).cloned())
            .ok_or_else(|| EngineError::InvalidOperation(format!("l'objet dynamique « {} » n'existe plus dans son document", link.name)))?;
        if let Some(path) = current.path() {
            if !io::is_project_path(path) {
                return Err(EngineError::InvalidOperation(format!(
                    "contenu lié à l'image « {} » : modifiez ce fichier dans son logiciel, ou incorporez le lien pour le modifier ici",
                    path.display()
                ))
                .into());
            }
            io::save_project(&document, path)?;
        }
        let mut session = parent.write();
        let (_, dirty) = session.apply("Modifier le contenu", None, |doc| doc.update_smart_content(Arc::new(current.edited(document)), SmartFit::Centre))?;
        Ok(EditResult { document: describe(link.parent, &parent, &session), dirty: dirty.into() })
    })
    .await
}

/// « Exporter le contenu… » : vers un `.graf`, le document du contenu ; vers un autre fichier, l'image d'origine
/// d'un contenu placé (à l'octet près) ou une copie du fichier lié. Le nom proposé est `SmartInfo::export_name`.
#[tauri::command]
pub async fn smart_content_export(state: State<'_, AppState>, doc_id: DocId, layer: LayerId, path: PathBuf) -> Result<(), CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let content = entry.read().document().layer(layer)?.smart().cloned().ok_or_else(|| EngineError::InvalidOperation("ce calque n'est pas un objet dynamique".into()))?;
        if io::is_project_path(&path) {
            return Ok(io::save_project(&content.document, &path)?);
        }
        if let Some(original) = content.original() {
            return Ok(std::fs::write(&path, original.bytes.as_slice()).map_err(EngineError::io(&path))?);
        }
        if let Some(linked) = content.path().filter(|p| p.is_file()) {
            std::fs::copy(linked, &path).map_err(EngineError::io(&path))?;
            return Ok(());
        }
        Err(EngineError::InvalidOperation(format!("ce contenu est un document : exportez-le en .{PROJECT_EXTENSION}")).into())
    })
    .await
}

/// Bilan de `smart_links_check`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartLinksResult {
    document: DocumentInfo,
    dirty: DirtyInfo,
    /// Contenus relus (fichier modifié ou réapparu).
    updated: Vec<ContentId>,
    /// Contenus dont le fichier est introuvable.
    missing: Vec<ContentId>,
    /// Fichiers illisibles, avec la raison (le contenu précédent reste en place).
    failed: Vec<String>,
}

/// Vérifie les fichiers liés des objets dynamiques (au retour dans la fenêtre, par exemple) : les fichiers modifiés
/// sont relus et leurs instances refaites ; les introuvables sont signalés (`SmartInfo::missing`). Une étape
/// d'historique « Mettre à jour les objets liés » seulement si quelque chose a changé.
#[tauri::command]
pub async fn smart_links_check(state: State<'_, AppState>, doc_id: DocId) -> Result<SmartLinksResult, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let mut session = entry.write();
        let before: std::collections::HashSet<ContentId> = session.document().walk_top_down().into_iter().filter_map(|(_, l)| l.smart().filter(|c| c.missing()).map(|c| c.id)).collect();
        let mut probe = session.document().clone();
        let report = probe.refresh_smart_links();
        let changed = !report.updated.is_empty() || report.missing.iter().any(|id| !before.contains(id));
        let dirty = if changed {
            session
                .apply("Mettre à jour les objets liés", None, |doc| {
                    *doc = probe;
                    Ok(())
                })?
                .1
        } else {
            Dirty::default()
        };
        Ok(SmartLinksResult { document: describe(doc_id, &entry, &session), dirty: dirty.into(), updated: report.updated, missing: report.missing, failed: report.failed })
    })
    .await
}

// ---------------------------------------------------------------------- rendu

/// Réponse binaire : 8 octets de révision (petit-boutiste), puis les données.
fn tagged(revision: u64, payload: &[u8]) -> Response {
    let mut bytes = Vec::with_capacity(8 + payload.len());
    bytes.extend_from_slice(&revision.to_le_bytes());
    bytes.extend_from_slice(payload);
    Response::new(bytes)
}

/// Tuile composée : révision, puis RGBA8 prémultiplié brut (262 144 octets), ou
/// rien du tout si la tuile est entièrement transparente.
#[tauri::command]
pub async fn get_tile(
    state: State<'_, AppState>,
    doc_id: DocId,
    level: u8,
    x: i32,
    y: i32,
) -> Result<Response, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let session = entry.read();
        let tile = session.renderer().tile(session.document(), level, TileCoord::new(x, y));
        Ok(tagged(session.revision(), tile.as_ref().map_or(&[][..], |t| t.bytes())))
    })
    .await
}

/// Tuile de la sélection : révision (8 octets), valeur de fond (1 octet), puis 65 536
/// octets de masque, ou rien si la tuile est uniformément égale au fond.
#[tauri::command]
pub async fn get_selection_tile(
    state: State<'_, AppState>,
    doc_id: DocId,
    level: u8,
    x: i32,
    y: i32,
) -> Result<Response, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let session = entry.read();
        let doc = session.document();
        let background = doc.selection().map_or(0, |s| s.background()[0]);
        let tile = session.renderer().selection_tile(doc, level, TileCoord::new(x, y));
        let mut payload = vec![background];
        payload.extend_from_slice(tile.as_ref().map_or(&[][..], |t| t.bytes()));
        Ok(tagged(session.revision(), &payload))
    })
    .await
}

/// Curseur et sélection d'un calque de texte en cours d'édition, en repère document.
#[derive(Serialize)]
pub struct CaretInfo {
    /// Haut et bas du trait du curseur.
    caret: [(f64, f64); 2],
    /// Un quadrilatère par ligne sélectionnée.
    highlights: Vec<[(f64, f64); 4]>,
}

/// `start`, `end` : positions en unités UTF-16, celles d'un champ de saisie du navigateur.
#[tauri::command]
pub async fn text_caret(state: State<'_, AppState>, doc_id: DocId, layer: LayerId, start: usize, end: usize) -> Result<CaretInfo, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let (caret, highlights) = entry.read().document().text_caret(layer, start, end)?;
        Ok(CaretInfo { caret, highlights })
    })
    .await
}

/// Position du curseur (UTF-16) la plus proche d'un point du document.
#[tauri::command]
pub async fn text_hit(state: State<'_, AppState>, doc_id: DocId, layer: LayerId, x: f64, y: f64) -> Result<usize, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || Ok(entry.read().document().text_hit(layer, x, y)?)).await
}

/// Pipette : couleur de l'image composée en un point, ou `None` hors du document ou sur du transparent.
#[tauri::command]
pub async fn sample_color(state: State<'_, AppState>, doc_id: DocId, x: i32, y: i32) -> Result<Option<[u8; 3]>, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let session = entry.read();
        let doc = session.document();
        if x < 0 || y < 0 || x >= doc.width() as i32 || y >= doc.height() as i32 {
            return Ok(None);
        }
        let (coord, px, py) = TileCoord::of_pixel(x, y);
        let pixel = session.renderer().tile(doc, 0, coord).map(|tile| engine::pixel::unpremultiply(tile.pixel(px, py)));
        Ok(pixel.filter(|p| p[3] > 0).map(|p| [p[0], p[1], p[2]]))
    })
    .await
}

/// Aperçu d'un filtre sur une réduction du calque : 8 octets d'en-tête (largeur, hauteur en u32
/// petit-boutistes) puis les pixels RGBA à alpha droit. Ne modifie pas le document.
#[tauri::command]
pub async fn filter_preview(state: State<'_, AppState>, doc_id: DocId, layer: LayerId, filter: Filter, max_side: u32, in_context: Option<bool>) -> Result<Response, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let session = entry.read();
        // `in_context` : le calque filtré est recomposé avec tous les autres ; sinon, le calque seul.
        let (width, height, rgba) = if in_context == Some(true) { session.filter_preview_in_context(layer, &filter, max_side)? } else { session.document().filter_preview(layer, &filter, max_side)? };
        let mut body = Vec::with_capacity(8 + rgba.len());
        body.extend_from_slice(&width.to_le_bytes());
        body.extend_from_slice(&height.to_le_bytes());
        body.extend_from_slice(&rgba);
        Ok(Response::new(body))
    })
    .await
}

/// Familles de polices installées. Le premier appel recense les polices du système.
#[tauri::command]
pub async fn list_fonts() -> Result<Vec<text::FontFamily>, CommandError> {
    blocking(|| Ok(text::font_families())).await
}

/// Fichier d'une police incluse (variante normale), pour l'échantillon du sélecteur ; vide si la police vient du
/// système (la webview la connaît déjà par son nom).
#[tauri::command]
pub async fn bundled_font_file(family: String) -> Result<Response, CommandError> {
    blocking(move || Ok(Response::new(text::bundled_font_file(&family).and_then(|p| std::fs::read(p).ok()).unwrap_or_default()))).await
}

/// Étendue exacte du contenu (ou de la sélection active) : le cadre de Ctrl+T et la base du magnétisme.
#[tauri::command]
pub async fn content_bounds(
    state: State<'_, AppState>,
    doc_id: DocId,
    ids: Vec<LayerId>,
) -> Result<Option<(i32, i32, i32, i32)>, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || Ok(entry.read().document().content_bounds(&ids)?)).await
}

/// Étendues des autres calques visibles : cibles du magnétisme entre objets.
#[tauri::command]
pub async fn other_bounds(state: State<'_, AppState>, doc_id: DocId, exclude: Vec<LayerId>) -> Result<Vec<(i32, i32, i32, i32)>, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || Ok(entry.read().document().other_bounds(&exclude))).await
}

/// Outil Texte : calque de texte sous le point, même recouvert.
#[tauri::command]
pub async fn pick_text_layer(state: State<'_, AppState>, doc_id: DocId, x: i32, y: i32) -> Result<Option<LayerId>, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || Ok(entry.read().document().pick_text_layer(x, y))).await
}

/// Aperçu de déplacement : découpage de l'image en « dessous / déplacé / dessus », ou `None`
/// si l'image ne se reconstitue pas exactement ainsi (le moteur recompose alors à chaque pas).
#[tauri::command]
pub async fn move_plan(state: State<'_, AppState>, doc_id: DocId, ids: Vec<LayerId>) -> Result<Option<MovePlan>, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || Ok(entry.read().document().move_preview_plan(&ids))).await
}

/// Aperçu instantané d'un réglage : calques dessous et dessus (voir `adjustment_preview_plan`), ou rien.
#[derive(Serialize)]
pub struct AdjustPlan {
    below: Vec<LayerId>,
    /// Calques corrigés par le réglage écrêté ; vide = le réglage corrige tout ce qui est dessous.
    target: Vec<LayerId>,
    above: Vec<LayerId>,
}

#[tauri::command]
pub async fn adjust_plan(state: State<'_, AppState>, doc_id: DocId, layer: LayerId) -> Result<Option<AdjustPlan>, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || Ok(entry.read().document().adjustment_preview_plan(layer).map(|(below, target, above)| AdjustPlan { below, target, above }))).await
}

/// Table de correspondance 3D d'un réglage (`size`³ couleurs, RGBA8, rouge le plus rapide) : chaque réglage agit
/// pixel par pixel, la carte graphique peut donc l'appliquer instantanément à l'image pendant qu'on tire un curseur.
#[tauri::command]
pub fn adjustment_lut(adjustment: engine::adjust::Adjustment, size: u32) -> Response {
    let n = size.clamp(2, 65) as usize;
    let prepared = adjustment.prepare();
    let mut out = Vec::with_capacity(n * n * n * 4);
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let rgb = [r as f32 / (n - 1) as f32, g as f32 / (n - 1) as f32, b as f32 / (n - 1) as f32];
                let [ar, ag, ab] = prepared.apply(rgb);
                out.extend_from_slice(&[(ar.clamp(0.0, 1.0) * 255.0 + 0.5) as u8, (ag.clamp(0.0, 1.0) * 255.0 + 0.5) as u8, (ab.clamp(0.0, 1.0) * 255.0 + 0.5) as u8, 255]);
            }
        }
    }
    Response::new(out)
}

/// Réglage mis à plat pour la composition sur carte graphique de l'interface : son type (u32), puis ses valeurs
/// (f32), petit-boutistes. Le shader refait exactement les calculs du moteur (`Prepared::to_gpu`), sans table
/// intermédiaire : l'image est celle du moteur, aux arrondis près.
#[tauri::command]
pub fn adjustment_gpu(adjustment: engine::adjust::Adjustment) -> Response {
    let gpu = adjustment.prepare().to_gpu();
    let mut out = Vec::with_capacity(4 + gpu.values.len() * 4);
    out.extend_from_slice(&gpu.kind.to_le_bytes());
    for value in &gpu.values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    Response::new(out)
}

/// Composition sur carte graphique : pixels d'UN calque, en RGBA8 prémultiplié, dans son propre repère (la
/// carte graphique le place elle-même à sa position). `rect` = `[x, y, largeur, hauteur]` d'une partie à
/// recharger ; absent = tout le contenu. `part` : ce qui est lu, comme la composition le fond —
///   - absent : le calque avec ceux de ses effets qui s'y fondent (ses pixels seuls s'il n'a pas d'effet) ;
///   - `raw` : ses pixels seuls (forme d'une base d'écrêtage à effets) ;
///   - `under:<n>` / `over:<n>` : le n-ième effet fondu à part, sous ou sur le calque (voir `layer_effects`).
/// Réponse : 16 octets d'en-tête (x, y en i32, largeur, hauteur en u32, petit-boutistes) puis les pixels. Un
/// contenu vide répond 0 × 0. `limit` = `[côté maximal, nombre de pixels maximal]` d'une texture acceptable :
/// au-delà, seul l'en-tête est renvoyé (largeur et hauteur réelles, aucun pixel) — l'interface sait ainsi que le
/// calque ne tient pas sur la carte graphique sans qu'il soit lu ni transféré en entier.
/// Vrai si une texture de `w` × `h` dépasse `limit` (`[côté maximal, nombre de pixels maximal]`, voir `layer_pixels`).
fn texture_limit_exceeded(limit: Option<[u64; 2]>, w: u32, h: u32) -> bool {
    limit.is_some_and(|[side, pixels]| u64::from(w) > side || u64::from(h) > side || u64::from(w) * u64::from(h) > pixels)
}

#[tauri::command]
pub async fn layer_pixels(state: State<'_, AppState>, doc_id: DocId, layer: LayerId, rect: Option<[i32; 4]>, part: Option<String>, limit: Option<[u64; 2]>) -> Result<Response, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let session = entry.read();
        let target = session.document().layer(layer)?;
        let raw = target.pixels().ok_or_else(|| EngineError::InvalidOperation("ce calque n'a pas de pixels".into()))?;
        let styled = || session.renderer().styled_layer(target).ok_or_else(|| EngineError::InvalidOperation("ce calque n'a pas de pixels".into()));
        let effect = |list: &[(engine::blend::BlendMode, std::sync::Arc<engine::tiles::TileStore>)], n: &str| {
            let index: usize = n.parse().map_err(|_| EngineError::InvalidOperation(format!("partie inconnue : {n}")))?;
            list.get(index).map(|(_, store)| std::sync::Arc::clone(store)).ok_or_else(|| EngineError::InvalidOperation(format!("effet absent : {n}")))
        };
        let pixels = match part.as_deref() {
            None => std::sync::Arc::clone(&styled()?.main),
            Some("raw") => std::sync::Arc::clone(raw),
            Some(other) => match other.split_once(':') {
                Some(("under", n)) => effect(&styled()?.under, n)?,
                Some(("over", n)) => effect(&styled()?.over, n)?,
                _ => return Err(EngineError::InvalidOperation(format!("partie inconnue : {other}")).into()),
            },
        };
        let (x0, y0, w, h) = match rect {
            Some([x, y, w, h]) => (x, y, w.max(0) as u32, h.max(0) as u32),
            None => match pixels.pixel_bounds(|px| px[3] > 0) {
                Some((x0, y0, x1, y1)) => (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32),
                None => (0, 0, 0, 0),
            },
        };
        // Au-delà de la limite (voir `texture_limit_exceeded`) : l'en-tête seul, sans lire ni transférer les pixels.
        let rgba = if texture_limit_exceeded(limit, w, h) { Vec::new() } else { pixels.read_rect(x0, y0, w, h) };
        let mut body = Vec::with_capacity(16 + rgba.len());
        body.extend_from_slice(&x0.to_le_bytes());
        body.extend_from_slice(&y0.to_le_bytes());
        body.extend_from_slice(&w.to_le_bytes());
        body.extend_from_slice(&h.to_le_bytes());
        body.extend_from_slice(&rgba);
        Ok(Response::new(body))
    })
    .await
}

/// Effets d'un calque fondus à part dans l'image, dans l'ordre de la composition : les modes de ceux du dessous,
/// puis de ceux du dessus. Leurs pixels se lisent avec `layer_pixels` (`under:<n>`, `over:<n>`).
#[derive(Serialize)]
pub struct LayerEffectParts {
    under: Vec<engine::blend::BlendMode>,
    over: Vec<engine::blend::BlendMode>,
}

#[tauri::command]
pub async fn layer_effects(state: State<'_, AppState>, doc_id: DocId, layer: LayerId) -> Result<LayerEffectParts, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let session = entry.read();
        let target = session.document().layer(layer)?;
        let styled = session.renderer().styled_layer(target).ok_or_else(|| EngineError::InvalidOperation("ce calque n'a pas de pixels".into()))?;
        Ok(LayerEffectParts { under: styled.under.iter().map(|(mode, _)| *mode).collect(), over: styled.over.iter().map(|(mode, _)| *mode).collect() })
    })
    .await
}

/// Composition sur carte graphique : masque d'UN calque, un octet par pixel, dans le repère du calque. `rect` =
/// `[x, y, largeur, hauteur]` d'une partie à recharger ; absent = toutes les zones peintes du masque. Réponse :
/// 20 octets d'en-tête (x, y en i32, largeur, hauteur, fond en u32, petit-boutistes) puis les valeurs. Le fond est
/// la valeur du masque partout ailleurs. Un calque sans masque répond une erreur.
#[tauri::command]
pub async fn layer_mask(state: State<'_, AppState>, doc_id: DocId, layer: LayerId, rect: Option<[i32; 4]>, limit: Option<[u64; 2]>) -> Result<Response, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let session = entry.read();
        let target = session.document().layer(layer)?;
        let mask = target.mask.as_ref().ok_or_else(|| EngineError::InvalidOperation("ce calque n'a pas de masque".into()))?;
        let (x0, y0, w, h) = match rect {
            Some([x, y, w, h]) => (x, y, w.max(0) as u32, h.max(0) as u32),
            None => match mask.pixels.bounds() {
                Some((x0, y0, x1, y1)) => (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32),
                None => (0, 0, 0, 0),
            },
        };
        let values = if texture_limit_exceeded(limit, w, h) { Vec::new() } else { mask.pixels.read_rect(x0, y0, w, h) };
        let mut body = Vec::with_capacity(20 + values.len());
        body.extend_from_slice(&x0.to_le_bytes());
        body.extend_from_slice(&y0.to_le_bytes());
        body.extend_from_slice(&w.to_le_bytes());
        body.extend_from_slice(&h.to_le_bytes());
        body.extend_from_slice(&u32::from(mask.pixels.background()[0]).to_le_bytes());
        body.extend_from_slice(&values);
        Ok(Response::new(body))
    })
    .await
}

/// `opaque` : ce calque est rendu à pleine opacité, l'interface appliquant la sienne.
#[tauri::command]
pub async fn get_tile_part(
    state: State<'_, AppState>,
    doc_id: DocId,
    level: u8,
    x: i32,
    y: i32,
    layers: Vec<LayerId>,
    opaque: Option<LayerId>,
) -> Result<Response, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let session = entry.read();
        let include = |id: LayerId| layers.contains(&id);
        let tile = session.renderer().tile_filtered(session.document(), level, TileCoord::new(x, y), &include, opaque);
        Ok(tagged(session.revision(), tile.as_ref().map_or(&[][..], |t| t.bytes())))
    })
    .await
}

/// Sélection automatique de l'outil Déplacement : calque visible le plus haut sous le point.
#[tauri::command]
pub async fn pick_layer(state: State<'_, AppState>, doc_id: DocId, x: i32, y: i32) -> Result<Option<LayerId>, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || Ok(entry.read().document().pick_layer(x, y))).await
}

/// Miniature d'un calque ou de son masque : largeur et hauteur (u32 petit-boutistes)
/// puis RGBA à alpha droit, réduite à 96 px de côté au plus.
#[tauri::command]
pub async fn thumbnail(
    state: State<'_, AppState>,
    doc_id: DocId,
    layer: LayerId,
    mask: bool,
) -> Result<Response, CommandError> {
    const MAX_SIDE: u32 = 96;
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let session = entry.read();
        let Some((w, h, rgba)) = session.renderer().thumbnail(session.document(), layer, mask) else {
            return Ok(Response::new(Vec::new()));
        };
        let step = w.max(h).div_ceil(MAX_SIDE).max(1);
        let (tw, th) = (w.div_ceil(step), h.div_ceil(step));
        let mut bytes = Vec::with_capacity(8 + (tw * th * 4) as usize);
        bytes.extend_from_slice(&tw.to_le_bytes());
        bytes.extend_from_slice(&th.to_le_bytes());
        for y in (0..h).step_by(step as usize) {
            for x in (0..w).step_by(step as usize) {
                let i = ((y * w + x) * 4) as usize;
                bytes.extend_from_slice(&rgba[i..i + 4]);
            }
        }
        Ok(Response::new(bytes))
    })
    .await
}

// ------------------------------------------------------------------- édition

/// Propriétés modifiables d'un calque ; seuls les champs présents changent.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PropsPatch {
    name: Option<String>,
    visible: Option<bool>,
    opacity: Option<f32>,
    fill: Option<f32>,
    blend: Option<BlendMode>,
    locks: Option<Locks>,
    clipped: Option<bool>,
}

impl PropsPatch {
    /// Libellé d'historique et clé de fusion (les curseurs ne créent qu'un état par glissement).
    fn describe(&self, id: LayerId) -> (&'static str, Option<String>) {
        if self.opacity.is_some() {
            ("Opacité", Some(format!("opacity:{id}")))
        } else if self.fill.is_some() {
            ("Fond", Some(format!("fill:{id}")))
        } else if self.name.is_some() {
            ("Renommer le calque", None)
        } else if self.visible.is_some() {
            // Cliquer sur plusieurs yeux d'affilée ne doit pas noyer l'historique.
            ("Visibilité des calques", Some("visibility".into()))
        } else if self.blend.is_some() {
            ("Mode de fusion", None)
        } else if self.clipped.is_some() {
            ("Masque d'écrêtage", None)
        } else {
            ("Verrouillage", None)
        }
    }
}

fn yes() -> bool {
    true
}

fn one() -> u32 {
    1
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum EditOp {
    /// Nouveau calque vide au-dessus de `above` (ou au sommet).
    AddLayer { above: Option<LayerId> },
    AddGroup { above: Option<LayerId> },
    ImportLayer { path: PathBuf, above: Option<LayerId> },
    Delete { ids: Vec<LayerId> },
    Move { ids: Vec<LayerId>, parent: Option<LayerId>, index: usize },
    Group { ids: Vec<LayerId> },
    Duplicate { id: LayerId },
    /// `gesture` : aperçu en direct (survol des modes de fusion). Chaque appel de même geste
    /// remplace le précédent ; `CancelGesture` remet tout en l'état si rien n'est choisi.
    SetProps { ids: Vec<LayerId>, patch: PropsPatch, gesture: Option<String> },
    SetExpanded { id: LayerId, expanded: bool },
    /// Couleur d'étiquette de calques ou dossiers dans le panneau (`null` : aucune).
    SetLabel { ids: Vec<LayerId>, label: Option<LayerLabel> },
    AddMask { id: LayerId, reveal_all: bool },
    RemoveMask { id: LayerId },
    SetMaskEnabled { id: LayerId, enabled: bool },
    Fill { id: LayerId, color: [u8; 3], on_mask: bool },
    /// Outil Texte : nouveau calque ancré en `(x, y)`.
    AddText { x: i32, y: i32, text: TextData, above: Option<LayerId> },
    /// `session` : les frappes d'une même séance d'édition ne font qu'un état d'historique.
    SetText { id: LayerId, text: TextData, session: String },
    /// Pixellise un texte, une forme ou un objet dynamique (rendu actuel).
    Rasterize { id: LayerId },
    /// « Convertir en objet dynamique » : les calques deviennent le contenu d'un objet dynamique qui prend leur place.
    ConvertToSmart { ids: Vec<LayerId> },
    /// « Placer » : fichier image ou projet placé comme objet dynamique, incorporé ou lié (`linked`). Emplacement :
    /// `parent` + `index` s'ils sont donnés, sinon au-dessus de `above` (ou au sommet).
    PlaceSmartObject {
        path: PathBuf,
        #[serde(default)]
        linked: bool,
        #[serde(default)]
        above: Option<LayerId>,
        #[serde(default)]
        parent: Option<LayerId>,
        #[serde(default)]
        index: Option<usize>,
    },
    /// « Remplacer le contenu » (toutes les instances), par un fichier image ou projet ; incorporé ou lié comme avant.
    ReplaceSmartContent { id: LayerId, path: PathBuf },
    /// « Relier au fichier » : le contenu devient lié à `path` (toutes les instances).
    RelinkSmart { id: LayerId, path: PathBuf },
    /// « Incorporer le lien » (toutes les instances).
    EmbedSmartLink { id: LayerId },
    /// « Réinitialiser la transformation » : 100 %, droit, centré sur la position actuelle.
    ResetSmartTransform { id: LayerId },
    /// « Nouvel objet dynamique par copie » : copie au contenu indépendant, juste au-dessus.
    NewSmartByCopy { id: LayerId },
    /// Suppr avec une sélection active : efface les pixels sélectionnés (`on_mask` : masque la sélection dans le masque de fusion).
    ClearPixels {
        id: LayerId,
        #[serde(default)]
        on_mask: bool,
    },
    /// Ctrl+C / Ctrl+X : pixels sélectionnés du calque (tout le calque sans sélection) vers le presse-papiers.
    Copy { id: LayerId },
    Cut { id: LayerId },
    /// Ctrl+V : nouveau calque, au même endroit que l'original.
    Paste { above: Option<LayerId> },
    BucketFill { id: LayerId, x: u32, y: u32, color: [u8; 3], tolerance: u8, contiguous: bool },
    /// Arrêts en RGBA à alpha droit ; points en pixels du document.
    GradientFill { id: LayerId, from: (f32, f32), to: (f32, f32), stops: Vec<GradientStop>, radial: bool,
        /// Tracé dans le masque de fusion plutôt que dans les pixels.
        #[serde(default)]
        on_mask: bool,
    },
    /// Alt + glisser la miniature d'un masque vers un autre calque : copie (ou déplacement sans Alt).
    CopyMask { from: LayerId, to: LayerId, remove_source: bool },
    /// `clipped` : écrêté sur le calque `above`, pour ne corriger que lui.
    AddAdjustment { adjustment: Adjustment, above: Option<LayerId>, clipped: bool },
    /// Cadre de transformation tiré sur une forme, sans rotation : la forme est redessinée à la nouvelle taille.
    PlaceShape { id: LayerId, x: i32, y: i32, width: f32, height: f32, gesture: String },
    /// Alt + glisser dans le panneau Calques : copie des calques (groupes compris) déposée à l'emplacement visé.
    DuplicateTo { ids: Vec<LayerId>, parent: Option<LayerId>, index: usize },
    /// Ctrl+I avec le masque de fusion pour cible : l'inverse.
    InvertMask { id: LayerId },
    /// Fusionne plusieurs calques (formes, textes, tracés compris) en un seul calque de pixels.
    MergeLayers { ids: Vec<LayerId> },
    /// Fusionne un groupe en un seul calque.
    MergeGroup { id: LayerId },
    /// Alt + clic sur l'œil : isole le calque, ou rallume tout s'il l'était déjà.
    Solo { id: LayerId },
    /// Effets de calque (fx) : la liste complète remplace l'ancienne ; les réglages successifs ne font qu'un état d'historique.
    /// `gesture` : fenêtre des effets ouverte — tous ses réglages forment un seul geste, qu'Annuler retire en bloc.
    SetEffects {
        id: LayerId,
        effects: Vec<LayerEffect>,
        #[serde(default)]
        gesture: Option<String>,
    },
    /// Outil Forme : boîte de la forme en `(x, y)`.
    AddShape { x: i32, y: i32, shape: ShapeData, above: Option<LayerId> },
    /// Les réglages successifs d'une même forme ne font qu'un état d'historique.
    SetShape { id: LayerId, shape: ShapeData },
    /// Correcteur localisé : trait (pixels du document) dont la zone est reconstruite.
    Heal { id: LayerId, points: Vec<(f32, f32)>, size: f32, hardness: f32 },
    /// Recadrage (ou agrandissement de la zone de travail) : rectangle en pixels du document actuel.
    Crop { x: i32, y: i32, width: u32, height: u32 },
    /// Ctrl+I : inverse les pixels du calque.
    Invert { id: LayerId },
    /// Les réglages successifs d'un même calque ne font qu'un état d'historique.
    SetAdjustment { id: LayerId, adjustment: Adjustment },
    /// `gesture` : aperçu en direct depuis la boîte de dialogue du filtre ; chaque appel repart
    /// de l'image d'avant, et `CancelGesture` retire tout si l'on annule.
    ApplyFilter { id: LayerId, filter: Filter, gesture: Option<String> },
    Select { shape: Shape, mode: CombineMode },
    MagicWand {
        layer: LayerId,
        x: u32,
        y: u32,
        tolerance: u8,
        contiguous: bool,
        /// Bord lissé (par défaut).
        #[serde(default = "yes")]
        anti_alias: bool,
        /// Côté du carré moyenné pour la couleur de référence : 1, 3 ou 5.
        #[serde(default = "one")]
        sample: u32,
        /// Lire l'image composée plutôt que le seul calque actif.
        #[serde(default)]
        all_layers: bool,
        /// Présent quand on règle la tolérance en glissant : les envois successifs ne font qu'un état d'historique.
        #[serde(default)]
        gesture: Option<String>,
        mode: CombineMode,
    },
    /// Sélection rapide : trait (pixels du document) à partir duquel la sélection s'étend.
    QuickSelect { layer: LayerId, points: Vec<(f32, f32)>, size: f32, tolerance: u8, all_layers: bool, mode: CombineMode },
    /// Dilater (> 0) ou contracter (< 0) la sélection, puis arrondir son contour.
    ModifySelection { amount: i32, smooth: u32 },
    /// Outil Déplacement : `dx`, `dy` = déplacement TOTAL depuis le début du geste `gesture`.
    Translate { ids: Vec<LayerId>, dx: i32, dy: i32, gesture: String,
        /// Calque à masque : « pixels » ou « mask » ne déplace que cette partie ; absent ou « both » : les deux.
        #[serde(default)]
        parts: Option<String>,
    },
    /// Outil Déplacement avec une sélection active : découpe et déplace les pixels sélectionnés.
    MovePixels { id: LayerId, dx: i32, dy: i32, gesture: String },
    /// Outil de sélection glissé à l'intérieur d'une sélection : déplace son contour seul.
    TranslateSelection { dx: i32, dy: i32, gesture: String },
    /// Ctrl+T : `matrix` = transformation TOTALE depuis le début du geste, en repère document.
    /// `finalize` : rendu de qualité (validation), sinon aperçu rapide.
    Transform { ids: Vec<LayerId>, matrix: [f64; 9], finalize: bool, gesture: String },
    /// Échap pendant une transformation : retire l'aperçu de l'historique.
    CancelGesture { gesture: String },
    /// Plume : l'interface envoie le tracé complet après chaque manipulation.
    SetPath { paths: Vec<SubPath>, label: String },
    PathToSelection { mode: CombineMode },
    FillPath { id: LayerId, color: [u8; 3] },
    StrokePath { id: LayerId, brush: BrushParams },
    AddGuide { orientation: Orientation, position: f32 },
    /// Disposition de repères : colonnes, rangées, marges, posées d'un coup. `clear` : les repères existants sont d'abord retirés.
    SetGuides { guides: Vec<Guide>, clear: bool },
    SetGuidesLocked { locked: bool },
    MoveGuide { index: usize, position: f32 },
    RemoveGuide { index: usize },
    ClearGuides,
    /// Ctrl+A. Avec un calque : son contenu opaque (ou tout le document s'il est vide) ; sans : tout le document.
    SelectAll {
        #[serde(default)]
        layer: Option<LayerId>,
    },
    Deselect,
    InvertSelection,
    Feather { radius: f32 },
    Undo,
    Redo,
    Jump { index: usize },
}

/// Emplacement d'insertion : au-dessus du calque donné, sinon au sommet du document.
fn slot(doc: &Document, above: Option<LayerId>) -> Result<(Option<LayerId>, usize), EngineError> {
    match above {
        Some(id) => doc.slot_above(id),
        None => Ok((None, doc.children_of(None)?.len())),
    }
}

/// « Calque 3 », « Groupe 2 »… d'après le nombre d'éléments existants du même genre.
fn next_name(doc: &Document, prefix: &str, groups: bool) -> String {
    let count = doc.walk_top_down().iter().filter(|(_, l)| l.is_group() == groups).count();
    // L'arrière-plan compte comme un calque : le premier calque ajouté s'appelle « Calque 1 ».
    format!("{prefix} {}", if groups { count + 1 } else { count.max(1) })
}

fn apply(session: &mut Session, op: EditOp, clipboard: Option<Clip>) -> Result<Dirty, EngineError> {
    let done = |result: Result<((), Dirty), EngineError>| result.map(|(_, dirty)| dirty);
    match op {
        EditOp::Undo => Ok(session.undo()),
        EditOp::Redo => Ok(session.redo()),
        EditOp::Jump { index } => Ok(session.jump(index)),
        EditOp::SetExpanded { id, expanded } => {
            session.apply_silently(|doc| doc.set_expanded(id, expanded))?;
            Ok(Dirty::default())
        }
        EditOp::SetLabel { ids, label } => done(session.apply("Couleur du calque", None, |doc| ids.iter().try_for_each(|id| doc.set_label(*id, label)))),
        EditOp::AddLayer { above } => done(session.apply("Nouveau calque", None, |doc| {
            let (parent, index) = slot(doc, above)?;
            let name = next_name(doc, "Calque", false);
            doc.add_pixel_layer(name, engine::TileStore::new(), parent, index).map(|_| ())
        })),
        EditOp::AddGroup { above } => done(session.apply("Nouveau groupe", None, |doc| {
            let (parent, index) = slot(doc, above)?;
            let name = next_name(doc, "Groupe", true);
            doc.add_group(name, parent, index).map(|_| ())
        })),
        EditOp::ImportLayer { path, above } => done(session.apply("Importer un calque", None, |doc| {
            let (parent, index) = slot(doc, above)?;
            io::import_as_layer(doc, &path, parent, index).map(|_| ())
        })),
        EditOp::Delete { ids } => done(session.apply("Supprimer le calque", None, |doc| doc.remove_layers(&ids))),
        EditOp::Move { ids, parent, index } => {
            done(session.apply("Ordre des calques", None, |doc| doc.move_layers(&ids, parent, index)))
        }
        EditOp::Group { ids } => done(session.apply("Grouper les calques", None, |doc| {
            let name = next_name(doc, "Groupe", true);
            doc.group_layers(&ids, name).map(|_| ())
        })),
        EditOp::Duplicate { id } => {
            done(session.apply("Dupliquer le calque", None, |doc| doc.duplicate_layer(id).map(|_| ())))
        }
        EditOp::SetProps { ids, patch, gesture } => {
            let (label, coalesce) = patch.describe(ids.first().copied().unwrap_or_default());
            let change = |doc: &mut Document| {
                for id in &ids {
                    doc.edit_props(*id, |props| {
                        if let Some(name) = &patch.name {
                            props.name.clone_from(name);
                        }
                        props.visible = patch.visible.unwrap_or(props.visible);
                        props.opacity = patch.opacity.unwrap_or(props.opacity);
                        props.fill = patch.fill.unwrap_or(props.fill);
                        props.blend = patch.blend.unwrap_or(props.blend);
                        props.locks = patch.locks.unwrap_or(props.locks);
                        props.clipped = patch.clipped.unwrap_or(props.clipped);
                    })?;
                }
                Ok(())
            };
            match gesture {
                Some(gesture) => done(session.apply_gesture(label, gesture, change)),
                None => done(session.apply(label, coalesce, change)),
            }
        }
        EditOp::AddMask { id, reveal_all } => {
            done(session.apply("Ajouter un masque de fusion", None, |doc| doc.add_mask(id, reveal_all)))
        }
        EditOp::RemoveMask { id } => {
            done(session.apply("Supprimer le masque de fusion", None, |doc| doc.remove_mask(id)))
        }
        EditOp::SetMaskEnabled { id, enabled } => {
            let label = if enabled { "Activer le masque de fusion" } else { "Désactiver le masque de fusion" };
            done(session.apply(label, None, |doc| doc.set_mask_enabled(id, enabled)))
        }
        EditOp::AddText { x, y, text, above } => done(session.apply("Nouveau calque de texte", None, |doc| {
            let (parent, index) = slot(doc, above)?;
            doc.add_text_layer(text, (x, y), parent, index).map(|_| ())
        })),
        EditOp::SetText { id, text, session: editing } => {
            done(session.apply("Modifier le texte", Some(editing), |doc| doc.set_text(id, text)))
        }
        EditOp::Rasterize { id } => done(session.apply("Pixelliser le calque", None, |doc| doc.rasterize_layer(id))),
        EditOp::ConvertToSmart { ids } => done(session.apply("Convertir en objet dynamique", None, |doc| doc.convert_to_smart(&ids).map(|_| ()))),
        EditOp::PlaceSmartObject { path, linked, above, parent, index } => {
            // Lecture et décodage du fichier hors de l'historique : une erreur de fichier ne crée aucun état.
            let content = smart::content_from_file(&path, linked)?;
            done(session.apply("Placer un objet dynamique", None, |doc| {
                let (parent, index) = match index {
                    Some(index) => (parent, index),
                    None => slot(doc, above)?,
                };
                doc.place_smart_object(content, parent, index).map(|_| ())
            }))
        }
        EditOp::ReplaceSmartContent { id, path } => {
            let linked = session.document().layer(id)?.smart().is_some_and(|c| c.is_linked());
            let content = smart::content_from_file(&path, linked)?;
            done(session.apply("Remplacer le contenu", None, |doc| doc.replace_smart_content(id, content).map(|_| ())))
        }
        EditOp::RelinkSmart { id, path } => {
            let content = smart::content_from_file(&path, true)?;
            done(session.apply("Relier l'objet dynamique", None, |doc| doc.replace_smart_content(id, content).map(|_| ())))
        }
        EditOp::EmbedSmartLink { id } => done(session.apply("Incorporer le lien", None, |doc| doc.embed_smart_link(id).map(|_| ()))),
        EditOp::ResetSmartTransform { id } => done(session.apply("Réinitialiser la transformation", None, |doc| doc.reset_smart_transform(id))),
        EditOp::NewSmartByCopy { id } => done(session.apply("Nouvel objet dynamique par copie", None, |doc| doc.new_smart_by_copy(id).map(|_| ()))),
        EditOp::ClearPixels { id, on_mask } => {
            if on_mask {
                done(session.apply("Masquer la sélection", None, |doc| doc.clear_selected_mask(id)))
            } else {
                done(session.apply("Effacer", None, |doc| doc.clear_selected_pixels(id)))
            }
        }
        EditOp::Copy { .. } => Ok(Dirty::default()), // la copie a été faite avant : le document ne change pas
        EditOp::Cut { id } => done(session.apply("Couper", None, |doc| doc.clear_selected_pixels(id))),
        EditOp::Paste { above } => {
            let clip = clipboard.ok_or_else(|| EngineError::InvalidOperation("le presse-papiers est vide".into()))?;
            done(session.apply("Coller", None, |doc| {
                let (parent, index) = slot(doc, above)?;
                let name = next_name(doc, "Calque", false);
                doc.paste(&clip, name, parent, index).map(|_| ())
            }))
        }
        EditOp::BucketFill { id, x, y, color, tolerance, contiguous } => done(session.apply("Pot de peinture", None, |doc| {
            doc.bucket_fill(id, (x, y), color, tolerance, contiguous)
        })),
        EditOp::GradientFill { id, from, to, stops, radial, on_mask } => {
            // Masque de fusion pour cible : le dégradé s'y inscrit en niveaux de gris.
            if on_mask {
                done(session.apply("Dégradé dans le masque", None, |doc| doc.gradient_mask(id, from, to, &stops, radial)))
            } else {
                done(session.apply("Dégradé", None, |doc| doc.gradient_fill(id, from, to, &stops, radial)))
            }
        }
        EditOp::CopyMask { from, to, remove_source } => {
            done(session.apply(if remove_source { "Déplacer le masque" } else { "Copier le masque" }, None, |doc| doc.copy_mask(from, to, remove_source)))
        }
        EditOp::AddAdjustment { adjustment, above, clipped } => done(session.apply(adjustment.label(), None, |doc| {
            let (parent, index) = slot(doc, above)?;
            doc.add_adjustment_layer(adjustment, parent, index, clipped && above.is_some()).map(|_| ())
        })),
        EditOp::PlaceShape { id, x, y, width, height, gesture } => {
            done(session.apply_gesture("Redimensionner la forme", gesture, |doc| doc.place_shape(id, x, y, width, height)))
        }
        EditOp::DuplicateTo { ids, parent, index } => done(session.apply("Dupliquer les calques", None, |doc| {
            let clip = doc.copy_layers(&ids)?;
            doc.paste_layers(&clip, parent, index).map(|_| ())
        })),
        EditOp::InvertMask { id } => done(session.apply("Inverser le masque", None, |doc| doc.invert_mask(id))),
        EditOp::MergeLayers { ids } => done(session.apply("Fusionner les calques", None, |doc| doc.merge_layers(&ids).map(|_| ()))),
        EditOp::MergeGroup { id } => done(session.apply("Fusionner le groupe", None, |doc| doc.merge_group(id).map(|_| ()))),
        EditOp::Solo { id } => done(session.apply("Isoler le calque", None, |doc| doc.solo(id))),
        EditOp::SetEffects { id, effects, gesture } => match gesture {
            Some(gesture) => done(session.apply_gesture("Effets de calque", gesture, |doc| doc.set_effects(id, effects))),
            None => done(session.apply("Effets de calque", Some(format!("fx-{id}")), |doc| doc.set_effects(id, effects))),
        },
        EditOp::AddShape { x, y, shape, above } => done(session.apply("Nouvelle forme", None, |doc| {
            let (parent, index) = slot(doc, above)?;
            doc.add_shape_layer(shape, (x, y), parent, index).map(|_| ())
        })),
        EditOp::SetShape { id, shape } => done(session.apply("Modifier la forme", Some(format!("shape-{id}")), |doc| doc.set_shape(id, shape))),
        EditOp::Heal { id, points, size, hardness } => done(session.apply("Correcteur localisé", None, |doc| doc.heal(id, &points, size, hardness))),
        EditOp::Crop { x, y, width, height } => done(session.apply("Recadrer", None, |doc| doc.crop(x, y, width, height))),
        EditOp::Invert { id } => done(session.apply("Inverser", None, |doc| doc.invert_layer(id))),
        EditOp::SetAdjustment { id, adjustment } => {
            done(session.apply("Modifier le réglage", Some(format!("adjust-{id}")), |doc| doc.set_adjustment(id, adjustment)))
        }
        EditOp::ApplyFilter { id, filter, gesture } => match gesture {
            Some(gesture) => done(session.apply_gesture(filter.label(), gesture, |doc| doc.apply_filter(id, &filter))),
            None => done(session.apply(filter.label(), None, |doc| doc.apply_filter(id, &filter))),
        },
        EditOp::Select { shape, mode } => done(session.apply("Sélection", None, |doc| {
            let mask = shape.rasterize(doc.width(), doc.height());
            let combined = selection::combine(doc.selection().map(|s| &**s), mask, mode);
            doc.set_selection(combined);
            Ok(())
        })),
        EditOp::MagicWand { layer, x, y, tolerance, contiguous, anti_alias, sample, all_layers, gesture, mode } => {
            // « Tous les calques » : la baguette lit l'image composée, telle qu'elle est affichée.
            let flat = all_layers.then(|| session.flat_store());
            let wand = selection::Wand { tolerance, contiguous, anti_alias, sample };
            let run = |doc: &mut Document| {
                let (pixels, offset) = match &flat {
                    Some(store) => (Arc::new(store.clone()), (0, 0)),
                    None => {
                        let target = doc.layer(layer)?;
                        let pixels = target.pixels().cloned().ok_or_else(|| EngineError::InvalidOperation("la baguette magique demande un calque de pixels, ou l'option « Tous les calques »".into()))?;
                        (pixels, target.offset)
                    }
                };
                let Some(mask) = selection::wand_select(&pixels, offset, (doc.width(), doc.height()), (x, y), wand) else { return Ok(()) };
                let combined = selection::combine(doc.selection().map(|s| &**s), mask, mode);
                doc.set_selection(combined);
                Ok(())
            };
            // Tolérance réglée en glissant : chaque envoi du même geste repart de la sélection d'avant le clic.
            match gesture {
                Some(gesture) => done(session.apply_gesture("Baguette magique", gesture, run)),
                None => done(session.apply("Baguette magique", None, run)),
            }
        }
        EditOp::QuickSelect { layer, points, size, tolerance, all_layers, mode } => {
            let flat = all_layers.then(|| session.flat_store());
            done(session.apply("Sélection rapide", None, |doc| {
                let (pixels, offset) = match &flat {
                    Some(store) => (Arc::new(store.clone()), (0, 0)),
                    None => {
                        let target = doc.layer(layer)?;
                        let pixels = target.pixels().cloned().ok_or_else(|| EngineError::InvalidOperation("la sélection rapide demande un calque de pixels, ou l'option « Tous les calques »".into()))?;
                        (pixels, target.offset)
                    }
                };
                // La sélection s'étend au plus à quatre diamètres de pinceau du trait : on la guide en peignant.
                let brushed = engine::paint::stroke_mask(&points, size, 1.0);
                let limit = engine::paint::stroke_mask(&points, size * 8.0, 1.0);
                let mask = selection::quick_select(&pixels, offset, (doc.width(), doc.height()), &brushed, &limit, tolerance);
                let combined = selection::combine(doc.selection().map(|s| &**s), mask, mode);
                doc.set_selection(combined);
                Ok(())
            }))
        }
        EditOp::ModifySelection { amount, smooth } => done(session.apply("Modifier la sélection", None, |doc| {
            let Some(current) = doc.selection().cloned() else { return Ok(()) };
            let modified = selection::modify(&current, (doc.width(), doc.height()), amount.clamp(-500, 500), smooth.min(200));
            doc.set_selection(modified);
            Ok(())
        })),
        EditOp::Translate { ids, dx, dy, gesture, parts } => done(session.apply_gesture("Déplacer", gesture, |doc| match (ids.as_slice(), parts.as_deref()) {
            // Un calque à masque dont une seule partie est ciblée : elle bouge seule.
            ([id], Some("pixels")) => doc.translate_parts(*id, dx, dy, true, false),
            ([id], Some("mask")) => doc.translate_parts(*id, dx, dy, false, true),
            _ => doc.translate_layers(&ids, dx, dy),
        })),
        EditOp::MovePixels { id, dx, dy, gesture } => {
            done(session.apply_gesture("Déplacer les pixels", gesture, |doc| doc.move_selected_pixels(id, dx, dy)))
        }
        EditOp::TranslateSelection { dx, dy, gesture } => {
            done(session.apply_gesture("Déplacer la sélection", gesture, |doc| {
                let moved = doc.selection().map(|s| s.translated(dx, dy));
                doc.set_selection(moved);
                Ok(())
            }))
        }
        EditOp::Transform { ids, matrix, finalize, gesture } => {
            let quality = if finalize { Quality::Final } else { Quality::Preview };
            done(session.apply_gesture("Transformation manuelle", gesture, |doc| {
                doc.transform_layers(&ids, &Matrix(matrix), quality)
            }))
        }
        EditOp::CancelGesture { gesture } => Ok(session.cancel_gesture(&gesture)),
        EditOp::SetPath { paths, label } => done(session.apply(label, None, |doc| {
            doc.paths = paths;
            Ok(())
        })),
        EditOp::PathToSelection { mode } => {
            done(session.apply("Tracé en sélection", None, |doc| doc.path_to_selection(mode)))
        }
        EditOp::FillPath { id, color } => done(session.apply("Fond du tracé", None, |doc| doc.fill_path(id, color))),
        EditOp::StrokePath { id, brush } => {
            done(session.apply("Contour du tracé", None, |doc| doc.stroke_path(id, brush.into())))
        }
        EditOp::SetGuides { guides, clear } => done(session.apply("Disposition de repères", None, |doc| {
            if clear {
                doc.guides.clear();
            }
            doc.guides.extend(guides.into_iter().filter(|g| g.position.is_finite()));
            // Une disposition qu'on vient de poser se verrouille : elle ne bougera pas par accident.
            doc.guides_locked = true;
            Ok(())
        })),
        EditOp::SetGuidesLocked { locked } => done(session.apply(if locked { "Verrouiller les repères" } else { "Déverrouiller les repères" }, None, |doc| {
            doc.guides_locked = locked;
            Ok(())
        })),
        EditOp::AddGuide { orientation, position } => done(session.apply("Nouveau repère", None, |doc| {
            doc.guides.push(Guide { orientation, position });
            Ok(())
        })),
        EditOp::MoveGuide { index, position } => done(session.apply("Déplacer le repère", None, |doc| {
            if doc.guides_locked {
                return Err(EngineError::InvalidOperation("les repères sont verrouillés".into()));
            }
            let guide = doc.guides.get_mut(index).ok_or_else(|| EngineError::InvalidOperation("repère introuvable".into()))?;
            guide.position = position;
            Ok(())
        })),
        EditOp::RemoveGuide { index } => done(session.apply("Supprimer le repère", None, |doc| {
            if doc.guides_locked {
                return Err(EngineError::InvalidOperation("les repères sont verrouillés".into()));
            }
            if index >= doc.guides.len() {
                return Err(EngineError::InvalidOperation("repère introuvable".into()));
            }
            doc.guides.remove(index);
            Ok(())
        })),
        EditOp::ClearGuides => done(session.apply("Effacer les repères", None, |doc| {
            doc.guides.clear();
            Ok(())
        })),
        EditOp::SelectAll { layer } => done(session.apply("Tout sélectionner", None, |doc| match layer {
            Some(id) => doc.select_layer_content(id),
            None => {
                doc.set_selection(Some(selection::select_all()));
                Ok(())
            }
        })),
        EditOp::Deselect => done(session.apply("Désélectionner", None, |doc| {
            doc.set_selection(None);
            Ok(())
        })),
        EditOp::InvertSelection => done(session.apply("Intervertir la sélection", None, |doc| {
            let inverted = selection::invert(doc.selection().map(|s| &**s));
            doc.set_selection(inverted);
            Ok(())
        })),
        EditOp::Feather { radius } => done(session.apply("Contour progressif", None, |doc| {
            let Some(current) = doc.selection().cloned() else {
                return Err(EngineError::InvalidOperation("aucune sélection à adoucir".into()));
            };
            let softened = selection::feather(&current, radius.clamp(0.1, 250.0), doc.width(), doc.height());
            doc.set_selection(Some(softened));
            Ok(())
        })),
        EditOp::Fill { id, color, on_mask } => done(session.apply("Remplir", None, |doc| {
            if on_mask {
                let [r, g, b] = color.map(u32::from);
                doc.fill_mask(id, ((r * 30 + g * 59 + b * 11 + 50) / 100) as u8)
            } else {
                doc.fill_layer(id, color)
            }
        })),
    }
}

/// Point d'entrée unique des modifications de structure, de propriétés et d'historique.
#[tauri::command]
pub async fn edit(state: State<'_, AppState>, doc_id: DocId, op: EditOp) -> Result<EditResult, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    let state = state.inner().clone();
    blocking(move || {
        let mut session = entry.write();
        // Copier et couper remplissent le presse-papiers AVANT toute modification du document.
        if let EditOp::Copy { id } | EditOp::Cut { id } = &op {
            state.set_clipboard(session.document().copy_pixels(*id)?);
        }
        let dirty = apply(&mut session, op, state.clipboard())?;
        Ok(EditResult { document: describe(doc_id, &entry, &session), dirty: dirty.into() })
    })
    .await
}

// -------------------------------------------------------------------- pinceau

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrushParams {
    size: f32,
    hardness: f32,
    opacity: f32,
    color: [u8; 3],
    erase: bool,
    pressure_size: bool,
    /// La pression du stylet module l'opacité.
    #[serde(default)]
    pressure_opacity: bool,
    /// Forme du pinceau ; absente = rond.
    #[serde(default)]
    kind: BrushKindParam,
    /// Dynamique de forme ; absente = comportement par défaut de la forme.
    #[serde(default)]
    dynamics: DynamicsParam,
    /// Le trait est esquissé à l'écran par la carte graphique : le moteur ne recompose le calque qu'à la fin.
    #[serde(default)]
    deferred: bool,
}

/// Voir `engine::paint::BrushDynamics`.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DynamicsParam {
    spacing: f32,
    angle: f32,
    follow_direction: bool,
    scatter: f32,
    size_jitter: f32,
    angle_jitter: f32,
    amount: f32,
    flow: f32,
    paint: f32,
    stretch: f32,
}

impl From<DynamicsParam> for BrushDynamics {
    fn from(d: DynamicsParam) -> Self {
        Self { spacing: d.spacing, angle: d.angle, follow_direction: d.follow_direction, scatter: d.scatter, size_jitter: d.size_jitter, angle_jitter: d.angle_jitter, amount: d.amount, flow: d.flow, paint: d.paint, stretch: d.stretch }
    }
}

/// Forme du pinceau vue de l'interface : `{"type":"tip","id":3}` désigne une pointe importée.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BrushKindParam {
    #[default]
    Round,
    Spray,
    Mixer,
    Tip { id: u32 },
}

impl From<BrushParams> for Brush {
    fn from(brush: BrushParams) -> Self {
        Self {
            size: brush.size,
            hardness: brush.hardness,
            opacity: brush.opacity,
            color: brush.color,
            mode: if brush.erase { BrushMode::Erase } else { BrushMode::Paint },
            pressure_size: brush.pressure_size,
            pressure_opacity: brush.pressure_opacity,
        }
    }
}

#[tauri::command]
pub async fn stroke_begin(
    state: State<'_, AppState>,
    doc_id: DocId,
    layer: LayerId,
    on_mask: bool,
    brush: BrushParams,
) -> Result<(), CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    let kind = match brush.kind {
        BrushKindParam::Round => BrushKind::Round,
        BrushKindParam::Spray => BrushKind::Spray,
        BrushKindParam::Mixer => BrushKind::Mixer,
        BrushKindParam::Tip { id } => BrushKind::Tip(state.brush(id).ok_or_else(|| EngineError::InvalidOperation("cette pointe de pinceau n'existe plus".into()))?),
    };
    let dynamics = brush.dynamics.into();
    let deferred = brush.deferred;
    blocking(move || Ok(entry.write().begin_stroke_with(layer, on_mask, brush.into(), kind, dynamics, deferred)?))
    .await
}

/// Filtre Verre : charge une image dont la clarté servira de relief. Retourne son nom et sa taille.
#[tauri::command]
pub async fn load_glass_texture(path: PathBuf) -> Result<(String, usize, usize), CommandError> {
    blocking(move || {
        let bytes = std::fs::read(&path).map_err(EngineError::io(&path))?;
        let map = engine::filters::HeightMap::from_image(&bytes)?;
        let described = (path.file_name().and_then(|n| n.to_str()).unwrap_or("texture").to_owned(), map.width, map.height);
        engine::filters::set_glass_texture(Some(map));
        Ok(described)
    })
    .await
}

/// Pointe de pinceau importée, telle que la liste l'interface.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrushTipInfo {
    id: u32,
    name: String,
    width: u32,
    height: u32,
    /// Espacement propre à la pointe, en fraction du diamètre.
    spacing: f32,
    /// Pointe fournie avec le logiciel (craie, fusain…), par opposition aux pointes importées.
    builtin: bool,
}

/// Les pointes intégrées occupent les premiers identifiants : ils sont stables d'un lancement à l'autre.
const BUILTIN_TIPS: u32 = 5;

fn tip_info((id, tip): &(u32, std::sync::Arc<engine::brushes::BrushTip>)) -> BrushTipInfo {
    BrushTipInfo { id: *id, name: tip.name.clone(), width: tip.width, height: tip.height, spacing: tip.spacing, builtin: *id <= BUILTIN_TIPS }
}

/// Dossier où sont recopiés les fichiers de pinceaux importés : ils sont rechargés à chaque lancement.
fn brushes_dir(app: &crate::rt::AppHandle) -> Result<PathBuf, CommandError> {
    use tauri::Manager;
    let dir = app.path().app_config_dir().map_err(|e| EngineError::InvalidOperation(e.to_string()))?.join("brushes");
    std::fs::create_dir_all(&dir).map_err(EngineError::io(&dir))?;
    Ok(dir)
}

/// Remplit la bibliothèque à son premier usage : pointes intégrées d'abord (identifiants stables),
/// puis fichiers importés lors des séances précédentes.
fn load_library(app: &crate::rt::AppHandle, state: &AppState) -> Result<(), CommandError> {
    if !state.brushes().is_empty() {
        return Ok(());
    }
    let builtin = engine::brushes::builtin_tips();
    debug_assert_eq!(builtin.len() as u32, BUILTIN_TIPS);
    state.add_brushes(builtin);
    let mut files: Vec<PathBuf> = std::fs::read_dir(brushes_dir(app)?).map(|dir| dir.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    for file in files {
        // Un fichier devenu illisible ne doit pas empêcher les autres de se charger.
        if let Ok(tips) = std::fs::read(&file).map_err(|_| ()).and_then(|bytes| engine::brushes::load_tips(&bytes, &file).map_err(|_| ())) {
            state.add_brushes(tips);
        }
    }
    Ok(())
}

/// Importe un fichier de pinceaux (.abr, .gbr ou image) : ses pointes rejoignent la bibliothèque, et
/// le fichier est recopié dans le dossier de l'application pour les retrouver au prochain lancement.
#[tauri::command]
pub async fn import_brushes(app: crate::rt::AppHandle, state: State<'_, AppState>, path: PathBuf) -> Result<Vec<BrushTipInfo>, CommandError> {
    let state = state.inner().clone();
    blocking(move || {
        let bytes = std::fs::read(&path).map_err(EngineError::io(&path))?;
        let tips = engine::brushes::load_tips(&bytes, &path)?;
        load_library(&app, &state)?; // les pointes intégrées et déjà importées gardent leurs identifiants
        if let Some(name) = path.file_name() {
            let copy = brushes_dir(&app)?.join(name);
            if copy != path {
                std::fs::write(&copy, &bytes).map_err(EngineError::io(&copy))?;
            }
        }
        Ok(state.add_brushes(tips).iter().map(tip_info).collect())
    })
    .await
}

/// Bibliothèque de pointes. Au premier appel, recharge les fichiers importés lors des séances précédentes.
#[tauri::command]
pub async fn list_brushes(app: crate::rt::AppHandle, state: State<'_, AppState>) -> Result<Vec<BrushTipInfo>, CommandError> {
    let state = state.inner().clone();
    blocking(move || {
        load_library(&app, &state)?;
        Ok(state.brushes().iter().map(tip_info).collect())
    })
    .await
}

/// Vignette PNG d'une pointe (64 px au plus), encre blanche sur transparent.
#[tauri::command]
pub async fn brush_thumbnail(state: State<'_, AppState>, id: u32) -> Result<Response, CommandError> {
    let tip = state.brush(id).ok_or_else(|| EngineError::InvalidOperation("pointe inconnue".into()))?;
    blocking(move || {
        let scale = 64.0 / tip.width.max(tip.height) as f32;
        let (w, h) = (((tip.width as f32 * scale).round() as u32).max(1), ((tip.height as f32 * scale).round() as u32).max(1));
        let image = image::RgbaImage::from_fn(w, h, |x, y| {
            let coverage = tip.sample((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            image::Rgba([255, 255, 255, (coverage * 255.0 + 0.5) as u8])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).map_err(|e| EngineError::Corrupt(e.to_string()))?;
        Ok(Response::new(out.into_inner()))
    })
    .await
}

/// Points du trait : `[x, y, pression]` en pixels du document.
#[tauri::command]
pub async fn stroke_points(
    state: State<'_, AppState>,
    doc_id: DocId,
    points: Vec<[f32; 3]>,
) -> Result<(u64, DirtyInfo), CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let points: Vec<StrokePoint> = points.iter().map(|p| StrokePoint { x: p[0], y: p[1], pressure: p[2] }).collect();
        let mut session = entry.write();
        let dirty = session.extend_stroke(&points)?;
        Ok((session.revision(), dirty.into()))
    })
    .await
}

#[tauri::command]
pub async fn stroke_end(state: State<'_, AppState>, doc_id: DocId) -> Result<EditResult, CommandError> {
    let entry = lookup(state.inner(), doc_id)?;
    blocking(move || {
        let mut session = entry.write();
        let dirty = session.finish_stroke();
        Ok(EditResult { document: describe(doc_id, &entry, &session), dirty: dirty.into() })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::selection::Shape;

    /// Rejoue ce que l'interface envoie : sélection d'une zone peinte puis glisser avec l'outil Déplacement.
    #[test]
    fn moving_a_selected_painted_area_through_the_command_layer() {
        let document = Document::with_background("x", 600, 400, 72.0, Background::White).expect("document");
        let mut session = Session::new(document, "Nouveau");
        let op = |json: &str| serde_json::from_str::<EditOp>(json).expect("opération lisible");

        apply(&mut session, op(r#"{"op":"addLayer","above":null}"#), None).expect("calque");
        let layer = session.document().walk_top_down()[0].1.id;
        apply(&mut session, op(&format!(r#"{{"op":"fill","id":{layer},"color":[255,0,0],"onMask":false}}"#)), None).expect("peinture");
        let select = r#"{"op":"select","shape":{"kind":"rect","x":10,"y":10,"width":50,"height":50},"mode":"replace"}"#;
        apply(&mut session, op(select), None).expect("sélection");

        for dx in [3, 40, 120] {
            let drag = format!(r#"{{"op":"movePixels","id":{layer},"dx":{dx},"dy":0,"gesture":"g1"}}"#);
            let dirty = apply(&mut session, op(&drag), None).expect("déplacement des pixels");
            assert!(!dirty.tiles.is_empty() || dirty.all, "l'affichage doit être prévenu");
        }
        let tile = session.renderer().tile(session.document(), 0, TileCoord::new(0, 0)).expect("tuile");
        assert_eq!(tile.pixel(20, 20), [255; 4], "trou : on voit le fond blanc");
        assert_eq!(tile.pixel(140, 20), [255, 0, 0, 255]);
        let _ = Shape::Rect { x: 0.0, y: 0.0, width: 1.0, height: 1.0 };
    }
}
