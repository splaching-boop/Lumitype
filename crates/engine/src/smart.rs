//! Objets dynamiques (les « smart objects » de Photoshop).
//!
//! Un calque objet dynamique ([`crate::document::LayerKind::Smart`]) contient un document entier, son **contenu** :
//! - **incorporé** : un document Lumitype complet, enregistré dans le projet ; s'il vient d'une image placée, le
//!   fichier d'origine est gardé tel quel ([`OriginalFile`]) et le document n'en est que le décodage ;
//! - **lié** : un fichier externe (image ou `.graf`), relu quand il change ([`SmartContent::reload`]). Le dernier
//!   document lu reste en mémoire et le dernier rendu dans le projet : un fichier introuvable n'efface rien, l'objet
//!   est seulement signalé « manquant ».
//!
//! # Rendu
//!
//! Le contenu est rendu une fois, aplati, à sa pleine résolution ([`SmartContent::flattened`]). Le calque garde une
//! matrice (sa transformation, comme celle d'un texte) et son `raster` n'est qu'un cache : l'aplat transformé.
//! Chaque transformation repart de l'aplat d'origine, jamais du rendu précédent : réduire puis ré-agrandir ne perd
//! rien. Pour les fortes réductions, l'aplat est d'abord divisé par deux autant de fois qu'il le faut (réductions
//! successives gardées en cache, [`SmartContent::level`]) : le rééchantillonnage final ne réduit jamais plus que de
//! moitié, sans crénelage.
//!
//! La transformation du calque se lit `T(position) · transformation` : la partie entière de la translation va dans
//! la position du calque ([`normalize`]). Une transformation réduite à une translation entière laisse donc le
//! `raster` égal à l'aplat lui-même, partagé, sans aucun rééchantillonnage : c'est ce qui rend la conversion en objet
//! dynamique exacte au pixel près, et le déplacement d'un objet dynamique instantané.
//!
//! # Instances
//!
//! Le contenu est partagé par `Arc` : dupliquer le calque (Ctrl+J) crée une **instance** qui partage le contenu et son
//! identifiant ([`ContentId`]). Modifier le contenu remplace le contenu de toutes les instances du document (elles
//! sont reconnues à leur identifiant). « Nouvel objet dynamique par copie » donne un nouvel identifiant.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use rayon::prelude::*;

use crate::document::Document;
use crate::error::EngineError;
use crate::render::TileRenderer;
use crate::tiles::{tiles_for, TileCoord, TileStore, TILE_SIZE};
use crate::transform::{transform_store, Matrix, Quality};

/// Identifiant d'un contenu d'objet dynamique, unique dans le processus. Les instances partagent le même.
/// Il n'est pas gardé tel quel dans les fichiers : un projet relu reçoit des identifiants neufs (les instances
/// restant instances), si bien qu'aucun identifiant ne peut en rencontrer un autre venu d'ailleurs.
pub type ContentId = u64;

static NEXT_CONTENT: AtomicU64 = AtomicU64::new(1);

/// Nouvel identifiant de contenu.
pub fn new_content_id() -> ContentId {
    NEXT_CONTENT.fetch_add(1, Ordering::Relaxed)
}

/// Message des outils de peinture, des remplissages et des filtres sur un objet dynamique.
pub const NOT_PAINTABLE: &str = "Objet dynamique : pixellisez-le ou modifiez son contenu";

/// Fichier image d'origine d'un contenu incorporé par « Placer » : gardé à l'octet près, pour l'exporter tel quel.
#[derive(Clone, Debug)]
pub struct OriginalFile {
    /// Nom du fichier, extension comprise.
    pub name: String,
    pub bytes: Arc<Vec<u8>>,
}

/// D'où vient le contenu.
#[derive(Clone, Debug)]
pub enum SmartSource {
    /// Enregistré dans le projet. `original` : image placée dont le document est le décodage, tant que le contenu
    /// n'a pas été modifié (« Modifier le contenu » en fait un document ordinaire).
    Embedded { original: Option<OriginalFile> },
    /// Fichier externe (chemin absolu). `modified` : date de modification du fichier quand il a été lu ;
    /// `missing` : introuvable à la dernière vérification (le dernier document lu sert toujours).
    Linked { path: PathBuf, modified: Option<SystemTime>, missing: bool },
}

/// Contenu d'un objet dynamique : un document, et son rendu aplati à pleine résolution.
#[derive(Debug)]
pub struct SmartContent {
    pub id: ContentId,
    /// Nom du contenu (fichier placé, ou calque converti) : c'est aussi celui du document ouvert par « Modifier le contenu ».
    pub name: String,
    pub source: SmartSource,
    /// Le document contenu. Partagé : l'ouvrir pour le modifier ne copie aucun pixel.
    pub document: Arc<Document>,
    /// Rendu aplati du document, sur sa zone de travail `[0, largeur) x [0, hauteur)` : la source de tout rendu
    /// du calque.
    pub flattened: Arc<TileStore>,
    /// Réductions successives de l'aplat par deux (indice 0 : moitié), calculées à la demande.
    mips: Mutex<Vec<Arc<TileStore>>>,
}

impl SmartContent {
    /// Contenu d'un document : l'aplat est calculé ici.
    pub fn new(id: ContentId, name: impl Into<String>, document: Arc<Document>, source: SmartSource) -> Self {
        let flattened = Arc::new(flatten(&document));
        Self::with_flattened(id, name, document, source, flattened)
    }

    /// Contenu dont l'aplat est déjà connu (lien manquant relu depuis un projet, copie d'un contenu).
    pub fn with_flattened(id: ContentId, name: impl Into<String>, document: Arc<Document>, source: SmartSource, flattened: Arc<TileStore>) -> Self {
        Self { id, name: name.into(), source, document, flattened, mips: Mutex::new(Vec::new()) }
    }

    pub fn width(&self) -> u32 {
        self.document.width()
    }

    pub fn height(&self) -> u32 {
        self.document.height()
    }

    pub fn is_linked(&self) -> bool {
        matches!(self.source, SmartSource::Linked { .. })
    }

    /// Fichier lié, s'il y en a un.
    pub fn path(&self) -> Option<&Path> {
        match &self.source {
            SmartSource::Linked { path, .. } => Some(path),
            SmartSource::Embedded { .. } => None,
        }
    }

