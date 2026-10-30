//! Lecture et écriture : images d'échange (PNG, JPEG, WebP, TIFF ; export PDF) et format de projet.
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
//! smart/<clé>/flat/<x>_<y>.png  dernier rendu d'un contenu lié, ou développement d'un RAW (v7)
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
//! | 8       | RAW « à redévelopper » : réglages (`develop`) et développement (`tiles`)|
//! | 9       | texte de paragraphe (`frame` du texte) et alignement justifié           |
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
//! - `original` : une image placée, gardée à l'octet près (`smart/<clé>/original`) et décodée au chargement ; un RAW
//!   « à redévelopper » y ajoute ses réglages (`develop`) et son développement (`smart/<clé>/flat/`, relu tel quel) ;
//! - `linked` : chemin absolu et chemin relatif au projet (essayé si l'absolu n'existe plus : dossier déplacé), date
//!   du fichier, et le dernier rendu (`smart/<clé>/flat/`), affiché si le fichier a disparu. Un fichier modifié
//!   depuis l'enregistrement est relu et ses instances rendues de nouveau.
//!
//! Chaque fichier s'écrit dans la plus ancienne version capable de le décrire, pour rester lisible par les versions
//! précédentes de Lumitype : 6 sans objet dynamique, 7 avec des objets dynamiques, 8 seulement si l'un d'eux est un
//! RAW « à redévelopper » (une version qui ne connaît que la 7 ignorerait `develop` et `tiles` : autre rendu, réglages
//! perdus au réenregistrement), 9 seulement s'il contient un texte de paragraphe ou justifié (voir
//! `has_paragraph_text`) ou si le mode n'est pas RVB (Image › Mode). Une version trop ancienne refuse d'ouvrir le fichier avec son message « projet
//! enregistré avec un format plus récent » — rien n'est lu à moitié ni abîmé.

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
use crate::cmyk::{CmykProfile, CmykSpace};
// Mode du document (Image › Mode), à ne pas confondre avec `ColorMode`, les couleurs du fichier exporté.
use crate::color_mode::ColorMode as DocumentColorMode;
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
pub const FORMAT_VERSION: u32 = 9;
/// Version écrite pour un document sans objet dynamique : les versions précédentes de Lumitype l'ouvrent encore.
const FORMAT_VERSION_WITHOUT_SMART: u32 = 6;
/// Version écrite pour un document dont aucun objet dynamique n'est un RAW « à redévelopper » (Lumitype 1.1.0).
const FORMAT_VERSION_WITHOUT_RAW_DEVELOP: u32 = 7;
/// Version écrite pour un document sans texte de paragraphe ni texte justifié.
const FORMAT_VERSION_WITHOUT_PARAGRAPH_TEXT: u32 = 8;

/// Vrai si un texte du document (contenus d'objets dynamiques compris) est un texte de paragraphe ou justifié : une
/// version qui ne connaît que la 8 refuserait l'alignement « justify », ou relirait le paragraphe comme un texte de
/// point et perdrait son cadre au réenregistrement.
fn has_paragraph_text(structure: &DocumentV2) -> bool {
    let paragraph = |layers: &[LayerV2]| {
        layers.iter().any(|layer| matches!(&layer.kind, KindV2::Text { text, .. } if text.frame.is_some() || text.align == crate::text::TextAlign::Justify))
    };
    paragraph(&structure.layers) || structure.contents.iter().any(|c| matches!(&c.source, SourceV7::Embedded { document } if paragraph(&document.layers)))
}

/// Plus ancienne version capable de décrire ce document (voir l'en-tête du module) : la table `contents` regroupe
/// tous les contenus, imbriqués compris, il suffit donc de la parcourir.
fn format_version_for(structure: &DocumentV2) -> u32 {
    let contents = &structure.contents;
    // Un mode autre que RVB (gris, bitmap, travail CMJN), dans le document ou un contenu incorporé : une version qui
    // ignore `color_mode` rouvrirait le document en RVB, et ce qu'on y peindrait resterait en couleur ; il faut une
    // version qui connaît les modes (la 9).
    let colored = |mode: &DocumentColorMode| !is_rgb(mode);
    let mode = colored(&structure.color_mode) || contents.iter().any(|c| matches!(&c.source, SourceV7::Embedded { document } if colored(&document.color_mode)));
    if has_paragraph_text(structure) || mode {
        FORMAT_VERSION
    } else if contents.is_empty() {
        FORMAT_VERSION_WITHOUT_SMART
    } else if contents.iter().any(|c| matches!(c.source, SourceV7::Original { develop: Some(_), .. })) {
        FORMAT_VERSION_WITHOUT_PARAGRAPH_TEXT
    } else {
        FORMAT_VERSION_WITHOUT_RAW_DEVELOP
    }
}
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
            color_mode: DocumentColorMode::Rgb,
            text_styles: Vec::new(),
            print: None,
            work_seconds: 0,
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
                    blend_if: None,
                    filters: None,
                    artboard: None,
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
    /// Mode colorimétrique (Image › Mode). Absent des anciens fichiers et d'un document RVB (le fichier ne change
    /// pas). Un autre mode fait écrire la version 9 (voir `format_version_for`) : une version qui ignore le champ
    /// rouvrirait le document en RVB, en couleur ; elle refuse donc proprement le fichier.
    #[serde(default, skip_serializing_if = "is_rgb")]
    color_mode: DocumentColorMode,
    /// Styles de caractère. Ajout compatible : une version qui l'ignore garde l'aspect des textes (les réglages des
    /// styles sont recopiés dans les textes), seule la liste des styles serait perdue.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    text_styles: Vec<crate::text_styles::CharStyle>,
    /// Format d'impression (fonds perdus, zone de sécurité, limite d'encrage). Ajout compatible : une version qui
    /// l'ignore garde le document entier (fonds perdus compris), seuls les repères d'impression seraient perdus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    print: Option<crate::print::PrintSetup>,
    /// Temps de travail actif, en secondes. Ajout compatible : absent des anciens fichiers (0).
    #[serde(default, skip_serializing_if = "is_zero")]
    work_seconds: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn is_rgb(mode: &DocumentColorMode) -> bool {
    *mode == DocumentColorMode::Rgb
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
    ///
    /// RAW « à redévelopper » (fenêtre Développement RAW) : `develop` porte ses réglages et `tiles` son développement
    /// (`smart/<clé>/flat/`), relu tel quel à l'ouverture — rien n'est redéveloppé tant qu'on ne modifie pas les
    /// réglages, et le rendu reste celui de l'enregistrement même si le développement évolue d'une version à l'autre.
    /// Ajout compatible : les deux champs sont absents d'une image placée ordinaire (et des anciens fichiers).
    Original {
        file_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        develop: Option<crate::raw::RawParams>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tiles: Vec<[i32; 2]>,
    },
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
    /// Comparaison (options de fusion). Ajout compatible : absente des fichiers plus anciens.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "blendIf")]
    blend_if: Option<crate::blend::BlendIf>,
    /// Filtres dynamiques. Ajout compatible : une version plus ancienne n'en voit que le résultat (les pixels du
    /// calque), l'origine et le masque de filtre étant rangés à part (voir `FILTER_KEY`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    filters: Option<FiltersV2>,
    /// Plan de travail (groupe). Ajout compatible : une version qui l'ignore rouvre un groupe ordinaire, sa découpe
    /// (le masque du groupe) et son fond (un calque de remplissage) compris.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artboard: Option<crate::document::Artboard>,
    #[serde(flatten)]
    kind: KindV2,
}

/// Filtres dynamiques d'un calque : la liste, l'œil de l'ensemble, les tuiles de l'origine et le masque de filtre.
#[derive(Serialize, Deserialize)]
struct FiltersV2 {
    list: Vec<crate::smart_filters::SmartFilter>,
    enabled: bool,
    source: Vec<[i32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mask: Option<MaskV2>,
}

/// Clé des tuiles de l'origine et du masque de filtre d'un calque à filtres dynamiques : l'identifiant du calque
/// marqué de ce bit, dans les mêmes dossiers (`layers/`, `masks/`) et les mêmes tables que ses autres tuiles.
const FILTER_KEY: LayerId = 1 << 61;

fn linked_default() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
struct MaskV2 {
    enabled: bool,
    /// Absent des projets d'avant la chaîne : lié, comme dans Photoshop.
    #[serde(default = "linked_default")]
    linked: bool,
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
    /// Calque de remplissage : seule sa description est enregistrée, le rendu se recalcule à l'ouverture.
    // `fill` est déjà pris par le fond (opacité) du calque, dans le même objet : d'où ce nom.
    Fill { fill_layer: crate::fill::FillContent },
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
            SmartSource::Embedded { original: Some(original) } => SourceV7::Original {
                file_name: original.name.clone(),
                develop: original.develop,
                tiles: if original.develop.is_some() { coords_of(&content.flattened) } else { Vec::new() },
            },
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
            blend_if: layer.props.blend_if,
            mask: layer.mask.as_ref().map(|m| MaskV2 {
                enabled: m.enabled,
                linked: m.linked,
                background: m.pixels.background()[0],
                tiles: coords_of(&m.pixels),
            }),
            effects: layer.effects.clone(),
            label: layer.label,
            filters: layer.filters.as_ref().map(|f| FiltersV2 {
                list: f.list.clone(),
                enabled: f.enabled,
                source: coords_of(&f.source),
                mask: f.mask.as_ref().map(|m| MaskV2 { enabled: m.enabled, linked: m.linked, background: m.pixels.background()[0], tiles: coords_of(&m.pixels) }),
            }),
            artboard: layer.artboard.clone(),
            kind: match &layer.kind {
                LayerKind::Pixel { pixels } => KindV2::Pixel { tiles: coords_of(pixels) },
                LayerKind::Adjustment { adjustment } => KindV2::Adjustment { adjustment: adjustment.clone() },
                LayerKind::Shape { shape, raster } => KindV2::Shape { shape: shape.clone(), tiles: coords_of(raster) },
                LayerKind::Fill { fill, .. } => KindV2::Fill { fill_layer: fill.clone() },
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
        color_mode: doc.color_mode(),
        text_styles: doc.text_styles.clone(),
        print: doc.print_setup(),
        work_seconds: doc.work_seconds,
    }
}

// --------------------------------------------------------------------- images

/// Décode une image d'échange en RGBA prémultiplié. `origin` ne sert qu'aux messages d'erreur.
///
/// PNG, JPEG, WebP et TIFF passent par la bibliothèque `image`. Les photos HEIC/HEIF (iPhone) ont leur
/// décodeur, choisi d'après le contenu du fichier (l'extension est souvent absente ou trompeuse quand
/// l'image vient d'un autre logiciel). Les RAW d'appareil (ARW, CR3, NEF… et DNG) passent par `rawler`,
/// reconnus à leur extension ou, pour un DNG, à son étiquette.
fn decode_image(bytes: &[u8], origin: &Path) -> Result<(u32, u32, Vec<u8>), EngineError> {
    let image_error = |source| EngineError::Image { path: origin.to_path_buf(), source };
    let mut rgba = if is_heif(bytes) {
        decode_heif(bytes, origin)?
    } else if is_raw(bytes, origin) {
        decode_raw(bytes, origin)?
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

/// Décodage seul, sans document : RGBA prémultiplié et dimensions. Sert aux vignettes de la bibliothèque
/// « My ref » (coque), qui lit ainsi tous les formats que le moteur sait ouvrir (HEIC et RAW compris).
pub fn decode_premultiplied(bytes: &[u8], origin: &Path) -> Result<(u32, u32, Vec<u8>), EngineError> {
    decode_image(bytes, origin)
}

/// Conteneur HEIF : une boîte `ftyp` en tête, avec une marque d'image fixe (heic/heix : HEVC ;
/// mif1/msf1 : HEIF générique, ce que produit un iPhone ; avif exclu, il passe par la webview).
fn is_heif(bytes: &[u8]) -> bool {
    bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && matches!(&bytes[8..12], b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"hevm" | b"hevs" | b"mif1" | b"msf1")
}

/// Extensions des fichiers RAW d'appareils photo que le moteur ouvre (décodeur `rawler`) : Sony (ARW,
/// SRF, SR2), Canon (CR2, CR3, CRW), Nikon (NEF, NRW), Fujifilm (RAF), Olympus/OM (ORF, ORI), Panasonic et
/// Leica (RW2, RWL, RAW), Pentax (PEF), Samsung (SRW), Hasselblad (3FR, FFF), Phase One/Leaf (IIQ, MOS),
/// Epson (ERF), Kodak (DCR, DCS, KDC), Mamiya (MEF), Minolta (MRW), Sigma (X3F), ARRI (ARI) et le DNG.
pub const RAW_EXTENSIONS: &[&str] =
    &["dng", "arw", "srf", "sr2", "cr2", "cr3", "crw", "nef", "nrw", "raf", "orf", "ori", "rw2", "rwl", "raw", "pef", "srw", "3fr", "fff", "iiq", "mos", "erf", "dcr", "dcs", "kdc", "mef", "mrw", "x3f", "ari"];

/// Le chemin désigne-t-il un RAW d'appareil (d'après son extension, sans tenir compte de la casse) ?
pub fn is_raw_path(origin: &Path) -> bool {
    origin.extension().and_then(|e| e.to_str()).is_some_and(|e| RAW_EXTENSIONS.iter().any(|raw| e.eq_ignore_ascii_case(raw)))
}

/// RAW : une extension connue (ARW, CR3, NEF…), ou un TIFF portant l'étiquette `DNGVersion` (0xC612)
/// dans son premier répertoire (DNG renommé). Un RAW ne doit jamais tomber dans le décodeur TIFF de
/// `image` : il n'y lirait que la petite vignette embarquée.
fn is_raw(bytes: &[u8], origin: &Path) -> bool {
    is_raw_path(origin) || is_dng(bytes, origin)
}

/// Un DNG est un TIFF portant l'étiquette `DNGVersion` (0xC612) dans son premier répertoire.
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

/// RAW (DNG, ARW, CR3, NEF…) : développement par défaut de Lumitype (dématriçage, balance des blancs de la prise de
/// vue, matrice couleur de l'appareil vers sRGB, hautes lumières, gamma sRGB), orientation appliquée — le même que
/// celui de la fenêtre « Développement RAW » sans réglage (`raw::develop_default`). Si le développement échoue — une
/// compression que le décodeur ne connaît pas, par exemple — l'aperçu JPEG que le fichier embarque sert de secours,
/// avec un avertissement dans l'erreur seulement s'il n'y en a pas non plus.
fn decode_raw(bytes: &[u8], origin: &Path) -> Result<image::RgbaImage, EngineError> {
    use rawler::{decoders::RawDecodeParams, rawsource::RawSource, Orientation};
    let developed = crate::raw::develop_default(bytes, origin)
        .and_then(|(w, h, rgba)| image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| raw_error(origin, "tampon développé de taille inattendue".into())));
    match developed {
        Ok(img) => Ok(img),
        Err(failure) => {
            let reason = failure.to_string();
            // Secours : l'aperçu embarqué (pleine taille sur un ProRAW ou un export Lightroom).
            let source = RawSource::new_from_slice(bytes);
            let decoder = rawler::get_decoder(&source).map_err(|e| raw_error(origin, format!("appareil ou format non reconnu ({e})")))?;
            let params = RawDecodeParams::default();
            let orientation = decoder.raw_metadata(&source, &params).ok().and_then(|m| m.exif.orientation).map(Orientation::from_u16).unwrap_or(Orientation::Unknown);
            let preview = decoder.preview_image(&source, &params).ok().flatten().or_else(|| decoder.thumbnail_image(&source, &params).ok().flatten());
            preview.map(|img| orient_raw(img, orientation).to_rgba8()).ok_or_else(|| raw_error(origin, format!("développement impossible ({reason}) et aucun aperçu embarqué")))
        }
    }
}

fn raw_error(origin: &Path, what: String) -> EngineError {
    let name = origin.file_name().and_then(|n| n.to_str()).unwrap_or("image");
    EngineError::Corrupt(format!("RAW « {name} » : {what}"))
}

/// Orientation enregistrée par l'appareil (EXIF), appliquée à l'image développée ou à l'aperçu.
fn orient_raw(img: image::DynamicImage, orientation: rawler::Orientation) -> image::DynamicImage {
    use rawler::Orientation;
    match orientation {
        Orientation::Normal | Orientation::Unknown => img,
        Orientation::HorizontalFlip => img.fliph(),
        Orientation::Rotate180 => img.rotate180(),
        Orientation::VerticalFlip => img.flipv(),
        Orientation::Transpose => img.rotate90().fliph(),
        Orientation::Rotate90 => img.rotate90(),
        Orientation::Transverse => img.rotate90().flipv(),
        Orientation::Rotate270 => img.rotate270(),
    }
}

/// Aperçu JPEG embarqué dans un RAW, orienté, si son côté long atteint `min_side` : de quoi faire une vignette ou
/// un affichage rapide sans développer le capteur (quelques millisecondes au lieu d'une à trois secondes).
/// `None` : pas de RAW, pas d'aperçu, ou un aperçu trop petit — développer alors le fichier.
pub fn raw_preview(bytes: &[u8], origin: &Path, min_side: u32) -> Option<image::RgbaImage> {
    use rawler::{decoders::RawDecodeParams, rawsource::RawSource, Orientation};
    if !is_raw(bytes, origin) {
        return None;
    }
    let source = RawSource::new_from_slice(bytes);
    let decoder = rawler::get_decoder(&source).ok()?;
    let params = RawDecodeParams::default();
    let preview = decoder.preview_image(&source, &params).ok().flatten().or_else(|| decoder.thumbnail_image(&source, &params).ok().flatten())?;
    if preview.width().max(preview.height()) < min_side {
        return None;
    }
    let orientation = decoder.raw_metadata(&source, &params).ok().and_then(|m| m.exif.orientation).map(Orientation::from_u16).unwrap_or(Orientation::Unknown);
    Some(orient_raw(preview, orientation).to_rgba8())
}

/// Définition réelle d'un RAW développé (zone recommandée par l'appareil, orientation appliquée), lue sans décoder le
/// capteur : celle qu'affiche l'explorateur de Windows (6024 × 4024 pour un Sony α6400), pas celle de l'aperçu embarqué.
pub fn raw_dimensions(bytes: &[u8], origin: &Path) -> Option<(u32, u32)> {
    use rawler::{decoders::RawDecodeParams, rawsource::RawSource, Orientation};
    if !is_raw(bytes, origin) {
        return None;
    }
    let source = RawSource::new_from_slice(bytes);
    let decoder = rawler::get_decoder(&source).ok()?;
    let raw = decoder.raw_image(&source, &RawDecodeParams::default(), true).ok()?;
    let (w, h) = raw.crop_area.or(raw.active_area).map_or((raw.width, raw.height), |r| (r.d.w, r.d.h));
    let (w, h) = (u32::try_from(w).ok()?, u32::try_from(h).ok()?);
    let turned = matches!(raw.orientation, Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Transpose | Orientation::Transverse);
    (w > 0 && h > 0).then_some(if turned { (h, w) } else { (w, h) })
}

/// Miniature d'un RAW pour l'explorateur de fichiers (générateur de miniatures Linux, voir src-tauri/linux) : PNG dont
/// le côté long fait au plus `size` pixels. L'aperçu embarqué par l'appareil suffit presque toujours (quelques
/// millisecondes) ; sinon, le capteur est développé.
pub fn raw_thumbnail_png(path: &Path, size: u32) -> Result<Vec<u8>, EngineError> {
    let bytes = std::fs::read(path).map_err(EngineError::io(path))?;
    if !is_raw(&bytes, path) {
        return Err(raw_error(path, "pas un RAW".into()));
    }
    let size = size.clamp(16, 2048);
    let image = match raw_preview(&bytes, path, size.min(256)) {
        Some(preview) => preview,
        None => decode_raw(&bytes, path)?,
    };
    let image = image::DynamicImage::ImageRgba8(image);
    let image = if image.width().max(image.height()) > size { image.thumbnail(size, size) } else { image };
    let mut out = Vec::new();
    image.to_rgb8().write_with_encoder(PngEncoder::new(&mut out)).map_err(|source| EngineError::Image { path: path.to_path_buf(), source })?;
    Ok(out)
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
    document_from_premultiplied(width, height, &rgba, origin)
}

/// Image déjà décodée (RGBA prémultiplié) en document d'un seul calque, nommé d'après `origin` : un RAW développé
/// par la fenêtre « Développement RAW » (`crate::raw`).
pub fn document_from_premultiplied(width: u32, height: u32, rgba: &[u8], origin: &Path) -> Result<Document, EngineError> {
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(EngineError::InvalidDimensions { width, height, max: MAX_DIMENSION });
    }
    let mut doc = Document::new(file_stem(origin), width, height, 72.0)?;
    let pixels = TileStore::from_premultiplied(width, height, rgba)
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
    layer_from_premultiplied(doc, width, height, &rgba, origin, parent, index)
}

/// Image déjà décodée (RGBA prémultiplié) importée comme calque, ajustée au document et centrée (voir
/// [`import_bytes_as_layer`]) : un RAW développé par la fenêtre « Développement RAW ».
pub fn layer_from_premultiplied(
    doc: &mut Document,
    width: u32,
    height: u32,
    rgba: &[u8],
    origin: &Path,
    parent: Option<LayerId>,
    index: usize,
) -> Result<LayerId, EngineError> {
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(EngineError::InvalidDimensions { width, height, max: MAX_DIMENSION });
    }
    let pixels = TileStore::from_premultiplied(width, height, rgba)
        .ok_or_else(|| EngineError::Corrupt("tampon décodé de taille inattendue".into()))?;
    // L'image est ajustée à son cadre (entièrement visible, proportions gardées) puis centrée, quelle que soit sa
    // taille d'origine : le plan de travail qui la reçoit, sinon le document (voir `Document::placement_frame`).
    // Ctrl+T permet ensuite de la redimensionner.
    let (fx, fy, fw, fh) = doc.placement_frame(parent);
    let fit = (f64::from(fw) / f64::from(width)).min(f64::from(fh) / f64::from(height));
    let (pixels, width, height) = if (fit - 1.0).abs() < 1e-9 {
        (pixels, f64::from(width), f64::from(height))
    } else {
        let (w, h) = ((f64::from(width) * fit).round().max(1.0), (f64::from(height) * fit).round().max(1.0));
        (resize_image(rgba, width, height, w as u32, h as u32)?, w, h)
    };
    let id = doc.add_pixel_layer(file_stem(origin), pixels, parent, index)?;
    let dx = fx + ((f64::from(fw) - width) / 2.0).round() as i32;
    let dy = fy + ((f64::from(fh) - height) / 2.0).round() as i32;
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
            flat.restore(coord, renderer.tile_uncached(doc, 0, coord));
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
    /// PDF d'une page (voir [`crate::pdf`]) : RVB ou CMJN, sans perte (Flate) ou JPEG selon `ExportOptions::pdf_quality`.
    Pdf,
    /// AVIF (AV1 dans un conteneur HEIF) : avec perte, transparence comprise, 2 à 3 fois plus léger que le WebP sur
    /// une photo, mais un encodage bien plus long (plusieurs secondes pour 10 Mpx). Encodé par rav1e (crate `ravif`),
    /// voir [`encode_avif`] : qualité du curseur commun (`ExportOptions::quality`), vitesse `ExportOptions::avif_speed`.
    Avif,
}

