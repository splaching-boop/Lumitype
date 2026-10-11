//! Lecture et écriture : images d'échange (PNG, JPEG, WebP) et format de projet.
//!
//! # Format de projet
//!
//! Archive zip **non compressée** (les PNG le sont déjà) :
//!
//! ```text
//! manifest.json                 { "format", "format_version", "app_version" }
//! document.json                 structure : dimensions, arbre de calques, listes de tuiles
//! layers/<id>/<x>_<y>.png       tuile de calque 256x256, RGBA 8 bits, alpha droit
//! masks/<id>/<x>_<y>.png        tuile de masque 256x256, niveaux de gris 8 bits   (v2)
//! smart/<clé>/layers/…          tuiles d'un contenu d'objet dynamique incorporé    (v7)
//! smart/<clé>/masks/…           masques de ce contenu                              (v7)
//! smart/<clé>/original          image d'origine d'un contenu placé, octets bruts   (v7)
//! smart/<clé>/flat/<x>_<y>.png  dernier rendu d'un contenu lié                     (v7)
//! ```
//!
//! | Version | Contenu                                                                 |
//! |---------|-------------------------------------------------------------------------|
//! | 1       | pile plate de calques de pixels                                         |
//! | 2       | arbre (groupes), opacité, fond, mode, verrous, écrêtage, masques        |
//! | 3       | position par calque ; coordonnées de tuiles signées (contenu hors cadre)|
//! | 4       | calques de texte (modèle éditable + rendu en cache)                     |
//! | 5       | calques de réglage                                                      |
//! | 6       | calques de forme ; extension `.graf` (les `.iep` s'ouvrent toujours)    |
//! | 7       | objets dynamiques : table `contents` des contenus, tuiles sous `smart/` |
//!
//! `format_version` augmente à chaque changement incompatible. Un fichier plus
//! récent que l'application est refusé avec un message clair ; un fichier plus
//! ancien est converti au chargement (voir [`DocumentV1::upgrade`] ; la version 2
//! se lit comme la 3, la position valant alors `(0, 0)`).
//!
//! # Objets dynamiques (version 7)
//!
//! Chaque contenu est décrit **une fois** dans la table `contents` de `document.json`, quel que soit le nombre de
//! ses instances (et de ses imbrications : un contenu peut contenir d'autres objets dynamiques, tous rangés dans la
//! même table). Un calque `smart` désigne son contenu par sa clé, garde sa matrice (`transform`) et son rendu
//! (`tiles`, sous `layers/<id>/` comme un texte : le fichier s'affiche sans rien recalculer). Un contenu est :
//! - `embedded` : la structure de son document, ses tuiles sous `smart/<clé>/layers|masks/` ;
//! - `original` : une image placée, gardée à l'octet près (`smart/<clé>/original`) et décodée au chargement ;
//! - `linked` : chemin absolu et chemin relatif au projet (essayé si l'absolu n'existe plus : dossier déplacé), date
//!   du fichier, et le dernier rendu (`smart/<clé>/flat/`), affiché si le fichier a disparu. Un fichier modifié
//!   depuis l'enregistrement est relu et ses instances rendues de nouveau.
//!
//! Un document sans objet dynamique s'écrit toujours en version 6 : les versions précédentes de Lumitype l'ouvrent
//! comme avant. Un document qui en contient s'écrit en version 7, qu'une version précédente refuse d'ouvrir avec
//! son message « projet enregistré avec un format plus récent » — rien n'est lu à moitié ni abîmé.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder, ImageFormat, ImageReader};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::blend::BlendMode;
use crate::document::{Document, Guide, Layer, LayerId, LayerKind, LayerProps, Locks, Mask};
use crate::error::EngineError;
use crate::metadata::ImageMetadata;
use crate::pixel::{premultiply, unpremultiply};
use crate::render::TileRenderer;
use crate::smart::{OriginalFile, SmartContent, SmartSource};
use crate::tiles::{tiles_for, MaskStore, MaskTile, Store, Tile, TileBuf, TileCoord, TileStore, MAX_DIMENSION, TILE_SIZE};

/// Garde-fou à la lecture : aucune tuile légitime n'est à plus d'un million de tuiles de l'origine.
const MAX_TILE_DISTANCE: i32 = 1 << 20;

pub const FORMAT_NAME: &str = "image-editor-project";
pub const FORMAT_VERSION: u32 = 7;
/// Version écrite pour un document sans objet dynamique : les versions précédentes de Lumitype l'ouvrent encore.
const FORMAT_VERSION_WITHOUT_SMART: u32 = 6;
/// Extension des fichiers de projet, sans le point.
pub const PROJECT_EXTENSION: &str = "graf";
/// Ancienne extension, du temps où le logiciel n'avait pas de nom : toujours acceptée à l'ouverture.
pub const LEGACY_PROJECT_EXTENSION: &str = "iep";

/// Vrai si le chemin désigne un projet (`.graf`, ou l'ancien `.iep`), d'après son extension.
pub fn is_project_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXTENSION) || e.eq_ignore_ascii_case(LEGACY_PROJECT_EXTENSION))
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: String,
    format_version: u32,
    app_version: String,
}

// ------------------------------------------------------------------ version 1

#[derive(Deserialize)]
struct DocumentV1 {
    name: String,
    width: u32,
    height: u32,
    dpi: f32,
    /// Du calque le plus bas au plus haut.
    layers: Vec<LayerV1>,
}

#[derive(Deserialize)]
struct LayerV1 {
    id: LayerId,
    name: String,
    visible: bool,
    tiles: Vec<[i32; 2]>,
}

impl DocumentV1 {
    /// Migration 1 -> 2 : chaque calque devient un calque de pixels de premier niveau.
    fn upgrade(self) -> DocumentV2 {
        DocumentV2 {
            name: self.name,
            width: self.width,
            height: self.height,
            dpi: self.dpi,
            root: self.layers.iter().map(|l| l.id).collect(),
            guides: Vec::new(),
            guides_locked: false,
            paths: Vec::new(),
            contents: Vec::new(),
            layers: self
                .layers
                .into_iter()
                .map(|l| LayerV2 {
                    id: l.id,
                    parent: None,
                    offset: [0, 0],
                    name: l.name,
                    visible: l.visible,
                    opacity: 1.0,
                    fill: 1.0,
                    blend: BlendMode::Normal,
                    locks: Locks::default(),
                    clipped: false,
                    mask: None,
                    effects: Vec::new(),
                    label: None,
                    kind: KindV2::Pixel { tiles: l.tiles },
                })
                .collect(),
        }
    }
}

// ------------------------------------------------------------------ version 2

#[derive(Serialize, Deserialize)]
struct DocumentV2 {
    name: String,
    width: u32,
    height: u32,
    dpi: f32,
    /// Calques de premier niveau, du bas vers le haut.
    root: Vec<LayerId>,
    layers: Vec<LayerV2>,
    /// Ajout compatible : absent des anciens fichiers, ignoré par les anciennes versions.
    #[serde(default)]
    guides: Vec<Guide>,
    /// Verrou des repères. Ajout compatible : un ancien fichier les rouvre déverrouillés.
    #[serde(default)]
    guides_locked: bool,
    /// Tracé de travail de la plume. Ajout compatible, comme les repères.
    #[serde(default)]
    paths: Vec<crate::path::SubPath>,
    /// Contenus des objets dynamiques (version 7), dans le seul document principal ; absent sans objet dynamique.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    contents: Vec<ContentV7>,
}