    /// Lien introuvable à la dernière vérification.
    pub fn missing(&self) -> bool {
        matches!(self.source, SmartSource::Linked { missing: true, .. })
    }

    /// Image d'origine d'un contenu incorporé par « Placer », tant qu'il n'a pas été modifié.
    pub fn original(&self) -> Option<&OriginalFile> {
        match &self.source {
            SmartSource::Embedded { original } => original.as_ref(),
            SmartSource::Linked { .. } => None,
        }
    }

    /// Le même contenu sous un nouvel identifiant (« Nouvel objet dynamique par copie ») : document et aplat
    /// restent partagés tant que l'un des deux n'est pas modifié, mais les deux objets sont désormais indépendants.
    pub fn copy_with_new_id(&self) -> Self {
        Self::with_flattened(new_content_id(), self.name.clone(), Arc::clone(&self.document), self.source.clone(), Arc::clone(&self.flattened))
    }

    /// Contenu modifié (« Modifier le contenu » puis enregistrement) : même identifiant, donc mêmes instances. Une
    /// image d'origine n'a plus cours : le contenu est désormais ce document. Un lien garde son chemin ; sa date est
    /// relue (le document vient d'être enregistré dans le fichier lié, ou le sera).
    pub fn edited(&self, document: Document) -> Self {
        let source = match &self.source {
            SmartSource::Embedded { .. } => SmartSource::Embedded { original: None },
            SmartSource::Linked { path, .. } => SmartSource::Linked { path: path.clone(), modified: modified_time(path), missing: false },
        };
        Self::new(self.id, self.name.clone(), Arc::new(document), source)
    }

    /// Contenu lié devenu incorporé (« Incorporer le lien ») : même identifiant, même document.
    pub fn embedded(&self) -> Self {
        Self::with_flattened(self.id, self.name.clone(), Arc::clone(&self.document), SmartSource::Embedded { original: None }, Arc::clone(&self.flattened))
    }

    /// Lien vérifié : `None` si rien n'a changé. Sinon le contenu à jour, sous le même identifiant : relu (fichier
    /// modifié, ou réapparu), ou marqué manquant (le dernier document lu reste affiché). Une lecture qui échoue
    /// (fichier devenu illisible) est une erreur ; le contenu actuel reste alors en place.
    pub fn reload(&self) -> Option<Result<Self, EngineError>> {
        let SmartSource::Linked { path, modified, missing } = &self.source else { return None };
        if !path.is_file() {
            if *missing {
                return None;
            }
            let source = SmartSource::Linked { path: path.clone(), modified: *modified, missing: true };
            return Some(Ok(Self::with_flattened(self.id, self.name.clone(), Arc::clone(&self.document), source, Arc::clone(&self.flattened))));
        }
        let now = modified_time(path);
        if !*missing && now == *modified {
            return None;
        }
        Some(read_file(path).map(|(document, _)| Self::new(self.id, self.name.clone(), Arc::new(document), SmartSource::Linked { path: path.clone(), modified: now, missing: false })))
    }

    /// Aplat réduit `level` fois de moitié (0 : l'aplat lui-même). Chaque niveau est calculé une fois, depuis le
    /// précédent, par moyenne exacte de quatre pixels.
    pub fn level(&self, level: u8) -> Result<Arc<TileStore>, EngineError> {
        if level == 0 {
            return Ok(Arc::clone(&self.flattened));
        }
        let mut mips = self.mips.lock().unwrap_or_else(PoisonError::into_inner);
        while mips.len() < usize::from(level) {
            let previous = mips.last().cloned().unwrap_or_else(|| Arc::clone(&self.flattened));
            mips.push(Arc::new(transform_store(&previous, &Matrix::scaling(0.5), Quality::Final)?));
        }
        Ok(Arc::clone(&mips[usize::from(level) - 1]))
    }

    /// Réductions déjà calculées (mémoire occupée par le contenu, voir le module `memory`).
    pub(crate) fn cached_levels(&self) -> Vec<Arc<TileStore>> {
        self.mips.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Rendu du contenu par `local` (repère contenu -> repère du calque). L'identité rend l'aplat lui-même, partagé.
    /// Une réduction passe d'abord par la réduction par deux la plus proche (voir [`Self::level`]).
    pub fn render(&self, local: &Matrix, quality: Quality) -> Result<Arc<TileStore>, EngineError> {
        if *local == Matrix::IDENTITY {
            return Ok(Arc::clone(&self.flattened));
        }
        let scale = local.mean_scale();
        let mut level = 0u8;
        let mut reach = scale;
        // Plus petite réduction par deux qui laisse au rééchantillonnage une réduction d'au plus moitié.
        while reach > 0.0 && reach * 2.0 <= 1.0 + 1e-9 && level < 16 && (self.width().max(self.height()) >> (level + 1)) >= 1 {
            level += 1;
            reach *= 2.0;
        }
        let source = self.level(level)?;
        let matrix = local.then_after(&Matrix::scaling(f64::from(1u32 << level)));
        Ok(Arc::new(transform_store(&source, &matrix, quality)?))
    }
}

/// Rendu aplati d'un document sur sa zone de travail (ce qui déborde est coupé, comme dans Photoshop), composé en
/// parallèle.
pub fn flatten(doc: &Document) -> TileStore {
    let renderer = TileRenderer::new();
    let (width, height) = (doc.width() as i32, doc.height() as i32);
    let coords: Vec<TileCoord> = (0..tiles_for(doc.height()) as i32).flat_map(|ty| (0..tiles_for(doc.width()) as i32).map(move |tx| TileCoord::new(tx, ty))).collect();
    let tiles: Vec<_> = coords.into_par_iter().map(|coord| (coord, renderer.tile(doc, 0, coord))).collect();
    let mut flat = TileStore::new();
    let size = TILE_SIZE as i32;
    for (coord, tile) in tiles {
        let Some(tile) = tile else { continue };
        let (ox, oy) = coord.origin();
        if ox + size <= width && oy + size <= height {
            flat.restore(coord, Some(tile));
            continue;
        }
        // Tuile du bord : ce qui dépasse de la zone de travail est effacé.
        let mut cut = (*tile).clone();
        for y in 0..size {
            for x in 0..size {
                if ox + x >= width || oy + y >= height {
                    cut.set_pixel(x as u32, y as u32, [0; 4]);
                }
            }
        }
        flat.insert(coord, cut);
    }
    flat
}

/// Arrondit ce qui n'est que bruit de calcul : une échelle de 0,999 999 999 9 est 1, une translation de
/// 2,000 000 1 est 2. Sans cela, réduire puis ré-agrandir laisserait un rééchantillonnage pour rien.
fn snap(value: f64, epsilon: f64) -> f64 {
    let rounded = value.round();
    if (value - rounded).abs() < epsilon {
        rounded
    } else {
        value
    }
}

/// Sépare une transformation complète (repère contenu -> document) en position du calque (partie entière de la
/// translation) et transformation locale, débarrassée du bruit de calcul. Une perspective garde tout dans sa
/// matrice (position nulle).
pub fn normalize(full: &Matrix) -> ((i32, i32), Matrix) {
    let mut m = full.0;
    if m[8].abs() > 1e-12 && m[8] != 1.0 {
        let w = m[8];
        m.iter_mut().for_each(|v| *v /= w);
    }
    for i in [0, 1, 3, 4] {
        m[i] = snap(m[i], 1e-9);
    }
    for i in [6, 7] {
        if m[i].abs() < 1e-12 {
            m[i] = 0.0;
        }
    }
    m[8] = snap(m[8], 1e-12);
    for i in [2, 5] {
        m[i] = snap(m[i], 1e-6);
    }
    if m[6] != 0.0 || m[7] != 0.0 {
        return ((0, 0), Matrix(m));
    }
    let limit = 1e8;
    let (ox, oy) = (m[2].floor().clamp(-limit, limit), m[5].floor().clamp(-limit, limit));
    m[2] -= ox;
    m[5] -= oy;
    ((ox as i32, oy as i32), Matrix(m))
}

/// Géométrie d'un objet dynamique telle qu'on l'affiche (barre d'options de la transformation) : échelles et angle
/// lus sur la partie linéaire de sa transformation complète.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmartGeometry {
    /// Échelles horizontale et verticale (1 = 100 %) ; négative : retourné.
    pub scale_x: f64,
    pub scale_y: f64,
    /// Rotation en degrés, sens horaire à l'écran.
    pub rotation_deg: f64,
    /// Coin haut gauche du contenu dans le document.
    pub x: f64,
    pub y: f64,
    /// Les quatre coins du contenu dans le document : haut gauche, haut droit, bas droit, bas gauche.
    pub corners: [(f64, f64); 4],
}