impl ExportFormat {
    /// Vrai si le format sait écrire du CMJN, et porter un profil ICC incorporé : PDF, TIFF et JPEG. Le PNG, le
    /// WebP et l'AVIF restent en RVB, sans profil (l'AVIF déclare le sRGB par ses codes CICP, voir [`encode_avif`]).
    pub fn supports_cmyk(self) -> bool {
        matches!(self, Self::Pdf | Self::Tiff | Self::Jpeg)
    }
}

/// Vitesse d'encodage de l'AVIF : préréglage de rav1e (0, le plus lent, à 10). Plus lent = un peu plus léger à
/// qualité égale, surtout sur les images graphiques (aplats, texte, captures d'écran) ; sur une photo, l'écart est
/// presque nul. Mesures (2 cœurs, nasm, qualité 85 du curseur), Rapide / Équilibrée / Maximale : photo 10,7 Mpx 175 Ko aux
/// trois vitesses, en 6,1 / 11,6 / 11,6 s ; capture d'écran 8,3 Mpx 116 / 86 / 78 Ko en 2,7 / 7,4 / 6,8 s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AvifSpeed {
    /// « Rapide » : préréglage 9, le défaut.
    #[default]
    Fast,
    /// « Équilibrée » : préréglage 6.
    Balanced,
    /// « Maximale » (compression) : préréglage 4.
    Max,
}

impl AvifSpeed {
    /// Préréglage de rav1e.
    pub fn preset(self) -> u8 {
        match self {
            Self::Fast => 9,
            Self::Balanced => 6,
            Self::Max => 4,
        }
    }
}

/// Estimation du poids d'un AVIF : surface de l'échantillon encodé (en dessous, l'image est encodée entièrement).
const AVIF_SAMPLE_PIXELS: u64 = 500_000;

/// Couleurs du fichier exporté : RVB (écran), CMJN (impression, selon un profil de presse), niveaux de gris ou
/// bitmap (noir et blanc). Gris : PNG, TIFF, JPEG et PDF en vrais fichiers gris 8 bits ; Bitmap : PNG, TIFF et PDF
/// 1 bit, JPEG en gris 8 bits (le format n'a pas de 1 bit). WebP et AVIF n'ont pas de mode gris dans leurs encodeurs :
/// ils restent en RVB, aux pixels ramenés au gris (ou au noir et blanc).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    #[default]
    Rgb,
    Cmyk,
    Gray,
    Bitmap,
}

/// Qualité d'un PDF, comme dans Photoshop : sans perte (Flate), ou JPEG plus ou moins compressé.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PdfQuality {
    /// « Qualité maximale (sans perte) » : FlateDecode, prédicteurs PNG.
    Lossless,
    /// JPEG 92 : indiscernable de l'original à l'impression.
    #[default]
    High,
    /// JPEG 80.
    Medium,
    /// JPEG 60 : le plus léger (épreuve, envoi par courriel).
    Low,
}

impl PdfQuality {
    /// Qualité JPEG, ou `None` sans perte.
    pub fn jpeg_quality(self) -> Option<u8> {
        match self {
            Self::Lossless => None,
            Self::High => Some(92),
            Self::Medium => Some(80),
            Self::Low => Some(60),
        }
    }
}

/// Réglages d'un export : format, qualité (JPEG, WebP avec perte et AVIF, 1 à 100), `lossless` (WebP seulement : sans
/// perte, la qualité est alors ignorée), échelle (1 = taille native) et métadonnées à inscrire dans le fichier
/// (auteur, copyright… voir [`crate::metadata`]).
///
/// Ajouts compatibles (absents des anciens réglages : valeurs par défaut) : `color` (RVB ou CMJN, pour le PDF, le
/// TIFF et le JPEG), `profile` (profil de presse du CMJN), `embedProfile` (incorporer le profil ICC : celui de presse
/// en CMJN, sRGB en RVB), `pdfQuality` et `avifSpeed` (vitesse d'encodage de l'AVIF, « rapide » par défaut).
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub quality: u8,
    #[serde(default)]
    pub lossless: bool,
    pub scale: f32,
    #[serde(default)]
    pub metadata: Option<ImageMetadata>,
    #[serde(default)]
    pub color: ColorMode,
    /// Espace CMJN de l'export : celui du document en mode Couleurs CMJN, sinon celui des Paramètres de couleur.
    #[serde(default)]
    pub cmyk_space: CmykSpace,
    #[serde(default)]
    pub embed_profile: bool,
    #[serde(default)]
    pub pdf_quality: PdfQuality,
    #[serde(default)]
    pub avif_speed: AvifSpeed,
    /// PDF d'impression (PDF/X-4) : fonds perdus, traits de coupe, limite d'encrage. Toujours en CMJN.
    #[serde(default)]
    pub pdf_print: Option<crate::pdf::PdfPrint>,
    /// Plan de travail à exporter seul (son cadre devient l'image) ; `None` : le document entier.
    #[serde(default)]
    pub artboard: Option<LayerId>,
}

/// Document tel qu'il sera exporté : le document entier, ou recadré sur le plan de travail demandé (copie légère,
/// les tuiles sont partagées). Un plan inconnu est une erreur : on n'exporte pas autre chose que ce qui est demandé.
pub fn export_view<'a>(doc: &'a Document, options: &ExportOptions) -> Result<std::borrow::Cow<'a, Document>, EngineError> {
    let Some(board) = options.artboard else { return Ok(std::borrow::Cow::Borrowed(doc)) };
    let [x0, y0, x1, y1] = doc.artboard_rect(board).ok_or_else(|| EngineError::InvalidOperation("ce plan de travail n'existe plus".into()))?;
    let mut view = doc.clone();
    view.crop(x0, y0, (x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32)?;
    Ok(std::borrow::Cow::Owned(view))
}

impl ExportOptions {
    /// Réglages par défaut d'un format : qualité 85, taille native, RVB sans profil, PDF en qualité haute, AVIF rapide.
    pub fn new(format: ExportFormat) -> Self {
        Self { format, quality: 85, lossless: false, scale: 1.0, metadata: None, color: ColorMode::Rgb, cmyk_space: CmykSpace::default(), embed_profile: false, pdf_quality: PdfQuality::default(), avif_speed: AvifSpeed::default(), pdf_print: None, artboard: None }
    }

    /// Profil du CMJN, si le CMJN est demandé et que le format sait l'écrire.
    pub fn cmyk(&self) -> Option<CmykSpace> {
        (self.color == ColorMode::Cmyk && self.format.supports_cmyk()).then_some(self.cmyk_space)
    }

    /// Profil ICC à incorporer au fichier (TIFF et JPEG ; le PDF place le sien lui-même). Aucun en gris : pas de
    /// profil gris livré (un profil sRGB dans un fichier gris serait invalide).
    fn icc(&self) -> Result<Option<Arc<[u8]>>, EngineError> {
        if !(self.embed_profile && self.format.supports_cmyk() && self.gray_bits().is_none()) {
            return Ok(None);
        }
        Ok(Some(match self.cmyk() {
            Some(space) => space.profile.icc_bytes()?,
            None => Arc::from(crate::cmyk::srgb_icc()),
        }))
    }

    /// Profondeur d'un vrai fichier gris, si le format en écrit un : 8 bits en gris (PNG, TIFF, JPEG, PDF) ; 1 bit en
    /// bitmap (PNG, TIFF, PDF), 8 bits pour le JPEG. `None` : fichier RVB (ou CMJN).
    pub fn gray_bits(&self) -> Option<u8> {
        match (self.color, self.format) {
            (ColorMode::Gray, ExportFormat::Png | ExportFormat::Tiff | ExportFormat::Jpeg | ExportFormat::Pdf) => Some(8),
            (ColorMode::Bitmap, ExportFormat::Png | ExportFormat::Tiff | ExportFormat::Pdf) => Some(1),
            (ColorMode::Bitmap, ExportFormat::Jpeg) => Some(8),
            _ => None,
        }
    }

    /// Pixels ramenés au gris ou au noir et blanc (RGBA, alpha droit) quand l'export le demande ; sinon tels quels.
    fn tone_pixels<'a>(&self, rgba: &'a [u8]) -> std::borrow::Cow<'a, [u8]> {
        match self.color {
            ColorMode::Gray => std::borrow::Cow::Owned(rgba.par_chunks_exact(4).flat_map_iter(|px| {
                let y = crate::color_mode::luma(px[0], px[1], px[2]);
                [y, y, y, px[3]]
            }).collect()),
            ColorMode::Bitmap => std::borrow::Cow::Owned(crate::color_mode::bilevel(rgba).into_par_iter().flat_map_iter(|white| if white { [255; 4] } else { [0, 0, 0, 255] }).collect()),
            ColorMode::Rgb | ColorMode::Cmyk => std::borrow::Cow::Borrowed(rgba),
        }
    }

    /// Format et qualité de l'aperçu quand l'encodage perd de l'information : on y voit ses défauts de compression.
    /// `None` : format sans perte, l'aperçu montre les pixels tels quels. L'AVIF aussi : la webview (WebKitGTK selon
    /// sa compilation) ne sait pas toujours l'afficher, le moteur ne le décode pas, et un encodage AV1 à chaque
    /// déplacement de l'aperçu serait trop lent.
    fn lossy_preview(&self) -> Option<(ExportFormat, u8)> {
        match self.format {
            ExportFormat::Jpeg => Some((ExportFormat::Jpeg, self.quality)),
            ExportFormat::Webp if !self.lossless => Some((ExportFormat::Webp, self.quality)),
            ExportFormat::Pdf => self.pdf_quality.jpeg_quality().map(|q| (ExportFormat::Jpeg, q)),
            _ => None,
        }
    }

    /// Pixels tels que l'aperçu doit les montrer : en CMJN, l'épreuve écran du profil (couleurs imprimables, sur
    /// fond blanc) ; sinon l'image elle-même.
    fn display_pixels<'a>(&self, rgba: &'a [u8]) -> Result<std::borrow::Cow<'a, [u8]>, EngineError> {
        Ok(match self.cmyk() {
            Some(space) => std::borrow::Cow::Owned(crate::cmyk::soft_proof(rgba, space)?),
            None => self.tone_pixels(rgba),
        })
    }
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
    encode_rgb(width, height, rgba, format, 90, lossless).expect("encodage")
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