/// Contenu d'objet dynamique (version 7). `key` : clé désignée par les calques `smart`, propre au fichier.
#[derive(Serialize, Deserialize)]
struct ContentV7 {
    key: u64,
    name: String,
    width: u32,
    height: u32,
    source: SourceV7,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SourceV7 {
    /// Document incorporé : sa structure (sans table `contents` : tous les contenus sont dans celle du document
    /// principal), ses tuiles sous `smart/<clé>/`.
    Embedded { document: Box<DocumentV2> },
    /// Image placée gardée telle quelle dans `smart/<clé>/original` ; le contenu est son décodage.
    Original { file_name: String },
    /// Fichier lié : chemin absolu, chemin relatif au dossier du projet, date du fichier lu ; `tiles` : dernier rendu
    /// (`smart/<clé>/flat/`), affiché si le fichier est introuvable.
    Linked {
        path: PathBuf,
        #[serde(default)]
        relative: Option<PathBuf>,
        #[serde(default)]
        modified: Option<std::time::SystemTime>,
        tiles: Vec<[i32; 2]>,
    },
}

#[derive(Serialize, Deserialize)]
struct LayerV2 {
    id: LayerId,
    parent: Option<LayerId>,
    /// Absent des fichiers version 2 : le calque est alors à l'origine.
    #[serde(default)]
    offset: [i32; 2],
    name: String,
    visible: bool,
    opacity: f32,
    fill: f32,
    blend: BlendMode,
    locks: Locks,
    clipped: bool,
    mask: Option<MaskV2>,
    /// Effets de calque. Ajout compatible : absent des fichiers plus anciens.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    effects: Vec<crate::effects::LayerEffect>,
    /// Couleur d'étiquette dans le panneau. Ajout compatible : absente des fichiers plus anciens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<crate::document::LayerLabel>,
    #[serde(flatten)]
    kind: KindV2,
}

#[derive(Serialize, Deserialize)]
struct MaskV2 {
    enabled: bool,
    /// Valeur des tuiles absentes : 255 = tout révéler, 0 = tout masquer.
    background: u8,
    tiles: Vec<[i32; 2]>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum KindV2 {
    Pixel { tiles: Vec<[i32; 2]> },
    /// Le rendu est enregistré avec le modèle : le fichier s'affiche à l'identique même
    /// sur une machine où la police manque ; elle n'est requise que pour modifier le texte.
    Text {
        text: crate::text::TextData,
        /// Transformation du texte (Ctrl+T). Absente des fichiers écrits avant son existence : identité.
        #[serde(default)]
        transform: crate::transform::Matrix,
        tiles: Vec<[i32; 2]>,
    },
    Adjustment { adjustment: crate::adjust::Adjustment },
    Shape { shape: crate::shape::ShapeData, tiles: Vec<[i32; 2]> },
    /// Objet dynamique (version 7) : clé de son contenu dans la table `contents`, transformation, rendu en cache.
    Smart {
        content: u64,
        #[serde(default)]
        transform: crate::transform::Matrix,
        tiles: Vec<[i32; 2]>,
    },
    /// Enfants du bas vers le haut.
    Group { children: Vec<LayerId>, expanded: bool },
}

fn coords_of<const C: usize>(store: &Store<C>) -> Vec<[i32; 2]> {
    store.sorted_coords().iter().map(|c| [c.x, c.y]).collect()
}

/// Contenus d'objets dynamiques rencontrés en décrivant un document : chacun reçoit une clé et n'est décrit qu'une
/// fois, quel que soit le nombre de ses instances. La clé est son identifiant, sauf si deux contenus différents
/// partagent le même (une instance imbriquée restée à l'ancienne version d'un contenu modifié depuis) : ils sont alors
/// enregistrés séparément.
#[derive(Default)]
struct ContentKeys {
    by_address: HashMap<usize, u64>,
    used: HashSet<u64>,
    list: Vec<(u64, Arc<SmartContent>)>,
}

impl ContentKeys {
    fn key(&mut self, content: &Arc<SmartContent>) -> u64 {
        let address = Arc::as_ptr(content) as usize;
        if let Some(key) = self.by_address.get(&address) {
            return *key;
        }
        let key = if self.used.contains(&content.id) { self.used.iter().max().map_or(1, |k| k + 1) } else { content.id };
        self.used.insert(key);
        self.by_address.insert(address, key);
        self.list.push((key, Arc::clone(content)));
        key
    }
}

/// Description complète : le document, et la table de tous ses contenus d'objets dynamiques (imbriqués compris), avec
/// la clé de chacun.
fn describe(doc: &Document) -> (DocumentV2, Vec<(u64, Arc<SmartContent>)>) {
    let mut keys = ContentKeys::default();
    let mut structure = describe_one(doc, &mut keys);
    let mut next = 0;
    while next < keys.list.len() {
        let (key, content) = keys.list[next].clone();
        next += 1;
        let source = match &content.source {
            SmartSource::Embedded { original: Some(original) } => SourceV7::Original { file_name: original.name.clone() },
            SmartSource::Embedded { original: None } => SourceV7::Embedded { document: Box::new(describe_one(&content.document, &mut keys)) },
            SmartSource::Linked { path, modified, .. } => SourceV7::Linked { path: path.clone(), relative: None, modified: *modified, tiles: coords_of(&content.flattened) },
        };
        structure.contents.push(ContentV7 { key, name: content.name.clone(), width: content.width(), height: content.height(), source });
    }
    (structure, keys.list)
}

fn describe_one(doc: &Document, keys: &mut ContentKeys) -> DocumentV2 {
    let mut layers: Vec<LayerV2> = doc
        .nodes()
        .map(|layer| LayerV2 {
            id: layer.id,
            parent: layer.parent,
            offset: [layer.offset.0, layer.offset.1],
            name: layer.props.name.clone(),
            visible: layer.props.visible,
            opacity: layer.props.opacity,
            fill: layer.props.fill,
            blend: layer.props.blend,
            locks: layer.props.locks,
            clipped: layer.props.clipped,
            mask: layer.mask.as_ref().map(|m| MaskV2 {
                enabled: m.enabled,
                background: m.pixels.background()[0],
                tiles: coords_of(&m.pixels),
            }),
            effects: layer.effects.clone(),
            label: layer.label,
            kind: match &layer.kind {
                LayerKind::Pixel { pixels } => KindV2::Pixel { tiles: coords_of(pixels) },
                LayerKind::Adjustment { adjustment } => KindV2::Adjustment { adjustment: adjustment.clone() },
                LayerKind::Shape { shape, raster } => KindV2::Shape { shape: shape.clone(), tiles: coords_of(raster) },
                LayerKind::Text { text, transform, raster } => {
                    KindV2::Text { text: text.clone(), transform: *transform, tiles: coords_of(raster) }
                }
                LayerKind::Group { children, expanded } => {
                    KindV2::Group { children: children.clone(), expanded: *expanded }
                }
                LayerKind::Smart { content, transform, raster } => KindV2::Smart { content: keys.key(content), transform: *transform, tiles: coords_of(raster) },
            },
        })
        .collect();
    layers.sort_unstable_by_key(|l| l.id); // fichier identique d'un enregistrement à l'autre
    DocumentV2 {
        name: doc.name.clone(),
        width: doc.width(),
        height: doc.height(),
        dpi: doc.dpi,
        root: doc.children_of(None).map(<[LayerId]>::to_vec).unwrap_or_default(),
        layers,
        guides: doc.guides.clone(),
        guides_locked: doc.guides_locked,
        paths: doc.paths.clone(),
        contents: Vec::new(),
    }
}

// --------------------------------------------------------------------- images

/// Décode une image d'échange en RGBA prémultiplié. `origin` ne sert qu'aux messages d'erreur.
///
/// PNG, JPEG, WebP et TIFF passent par la bibliothèque `image`. Les photos HEIC/HEIF (iPhone) et
/// les négatifs numériques DNG ont leur propre décodeur, choisi d'après le contenu du fichier — pas
/// son extension, souvent absente ou trompeuse quand l'image vient d'un autre logiciel.
fn decode_image(bytes: &[u8], origin: &Path) -> Result<(u32, u32, Vec<u8>), EngineError> {
    let image_error = |source| EngineError::Image { path: origin.to_path_buf(), source };
    let mut rgba = if is_heif(bytes) {
        decode_heif(bytes, origin)?
    } else if is_dng(bytes, origin) {
        decode_dng(bytes, origin)?
    } else {
        let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(EngineError::io(origin))?;
        reader.decode().map_err(image_error)?.into_rgba8()
    };
    let (width, height) = rgba.dimensions();
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(EngineError::InvalidDimensions { width, height, max: MAX_DIMENSION });
    }
    rgba.par_chunks_exact_mut(4).for_each(|px| {
        let p = premultiply([px[0], px[1], px[2], px[3]]);
        px.copy_from_slice(&p);
    });
    Ok((width, height, rgba.into_raw()))
}

/// Conteneur HEIF : une boîte `ftyp` en tête, avec une marque d'image fixe (heic/heix : HEVC ;
/// mif1/msf1 : HEIF générique, ce que produit un iPhone ; avif exclu, il passe par la webview).
fn is_heif(bytes: &[u8]) -> bool {
    bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && matches!(&bytes[8..12], b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"hevm" | b"hevs" | b"mif1" | b"msf1")
}

/// Un DNG est un TIFF portant l'étiquette `DNGVersion` (0xC612) dans son premier répertoire.
/// L'extension `.dng` suffit aussi : un fichier ainsi nommé mais sans étiquette serait de toute
/// façon refusé par la bibliothèque `image`, faute de pixels lisibles dans le premier répertoire.
fn is_dng(bytes: &[u8], origin: &Path) -> bool {
    if origin.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("dng")) {
        return true;
    }
    let little = match bytes.get(0..4) {
        Some(b"II*\0") => true,
        Some(b"MM\0*") => false,
        _ => return false,
    };
    let read16 = |at: usize| bytes.get(at..at + 2).map(|b| if little { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) });
    let read32 = |at: usize| bytes.get(at..at + 4).map(|b| if little { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) });
    let Some(ifd) = read32(4).map(|o| o as usize) else { return false };
    let Some(count) = read16(ifd).map(usize::from) else { return false };
    (0..count.min(512)).any(|i| read16(ifd + 2 + i * 12) == Some(0xC612))
}

/// HEIC/HEIF : décodeur HEVC 100 % Rust. Les rotations et miroirs enregistrés par l'iPhone
/// (`irot`, `imir`) sont appliqués par le décodeur : l'image arrive droite, comme dans Photos.
fn decode_heif(bytes: &[u8], origin: &Path) -> Result<image::RgbaImage, EngineError> {
    use heic_rs::{DecodeOptions, PixelLayout};
    let name = origin.file_name().and_then(|n| n.to_str()).unwrap_or("image");
    let decoded = heic_rs::decode(bytes, &DecodeOptions::default().with_layout(PixelLayout::Rgba8))
        .map_err(|e| EngineError::Corrupt(format!("HEIC « {name} » illisible : {e}")))?;
    image::RgbaImage::from_raw(decoded.width, decoded.height, decoded.data)
        .ok_or_else(|| EngineError::Corrupt(format!("HEIC « {name} » : tampon décodé de taille inattendue")))
}

/// DNG : développement complet (dématriçage, balance des blancs de la prise de vue, matrice couleur
/// de l'appareil vers sRGB, gamma sRGB), orientation appliquée. Si le développement échoue — une
/// compression que le décodeur ne connaît pas, par exemple — l'aperçu JPEG que le fichier embarque
/// sert de secours, avec un avertissement dans l'erreur seulement s'il n'y en a pas non plus.
fn decode_dng(bytes: &[u8], origin: &Path) -> Result<image::RgbaImage, EngineError> {
    use rawler::{decoders::RawDecodeParams, imgop::develop::RawDevelop, rawsource::RawSource, Orientation};
    let name = origin.file_name().and_then(|n| n.to_str()).unwrap_or("image");
    let corrupt = |what: String| EngineError::Corrupt(format!("DNG « {name} » : {what}"));
    let orient = |img: image::DynamicImage, o: Orientation| match o {
        Orientation::Normal | Orientation::Unknown => img,
        Orientation::HorizontalFlip => img.fliph(),
        Orientation::Rotate180 => img.rotate180(),
        Orientation::VerticalFlip => img.flipv(),
        Orientation::Transpose => img.rotate90().fliph(),
        Orientation::Rotate90 => img.rotate90(),
        Orientation::Transverse => img.rotate90().flipv(),
        Orientation::Rotate270 => img.rotate270(),
    };
    let source = RawSource::new_from_slice(bytes);
    let decoder = rawler::get_decoder(&source).map_err(|e| corrupt(format!("format non reconnu ({e})")))?;
    let params = RawDecodeParams::default();
    let developed = decoder
        .raw_image(&source, &params, false)
        .map_err(|e| e.to_string())
        .and_then(|raw| {
            let orientation = raw.orientation;
            RawDevelop::default()
                .develop_intermediate(&raw)
                .map_err(|e| e.to_string())
                .and_then(|inter| inter.to_dynamic_image().ok_or_else(|| "développement vide".to_string()))
                .map(|img| orient(img, orientation))
        });
    match developed {
        Ok(img) => Ok(img.to_rgba8()),
        Err(reason) => {
            // Secours : l'aperçu embarqué (pleine taille sur un ProRAW ou un export Lightroom).
            let orientation = decoder.raw_metadata(&source, &params).ok().and_then(|m| m.exif.orientation).map(Orientation::from_u16).unwrap_or(Orientation::Unknown);
            let preview = decoder.preview_image(&source, &params).ok().flatten().or_else(|| decoder.thumbnail_image(&source, &params).ok().flatten());
            preview.map(|img| orient(img, orientation).to_rgba8()).ok_or_else(|| corrupt(format!("développement impossible ({reason}) et aucun aperçu embarqué")))
        }
    }
}

fn file_stem(path: &Path) -> &str {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("Sans titre")
}

/// Importe une image d'échange comme document à un seul calque.
pub fn import_image(path: &Path) -> Result<Document, EngineError> {
    import_image_bytes(&std::fs::read(path).map_err(EngineError::io(path))?, path)
}

/// Variante depuis la mémoire (fichier déposé dans la fenêtre). `origin` donne le nom du document.
pub fn import_image_bytes(bytes: &[u8], origin: &Path) -> Result<Document, EngineError> {
    let (width, height, rgba) = decode_image(bytes, origin)?;
    let mut doc = Document::new(file_stem(origin), width, height, 72.0)?;
    let pixels = TileStore::from_premultiplied(width, height, &rgba)
        .ok_or_else(|| EngineError::Corrupt("tampon décodé de taille inattendue".into()))?;
    doc.add_pixel_layer("Arrière-plan", pixels, None, 0)?;
    Ok(doc)
}

/// Importe une image comme nouveau calque, centrée dans le document et rognée à ses bords.
pub fn import_as_layer(
    doc: &mut Document,
    path: &Path,
    parent: Option<LayerId>,
    index: usize,
) -> Result<LayerId, EngineError> {
    import_bytes_as_layer(doc, &std::fs::read(path).map_err(EngineError::io(path))?, path, parent, index)
}

/// Variante depuis la mémoire (fichier déposé dans la fenêtre). `origin` donne le nom du calque.
pub fn import_bytes_as_layer(
    doc: &mut Document,
    bytes: &[u8],
    origin: &Path,
    parent: Option<LayerId>,
    index: usize,
) -> Result<LayerId, EngineError> {
    let (width, height, rgba) = decode_image(bytes, origin)?;
    let pixels = TileStore::from_premultiplied(width, height, &rgba)
        .ok_or_else(|| EngineError::Corrupt("tampon décodé de taille inattendue".into()))?;
    // L'image est ajustée au document (entièrement visible, proportions gardées) puis centrée,
    // quelle que soit sa taille d'origine. Ctrl+T permet ensuite de la redimensionner.
    let fit = (f64::from(doc.width()) / f64::from(width)).min(f64::from(doc.height()) / f64::from(height));
    let (pixels, width, height) = if (fit - 1.0).abs() < 1e-9 {
        (pixels, f64::from(width), f64::from(height))
    } else {
        let (w, h) = ((f64::from(width) * fit).round().max(1.0), (f64::from(height) * fit).round().max(1.0));
        (resize_image(&rgba, width, height, w as u32, h as u32)?, w, h)
    };
    let id = doc.add_pixel_layer(file_stem(origin), pixels, parent, index)?;
    let dx = ((f64::from(doc.width()) - width) / 2.0).round() as i32;
    let dy = ((f64::from(doc.height()) - height) / 2.0).round() as i32;
    doc.translate_layers(&[id], dx, dy)?;
    Ok(id)
}