impl SmartGeometry {
    pub fn of(full: &Matrix, width: u32, height: u32) -> Self {
        let m = full.0;
        let w = if m[8].abs() > 1e-12 { m[8] } else { 1.0 };
        let (a, b, d, e) = (m[0] / w, m[1] / w, m[3] / w, m[4] / w);
        let scale_x = a.hypot(d);
        let scale_y = if scale_x > 1e-12 { (a * e - b * d) / scale_x } else { 0.0 };
        let (w, h) = (f64::from(width), f64::from(height));
        let at = |x: f64, y: f64| full.apply(x, y).unwrap_or((x, y));
        let (x, y) = at(0.0, 0.0);
        Self { scale_x, scale_y, rotation_deg: d.atan2(a).to_degrees(), x, y, corners: [at(0.0, 0.0), at(w, 0.0), at(w, h), at(0.0, h)] }
    }
}

/// Date de modification d'un fichier, ou rien s'il est illisible.
pub fn modified_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

thread_local! {
    /// Profondeur des lectures de fichiers liés imbriquées : un projet lié à lui-même (directement ou non) ne doit
    /// pas boucler sans fin.
    static LINK_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Au-delà, une chaîne de liens est tenue pour une boucle.
const MAX_LINK_DEPTH: u32 = 8;

/// Lit un fichier à placer : un projet (`.graf`) devient son document ; une image, un document d'un seul calque
/// décodé par les décodeurs habituels (PNG, JPEG, WebP, TIFF, HEIC, DNG…), avec ses octets d'origine.
pub fn read_file(path: &Path) -> Result<(Document, Option<OriginalFile>), EngineError> {
    let depth = LINK_DEPTH.with(Cell::get);
    if depth >= MAX_LINK_DEPTH {
        return Err(EngineError::InvalidOperation(format!("objets dynamiques liés en boucle : {}", path.display())));
    }
    LINK_DEPTH.with(|d| d.set(depth + 1));
    let result = (|| {
        if crate::io::is_project_path(path) {
            return Ok((crate::io::load_project(path)?, None));
        }
        let bytes = std::fs::read(path).map_err(EngineError::io(path))?;
        read_bytes(bytes, path)
    })();
    LINK_DEPTH.with(|d| d.set(depth));
    result
}

/// Comme [`read_file`], depuis la mémoire (fichier déposé dans la fenêtre). `origin` donne le nom et le format.
pub fn read_bytes(bytes: Vec<u8>, origin: &Path) -> Result<(Document, Option<OriginalFile>), EngineError> {
    if crate::io::is_project_path(origin) {
        return Ok((crate::io::load_project_bytes(&bytes, origin)?, None));
    }
    let document = crate::io::import_image_bytes(&bytes, origin)?;
    let name = origin.file_name().and_then(|n| n.to_str()).unwrap_or("image").to_owned();
    Ok((document, Some(OriginalFile { name, bytes: Arc::new(bytes) })))
}

/// Nom d'un contenu placé depuis un fichier : le nom du fichier sans extension.
pub fn name_of(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("Objet dynamique").to_owned()
}

/// Contenu lu depuis un fichier, incorporé ou lié (`linked` : le chemin est rendu absolu).
pub fn content_from_file(path: &Path, linked: bool) -> Result<SmartContent, EngineError> {
    let (document, original) = read_file(path)?;
    let name = name_of(path);
    let source = if linked {
        let path = std::path::absolute(path).map_err(EngineError::io(path))?;
        SmartSource::Linked { modified: modified_time(&path), path, missing: false }
    } else {
        SmartSource::Embedded { original }
    };
    Ok(SmartContent::new(new_content_id(), name, Arc::new(document), source))
}

/// Contenu incorporé depuis la mémoire (fichier déposé).
pub fn content_from_bytes(bytes: Vec<u8>, origin: &Path) -> Result<SmartContent, EngineError> {
    let (document, original) = read_bytes(bytes, origin)?;
    Ok(SmartContent::new(new_content_id(), name_of(origin), Arc::new(document), SmartSource::Embedded { original }))
}

/// Chemin de `path` relatif au dossier `base` (`..` compris), ou rien si les deux n'ont pas de racine commune
/// (disques différents sous Windows).
pub fn relative_to(path: &Path, base: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let path: Vec<Component> = path.components().collect();
    let base: Vec<Component> = base.components().collect();
    let common = path.iter().zip(&base).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return None;
    }
    let mut out = PathBuf::new();
    for _ in common..base.len() {
        out.push("..");
    }
    for part in &path[common..] {
        out.push(part.as_os_str());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_moves_the_integer_translation_into_the_position() {
        let ((x, y), local) = normalize(&Matrix::translation(12.0, -7.0));
        assert_eq!(((x, y), local), ((12, -7), Matrix::IDENTITY));
        // Bruit de calcul d'un agrandissement suivi de la réduction inverse : identité exacte.
        let noisy = Matrix([1.000_000_000_000_1, 1e-17, 30.000_000_01, -1e-16, 0.999_999_999_999_9, 4.999_999_999, 0.0, 0.0, 1.0]);
        assert_eq!(normalize(&noisy), ((30, 5), Matrix::IDENTITY));
        let ((x, _), local) = normalize(&Matrix([2.0, 0.0, 10.5, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0]));
        assert_eq!((x, local.0[2]), (10, 0.5));
    }

    #[test]
    fn relative_paths_climb_out_of_the_project_folder() {
        assert_eq!(relative_to(Path::new("/a/b/img/x.png"), Path::new("/a/b")), Some(PathBuf::from("img/x.png")));
        assert_eq!(relative_to(Path::new("/a/c/x.png"), Path::new("/a/b")), Some(PathBuf::from("../c/x.png")));
    }

    #[test]
    fn geometry_reads_scale_and_rotation() {
        let r = std::f64::consts::FRAC_PI_6;
        let m = Matrix([2.0 * r.cos(), -3.0 * r.sin(), 5.0, 2.0 * r.sin(), 3.0 * r.cos(), 6.0, 0.0, 0.0, 1.0]);
        let g = SmartGeometry::of(&m, 10, 10);
        assert!((g.scale_x - 2.0).abs() < 1e-9 && (g.scale_y - 3.0).abs() < 1e-9 && (g.rotation_deg - 30.0).abs() < 1e-9);
        assert_eq!((g.x, g.y), (5.0, 6.0));
    }

    // ------------------------------------------------------------ objets dynamiques dans un document

    use crate::document::{Background, LayerKind, SmartFit};
    use crate::pixel::premultiply;
    use crate::session::Session;

    /// Motif varié (couleurs, transparences) : un rééchantillonnage, même d'un demi-pixel, s'y verrait.
    fn pattern(width: u32, height: u32) -> TileStore {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let alpha = if (x / 10 + y / 10) % 3 == 0 { 128 } else { 255 };
                rgba.extend_from_slice(&premultiply([(x * 7 % 256) as u8, (y * 5 % 256) as u8, ((x + y) * 3 % 256) as u8, alpha]));
            }
        }
        TileStore::from_premultiplied(width, height, &rgba).expect("motif")
    }

    fn document() -> Document {
        Document::with_background("x", 400, 300, 72.0, Background::White).expect("document")
    }

    /// Calque de motif `w x h` placé en `(x, y)`.
    fn add_pattern(doc: &mut Document, name: &str, (x, y): (i32, i32), (w, h): (u32, u32)) -> crate::document::LayerId {
        let index = doc.children_of(None).expect("racine").len();
        let id = doc.add_pixel_layer(name, pattern(w, h), None, index).expect("calque");
        doc.translate_layers(&[id], x, y).expect("position");
        id
    }

    /// Image composée du document, RGBA prémultiplié.
    fn picture(doc: &Document) -> Vec<u8> {
        flatten(doc).to_premultiplied(doc.width(), doc.height())
    }

    fn content_of(doc: &Document, id: crate::document::LayerId) -> Arc<SmartContent> {
        Arc::clone(doc.layer(id).expect("calque").smart().expect("objet dynamique"))
    }

    fn raster_of(doc: &Document, id: crate::document::LayerId) -> Arc<TileStore> {
        Arc::clone(doc.layer(id).expect("calque").pixels().expect("rendu"))
    }

    fn about(center: (f64, f64), m: Matrix) -> Matrix {
        Matrix::translation(center.0, center.1).then_after(&m).then_after(&Matrix::translation(-center.0, -center.1))
    }

    fn rotation(degrees: f64) -> Matrix {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Matrix([cos, -sin, 0.0, sin, cos, 0.0, 0.0, 0.0, 1.0])
    }

    /// PNG uni de `w x h`, écrit dans `path`.
    fn write_png(path: &Path, (w, h): (u32, u32), color: [u8; 4]) {
        let mut doc = Document::new("png", w, h, 72.0).expect("document");
        let mut pixels = TileStore::new();
        pixels.fill(w, h, color);
        doc.add_pixel_layer("a", pixels, None, 0).expect("calque");
        crate::io::export_png(&doc, path).expect("png");
    }

    #[test]
    fn converting_one_layer_keeps_the_picture_identical_and_shares_its_render() {
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (37, 51), (120, 90));
        let before = picture(&doc);
        let smart = doc.convert_to_smart(&[id]).expect("conversion");
        assert!(doc.layer(id).is_err(), "le calque d'origine est devenu le contenu");
        let layer = doc.layer(smart).expect("objet");
        assert_eq!((layer.props.name.as_str(), layer.offset), ("Motif", (37, 51)), "nom et position repris");
        let content = content_of(&doc, smart);
        assert_eq!((content.width(), content.height()), (120, 90), "document de la taille de l'emprise");
        assert!(Arc::ptr_eq(&raster_of(&doc, smart), &content.flattened), "transformation identité : l'aplat lui-même");
        assert_eq!(picture(&doc), before, "rendu identique au pixel près");
        assert_eq!(doc.children_of(None).expect("racine").len(), 2, "il a pris la place du calque");
    }

    #[test]
    fn converting_several_layers_keeps_groups_masks_effects_and_the_picture() {
        let mut doc = document();
        let a = add_pattern(&mut doc, "A", (-30, 20), (100, 80)); // déborde du cadre : gardé dans le contenu
        let group = doc.add_group("G", None, 2).expect("groupe");
        let b = doc.add_pixel_layer("B", pattern(60, 60), Some(group), 0).expect("calque");
        doc.translate_layers(&[b], 200, 150).expect("position");
        doc.add_mask(b, true).expect("masque");
        doc.mask_mut(b).expect("masque").get_mut(TileCoord::new(0, 0)).set_pixel(5, 5, [0]); // un point masqué en (205, 155)
        let glow = crate::effects::LayerEffect::OuterGlow { enabled: true, color: [255, 0, 0], opacity: 0.8, size: 6.0, blend: crate::blend::BlendMode::Normal };
        let c = add_pattern(&mut doc, "C", (300, 10), (50, 50));
        doc.set_effects(c, vec![glow]).expect("effets");
        let before = picture(&doc);
        let smart = doc.convert_to_smart(&[a, group, c]).expect("conversion");
        assert_eq!(doc.layer(smart).expect("objet").props.name, "C", "nom du plus haut");
        let content = content_of(&doc, smart);
        let inner: Vec<String> = content.document.walk_top_down().iter().map(|(d, l)| format!("{d}{}", l.props.name)).collect();
        assert_eq!(inner, ["0C", "0G", "1B", "0A"], "groupes et ordre conservés");
        let kept = |name: &str| content.document.walk_top_down().into_iter().find(|(_, l)| l.props.name == name).map(|(_, l)| (l.mask.is_some(), l.effects.len()));
        assert_eq!((kept("B"), kept("C")), (Some((true, 0)), Some((false, 1))), "masque et effets restent dans le contenu");
        // Emprise : de A (-30) jusqu'au halo de C (350 + marge).
        let (x, _) = doc.layer(smart).expect("objet").offset;
        assert_eq!(x, -30);
        assert!(content.width() > 380, "le halo de C est compris dans l'emprise");
        assert_eq!(picture(&doc), before, "rendu identique au pixel près");
    }

    #[test]
    fn scaling_up_then_back_to_100_percent_loses_nothing() {
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (40, 30), (128, 96));
        let original = picture(&doc);
        let smart = doc.convert_to_smart(&[id]).expect("conversion");
        let center = (104.0, 78.0);
        doc.transform_layers(&[smart], &about(center, Matrix::scaling(1.25)), Quality::Final).expect("agrandir");
        assert_ne!(picture(&doc), original, "agrandi");
        let geometry = doc.smart_geometry(smart).expect("géométrie");
        assert!((geometry.scale_x - 1.25).abs() < 1e-9 && (geometry.scale_y - 1.25).abs() < 1e-9);
        // Réduction très forte puis retour : le rendu repart toujours de l'original.
        doc.transform_layers(&[smart], &about(center, Matrix::scaling(0.08)), Quality::Final).expect("réduire");
        doc.transform_layers(&[smart], &about(center, Matrix::scaling(1.0 / 0.08 * 0.8)), Quality::Final).expect("ré-agrandir");
        doc.transform_layers(&[smart], &about(center, Matrix::scaling(1.0)), Quality::Final).expect("identité");
        let geometry = doc.smart_geometry(smart).expect("géométrie");
        assert!((geometry.scale_x - 1.0).abs() < 1e-9, "retour à 100 % : {}", geometry.scale_x);
        assert_eq!(picture(&doc), original, "pixels identiques à l'original");
        assert!(Arc::ptr_eq(&raster_of(&doc, smart), &content_of(&doc, smart).flattened));
    }

    #[test]
    fn a_rotation_is_parametric_and_reset_brings_back_the_original() {
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (100, 60), (120, 80));
        let original = picture(&doc);
        let smart = doc.convert_to_smart(&[id]).expect("conversion");
        doc.transform_layers(&[smart], &about((160.0, 100.0), rotation(30.0)), Quality::Final).expect("rotation");
        let geometry = doc.smart_geometry(smart).expect("géométrie");
        assert!((geometry.rotation_deg - 30.0).abs() < 1e-9 && (geometry.scale_x - 1.0).abs() < 1e-9);
        assert!(matches!(doc.layer(smart).expect("objet").kind, LayerKind::Smart { .. }), "jamais pixellisé");
        doc.transform_layers(&[smart], &about((160.0, 100.0), Matrix::scaling(0.5)), Quality::Final).expect("réduction");
        doc.reset_smart_transform(smart).expect("réinitialiser");
        let geometry = doc.smart_geometry(smart).expect("géométrie");
        assert_eq!((geometry.scale_x, geometry.rotation_deg, geometry.x, geometry.y), (1.0, 0.0, 100.0, 60.0), "100 %, droit, même centre");
        assert_eq!(picture(&doc), original);
    }

    #[test]
    fn a_strong_reduction_goes_through_the_halved_levels() {
        let mut doc = Document::with_background("x", 600, 600, 72.0, Background::Transparent).expect("document");
        // Damier d'un pixel : sans pré-réduction, une réduction au dixième le crénelerait (noir ou blanc selon l'endroit).
        let mut rgba = Vec::new();
        for y in 0..512u32 {
            for x in 0..512u32 {
                rgba.extend_from_slice(&if (x + y) % 2 == 0 { [255, 255, 255, 255] } else { [0, 0, 0, 255] });
            }
        }
        let id = doc.add_pixel_layer("damier", TileStore::from_premultiplied(512, 512, &rgba).expect("damier"), None, 1).expect("calque");
        let smart = doc.convert_to_smart(&[id]).expect("conversion");
        doc.transform_layers(&[smart], &Matrix::scaling(0.1), Quality::Final).expect("réduction");
        assert!(content_of(&doc, smart).cached_levels().len() >= 3, "réductions par deux calculées et gardées");
        let raster = raster_of(&doc, smart);
        let mut sampler = crate::tiles::Sampler::new(&raster);
        for (x, y) in [(10, 10), (25, 30), (40, 12)] {
            let px = sampler.get(x, y);
            assert!((110..=145).contains(&px[0]) && px[3] == 255, "gris moyen attendu, pas du crénelage : {px:?}");
        }
    }

    #[test]
    fn instances_share_their_content_and_a_copy_does_not() {
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (10, 10), (100, 100));
        let first = doc.convert_to_smart(&[id]).expect("conversion");
        let second = doc.duplicate_layer(first).expect("instance");
        doc.translate_layers(&[second], 150, 0).expect("décalage");
        let copy = doc.new_smart_by_copy(first).expect("copie");
        let shared = content_of(&doc, first);
        assert!(Arc::ptr_eq(&shared, &content_of(&doc, second)), "Ctrl+J : même contenu");
        assert_ne!(content_of(&doc, copy).id, shared.id, "copie : contenu séparé");
        assert_eq!(doc.smart_instances(shared.id), vec![second, first]);
        // Modifier le contenu (comme à l'enregistrement de « Modifier le contenu ») : toutes les instances suivent.
        let mut edited = (*shared.document).clone();
        let inner = edited.walk_top_down()[0].1.id;
        edited.pixels_mut(inner).expect("pixels").fill(100, 100, [0, 0, 255, 255]);
        let copy_raster = raster_of(&doc, copy);
        assert_eq!(doc.update_smart_content(Arc::new(shared.edited(edited)), SmartFit::Centre).expect("mise à jour"), 2);
        for instance in [first, second] {
            assert_eq!(crate::tiles::Sampler::new(&raster_of(&doc, instance)).get(50, 50), [0, 0, 255, 255], "instance mise à jour");
            assert_eq!(content_of(&doc, instance).id, shared.id, "toujours instances l'une de l'autre");
        }
        assert!(Arc::ptr_eq(&copy_raster, &raster_of(&doc, copy)), "la copie n'a pas bougé");
    }

    #[test]
    fn a_resized_content_keeps_its_centre_or_its_displayed_size() {
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (100, 100), (100, 50));
        let smart = doc.convert_to_smart(&[id]).expect("conversion");
        // Remplacé par un contenu 200 x 200 : il tient dans l'ancienne place (100 x 50), même centre (150, 125).
        let mut other = Document::new("autre", 200, 200, 72.0).expect("document");
        other.add_pixel_layer("p", pattern(200, 200), None, 0).expect("calque");
        let replacement = SmartContent::new(new_content_id(), "autre", Arc::new(other), SmartSource::Embedded { original: None });
        let old_id = content_of(&doc, smart).id;
        doc.replace_smart_content(smart, replacement).expect("remplacer");
        let content = content_of(&doc, smart);
        assert_eq!((content.id, content.name.as_str()), (old_id, "autre"), "même identifiant : les instances suivent");
        let geometry = doc.smart_geometry(smart).expect("géométrie");
        assert_eq!((geometry.scale_x, geometry.corners[0], geometry.corners[2]), (0.25, (125.0, 100.0), (175.0, 150.0)));
    }

    #[test]
    fn rasterizing_keeps_the_current_render() {
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (10, 10), (100, 100));
        let smart = doc.convert_to_smart(&[id]).expect("conversion");
        doc.transform_layers(&[smart], &rotation(10.0), Quality::Final).expect("rotation");
        let render = raster_of(&doc, smart);
        doc.rasterize_layer(smart).expect("pixelliser");
        let layer = doc.layer(smart).expect("calque");
        assert!(matches!(layer.kind, LayerKind::Pixel { .. }) && Arc::ptr_eq(layer.pixels().expect("pixels"), &render));
    }

    #[test]
    fn painting_filling_and_filtering_are_refused_but_the_mask_can_be_painted() {
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (10, 10), (100, 100));
        let smart = doc.convert_to_smart(&[id]).expect("conversion");
        let refused = |result: Result<(), EngineError>| matches!(result, Err(EngineError::InvalidOperation(m)) if m == NOT_PAINTABLE);
        let brush = crate::paint::Brush { size: 10.0, hardness: 1.0, opacity: 1.0, color: [0, 0, 0], mode: crate::paint::BrushMode::Paint, pressure_size: false, pressure_opacity: false };
        assert!(refused(crate::paint::Stroke::begin(&doc, smart, false, brush).map(|_| ())), "pinceau");
        assert!(refused(doc.fill_layer(smart, [1, 2, 3])), "remplir");
        assert!(refused(doc.invert_layer(smart)), "inverser");
        assert!(refused(doc.apply_filter(smart, &crate::filters::Filter::GaussianBlur { radius: 2.0 })), "filtre");
        assert!(refused(doc.filter_preview(smart, &crate::filters::Filter::GaussianBlur { radius: 2.0 }, 64).map(|_| ())), "aperçu de filtre");
        assert!(refused(doc.pixels_mut(smart).map(|_| ())));
        doc.add_mask(smart, true).expect("masque");
        assert!(crate::paint::Stroke::begin(&doc, smart, true, brush).is_ok(), "le masque se peint");
        doc.set_selection(Some(crate::selection::select_all()));
        assert!(doc.transform_layers(&[smart], &Matrix::scaling(2.0), Quality::Final).is_err(), "une sélection ne découpe pas un objet dynamique");
    }

    #[test]
    fn the_mask_follows_ctrl_t_and_stays_in_place_when_the_transform_is_reset() {
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (10, 10), (100, 100));
        let smart = doc.convert_to_smart(&[id]).expect("conversion");
        doc.add_mask(smart, true).expect("masque");
        // Carré masqué de (25, 25) à (35, 35) dans le document.
        for y in 15..25 {
            for x in 15..25 {
                doc.mask_mut(smart).expect("masque").get_mut(TileCoord::new(0, 0)).set_pixel(x, y, [0]);
            }
        }
        let masked = |doc: &Document, x: i32, y: i32| {
            let layer = doc.layer(smart).expect("objet");
            crate::tiles::Sampler::new(&layer.mask.as_ref().expect("masque").pixels).get(x - layer.offset.0, y - layer.offset.1)[0] < 128
        };
        // Ctrl+T : le masque est transformé avec l'objet (réduit de moitié autour du coin : le carré va en (20, 20)).
        doc.transform_layers(&[smart], &about((10.0, 10.0), Matrix::scaling(0.5)), Quality::Final).expect("réduction");
        assert!(masked(&doc, 20, 20) && !masked(&doc, 30, 30));
        // Réinitialiser déplace l'objet (100 %, centré en (35, 35)) mais pas le masque, qui reste où il est dans le document.
        doc.reset_smart_transform(smart).expect("réinitialiser");
        assert_eq!(doc.layer(smart).expect("objet").offset, (-15, -15));
        assert!(masked(&doc, 20, 20) && !masked(&doc, 30, 30));
    }

    #[test]
    fn pick_layer_sees_the_rendered_object() {
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (50, 50), (100, 100));
        let smart = doc.convert_to_smart(&[id]).expect("conversion");
        assert_eq!(doc.pick_layer(60, 60), Some(smart));
        doc.transform_layers(&[smart], &Matrix::translation(200.0, 0.0), Quality::Final).expect("déplacement");
        assert_ne!(doc.pick_layer(60, 60), Some(smart));
        assert_eq!(doc.pick_layer(260, 60), Some(smart));
    }

    #[test]
    fn a_shared_content_counts_once_in_memory() {
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (0, 0), (300, 300));
        let smart = doc.convert_to_smart(&[id]).expect("conversion");
        let once = crate::memory::document_tile_bytes(&doc, &mut std::collections::HashSet::new());
        doc.duplicate_layer(smart).expect("instance");
        doc.duplicate_layer(smart).expect("instance");
        assert_eq!(crate::memory::document_tile_bytes(&doc, &mut std::collections::HashSet::new()), once, "les instances ne coûtent rien");
        let content = content_of(&doc, smart);
        assert!(once >= content.flattened.tile_count() as u64 * crate::tiles::Tile::BYTES, "le contenu est compté");
    }

    #[test]
    fn every_operation_is_undone_and_redone() {
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (20, 20), (100, 100));
        let mut session = Session::new(doc, "Ouvrir");
        let initial = picture(session.document());
        let (smart, _) = session.apply("Convertir en objet dynamique", None, |d| d.convert_to_smart(&[id])).expect("conversion");
        session.undo();
        assert!(session.document().layer(id).is_ok() && session.document().layer(smart).is_err(), "conversion annulée");
        session.redo();
        assert!(session.document().layer(smart).is_ok());
        let steps: Vec<(&str, Box<dyn Fn(&mut Document) -> Result<(), EngineError>>)> = vec![
            ("Transformation", Box::new(move |d| d.transform_layers(&[smart], &rotation(20.0), Quality::Final))),
            ("Réinitialiser", Box::new(move |d| d.reset_smart_transform(smart))),
            ("Copie", Box::new(move |d| d.new_smart_by_copy(smart).map(|_| ()))),
            ("Instance", Box::new(move |d| d.duplicate_layer(smart).map(|_| ()))),
            ("Pixelliser", Box::new(move |d| d.rasterize_layer(smart))),
        ];
        for (label, step) in steps {
            let before = (session.document().layer_count(), picture(session.document()), raster_of(session.document(), smart));
            session.apply(label, None, |d| step(d)).expect(label);
            session.undo();
            let after = (session.document().layer_count(), picture(session.document()), raster_of(session.document(), smart));
            assert!(before.0 == after.0 && before.1 == after.1 && Arc::ptr_eq(&before.2, &after.2), "{label} annulé");
            session.redo();
            session.undo();
        }
        session.jump(0);
        assert_eq!(picture(session.document()), initial);
    }

    #[test]
    fn projects_store_each_content_once_and_restore_instances_originals_and_nesting() {
        let dir = tempfile::tempdir().expect("dossier");
        let image = dir.path().join("photo.png");
        write_png(&image, (80, 60), [10, 200, 30, 255]);
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (20, 20), (100, 100));
        let a = doc.convert_to_smart(&[id]).expect("conversion");
        let instance = doc.duplicate_layer(a).expect("instance");
        doc.transform_layers(&[instance], &about((70.0, 70.0), rotation(12.0)), Quality::Final).expect("rotation");
        let copy = doc.new_smart_by_copy(a).expect("copie");
        doc.translate_layers(&[copy], 150, 120).expect("décalage");
        let nested = doc.convert_to_smart(&[copy]).expect("imbrication");
        let placed = doc.place_smart_object(content_from_file(&image, false).expect("contenu"), None, 1).expect("placer");
        let before = picture(&doc);

        let path = dir.path().join("objets.graf");
        crate::io::save_project(&doc, &path).expect("enregistrement");
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).expect("fichier")).expect("archive");
        let mut read = |name: &str| {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut zip.by_name(name).expect(name), &mut bytes).expect("lecture");
            bytes
        };
        let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).expect("manifeste");
        assert_eq!(manifest["format_version"], 7);
        let structure: serde_json::Value = serde_json::from_slice(&read("document.json")).expect("structure");
        let contents = structure["contents"].as_array().expect("table des contenus");
        assert_eq!(contents.len(), 4, "A (et son instance), la copie imbriquée, l'objet qui l'imbrique, la photo : chacun une fois");
        assert_eq!(contents.iter().filter(|c| c["source"]["kind"] == "original").count(), 1);
        let key = content_of(&doc, placed).id;
        assert_eq!(read(&format!("smart/{key}/original")), std::fs::read(&image).expect("photo"), "image d'origine à l'octet près");

        let loaded = crate::io::load_project(&path).expect("chargement");
        assert_eq!(picture(&loaded), before, "même image");
        assert!(Arc::ptr_eq(&content_of(&loaded, a), &content_of(&loaded, instance)), "instances restaurées");
        assert_ne!(content_of(&loaded, a).id, content_of(&loaded, nested).id);
        let inner = content_of(&loaded, nested);
        let inner_smart = inner.document.walk_top_down().into_iter().find_map(|(_, l)| l.smart().cloned()).expect("objet imbriqué");
        assert_eq!(inner_smart.width(), 100);
        assert_ne!(inner_smart.id, content_of(&loaded, a).id, "la copie reste indépendante");
        let photo = content_of(&loaded, placed);
        assert_eq!(photo.original().map(|o| o.name.as_str()), Some("photo.png"));
        let angle = loaded.smart_geometry(instance).expect("géométrie").rotation_deg;
        assert!((angle - 12.0).abs() < 1e-9, "transformation de l'instance conservée : {angle}");
    }

    #[test]
    fn documents_without_smart_objects_stay_readable_by_older_versions() {
        let dir = tempfile::tempdir().expect("dossier");
        let path = dir.path().join("simple.graf");
        let mut doc = document();
        add_pattern(&mut doc, "Motif", (0, 0), (10, 10));
        crate::io::save_project(&doc, &path).expect("enregistrement");
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).expect("fichier")).expect("archive");
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut zip.by_name("manifest.json").expect("manifeste"), &mut bytes).expect("lecture");
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).expect("manifeste");
        assert_eq!(manifest["format_version"], 6, "aucun objet dynamique : format 6, celui des versions précédentes");
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut zip.by_name("document.json").expect("structure"), &mut bytes).expect("lecture");
        assert!(!String::from_utf8_lossy(&bytes).contains("contents"));
        assert_eq!(picture(&crate::io::load_project(&path).expect("relu")), picture(&doc));
    }

    #[test]
    fn linked_files_are_reloaded_when_they_change_and_kept_when_they_vanish() {
        let dir = tempfile::tempdir().expect("dossier");
        let image = dir.path().join("lien.png");
        write_png(&image, (64, 48), [255, 0, 0, 255]);
        let mut doc = document();
        let content = content_from_file(&image, true).expect("lien");
        assert!(content.is_linked() && content.original().is_none());
        let smart = doc.place_smart_object(content, None, 1).expect("placer");
        let color = |doc: &Document| crate::tiles::Sampler::new(&raster_of(doc, smart)).get(10, 10);
        assert_eq!(color(&doc), [255, 0, 0, 255]);
        assert_eq!(doc.refresh_smart_links(), crate::document::SmartLinksReport::default(), "rien n'a changé");

        // Fichier modifié : relu, l'objet est refait.
        write_png(&image, (64, 48), [0, 0, 255, 255]);
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&image).expect("fichier").set_modified(later).expect("date");
        let report = doc.refresh_smart_links();
        assert_eq!(report.updated, vec![content_of(&doc, smart).id]);
        assert_eq!(color(&doc), [0, 0, 255, 255]);

        // Projet enregistré, puis fichier lié supprimé : le dernier rendu reste, l'objet est signalé manquant.
        let project = dir.path().join("projet.graf");
        crate::io::save_project(&doc, &project).expect("enregistrement");
        std::fs::remove_file(&image).expect("suppression");
        let before = raster_of(&doc, smart);
        let report = doc.refresh_smart_links();
        assert_eq!(report.missing, vec![content_of(&doc, smart).id]);
        assert!(content_of(&doc, smart).missing() && Arc::ptr_eq(&before, &raster_of(&doc, smart)), "dernier rendu conservé");
        let mut reopened = crate::io::load_project(&project).expect("relu sans le fichier lié");
        assert!(content_of(&reopened, smart).missing());
        assert_eq!(color(&reopened), [0, 0, 255, 255], "dernier rendu enregistré");

        // Le fichier revient (modifié) : relu à la vérification suivante.
        write_png(&image, (64, 48), [0, 255, 0, 255]);
        let report = reopened.refresh_smart_links();
        assert_eq!((report.updated.len(), report.missing.len()), (1, 0));
        assert_eq!(color(&reopened), [0, 255, 0, 255]);
        // Projet et fichier lié déplacés ensemble : retrouvé par le chemin relatif.
        let moved = tempfile::tempdir().expect("dossier");
        crate::io::save_project(&reopened, &project).expect("enregistrement");
        std::fs::rename(&image, moved.path().join("lien.png")).expect("déplacement");
        std::fs::rename(&project, moved.path().join("projet.graf")).expect("déplacement");
        let found = crate::io::load_project(&moved.path().join("projet.graf")).expect("relu ailleurs");
        let content = content_of(&found, smart);
        assert!(!content.missing() && content.path() == Some(&std::path::absolute(moved.path().join("lien.png")).expect("chemin")));
    }

    #[test]
    fn embedding_a_link_keeps_the_render() {
        let dir = tempfile::tempdir().expect("dossier");
        let image = dir.path().join("lien.png");
        write_png(&image, (30, 30), [9, 9, 9, 255]);
        let mut doc = document();
        let smart = doc.place_smart_object(content_from_file(&image, true).expect("lien"), None, 1).expect("placer");
        let render = raster_of(&doc, smart);
        doc.embed_smart_link(smart).expect("incorporer");
        assert!(!content_of(&doc, smart).is_linked() && Arc::ptr_eq(&render, &raster_of(&doc, smart)));
    }

    #[test]
    fn placing_a_large_image_fits_it_without_enlarging_a_small_one() {
        let dir = tempfile::tempdir().expect("dossier");
        let (big, small) = (dir.path().join("grande.png"), dir.path().join("petite.png"));
        write_png(&big, (800, 300), [1, 2, 3, 255]);
        write_png(&small, (40, 20), [1, 2, 3, 255]);
        let mut doc = document();
        let a = doc.place_smart_object(content_from_file(&big, false).expect("contenu"), None, 1).expect("placer");
        let b = doc.place_smart_object(content_from_file(&small, false).expect("contenu"), None, 2).expect("placer");
        let ga = doc.smart_geometry(a).expect("géométrie");
        assert_eq!((ga.scale_x, ga.corners[0], ga.corners[2]), (0.5, (0.0, 75.0), (400.0, 225.0)), "réduite pour tenir, centrée");
        let gb = doc.smart_geometry(b).expect("géométrie");
        assert_eq!((gb.scale_x, gb.x, gb.y), (1.0, 180.0, 140.0), "petite : taille réelle, centrée");
        assert_eq!(doc.layer(b).expect("calque").props.name, "petite");
    }

    #[test]
    fn the_recovery_copy_keeps_smart_objects() {
        let dir = tempfile::tempdir().expect("dossier");
        let image = dir.path().join("photo.png");
        write_png(&image, (50, 50), [200, 100, 0, 255]);
        let mut doc = document();
        let id = add_pattern(&mut doc, "Motif", (5, 5), (60, 60));
        let mut session = Session::new(doc, "Ouvrir");
        let (smart, _) = session.apply("Convertir", None, |d| d.convert_to_smart(&[id])).expect("conversion");
        session.apply("Placer", None, |d| d.place_smart_object(content_from_file(&image, false)?, None, 1)).expect("placer");
        session.apply("Instance", None, |d| d.duplicate_layer(smart)).expect("instance");
        let path = dir.path().join("secours.graf");
        crate::io::save_recovery(&session.snapshot(), None, &path, &|_| {}).expect("copie de secours");
        let (history, _) = crate::io::load_recovery(&path).expect("relue");
        assert_eq!(history.states.len(), 4);
        let last = &history.states[3].1;
        assert_eq!(picture(last), picture(session.document()));
        let instances: Vec<_> = last.walk_top_down().into_iter().filter_map(|(_, l)| l.smart().cloned()).collect();
        assert_eq!(instances.len(), 3);
        assert!(instances.iter().filter(|c| c.original().is_some()).count() == 1);
    }
}