/// Taille de la page d'un PDF, en points (1/72 de pouce) : la taille du document à sa résolution (px × 72 / ppp),
/// quelle que soit l'échelle de l'export (qui change la finesse, pas la taille imprimée).
fn page_of(doc: &Document) -> [f32; 2] {
    let dpi = if doc.dpi.is_finite() && doc.dpi > 0.0 { doc.dpi } else { 72.0 };
    [doc.width() as f32 * 72.0 / dpi, doc.height() as f32 * 72.0 / dpi]
}

/// Aplat du document (RGBA prémultiplié, à l'échelle `scale`). Composé sans garder les tuiles composées en cache
/// (chacune n'est demandée qu'une fois) : la mémoire de l'export ne double pas celle de l'image ; les caches par
/// calque du rendu neuf (effets, réductions) sont libérés avec lui à la fin.
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
                    let row: Vec<_> = (0..columns).map(|tx| TileCoord::new(tx, ty)).map(|coord| (coord, renderer.tile_uncached(doc, 0, coord))).collect();
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

/// Pose une image RGBA (alpha droit) sur du blanc, en RGB : pour les formats sans transparence.
fn rgb_on_white(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = vec![0u8; rgba.len() / 4 * 3];
    rgb.par_chunks_exact_mut(3).zip(rgba.par_chunks_exact(4)).for_each(|(out, px)| {
        let a = u32::from(px[3]);
        for c in 0..3 {
            out[c] = ((u32::from(px[c]) * a + 255 * (255 - a) + 127) / 255) as u8;
        }
    });
    rgb
}

fn jpeg_size(width: u32, height: u32) -> Result<(u16, u16), EngineError> {
    match (u16::try_from(width), u16::try_from(height)) {
        (Ok(w), Ok(h)) => Ok((w, h)),
        _ => Err(EngineError::Corrupt("le JPEG est limité à 65 535 px de côté".into())),
    }
}

fn jpeg_failed(e: jpeg_encoder::EncodingError) -> EngineError {
    EngineError::Corrupt(format!("encodage impossible : {e}"))
}

/// JPEG RVB (encodeur SIMD `jpeg-encoder`, nettement plus rapide que celui de la bibliothèque image), avec le profil
/// ICC donné dans des segments APP2 `ICC_PROFILE`.
pub(crate) fn encode_jpeg_rgb(rgb: &[u8], width: u32, height: u32, quality: u8, icc: Option<&[u8]>) -> Result<Vec<u8>, EngineError> {
    let (w, h) = jpeg_size(width, height)?;
    let mut bytes = Vec::with_capacity(rgb.len() / 6);
    let mut encoder = jpeg_encoder::Encoder::new(&mut bytes, quality.clamp(1, 100));
    if let Some(icc) = icc {
        encoder.add_icc_profile(icc).map_err(jpeg_failed)?;
    }
    encoder.encode(rgb, w, h, jpeg_encoder::ColorType::Rgb).map_err(jpeg_failed)?;
    Ok(bytes)
}

/// JPEG en niveaux de gris (une composante).
pub(crate) fn encode_jpeg_gray(gray: &[u8], width: u32, height: u32, quality: u8) -> Result<Vec<u8>, EngineError> {
    let (w, h) = jpeg_size(width, height)?;
    let mut bytes = Vec::with_capacity(gray.len() / 6);
    jpeg_encoder::Encoder::new(&mut bytes, quality.clamp(1, 100)).encode(gray, w, h, jpeg_encoder::ColorType::Luma).map_err(jpeg_failed)?;
    Ok(bytes)
}

/// Vrai fichier gris (voir [`ExportOptions::gray_bits`]) : PNG gris (alpha compris) ou 1 bit, TIFF gris 8 bits (sur
/// blanc : l'encodeur n'écrit pas de gris + alpha) ou 1 bit PackBits, JPEG gris, PDF `DeviceGray`.
fn encode_gray(width: u32, height: u32, rgba: &[u8], options: &ExportOptions, bits: u8, page: [f32; 2], metadata: Option<&ImageMetadata>) -> Result<Vec<u8>, EngineError> {
    let failed = |e: image::ImageError| EngineError::Corrupt(format!("encodage impossible : {e}"));
    match options.format {
        ExportFormat::Png => crate::color_mode::encode_png_gray(width, height, rgba, bits),
        ExportFormat::Tiff if bits == 1 => Ok(crate::color_mode::encode_tiff_bilevel(width, height, rgba)),
        ExportFormat::Tiff => {
            let mut out = std::io::Cursor::new(Vec::new());
            image::codecs::tiff::TiffEncoder::new(&mut out).write_image(&crate::color_mode::gray_on_white_straight(rgba), width, height, ExtendedColorType::L8).map_err(failed)?;
            Ok(out.into_inner())
        }
        ExportFormat::Jpeg => {
            let gray = if options.color == ColorMode::Bitmap { crate::color_mode::bilevel(rgba).into_iter().map(|w| if w { 255 } else { 0 }).collect() } else { crate::color_mode::gray_on_white_straight(rgba) };
            encode_jpeg_gray(&gray, width, height, options.quality)
        }
        ExportFormat::Pdf => crate::pdf::encode(
            width,
            height,
            rgba,
            &crate::pdf::PdfSettings { page, cmyk: None, embed_profile: false, jpeg_quality: options.pdf_quality.jpeg_quality(), metadata, gray_bits: Some(bits), print: None },
        ),
        ExportFormat::Webp | ExportFormat::Avif => Err(EngineError::Corrupt("pas de fichier gris dans ce format".into())),
    }
}

/// JPEG CMJN (4 octets par pixel, 0 = pas d'encre). L'encodeur suit la convention d'Adobe (segment APP14, encres
/// inversées dans le flux) que Photoshop, InDesign et les RIP attendent. Les quatre encres sont gardées à pleine
/// résolution (pas de sous-échantillonnage, réservé à la chrominance d'un JPEG RVB) et quantifiées avec la même
/// table, celle de la luminance : le cyan, le magenta et le jaune comptent autant que le noir.
pub(crate) fn encode_jpeg_cmyk(cmyk: &[u8], width: u32, height: u32, quality: u8, icc: Option<&[u8]>) -> Result<Vec<u8>, EngineError> {
    // Table de luminance de l'annexe K (ordre naturel), mise à l'échelle comme le fait libjpeg.
    const ANNEX_K: [u16; 64] = [
        16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104,
        113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
    ];
    let (w, h) = jpeg_size(width, height)?;
    let quality = u32::from(quality.clamp(1, 100));
    let scale = if quality < 50 { 5000 / quality } else { 200 - quality * 2 };
    let table: [u16; 64] = ANNEX_K.map(|v| ((u32::from(v) * scale + 50) / 100).clamp(1, 255) as u16);
    let mut bytes = Vec::with_capacity(cmyk.len() / 6);
    let mut encoder = jpeg_encoder::Encoder::new(&mut bytes, quality as u8);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1);
    encoder.set_quantization_tables(jpeg_encoder::QuantizationTableType::Custom(Box::new(table)), jpeg_encoder::QuantizationTableType::Custom(Box::new(table)));
    if let Some(icc) = icc {
        encoder.add_icc_profile(icc).map_err(jpeg_failed)?;
    }
    encoder.encode(cmyk, w, h, jpeg_encoder::ColorType::Cmyk).map_err(jpeg_failed)?;
    Ok(bytes)
}

/// Qualité de ravif (1 à 100) pour une valeur du curseur commun, calé sur le JPEG. À valeur égale, l'AV1 garde
/// nettement plus de détail que le JPEG (son échelle de quantification n'est pas la même) : le curseur à 85 (le défaut)
/// donne 70, la valeur mesurée et validée (photo 10,7 Mpx : 175 Ko, contre 1 624 Ko en JPEG 85 et 454 Ko en WebP 80),
/// proportionnellement en dessous ; de 85 à 100, la qualité de ravif remonte jusqu'à 100 (presque sans perte).
fn avif_quality(quality: u8) -> f32 {
    let q = f32::from(quality.clamp(1, 100));
    if q <= 85.0 { 1.0 + (q - 1.0) * 69.0 / 84.0 } else { 70.0 + (q - 85.0) * 2.0 }
}

/// AVIF : AV1 4:4:4 en 10 bits (meilleur à poids égal, même pour une source 8 bits), encodé par rav1e (crate
/// `ravif`) sur le réservoir de fils rayon, au préréglage `preset` de rav1e (voir [`AvifSpeed`]). `rgba` est en alpha DROIT (l'aplat d'export, voir [`flattened`]) :
/// l'AVIF le stocke tel quel (alpha non associé, sans boîte `prem`) ; la couleur des pixels entièrement transparents,
/// invisible, est lissée par ravif pour peser moins. Une image opaque est écrite sans plan alpha.
///
/// Colorimétrie : codes CICP 1/13/6, pleine plage (primaires BT.709 = sRGB, transfert sRGB, matrice BT.601) dans
/// l'en-tête de séquence AV1. La boîte `colr` (nclx) porterait les mêmes valeurs ; `avif-serialize` ne l'écrit pas
/// quand elles sont celles par défaut des lecteurs (Safari suppose justement la BT.601 sans elle). Pas de profil ICC.
///
/// Métadonnées : EXIF seulement (description, logiciel, date, auteur, copyright), en élément `Exif` relié à l'image
/// (`cdsc`). `ravif` n'en écrit pas : le conteneur est alors refait par `avif-serialize` autour des mêmes flux AV1
/// (ceux de ravif, repris en fin de fichier). `avif-serialize` ne sait pas joindre de XMP : les mots-clés et le
/// contact (que seul le XMP porte) ne sont pas inscrits dans un AVIF.
fn encode_avif(width: u32, height: u32, rgba: &[u8], quality: u8, preset: u8, metadata: Option<&ImageMetadata>) -> Result<Vec<u8>, EngineError> {
    let encoded = ravif_encode(width, height, rgba, quality, preset)?;
    match metadata.filter(|meta| meta.has_exif()) {
        None => Ok(encoded.avif_file),
        Some(meta) => avif_container(&encoded.avif_file, encoded.color_byte_size, encoded.alpha_byte_size, width, height, Some(meta.exif_tiff())),
    }
}

/// Le fichier de ravif, et la taille de ses flux AV1 (couleur, alpha) : voir [`encode_avif`].
fn ravif_encode(width: u32, height: u32, rgba: &[u8], quality: u8, preset: u8) -> Result<ravif::EncodedImage, EngineError> {
    let q = avif_quality(quality);
    // Plan alpha un peu plus fin que la couleur (règle de cavif, l'outil de ravif) : les contours restent nets.
    let alpha_q = ((q + 100.0) / 2.0).min(q + q / 4.0 + 2.0).clamp(1.0, 100.0);
    let pixels: Vec<ravif::RGBA8> = rgba.par_chunks_exact(4).map(|px| ravif::RGBA8::new(px[0], px[1], px[2], px[3])).collect();
    ravif::Encoder::new()
        .with_quality(q)
        .with_alpha_quality(alpha_q)
        .with_speed(preset.clamp(1, 10))
        .with_bit_depth(ravif::BitDepth::Ten)
        .with_internal_color_model(ravif::ColorModel::YCbCr)
        .with_alpha_color_mode(ravif::AlphaColorMode::UnassociatedClean)
        .with_num_threads(None)
        .encode_rgba(ravif::Img::new(&pixels[..], width as usize, height as usize))
        .map_err(|e| EngineError::Corrupt(format!("encodage AVIF impossible : {e}")))
}

/// Refait le conteneur d'un AVIF de ravif, avec l'élément EXIF donné. `avif-serialize` range les flux AV1 à la fin
/// du fichier (boîte `mdat`, la dernière), l'alpha d'abord puis la couleur : on les reprend là, d'après leurs tailles.
/// Mêmes réglages que ravif (matrice BT.601, alpha non associé, 10 bits) : sans EXIF, le fichier refait est identique
/// octet pour octet (vérifié par un test).
fn avif_container(file: &[u8], color_size: usize, alpha_size: usize, width: u32, height: u32, exif: Option<Vec<u8>>) -> Result<Vec<u8>, EngineError> {
    let end = file.len();
    let color_start = end.checked_sub(color_size).ok_or_else(|| EngineError::Corrupt("AVIF : flux couleur introuvable".into()))?;
    let alpha_start = color_start.checked_sub(alpha_size).ok_or_else(|| EngineError::Corrupt("AVIF : flux alpha introuvable".into()))?;
    let color = &file[color_start..];
    let alpha = (alpha_size > 0).then(|| &file[alpha_start..color_start]);
    let mut aviffy = avif_serialize::Aviffy::new();
    aviffy.set_matrix_coefficients(avif_serialize::constants::MatrixCoefficients::Bt601).set_premultiplied_alpha(false);
    if let Some(exif) = exif {
        aviffy.set_exif(exif);
    }
    let mut out = Vec::with_capacity(file.len() + 256);
    aviffy.write(&mut out, color, alpha, width, height, 10).map_err(|e| EngineError::Corrupt(format!("conteneur AVIF impossible : {e}")))?;
    Ok(out)
}

/// Encodage RVB sans profil (PNG, JPEG, WebP, TIFF) : celui de l'aperçu, et de l'export quand rien d'autre n'est
/// demandé. `lossless` ne concerne que le WebP ; `quality` (1 à 100) que le JPEG et le WebP avec perte.
fn encode_rgb(width: u32, height: u32, rgba: &[u8], format: ExportFormat, quality: u8, lossless: bool) -> Result<Vec<u8>, EngineError> {
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
        // Le JPEG n'a pas de transparence : l'image est posée sur du blanc.
        ExportFormat::Jpeg => encode_jpeg_rgb(&rgb_on_white(rgba), width, height, quality, None),
        ExportFormat::Pdf => crate::pdf::encode(width, height, rgba, &crate::pdf::PdfSettings { page: [width as f32, height as f32], cmyk: None, embed_profile: false, jpeg_quality: Some(quality), metadata: None, gray_bits: None, print: None }),
        ExportFormat::Avif => encode_avif(width, height, rgba, quality, AvifSpeed::default().preset(), None),
    }
}

/// Encodage du fichier avec tous ses réglages (CMJN, profil ICC, qualité du PDF), hors métadonnées. `page` : taille
/// de la page d'un PDF, en points.
fn encode(width: u32, height: u32, rgba: &[u8], options: &ExportOptions, page: [f32; 2]) -> Result<Vec<u8>, EngineError> {
    encode_with(width, height, rgba, options, page, None)
}

/// Le fichier complet, métadonnées comprises : écrites par le PDF et l'AVIF eux-mêmes (à l'encodage), ajoutées après
/// l'encodage ailleurs.
fn encode_file(width: u32, height: u32, rgba: &[u8], options: &ExportOptions, page: [f32; 2]) -> Result<Vec<u8>, EngineError> {
    let metadata = options.metadata.as_ref();
    if matches!(options.format, ExportFormat::Pdf | ExportFormat::Avif) {
        return encode_with(width, height, rgba, options, page, metadata);
    }
    with_metadata(options.format, encode(width, height, rgba, options, page)?, metadata)
}