/// Redimensionne une image rectangulaire pleine en gardant des bords francs. Un rééchantillonnage
/// ordinaire verrait du transparent autour de l'image et fondrait ses bords sur plusieurs pixels
/// en agrandissement ; ici les bords sont d'abord prolongés, puis le résultat est rogné au rectangle exact.
fn resize_image(rgba: &[u8], width: u32, height: u32, new_width: u32, new_height: u32) -> Result<TileStore, EngineError> {
    const PAD: u32 = 3; // portée du noyau bicubique
    let (w, h) = (width as usize, height as usize);
    let (pw, ph) = (w + 2 * PAD as usize, h + 2 * PAD as usize);
    let mut padded = vec![0u8; pw * ph * 4];
    for y in 0..ph {
        let sy = y.saturating_sub(PAD as usize).min(h - 1);
        for x in 0..pw {
            let sx = x.saturating_sub(PAD as usize).min(w - 1);
            let (src, dst) = ((sy * w + sx) * 4, (y * pw + x) * 4);
            padded[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
        }
    }
    let source = TileStore::from_premultiplied(pw as u32, ph as u32, &padded)
        .ok_or_else(|| EngineError::Corrupt("tampon d'image de taille inattendue".into()))?
        .translated(-(PAD as i32), -(PAD as i32));
    let (sx, sy) = (f64::from(new_width) / f64::from(width), f64::from(new_height) / f64::from(height));
    let matrix = crate::transform::Matrix([sx, 0.0, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 1.0]);
    let resized = crate::transform::transform_store(&source, &matrix, crate::transform::Quality::Final)?;
    // Rognage au rectangle exact : `to_premultiplied` ne garde que [0, largeur) x [0, hauteur).
    TileStore::from_premultiplied(new_width, new_height, &resized.to_premultiplied(new_width, new_height))
        .ok_or_else(|| EngineError::Corrupt("tampon redimensionné de taille inattendue".into()))
}

/// PNG des pixels copiés, rogné à leur étendue : c'est ce qui part vers le presse-papiers du
/// système, pour coller dans un autre logiciel.
pub fn clip_to_png(clip: &crate::document::Clip) -> Result<Vec<u8>, EngineError> {
    let (x0, y0, x1, y1) = clip.pixels.pixel_bounds(|px| px[3] > 0).ok_or_else(|| EngineError::InvalidOperation("rien à copier".into()))?;
    let (width, height) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let mut rgba = clip.pixels.translated(-x0, -y0).to_premultiplied(width, height);
    for px in rgba.chunks_exact_mut(4) {
        let straight = crate::pixel::unpremultiply([px[0], px[1], px[2], px[3]]);
        px.copy_from_slice(&straight);
    }
    let image = image::RgbaImage::from_raw(width, height, rgba).ok_or_else(|| EngineError::Corrupt("tampon de taille inattendue".into()))?;
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).map_err(|e| EngineError::Corrupt(e.to_string()))?;
    Ok(out.into_inner())
}

/// Exporte l'aplat du document en PNG (alpha droit), toujours depuis le niveau 0.
pub fn export_png(doc: &Document, path: &Path) -> Result<(), EngineError> {
    let renderer = TileRenderer::new();
    let mut flat = TileStore::new();
    for ty in 0..tiles_for(doc.height()) as i32 {
        for tx in 0..tiles_for(doc.width()) as i32 {
            let coord = TileCoord::new(tx, ty);
            flat.restore(coord, renderer.tile(doc, 0, coord));
        }
    }
    let mut rgba = flat.to_premultiplied(doc.width(), doc.height());
    rgba.par_chunks_exact_mut(4).for_each(|px| {
        let p = unpremultiply([px[0], px[1], px[2], px[3]]);
        px.copy_from_slice(&p);
    });

    write_atomically(path, |file| {
        PngEncoder::new_with_quality(BufWriter::new(file), CompressionType::Default, FilterType::Adaptive)
            .write_image(&rgba, doc.width(), doc.height(), ExtendedColorType::Rgba8)
            .map_err(|source| EngineError::Image { path: path.to_path_buf(), source })
    })
}

/// Format d'un export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Png,
    Jpeg,
    /// Avec perte (VP8, 25 à 35 % plus léger que le JPEG à qualité égale, transparence comprise) par défaut,
    /// sans perte (VP8L) sur demande : voir `ExportOptions::lossless`. Encodé par libwebp (crate `webp`).
    Webp,
    Tiff,
}

impl ExportFormat {
    /// Vrai si l'encodage perd de l'information avec ces réglages : l'aperçu montre alors le format lui-même,
    /// pour qu'on voie ses défauts de compression.
    pub fn is_lossy(self, lossless: bool) -> bool {
        match self {
            Self::Jpeg => true,
            Self::Webp => !lossless,
            Self::Png | Self::Tiff => false,
        }
    }
}

/// Réglages d'un export : format, qualité (JPEG et WebP avec perte, 1 à 100), `lossless` (WebP seulement : sans
/// perte, la qualité est alors ignorée), échelle (1 = taille native) et métadonnées à inscrire dans le fichier
/// (auteur, copyright… voir [`crate::metadata`]).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub quality: u8,
    #[serde(default)]
    pub lossless: bool,
    pub scale: f32,
    #[serde(default)]
    pub metadata: Option<ImageMetadata>,
}

/// Inscrit les métadonnées (s'il y en a) dans l'image encodée.
fn with_metadata(format: ExportFormat, bytes: Vec<u8>, metadata: Option<&ImageMetadata>) -> Result<Vec<u8>, EngineError> {
    match metadata {
        Some(meta) => crate::metadata::embed(format, bytes, meta),
        None => Ok(bytes),
    }
}

/// Encodage nu, pour les tests du module `metadata` ; `lossless` ne concerne que le WebP.
#[cfg(test)]
pub(crate) fn encode_for_tests(width: u32, height: u32, rgba: &[u8], format: ExportFormat, lossless: bool) -> Vec<u8> {
    encode(width, height, rgba, format, 90, lossless).expect("encodage")
}

/// Image aplatie puis mise à l'échelle : largeur, hauteur, pixels RGBA à alpha droit.
/// Redimensionnement Lanczos3 d'une image RGBA prémultipliée, en SIMD et sur tous les cœurs (fast_image_resize).
fn resize_premultiplied(rgba: Vec<u8>, width: u32, height: u32, new_width: u32, new_height: u32) -> Result<Vec<u8>, EngineError> {
    use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images::Image};
    let failed = |e: &dyn std::fmt::Display| EngineError::Corrupt(format!("mise à l'échelle impossible : {e}"));
    let source = Image::from_vec_u8(width, height, rgba, PixelType::U8x4).map_err(|e| failed(&e))?;
    let mut target = Image::new(new_width, new_height, PixelType::U8x4);
    // Les pixels sont déjà prémultipliés : pas de traitement de l'alpha par la bibliothèque.
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)).use_alpha(false);
    Resizer::new().resize(&source, &mut target, &options).map_err(|e| failed(&e))?;
    let mut out = target.into_vec();
    // Lanczos dépasse un peu aux contrastes forts : une couleur prémultipliée ne doit jamais dépasser son alpha.
    out.par_chunks_exact_mut(4).for_each(|px| {
        let a = px[3];
        px[0] = px[0].min(a);
        px[1] = px[1].min(a);
        px[2] = px[2].min(a);
    });
    Ok(out)
}

/// Petit aperçu PNG du document (projets récents de l'écran d'accueil), côté le plus long ≤ `max_side`. Composé au
/// niveau réduit de la pyramide le plus proche : rapide même sur un très grand document.
pub fn preview_png(doc: &Document, max_side: u32) -> Result<Vec<u8>, EngineError> {
    let renderer = TileRenderer::new();
    let longest = doc.width().max(doc.height()).max(1);
    // Niveau le plus réduit qui garde au moins `max_side` pixels sur le côté le plus long.
    let mut level = 0u8;
    while level < TileRenderer::max_level(doc) && (longest >> (level + 1)) >= max_side.max(1) {
        level += 1;
    }
    let (w, h) = (doc.width().div_ceil(1 << level).max(1), doc.height().div_ceil(1 << level).max(1));
    let mut flat = TileStore::new();
    for ty in 0..tiles_for(h) as i32 {
        for tx in 0..tiles_for(w) as i32 {
            let coord = TileCoord::new(tx, ty);
            flat.restore(coord, renderer.tile(doc, level, coord));
        }
    }
    let source = image::RgbaImage::from_raw(w, h, flat.to_premultiplied(w, h)).ok_or_else(|| EngineError::Corrupt("tampon de taille inattendue".into()))?;
    let k = (max_side as f32 / w.max(h) as f32).min(1.0);
    let (pw, ph) = (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1));
    let mut rgba = image::imageops::resize(&source, pw, ph, image::imageops::FilterType::Triangle).into_raw();
    for px in rgba.chunks_exact_mut(4) {
        let p = unpremultiply([px[0], px[1], px[2], px[3]]);
        px.copy_from_slice(&p);
    }
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive)
        .write_image(&rgba_to_rgb_on(&rgba, [29, 29, 30]), pw, ph, ExtendedColorType::Rgb8)
        .map_err(|e| EngineError::Corrupt(format!("aperçu impossible : {e}")))?;
    Ok(png)
}

/// Pose une image RGBA (non prémultipliée) sur un fond uni : la transparence d'un aperçu se lit sur le gris du canevas.
fn rgba_to_rgb_on(rgba: &[u8], background: [u8; 3]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|px| {
            let a = u32::from(px[3]);
            std::array::from_fn::<u8, 3, _>(|i| ((u32::from(px[i]) * a + u32::from(background[i]) * (255 - a) + 127) / 255) as u8)
        })
        .collect()
}

fn flattened(doc: &Document, scale: f32, progress: &dyn Fn(f32, &str)) -> Result<(u32, u32, Vec<u8>), EngineError> {
    let renderer = TileRenderer::new();
    let mut flat = TileStore::new();
    let (rows, columns) = (tiles_for(doc.height()) as i32, tiles_for(doc.width()) as i32);
    // Composition EN PARALLÈLE, une rangée de tuiles par tâche, sur tous les cœurs (auparavant : une tuile après
    // l'autre). Les rangées terminées sont signalées au fil de l'eau ; l'avancement est remonté depuis ce fil-ci.
    let (finished_row, finished_rows) = std::sync::mpsc::channel::<()>();
    let composed: Vec<(TileCoord, Option<Arc<crate::tiles::Tile>>)> = std::thread::scope(|scope| {
        let renderer = &renderer;
        let work = scope.spawn(move || {
            (0..rows)
                .into_par_iter()
                .flat_map_iter(|ty| {
                    let row: Vec<_> = (0..columns).map(|tx| TileCoord::new(tx, ty)).map(|coord| (coord, renderer.tile(doc, 0, coord))).collect();
                    let _ = finished_row.send(());
                    row
                })
                .collect::<Vec<_>>()
        });
        for (count, ()) in finished_rows.iter().enumerate() {
            progress(0.6 * (count + 1) as f32 / rows as f32, "Composition des calques");
        }
        work.join().map_err(|_| EngineError::Corrupt("composition interrompue".into()))
    })?;
    for (coord, tile) in composed {
        flat.restore(coord, tile);
    }
    progress(0.62, "Mise à l'échelle");
    let scale = scale.clamp(0.01, 8.0);
    let (width, height) = (((doc.width() as f32 * scale).round() as u32).clamp(1, MAX_DIMENSION), ((doc.height() as f32 * scale).round() as u32).clamp(1, MAX_DIMENSION));
    let premultiplied = flat.to_premultiplied(doc.width(), doc.height());
    // Le redimensionnement se fait en alpha prémultiplié : pas de frange sombre au bord du transparent.
    let mut rgba = if (width, height) == (doc.width(), doc.height()) {
        premultiplied
    } else {
        resize_premultiplied(premultiplied, doc.width(), doc.height(), width, height)?
    };
    rgba.par_chunks_exact_mut(4).for_each(|px| {
        let p = unpremultiply([px[0], px[1], px[2], px[3]]);
        px.copy_from_slice(&p);
    });
    Ok((width, height, rgba))
}