fn encode_with(width: u32, height: u32, rgba: &[u8], options: &ExportOptions, page: [f32; 2], metadata: Option<&ImageMetadata>) -> Result<Vec<u8>, EngineError> {
    if let Some(bits) = options.gray_bits() {
        return encode_gray(width, height, rgba, options, bits, page, metadata);
    }
    // WebP et AVIF en gris ou en bitmap : fichier RVB, pixels ramenés au gris (ou au noir et blanc).
    let toned = options.tone_pixels(rgba);
    let rgba: &[u8] = &toned;
    let icc = options.icc()?;
    let icc = icc.as_deref();
    match (options.format, options.cmyk()) {
        (ExportFormat::Pdf, cmyk) => crate::pdf::encode(
            width,
            height,
            rgba,
            &crate::pdf::PdfSettings { page, cmyk: if options.pdf_print.is_some() { Some(options.cmyk_space) } else { cmyk }, embed_profile: options.embed_profile, jpeg_quality: options.pdf_quality.jpeg_quality(), metadata, gray_bits: None, print: options.pdf_print.as_ref() },
        ),
        // Ni CMJN ni profil ICC (`supports_cmyk` est faux) : `options.cmyk()` vaut toujours `None` ici.
        (ExportFormat::Avif, _) => encode_avif(width, height, rgba, options.quality, options.avif_speed.preset(), metadata),
        (ExportFormat::Jpeg, Some(space)) => encode_jpeg_cmyk(&crate::cmyk::rgba_to_cmyk(rgba, space)?, width, height, options.quality, icc),
        (ExportFormat::Jpeg, None) => encode_jpeg_rgb(&rgb_on_white(rgba), width, height, options.quality, icc),
        (ExportFormat::Tiff, Some(space)) => {
            // L'encodeur TIFF de la bibliothèque `image` ne connaît pas le CMJN : les quatre encres sont écrites comme
            // quatre composantes 8 bits, puis l'IFD est réécrit pour les déclarer (PhotometricInterpretation = 5,
            // séparations ; InkSet = 1, CMJN), sans ExtraSamples : la quatrième composante est le noir, pas un alpha.
            let cmyk = crate::cmyk::rgba_to_cmyk(rgba, space)?;
            let mut out = std::io::Cursor::new(Vec::new());
            image::codecs::tiff::TiffEncoder::new(&mut out)
                .write_image(&cmyk, width, height, ExtendedColorType::Rgba8)
                .map_err(|e| EngineError::Corrupt(format!("encodage impossible : {e}")))?;
            use crate::metadata::TiffValue;
            let mut entries = vec![(262, TiffValue::Short(5)), (332, TiffValue::Short(1))];
            if let Some(icc) = icc {
                entries.push((34675, TiffValue::Undefined(icc.to_vec())));
            }
            crate::metadata::tiff_with_entries(out.into_inner(), entries, &[317, 338])
        }
        (format, _) => {
            let bytes = encode_rgb(width, height, rgba, format, options.quality, options.lossless)?;
            match (format, icc) {
                (ExportFormat::Tiff, Some(icc)) => crate::metadata::tiff_with_entries(bytes, vec![(34675, crate::metadata::TiffValue::Undefined(icc.to_vec()))], &[]),
                _ => Ok(bytes),
            }
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
    /// Image affichable par la webview pour l'aperçu : le fichier lui-même, ou son équivalent PNG pour le TIFF et le
    /// PDF (qu'aucun navigateur n'affiche) et pour le CMJN (épreuve écran du profil).
    pub display: Vec<u8>,
}

// ------------------------------------------------------------------ aperçu de l'export

/// Image composée et mise à la taille d'export (RGBA, alpha droit). Calculée une fois, puis réutilisée par l'aperçu
/// tant que le document et la taille ne changent pas : changer de format ou de qualité ne recompose rien.
pub struct Flattened {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Taille de la page d'un PDF, en points (celle du document à sa résolution).
    pub page: [f32; 2],
}

pub fn flatten_for_export(doc: &Document, scale: f32) -> Result<Flattened, EngineError> {
    let (width, height, rgba) = flattened(doc, scale, &|_, _| {})?;
    Ok(Flattened { width, height, rgba, page: page_of(doc) })
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

    /// Vue d'ensemble, côté long ≤ `max_side`, encodée pour la webview : avec perte (JPEG, WebP, PDF en JPEG), dans
    /// ce format à cette qualité (on voit ses défauts) ; sans perte, en PNG rapide ou JPEG 95. En CMJN, l'image
    /// montrée est l'épreuve écran du profil. Renvoie (largeur, hauteur, octets, format « jpeg », « webp » ou « png »).
    pub fn overview(&self, max_side: u32, options: &ExportOptions) -> Result<(u32, u32, Vec<u8>, &'static str), EngineError> {
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
        let small = options.display_pixels(&small)?;
        // Format avec perte : on voit sa qualité. Sans perte : JPEG à 95 % si l'image est opaque (dix fois plus léger
        // à transmettre, indiscernable à cette taille), PNG rapide si elle a de la transparence (damier visible).
        Ok(match options.lossy_preview() {
            Some((format, quality)) => (w, h, encode_rgb(w, h, &small, format, quality, false)?, kind_of(format)),
            None if small.par_chunks_exact(4).all(|px| px[3] == 255) => (w, h, encode_rgb(w, h, &small, ExportFormat::Jpeg, 95, false)?, "jpeg"),
            None => (w, h, crate::png_parallel::encode(w, h, &small, 4, 1)?, "png"),
        })
    }

    /// Rectangle visible à pleine résolution, encodé comme il le sera dans le fichier (JPEG, WebP et PDF avec perte :
    /// leurs défauts de compression sont visibles ; formats sans perte : PNG rapide, pixels identiques ; CMJN :
    /// épreuve écran).
    pub fn region(&self, rect: [u32; 4], options: &ExportOptions) -> Result<(u32, u32, Vec<u8>, &'static str), EngineError> {
        let (w, h, pixels) = self.crop(rect);
        let pixels = options.display_pixels(&pixels)?;
        Ok(match options.lossy_preview() {
            Some((format, quality)) => (w, h, encode_rgb(w, h, &pixels, format, quality, false)?, kind_of(format)),
            None => (w, h, crate::png_parallel::encode(w, h, &pixels, 4, 1)?, "png"),
        })
    }

    /// Poids estimé du fichier, vite : 8 bandes réparties sur la hauteur (environ un sixième de l'image) sont
    /// encodées, et leur poids extrapolé. Une petite image est encodée entièrement : le poids est alors exact.
    pub fn estimate_size(&self, options: &ExportOptions) -> Result<(u64, bool), EngineError> {
        // L'AVIF est lent à encoder : poids exact jusqu'à un demi-mégapixel seulement (moins d'une demi-seconde).
        let exact_up_to = if options.format == ExportFormat::Avif { AVIF_SAMPLE_PIXELS } else { 2_000_000 };
        if u64::from(self.width) * u64::from(self.height) <= exact_up_to {
            return Ok((self.exact_size(options)?, true));
        }
        let vignette = || self.crop([0, 0, 16, 16]);
        // Les métadonnées pèsent la même chose quelle que soit l'image : mesurées sur une vignette.
        let extra = match &options.metadata {
            Some(meta) if !meta.is_empty() => {
                let (w, h, pixels) = vignette();
                let plain = encode(w, h, &pixels, options, self.page)?;
                let full = encode_file(w, h, &pixels, options, self.page)?;
                full.len().saturating_sub(plain.len()) as u64
            }
            _ => 0,
        };
        if options.format == ExportFormat::Avif {
            return Ok((self.estimate_avif(options)? + extra, false));
        }
        if options.format == ExportFormat::Png {
            // Gris : un octet par pixel (deux avec l'alpha) ; 1 bit : lignes de bits.
            let estimate = match options.gray_bits() {
                Some(1) => crate::png_parallel::estimate(self.width.div_ceil(8), self.height, &crate::color_mode::pack_bits_rows(&crate::color_mode::bilevel(&self.rgba), self.width, self.height), 1, 6, 12)?,
                Some(_) => match crate::color_mode::gray_planes(&self.rgba) {
                    (gray, None) => crate::png_parallel::estimate(self.width, self.height, &gray, 1, 6, 12)?,
                    (gray, Some(alpha)) => crate::png_parallel::estimate(self.width, self.height, &gray.iter().zip(&alpha).flat_map(|(g, a)| [*g, *a]).collect::<Vec<u8>>(), 2, 6, 12)?,
                },
                None => crate::png_parallel::estimate(self.width, self.height, &self.rgba, 4, 6, 12)?,
            };
            return Ok((estimate + extra, false));
        }
        // PDF sans perte : mêmes flux que le PNG (lignes filtrées, compressées par bandes), estimés de même ; la
        // structure du PDF (et son profil ICC) est mesurée sur une vignette.
        if options.format == ExportFormat::Pdf && options.pdf_quality.jpeg_quality().is_none() && options.gray_bits().is_none() {
            let (w, h) = (self.width, self.height);
            let header = {
                let (vw, vh, pixels) = vignette();
                encode(vw, vh, &pixels, options, self.page)?.len() as u64
            };
            let data = match options.cmyk() {
                Some(space) => crate::png_parallel::estimate(w, h, &crate::cmyk::rgba_to_cmyk(&self.rgba, space)?, 4, 6, 12)?,
                None => {
                    let rgb: Vec<u8> = self.rgba.par_chunks_exact(4).flat_map_iter(|px| [px[0], px[1], px[2]]).collect();
                    let mut data = crate::png_parallel::estimate(w, h, &rgb, 3, 6, 12)?;
                    if self.rgba.par_chunks_exact(4).any(|px| px[3] != 255) {
                        let alpha: Vec<u8> = self.rgba.par_chunks_exact(4).map(|px| px[3]).collect();
                        data += crate::png_parallel::estimate(w, h, &alpha, 1, 6, 12)?;
                    }
                    data
                }
            };
            return Ok((data + header + extra, false));
        }
        // Bandes alignées sur les blocs de 16 lignes du JPEG (et les macroblocs du WebP) ; le coût des en-têtes
        // (tables, marqueurs, structure du PDF, profil ICC), payé par chaque bande, est mesuré sur une vignette de
        // 16 × 16 et retranché.
        const STRIPS: u32 = 8;
        let strip = ((self.height / (STRIPS * 6)) / 16 * 16).max(64).min(self.height / 16 * 16).max(16);
        let header = {
            let (w, h, pixels) = vignette();
            encode(w, h, &pixels, options, self.page)?.len() as u64
        };
        let sizes: Vec<u64> = (0..STRIPS)
            .into_par_iter()
            .map(|i| {
                let y = (self.height - strip) * i / (STRIPS - 1) / 16 * 16;
                let (w, h, pixels) = self.crop([0, y, self.width, strip]);
                encode(w, h, &pixels, options, self.page).map(|bytes| bytes.len() as u64)
            })
            .collect::<Result<_, EngineError>>()?;
        let data: u64 = sizes.iter().map(|size| size.saturating_sub(header)).sum();
        let sampled = u64::from(strip * STRIPS);
        Ok((data * u64::from(self.height) / sampled.max(1) + header + extra, false))
    }

    /// Poids estimé d'un AVIF (hors métadonnées), sans encoder l'image entière (plusieurs secondes) : une trentaine de
    /// carrés de 128 px (un demi-mégapixel en tout), pris À PLEINE RÉSOLUTION, un par case d'une grille aux proportions
    /// de l'image, à une place tirée au hasard (graine fixe) dans sa case, sont encodés chacun à part, en parallèle,
    /// à la vitesse choisie, avec leur quart central ; le coût par pixel qui s'en déduit (bords et coût fixe d'une
    /// image retirés) est extrapolé à la surface de l'image. Durée (2 cœurs) : 0,2 à 0,3 s à la vitesse Rapide, 0,4 à
    /// 0,7 s aux deux autres, quelle que soit la taille de l'image.
    ///
    /// Pourquoi ainsi (mesures sur six images, photo, captures d'écran, motifs ; voir `avif_estimate_calibration`) :
    /// une réduction de toute l'image garde plus de détail par pixel et surestime d'un facteur qui dépend de l'image ;
    /// une mosaïque de carrés recollés paie ses jointures ; des carrés encodés seuls paient leurs bords (sans voisins
    /// pour prédire : +20 à +75 %, d'où la correction par le quart central) ; une grille sans hasard tombe en phase avec
    /// la mise en page d'une capture ; un échantillon encodé à la vitesse 10 puis corrigé se trompe selon le contenu (le
    /// gain des vitesses lentes va de 0 % sur une photo à 55 % sur certaines captures). Écart restant à la vitesse
    /// Rapide : de −23 % à +20 % sur photos et captures, +50 % sur un damier de grands aplats ; davantage aux vitesses
    /// lentes sur une image dont le gain est exceptionnel. L'interface l'affiche « ≈ ».
    fn estimate_avif(&self, options: &ExportOptions) -> Result<u64, EngineError> {
        const SQUARE: u32 = 128;
        let preset = options.avif_speed.preset();
        let (tw, th) = (SQUARE.min(self.width), SQUARE.min(self.height));
        let squares = (AVIF_SAMPLE_PIXELS / u64::from(tw * th)).max(1) as f64;
        let aspect = (f64::from(self.width) / f64::from(tw)) / (f64::from(self.height) / f64::from(th));
        let columns = ((squares * aspect).sqrt().round() as u32).clamp(1, (self.width / tw).max(1));
        let rows = ((squares / f64::from(columns)).round() as u32).clamp(1, (self.height / th).max(1));
        // Place dans la case : hachage splitmix64 du numéro de case, pour une estimation reproductible.
        let jitter = |n: u64| {
            let mut z = n.wrapping_add(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) % 1024) as f64 / 1024.0
        };
        let cells: Vec<[u32; 4]> = (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (row, column)))
            .map(|(row, column)| {
                let cell = u64::from(row) * u64::from(columns) + u64::from(column);
                let x = ((f64::from(column) + jitter(2 * cell)) * f64::from(self.width - tw) / f64::from(columns)) as u32;
                let y = ((f64::from(row) + jitter(2 * cell + 1)) * f64::from(self.height - th) / f64::from(rows)) as u32;
                [x, y, tw, th]
            })
            .collect();
        // Coût fixe d'un carré (conteneur, en-tête de séquence, image minimale) : un carré uni, opaque ou non selon
        // l'image (l'alpha ajoute son propre flux).
        let alpha = if self.rgba.par_chunks_exact(4).any(|px| px[3] != 255) { 128 } else { 255 };
        let uniform = [128u8, 128, 128, alpha].repeat((tw * th) as usize);
        let overhead = encode_avif(tw, th, &uniform, options.quality, preset, None)?.len() as u64;
        // Chaque carré est encodé deux fois : entier, et son quart central (côté moitié). Le poids d'un carré, moins
        // son coût fixe, vaut a × surface + b × périmètre (b : surcoût des bords, sans voisins pour prédire) : avec
        // deux tailles, b s'élimine, a = [(P - fixe) - 2 (p - fixe)] / (surface - 2 × surface du quart).
        let (hw, hh) = ((tw / 2).max(1), (th / 2).max(1));
        let half_uniform = [128u8, 128, 128, alpha].repeat((hw * hh) as usize);
        let half_overhead = encode_avif(hw, hh, &half_uniform, options.quality, preset, None)?.len() as f64;
        let per_pixel: f64 = cells
            .par_iter()
            .map(|&[x, y, w, h]| {
                let (_, _, whole) = self.crop([x, y, w, h]);
                let (_, _, center) = self.crop([x + (w - hw) / 2, y + (h - hh) / 2, hw, hh]);
                let big = encode_avif(w, h, &whole, options.quality, preset, None)?.len() as f64 - overhead as f64;
                let small = encode_avif(hw, hh, &center, options.quality, preset, None)?.len() as f64 - half_overhead;
                Ok(((big - 2.0 * small) / (f64::from(w * h) - 2.0 * f64::from(hw * hh))).max(0.0))
            })
            .sum::<Result<f64, EngineError>>()?;
        Ok((per_pixel / cells.len() as f64 * f64::from(self.width) * f64::from(self.height)) as u64 + overhead)
    }

    /// Poids exact : l'image entière encodée comme à l'export, métadonnées comprises.
    pub fn exact_size(&self, options: &ExportOptions) -> Result<u64, EngineError> {
        Ok(encode_file(self.width, self.height, &self.rgba, options, self.page)?.len() as u64)
    }
}

pub fn export_encoded(doc: &Document, options: ExportOptions) -> Result<Encoded, EngineError> {
    let view = export_view(doc, &options)?;
    let doc = view.as_ref();
    let (width, height, rgba) = flattened(doc, options.scale, &|_, _| {})?;
    let bytes = encode_file(width, height, &rgba, &options, page_of(doc))?;
    let size = bytes.len() as u64;
    // Le fichier lui-même s'il s'affiche tel quel (PNG, WebP, JPEG RVB) ; sinon son équivalent PNG.
    let shown_as_is = matches!(options.format, ExportFormat::Png | ExportFormat::Webp) || (options.format == ExportFormat::Jpeg && options.cmyk().is_none());
    let display = if shown_as_is { bytes } else { crate::png_parallel::encode(width, height, &options.display_pixels(&rgba)?, 4, 6)? };
    Ok(Encoded { width, height, size, display })
}

/// Écrit l'export sur disque (écriture atomique : jamais de fichier à moitié écrit).
pub fn export_image(doc: &Document, path: &Path, options: ExportOptions) -> Result<(), EngineError> {
    export_image_with_progress(doc, path, options, &|_, _| {})
}