/// `lossless` ne concerne que le WebP ; `quality` (1 à 100) que le JPEG et le WebP avec perte.
fn encode(width: u32, height: u32, rgba: &[u8], format: ExportFormat, quality: u8, lossless: bool) -> Result<Vec<u8>, EngineError> {
    match format {
        // PNG multi-cœurs (voir png_parallel.rs) : même image, au pixel près, plusieurs fois plus vite.
        ExportFormat::Png => crate::png_parallel::encode(width, height, rgba, 4, 6),
        ExportFormat::Webp => {
            // libwebp (crate `webp`) : la bibliothèque de référence, avec perte (VP8 + alpha) ou sans (VP8L).
            // L'image est limitée à 16 383 px de côté par le format lui-même.
            if width > 16_383 || height > 16_383 {
                return Err(EngineError::Corrupt("le WebP est limité à 16 383 px de côté".into()));
            }
            let encoder = webp::Encoder::from_rgba(rgba, width, height);
            let encoded = if lossless { encoder.encode_lossless() } else { encoder.encode(f32::from(quality.clamp(1, 100))) };
            Ok(encoded.to_vec())
        }
        ExportFormat::Tiff => {
            let mut out = std::io::Cursor::new(Vec::new());
            image::codecs::tiff::TiffEncoder::new(&mut out)
                .write_image(rgba, width, height, ExtendedColorType::Rgba8)
                .map_err(|e| EngineError::Corrupt(format!("encodage impossible : {e}")))?;
            // L'encodeur ne déclare pas la quatrième composante : tag ExtraSamples ajouté (alpha non associé).
            crate::metadata::tiff_with_unassociated_alpha(out.into_inner())
        }
        ExportFormat::Jpeg => {
            // Le JPEG n'a pas de transparence : l'image est posée sur du blanc.
            let mut rgb = vec![0u8; (width as usize) * (height as usize) * 3];
            rgb.par_chunks_exact_mut(3).zip(rgba.par_chunks_exact(4)).for_each(|(out, px)| {
                let a = u32::from(px[3]);
                for c in 0..3 {
                    out[c] = ((u32::from(px[c]) * a + 255 * (255 - a) + 127) / 255) as u8;
                }
            });
            // Encodeur JPEG SIMD (jpeg-encoder) : nettement plus rapide que celui de la bibliothèque image.
            let (w, h) = (u16::try_from(width), u16::try_from(height));
            let (Ok(w), Ok(h)) = (w, h) else { return Err(EngineError::Corrupt("le JPEG est limité à 65 535 px de côté".into())) };
            let mut bytes = Vec::with_capacity(rgb.len() / 6);
            jpeg_encoder::Encoder::new(&mut bytes, quality.clamp(1, 100))
                .encode(&rgb, w, h, jpeg_encoder::ColorType::Rgb)
                .map_err(|e| EngineError::Corrupt(format!("encodage impossible : {e}")))?;
            Ok(bytes)
        }
    }
}

/// Nom du format tel que l'aperçu de l'export le renvoie à l'interface (« jpeg », « webp » ou « png »).
fn kind_of(format: ExportFormat) -> &'static str {
    if format == ExportFormat::Jpeg { "jpeg" } else if format == ExportFormat::Webp { "webp" } else { "png" }
}

/// Résultat d'un encodage en mémoire, pour la fenêtre d'export.
pub struct Encoded {
    pub width: u32,
    pub height: u32,
    /// Poids exact du fichier qui sera écrit.
    pub size: u64,
    /// Image affichable par la webview pour l'aperçu : le fichier lui-même, ou son équivalent PNG
    /// pour le TIFF (qu'aucun navigateur n'affiche, et qui est sans perte de toute façon).
    pub display: Vec<u8>,
}

// ------------------------------------------------------------------ aperçu de l'export

/// Image composée et mise à la taille d'export (RGBA, alpha droit). Calculée une fois, puis réutilisée par l'aperçu
/// tant que le document et la taille ne changent pas : changer de format ou de qualité ne recompose rien.
pub struct Flattened {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn flatten_for_export(doc: &Document, scale: f32) -> Result<Flattened, EngineError> {
    let (width, height, rgba) = flattened(doc, scale, &|_, _| {})?;
    Ok(Flattened { width, height, rgba })
}

impl Flattened {
    /// Pixels du rectangle `[x, y, largeur, hauteur]` (rogné à l'image).
    fn crop(&self, [x, y, w, h]: [u32; 4]) -> (u32, u32, Vec<u8>) {
        let x0 = x.min(self.width.saturating_sub(1));
        let y0 = y.min(self.height.saturating_sub(1));
        let (w, h) = (w.clamp(1, self.width - x0), h.clamp(1, self.height - y0));
        let stride = self.width as usize * 4;
        let mut out = Vec::with_capacity(w as usize * h as usize * 4);
        for row in y0..y0 + h {
            let start = row as usize * stride + x0 as usize * 4;
            out.extend_from_slice(&self.rgba[start..start + w as usize * 4]);
        }
        (w, h, out)
    }

    /// Vue d'ensemble, côté long ≤ `max_side`, encodée pour la webview : dans le format d'export s'il est avec perte
    /// (on voit sa qualité), en PNG rapide sinon. Renvoie (largeur, hauteur, octets, format « jpeg » ou « png »).
    pub fn overview(&self, max_side: u32, format: ExportFormat, quality: u8, lossless: bool) -> Result<(u32, u32, Vec<u8>, &'static str), EngineError> {
        let k = (max_side as f32 / self.width.max(self.height) as f32).min(1.0);
        let (w, h) = (((self.width as f32 * k).round() as u32).max(1), ((self.height as f32 * k).round() as u32).max(1));
        let small = if (w, h) == (self.width, self.height) {
            self.rgba.clone()
        } else {
            // Réduction en alpha prémultiplié, puis retour en alpha droit.
            let mut premultiplied = self.rgba.clone();
            premultiplied.par_chunks_exact_mut(4).for_each(|px| {
                let p = premultiply([px[0], px[1], px[2], px[3]]);
                px.copy_from_slice(&p);
            });
            let mut out = resize_premultiplied(premultiplied, self.width, self.height, w, h)?;
            out.par_chunks_exact_mut(4).for_each(|px| {
                let p = unpremultiply([px[0], px[1], px[2], px[3]]);
                px.copy_from_slice(&p);
            });
            out
        };
        // Format avec perte : on voit sa qualité. Sans perte : JPEG à 95 % si l'image est opaque (dix fois plus léger
        // à transmettre, indiscernable à cette taille), PNG rapide si elle a de la transparence (damier visible).
        Ok(if format.is_lossy(lossless) {
            (w, h, encode(w, h, &small, format, quality, lossless)?, kind_of(format))
        } else if small.par_chunks_exact(4).all(|px| px[3] == 255) {
            (w, h, encode(w, h, &small, ExportFormat::Jpeg, 95, false)?, "jpeg")
        } else {
            (w, h, crate::png_parallel::encode(w, h, &small, 4, 1)?, "png")
        })
    }

    /// Rectangle visible à pleine résolution, encodé comme il le sera dans le fichier (JPEG et WebP avec perte :
    /// leurs défauts de compression sont visibles ; formats sans perte : PNG rapide, pixels identiques).
    pub fn region(&self, rect: [u32; 4], format: ExportFormat, quality: u8, lossless: bool) -> Result<(u32, u32, Vec<u8>, &'static str), EngineError> {
        let (w, h, pixels) = self.crop(rect);
        Ok(if format.is_lossy(lossless) {
            (w, h, encode(w, h, &pixels, format, quality, lossless)?, kind_of(format))
        } else {
            (w, h, crate::png_parallel::encode(w, h, &pixels, 4, 1)?, "png")
        })
    }

    /// Poids estimé du fichier, vite : 12 bandes réparties sur la hauteur (environ un sixième de l'image) sont
    /// encodées, et leur poids extrapolé. Une petite image est encodée entièrement : le poids est alors exact.
    pub fn estimate_size(&self, format: ExportFormat, quality: u8, lossless: bool, metadata: Option<&ImageMetadata>) -> Result<(u64, bool), EngineError> {
        if u64::from(self.width) * u64::from(self.height) <= 2_000_000 {
            return Ok((self.exact_size(format, quality, lossless, metadata)?, true));
        }
        // Les métadonnées pèsent la même chose quelle que soit l'image : mesurées sur une vignette.
        let extra = match metadata {
            Some(meta) if !meta.is_empty() => {
                let (w, h, pixels) = self.crop([0, 0, 16, 16]);
                let plain = encode(w, h, &pixels, format, quality, lossless)?;
                let full = crate::metadata::embed(format, plain.clone(), meta)?;
                (full.len() - plain.len()) as u64
            }
            _ => 0,
        };
        if format == ExportFormat::Png {
            return Ok((crate::png_parallel::estimate(self.width, self.height, &self.rgba, 4, 6, 12)? + extra, false));
        }
        // Bandes alignées sur les blocs de 16 lignes du JPEG (et les macroblocs du WebP) ; le coût des en-têtes
        // (tables, marqueurs), payé par chaque bande, est mesuré sur une vignette de 16 × 16 et retranché.
        const STRIPS: u32 = 8;
        let strip = ((self.height / (STRIPS * 6)) / 16 * 16).max(64).min(self.height / 16 * 16).max(16);
        let header = {
            let (w, h, pixels) = self.crop([0, 0, 16, 16]);
            encode(w, h, &pixels, format, quality, lossless)?.len() as u64
        };
        let sizes: Vec<u64> = (0..STRIPS)
            .into_par_iter()
            .map(|i| {
                let y = (self.height - strip) * i / (STRIPS - 1) / 16 * 16;
                let (w, h, pixels) = self.crop([0, y, self.width, strip]);
                encode(w, h, &pixels, format, quality, lossless).map(|bytes| bytes.len() as u64)
            })
            .collect::<Result<_, EngineError>>()?;
        let data: u64 = sizes.iter().map(|size| size.saturating_sub(header)).sum();
        let sampled = u64::from(strip * STRIPS);
        Ok((data * u64::from(self.height) / sampled.max(1) + header + extra, false))
    }