/// Comme [`export_image`], en tenant l'appelant informé : `progress(part de 0 à 1, étape en cours)`.
pub fn export_image_with_progress(doc: &Document, path: &Path, options: ExportOptions, progress: &dyn Fn(f32, &str)) -> Result<(), EngineError> {
    let view = export_view(doc, &options)?;
    let doc = view.as_ref();
    let (width, height, rgba) = flattened(doc, options.scale, progress)?;
    // L'AVIF prend plusieurs secondes (rav1e, d'un seul tenant : pas d'avancement intermédiaire) : on le dit.
    let step = match options.format {
        _ if options.cmyk().is_some() => "Conversion en CMJN et encodage",
        _ if options.gray_bits().is_some() => "Conversion en niveaux de gris et encodage",
        ExportFormat::Avif => "Encodage AVIF",
        _ => "Encodage du fichier",
    };
    progress(0.7, step);
    let bytes = encode_file(width, height, &rgba, &options, page_of(doc))?;
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
        // Calque de remplissage : son rendu se recalcule à l'ouverture, inutile de l'écrire.
        if let Some(pixels) = layer.pixels().filter(|_| layer.fill_content().is_none()) {
            jobs.extend(pixels.iter().map(|(c, t)| Job::Pixels(scope, layer.id, c, t)));
        }
        if let Some(mask) = &layer.mask {
            jobs.extend(mask.pixels.iter().map(|(c, t)| Job::Mask(scope, layer.id, c, t)));
        }
        if let Some(filters) = &layer.filters {
            jobs.extend(filters.source.iter().map(|(c, t)| Job::Pixels(scope, layer.id | FILTER_KEY, c, t)));
            if let Some(mask) = &filters.mask {
                jobs.extend(mask.pixels.iter().map(|(c, t)| Job::Mask(scope, layer.id | FILTER_KEY, c, t)));
            }
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
            SmartSource::Embedded { original: Some(original) } => {
                originals.push((original_entry(*key), Arc::clone(&original.bytes)));
                // RAW à redévelopper : son développement est enregistré avec lui (voir `SourceV7::Original`).
                if original.develop.is_some() {
                    jobs.extend(content.flattened.iter().map(|(c, t)| Job::Flat(*key, c, t)));
                }
            }
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
    // Les tuiles sont rangées par nom d'entrée avant l'encodage, puis encodées PAR LOTS, en parallèle, pendant que
    // l'archive s'écrit : jamais tout le projet encodé en mémoire à la fois (un gros projet pesait le double).
    let mut jobs: Vec<(String, Job<'_>)> = jobs
        .into_iter()
        .map(|job| {
            let name = match &job {
                Job::Pixels(scope, id, coord, _) => tile_entry(&tiles_folder(*scope, "layers", *id), *coord),
                Job::Mask(scope, id, coord, _) => tile_entry(&tiles_folder(*scope, "masks", *id), *coord),
                Job::Flat(key, coord, _) => tile_entry(&flat_folder(*key), *coord),
            };
            (name, job)
        })
        .collect();
    jobs.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let encode_batch = |batch: &[(String, Job<'_>)]| -> Result<Vec<(String, Vec<u8>)>, EngineError> {
        batch
            .par_iter()
            .map(|(name, job)| {
                let bytes = match job {
                    Job::Pixels(_, _, _, tile) | Job::Flat(_, _, tile) => straight_png(tile, path)?,
                    Job::Mask(_, _, _, tile) => encode_png(tile.bytes(), ExtendedColorType::L8, path)?,
                };
                Ok((name.clone(), bytes))
            })
            .collect()
    };

    let manifest = Manifest {
        format: FORMAT_NAME.into(),
        // La plus ancienne version suffisante : le fichier reste lisible par les versions précédentes tant que possible.
        format_version: format_version_for(&structure),
        app_version: env!("CARGO_PKG_VERSION").into(),
    };

    // Profil CMJN personnalisé du mode Couleurs CMJN (document ou contenu incorporé) : gardé dans l'archive, le
    // projet se rouvre à l'identique sur un ordinateur qui ne l'a pas.
    let modes = std::iter::once(&structure.color_mode).chain(structure.contents.iter().filter_map(|c| match &c.source {
        SourceV7::Embedded { document } => Some(&document.color_mode),
        _ => None,
    }));
    let mut profiles: Vec<(String, Arc<[u8]>)> = Vec::new();
    for space in modes.filter_map(|m| m.cmyk_space()) {
        let name = profile_entry(space.profile);
        if let (Some(bytes), false) = (crate::cmyk::custom_icc(space.profile), profiles.iter().any(|(n, _)| *n == name)) {
            profiles.push((name, bytes));
        }
    }

    write_atomically(path, |file| {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let mut zip = ZipWriter::new(BufWriter::new(file));
        zip.start_file("manifest.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest)?).map_err(EngineError::io(path))?;
        zip.start_file("document.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(&structure)?).map_err(EngineError::io(path))?;
        // Un fil encode les lots (chacun sur tous les cœurs) avec au plus deux lots d'avance ; ce fil-ci écrit.
        const BATCH: usize = 64;
        let (send, receive) = std::sync::mpsc::sync_channel::<Result<Vec<(String, Vec<u8>)>, EngineError>>(2);
        std::thread::scope(|scope| -> Result<(), EngineError> {
            let jobs = &jobs;
            scope.spawn(move || {
                for batch in jobs.chunks(BATCH) {
                    if send.send(encode_batch(batch)).is_err() {
                        break;
                    }
                }
            });
            for batch in receive {
                for (name, bytes) in batch? {
                    zip.start_file(name.as_str(), options)?;
                    zip.write_all(&bytes).map_err(EngineError::io(path))?;
                }
            }
            Ok(())
        })?;
        for (name, bytes) in originals.iter().map(|(n, b)| (n, &b[..])).chain(profiles.iter().map(|(n, b)| (n, &b[..]))) {
            zip.start_file(name.as_str(), options)?;
            zip.write_all(bytes).map_err(EngineError::io(path))?;
        }
        zip.finish()?.flush().map_err(EngineError::io(path))
    })
}

/// Entrée d'archive d'un profil CMJN personnalisé (projet, copie de secours).
fn profile_entry(profile: CmykProfile) -> String {
    format!("profils/{}.icc", profile.id())
}

/// Rend disponibles les profils CMJN gardés dans une archive. Un profil illisible est ignoré : le document s'ouvre,
/// et l'épreuve signale le profil manquant.
fn register_archived_profiles<R: Read + Seek>(zip: &mut ZipArchive<R>, path: &Path) {
    let names: Vec<String> = zip.file_names().filter(|n| n.starts_with("profils/") && n.ends_with(".icc")).map(str::to_string).collect();
    for name in names {
        if let Ok(bytes) = read_entry(zip, &name, path) {
            let _ = crate::cmyk::register_icc(&bytes);
        }
    }
}

pub fn load_project(path: &Path) -> Result<Document, EngineError> {
    let file = File::open(path).map_err(EngineError::io(path))?;
    load_project_from(BufReader::new(file), path)
}

/// Variante depuis la mémoire (fichier déposé dans la fenêtre).
pub fn load_project_bytes(bytes: &[u8], origin: &Path) -> Result<Document, EngineError> {
    load_project_from(Cursor::new(bytes), origin)
}

/// Styles de caractère d'un projet, lus sans charger ses calques (« Importer d'un projet… »).
pub fn project_text_styles(path: &Path) -> Result<Vec<crate::text_styles::CharStyle>, EngineError> {
    #[derive(Deserialize)]
    struct OnlyStyles {
        #[serde(default)]
        text_styles: Vec<crate::text_styles::CharStyle>,
    }
    let file = File::open(path).map_err(EngineError::io(path))?;
    let mut zip = ZipArchive::new(BufReader::new(file)).map_err(|_| EngineError::NotAProject)?;
    let json = read_entry(&mut zip, "document.json", path)?;
    Ok(serde_json::from_slice::<OnlyStyles>(&json).map(|s| s.text_styles).unwrap_or_default())
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
    register_archived_profiles(&mut zip, path);
    let json = read_entry(&mut zip, "document.json", path)?;
    let mut structure: DocumentV2 = match manifest.format_version {
        1 => serde_json::from_slice::<DocumentV1>(&json)?.upgrade(),
        2..=FORMAT_VERSION => serde_json::from_slice(&json)?,
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
            SourceV7::Original { file_name, develop: None, .. } => {
                let bytes = source.original(key)?;
                let document = import_image_bytes(&bytes, Path::new(&file_name))?;
                SmartContent::new(id, content.name, Arc::new(document), SmartSource::Embedded { original: Some(OriginalFile { name: file_name, bytes: Arc::new(bytes), develop: None }) })
            }
            SourceV7::Original { file_name, develop: Some(develop), tiles } => {
                let bytes = Arc::new(source.original(key)?);
                let original = OriginalFile { name: file_name.clone(), bytes: Arc::clone(&bytes), develop: Some(develop) };
                if tiles.is_empty() {
                    // Développement absent (fichier écrit à la main) : refait avec les réglages enregistrés.
                    let developed = crate::raw::RawSession::open(Arc::clone(&bytes), Path::new(&file_name))?.develop_full(&develop);
                    let document = document_from_premultiplied(developed.width, developed.height, &developed.to_rgba(), Path::new(&file_name))?;
                    SmartContent::new(id, content.name, Arc::new(document), SmartSource::Embedded { original: Some(original) })
                } else {
                    // Le développement enregistré, tel quel : le contenu est un document d'un seul calque.
                    let flat = source.flat(key, &tiles)?;
                    let mut document = Document::new(file_stem(Path::new(&file_name)), content.width, content.height, 72.0)?;
                    document.add_pixel_layer("Arrière-plan", flat.clone(), None, 0)?;
                    SmartContent::with_flattened(id, content.name, Arc::new(document), SmartSource::Embedded { original: Some(original) }, Arc::new(flat))
                }
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
    // Projets d'avant les repères par plan : chaque repère rejoint le plan où il tombe.
    doc.adopt_orphan_guides();
    doc.paths = structure.paths;
    doc.color_mode = structure.color_mode;
    doc.text_styles = structure.text_styles;
    doc.print = structure.print;
    doc.work_seconds = structure.work_seconds;
    for layer in structure.layers {
        let mask = match layer.mask {
            None => None,
            Some(m) => Some(Mask { pixels: Arc::new(source.mask(scope, layer.id, &m)?), enabled: m.enabled, linked: m.linked }),
        };
        let kind = match layer.kind {
            KindV2::Group { children, expanded } => LayerKind::Group { children, expanded },
            KindV2::Pixel { tiles } => LayerKind::Pixel { pixels: Arc::new(source.pixels(scope, layer.id, &tiles)?) },
            KindV2::Adjustment { adjustment } => LayerKind::Adjustment { adjustment },
            KindV2::Shape { shape, tiles } => LayerKind::Shape { shape, raster: Arc::new(source.pixels(scope, layer.id, &tiles)?) },
            KindV2::Fill { fill_layer: fill } => {
                let (width, height) = (doc.width(), doc.height());
                let raster = crate::fill::render(&fill, (width, height), (-layer.offset[0], -layer.offset[1], width, height))?;
                LayerKind::Fill { fill, raster: Arc::new(raster) }
            }
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
            blend_if: layer.blend_if.map(crate::blend::BlendIf::normalized).filter(|r| !r.is_identity()),
        };
        let offset = (layer.offset[0], layer.offset[1]);
        // Filtres dynamiques : seulement sur un calque de pixels (le résultat est déjà dans ses pixels).
        let filters = match (layer.filters, &kind) {
            (Some(f), LayerKind::Pixel { .. }) => Some(crate::smart_filters::LayerFilters {
                list: f.list,
                enabled: f.enabled,
                source: Arc::new(source.pixels(scope, layer.id | FILTER_KEY, &f.source)?),
                mask: match f.mask {
                    Some(m) => Some(Mask { pixels: Arc::new(source.mask(scope, layer.id | FILTER_KEY, &m)?), enabled: m.enabled, linked: m.linked }),
                    None => None,
                },
            }),
            _ => None,
        };
        doc.restore(Layer { id: layer.id, parent: layer.parent, label: layer.label, offset, props, mask, effects: layer.effects, filters, artboard: layer.artboard, kind }, structure.root.clone());
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
    /// Dernier rendu d'un contenu lié, ou développement d'un RAW à redévelopper.
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
            if let Some(filters) = &layer.filters {
                pixels.insert(layer.id | FILTER_KEY, self.pixels(&filters.source));
                if let Some(mask) = &filters.mask {
                    masks.insert(layer.id | FILTER_KEY, self.masks(&mask.pixels));
                }
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
                SmartSource::Embedded { original: Some(original) } => RecoveryContent {
                    original: Some(unique.original(&original.bytes)),
                    // RAW à redévelopper : son développement, comme pour un projet.
                    flat: if original.develop.is_some() { unique.pixels(&content.flattened) } else { Vec::new() },
                    ..Default::default()
                },
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
        if n.is_multiple_of(16) {
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
        // Profils CMJN personnalisés : tous ceux de la session (l'historique peut en avoir employé plusieurs).
        for (profile, bytes) in crate::cmyk::registered_custom() {
            zip.start_file(profile_entry(profile), options)?;
            zip.write_all(&bytes).map_err(EngineError::io(path))?;
        }
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
    register_archived_profiles(&mut zip, path);
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
            let image = decode(bytes)?;
            if (image.width(), image.height()) != (TILE_SIZE, TILE_SIZE) {
                return Err(corrupt_recovery());
            }
            let rgba = premultiplied_rgba(image)?;
            let mut tile = TileBuf::<4>::filled([0; 4]);
            tile.bytes_mut().copy_from_slice(&rgba);
            Ok(Arc::new(tile))
        })
        .collect::<Result<_, EngineError>>()?;
    let mask_tiles: Vec<Arc<MaskTile>> = raw_masks
        .par_iter()
        .map(|bytes| {
            let image = decode(bytes)?;
            if (image.width(), image.height()) != (TILE_SIZE, TILE_SIZE) {
                return Err(corrupt_recovery());
            }
            let mut tile = TileBuf::<1>::filled([0]);
            tile.bytes_mut().copy_from_slice(&image.into_luma8().into_raw());
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

/// Lit une entrée de l'archive en entier. La taille annoncée dans l'en-tête sert d'indication pour réserver la
/// mémoire, plafonnée : un fichier forgé pourrait annoncer des téraoctets et faire avorter le programme avant
/// même la lecture (qui, elle, échouerait proprement).
fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str, path: &Path) -> Result<Vec<u8>, EngineError> {
    const RESERVE_LIMIT: u64 = 64 << 20;
    let mut entry = zip.by_name(name)?;
    let mut bytes = Vec::with_capacity(entry.size().min(RESERVE_LIMIT) as usize);
    entry.read_to_end(&mut bytes).map_err(EngineError::io(path))?;
    Ok(bytes)
}

/// Écrit dans `<path>.tmp`, force l'écriture sur le disque, puis renomme. Sans `sync_all`, un renommage peut être
/// journalisé avant les données : après une coupure de courant, le fichier existe mais est vide ou tronqué.
/// Le fichier temporaire est supprimé en cas d'échec.
fn write_atomically(path: &Path, write: impl FnOnce(&mut File) -> Result<(), EngineError>) -> Result<(), EngineError> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = Path::new(&tmp);
    let result = File::create(tmp)
        .map_err(EngineError::io(tmp))
        .and_then(|mut file| {
            write(&mut file)?;
            file.sync_all().map_err(EngineError::io(tmp))
        })
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
        let pdf = |pdf_quality, color| ExportOptions { pdf_quality, color, embed_profile: true, ..ExportOptions::new(ExportFormat::Pdf) };
        for options in [ExportOptions::new(ExportFormat::Png), ExportOptions::new(ExportFormat::Jpeg), pdf(PdfQuality::High, ColorMode::Cmyk), pdf(PdfQuality::Lossless, ColorMode::Rgb)] {
            let format = (options.format, options.color);
            let (estimate, exact) = flat.estimate_size(&options).expect("estimation");
            let real = flat.exact_size(&options).expect("poids");
            let gap = (estimate as f64 - real as f64).abs() / real as f64;
            assert!(!exact && gap < 0.15, "{format:?} : estimé {estimate}, réel {real} ({:.1} %)", gap * 100.0);
        }
        // AVIF : estimé sur ~5 % de l'image (30 carrés de 128 px et leur centre, encodés à part), sans l'encoder entière.
        // Tolérance de 30 % : c'est un ordre de grandeur affiché « ≈ » ; l'écart mesuré sur de vraies photos et captures
        // va de −23 % à +20 % (voir `estimate_avif`), −26 % sur ce motif (dégradés lisses, bruit effacé par l'AV1 :
        // 12 Ko pour 6 Mpx, où le moindre écart par carré pèse lourd).
        let avif = ExportOptions::new(ExportFormat::Avif);
        let started = std::time::Instant::now();
        let (estimate, exact) = flat.estimate_size(&avif).expect("estimation");
        let estimated_in = started.elapsed();
        let real = flat.exact_size(&avif).expect("poids");
        let gap = (estimate as f64 - real as f64).abs() / real as f64;
        assert!(!exact && gap < 0.30, "AVIF : estimé {estimate}, réel {real} ({:.1} %, estimé en {estimated_in:?})", gap * 100.0);
        eprintln!("AVIF 6 Mpx : estimé {estimate} o en {estimated_in:?}, réel {real} o ({:+.1} %)", (estimate as f64 / real as f64 - 1.0) * 100.0);
        // La zone visible, encodée : dimensions demandées, image lisible.
        let (w, h, png, kind) = flat.region([2900, 1950, 400, 300], &ExportOptions::new(ExportFormat::Png)).expect("zone");
        assert_eq!((w, h, kind), (100, 50, "png"), "rognée au bord de l'image");
        let decoded = image::load_from_memory(&png).expect("png");
        assert_eq!((decoded.width(), decoded.height()), (100, 50));
        let (ow, oh, _, _) = flat.overview(600, &ExportOptions::new(ExportFormat::Jpeg)).expect("vue d'ensemble");
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

    #[test]
    fn camera_raws_are_recognised_whatever_the_case_and_never_read_as_tiff() {
        for name in ["DSC0001.ARW", "img.cr3", "photo.NEF", "x.Raf", "a.dng", "b.ORF", "c.rw2"] {
            assert!(is_raw_path(Path::new(name)), "{name}");
        }
        assert!(!is_raw_path(Path::new("photo.jpg")) && !is_raw_path(Path::new("scan.tif")) && !is_raw_path(Path::new("sans-extension")));
        // Un ARW est un TIFF : sans la reconnaissance du RAW, le décodeur TIFF n'y lirait que la vignette embarquée.
        // Un faux ARW (en-tête TIFF, pas de capteur) doit être refusé par le décodeur RAW, nommément.
        let fake = b"II*\0\x08\0\0\0\0\0";
        let error = decode_premultiplied(fake, Path::new("DSC0001.ARW")).expect_err("faux RAW").to_string();
        assert!(error.contains("RAW") && error.contains("DSC0001.ARW"), "{error}");
        assert!(raw_preview(fake, Path::new("DSC0001.ARW"), 100).is_none());
        assert!(raw_preview(fake, Path::new("photo.png"), 100).is_none(), "pas un RAW : pas d'aperçu");
    }
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
        doc.guides.push(Guide { orientation: crate::document::Orientation::Vertical, position: 350.0, board: None });
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
    fn work_time_is_saved_with_the_project() {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("temps.graf");
        let mut doc = sample_document();
        assert_eq!(load_or(&doc, &path).work_seconds, 0, "ancien fichier ou rien de compté : 0");
        doc.work_seconds = 46_020; // 12 h 47
        save_project(&doc, &path).expect("enregistrement");
        assert_eq!(load_project(&path).expect("chargement").work_seconds, 46_020);
    }

    fn load_or(doc: &Document, path: &Path) -> Document {
        save_project(doc, path).expect("enregistrement");
        load_project(path).expect("chargement")
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

    /// Filtres dynamiques : la liste, l'œil, l'origine et le masque de filtre survivent à l'enregistrement ; le
    /// résultat aussi (pas de recalcul à l'ouverture).
    #[test]
    fn smart_filters_round_trip_with_their_source_and_mask() {
        use crate::filters::Filter;
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("filtres.graf");
        let mut doc = Document::new("Filtres", 300, 200, 72.0).expect("document");
        let id = doc.add_pixel_layer("Carré", TileStore::new(), None, 0).expect("calque");
        doc.pixels_mut(id).expect("pixels").fill(100, 100, [200, 30, 30, 255]);
        doc.set_selection(Some(crate::selection::Shape::Rect { x: 0.0, y: 0.0, width: 60.0, height: 100.0 }.rasterize(300, 200)));
        doc.apply_filter(id, &Filter::GaussianBlur { radius: 6.0 }).expect("flou");
        doc.set_selection(None);
        doc.apply_filter(id, &Filter::Posterize { levels: 3 }).expect("isohélie");
        save_project(&doc, &path).expect("enregistrement");
        let loaded = load_project(&path).expect("chargement");
        let (a, b) = (doc.layer(id).expect("calque"), loaded.layer(id).expect("calque relu"));
        let (fa, fb) = (a.filters.as_ref().expect("filtres"), b.filters.as_ref().expect("filtres relus"));
        assert_eq!(fa.list, fb.list);
        assert_eq!(fa.enabled, fb.enabled);
        assert_eq!(fa.source.sorted_coords(), fb.source.sorted_coords());
        assert_eq!(fa.source.get(ORIGIN).map(|t| t.pixel(50, 50)), fb.source.get(ORIGIN).map(|t| t.pixel(50, 50)), "origine conservée");
        assert!(fb.mask.is_some(), "masque de filtre conservé");
        assert_eq!(a.pixels().expect("pixels").get(ORIGIN).map(|t| t.pixel(99, 50)), b.pixels().expect("pixels").get(ORIGIN).map(|t| t.pixel(99, 50)), "résultat conservé");
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
            let encoded = export_encoded(&doc, ExportOptions { scale: 0.5, ..ExportOptions::new(format) }).expect("export");
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
        let small = export_encoded(&doc, ExportOptions { quality: 20, ..ExportOptions::new(ExportFormat::Jpeg) }).expect("export");
        let large = export_encoded(&doc, ExportOptions { quality: 95, ..ExportOptions::new(ExportFormat::Jpeg) }).expect("export");
        assert!(small.size < large.size, "la qualité fait varier le poids");

        let path = std::env::temp_dir().join(format!("lumitype-export-{}.tif", std::process::id()));
        let steps = std::sync::Mutex::new(Vec::new());
        export_image_with_progress(&doc, &path, ExportOptions::new(ExportFormat::Tiff), &|part, _| steps.lock().expect("verrou").push(part)).expect("écriture");
        let steps = steps.into_inner().expect("verrou");
        assert!(steps.windows(2).all(|w| w[0] <= w[1]) && steps.last() == Some(&1.0), "l'avancement ne recule jamais et finit à 100 % : {steps:?}");
        assert!(std::fs::read(&path).expect("fichier").starts_with(b"II"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn grayscale_and_bitmap_exports_are_real_gray_files() {
        use crate::color_mode::{BitmapMethod, BitmapOptions, ColorMode as Mode};
        let mut doc = Document::with_background("x", 64, 32, 72.0, Background::White).expect("document");
        let layer = doc.walk_top_down().last().expect("calque").1.id;
        doc.set_selection(Some(crate::selection::Shape::Rect { x: 0.0, y: 0.0, width: 32.0, height: 32.0 }.rasterize(64, 32)));
        doc.fill_layer(layer, [200, 40, 40]).expect("moitié rouge");
        doc.set_selection(None);
        doc.convert_color_mode(Mode::Grayscale, None).expect("gris");
        let gray = crate::color_mode::luma(200, 40, 40);
        let rgb_png = export_encoded(&doc, ExportOptions::new(ExportFormat::Png)).expect("png rvb");
        for format in [ExportFormat::Png, ExportFormat::Tiff, ExportFormat::Jpeg] {
            let encoded = export_encoded(&doc, ExportOptions { color: ColorMode::Gray, ..ExportOptions::new(format) }).expect("export");
            let bytes = encode_file(64, 32, &flattened(&doc, 1.0, &|_, _| {}).expect("aplat").2, &ExportOptions { color: ColorMode::Gray, ..ExportOptions::new(format) }, [64.0, 32.0]).expect("fichier");
            let image = image::load_from_memory(&bytes).expect("relu");
            assert_eq!(image.color(), image::ColorType::L8, "{format:?} : un vrai fichier gris 8 bits");
            let value = image.into_luma8().get_pixel(10, 10).0[0];
            assert!(value.abs_diff(gray) <= 2, "{format:?} : {value} au lieu de {gray}");
            if format == ExportFormat::Png {
                assert!(encoded.size < rgb_png.size, "plus léger que le RVB : {} contre {}", encoded.size, rgb_png.size);
            }
        }
        doc.convert_color_mode(Mode::Bitmap, Some(&BitmapOptions { method: BitmapMethod::Threshold, output_dpi: 72.0 })).expect("bitmap");
        let rgba = flattened(&doc, 1.0, &|_, _| {}).expect("aplat").2;
        for format in [ExportFormat::Png, ExportFormat::Tiff] {
            let bytes = encode_file(64, 32, &rgba, &ExportOptions { color: ColorMode::Bitmap, ..ExportOptions::new(format) }, [64.0, 32.0]).expect("fichier");
            match format {
                // IHDR : profondeur 1, type 0 (gris).
                ExportFormat::Png => assert_eq!(&bytes[24..26], &[1, 0], "PNG 1 bit"),
                _ => assert_eq!(image::codecs::tiff::TiffDecoder::new(std::io::Cursor::new(&bytes)).map(|d| image::ImageDecoder::original_color_type(&d)).expect("tiff"), ExtendedColorType::L1, "TIFF 1 bit"),
            }
            let image = image::load_from_memory(&bytes).expect("relu").into_luma8();
            assert_eq!(image.get_pixel(10, 10).0[0], 0, "{format:?} : rouge sombre -> noir");
            assert_eq!(image.get_pixel(50, 10).0[0], 255, "{format:?} : blanc");
        }
    }

    #[test]
    fn raw_dimensions_are_those_of_the_developed_image() {
        let raw = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mire.dng");
        let bytes = std::fs::read(&raw).expect("fixture");
        let developed = decode_raw(&bytes, &raw).expect("développement");
        assert_eq!(raw_dimensions(&bytes, &raw), Some(developed.dimensions()));
        assert_eq!(raw_dimensions(b"pas un raw", Path::new("x.png")), None);
    }

    #[test]
    fn raw_thumbnails_fit_the_requested_size_and_refuse_other_files() {
        let raw = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mire.dng");
        let png = raw_thumbnail_png(&raw, 64).expect("miniature");
        let image = image::load_from_memory(&png).expect("PNG");
        assert!(image.width().max(image.height()) <= 64 && image.width().min(image.height()) > 0);
        assert!(raw_thumbnail_png(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml").as_path(), 64).is_err());
    }

    #[test]
    fn a_custom_cmyk_profile_travels_inside_the_project() {
        use crate::color_mode::ColorMode as Mode;
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("profil.graf");
        let mut icc = CmykProfile::Fogra52.icc_bytes().expect("profil").to_vec();
        icc[44] ^= 2; // un profil « inconnu » (drapeaux de l'en-tête), valide
        let profile = crate::cmyk::register_icc(&icc).expect("profil personnalisé");
        let mut doc = Document::with_background("Profil", 16, 16, 72.0, Background::White).expect("document");
        let mode = Mode::cmyk(crate::cmyk::CmykSpace { profile, intent: crate::cmyk::RenderIntent::Perceptual, bpc: false });
        doc.convert_color_mode(mode, None).expect("conversion");
        save_project(&doc, &path).expect("enregistrement");
        let mut zip = ZipArchive::new(File::open(&path).expect("fichier")).expect("archive");
        let entry = format!("profils/{}.icc", profile.id());
        assert_eq!(read_entry(&mut zip, &entry, &path).expect("profil gardé"), icc);
        assert_eq!(zip.file_names().next(), Some("manifest.json"), "le manifeste reste en tête");
        assert_eq!(load_project(&path).expect("chargement").color_mode(), mode);
    }

    #[test]
    fn the_color_mode_round_trips_through_the_project_and_rgb_files_do_not_change() {
        use crate::color_mode::ColorMode as Mode;
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("mode.graf");
        let mut doc = Document::with_background("Mode", 32, 32, 72.0, Background::White).expect("document");
        save_project(&doc, &path).expect("enregistrement");
        let rgb = std::fs::read(&path).expect("fichier");
        assert_eq!(load_project(&path).expect("chargement").color_mode(), Mode::Rgb);
        for mode in [Mode::Grayscale, Mode::cmyk(CmykProfile::Fogra52.into())] {
            doc.convert_color_mode(mode, None).expect("conversion");
            save_project(&doc, &path).expect("enregistrement");
            assert_eq!(load_project(&path).expect("chargement").color_mode(), mode);
        }
        assert_eq!(written_version(&path), FORMAT_VERSION, "un mode autre que RVB demande une version qui connaît les modes");
        // Un document RVB s'écrit sans le champ : fichier identique à celui d'avant les modes.
        let structure = |bytes: &[u8]| {
            let mut zip = ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("archive");
            let text = String::from_utf8(read_entry(&mut zip, "document.json", Path::new("x")).expect("structure")).expect("texte");
            text
        };
        assert!(!structure(&rgb).contains("color_mode"));
        let written: serde_json::Value = serde_json::from_str(&structure(&std::fs::read(&path).expect("fichier"))).expect("json");
        assert_eq!(written["color_mode"], serde_json::json!({ "kind": "cmykWorkflow", "profile": "fogra52", "intent": "relative", "bpc": true }));
        // Retour en RVB : la version d'avant les modes ; de nouveau en gris : la 9 (une ancienne version refuse le
        // fichier au lieu de le rouvrir en couleur).
        doc.convert_color_mode(Mode::Rgb, None).expect("retour en RVB");
        save_project(&doc, &path).expect("enregistrement");
        assert_eq!(written_version(&path), FORMAT_VERSION_WITHOUT_SMART, "en RVB, le fichier reste lisible par les versions précédentes");
        doc.convert_color_mode(Mode::Grayscale, None).expect("gris");
        save_project(&doc, &path).expect("enregistrement");
        assert_eq!(written_version(&path), FORMAT_VERSION);
    }

    /// Position de `needle` dans `haystack` (lecture de la structure d'un AVIF, que le moteur ne décode pas).
    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    #[test]
    fn exports_avif_with_its_container_dimensions_transparency_and_exif() {
        // Moitié gauche rouge, moitié droite transparente.
        let mut doc = Document::with_background("x", 200, 100, 72.0, Background::Transparent).expect("document");
        let layer = doc.walk_top_down().last().expect("calque").1.id;
        doc.set_selection(Some(crate::selection::Shape::Rect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 }.rasterize(200, 100)));
        doc.fill_layer(layer, [200, 40, 40]).expect("moitié rouge");
        doc.set_selection(None);

        let encoded = export_encoded(&doc, ExportOptions { scale: 0.5, ..ExportOptions::new(ExportFormat::Avif) }).expect("export");
        assert_eq!((encoded.width, encoded.height), (100, 50), "mise à l'échelle de l'export respectée");
        // L'aperçu est l'équivalent PNG (la webview ne lit pas toujours l'AVIF) : transparence conservée.
        let shown = image::load_from_memory(&encoded.display).expect("aperçu PNG").into_rgba8();
        assert_eq!(shown.dimensions(), (100, 50));
        let (left, right) = (shown.get_pixel(20, 25).0, shown.get_pixel(80, 25).0);
        assert!(left[0] > 180 && left[1] < 70 && left[3] == 255 && right[3] == 0, "{left:?} {right:?}");

        // Le fichier : ISOBMFF, marque majeure « avif », compatible mif1/miaf.
        let (w, h, rgba) = flattened(&doc, 0.5, &|_, _| {}).expect("aplat");
        let options = ExportOptions::new(ExportFormat::Avif);
        let file = encode_file(w, h, &rgba, &options, page_of(&doc)).expect("avif");
        assert_eq!(file.len() as u64, encoded.size);
        assert_eq!(&file[4..12], b"ftypavif", "boîte ftyp en tête, marque avif");
        assert!(find(&file[..32], b"mif1").is_some() && find(&file[..32], b"miaf").is_some());
        // Dimensions (boîte ispe : version/drapeaux, largeur, hauteur en gros-boutiste).
        let ispe = find(&file, b"ispe").expect("ispe") + 8;
        let dims = (u32::from_be_bytes(file[ispe..ispe + 4].try_into().unwrap()), u32::from_be_bytes(file[ispe + 4..ispe + 8].try_into().unwrap()));
        assert_eq!(dims, (100, 50));
        // Transparence : image auxiliaire alpha reliée à la couleur, NON prémultipliée (pas de référence « prem »).
        assert!(find(&file, b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha").is_some() && find(&file, b"auxl").is_some());
        assert!(find(&file, b"prem").is_none(), "alpha droit");
        // Une image opaque n'a pas de plan alpha.
        let opaque = vec![[90u8, 140, 200, 255]; 64 * 64].concat();
        let opaque_file = encode_avif(64, 64, &opaque, 85, 9, None).expect("avif opaque");
        assert!(find(&opaque_file, b"auxl").is_none());
        // La qualité fait varier le poids.
        let photo: Vec<u8> = (0..128 * 128u32).flat_map(|i| [(i % 128 * 2) as u8, (i / 128 * 2) as u8, ((i.wrapping_mul(2_654_435_761)) >> 24) as u8, 255]).collect();
        let (low, high) = (encode_avif(128, 128, &photo, 20, 9, None).expect("avif").len(), encode_avif(128, 128, &photo, 95, 9, None).expect("avif").len());
        assert!(low < high, "qualité 20 : {low} o, qualité 95 : {high} o");

        // EXIF : élément « Exif » relié à l'image, avec l'auteur ; le reste du fichier (flux AV1) inchangé.
        let meta = ImageMetadata { author: "Jeanne Dupont".into(), copyright: "© 2026 Jeanne Dupont".into(), keywords: vec!["mer".into()], ..Default::default() };
        let tagged = encode_file(w, h, &rgba, &ExportOptions { metadata: Some(meta), ..options.clone() }, page_of(&doc)).expect("avif + exif");
        assert!(find(&tagged, b"Exif").is_some() && find(&tagged, b"cdsc").is_some() && find(&tagged, b"Jeanne Dupont").is_some());
        assert!(tagged.ends_with(&file[file.len() - 64..]), "mêmes flux AV1 en fin de fichier");
        // Mots-clés seuls (XMP uniquement) : rien à inscrire en EXIF, fichier inchangé.
        let only_keywords = ImageMetadata { keywords: vec!["mer".into()], ..Default::default() };
        assert_eq!(encode_file(w, h, &rgba, &ExportOptions { metadata: Some(only_keywords), ..options.clone() }, page_of(&doc)).expect("avif"), file);

        // L'export sur disque annonce l'encodage AVIF et écrit le même fichier.
        let path = std::env::temp_dir().join(format!("lumitype-export-{}.avif", std::process::id()));
        let labels = std::sync::Mutex::new(Vec::new());
        export_image_with_progress(&doc, &path, ExportOptions { scale: 0.5, ..options }, &|_, label| labels.lock().expect("verrou").push(label.to_owned())).expect("écriture");
        assert!(labels.into_inner().expect("verrou").iter().any(|l| l == "Encodage AVIF"));
        assert_eq!(std::fs::read(&path).expect("fichier"), file);
        let _ = std::fs::remove_file(path);
    }

    /// Justesse de l'estimation AVIF sur de vraies images (long : encodages complets aux trois vitesses) :
    /// `LUMITYPE_AVIF_IMAGES=photo.png:capture.png cargo test -p engine --release --lib avif_estimate_calibration -- --ignored --nocapture`.
    /// Affiche, pour chaque vitesse, le poids réel, l'estimation, leurs durées et l'écart.
    #[test]
    #[ignore]
    fn avif_estimate_calibration() {
        let paths = std::env::var("LUMITYPE_AVIF_IMAGES").expect("LUMITYPE_AVIF_IMAGES : chemins séparés par « : »");
        for path in paths.split(':') {
            let doc = import_image(Path::new(path)).expect("image");
            let flat = flatten_for_export(&doc, 1.0).expect("aplat");
            for speed in [AvifSpeed::Fast, AvifSpeed::Balanced, AvifSpeed::Max] {
                let options = ExportOptions { avif_speed: speed, ..ExportOptions::new(ExportFormat::Avif) };
                let started = std::time::Instant::now();
                let (estimate, _) = flat.estimate_size(&options).expect("estimation");
                let estimated_in = started.elapsed().as_secs_f64();
                let started = std::time::Instant::now();
                let real = flat.exact_size(&options).expect("poids");
                println!(
                    "{path} {}x{} {speed:?} : réel {} Ko en {:.1} s ; estimé {} Ko en {:.2} s ; écart {:+.0} %",
                    flat.width,
                    flat.height,
                    real / 1024,
                    started.elapsed().as_secs_f64(),
                    estimate / 1024,
                    estimated_in,
                    (estimate as f64 / real as f64 - 1.0) * 100.0
                );
            }
        }
    }

    #[test]
    fn the_avif_container_rebuilt_around_ravif_streams_is_identical() {
        // Garde-fou de `avif_container` : il suppose les flux AV1 en fin de fichier (alpha puis couleur) et les
        // réglages de ravif ; si une version de ravif changeait cela, ce test le dirait.
        let transparent: Vec<u8> = (0..96 * 80u32).flat_map(|i| [(i % 96) as u8 * 2, (i / 96) as u8 * 3, 90, if i % 96 < 40 { 0 } else { 200 }]).collect();
        let opaque: Vec<u8> = transparent.chunks_exact(4).flat_map(|px| [px[0], px[1], px[2], 255]).collect();
        for (rgba, alpha) in [(&transparent, true), (&opaque, false)] {
            let encoded = ravif_encode(96, 80, rgba, 85, 9).expect("ravif");
            assert_eq!(encoded.alpha_byte_size > 0, alpha);
            let rebuilt = avif_container(&encoded.avif_file, encoded.color_byte_size, encoded.alpha_byte_size, 96, 80, None).expect("conteneur");
            assert!(rebuilt == encoded.avif_file, "alpha {alpha} : conteneur refait différent de celui de ravif");
        }
    }

    #[test]
    fn export_options_without_the_avif_speed_still_read() {
        // Réglages d'avant l'AVIF (interface ou préférences enregistrées) : vitesse « rapide » par défaut.
        let old: ExportOptions = serde_json::from_str(r#"{"format":"webp","quality":80,"lossless":false,"scale":1,"color":"rgb","embedProfile":false,"pdfQuality":"high"}"#).expect("anciens réglages");
        assert_eq!((old.format, old.avif_speed), (ExportFormat::Webp, AvifSpeed::Fast));
        let avif: ExportOptions = serde_json::from_str(r#"{"format":"avif","quality":85,"scale":0.5,"avifSpeed":"max"}"#).expect("réglages AVIF");
        assert_eq!((avif.format, avif.avif_speed, avif.avif_speed.preset()), (ExportFormat::Avif, AvifSpeed::Max, 4));
        let balanced: ExportOptions = serde_json::from_str(r#"{"format":"avif","quality":85,"scale":1,"avifSpeed":"balanced"}"#).expect("réglages AVIF");
        assert_eq!(balanced.avif_speed.preset(), 6);
        assert_eq!(ExportOptions::new(ExportFormat::Avif).avif_speed, AvifSpeed::Fast);
        assert!(!ExportFormat::Avif.supports_cmyk(), "AVIF : ni CMJN ni profil ICC");
        // Curseur commun : 85 (défaut) donne la qualité 70 de ravif, les bornes restent dans 1..=100.
        assert_eq!((avif_quality(1), avif_quality(85), avif_quality(100)), (1.0, 70.0, 100.0));
    }

    #[test]
    fn exports_pdf_and_cmyk_files_with_or_without_the_icc_profile() {
        // Moitié gauche rouge, moitié droite transparente (blanche à l'impression).
        let mut doc = Document::with_background("x", 200, 100, 300.0, Background::Transparent).expect("document");
        let layer = doc.walk_top_down().last().expect("calque").1.id;
        doc.set_selection(Some(crate::selection::Shape::Rect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 }.rasterize(200, 100)));
        doc.fill_layer(layer, [220, 30, 30]).expect("moitié rouge");
        doc.set_selection(None);
        let has = |bytes: &[u8], needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        let cmyk = |format, embed_profile| ExportOptions { color: ColorMode::Cmyk, cmyk_space: CmykProfile::Fogra39.into(), embed_profile, ..ExportOptions::new(format) };

        // PDF : page à la taille du document à 300 ppp (200 px = 48 pt), aperçu PNG lisible (épreuve en CMJN).
        for options in [cmyk(ExportFormat::Pdf, true), cmyk(ExportFormat::Pdf, false), ExportOptions { pdf_quality: PdfQuality::Lossless, embed_profile: true, ..ExportOptions::new(ExportFormat::Pdf) }] {
            let encoded = export_encoded(&doc, options.clone()).expect("pdf");
            let shown = image::load_from_memory(&encoded.display).expect("aperçu").into_rgba8();
            let (left, right) = (shown.get_pixel(20, 50).0, shown.get_pixel(180, 50).0);
            assert!(left[0] > 150 && left[1] < 90, "{options:?} : {left:?}");
            if options.cmyk().is_some() {
                assert!(right[..3].iter().all(|&v| v > 245) && right[3] == 255, "CMJN : transparence sur blanc");
            } else {
                assert_eq!(right[3], 0, "RVB : transparence gardée");
            }
            let flat = flatten_for_export(&doc, 1.0).expect("composition");
            assert_eq!(flat.page, [48.0, 24.0]);
            let pdf = encode_file(flat.width, flat.height, &flat.rgba, &options, flat.page).expect("pdf");
            assert_eq!(pdf.len() as u64, encoded.size);
            assert!(pdf.starts_with(b"%PDF-") && has(&pdf, b"/MediaBox [0 0 48 24]"));
            assert_eq!(has(&pdf, b"/ICCBased"), options.embed_profile);
        }

        // JPEG et TIFF CMJN : relus (la bibliothèque `image` les ramène en RVB), le rouge reste rouge, le blanc blanc.
        for format in [ExportFormat::Jpeg, ExportFormat::Tiff] {
            for embed in [false, true] {
                let encoded = export_encoded(&doc, cmyk(format, embed)).expect("cmjn");
                let flat = flatten_for_export(&doc, 1.0).expect("composition");
                let file = encode_file(flat.width, flat.height, &flat.rgba, &cmyk(format, embed), flat.page).expect("fichier");
                assert_eq!(file.len() as u64, encoded.size);
                let icc_marker: &[u8] = if format == ExportFormat::Jpeg { b"ICC_PROFILE\0" } else { b"FOGRA39L" };
                assert_eq!(has(&file, icc_marker), embed, "{format:?} : profil ICC incorporé seulement sur demande");
                let decoded = image::load_from_memory(&file).expect("fichier CMJN lisible").into_rgb8();
                let (left, right) = (decoded.get_pixel(20, 50).0, decoded.get_pixel(180, 50).0);
                assert!(left[0] > 150 && left[1] < 110 && left[2] < 110, "{format:?} : rouge {left:?}");
                assert!(right.iter().all(|&v| v > 240), "{format:?} : blanc {right:?}");
            }
        }
        // TIFF CMJN : PhotometricInterpretation = 5 déclaré.
        let tiff = export_encoded(&doc, cmyk(ExportFormat::Tiff, true)).expect("tiff").size;
        assert!(tiff > 200 * 100 * 4);
        // RVB avec profil : sRGB incorporé au JPEG.
        let flat = flatten_for_export(&doc, 1.0).expect("composition");
        let jpeg = encode_file(flat.width, flat.height, &flat.rgba, &ExportOptions { embed_profile: true, ..ExportOptions::new(ExportFormat::Jpeg) }, flat.page).expect("jpeg");
        assert!(has(&jpeg, b"ICC_PROFILE\0"));
        // Le PNG et le WebP ignorent le CMJN et le profil.
        let png = encode_file(flat.width, flat.height, &flat.rgba, &cmyk(ExportFormat::Png, true), flat.page).expect("png");
        assert_eq!(png, encode_file(flat.width, flat.height, &flat.rgba, &ExportOptions::new(ExportFormat::Png), flat.page).expect("png"));
    }

    #[test]
    fn old_export_settings_still_deserialize() {
        let options: ExportOptions = serde_json::from_str(r#"{"format":"jpeg","quality":80,"scale":1}"#).expect("anciens réglages");
        assert_eq!((options.color, options.embed_profile, options.pdf_quality, options.cmyk_space), (ColorMode::Rgb, false, PdfQuality::High, CmykSpace::default()));
        let options: ExportOptions = serde_json::from_str(r#"{"format":"pdf","quality":80,"scale":1,"color":"cmyk","cmykSpace":{"profile":"swop2006c3"},"embedProfile":true,"pdfQuality":"lossless"}"#).expect("réglages PDF");
        assert_eq!(options.cmyk(), Some(CmykProfile::Swop2006c3.into()));
        assert!(options.embed_profile && options.pdf_quality == PdfQuality::Lossless);
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
        let lossy = |quality| export_encoded(&doc, ExportOptions { quality, ..ExportOptions::new(ExportFormat::Webp) }).expect("webp");
        let lossless = export_encoded(&doc, ExportOptions { quality: 100, lossless: true, ..ExportOptions::new(ExportFormat::Webp) }).expect("webp");
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
        let tagged = export_encoded(&doc, ExportOptions { quality: 80, metadata: Some(meta), ..ExportOptions::new(ExportFormat::Webp) }).expect("webp");
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

    /// Plan de travail sélectionné : l'image importée entre DANS le plan (pas au-dessus de tous les plans), ajustée et
    /// centrée sur son cadre, pas sur le document entier.
    #[test]
    fn import_into_a_selected_artboard_stays_inside_it_and_fits_its_frame() {
        use crate::document::Artboard;
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("aplat.png");
        let mut source = Document::with_background("x", 100, 60, 72.0, Background::Transparent).expect("document");
        let id = source.walk_top_down()[0].1.id;
        source.pixels_mut(id).expect("pixels").fill(100, 60, [0, 128, 0, 255]);
        export_png(&source, &path).expect("export");

        let mut doc = Document::new("plans", 10, 10, 72.0).expect("document");
        let board = |x, y, w, h| Artboard { x, y, width: w, height: h, background: Some([255, 255, 255]), clip: true, preset: None, background_layer: None };
        let left = doc.add_artboard("Gauche", board(0, 0, 400, 300)).expect("plan");
        let right = doc.add_artboard("Droite", board(500, 0, 200, 300)).expect("plan");
        doc.fit_to_artboards().expect("document ajusté aux plans");
        assert_eq!((doc.width(), doc.height()), (700, 300));

        // « Au-dessus » du plan de droite sélectionné : dans ce plan, en haut de sa pile, centré sur ses 200 x 300.
        let (parent, index) = doc.placement_slot(Some(right)).expect("emplacement");
        assert_eq!(parent, Some(right));
        let layer = import_as_layer(&mut doc, &path, parent, index).expect("calque importé");
        assert_eq!(doc.layer(layer).expect("calque").parent, Some(right), "dans le plan, pas au-dessus des plans");
        assert_eq!(doc.children_of(Some(right)).expect("enfants").last(), Some(&layer), "en haut de la pile du plan");
        let bounds = doc.content_bounds(&[layer]).expect("bornes").expect("contenu");
        assert_eq!(bounds, (500, 90, 700, 210), "ajusté à la largeur du plan (200), centré en hauteur");

        // Au-dessus d'un calque DU plan de gauche : même plan, au-dessus de ce calque.
        let (parent, index) = doc.placement_slot(Some(left)).expect("emplacement");
        let first = import_as_layer(&mut doc, &path, parent, index).expect("calque");
        let (parent, index) = doc.placement_slot(Some(first)).expect("emplacement");
        let second = import_as_layer(&mut doc, &path, parent, index).expect("calque");
        assert_eq!(doc.layer(second).expect("calque").parent, Some(left));
        let children = doc.children_of(Some(left)).expect("enfants");
        assert_eq!(&children[children.len() - 2..], &[first, second]);
        assert_eq!(doc.content_bounds(&[second]).expect("bornes").expect("contenu"), (0, 30, 400, 270));

        // Sans plan visé : comportement d'avant, ajusté au document entier.
        let (parent, index) = doc.placement_slot(None).expect("emplacement");
        let root = import_as_layer(&mut doc, &path, parent, index).expect("calque");
        assert_eq!(doc.layer(root).expect("calque").parent, None);
        assert_eq!(doc.content_bounds(&[root]).expect("bornes").expect("contenu"), (100, 0, 600, 300));
    }

    // ------------------------------------------------------------ RAW « à redévelopper »

    fn raw_fixture() -> (Arc<Vec<u8>>, PathBuf) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mire.dng");
        (Arc::new(std::fs::read(&path).expect("mire")), path)
    }

    /// Objet dynamique RAW développé avec `params`, comme le fait la fenêtre de développement.
    fn raw_content(params: crate::raw::RawParams) -> SmartContent {
        let (bytes, path) = raw_fixture();
        let developed = crate::raw::RawSession::open(Arc::clone(&bytes), &path).expect("RAW").develop_full(&params);
        let document = document_from_premultiplied(developed.width, developed.height, &developed.to_rgba(), &path).expect("document");
        crate::smart::content_from_raw("mire.dng", bytes, params, document)
    }

    fn warm() -> crate::raw::RawParams {
        crate::raw::RawParams { white_balance: crate::raw::WhiteBalance::Custom, temperature: 7500.0, tint: 12.0, exposure: 0.5, contrast: 20.0, sharpen: 30.0, ..Default::default() }
    }

    fn smart_of(doc: &Document) -> Arc<SmartContent> {
        doc.walk_top_down().into_iter().find_map(|(_, l)| l.smart().cloned()).expect("objet dynamique")
    }

    fn flat_bytes(content: &SmartContent) -> Vec<u8> {
        content.flattened.to_premultiplied(content.width(), content.height())
    }

    #[test]
    fn a_redevelopable_raw_round_trips_through_the_project_with_its_settings() {
        let content = raw_content(warm());
        let expected = flat_bytes(&content);
        let mut doc = Document::new("photo", 800, 800, 72.0).expect("document");
        doc.place_smart_object(content, None, 0).expect("placement");
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("photo.graf");
        save_project(&doc, &path).expect("enregistrement");

        // Le développement est enregistré avec le RAW et ses réglages.
        let mut zip = ZipArchive::new(File::open(&path).expect("archive")).expect("zip");
        let json = String::from_utf8(read_entry(&mut zip, "document.json", &path).expect("structure")).expect("utf-8");
        assert!(json.contains("\"develop\"") && json.contains("\"temperature\": 7500.0"), "{json}");
        assert!((0..zip.len()).any(|i| zip.by_index(i).is_ok_and(|f| f.name().contains("/flat/"))), "développement enregistré");
        drop(zip);

        let loaded = load_project(&path).expect("lecture");
        let content = smart_of(&loaded);
        assert_eq!(content.raw_develop(), Some(&warm()), "réglages relus");
        assert_eq!(content.original().map(|o| o.bytes.len()), Some(raw_fixture().0.len()), "RAW gardé à l'octet près");
        assert_eq!(flat_bytes(&content), expected, "rendu relu tel quel");
        // Le document du contenu est l'image développée (un calque), prête pour « Pixelliser » ou l'export.
        assert_eq!(content.document.walk_top_down().len(), 1);
    }

    fn written_version(path: &Path) -> u32 {
        let mut zip = ZipArchive::new(File::open(path).expect("archive")).expect("zip");
        let manifest: Manifest = serde_json::from_slice(&read_entry(&mut zip, "manifest.json", path).expect("manifeste")).expect("json");
        manifest.format_version
    }

    #[test]
    fn the_written_version_is_the_oldest_able_to_describe_the_document() {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        // Sans objet dynamique : 6 (inchangé).
        let path = dir.path().join("simple.graf");
        save_project(&Document::new("simple", 64, 64, 72.0).expect("document"), &path).expect("enregistrement");
        assert_eq!(written_version(&path), 6);

        // Objet dynamique sans RAW à redévelopper : 7, lisible par Lumitype 1.1.0.
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 3, image::Rgba([200, 30, 30, 255])).write_to(&mut Cursor::new(&mut png), ImageFormat::Png).expect("png");
        let mut doc = Document::new("placé", 64, 64, 72.0).expect("document");
        doc.place_smart_object(crate::smart::content_from_bytes(png, Path::new("rouge.png")).expect("contenu"), None, 0).expect("placement");
        let path = dir.path().join("placé.graf");
        save_project(&doc, &path).expect("enregistrement");
        assert_eq!(written_version(&path), 7);

        // RAW développé : 8, et le fichier se relit avec ses réglages.
        let content = raw_content(warm());
        let expected = flat_bytes(&content);
        doc.place_smart_object(content, None, 0).expect("placement");
        let path = dir.path().join("raw.graf");
        save_project(&doc, &path).expect("enregistrement");
        assert_eq!(written_version(&path), 8);
        let loaded = load_project(&path).expect("relecture de la version 8");
        let raw = loaded.walk_top_down().into_iter().filter_map(|(_, l)| l.smart().cloned()).find(|c| c.raw_develop().is_some()).expect("RAW relu");
        assert_eq!((raw.raw_develop(), flat_bytes(&raw)), (Some(&warm()), expected));
    }

    #[test]
    fn a_version_8_file_is_refused_cleanly_by_a_version_7_reader() {
        // Lumitype 1.1.0 lisait `2..=7` et refusait le reste avec `UnsupportedVersion { found, supported: 7 }` :
        // son message pour un fichier de la version 8 est celui-ci (rien n'est lu à moitié).
        let message = EngineError::UnsupportedVersion { found: 8, supported: 7 }.to_string();
        assert_eq!(message, "projet enregistré avec un format plus récent (version 8, maximum géré 7)");
        // Et la version actuelle refuse de même une version supérieure à la sienne.
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("futur.graf");
        write_archive(&path, &[("manifest.json", manifest(FORMAT_VERSION + 1)), ("document.json", b"{}".to_vec())]);
        assert!(matches!(load_project(&path), Err(EngineError::UnsupportedVersion { found, supported }) if found == FORMAT_VERSION + 1 && supported == FORMAT_VERSION));
    }

    #[test]
    fn paragraph_text_round_trips_and_raises_the_version_only_when_present() {
        use crate::text::{TextAlign, TextData, TextFrame};
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let mut doc = Document::new("texte", 200, 200, 72.0).expect("document");
        let point = doc.add_text_layer(TextData { content: "Point".into(), size: 20.0, ..TextData::default() }, (10, 10), None, 0).expect("point");
        let path = dir.path().join("point.graf");
        save_project(&doc, &path).expect("enregistrement");
        assert_eq!(written_version(&path), 6, "un texte de point ne change pas la version");

        let frame = Some(TextFrame { width: 120.0, height: 60.0 });
        let paragraph = TextData { content: "Un texte de paragraphe assez long pour revenir à la ligne".into(), size: 18.0, align: TextAlign::Justify, frame, ..TextData::default() };
        let id = doc.add_text_layer(paragraph.clone(), (20, 50), None, 1).expect("paragraphe");
        let path = dir.path().join("paragraphe.graf");
        save_project(&doc, &path).expect("enregistrement");
        assert_eq!(written_version(&path), 9, "texte de paragraphe : version 9");
        let loaded = load_project(&path).expect("relecture");
        let layer = loaded.layer(id).expect("calque relu");
        assert_eq!(layer.text(), Some(&paragraph), "cadre et alignement relus");
        assert_eq!(loaded.layer(point).expect("point").text().and_then(|t| t.frame), None);
        assert_eq!(layer.pixels().map(|p| p.pixel_bounds(|px| px[3] > 0)), doc.layer(id).expect("calque").pixels().map(|p| p.pixel_bounds(|px| px[3] > 0)), "même rendu");

        // Copie de secours : le cadre y survit aussi.
        let recovery = dir.path().join("secours.zip");
        save_recovery(&History { states: vec![("Texte".into(), doc)], cursor: 0 }, None, &recovery, &|_| {}).expect("copie de secours");
        let (history, _) = load_recovery(&recovery).expect("relecture");
        assert_eq!(history.states[0].1.layer(id).expect("calque").text(), Some(&paragraph));
    }

    /// Plans de travail (fonds = remplissages masqués) et calque de remplissage masqué : après une copie de secours,
    /// les miniatures sont les mêmes qu'avant (elles étaient vides ou noires à la récupération).
    /// Diagnostic : `LUMI_RECOVERY=copie.graf LUMI_PROJECT=projet.graf cargo test --lib -- --ignored recovery_thumbnail_report --nocapture`.
    #[test]
    #[ignore]
    fn recovery_thumbnail_report() {
        use crate::render::TileRenderer;
        let report = |doc: &Document, what: &str| {
            println!("== {what} : {} × {}", doc.width(), doc.height());
            for (depth, layer) in doc.walk_top_down() {
                let tiles = layer.pixels().map_or(0, |p| p.tile_count());
                let seen = |mask| TileRenderer::new().board_thumbnail(doc, layer.id, mask).map(|(w, h, px)| (w, h, px.chunks_exact(4).filter(|p| if mask { p[0] > 0 } else { p[3] > 0 }).count()));
                println!("{}{} offset {:?} tuiles {tiles} masque {:?} → pixels {:?} masque {:?}", "  ".repeat(depth), layer.props.name, layer.offset, layer.mask.as_ref().map(|m| m.pixels.tile_count()), seen(false), seen(true));
            }
        };
        if let Ok(path) = std::env::var("LUMI_RECOVERY") {
            let (history, _) = load_recovery(Path::new(&path)).expect("copie de secours");
            println!("{} états, curseur {}", history.states.len(), history.cursor);
            report(&history.states[history.cursor].1, "copie de secours");
        }
        if let Ok(path) = std::env::var("LUMI_PROJECT") {
            report(&load_project(Path::new(&path)).expect("projet"), "projet");
        }
    }

    #[test]
    fn thumbnails_survive_the_recovery_copy() {
        use crate::document::Artboard;
        use crate::fill::FillContent;
        use crate::render::TileRenderer;
        let mut doc = Document::new("plans", 100, 100, 72.0).expect("document");
        let board = |x, y, w, h| Artboard { x, y, width: w, height: h, background: Some([255, 255, 255]), clip: true, preset: None, background_layer: None };
        let a = doc.add_artboard("A", board(0, 0, 600, 300)).expect("plan");
        let b = doc.add_artboard("B", board(700, 0, 300, 200)).expect("plan");
        doc.fit_to_artboards().expect("ajuster");
        let fill = doc.add_fill_layer(FillContent::SolidColor { color: [0, 0, 0] }, "Noir", Some(a), 1).expect("remplissage");
        for y in 0..64 {
            for x in 0..64 {
                doc.mask_mut(fill).expect("masque").get_mut(crate::tiles::TileCoord::new(0, 0)).set_pixel(x, y, [0]);
            }
        }
        let bg = doc.layer(b).expect("plan").artboard.as_ref().and_then(|x| x.background_layer).expect("fond");
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("secours.zip");
        save_recovery(&History { states: vec![("Plans".into(), doc.clone())], cursor: 0 }, None, &path, &|_| {}).expect("copie de secours");
        let (history, _) = load_recovery(&path).expect("relecture");
        let back = &history.states[0].1;
        for (id, what) in [(bg, "fond du plan B"), (fill, "remplissage noir masqué")] {
            for mask in [false, true] {
                let before = TileRenderer::new().board_thumbnail(&doc, id, mask).expect("miniature");
                let after = TileRenderer::new().board_thumbnail(back, id, mask).expect("miniature relue");
                assert_eq!(before, after, "{what} ({}) : même miniature après la copie de secours", if mask { "masque" } else { "pixels" });
            }
        }
        let (_, _, px) = TileRenderer::new().board_thumbnail(back, bg, false).expect("fond");
        assert!(px.chunks_exact(4).all(|p| p == [255, 255, 255, 255]), "fond blanc");
    }

    /// Copie de secours dont une tuile n'a pas la taille d'une tuile (fichier abîmé ou forgé) : erreur propre, pas
    /// de panique. Et une entrée d'archive annonçant une taille démesurée se lit sans réserver cette mémoire.
    #[test]
    fn a_corrupt_recovery_copy_is_refused_without_panicking() {
        let mut doc = Document::with_background("x", 300, 300, 72.0, crate::document::Background::White).expect("document");
        let mut pixels = TileStore::new();
        pixels.get_mut(TileCoord::new(0, 0)).set_pixel(3, 3, [255, 0, 0, 255]);
        doc.add_pixel_layer("calque", pixels, None, 1).expect("calque");
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("secours.zip");
        save_recovery(&History { states: vec![("Ouvrir".into(), doc)], cursor: 0 }, None, &path, &|_| {}).expect("copie de secours");
        // Réécriture de l'archive avec une tuile de pixels de 8×8.
        let mut source = ZipArchive::new(BufReader::new(File::open(&path).expect("archive"))).expect("zip");
        let names: Vec<String> = source.file_names().map(str::to_owned).collect();
        let forged = dir.path().join("forge.zip");
        let mut out = ZipWriter::new(BufWriter::new(File::create(&forged).expect("création")));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for name in names {
            let bytes = if name == "tuiles/p0.png" {
                let mut png = Vec::new();
                image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255])).write_to(&mut Cursor::new(&mut png), ImageFormat::Png).expect("png");
                png
            } else {
                read_entry(&mut source, &name, &path).expect("entrée")
            };
            out.start_file(name, options).expect("entrée");
            out.write_all(&bytes).expect("écriture");
        }
        out.finish().expect("fin");
        assert!(load_recovery(&forged).is_err(), "tuile de taille inattendue : refusée");
        // Taille annoncée fausse (en-tête local trafiqué) : la lecture réserve au plus 64 Mo et lit ce qu'il y a.
        let mut bytes = std::fs::read(&path).expect("archive");
        let pos = bytes.windows(4).position(|w| w == b"PK\x01\x02").expect("répertoire central");
        bytes[pos + 24..pos + 28].copy_from_slice(&u32::MAX.to_le_bytes());
        let huge = dir.path().join("enorme.zip");
        std::fs::write(&huge, &bytes).expect("écriture");
        let mut zip = ZipArchive::new(BufReader::new(File::open(&huge).expect("archive"))).expect("zip");
        let name = zip.file_names().next().expect("entrée").to_owned();
        let _ = read_entry(&mut zip, &name, &huge);
    }

    #[test]
    fn a_redevelopable_raw_survives_the_recovery_copy() {
        let content = raw_content(warm());
        let expected = flat_bytes(&content);
        let mut doc = Document::new("photo", 800, 800, 72.0).expect("document");
        doc.place_smart_object(content, None, 0).expect("placement");
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("secours.zip");
        save_recovery(&History { states: vec![("Ouvrir".into(), doc)], cursor: 0 }, None, &path, &|_| {}).expect("copie de secours");
        let (history, _) = load_recovery(&path).expect("relecture");
        let content = smart_of(&history.states[0].1);
        assert_eq!(content.raw_develop(), Some(&warm()));
        assert_eq!(flat_bytes(&content), expected);
    }

    #[test]
    fn placed_images_and_older_projects_have_no_development_field() {
        // Une image placée ordinaire s'écrit comme avant : ni `develop` ni `tiles` dans son entrée.
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 3, image::Rgba([200, 30, 30, 255])).write_to(&mut Cursor::new(&mut png), ImageFormat::Png).expect("png");
        let content = crate::smart::content_from_bytes(png, Path::new("rouge.png")).expect("contenu");
        let mut doc = Document::new("placé", 64, 64, 72.0).expect("document");
        doc.place_smart_object(content, None, 0).expect("placement");
        let (structure, _) = describe(&doc);
        let json = serde_json::to_string(&structure.contents).expect("json");
        assert!(json.contains("\"kind\":\"original\"") && !json.contains("develop") && !json.contains("tiles"), "{json}");
        // Entrée écrite avant l'existence des RAW à redévelopper : relue sans réglages.
        let old: ContentV7 = serde_json::from_str(r#"{"key":3,"name":"rouge","width":4,"height":3,"source":{"kind":"original","file_name":"rouge.png"}}"#).expect("ancien format");
        assert!(matches!(old.source, SourceV7::Original { develop: None, ref tiles, .. } if tiles.is_empty()));
        // Et le projet se relit à l'identique.
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("placé.graf");
        save_project(&doc, &path).expect("enregistrement");
        let loaded = smart_of(&load_project(&path).expect("lecture"));
        assert!(loaded.raw_develop().is_none());
        assert_eq!(flat_bytes(&loaded), flat_bytes(&smart_of(&doc)));
    }

    #[test]
    fn the_bundled_template_still_opens_and_round_trips() {
        // Projet existant (format 6, écrit par une version précédente) : ouverture, enregistrement, relecture.
        let template = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/templates/Design_1_ref.graf");
        let doc = load_project(&template).expect("modèle");
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("copie.graf");
        save_project(&doc, &path).expect("enregistrement");
        let again = load_project(&path).expect("relecture");
        assert_eq!((again.width(), again.height(), again.walk_top_down().len()), (doc.width(), doc.height(), doc.walk_top_down().len()));
        let renderer = TileRenderer::new();
        assert_eq!(renderer.tile(&again, 0, TileCoord::new(0, 0)).map(|t| t.bytes().to_vec()), renderer.tile(&doc, 0, TileCoord::new(0, 0)).map(|t| t.bytes().to_vec()));
    }

    #[test]
    fn redeveloping_replaces_every_instance_in_one_undoable_step() {
        let mut doc = Document::new("photo", 800, 800, 72.0).expect("document");
        let first = doc.place_smart_object(raw_content(crate::raw::RawParams::default()), None, 0).expect("placement");
        let mut session = crate::Session::new(doc, "Ouvrir");
        session.apply("Dupliquer", None, |doc| doc.duplicate_layer(first).map(|_| ())).expect("instance");
        let before = flat_bytes(&smart_of(session.document()));
        let render = |session: &crate::Session| session.document().walk_top_down().into_iter().filter_map(|(_, l)| l.smart().map(|c| (c.raw_develop().copied(), flat_bytes(c)))).collect::<Vec<_>>();
        assert_eq!(render(&session).len(), 2, "deux instances");

        // Ce que fait `raw_redevelop` : nouveau développement, même contenu (toutes les instances), une étape.
        let current = smart_of(session.document());
        let (bytes, path) = raw_fixture();
        let developed = crate::raw::RawSession::open(bytes, &path).expect("RAW").develop_full(&warm());
        let document = document_from_premultiplied(developed.width, developed.height, &developed.to_rgba(), &path).expect("document");
        let content = current.redeveloped(document, warm()).expect("redéveloppé");
        session.apply("Développer le RAW", None, |doc| doc.update_smart_content(Arc::new(content), crate::document::SmartFit::Centre).map(|_| ())).expect("remplacement");
        let after = render(&session);
        assert!(after.iter().all(|(params, pixels)| *params == Some(warm()) && *pixels != before), "les deux instances changent");

        session.undo();
        assert!(render(&session).iter().all(|(params, pixels)| *params == Some(crate::raw::RawParams::default()) && *pixels == before), "Ctrl+Z rend l'ancien développement");
    }
}