    /// Poids exact : l'image entière encodée comme à l'export, métadonnées comprises.
    pub fn exact_size(&self, format: ExportFormat, quality: u8, lossless: bool, metadata: Option<&ImageMetadata>) -> Result<u64, EngineError> {
        let bytes = encode(self.width, self.height, &self.rgba, format, quality, lossless)?;
        Ok(with_metadata(format, bytes, metadata)?.len() as u64)
    }
}

pub fn export_encoded(doc: &Document, options: ExportOptions) -> Result<Encoded, EngineError> {
    let (width, height, rgba) = flattened(doc, options.scale, &|_, _| {})?;
    let bytes = encode(width, height, &rgba, options.format, options.quality, options.lossless)?;
    let bytes = with_metadata(options.format, bytes, options.metadata.as_ref())?;
    let size = bytes.len() as u64;
    let display = if options.format == ExportFormat::Tiff { encode(width, height, &rgba, ExportFormat::Png, 100, false)? } else { bytes };
    Ok(Encoded { width, height, size, display })
}

/// Écrit l'export sur disque (écriture atomique : jamais de fichier à moitié écrit).
pub fn export_image(doc: &Document, path: &Path, options: ExportOptions) -> Result<(), EngineError> {
    export_image_with_progress(doc, path, options, &|_, _| {})
}

/// Comme [`export_image`], en tenant l'appelant informé : `progress(part de 0 à 1, étape en cours)`.
pub fn export_image_with_progress(doc: &Document, path: &Path, options: ExportOptions, progress: &dyn Fn(f32, &str)) -> Result<(), EngineError> {
    let (width, height, rgba) = flattened(doc, options.scale, progress)?;
    progress(0.7, "Encodage du fichier");
    let bytes = encode(width, height, &rgba, options.format, options.quality, options.lossless)?;
    let bytes = with_metadata(options.format, bytes, options.metadata.as_ref())?;
    progress(0.95, "Écriture sur le disque");
    write_atomically(path, |file| std::io::Write::write_all(&mut BufWriter::new(file), &bytes).map_err(EngineError::io(path)))?;
    progress(1.0, "Terminé");
    Ok(())
}

// --------------------------------------------------------------------- projet

/// Une tuile à encoder. `scope` : `None` pour le document principal, la clé d'un contenu d'objet dynamique sinon.
enum Job<'a> {
    Pixels(Option<u64>, LayerId, TileCoord, &'a Arc<Tile>),
    Mask(Option<u64>, LayerId, TileCoord, &'a Arc<MaskTile>),
    /// Dernier rendu d'un contenu lié.
    Flat(u64, TileCoord, &'a Arc<Tile>),
}

/// Dossier des tuiles d'un calque dans l'archive : `layers/<id>` (ou `masks/<id>`) pour le document principal,
/// `smart/<clé>/layers/<id>` pour le document d'un contenu d'objet dynamique.
fn tiles_folder(scope: Option<u64>, folder: &str, layer: LayerId) -> String {
    match scope {
        None => format!("{folder}/{layer}"),
        Some(key) => format!("smart/{key}/{folder}/{layer}"),
    }
}

fn flat_folder(key: u64) -> String {
    format!("smart/{key}/flat")
}

fn original_entry(key: u64) -> String {
    format!("smart/{key}/original")
}

fn tile_entry(folder: &str, coord: TileCoord) -> String {
    format!("{folder}/{}_{}.png", coord.x, coord.y)
}

/// Tuiles (pixels et masques) d'un document, pour l'encodage.
fn document_jobs<'a>(doc: &'a Document, scope: Option<u64>, jobs: &mut Vec<Job<'a>>) {
    for layer in doc.nodes() {
        if let Some(pixels) = layer.pixels() {
            jobs.extend(pixels.iter().map(|(c, t)| Job::Pixels(scope, layer.id, c, t)));
        }
        if let Some(mask) = &layer.mask {
            jobs.extend(mask.pixels.iter().map(|(c, t)| Job::Mask(scope, layer.id, c, t)));
        }
    }
}

fn straight_png(tile: &Tile, path: &Path) -> Result<Vec<u8>, EngineError> {
    let straight: Vec<u8> = tile.bytes().chunks_exact(4).flat_map(|px| unpremultiply([px[0], px[1], px[2], px[3]])).collect();
    encode_png(&straight, ExtendedColorType::Rgba8, path)
}

/// Enregistre le projet. L'écriture passe par un fichier temporaire renommé à la
/// fin : une coupure ne laisse jamais un projet à moitié écrit.
pub fn save_project(doc: &Document, path: &Path) -> Result<(), EngineError> {
    let (mut structure, contents) = describe(doc);
    let mut jobs = Vec::new();
    document_jobs(doc, None, &mut jobs);
    let mut originals: Vec<(String, Arc<Vec<u8>>)> = Vec::new();
    for (key, content) in &contents {
        match &content.source {
            SmartSource::Embedded { original: Some(original) } => originals.push((original_entry(*key), Arc::clone(&original.bytes))),
            SmartSource::Embedded { original: None } => document_jobs(&content.document, Some(*key), &mut jobs),
            SmartSource::Linked { .. } => jobs.extend(content.flattened.iter().map(|(c, t)| Job::Flat(*key, c, t))),
        }
    }
    // Liens : chemin relatif au dossier du projet, pour retrouver les fichiers si l'ensemble est déplacé.
    let folder = std::path::absolute(path).ok().and_then(|p| p.parent().map(Path::to_path_buf));
    for content in &mut structure.contents {
        if let (SourceV7::Linked { path: linked, relative, .. }, Some(folder)) = (&mut content.source, &folder) {
            *relative = crate::smart::relative_to(linked, folder);
        }
    }
    // Encodage PNG de toutes les tuiles en parallèle, avant d'ouvrir l'archive.
    let mut encoded: Vec<(String, Vec<u8>)> = jobs
        .par_iter()
        .map(|job| match job {
            Job::Pixels(scope, id, coord, tile) => Ok((tile_entry(&tiles_folder(*scope, "layers", *id), *coord), straight_png(tile, path)?)),
            Job::Mask(scope, id, coord, tile) => Ok((tile_entry(&tiles_folder(*scope, "masks", *id), *coord), encode_png(tile.bytes(), ExtendedColorType::L8, path)?)),
            Job::Flat(key, coord, tile) => Ok((tile_entry(&flat_folder(*key), *coord), straight_png(tile, path)?)),
        })
        .collect::<Result<_, EngineError>>()?;
    encoded.sort_unstable_by(|a, b| a.0.cmp(&b.0));

    let manifest = Manifest {
        format: FORMAT_NAME.into(),
        // Sans objet dynamique, l'ancienne version : le fichier reste lisible par les versions précédentes.
        format_version: if structure.contents.is_empty() { FORMAT_VERSION_WITHOUT_SMART } else { FORMAT_VERSION },
        app_version: env!("CARGO_PKG_VERSION").into(),
    };

    write_atomically(path, |file| {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let mut zip = ZipWriter::new(BufWriter::new(file));
        zip.start_file("manifest.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest)?).map_err(EngineError::io(path))?;
        zip.start_file("document.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(&structure)?).map_err(EngineError::io(path))?;
        for (name, bytes) in &encoded {
            zip.start_file(name.as_str(), options)?;
            zip.write_all(bytes).map_err(EngineError::io(path))?;
        }
        for (name, bytes) in &originals {
            zip.start_file(name.as_str(), options)?;
            zip.write_all(bytes).map_err(EngineError::io(path))?;
        }
        zip.finish()?.flush().map_err(EngineError::io(path))
    })
}

pub fn load_project(path: &Path) -> Result<Document, EngineError> {
    let file = File::open(path).map_err(EngineError::io(path))?;
    load_project_from(BufReader::new(file), path)
}

/// Variante depuis la mémoire (fichier déposé dans la fenêtre).
pub fn load_project_bytes(bytes: &[u8], origin: &Path) -> Result<Document, EngineError> {
    load_project_from(Cursor::new(bytes), origin)
}

fn load_project_from<R: Read + Seek>(reader: R, path: &Path) -> Result<Document, EngineError> {
    let mut zip = ZipArchive::new(reader).map_err(|_| EngineError::NotAProject)?;

    let manifest: Manifest = read_entry(&mut zip, "manifest.json", path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or(EngineError::NotAProject)?;
    if manifest.format != FORMAT_NAME {
        return Err(EngineError::NotAProject);
    }
    let json = read_entry(&mut zip, "document.json", path)?;
    let mut structure: DocumentV2 = match manifest.format_version {
        1 => serde_json::from_slice::<DocumentV1>(&json)?.upgrade(),
        2..=7 => serde_json::from_slice(&json)?,
        found => return Err(EngineError::UnsupportedVersion { found, supported: FORMAT_VERSION }),
    };
    // Dossier du projet, pour les liens relatifs (un fichier déposé n'a pas de dossier connu).
    let folder = path.is_absolute().then(|| path.parent().map(Path::to_path_buf)).flatten();
    let mut source = ZipSource { zip: &mut zip, path };
    let contents = std::mem::take(&mut structure.contents);
    let (contents, stale) = load_contents(contents, &mut source, folder.as_deref())?;
    build_document(structure, None, &contents, &stale, &mut source)
}

/// D'où viennent les tuiles d'un document en cours de reconstruction : un projet, ou une copie de secours.
trait TileSource {
    fn pixels(&mut self, scope: Option<u64>, layer: LayerId, tiles: &[[i32; 2]]) -> Result<TileStore, EngineError>;
    fn mask(&mut self, scope: Option<u64>, layer: LayerId, mask: &MaskV2) -> Result<MaskStore, EngineError>;
    /// Dernier rendu d'un contenu lié.
    fn flat(&mut self, key: u64, tiles: &[[i32; 2]]) -> Result<TileStore, EngineError>;
    /// Image d'origine d'un contenu placé.
    fn original(&mut self, key: u64) -> Result<Vec<u8>, EngineError>;
}

struct ZipSource<'a, R> {
    zip: &'a mut ZipArchive<R>,
    path: &'a Path,
}

fn premultiplied_rgba(image: image::DynamicImage) -> Result<Vec<u8>, EngineError> {
    let mut rgba = image.into_rgba8().into_raw();
    for px in rgba.chunks_exact_mut(4) {
        let p = premultiply([px[0], px[1], px[2], px[3]]);
        px.copy_from_slice(&p);
    }
    Ok(rgba)
}

impl<R: Read + Seek> TileSource for ZipSource<'_, R> {
    fn pixels(&mut self, scope: Option<u64>, layer: LayerId, tiles: &[[i32; 2]]) -> Result<TileStore, EngineError> {
        let mut pixels = TileStore::new();
        load_tiles(self.zip, self.path, &tiles_folder(scope, "layers", layer), tiles, &mut pixels, premultiplied_rgba)?;
        Ok(pixels)
    }

    fn mask(&mut self, scope: Option<u64>, layer: LayerId, mask: &MaskV2) -> Result<MaskStore, EngineError> {
        let mut pixels = MaskStore::with_background([mask.background]);
        load_tiles(self.zip, self.path, &tiles_folder(scope, "masks", layer), &mask.tiles, &mut pixels, |image| Ok(image.into_luma8().into_raw()))?;
        Ok(pixels)
    }

    fn flat(&mut self, key: u64, tiles: &[[i32; 2]]) -> Result<TileStore, EngineError> {
        let mut pixels = TileStore::new();
        load_tiles(self.zip, self.path, &flat_folder(key), tiles, &mut pixels, premultiplied_rgba)?;
        Ok(pixels)
    }

    fn original(&mut self, key: u64) -> Result<Vec<u8>, EngineError> {
        read_entry(self.zip, &original_entry(key), self.path)
    }
}

/// Reconstruit les contenus d'objets dynamiques, les plus imbriqués d'abord (un contenu peut contenir des objets
/// dynamiques désignant d'autres contenus de la table). Chaque contenu reçoit un identifiant neuf ; ses instances le
/// partagent. Retourne aussi les clés des contenus liés dont le fichier a changé depuis l'enregistrement : le rendu
/// enregistré de leurs instances est périmé, il sera refait.
#[allow(clippy::type_complexity)]
fn load_contents(contents: Vec<ContentV7>, source: &mut dyn TileSource, folder: Option<&Path>) -> Result<(HashMap<u64, Arc<SmartContent>>, HashSet<u64>), EngineError> {
    let references = |content: &ContentV7| -> Vec<u64> {
        match &content.source {
            SourceV7::Embedded { document } => document.layers.iter().filter_map(|l| if let KindV2::Smart { content, .. } = l.kind { Some(content) } else { None }).collect(),
            SourceV7::Original { .. } | SourceV7::Linked { .. } => Vec::new(),
        }
    };
    // Ordre de construction : parcours en profondeur, chaque contenu après ceux qu'il utilise.
    let graph: HashMap<u64, Vec<u64>> = contents.iter().map(|c| (c.key, references(c))).collect();
    let mut order = Vec::with_capacity(contents.len());
    let mut state: HashMap<u64, bool> = HashMap::new(); // faux : en cours ; vrai : fait
    for start in contents.iter().map(|c| c.key) {
        let mut stack = vec![(start, false)];
        while let Some((key, children_done)) = stack.pop() {
            if children_done {
                state.insert(key, true);
                order.push(key);
                continue;
            }
            match state.get(&key) {
                Some(true) => continue,
                Some(false) => return Err(EngineError::Corrupt("objets dynamiques imbriqués en boucle".into())),
                None => {}
            }
            state.insert(key, false);
            stack.push((key, true));
            for child in graph.get(&key).ok_or_else(|| EngineError::Corrupt(format!("contenu d'objet dynamique {key} absent")))? {
                if state.get(child) != Some(&true) {
                    stack.push((*child, false));
                }
            }
        }
    }
    let mut pending: HashMap<u64, ContentV7> = contents.into_iter().map(|c| (c.key, c)).collect();
    let (mut built, mut stale) = (HashMap::new(), HashSet::new());
    for key in order {
        let Some(content) = pending.remove(&key) else { continue };
        let id = crate::smart::new_content_id();
        let smart = match content.source {
            SourceV7::Embedded { document } => {
                let document = build_document(*document, Some(key), &built, &stale, source)?;
                SmartContent::new(id, content.name, Arc::new(document), SmartSource::Embedded { original: None })
            }
            SourceV7::Original { file_name } => {
                let bytes = source.original(key)?;
                let document = import_image_bytes(&bytes, Path::new(&file_name))?;
                SmartContent::new(id, content.name, Arc::new(document), SmartSource::Embedded { original: Some(OriginalFile { name: file_name, bytes: Arc::new(bytes) }) })
            }
            SourceV7::Linked { path, relative, modified, tiles } => {
                // Le chemin absolu d'abord, puis le relatif (projet et fichiers liés déplacés ensemble).
                let found = [Some(path.clone()), relative.and_then(|r| folder.map(|f| f.join(r)))].into_iter().flatten().find(|p| p.is_file());
                match found.map(|p| (crate::smart::read_file(&p), p)) {
                    Some((Ok((document, _)), found)) => {
                        let now = crate::smart::modified_time(&found);
                        if now != modified {
                            stale.insert(key);
                        }
                        let found = std::path::absolute(&found).unwrap_or(found);
                        SmartContent::new(id, content.name, Arc::new(document), SmartSource::Linked { path: found, modified: now, missing: false })
                    }
                    // Introuvable ou illisible : le dernier rendu enregistré tient lieu de contenu.
                    _ => {
                        let flat = source.flat(key, &tiles)?;
                        let mut document = Document::new(content.name.clone(), content.width, content.height, 72.0)?;
                        document.add_pixel_layer(content.name.clone(), flat.clone(), None, 0)?;
                        SmartContent::with_flattened(id, content.name, Arc::new(document), SmartSource::Linked { path, modified, missing: true }, Arc::new(flat))
                    }
                }
            }
        };
        built.insert(key, Arc::new(smart));
    }
    Ok((built, stale))
}

/// Reconstruit un document depuis sa description. `scope` : `None` pour le document principal, la clé du contenu
/// sinon ; `contents` : les contenus d'objets dynamiques déjà reconstruits ; `stale` : ceux dont le rendu enregistré
/// est périmé.
fn build_document(
    structure: DocumentV2,
    scope: Option<u64>,
    contents: &HashMap<u64, Arc<SmartContent>>,
    stale: &HashSet<u64>,
    source: &mut dyn TileSource,
) -> Result<Document, EngineError> {
    let mut doc = Document::new(structure.name, structure.width, structure.height, structure.dpi)?;
    doc.guides = structure.guides;
    doc.guides_locked = structure.guides_locked;
    doc.paths = structure.paths;
    for layer in structure.layers {
        let mask = match layer.mask {
            None => None,
            Some(m) => Some(Mask { pixels: Arc::new(source.mask(scope, layer.id, &m)?), enabled: m.enabled }),
        };
        let kind = match layer.kind {
            KindV2::Group { children, expanded } => LayerKind::Group { children, expanded },
            KindV2::Pixel { tiles } => LayerKind::Pixel { pixels: Arc::new(source.pixels(scope, layer.id, &tiles)?) },
            KindV2::Adjustment { adjustment } => LayerKind::Adjustment { adjustment },
            KindV2::Shape { shape, tiles } => LayerKind::Shape { shape, raster: Arc::new(source.pixels(scope, layer.id, &tiles)?) },
            KindV2::Text { text, transform, tiles } => {
                let text = text.upgraded();
                LayerKind::Text { text, transform, raster: Arc::new(source.pixels(scope, layer.id, &tiles)?) }
            }
            KindV2::Smart { content: key, transform, tiles } => {
                let content = contents.get(&key).cloned().ok_or_else(|| EngineError::Corrupt(format!("contenu d'objet dynamique {key} absent")))?;
                let raster = if stale.contains(&key) { content.render(&transform, crate::transform::Quality::Final)? } else { Arc::new(source.pixels(scope, layer.id, &tiles)?) };
                LayerKind::Smart { content, transform, raster }
            }
        };
        let props = LayerProps {
            name: layer.name,
            visible: layer.visible,
            opacity: layer.opacity.clamp(0.0, 1.0),
            fill: layer.fill.clamp(0.0, 1.0),
            blend: layer.blend,
            locks: layer.locks,
            clipped: layer.clipped,
        };
        let offset = (layer.offset[0], layer.offset[1]);
        doc.restore(Layer { id: layer.id, parent: layer.parent, label: layer.label, offset, props, mask, effects: layer.effects, kind }, structure.root.clone());
    }
    doc.validate()?;
    Ok(doc)
}

// ------------------------------------------------------------------ récupération (sauvegarde automatique)

const RECOVERY_FORMAT: &str = "grafism-recuperation";

/// Historique complet d'un document : ses états (libellé, document) et l'état courant.
pub struct History {
    pub states: Vec<(String, Document)>,
    pub cursor: usize,
}

#[derive(Serialize, Deserialize)]
struct RecoveryState {
    label: String,
    document: DocumentV2,
    /// Pour chaque calque, l'index de chaque tuile (dans l'ordre des coordonnées de sa description).
    pixels: HashMap<LayerId, Vec<u32>>,
    masks: HashMap<LayerId, Vec<u32>>,
    /// Tuiles des contenus d'objets dynamiques, par clé de contenu. Ajout compatible : absent sans objet dynamique.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    smart: HashMap<u64, RecoveryContent>,
}

/// Tuiles d'un contenu d'objet dynamique dans une copie de secours, mêmes index que pour le document.
#[derive(Serialize, Deserialize, Default)]
struct RecoveryContent {
    #[serde(default)]
    pixels: HashMap<LayerId, Vec<u32>>,
    #[serde(default)]
    masks: HashMap<LayerId, Vec<u32>>,
    /// Dernier rendu d'un contenu lié.
    #[serde(default)]
    flat: Vec<u32>,
    /// Image d'origine d'un contenu placé : index du fichier `tuiles/o<index>`.
    #[serde(default)]
    original: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct Recovery {
    format: String,
    app_version: String,
    /// Projet d'origine, s'il avait déjà été enregistré.
    project: Option<std::path::PathBuf>,
    cursor: usize,
    states: Vec<RecoveryState>,
}

/// Tuiles uniques d'une copie de secours : chacune n'est écrite qu'une fois, quel que soit le nombre d'états (et de
/// contenus d'objets dynamiques) qui la partagent.
#[derive(Default)]
struct RecoveryTiles {
    pixel_index: HashMap<usize, u32>,
    mask_index: HashMap<usize, u32>,
    original_index: HashMap<usize, u32>,
    pixel_tiles: Vec<Arc<Tile>>,
    mask_tiles: Vec<Arc<MaskTile>>,
    originals: Vec<Arc<Vec<u8>>>,
}

impl RecoveryTiles {
    fn pixels(&mut self, store: &TileStore) -> Vec<u32> {
        store
            .sorted_coords()
            .iter()
            .filter_map(|coord| store.get(*coord))
            .map(|tile| {
                *self.pixel_index.entry(Arc::as_ptr(tile) as usize).or_insert_with(|| {
                    self.pixel_tiles.push(Arc::clone(tile));
                    (self.pixel_tiles.len() - 1) as u32
                })
            })
            .collect()
    }

    fn masks(&mut self, store: &MaskStore) -> Vec<u32> {
        store
            .sorted_coords()
            .iter()
            .filter_map(|coord| store.get(*coord))
            .map(|tile| {
                *self.mask_index.entry(Arc::as_ptr(tile) as usize).or_insert_with(|| {
                    self.mask_tiles.push(Arc::clone(tile));
                    (self.mask_tiles.len() - 1) as u32
                })
            })
            .collect()
    }

    /// Index des tuiles de chaque calque d'un document (pixels, masques).
    #[allow(clippy::type_complexity)]
    fn document(&mut self, doc: &Document) -> (HashMap<LayerId, Vec<u32>>, HashMap<LayerId, Vec<u32>>) {
        let (mut pixels, mut masks) = (HashMap::new(), HashMap::new());
        for layer in doc.nodes() {
            if let Some(store) = layer.pixels() {
                pixels.insert(layer.id, self.pixels(store));
            }
            if let Some(mask) = &layer.mask {
                masks.insert(layer.id, self.masks(&mask.pixels));
            }
        }
        (pixels, masks)
    }

    fn original(&mut self, bytes: &Arc<Vec<u8>>) -> u32 {
        *self.original_index.entry(Arc::as_ptr(bytes) as usize).or_insert_with(|| {
            self.originals.push(Arc::clone(bytes));
            (self.originals.len() - 1) as u32
        })
    }
}

/// Copie de secours d'un document, historique compris. Les tuiles communes à plusieurs états (tout ce qu'une
/// action n'a pas touché) ne sont écrites qu'une fois : l'historique ne coûte presque que ce qui a changé.
/// `progress` reçoit l'avancement (0 à 1). Écriture atomique : une copie précédente n'est jamais abîmée.
pub fn save_recovery(history: &History, project: Option<&Path>, path: &Path, progress: &(dyn Fn(f32) + Sync)) -> Result<(), EngineError> {
    let mut unique = RecoveryTiles::default();
    let mut states = Vec::with_capacity(history.states.len());
    for (label, doc) in &history.states {
        let (structure, contents) = describe(doc);
        let (pixels, masks) = unique.document(doc);
        let mut smart = HashMap::new();
        for (key, content) in &contents {
            let entry = match &content.source {
                SmartSource::Embedded { original: Some(original) } => RecoveryContent { original: Some(unique.original(&original.bytes)), ..Default::default() },
                SmartSource::Embedded { original: None } => {
                    let (pixels, masks) = unique.document(&content.document);
                    RecoveryContent { pixels, masks, ..Default::default() }
                }
                SmartSource::Linked { .. } => RecoveryContent { flat: unique.pixels(&content.flattened), ..Default::default() },
            };
            smart.insert(*key, entry);
        }
        states.push(RecoveryState { label: label.clone(), document: structure, pixels, masks, smart });
    }
    progress(0.05);
    let RecoveryTiles { pixel_tiles, mask_tiles, originals, .. } = unique;
    // Encodage PNG des tuiles uniques, en parallèle, par lots pour tenir l'avancement à jour.
    let total = (pixel_tiles.len() + mask_tiles.len()).max(1);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let tick = || {
        let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if n % 16 == 0 {
            progress(0.05 + 0.85 * n as f32 / total as f32);
        }
    };
    let encoded_pixels: Vec<Vec<u8>> = pixel_tiles
        .par_iter()
        .map(|tile| {
            let png = straight_png(tile, path);
            tick();
            png
        })
        .collect::<Result<_, EngineError>>()?;
    let encoded_masks: Vec<Vec<u8>> = mask_tiles
        .par_iter()
        .map(|tile| {
            let png = encode_png(tile.bytes(), ExtendedColorType::L8, path);
            tick();
            png
        })
        .collect::<Result<_, EngineError>>()?;
    let recovery = Recovery {
        format: RECOVERY_FORMAT.into(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        project: project.map(Path::to_path_buf),
        cursor: history.cursor,
        states,
    };
    progress(0.92);
    write_atomically(path, |file| {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let mut zip = ZipWriter::new(BufWriter::new(file));
        zip.start_file("recuperation.json", options)?;
        zip.write_all(&serde_json::to_vec(&recovery)?).map_err(EngineError::io(path))?;
        for (index, bytes) in encoded_pixels.iter().enumerate() {
            zip.start_file(format!("tuiles/p{index}.png"), options)?;
            zip.write_all(bytes).map_err(EngineError::io(path))?;
        }
        for (index, bytes) in encoded_masks.iter().enumerate() {
            zip.start_file(format!("tuiles/m{index}.png"), options)?;
            zip.write_all(bytes).map_err(EngineError::io(path))?;
        }
        for (index, bytes) in originals.iter().enumerate() {
            zip.start_file(format!("tuiles/o{index}"), options)?;
            zip.write_all(bytes).map_err(EngineError::io(path))?;
        }
        zip.finish()?.flush().map_err(EngineError::io(path))
    })?;
    progress(1.0);
    Ok(())
}

/// Tuiles d'un état de copie de secours, puisées dans les tuiles uniques décodées.
struct RecoverySource<'a> {
    pixel_tiles: &'a [Arc<Tile>],
    mask_tiles: &'a [Arc<MaskTile>],
    originals: &'a [Vec<u8>],
    pixels: HashMap<LayerId, Vec<u32>>,
    masks: HashMap<LayerId, Vec<u32>>,
    smart: HashMap<u64, RecoveryContent>,
}

fn corrupt_recovery() -> EngineError {
    EngineError::Corrupt("copie de secours incomplète".into())
}

impl RecoverySource<'_> {
    fn indexes(&self, scope: Option<u64>, layer: LayerId, mask: bool) -> &[u32] {
        let map = match scope {
            None => Some(if mask { &self.masks } else { &self.pixels }),
            Some(key) => self.smart.get(&key).map(|c| if mask { &c.masks } else { &c.pixels }),
        };
        map.and_then(|m| m.get(&layer)).map(Vec::as_slice).unwrap_or(&[])
    }

    fn rgba(&self, coords: &[[i32; 2]], ids: &[u32]) -> Result<TileStore, EngineError> {
        let mut store = TileStore::new();
        for (coord, id) in coords.iter().zip(ids) {
            store.restore(TileCoord::new(coord[0], coord[1]), Some(Arc::clone(self.pixel_tiles.get(*id as usize).ok_or_else(corrupt_recovery)?)));
        }
        Ok(store)
    }
}

impl TileSource for RecoverySource<'_> {
    fn pixels(&mut self, scope: Option<u64>, layer: LayerId, tiles: &[[i32; 2]]) -> Result<TileStore, EngineError> {
        self.rgba(tiles, self.indexes(scope, layer, false))
    }

    fn mask(&mut self, scope: Option<u64>, layer: LayerId, mask: &MaskV2) -> Result<MaskStore, EngineError> {
        let mut store = MaskStore::with_background([mask.background]);
        for (coord, id) in mask.tiles.iter().zip(self.indexes(scope, layer, true)) {
            store.restore(TileCoord::new(coord[0], coord[1]), Some(Arc::clone(self.mask_tiles.get(*id as usize).ok_or_else(corrupt_recovery)?)));
        }
        Ok(store)
    }

    fn flat(&mut self, key: u64, tiles: &[[i32; 2]]) -> Result<TileStore, EngineError> {
        let ids = self.smart.get(&key).map(|c| c.flat.as_slice()).unwrap_or(&[]);
        self.rgba(tiles, ids)
    }

    fn original(&mut self, key: u64) -> Result<Vec<u8>, EngineError> {
        let index = self.smart.get(&key).and_then(|c| c.original).ok_or_else(corrupt_recovery)?;
        self.originals.get(index as usize).cloned().ok_or_else(corrupt_recovery)
    }
}

/// Relit une copie de secours : l'historique complet, et le projet d'origine s'il y en avait un.
pub fn load_recovery(path: &Path) -> Result<(History, Option<std::path::PathBuf>), EngineError> {
    let file = File::open(path).map_err(EngineError::io(path))?;
    let mut zip = ZipArchive::new(BufReader::new(file)).map_err(|_| EngineError::NotAProject)?;
    let recovery: Recovery = serde_json::from_slice(&read_entry(&mut zip, "recuperation.json", path)?)?;
    if recovery.format != RECOVERY_FORMAT {
        return Err(EngineError::NotAProject);
    }
    // Tuiles uniques, décodées une fois et partagées entre les états (comme en mémoire avant la fermeture).
    let read_all = |zip: &mut ZipArchive<BufReader<File>>, prefix: &str, suffix: &str| -> Vec<Vec<u8>> {
        let mut raw = Vec::new();
        for index in 0.. {
            match read_entry(zip, &format!("tuiles/{prefix}{index}{suffix}"), path) {
                Ok(bytes) => raw.push(bytes),
                Err(_) => break,
            }
        }
        raw
    };
    let raw_pixels = read_all(&mut zip, "p", ".png");
    let raw_masks = read_all(&mut zip, "m", ".png");
    let originals = read_all(&mut zip, "o", "");
    let decode = |bytes: &Vec<u8>| image::load(Cursor::new(bytes), ImageFormat::Png).map_err(|source| EngineError::Image { path: path.to_path_buf(), source });
    let pixel_tiles: Vec<Arc<Tile>> = raw_pixels
        .par_iter()
        .map(|bytes| {
            let rgba = premultiplied_rgba(decode(bytes)?)?;
            let mut tile = TileBuf::<4>::filled([0; 4]);
            tile.bytes_mut().copy_from_slice(&rgba);
            Ok(Arc::new(tile))
        })
        .collect::<Result<_, EngineError>>()?;
    let mask_tiles: Vec<Arc<MaskTile>> = raw_masks
        .par_iter()
        .map(|bytes| {
            let mut tile = TileBuf::<1>::filled([0]);
            tile.bytes_mut().copy_from_slice(&decode(bytes)?.into_luma8().into_raw());
            Ok(Arc::new(tile))
        })
        .collect::<Result<_, EngineError>>()?;
    let folder = recovery.project.as_deref().and_then(Path::parent).map(Path::to_path_buf);
    let mut states = Vec::with_capacity(recovery.states.len());
    for state in recovery.states {
        let mut structure = state.document;
        let mut source = RecoverySource { pixel_tiles: &pixel_tiles, mask_tiles: &mask_tiles, originals: &originals, pixels: state.pixels, masks: state.masks, smart: state.smart };
        let contents = std::mem::take(&mut structure.contents);
        let (contents, stale) = load_contents(contents, &mut source, folder.as_deref())?;
        states.push((state.label, build_document(structure, None, &contents, &stale, &mut source)?));
    }
    if states.is_empty() {
        return Err(corrupt_recovery());
    }
    let cursor = recovery.cursor.min(states.len() - 1);
    Ok((History { states, cursor }, recovery.project))
}

/// Lit les tuiles d'un calque (dossier `folder` de l'archive) : lecture séquentielle de l'archive, décodage en parallèle.
fn load_tiles<const C: usize, R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    path: &Path,
    folder: &str,
    tiles: &[[i32; 2]],
    store: &mut Store<C>,
    convert: impl Fn(image::DynamicImage) -> Result<Vec<u8>, EngineError> + Sync,
) -> Result<(), EngineError> {
    let mut raw = Vec::with_capacity(tiles.len());
    for [x, y] in tiles {
        if x.abs() > MAX_TILE_DISTANCE || y.abs() > MAX_TILE_DISTANCE {
            return Err(EngineError::Corrupt(format!("tuile ({x}, {y}) invraisemblable")));
        }
        let coord = TileCoord::new(*x, *y);
        raw.push((coord, read_entry(zip, &tile_entry(folder, coord), path)?));
    }
    let decoded: Vec<(TileCoord, TileBuf<C>)> = raw
        .par_iter()
        .map(|(coord, bytes)| {
            let image = image::load(Cursor::new(bytes), ImageFormat::Png)
                .map_err(|source| EngineError::Image { path: path.to_path_buf(), source })?;
            if (image.width(), image.height()) != (TILE_SIZE, TILE_SIZE) {
                return Err(EngineError::Corrupt("tuile de taille inattendue".into()));
            }
            let mut tile = TileBuf::<C>::filled([0; C]);
            tile.bytes_mut().copy_from_slice(&convert(image)?);
            Ok((*coord, tile))
        })
        .collect::<Result<_, EngineError>>()?;
    for (coord, tile) in decoded {
        store.insert(coord, tile);
    }
    Ok(())
}


fn encode_png(bytes: &[u8], color: ExtendedColorType, path: &Path) -> Result<Vec<u8>, EngineError> {
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive)
        .write_image(bytes, TILE_SIZE, TILE_SIZE, color)
        .map_err(|source| EngineError::Image { path: path.to_path_buf(), source })?;
    Ok(out)
}

fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str, path: &Path) -> Result<Vec<u8>, EngineError> {
    let mut entry = zip.by_name(name)?;
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes).map_err(EngineError::io(path))?;
    Ok(bytes)
}

/// Écrit dans `<path>.tmp` puis renomme. Le fichier temporaire est supprimé en cas d'échec.
fn write_atomically(path: &Path, write: impl FnOnce(File) -> Result<(), EngineError>) -> Result<(), EngineError> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = Path::new(&tmp);
    let result = File::create(tmp)
        .map_err(EngineError::io(tmp))
        .and_then(write)
        .and_then(|()| std::fs::rename(tmp, path).map_err(EngineError::io(path)));
    if result.is_err() {
        // Nettoyage au mieux : l'erreur d'origine est celle qui compte.
        let _ = std::fs::remove_file(tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_export_preview_estimates_the_file_size_closely() {
        let mut doc = Document::with_background("x", 3000, 2000, 72.0, crate::document::Background::White).expect("document");
        let mut rgba = vec![0u8; 3000 * 2000 * 4];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let (x, y) = ((i % 3000) as u32, (i / 3000) as u32);
            let n = (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)) >> 27;
            px.copy_from_slice(&[(x / 12) as u8 ^ n as u8, (y / 8) as u8, ((x + y) / 20) as u8, 255]);
        }
        doc.add_pixel_layer("motif", TileStore::from_premultiplied(3000, 2000, &rgba).expect("pixels"), None, 1).expect("calque");
        let flat = flatten_for_export(&doc, 1.0).expect("composition");
        for format in [ExportFormat::Png, ExportFormat::Jpeg] {
            let (estimate, exact) = flat.estimate_size(format, 85, false, None).expect("estimation");
            let real = flat.exact_size(format, 85, false, None).expect("poids");
            let gap = (estimate as f64 - real as f64).abs() / real as f64;
            assert!(!exact && gap < 0.15, "{format:?} : estimé {estimate}, réel {real} ({:.1} %)", gap * 100.0);
        }
        // La zone visible, encodée : dimensions demandées, image lisible.
        let (w, h, png, kind) = flat.region([2900, 1950, 400, 300], ExportFormat::Png, 85, false).expect("zone");
        assert_eq!((w, h, kind), (100, 50, "png"), "rognée au bord de l'image");
        let decoded = image::load_from_memory(&png).expect("png");
        assert_eq!((decoded.width(), decoded.height()), (100, 50));
        let (ow, oh, _, _) = flat.overview(600, ExportFormat::Jpeg, 85, false).expect("vue d'ensemble");
        assert_eq!((ow, oh), (600, 400));
    }

    #[test]
    fn a_small_preview_keeps_the_proportions_and_the_colors() {
        let mut doc = Document::with_background("x", 4000, 2000, 72.0, crate::document::Background::White).expect("document");
        let layer = doc.add_pixel_layer("rouge", TileStore::new(), None, 1).expect("calque");
        doc.pixels_mut(layer).expect("pixels").fill(2000, 2000, [255, 0, 0, 255]);
        let png = preview_png(&doc, 320).expect("aperçu");
        let image = image::load_from_memory(&png).expect("png lisible").into_rgb8();
        assert_eq!(image.dimensions(), (320, 160), "côté long à 320 px, proportions gardées");
        assert_eq!((image.get_pixel(40, 80).0, image.get_pixel(280, 80).0), ([255, 0, 0], [255, 255, 255]));
    }

    use super::*;
    use crate::document::Background;

    const ORIGIN: TileCoord = TileCoord::new(0, 0);

    /// Document exerçant tout le format : groupe, masque, écrêtage, mode, verrous.
    fn sample_document() -> Document {
        let mut doc = Document::with_background("Essai", 700, 300, 300.0, Background::White).expect("document");
        let group = doc.add_group("Groupe 1", None, 1).expect("groupe");
        doc.set_label(group, Some(crate::document::LayerLabel::Blue)).expect("étiquette");
        let mut pixels = TileStore::new();
        pixels.get_mut(TileCoord::new(2, 1)).set_pixel(10, 10, [40, 20, 10, 60]);
        pixels.get_mut(TileCoord::new(-3, 1)).set_pixel(11, 10, [0, 0, 255, 255]); // contenu hors cadre
        let base = doc.add_pixel_layer("Retouches", pixels, Some(group), 0).expect("calque");
        doc.translate_layers(&[base], -41, 1200).expect("position");
        doc.add_mask(base, true).expect("masque");
        doc.mask_mut(base).expect("masque").get_mut(ORIGIN).set_pixel(4, 4, [17]);
        let clipped = doc.add_pixel_layer("Écrêté", TileStore::new(), Some(group), 1).expect("calque");
        doc.edit_props(clipped, |p| {
            p.clipped = true;
            p.blend = BlendMode::SoftLight;
            p.opacity = 0.42;
            p.fill = 0.9;
            p.locks.transparency = true;
        })
        .expect("propriétés");
        doc.edit_props(group, |p| p.visible = false).expect("propriétés");
        doc.guides.push(Guide { orientation: crate::document::Orientation::Vertical, position: 350.0 });
        let text = crate::text::TextData { content: "Été".into(), tracking: 50.0, italic: true, ..Default::default() };
        doc.add_text_layer(text, (20, 30), None, 2).expect("texte");
        doc.add_adjustment_layer(crate::adjust::Adjustment::HueSaturation { hue: 40.0, saturation: -20.0, lightness: 5.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 30.0, range_width: 45.0 }, None, 3, true).expect("réglage");
        let rounded = crate::shape::ShapeData { kind: crate::shape::ShapeKind::Rect, width: 120.0, height: 80.0, radius: 16.0, fill: Some([10, 120, 240, 255]), stroke: Some([255, 255, 255, 200]), stroke_width: 3.0, sides: 3, transform: Default::default() };
        let shape = doc.add_shape_layer(rounded, (60, 70), None, 4).expect("forme");
        let glow = crate::effects::LayerEffect::OuterGlow { enabled: true, color: [255, 200, 0], opacity: 0.8, size: 14.0, blend: crate::blend::BlendMode::Normal };
        doc.set_effects(shape, vec![glow]).expect("effets");
        doc
    }

    #[test]
    fn project_roundtrip_preserves_tree_properties_pixels_and_masks() {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("essai.iep");
        let doc = sample_document();
        save_project(&doc, &path).expect("enregistrement");
        assert!(!dir.path().join("essai.iep.tmp").exists(), "pas de fichier temporaire résiduel");

        let loaded = load_project(&path).expect("chargement");
        assert_eq!((loaded.width(), loaded.height(), loaded.dpi, loaded.name.as_str()), (700, 300, 300.0, "Essai"));
        assert!(doc.same_structure(&loaded), "arbre et propriétés identiques");
        assert_eq!(doc.guides, loaded.guides);
        for (_, layer) in doc.walk_top_down() {
            let other = loaded.layer(layer.id).expect("calque relu");
            assert_eq!(layer.offset, other.offset);
            assert_eq!(layer.text(), other.text(), "le texte reste éditable après rechargement");
            assert_eq!(layer.adjustment(), other.adjustment());
            assert_eq!(layer.shape(), other.shape(), "la forme reste réglable après rechargement");
            assert_eq!(layer.effects, other.effects, "les effets de calque sont conservés");
            assert_eq!(layer.label, other.label, "la couleur d'étiquette est conservée");
            if let (Some(a), Some(b)) = (layer.pixels(), other.pixels()) {
                assert_eq!(a.sorted_coords(), b.sorted_coords(), "tuiles hors cadre comprises");
                for coord in a.sorted_coords() {
                    assert_eq!(a.get(coord).map(|t| t.bytes()), b.get(coord).map(|t| t.bytes()));
                }
            }
            if let (Some(a), Some(b)) = (&layer.mask, &other.mask) {
                assert_eq!(a.pixels.background(), b.pixels.background());
                assert_eq!(a.pixels.get(ORIGIN).map(|t| t.pixel(4, 4)), b.pixels.get(ORIGIN).map(|t| t.pixel(4, 4)));
            }
        }
    }

    fn write_archive(path: &Path, entries: &[(&str, Vec<u8>)]) {
        let mut zip = ZipWriter::new(File::create(path).expect("fichier"));
        for (name, bytes) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).expect("entrée");
            zip.write_all(bytes).expect("écriture");
        }
        zip.finish().expect("fin d'archive");
    }

    fn manifest(version: u32) -> Vec<u8> {
        format!(r#"{{"format":"{FORMAT_NAME}","format_version":{version},"app_version":"0.1.0"}}"#).into_bytes()
    }

    #[test]
    fn version_1_projects_are_migrated() {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("ancien.iep");
        let tile = encode_png(Tile::filled([255, 0, 0, 255]).bytes(), ExtendedColorType::Rgba8, &path).expect("png");
        let structure = br#"{"name":"Ancien","width":256,"height":256,"dpi":72.0,
            "layers":[{"id":1,"name":"Arriere-plan","visible":true,"tiles":[[0,0]]}]}"#;
        write_archive(&path, &[("manifest.json", manifest(1)), ("document.json", structure.to_vec()), ("layers/1/0_0.png", tile)]);

        let doc = load_project(&path).expect("migration");
        let layer = doc.layer(1).expect("calque");
        assert_eq!((layer.props.opacity, layer.props.blend, layer.parent), (1.0, BlendMode::Normal, None));
        assert_eq!(layer.pixels().expect("pixels").get(ORIGIN).expect("tuile").pixel(0, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn newer_format_version_is_refused() {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("futur.iep");
        write_archive(&path, &[("manifest.json", manifest(99)), ("document.json", b"{}".to_vec())]);
        assert!(matches!(load_project(&path), Err(EngineError::UnsupportedVersion { found: 99, .. })));
    }

    #[test]
    fn inconsistent_tree_is_refused() {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("casse.iep");
        let structure = br#"{"name":"x","width":256,"height":256,"dpi":72.0,"root":[1,7],"layers":[
            {"id":1,"parent":null,"name":"a","visible":true,"opacity":1.0,"fill":1.0,"blend":"normal",
             "locks":{"transparency":false,"pixels":false,"position":false,"all":false},
             "clipped":false,"mask":null,"kind":"pixel","tiles":[]}]}"#;
        write_archive(&path, &[("manifest.json", manifest(2)), ("document.json", structure.to_vec())]);
        assert!(matches!(load_project(&path), Err(EngineError::Corrupt(_))));
    }

    #[test]
    fn arbitrary_file_is_not_a_project() {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("note.iep");
        std::fs::write(&path, b"bonjour").expect("écriture");
        assert!(matches!(load_project(&path), Err(EngineError::NotAProject)));
    }

    #[test]
    fn exports_in_every_format_at_any_scale() {
        let mut doc = Document::with_background("x", 200, 100, 72.0, Background::Transparent).expect("document");
        let layer = doc.walk_top_down().last().expect("calque").1.id;
        doc.set_selection(Some(crate::selection::Shape::Rect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 }.rasterize(200, 100)));
        doc.fill_layer(layer, [200, 40, 40]).expect("moitié rouge");
        doc.set_selection(None);

        for (format, magic) in [(ExportFormat::Png, &b"\x89PNG"[..]), (ExportFormat::Jpeg, &b"\xFF\xD8"[..]), (ExportFormat::Webp, &b"RIFF"[..]), (ExportFormat::Tiff, &b"II"[..])] {
            let encoded = export_encoded(&doc, ExportOptions { format, quality: 85, lossless: false, scale: 0.5, metadata: None }).expect("export");
            assert_eq!((encoded.width, encoded.height), (100, 50), "{format:?} : moitié de la taille");
            assert!(encoded.size > 0);
            let shown = image::load_from_memory(&encoded.display).expect("aperçu lisible par un navigateur").into_rgba8();
            assert_eq!(shown.dimensions(), (100, 50));
            let (left, right) = (shown.get_pixel(20, 25).0, shown.get_pixel(80, 25).0);
            assert!(left[0] > 180 && left[1] < 70, "{format:?} : {left:?}");
            // La moitié transparente le reste, sauf en JPEG où elle devient blanche.
            if format == ExportFormat::Jpeg { assert!(right[0] > 240 && right[3] == 255) } else { assert_eq!(right[3], 0) }
            if format != ExportFormat::Tiff {
                assert!(encoded.display.starts_with(magic) && encoded.size == encoded.display.len() as u64);
            }
        }
        let small = export_encoded(&doc, ExportOptions { format: ExportFormat::Jpeg, quality: 20, lossless: false, scale: 1.0, metadata: None }).expect("export");
        let large = export_encoded(&doc, ExportOptions { format: ExportFormat::Jpeg, quality: 95, lossless: false, scale: 1.0, metadata: None }).expect("export");
        assert!(small.size < large.size, "la qualité fait varier le poids");

        let path = std::env::temp_dir().join(format!("lumitype-export-{}.tif", std::process::id()));
        let steps = std::sync::Mutex::new(Vec::new());
        export_image_with_progress(&doc, &path, ExportOptions { format: ExportFormat::Tiff, quality: 100, lossless: false, scale: 1.0, metadata: None }, &|part, _| steps.lock().expect("verrou").push(part)).expect("écriture");
        let steps = steps.into_inner().expect("verrou");
        assert!(steps.windows(2).all(|w| w[0] <= w[1]) && steps.last() == Some(&1.0), "l'avancement ne recule jamais et finit à 100 % : {steps:?}");
        assert!(std::fs::read(&path).expect("fichier").starts_with(b"II"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn lossy_webp_is_smaller_than_lossless_and_keeps_its_alpha() {
        // Une image « photo » (bruit coloré sur un dégradé, et un coin transparent) : le sans perte y est à la peine.
        let (w, h) = (320u32, 240u32);
        let mut doc = Document::with_background("x", w, h, 72.0, Background::Transparent).expect("document");
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let (x, y) = ((i as u32) % w, (i as u32) / w);
            // Hachage mélangé (pas une simple progression, que le sans perte prédit trop bien).
            let mut n = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA6B);
            n ^= n >> 15;
            n = n.wrapping_mul(0x2C1B_3C6D);
            n ^= n >> 12;
            if x < 64 && y < 64 {
                continue; // coin transparent (pixels nuls : alpha prémultiplié)
            }
            px.copy_from_slice(&[((x * 255 / w) as u8).wrapping_add((n & 31) as u8), ((y * 255 / h) as u8).wrapping_add(((n >> 8) & 31) as u8), (((x + y) * 255 / (w + h)) as u8).wrapping_add(((n >> 16) & 31) as u8), 255]);
        }
        doc.add_pixel_layer("photo", TileStore::from_premultiplied(w, h, &rgba).expect("pixels"), None, 1).expect("calque");
        let lossy = |quality| export_encoded(&doc, ExportOptions { format: ExportFormat::Webp, quality, lossless: false, scale: 1.0, metadata: None }).expect("webp");
        let lossless = export_encoded(&doc, ExportOptions { format: ExportFormat::Webp, quality: 100, lossless: true, scale: 1.0, metadata: None }).expect("webp");
        let (q80, q30) = (lossy(80), lossy(30));
        assert!(q80.size < lossless.size * 2 / 3, "avec perte {} ≪ sans perte {}", q80.size, lossless.size);
        assert!(q30.size < q80.size, "la qualité fait varier le poids");
        // Les deux se relisent, gardent la transparence, et le sans perte est exact au pixel près.
        for (encoded, exact) in [(&q80, false), (&lossless, true)] {
            assert!(encoded.display.starts_with(b"RIFF") && &encoded.display[8..12] == b"WEBP");
            let decoded = image::load_from_memory(&encoded.display).expect("webp lisible").into_rgba8();
            assert_eq!(decoded.dimensions(), (w, h));
            assert_eq!(decoded.get_pixel(10, 10).0[3], 0, "coin transparent conservé");
            if exact {
                assert_eq!(decoded.as_raw(), &rgba, "sans perte : identique au pixel près");
            } else {
                // Avec perte : du bruit pur ne survit pas tel quel (sous-échantillonnage de la couleur), mais
                // l'écart moyen reste modeste.
                let (sum, count) = decoded.as_raw().chunks_exact(4).zip(rgba.chunks_exact(4)).filter(|(_, q)| q[3] == 255).fold((0u64, 0u64), |(sum, count), (p, q)| {
                    (sum + (0..3).map(|c| u64::from(p[c].abs_diff(q[c]))).sum::<u64>(), count + 3)
                });
                assert!(sum / count < 24, "avec perte : écart moyen {} par canal", sum / count);
            }
        }
        // Les métadonnées s'inscrivent aussi dans un WebP avec perte (conteneur VP8X autour du VP8).
        let meta = crate::metadata::ImageMetadata { author: "Ada".into(), ..Default::default() };
        let tagged = export_encoded(&doc, ExportOptions { format: ExportFormat::Webp, quality: 80, lossless: false, scale: 1.0, metadata: Some(meta) }).expect("webp");
        assert!(tagged.size > q80.size && tagged.display.windows(4).any(|c| c == b"VP8X") && tagged.display.windows(4).any(|c| c == b"EXIF"));
        assert_eq!(image::load_from_memory(&tagged.display).expect("webp lisible").into_rgba8().dimensions(), (w, h));
    }

    #[test]
    fn png_export_then_import_as_document_and_as_centered_layer() {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("aplat.png");
        let mut source = Document::with_background("x", 100, 60, 72.0, Background::Transparent).expect("document");
        let id = source.walk_top_down()[0].1.id;
        source.pixels_mut(id).expect("pixels").fill(100, 60, [0, 128, 0, 255]);
        export_png(&source, &path).expect("export");

        let imported = import_image(&path).expect("import");
        assert_eq!((imported.width(), imported.height()), (100, 60));

        // Image 100 x 60 dans un document 600 x 500 : agrandie 6 fois (600 x 360), centrée verticalement.
        let mut doc = Document::with_background("y", 600, 500, 72.0, Background::White).expect("document");
        let layer = import_as_layer(&mut doc, &path, None, 1).expect("calque importé");
        assert_eq!(doc.layer(layer).expect("calque").props.name, "aplat");
        let bounds = doc.content_bounds(&[layer]).expect("bornes").expect("contenu");
        assert_eq!(bounds, (0, 70, 600, 430), "toute la largeur, centrée en hauteur, bords francs");
        let tile = TileRenderer::new().tile(&doc, 0, TileCoord::new(0, 0)).expect("tuile");
        assert_eq!((tile.pixel(0, 70), tile.pixel(0, 69)), ([0, 128, 0, 255], [255; 4]), "pas de bord fondu");

        // Image plus grande que le document : réduite pour y tenir entièrement (40 x 24 -> limité par la hauteur : 33 x 20).
        let mut small = Document::with_background("z", 40, 20, 72.0, Background::White).expect("document");
        let big = import_as_layer(&mut small, &path, None, 1).expect("calque réduit");
        let bounds = small.content_bounds(&[big]).expect("bornes").expect("contenu");
        assert!(bounds.1 >= -1 && bounds.3 <= 21 && (bounds.2 - bounds.0 - 33).abs() <= 2, "entièrement visible : {bounds:?}");

        // Image déjà à la taille du document : aucun rééchantillonnage.
        let mut same = Document::with_background("w", 100, 60, 72.0, Background::White).expect("document");
        let exact = import_as_layer(&mut same, &path, None, 1).expect("calque");
        assert_eq!(same.layer(exact).expect("calque").offset, (0, 0));
        assert_eq!(TileRenderer::new().tile(&same, 0, ORIGIN).expect("tuile").pixel(99, 59), [0, 128, 0, 255]);
    }
}
