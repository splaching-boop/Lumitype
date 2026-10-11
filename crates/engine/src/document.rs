//! Modèle de document : un arbre de calques.
//!
//! Chaque nœud est partagé par `Arc`, comme ses pixels : cloner un [`Document`]
//! ne copie que la table des nœuds. L'historique conserve donc des documents
//! entiers à très faible coût (voir `session`), et toute opération (pixels,
//! structure, propriétés) est annulable par le même mécanisme.
//!
//! Dans tout le module, les listes d'enfants vont **du bas vers le haut** de la pile.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::blend::BlendMode;
use crate::error::EngineError;
use crate::adjust::Adjustment;
use crate::effects::LayerEffect;
use crate::filters::{self, Filter};
use crate::paint::{Brush, Stroke, StrokePoint};
use crate::path::SubPath;
use crate::pixel::{mul_div_255, premultiply, unpremultiply};
use crate::selection::{combine, magic_wand, CombineMode, Shape};
use crate::shape::{self, ShapeData};
use crate::smart::{self, ContentId, SmartContent, SmartGeometry, SmartSource};
use crate::text::{self, TextData};
use crate::tiles::{MaskStore, Sampler, Tile, TileCoord, TileStore, MAX_DIMENSION, TILE_SIZE};
use crate::transform::{transform_store, Matrix, Quality};

pub type LayerId = u64;

/// Verrouillages du panneau Calques.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locks {
    /// Pixels transparents : la peinture ne modifie pas l'alpha.
    pub transparency: bool,
    /// Pixels : aucune peinture.
    pub pixels: bool,
    /// Position : aucun déplacement.
    pub position: bool,
    /// Tout.
    pub all: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayerProps {
    pub name: String,
    pub visible: bool,
    /// Opacité du calque, effets compris. `0..=1`.
    pub opacity: f32,
    /// Fond : opacité des pixels seuls. Identique à l'opacité tant que les effets
    /// de calque n'existent pas ; les deux valeurs se multiplient.
    pub fill: f32,
    pub blend: BlendMode,
    pub locks: Locks,
    /// Masque d'écrêtage : ce calque n'est visible qu'à travers l'alpha du
    /// premier calque non écrêté situé sous lui.
    pub clipped: bool,
}

impl LayerProps {
    fn named(name: impl Into<String>, blend: BlendMode) -> Self {
        Self {
            name: name.into(),
            visible: true,
            opacity: 1.0,
            fill: 1.0,
            blend,
            locks: Locks::default(),
            clipped: false,
        }
    }
}

/// Masque de fusion, en niveaux de gris.
#[derive(Clone, Debug)]
pub struct Mask {
    pub pixels: Arc<MaskStore>,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub enum LayerKind {
    Pixel { pixels: Arc<TileStore> },
    Group { children: Vec<LayerId>, expanded: bool },
    /// Texte éditable. `transform` place le texte (repère texte, origine à l'ancre) dans le
    /// repère du calque : c'est ce que Ctrl+T modifie, sans jamais pixelliser. `raster` n'est
    /// qu'un cache du rendu, recalculé à chaque modification du texte ou de la transformation.
    Text { text: TextData, transform: Matrix, raster: Arc<TileStore> },
    /// Forme vectorielle (rectangle, ellipse). `raster` n'est qu'un cache du dessin.
    Shape { shape: ShapeData, raster: Arc<TileStore> },
    /// Calque de réglage : corrige tout ce qui se trouve dessous, sans toucher aux pixels.
    Adjustment { adjustment: Adjustment },
    /// Objet dynamique (module `smart`) : un document entier, partagé par ses instances. `transform` place le
    /// contenu (repère contenu, origine au coin haut gauche) dans le repère du calque, comme pour un texte : c'est ce
    /// que Ctrl+T modifie, sans jamais pixelliser. `raster` n'est qu'un cache : l'aplat du contenu transformé, égal à
    /// l'aplat lui-même (partagé) quand la transformation est l'identité.
    Smart { content: Arc<SmartContent>, transform: Matrix, raster: Arc<TileStore> },
    // Forme vectorielle : à venir.
}

/// Couleur d'étiquette d'un calque ou d'un dossier dans le panneau (repère visuel, sans effet sur l'image).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayerLabel {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Violet,
    Gray,
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub id: LayerId,
    pub parent: Option<LayerId>,
    /// Couleur d'étiquette dans le panneau des calques ; `None` : aucune. Un dossier teinte ses enfants sans étiquette.
    pub label: Option<LayerLabel>,
    /// Position du calque : le pixel local `(0, 0)` se trouve en `offset` dans le
    /// document. Pixels et masque de fusion partagent ce repère local, ce qui rend un
    /// déplacement instantané (aucun pixel touché) et conserve ce qui sort du cadre.
    pub offset: (i32, i32),
    pub props: LayerProps,
    pub mask: Option<Mask>,
    /// Effets de calque (ombre portée, lueurs, contour…), non destructifs.
    pub effects: Vec<LayerEffect>,
    pub kind: LayerKind,
}

impl Layer {
    pub fn is_group(&self) -> bool {
        matches!(self.kind, LayerKind::Group { .. })
    }

    pub fn children(&self) -> &[LayerId] {
        match &self.kind {
            LayerKind::Group { children, .. } => children,
            LayerKind::Pixel { .. } | LayerKind::Text { .. } | LayerKind::Shape { .. } | LayerKind::Adjustment { .. } | LayerKind::Smart { .. } => &[],
        }
    }

    pub fn adjustment(&self) -> Option<&Adjustment> {
        match &self.kind {
            LayerKind::Adjustment { adjustment } => Some(adjustment),
            _ => None,
        }
    }

    /// Pixels affichés : ceux du calque, ou le rendu en cache d'un texte, d'une forme ou d'un objet dynamique.
    pub fn pixels(&self) -> Option<&Arc<TileStore>> {
        match &self.kind {
            LayerKind::Pixel { pixels } => Some(pixels),
            LayerKind::Text { raster, .. } | LayerKind::Shape { raster, .. } | LayerKind::Smart { raster, .. } => Some(raster),
            LayerKind::Group { .. } | LayerKind::Adjustment { .. } => None,
        }
    }

    pub fn shape(&self) -> Option<&ShapeData> {
        match &self.kind {
            LayerKind::Shape { shape, .. } => Some(shape),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&TextData> {
        match &self.kind {
            LayerKind::Text { text, .. } => Some(text),
            _ => None,
        }
    }

    /// Contenu d'un objet dynamique.
    pub fn smart(&self) -> Option<&Arc<SmartContent>> {
        match &self.kind {
            LayerKind::Smart { content, .. } => Some(content),
            _ => None,
        }
    }

    /// Transformation complète d'un objet dynamique : repère contenu -> document (position du calque comprise).
    pub fn smart_transform(&self) -> Option<Matrix> {
        match &self.kind {
            LayerKind::Smart { transform, .. } => Some(Matrix::translation(f64::from(self.offset.0), f64::from(self.offset.1)).then_after(transform)),
            _ => None,
        }
    }
}

/// Repère de mise en page. `position` est en pixels du document, le long de l'axe perpendiculaire.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub orientation: Orientation,
    pub position: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Horizontal,
    Vertical,
}

/// Découpage de l'image pour l'aperçu de déplacement : calques de pixels de chaque partie.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MovePlan {
    pub below: Vec<LayerId>,
    pub moving: Vec<LayerId>,
    pub above: Vec<LayerId>,
}

/// Arrêt d'un dégradé : position de 0 à 1, couleur RGBA à alpha droit.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct GradientStop {
    pub position: f32,
    pub color: [u8; 4],
}

/// Calques copiés (Ctrl+C dans le panneau Calques) : les calques de premier niveau et, pour les groupes,
/// tout leur contenu. Les pixels restent partagés avec l'original jusqu'à la première retouche.
#[derive(Clone, Debug)]
pub struct LayerClip {
    roots: Vec<LayerId>,
    nodes: HashMap<LayerId, Arc<Layer>>,
}

/// Pixels copiés (Ctrl+C) : leur contenu, en repère local, et la position qu'ils occupaient.
#[derive(Clone, Debug)]
pub struct Clip {
    pub pixels: Arc<TileStore>,
    pub offset: (i32, i32),
}

/// Fond d'un nouveau document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    White,
    Transparent,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub name: String,
    width: u32,
    height: u32,
    /// Résolution d'impression en pixels par pouce (métadonnée).
    pub dpi: f32,
    nodes: HashMap<LayerId, Arc<Layer>>,
    root: Vec<LayerId>,
    next_id: LayerId,
    /// Sélection active (`None` = rien de sélectionné, les outils agissent partout).
    selection: Option<Arc<MaskStore>>,
    /// Repères : ils font partie du document, donc de l'historique et du fichier.
    pub guides: Vec<Guide>,
    /// Repères verrouillés : ils restent visibles et aimantent toujours, mais ne se déplacent plus.
    /// Coché après une disposition de repères, pour ne pas défaire d'un geste ce qu'on vient de poser.
    pub guides_locked: bool,
    /// Tracé de travail de la plume.
    pub paths: Vec<SubPath>,
}

impl Document {
    pub fn new(name: impl Into<String>, width: u32, height: u32, dpi: f32) -> Result<Self, EngineError> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(EngineError::InvalidDimensions { width, height, max: MAX_DIMENSION });
        }
        Ok(Self { name: name.into(), width, height, dpi, nodes: HashMap::new(), root: Vec::new(), next_id: 1, selection: None, guides: Vec::new(), guides_locked: false, paths: Vec::new() })
    }

    pub fn with_background(
        name: impl Into<String>,
        width: u32,
        height: u32,
        dpi: f32,
        background: Background,
    ) -> Result<Self, EngineError> {
        let mut doc = Self::new(name, width, height, dpi)?;
        let mut pixels = TileStore::new();
        if background == Background::White {
            pixels.fill(width, height, [255; 4]);
        }
        doc.add_pixel_layer("Arrière-plan", pixels, None, 0)?;
        Ok(doc)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn layer(&self, id: LayerId) -> Result<&Layer, EngineError> {
        self.nodes.get(&id).map(Arc::as_ref).ok_or(EngineError::UnknownLayer(id))
    }

    pub fn layer_count(&self) -> usize {
        self.nodes.len()
    }

    /// Enfants d'un groupe, ou calques de premier niveau pour `None`. Du bas vers le haut.
    pub fn children_of(&self, parent: Option<LayerId>) -> Result<&[LayerId], EngineError> {
        match parent {
            None => Ok(&self.root),
            Some(id) => {
                let layer = self.layer(id)?;
                if layer.is_group() {
                    Ok(layer.children())
                } else {
                    Err(EngineError::InvalidOperation("ce calque n'est pas un groupe".into()))
                }
            }
        }
    }

    /// Parcours en profondeur, **du haut vers le bas** : l'ordre du panneau Calques.
    pub fn walk_top_down(&self) -> Vec<(usize, &Layer)> {
        fn visit<'a>(doc: &'a Document, ids: &[LayerId], depth: usize, out: &mut Vec<(usize, &'a Layer)>) {
            for id in ids.iter().rev() {
                if let Some(layer) = doc.nodes.get(id) {
                    out.push((depth, layer.as_ref()));
                    visit(doc, layer.children(), depth + 1, out);
                }
            }
        }
        let mut out = Vec::with_capacity(self.nodes.len());
        visit(self, &self.root, 0, &mut out);
        out
    }

    /// Emplacement juste au-dessus d'un calque : `(parent, index)`.
    pub fn slot_above(&self, id: LayerId) -> Result<(Option<LayerId>, usize), EngineError> {
        let parent = self.layer(id)?.parent;
        let index = self.children_of(parent)?.iter().position(|c| *c == id).map_or(0, |i| i + 1);
        Ok((parent, index))
    }

    // ---------------------------------------------------------------- création

    pub fn add_pixel_layer(
        &mut self,
        name: impl Into<String>,
        pixels: TileStore,
        parent: Option<LayerId>,
        index: usize,
    ) -> Result<LayerId, EngineError> {
        let props = LayerProps::named(name, BlendMode::Normal);
        self.attach(props, LayerKind::Pixel { pixels: Arc::new(pixels) }, parent, index)
    }

    /// Nouveau groupe vide, en mode Transfert comme dans Photoshop.
    pub fn add_group(
        &mut self,
        name: impl Into<String>,
        parent: Option<LayerId>,
        index: usize,
    ) -> Result<LayerId, EngineError> {
        let props = LayerProps::named(name, BlendMode::PassThrough);
        self.attach(props, LayerKind::Group { children: Vec::new(), expanded: true }, parent, index)
    }

    fn attach(
        &mut self,
        props: LayerProps,
        kind: LayerKind,
        parent: Option<LayerId>,
        index: usize,
    ) -> Result<LayerId, EngineError> {
        self.children_of(parent)?; // valide le parent avant toute modification
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.insert(id, Arc::new(Layer { id, parent, label: None, offset: (0, 0), props, mask: None, effects: Vec::new(), kind }));
        let siblings = self.children_mut(parent)?;
        let index = index.min(siblings.len());
        siblings.insert(index, id);
        // Inséré au milieu d'un groupe d'écrêtage (le calque juste au-dessus est écrêté) : le nouveau calque
        // l'est aussi, comme dans Photoshop. Sinon il deviendrait, vide, la nouvelle base du calque écrêté,
        // qui disparaîtrait de l'image.
        let above = siblings.get(index + 1).copied();
        let joins_clipping_group = index > 0 && above.is_some_and(|other| self.nodes.get(&other).is_some_and(|l| l.props.clipped));
        if joins_clipping_group && !self.layer(id)?.is_group() {
            self.layer_mut(id)?.props.clipped = true;
        }
        Ok(id)
    }

    /// Réinsère un nœud lu depuis un fichier, identifiant compris. La cohérence
    /// de l'arbre est vérifiée ensuite par [`Document::validate`].
    pub(crate) fn restore(&mut self, layer: Layer, root: Vec<LayerId>) {
        self.next_id = self.next_id.max(layer.id + 1);
        self.nodes.insert(layer.id, Arc::new(layer));
        self.root = root;
    }

    /// Vérifie que chaque nœud est atteignable exactement une fois depuis la racine.
    pub(crate) fn validate(&self) -> Result<(), EngineError> {
        let mut seen = HashSet::new();
        let mut stack: Vec<(Option<LayerId>, LayerId)> = self.root.iter().map(|id| (None, *id)).collect();
        while let Some((parent, id)) = stack.pop() {
            let layer = self.layer(id).map_err(|_| EngineError::Corrupt(format!("calque {id} référencé mais absent")))?;
            if layer.parent != parent || !seen.insert(id) {
                return Err(EngineError::Corrupt(format!("calque {id} mal rattaché")));
            }
            stack.extend(layer.children().iter().map(|child| (Some(id), *child)));
        }
        if seen.len() != self.nodes.len() {
            return Err(EngineError::Corrupt("calques orphelins".into()));
        }
        Ok(())
    }

    // ------------------------------------------------------------ modification

    fn layer_mut(&mut self, id: LayerId) -> Result<&mut Layer, EngineError> {
        self.nodes.get_mut(&id).map(Arc::make_mut).ok_or(EngineError::UnknownLayer(id))
    }

    fn children_mut(&mut self, parent: Option<LayerId>) -> Result<&mut Vec<LayerId>, EngineError> {
        match parent {
            None => Ok(&mut self.root),
            Some(id) => match &mut self.layer_mut(id)?.kind {
                LayerKind::Group { children, .. } => Ok(children),
                _ => Err(EngineError::InvalidOperation("ce calque n'est pas un groupe".into())),
            },
        }
    }

    /// Modifie les propriétés d'un calque. Le mode Transfert est refusé hors des groupes.
    pub fn edit_props(&mut self, id: LayerId, edit: impl FnOnce(&mut LayerProps)) -> Result<(), EngineError> {
        let layer = self.layer_mut(id)?;
        let mut props = layer.props.clone();
        edit(&mut props);
        if props.blend == BlendMode::PassThrough && !layer.is_group() {
            return Err(EngineError::InvalidOperation("le mode Transfert est réservé aux groupes".into()));
        }
        props.opacity = props.opacity.clamp(0.0, 1.0);
        props.fill = props.fill.clamp(0.0, 1.0);
        layer.props = props;
        Ok(())
    }

    /// Couleur d'étiquette d'un calque ou d'un dossier (panneau des calques). Sans effet sur l'image : la composition
    /// n'est pas périmée (l'étiquette ne fait pas partie de `same_structure`).
    pub fn set_label(&mut self, id: LayerId, label: Option<LayerLabel>) -> Result<(), EngineError> {
        self.layer_mut(id)?.label = label;
        Ok(())
    }

    pub fn set_expanded(&mut self, id: LayerId, value: bool) -> Result<(), EngineError> {
        match &mut self.layer_mut(id)?.kind {
            LayerKind::Group { expanded, .. } => {
                *expanded = value;
                Ok(())
            }
            _ => Err(EngineError::InvalidOperation("ce calque n'est pas un groupe".into())),
        }
    }

    /// Pixels modifiables d'un calque (copie à l'écriture à tous les niveaux).
    pub fn pixels_mut(&mut self, id: LayerId) -> Result<&mut TileStore, EngineError> {
        match &mut self.layer_mut(id)?.kind {
            LayerKind::Pixel { pixels } => Ok(Arc::make_mut(pixels)),
            LayerKind::Group { .. } => Err(EngineError::InvalidOperation("un groupe n'a pas de pixels".into())),
            LayerKind::Adjustment { .. } => Err(EngineError::InvalidOperation("un calque de réglage n'a pas de pixels".into())),
            LayerKind::Text { .. } => Err(EngineError::InvalidOperation(TEXT_IS_NOT_PAINTABLE.into())),
            LayerKind::Shape { .. } => Err(EngineError::InvalidOperation(SHAPE_IS_NOT_PAINTABLE.into())),
            LayerKind::Smart { .. } => Err(EngineError::InvalidOperation(smart::NOT_PAINTABLE.into())),
        }
    }

    pub fn mask_mut(&mut self, id: LayerId) -> Result<&mut MaskStore, EngineError> {
        match &mut self.layer_mut(id)?.mask {
            Some(mask) => Ok(Arc::make_mut(&mut mask.pixels)),
            None => Err(EngineError::InvalidOperation("ce calque n'a pas de masque de fusion".into())),
        }
    }

    /// Ajoute un masque de fusion : `reveal_all` = blanc (tout visible), sinon noir.
    pub fn add_mask(&mut self, id: LayerId, reveal_all: bool) -> Result<(), EngineError> {
        // Comme Photoshop : avec une sélection active, le masque la reprend (ce qui est sélectionné
        // reste visible, le reste est masqué sans être détruit), puis la sélection est abandonnée.
        let from_selection = if reveal_all { self.selection.take() } else { None };
        let layer = self.layer_mut(id)?;
        if layer.mask.is_some() {
            return Err(EngineError::InvalidOperation("ce calque a déjà un masque de fusion".into()));
        }
        let (ox, oy) = layer.offset;
        let pixels = match from_selection {
            Some(selection) => selection.translated(-ox, -oy), // le masque vit en repère du calque
            None => MaskStore::with_background(if reveal_all { [255] } else { [0] }),
        };
        layer.mask = Some(Mask { pixels: Arc::new(pixels), enabled: true });
        Ok(())
    }

    pub fn remove_mask(&mut self, id: LayerId) -> Result<(), EngineError> {
        self.layer_mut(id)?
            .mask
            .take()
            .map(|_| ())
            .ok_or_else(|| EngineError::InvalidOperation("ce calque n'a pas de masque de fusion".into()))
    }

    pub fn set_mask_enabled(&mut self, id: LayerId, enabled: bool) -> Result<(), EngineError> {
        match &mut self.layer_mut(id)?.mask {
            Some(mask) => {
                mask.enabled = enabled;
                Ok(())
            }
            None => Err(EngineError::InvalidOperation("ce calque n'a pas de masque de fusion".into())),
        }
    }

    pub fn selection(&self) -> Option<&Arc<MaskStore>> {
        self.selection.as_ref()
    }

    pub fn set_selection(&mut self, selection: Option<MaskStore>) {
        self.selection = selection.map(Arc::new);
    }

    /// Sélection modifiable en place (masque rapide) ; créée vide s'il n'y en a pas.
    pub(crate) fn selection_mut(&mut self) -> &mut MaskStore {
        Arc::make_mut(self.selection.get_or_insert_with(|| Arc::new(MaskStore::with_background([0]))))
    }

    // --------------------------------------------------------------- recadrage

    /// Recadre le document au rectangle `(x, y, largeur, hauteur)`, qui peut aussi déborder pour
    /// agrandir la zone de travail. Non destructif : les calques sont simplement décalés et gardent
    /// ce qui se retrouve hors cadre ; sélection, repères et tracé suivent.
    pub fn crop(&mut self, x: i32, y: i32, width: u32, height: u32) -> Result<(), EngineError> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(EngineError::InvalidDimensions { width, height, max: MAX_DIMENSION });
        }
        for layer in self.nodes.values_mut().map(Arc::make_mut) {
            layer.offset = (layer.offset.0 - x, layer.offset.1 - y);
        }
        if let Some(selection) = self.selection.take() {
            // La sélection est recalée puis limitée au nouveau cadre.
            let moved = selection.translated(-x, -y);
            let frame = Shape::Rect { x: 0.0, y: 0.0, width: width as f32, height: height as f32 }.rasterize(width, height);
            self.selection = combine(Some(&moved), frame, CombineMode::Intersect).map(Arc::new);
        }
        for guide in &mut self.guides {
            guide.position -= if guide.orientation == Orientation::Vertical { x as f32 } else { y as f32 };
        }
        let (w, h) = (width as f32, height as f32);
        self.guides.retain(|g| g.position >= 0.0 && g.position <= if g.orientation == Orientation::Vertical { w } else { h });
        for anchor in self.paths.iter_mut().flat_map(|sub| sub.anchors.iter_mut()) {
            for point in [&mut anchor.point, &mut anchor.handle_in, &mut anchor.handle_out] {
                *point = [point[0] - x as f32, point[1] - y as f32];
            }
        }
        (self.width, self.height) = (width, height);
        Ok(())
    }

    fn ensure_paintable(&self, id: LayerId) -> Result<Locks, EngineError> {
        if self.layer(id)?.text().is_some() {
            return Err(EngineError::InvalidOperation(TEXT_IS_NOT_PAINTABLE.into()));
        }
        if self.layer(id)?.shape().is_some() {
            return Err(EngineError::InvalidOperation(SHAPE_IS_NOT_PAINTABLE.into()));
        }
        if self.layer(id)?.smart().is_some() {
            return Err(EngineError::InvalidOperation(smart::NOT_PAINTABLE.into()));
        }
        let locks = self.layer(id)?.props.locks;
        if locks.all || locks.pixels {
            return Err(EngineError::InvalidOperation("les pixels de ce calque sont verrouillés".into()));
        }
        Ok(locks)
    }

    /// Applique `edit(pixel, force)` aux pixels du calque situés dans le document et
    /// dans la sélection. `force` vaut `1..=255` (valeur de la sélection, 255 sans sélection).
    /// `only_existing` : les tuiles absentes (transparentes) ne sont pas créées.
    fn edit_selected_pixels(
        &mut self,
        id: LayerId,
        only_existing: bool,
        edit: impl Fn(&mut [u8], u8, (i32, i32)),
    ) -> Result<(), EngineError> {
        let (ox, oy) = self.layer(id)?.offset;
        let selection = self.selection.clone();
        // Zone de travail en coordonnées du document : la sélection si elle est bornée, sinon tout le document.
        let (dw, dh) = (self.width as i32, self.height as i32);
        let (x0, y0, x1, y1) = match selection.as_ref().filter(|s| s.background() == [0]).map(|s| s.bounds()) {
            Some(None) => return Ok(()),
            Some(Some((bx0, by0, bx1, by1))) => (bx0.max(0), by0.max(0), bx1.min(dw), by1.min(dh)),
            None => (0, 0, dw, dh),
        };
        let mut mask = selection.as_deref().map(Sampler::new);
        let pixels = self.pixels_mut(id)?;
        let size = TILE_SIZE as i32;
        let (first, _, _) = TileCoord::of_pixel(x0 - ox, y0 - oy);
        let (last, _, _) = TileCoord::of_pixel(x1 - 1 - ox, y1 - 1 - oy);
        for ty in first.y..=last.y {
            for tx in first.x..=last.x {
                let coord = TileCoord::new(tx, ty);
                if only_existing && pixels.get(coord).is_none() {
                    continue;
                }
                // Coin haut gauche de la tuile locale, exprimé dans le document.
                let (doc_x, doc_y) = (tx * size + ox, ty * size + oy);
                // Tuile entièrement hors de la sélection (son rectangle englobant n'est pas elle) : on la
                // laisse partagée telle quelle, pour que seules les tuiles vraiment modifiées soient
                // signalées comme changées et retransmises à l'affichage.
                let rect = (x0.max(doc_x), y0.max(doc_y), x1.min(doc_x + size), y1.min(doc_y + size));
                if selection.as_deref().is_some_and(|s| !s.any_nonzero_in(rect)) {
                    continue;
                }
                let mut tile = pixels.get(coord).map_or_else(Tile::transparent, |t| (**t).clone());
                for y in y0.max(doc_y)..y1.min(doc_y + size) {
                    for x in x0.max(doc_x)..x1.min(doc_x + size) {
                        let force = mask.as_mut().map_or(255, |m| m.get(x, y)[0]);
                        if force > 0 {
                            let index = ((y - doc_y) * size + (x - doc_x)) as usize * 4;
                            edit(&mut tile.bytes_mut()[index..index + 4], force, (x, y));
                        }
                    }
                }
                pixels.insert(coord, tile);
            }
        }
        Ok(())
    }

    /// Alt+Retour arrière : remplit le calque sur l'étendue du document, dans la sélection
    /// s'il y en a une. Avec le verrou des pixels transparents, seule la couleur change.
    pub fn fill_layer(&mut self, id: LayerId, color: [u8; 3]) -> Result<(), EngineError> {
        let locks = self.ensure_paintable(id)?;
        let [r, g, b] = color;
        self.edit_selected_pixels(id, locks.transparency, |px, force, _| {
            let ink = premultiply([r, g, b, if locks.transparency { px[3] } else { 255 }]);
            for c in 0..4 {
                px[c] = mul_div_255(ink[c], force).saturating_add(mul_div_255(px[c], 255 - force));
            }
        })
    }

    /// Ctrl+I : inverse les couleurs des pixels du calque (dans la sélection s'il y en a une), avec
    /// la courbe `adjust::invert` — la même que le calque de réglage Inverser, destructive ici, le
    /// papier (blanc pur) restant blanc ; l'alpha n'est pas touché.
    pub fn invert_layer(&mut self, id: LayerId) -> Result<(), EngineError> {
        self.ensure_paintable(id)?;
        let table = crate::adjust::invert_table();
        self.edit_selected_pixels(id, true, move |px, force, _| {
            if px[3] == 0 {
                return;
            }
            // La courbe s'applique à la couleur droite : on sort du prémultiplié, puis on y revient.
            let straight = unpremultiply([px[0], px[1], px[2], px[3]]);
            let paper = crate::adjust::paper_weight([f32::from(straight[0]) / 255.0, f32::from(straight[1]) / 255.0, f32::from(straight[2]) / 255.0]);
            let mix = |c: usize| {
                let inverted = f32::from(table[usize::from(straight[c])]);
                (inverted + (f32::from(straight[c]) - inverted) * paper + 0.5) as u8
            };
            let inverted = premultiply([mix(0), mix(1), mix(2), px[3]]);
            for (channel, target) in px.iter_mut().zip(inverted).take(3) {
                *channel = mul_div_255(target, force).saturating_add(mul_div_255(*channel, 255 - force));
            }
        })
    }

    /// Touche Suppr avec une sélection active : efface les pixels sélectionnés du calque.
    pub fn clear_selected_pixels(&mut self, id: LayerId) -> Result<(), EngineError> {
        if self.ensure_paintable(id)?.transparency {
            return Err(EngineError::InvalidOperation("les pixels transparents de ce calque sont verrouillés".into()));
        }
        self.edit_selected_pixels(id, true, |px, force, _| {
            for channel in px.iter_mut() {
                *channel = mul_div_255(*channel, 255 - force);
            }
        })
    }

    /// Touche Suppr avec une sélection active, le masque de fusion étant visé : la sélection passe au
    /// noir dans le masque (masquée), à proportion de la sélection. Sans sélection, tout le masque.
    pub fn clear_selected_mask(&mut self, id: LayerId) -> Result<(), EngineError> {
        let (ox, oy) = self.layer(id)?.offset;
        let selection = self.selection.clone();
        if self.layer(id)?.props.locks.all {
            return Err(EngineError::InvalidOperation("ce calque est verrouillé".into()));
        }
        let store = self.mask_mut(id)?;
        let Some(selection) = selection else {
            *store = MaskStore::with_background([0]);
            return Ok(());
        };
        let Some((x0, y0, x1, y1)) = selection.pixel_bounds(|v| v[0] > 0) else { return Ok(()) };
        let mut chosen = Sampler::new(&selection);
        for y in y0..y1 {
            for x in x0..x1 {
                let force = u32::from(chosen.get(x, y)[0]);
                if force == 0 {
                    continue;
                }
                let (coord, px, py) = TileCoord::of_pixel(x - ox, y - oy);
                let tile = store.get_mut(coord);
                let old = u32::from(tile.pixel(px, py)[0]);
                tile.set_pixel(px, py, [((old * (255 - force) + 127) / 255) as u8]);
            }
        }
        Ok(())
    }

    /// Remplit le masque de fusion d'une valeur uniforme (0 = tout masquer, 255 = tout révéler).
    pub fn fill_mask(&mut self, id: LayerId, value: u8) -> Result<(), EngineError> {
        match &mut self.layer_mut(id)?.mask {
            Some(mask) => {
                mask.pixels = Arc::new(MaskStore::with_background([value]));
                Ok(())
            }
            None => Err(EngineError::InvalidOperation("ce calque n'a pas de masque de fusion".into())),
        }
    }

    /// Ctrl+A : sélectionne le CONTENU du calque — ses pixels opaques, à proportion de leur opacité —
    /// et non tout le plan de travail. Sur un calque sans contenu (groupe, réglage, calque vide), c'est
    /// tout le document qui est sélectionné, comme avant.
    pub fn select_layer_content(&mut self, id: LayerId) -> Result<(), EngineError> {
        let layer = self.layer(id)?;
        let (ox, oy) = layer.offset;
        let (width, height) = (self.width as i32, self.height as i32);
        let mut mask = MaskStore::with_background([0]);
        let mut any = false;
        if let Some(pixels) = layer.pixels() {
            for (coord, tile) in pixels.iter() {
                let (tx, ty) = coord.origin();
                for py in 0..TILE_SIZE as i32 {
                    let y = ty + py + oy;
                    if y < 0 || y >= height {
                        continue;
                    }
                    for px in 0..TILE_SIZE as i32 {
                        let x = tx + px + ox;
                        let alpha = tile.pixel(px as u32, py as u32)[3];
                        if alpha > 0 && x >= 0 && x < width {
                            let (target, mx, my) = TileCoord::of_pixel(x, y);
                            mask.get_mut(target).set_pixel(mx, my, [alpha]);
                            any = true;
                        }
                    }
                }
            }
        }
        self.selection = Some(Arc::new(if any { mask } else { crate::selection::select_all() }));
        Ok(())
    }

    /// Dégradé tracé dans le masque de fusion : la clarté du dégradé devient la valeur du masque (blanc =
    /// visible, noir = masqué), sur tout le plan de travail ou dans la sélection. C'est le moyen le plus
    /// simple de faire disparaître progressivement un calque.
    pub fn gradient_mask(&mut self, id: LayerId, from: (f32, f32), to: (f32, f32), stops: &[GradientStop], radial: bool) -> Result<(), EngineError> {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length_sq = dx * dx + dy * dy;
        if length_sq < 1.0 {
            return Err(EngineError::InvalidOperation("dégradé trop court : glissez sur une plus grande distance".into()));
        }
        if stops.len() < 2 {
            return Err(EngineError::InvalidOperation("un dégradé a besoin d'au moins deux arrêts".into()));
        }
        let mut stops = stops.to_vec();
        stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        // Clarté (0..255) du dégradé à la position `t` ; un arrêt transparent compte comme du noir.
        let gray_at = |t: f32| -> f32 {
            let after = stops.iter().position(|s| s.position >= t).unwrap_or(stops.len() - 1);
            let (a, b) = (stops[after.saturating_sub(1)], stops[after]);
            let span = b.position - a.position;
            let k = if span > 1e-6 { ((t - a.position) / span).clamp(0.0, 1.0) } else { 1.0 };
            let gray = |c: [u8; 4]| (f32::from(c[0]) * 0.299 + f32::from(c[1]) * 0.587 + f32::from(c[2]) * 0.114) * f32::from(c[3]) / 255.0;
            gray(a.color) + (gray(b.color) - gray(a.color)) * k
        };
        let (width, height) = (self.width as i32, self.height as i32);
        let (ox, oy) = self.layer(id)?.offset;
        let selection = self.selection.clone();
        let layer = self.layer_mut(id)?;
        if layer.props.locks.all {
            return Err(EngineError::InvalidOperation("ce calque est verrouillé".into()));
        }
        let mask = layer.mask.as_mut().ok_or_else(|| EngineError::InvalidOperation("ce calque n'a pas de masque de fusion".into()))?;
        let store = Arc::make_mut(&mut mask.pixels);
        let mut chosen = selection.as_deref().map(Sampler::new);
        for y in 0..height {
            for x in 0..width {
                let force = chosen.as_mut().map_or(255, |s| u32::from(s.get(x, y)[0]));
                if force == 0 {
                    continue;
                }
                let (vx, vy) = (x as f32 + 0.5 - from.0, y as f32 + 0.5 - from.1);
                let t = if radial { (vx * vx + vy * vy).sqrt() / length_sq.sqrt() } else { (vx * dx + vy * dy) / length_sq };
                // Un soupçon de bruit évite les bandes dans un long dégradé sur 8 bits.
                let hash = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
                let dither = (hash >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                let value = (gray_at(t.clamp(0.0, 1.0)) + dither).clamp(0.0, 255.0) as u32;
                let (coord, px, py) = TileCoord::of_pixel(x - ox, y - oy);
                let tile = store.get_mut(coord);
                let old = u32::from(tile.pixel(px, py)[0]);
                tile.set_pixel(px, py, [((old * (255 - force) + value * force + 127) / 255) as u8]);
            }
        }
        Ok(())
    }

    /// Copie (ou déplace) le masque de fusion d'un calque vers un autre : Alt + glisser la miniature du masque.
    /// Le masque reste à la même place dans le document, même si les deux calques n'ont pas la même position.
    pub fn copy_mask(&mut self, from: LayerId, to: LayerId, remove_source: bool) -> Result<(), EngineError> {
        if from == to {
            return Ok(());
        }
        let source = self.layer(from)?;
        let mask = source.mask.clone().ok_or_else(|| EngineError::InvalidOperation("ce calque n'a pas de masque de fusion".into()))?;
        let (sx, sy) = source.offset;
        let target = self.layer(to)?;
        if target.props.locks.all {
            return Err(EngineError::InvalidOperation("le calque de destination est verrouillé".into()));
        }
        let (tx, ty) = target.offset;
        let moved = Mask { pixels: Arc::new(mask.pixels.translated(sx - tx, sy - ty)), ..mask };
        self.layer_mut(to)?.mask = Some(moved);
        if remove_source {
            self.layer_mut(from)?.mask = None;
        }
        Ok(())
    }

    /// Ctrl+I sur un masque de fusion : ce qui était masqué se révèle et inversement (dans la sélection s'il y en a une).
    pub fn invert_mask(&mut self, id: LayerId) -> Result<(), EngineError> {
        let (ox, oy) = self.layer(id)?.offset;
        let selection = self.selection.clone();
        let layer = self.layer_mut(id)?;
        if layer.props.locks.all {
            return Err(EngineError::InvalidOperation("ce calque est verrouillé".into()));
        }
        let mask = layer.mask.as_mut().ok_or_else(|| EngineError::InvalidOperation("ce calque n'a pas de masque de fusion".into()))?;
        let store = Arc::make_mut(&mut mask.pixels);
        match selection {
            None => {
                // Tout le masque : les tuiles existantes ET le fond, qui couvre le reste du plan.
                let coords: Vec<TileCoord> = store.iter().map(|(coord, _)| coord).collect();
                for coord in coords {
                    let tile = store.get_mut(coord);
                    for value in tile.bytes_mut() {
                        *value = 255 - *value;
                    }
                }
                let background = store.background()[0];
                store.set_background([255 - background]);
            }
            Some(selection) => {
                let Some((x0, y0, x1, y1)) = selection.pixel_bounds(|v| v[0] > 0) else { return Ok(()) };
                let mut chosen = Sampler::new(&selection);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let force = u32::from(chosen.get(x, y)[0]);
                        if force == 0 {
                            continue;
                        }
                        let (coord, px, py) = TileCoord::of_pixel(x - ox, y - oy);
                        let tile = store.get_mut(coord);
                        let old = u32::from(tile.pixel(px, py)[0]);
                        tile.set_pixel(px, py, [((old * (255 - force) + (255 - old) * force + 127) / 255) as u8]);
                    }
                }
            }
        }
        Ok(())
    }

    /// Fusionne un groupe en un seul calque de pixels : tout ce qu'il contient (calques, réglages, effets,
    /// masques, modes de fusion internes) est rendu tel qu'affiché, puis remplace le groupe. Le calque obtenu
    /// garde le nom, l'opacité, le mode de fusion, l'écrêtage et le masque du groupe. Limité au plan de travail.
    /// Aperçu instantané d'un calque de réglage (curseurs) : l'image découpée en `(dessous, cible, dessus)`.
    /// L'interface applique le réglage sur la carte graphique à la cible — ou à « dessous » si la cible est vide
    /// (réglage libre, qui corrige tout ce qui est dessous). Cas pris en charge, à la racine de la pile, visible,
    /// en mode Normal, sans masque actif :
    ///  - réglage libre, sans calque écrêté sur lui ;
    ///  - réglage écrêté sur un calque de base en mode Normal, sans autre calque écrêté AU-DESSUS de lui (la cible
    ///    est alors la base et ce qui est écrêté sous le réglage).
    /// Sinon `None` : le moteur rend comme avant.
    pub fn adjustment_preview_plan(&self, id: LayerId) -> Option<(Vec<LayerId>, Vec<LayerId>, Vec<LayerId>)> {
        let layer = self.layer(id).ok()?;
        if layer.adjustment().is_none() || layer.parent.is_some() || !layer.props.visible || layer.props.blend != BlendMode::Normal {
            return None;
        }
        if layer.mask.as_ref().is_some_and(|m| m.enabled) {
            return None;
        }
        let roots = self.children_of(None).ok()?;
        let index = roots.iter().position(|r| *r == id)?;
        let clipped_at = |i: usize| roots.get(i).and_then(|r| self.layer(*r).ok()).is_some_and(|l| l.props.clipped);
        if clipped_at(index + 1) {
            return None; // un calque écrêté au-dessus du réglage : il faudrait le recomposer aussi
        }
        // Tous les calques (et leurs descendants) d'une tranche de la pile.
        let collect = |ids: &[LayerId]| -> Vec<LayerId> {
            let mut out = Vec::new();
            let mut stack: Vec<LayerId> = ids.to_vec();
            while let Some(current) = stack.pop() {
                out.push(current);
                if let Ok(node) = self.layer(current) {
                    stack.extend_from_slice(node.children());
                }
            }
            out
        };
        if !layer.props.clipped {
            return Some((collect(&roots[..index]), Vec::new(), collect(&roots[index + 1..])));
        }
        // Réglage écrêté : sa base est le premier calque non écrêté en dessous.
        let base = (0..index).rev().find(|i| !clipped_at(*i))?;
        let base_layer = self.layer(roots[base]).ok()?;
        if base_layer.props.blend != BlendMode::Normal || base_layer.adjustment().is_some() {
            return None;
        }
        Some((collect(&roots[..base]), collect(&roots[base..index]), collect(&roots[index + 1..])))
    }

    /// Fusionne plusieurs calques (pixels, textes, formes, réglages, groupes) en UN calque de pixels, placé là où
    /// était le plus haut d'entre eux et portant son nom. Les calques sont composés entre eux tels qu'affichés
    /// (modes de fusion, opacités, masques, effets), comme la fusion de Photoshop.
    pub fn merge_layers(&mut self, ids: &[LayerId]) -> Result<LayerId, EngineError> {
        let members = self.top_level_selection(ids)?;
        if members.len() < 2 {
            return Err(EngineError::InvalidOperation("sélectionnez au moins deux calques à fusionner".into()));
        }
        let name = self.layer(*members.last().expect("au moins deux calques"))?.props.name.clone();
        let group = self.group_layers(&members, name)?;
        // Le groupe temporaire est neutre (passe-à-travers, 100 %) : seul compte le rendu des calques eux-mêmes.
        self.layer_mut(group)?.props.blend = BlendMode::PassThrough;
        self.merge_group(group)
    }

    pub fn merge_group(&mut self, id: LayerId) -> Result<LayerId, EngineError> {
        let group = self.layer(id)?.clone();
        if !group.is_group() {
            return Err(EngineError::InvalidOperation("seul un groupe peut être fusionné ainsi".into()));
        }
        // Copie de travail où seul le contenu du groupe est visible, sans rien qui le module de l'extérieur.
        let mut scratch = self.clone();
        let mut inside: HashSet<LayerId> = HashSet::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            inside.insert(current);
            stack.extend_from_slice(scratch.layer(current)?.children());
        }
        let mut lineage: HashSet<LayerId> = HashSet::new();
        let mut cursor = Some(id);
        while let Some(current) = cursor {
            lineage.insert(current);
            cursor = scratch.layer(current)?.parent;
        }
        let all: Vec<LayerId> = scratch.nodes.keys().copied().collect();
        for other in all {
            let layer = scratch.layer_mut(other)?;
            if lineage.contains(&other) {
                layer.props.visible = true;
                layer.props.opacity = 1.0;
                layer.props.fill = 1.0;
                layer.props.clipped = false;
                layer.props.blend = if other == id { BlendMode::Normal } else { BlendMode::PassThrough };
                layer.mask = None;
                layer.effects.clear();
            } else if !inside.contains(&other) {
                layer.props.visible = false;
            }
        }
        let renderer = crate::render::TileRenderer::new();
        let mut flat = TileStore::new();
        for ty in 0..crate::tiles::tiles_for(self.height) as i32 {
            for tx in 0..crate::tiles::tiles_for(self.width) as i32 {
                let coord = TileCoord::new(tx, ty);
                flat.restore(coord, renderer.tile(&scratch, 0, coord));
            }
        }
        // Le calque prend la place du groupe, avec ses réglages ; ses pixels sont ramenés dans son repère.
        let (ox, oy) = group.offset;
        let (parent, index) = self.slot_above(id)?;
        let merged = self.add_pixel_layer(group.props.name.clone(), flat.translated(-ox, -oy), parent, index)?;
        let layer = self.layer_mut(merged)?;
        layer.offset = group.offset;
        layer.props = LayerProps { blend: if group.props.blend == BlendMode::PassThrough { BlendMode::Normal } else { group.props.blend }, ..group.props.clone() };
        layer.mask = group.mask.clone();
        self.remove_layers(&[id])?;
        Ok(merged)
    }

    // -------------------------------------------------- copier / coller des calques

    /// Copie des calques (groupes compris, avec tout leur contenu) pour les coller ensuite, dans ce document ou un autre.
    pub fn copy_layers(&self, ids: &[LayerId]) -> Result<LayerClip, EngineError> {
        let roots = self.top_level_selection(ids)?;
        let mut nodes = HashMap::new();
        let mut stack = roots.clone();
        while let Some(current) = stack.pop() {
            let layer = self.nodes.get(&current).ok_or_else(|| EngineError::InvalidOperation("calque introuvable".into()))?;
            stack.extend_from_slice(layer.children());
            nodes.insert(current, Arc::clone(layer));
        }
        Ok(LayerClip { roots, nodes })
    }

    /// Colle des calques copiés à l'emplacement `(parent, index)` ; retourne les nouveaux calques de premier niveau.
    pub fn paste_layers(&mut self, clip: &LayerClip, parent: Option<LayerId>, index: usize) -> Result<Vec<LayerId>, EngineError> {
        self.children_of(parent)?;
        let mut created = Vec::new();
        for (n, root) in clip.roots.iter().enumerate() {
            let copy = self.graft(clip, *root, parent)?;
            let siblings = self.children_mut(parent)?;
            let at = (index + n).min(siblings.len());
            siblings.insert(at, copy);
            created.push(copy);
        }
        Ok(created)
    }

    fn graft(&mut self, clip: &LayerClip, source: LayerId, parent: Option<LayerId>) -> Result<LayerId, EngineError> {
        let mut layer = (**clip.nodes.get(&source).ok_or_else(|| EngineError::Corrupt("calque copié incomplet".into()))?).clone();
        let id = self.next_id;
        self.next_id += 1;
        (layer.id, layer.parent) = (id, parent);
        if let LayerKind::Group { children, .. } = &mut layer.kind {
            let originals = std::mem::take(children);
            for child in originals {
                let copy = self.graft(clip, child, Some(id))?;
                if let LayerKind::Group { children, .. } = &mut layer.kind {
                    children.push(copy);
                }
            }
        }
        self.nodes.insert(id, Arc::new(layer));
        Ok(id)
    }

    /// Alt + clic sur l'œil : isole ce calque (tous les autres sont masqués) ; refait, tout redevient
    /// visible. Les ancêtres et les descendants du calque restent visibles : sans eux il ne s'afficherait pas.
    pub fn solo(&mut self, id: LayerId) -> Result<(), EngineError> {
        let mut kept: HashSet<LayerId> = HashSet::new();
        let mut cursor = Some(id);
        while let Some(current) = cursor {
            kept.insert(current);
            cursor = self.layer(current)?.parent;
        }
        let mut stack = self.layer(id)?.children().to_vec();
        while let Some(child) = stack.pop() {
            kept.insert(child);
            stack.extend_from_slice(self.layer(child)?.children());
        }
        let others: Vec<LayerId> = self.nodes.keys().copied().filter(|other| !kept.contains(other)).collect();
        let isolated = others.iter().all(|other| self.nodes.get(other).is_some_and(|l| !l.props.visible));
        for other in others {
            self.layer_mut(other)?.props.visible = isolated; // déjà isolé : on rallume tout
        }
        for own in kept {
            self.layer_mut(own)?.props.visible = true;
        }
        Ok(())
    }

    /// Remplace les effets d'un calque (« fx »). Les groupes et les calques de réglage n'en ont pas.
    pub fn set_effects(&mut self, id: LayerId, effects: Vec<LayerEffect>) -> Result<(), EngineError> {
        let layer = self.layer_mut(id)?;
        if layer.props.locks.all {
            return Err(EngineError::InvalidOperation("ce calque est verrouillé".into()));
        }
        if layer.pixels().is_none() {
            return Err(EngineError::InvalidOperation("les effets s'appliquent à un calque de pixels, de texte ou de forme".into()));
        }
        layer.effects = effects;
        Ok(())
    }

    // ------------------------------------------------------- réglages et filtres

    /// `clipped` : le réglage est écrêté sur le calque placé juste dessous, et ne corrige donc que lui.
    pub fn add_adjustment_layer(&mut self, adjustment: Adjustment, parent: Option<LayerId>, index: usize, clipped: bool) -> Result<LayerId, EngineError> {
        let mut props = LayerProps::named(adjustment.label(), BlendMode::Normal);
        // Rien dessous dans ce groupe : pas de base sur laquelle s'écrêter.
        props.clipped = clipped && index > 0;
        self.attach(props, LayerKind::Adjustment { adjustment }, parent, index)
    }

    /// Modifie les paramètres d'un calque de réglage ; le type de réglage ne change pas.
    pub fn set_adjustment(&mut self, id: LayerId, new: Adjustment) -> Result<(), EngineError> {
        let layer = self.layer_mut(id)?;
        if layer.props.locks.all {
            return Err(EngineError::InvalidOperation("ce calque est verrouillé".into()));
        }
        match &mut layer.kind {
            LayerKind::Adjustment { adjustment } if std::mem::discriminant(adjustment) == std::mem::discriminant(&new) => {
                *adjustment = new;
                Ok(())
            }
            _ => Err(EngineError::InvalidOperation("ce calque n'est pas un calque de réglage de ce type".into())),
        }
    }

    /// Filtre destructif sur les pixels du calque. Avec une sélection, seule la zone sélectionnée
    /// reçoit le résultat (fondu selon la force de la sélection) ; le verrou de transparence conserve l'alpha.
    pub fn apply_filter(&mut self, id: LayerId, filter: &Filter) -> Result<(), EngineError> {
        // Un filtre agit sur des pixels : une forme ou un texte sont d'abord pixellisés, dans le même état
        // d'historique (Ctrl+Z rend la forme ou le texte, réglables comme avant). Leurs effets sont conservés.
        let vector = self.layer(id)?;
        // Objet dynamique : les filtres dynamiques viendront ; d'ici là, refus explicite plutôt qu'une pixellisation.
        if vector.smart().is_some() {
            return Err(EngineError::InvalidOperation(smart::NOT_PAINTABLE.into()));
        }
        if vector.shape().is_some() || vector.text().is_some() {
            self.rasterize_layer(id)?;
        }
        let locks = self.ensure_paintable(id)?;
        let layer = self.layer(id)?;
        let offset = layer.offset;
        let original = Arc::clone(layer.pixels().ok_or_else(|| EngineError::InvalidOperation("un groupe ne se filtre pas".into()))?);
        let filtered = filters::apply(&original, filter)?;
        let result = match (&self.selection, locks.transparency) {
            (None, false) => filtered,
            (selection, keep_alpha) => {
                let selection = selection.as_deref();
                let mut out = TileStore::clone(&original);
                let mut coords = filtered.sorted_coords();
                coords.extend(original.sorted_coords());
                coords.sort_unstable();
                coords.dedup();
                let (mut before, mut after) = (Sampler::new(&original), Sampler::new(&filtered));
                let mut mask = selection.map(Sampler::new);
                for coord in coords {
                    let (ox, oy) = coord.origin();
                    let tile = out.get_mut(coord);
                    for py in 0..TILE_SIZE {
                        for px in 0..TILE_SIZE {
                            let (x, y) = (ox + px as i32, oy + py as i32);
                            let force = mask.as_mut().map_or(255, |m| m.get(x + offset.0, y + offset.1)[0]);
                            if force == 0 {
                                continue;
                            }
                            let (old, new) = (before.get(x, y), after.get(x, y));
                            let mut mixed = [0, 1, 2, 3].map(|c| mul_div_255(new[c], force).saturating_add(mul_div_255(old[c], 255 - force)));
                            if keep_alpha {
                                mixed[3] = old[3];
                                for channel in mixed.iter_mut().take(3) {
                                    *channel = (*channel).min(old[3]);
                                }
                            }
                            tile.set_pixel(px, py, mixed);
                        }
                    }
                }
                out
            }
        };
        match &mut self.layer_mut(id)?.kind {
            LayerKind::Pixel { pixels } => *pixels = Arc::new(result),
            _ => return Err(EngineError::InvalidOperation("seul un calque de pixels se filtre".into())),
        }
        Ok(())
    }

    /// Aperçu d'un filtre pour sa boîte de dialogue : le contenu du calque réduit pour tenir dans
    /// `max_side` pixels, filtré avec des réglages mis à la même échelle. Retourne largeur, hauteur
    /// et pixels RGBA à alpha droit. Rapide quelle que soit la taille du calque, et sans toucher au document.
    pub fn filter_preview(&self, id: LayerId, filter: &Filter, max_side: u32) -> Result<(u32, u32, Vec<u8>), EngineError> {
        let layer = self.layer(id)?;
        if layer.smart().is_some() {
            return Err(EngineError::InvalidOperation(smart::NOT_PAINTABLE.into()));
        }
        let pixels = layer.pixels().ok_or_else(|| EngineError::InvalidOperation("un groupe ne se filtre pas".into()))?;
        let Some((x0, y0, x1, y1)) = pixels.pixel_bounds(|px| px[3] > 0) else {
            return Err(EngineError::InvalidOperation("ce calque est vide".into()));
        };
        let k = (f64::from(max_side.max(16)) / f64::from((x1 - x0).max(y1 - y0))).min(1.0);
        let (w, h) = ((f64::from(x1 - x0) * k).round().max(1.0) as u32, (f64::from(y1 - y0) * k).round().max(1.0) as u32);
        let origin = pixels.translated(-x0, -y0);
        let small = if k < 1.0 { transform_store(&origin, &Matrix::scaling(k), Quality::Final)? } else { origin };
        let filtered = filters::apply(&small, &filter.scaled(k as f32))?;
        let mut rgba = filtered.to_premultiplied(w, h);
        for px in rgba.chunks_exact_mut(4) {
            let straight = crate::pixel::unpremultiply([px[0], px[1], px[2], px[3]]);
            px.copy_from_slice(&straight);
        }
        Ok((w, h, rgba))
    }

    /// Correcteur localisé : la zone couverte par le trait (points en repère document) est reconstruite
    /// à partir d'une zone voisine, raccordée en lumière et en couleur. Limité à la sélection s'il y en a une.
    pub fn heal(&mut self, id: LayerId, points: &[(f32, f32)], size: f32, hardness: f32) -> Result<(), EngineError> {
        self.ensure_paintable(id)?;
        let layer = self.layer(id)?;
        let (ox, oy) = layer.offset;
        let pixels = Arc::clone(layer.pixels().ok_or_else(|| EngineError::InvalidOperation("un groupe ne se retouche pas ainsi".into()))?);
        let local: Vec<(f32, f32)> = points.iter().map(|(x, y)| (x - ox as f32, y - oy as f32)).collect();
        let mut mask = crate::paint::stroke_mask(&local, size, hardness);
        if let Some(selection) = &self.selection {
            match combine(Some(&selection.translated(-ox, -oy)), mask, CombineMode::Intersect) {
                Some(limited) => mask = limited,
                None => return Ok(()), // le trait est entièrement hors de la sélection
            }
        }
        let healed = crate::heal::heal(&pixels, &mask)?;
        match &mut self.layer_mut(id)?.kind {
            LayerKind::Pixel { pixels } => *pixels = Arc::new(healed),
            _ => return Err(EngineError::InvalidOperation("seul un calque de pixels se retouche".into())),
        }
        Ok(())
    }

    // ---------------------------------------------------------- presse-papiers

    /// Ctrl+C : pixels du calque compris dans la sélection (tout le calque sans sélection).
    pub fn copy_pixels(&self, id: LayerId) -> Result<Clip, EngineError> {
        let layer = self.layer(id)?;
        let source = layer.pixels().ok_or_else(|| EngineError::InvalidOperation("un groupe ne se copie pas ainsi".into()))?;
        let pixels = match &self.selection {
            Some(selection) => Arc::new(split_by_selection(source, layer.offset, selection).0),
            None => Arc::clone(source),
        };
        if pixels.tile_count() == 0 {
            return Err(EngineError::InvalidOperation("rien à copier : la sélection est vide sur ce calque".into()));
        }
        Ok(Clip { pixels, offset: layer.offset })
    }

    /// Ctrl+V : nouveau calque au même endroit que l'original. Les pixels sont partagés, pas recopiés.
    pub fn paste(&mut self, clip: &Clip, name: impl Into<String>, parent: Option<LayerId>, index: usize) -> Result<LayerId, EngineError> {
        let id = self.add_pixel_layer(name, TileStore::clone(&clip.pixels), parent, index)?;
        self.layer_mut(id)?.offset = clip.offset;
        Ok(id)
    }

    // ------------------------------------------------------------- remplissages

    /// Exécute `work` avec `mask` (croisé avec la sélection active) comme sélection provisoire.
    fn within_mask(&mut self, mask: MaskStore, work: impl FnOnce(&mut Self) -> Result<(), EngineError>) -> Result<(), EngineError> {
        let saved = self.selection.take();
        self.selection = match &saved {
            Some(current) => combine(Some(current), mask, CombineMode::Intersect).map(Arc::new),
            None => Some(Arc::new(mask)),
        };
        let result = if self.selection.is_some() { work(self) } else { Ok(()) };
        self.selection = saved;
        result
    }

    /// Pot de peinture : remplit la zone de couleur voisine du point cliqué sur le calque.
    pub fn bucket_fill(&mut self, id: LayerId, (x, y): (u32, u32), color: [u8; 3], tolerance: u8, contiguous: bool) -> Result<(), EngineError> {
        self.ensure_paintable(id)?;
        let layer = self.layer(id)?;
        let pixels = Arc::clone(layer.pixels().ok_or_else(|| EngineError::InvalidOperation("un groupe n'a pas de pixels".into()))?);
        let Some(mask) = magic_wand(&pixels, layer.offset, (self.width, self.height), (x, y), tolerance, contiguous) else { return Ok(()) };
        self.within_mask(mask, |doc| doc.fill_layer(id, color))
    }

    /// Dégradé linéaire ou radial de `from` à `to` (repère document), entre deux couleurs RGBA à
    /// alpha droit. Un léger bruit déterministe casse les bandes visibles des dégradés en 8 bits.
    pub fn gradient_fill(&mut self, id: LayerId, from: (f32, f32), to: (f32, f32), stops: &[GradientStop], radial: bool) -> Result<(), EngineError> {
        let locks = self.ensure_paintable(id)?;
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length_sq = dx * dx + dy * dy;
        if length_sq < 1.0 {
            return Err(EngineError::InvalidOperation("dégradé trop court : glissez sur une plus grande distance".into()));
        }
        if stops.len() < 2 {
            return Err(EngineError::InvalidOperation("un dégradé a besoin d'au moins deux arrêts".into()));
        }
        let mut stops = stops.to_vec();
        stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        // Couleur RGBA (0..1) à la position `t` : interpolation entre les deux arrêts qui l'encadrent.
        let color_at = move |t: f32| -> [f32; 4] {
            let after = stops.iter().position(|s| s.position >= t).unwrap_or(stops.len() - 1);
            let (a, b) = (stops[after.saturating_sub(1)], stops[after]);
            let span = b.position - a.position;
            let k = if span > 1e-6 { ((t - a.position) / span).clamp(0.0, 1.0) } else { 1.0 };
            [0, 1, 2, 3].map(|c| (f32::from(a.color[c]) + (f32::from(b.color[c]) - f32::from(a.color[c])) * k) / 255.0)
        };
        self.edit_selected_pixels(id, locks.transparency, move |px, force, (x, y)| {
            let (vx, vy) = (x as f32 + 0.5 - from.0, y as f32 + 0.5 - from.1);
            let t = if radial { (vx * vx + vy * vy).sqrt() / length_sq.sqrt() } else { (vx * dx + vy * dy) / length_sq };
            let hash = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
            let dither = ((hash >> 8) as f32 / (1u32 << 24) as f32 - 0.5) / 255.0;
            let color = color_at(t.clamp(0.0, 1.0));
            let mix = |c: usize| (color[c] + dither).clamp(0.0, 1.0);
            let alpha = mix(3) * f32::from(force) / 255.0;
            let keep = 1.0 - alpha;
            let existing = px[3];
            for c in 0..3 {
                px[c] = ((mix(c) * alpha + f32::from(px[c]) / 255.0 * keep) * 255.0 + 0.5) as u8;
            }
            px[3] = ((alpha + f32::from(existing) / 255.0 * keep) * 255.0 + 0.5) as u8;
            if locks.transparency {
                // L'alpha d'origine est conservé ; la couleur, prémultipliée, ne doit pas le dépasser.
                px[3] = existing;
                for channel in px.iter_mut().take(3) {
                    *channel = (*channel).min(existing);
                }
            }
        })
    }

    // ------------------------------------------------------------------ tracés

    /// Masque du tracé de travail (sous-tracés d'au moins trois points, fermés d'office).
    fn path_mask(&self) -> Option<MaskStore> {
        let contours: Vec<Vec<[f32; 2]>> = self.paths.iter().map(SubPath::flatten).filter(|c| c.len() >= 3).collect();
        (!contours.is_empty()).then(|| Shape::Contours { contours }.rasterize(self.width, self.height))
    }

    /// Ctrl+Entrée : convertit le tracé en sélection, combinée à la sélection existante. Le tracé a
    /// fait son office : il disparaît. Rien n'est perdu pour autant, l'opération étant un état
    /// d'historique comme un autre : Ctrl+Z rend le tracé, et une sélection abandonnée par
    /// mégarde (Ctrl+D) se récupère de la même façon.
    pub fn path_to_selection(&mut self, mode: CombineMode) -> Result<(), EngineError> {
        let mask = self.path_mask().ok_or_else(|| EngineError::InvalidOperation(NO_CLOSED_PATH.into()))?;
        let combined = combine(self.selection.as_deref(), mask, mode);
        self.set_selection(combined);
        self.paths.clear();
        Ok(())
    }

    /// Fond du tracé : remplit l'intérieur du tracé sur le calque, dans la sélection active s'il y en a une.
    pub fn fill_path(&mut self, id: LayerId, color: [u8; 3]) -> Result<(), EngineError> {
        let mask = self.path_mask().ok_or_else(|| EngineError::InvalidOperation(NO_CLOSED_PATH.into()))?;
        // Le remplissage habituel fait le travail : il suffit que le tracé tienne lieu de sélection.
        self.within_mask(mask, |doc| doc.fill_layer(id, color))
    }

    /// Contour du tracé : le pinceau suit chaque sous-tracé, comme un trait fait à la main.
    pub fn stroke_path(&mut self, id: LayerId, brush: Brush) -> Result<(), EngineError> {
        let contours: Vec<Vec<[f32; 2]>> = self.paths.iter().map(SubPath::flatten).filter(|c| c.len() >= 2).collect();
        if contours.is_empty() {
            return Err(EngineError::InvalidOperation("le tracé est vide".into()));
        }
        for contour in contours {
            let points: Vec<StrokePoint> = contour.iter().map(|p| StrokePoint { x: p[0], y: p[1], pressure: 1.0 }).collect();
            Stroke::begin(self, id, false, brush)?.extend(self, &points)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------ formes

    /// Outil Forme : nouveau calque de forme dont la boîte a son coin haut gauche en `(x, y)`.
    pub fn add_shape_layer(&mut self, data: ShapeData, (x, y): (i32, i32), parent: Option<LayerId>, index: usize) -> Result<LayerId, EngineError> {
        let (data, (dx, dy)) = data.normalized();
        let (x, y) = (x + dx, y + dy);
        let raster = Arc::new(shape::render(&data)?);
        let name = match (data.kind, data.sides) {
            (shape::ShapeKind::Rect, _) => "Rectangle",
            (shape::ShapeKind::Ellipse, _) => "Ellipse",
            (shape::ShapeKind::Polygon, 3) => "Triangle",
            (shape::ShapeKind::Polygon, _) => "Polygone",
        };
        let id = self.attach(LayerProps::named(name, BlendMode::Normal), LayerKind::Shape { shape: data, raster }, parent, index)?;
        self.layer_mut(id)?.offset = (x, y);
        Ok(id)
    }

    /// Modifie une forme (taille, arrondi, fond, contour) et la redessine.
    /// Redimensionne une forme en la redessinant à sa nouvelle taille, posée au coin `(x, y)` : ses coins arrondis,
    /// son contour et ses côtés gardent leurs dimensions (rien n'est étiré), comme une forme de Photoshop.
    pub fn place_shape(&mut self, id: LayerId, x: i32, y: i32, width: f32, height: f32) -> Result<(), EngineError> {
        let current = self.layer(id)?.shape().cloned().ok_or_else(|| EngineError::InvalidOperation("ce calque n'est pas un calque de forme".into()))?;
        let (width, height) = (width.max(1.0), height.max(1.0));
        if current.transform == Matrix::IDENTITY {
            self.set_shape(id, ShapeData { width, height, ..current })?;
            self.layer_mut(id)?.offset = (x, y);
            return Ok(());
        }
        // Forme tournée ou déformée : `(x, y, largeur, hauteur)` décrivent sa boîte englobante ; la
        // transformation est ajustée pour que le dessin la remplisse, boîte et contour à l'échelle.
        let (bx0, by0, bx1, by1) = current.bounds();
        let (bw, bh) = (f64::from((bx1 - bx0).max(1)), f64::from((by1 - by0).max(1)));
        let fit = Matrix([f64::from(width) / bw, 0.0, 0.0, 0.0, f64::from(height) / bh, 0.0, 0.0, 0.0, 1.0]);
        let transform = fit.then_after(&Matrix::translation(-f64::from(bx0), -f64::from(by0))).then_after(&current.transform);
        self.set_shape(id, ShapeData { transform, ..current })?;
        self.layer_mut(id)?.offset = (x, y);
        Ok(())
    }

    /// Modifie une forme (taille, arrondi, fond, contour, transformation) et la redessine. Le dessin
    /// reste calé sur la position du calque : si la boîte englobante bouge (forme tournée dont la
    /// taille change), la position suit.
    pub fn set_shape(&mut self, id: LayerId, new: ShapeData) -> Result<(), EngineError> {
        let (new, (dx, dy)) = new.normalized();
        let rendered = Arc::new(shape::render(&new)?);
        let layer = self.layer_mut(id)?;
        if layer.props.locks.all {
            return Err(EngineError::InvalidOperation("ce calque est verrouillé".into()));
        }
        let LayerKind::Shape { shape, raster } = &mut layer.kind else {
            return Err(EngineError::InvalidOperation("ce calque n'est pas un calque de forme".into()));
        };
        (*shape, *raster) = (new, rendered);
        layer.offset = (layer.offset.0 + dx, layer.offset.1 + dy);
        Ok(())
    }

    // ------------------------------------------------------------------- texte

    /// Outil Texte : nouveau calque de texte ancré en `(x, y)` dans le document.
    pub fn add_text_layer(
        &mut self,
        text: TextData,
        (x, y): (i32, i32),
        parent: Option<LayerId>,
        index: usize,
    ) -> Result<LayerId, EngineError> {
        let raster = Arc::new(text::render(&text)?);
        let props = LayerProps::named(text_layer_name(&text), BlendMode::Normal);
        let id = self.attach(props, LayerKind::Text { text, transform: Matrix::IDENTITY, raster }, parent, index)?;
        self.layer_mut(id)?.offset = (x, y);
        Ok(id)
    }

    /// Modifie un calque de texte et recalcule son rendu. Le nom suit le contenu tant
    /// qu'il n'a pas été choisi par l'utilisateur, comme dans Photoshop.
    pub fn set_text(&mut self, id: LayerId, new: TextData) -> Result<(), EngineError> {
        let matrix = self.text_transform(id)?;
        let rendered = Arc::new(text::render_transformed(&new, &matrix, Quality::Final)?);
        let layer = self.layer_mut(id)?;
        if layer.props.locks.all {
            return Err(EngineError::InvalidOperation("ce calque est verrouillé".into()));
        }
        let LayerKind::Text { text, raster, .. } = &mut layer.kind else {
            return Err(EngineError::InvalidOperation(NOT_A_TEXT_LAYER.into()));
        };
        if layer.props.name == text_layer_name(text) {
            layer.props.name = text_layer_name(&new);
        }
        *text = new;
        *raster = rendered;
        Ok(())
    }

    fn text_transform(&self, id: LayerId) -> Result<Matrix, EngineError> {
        match &self.layer(id)?.kind {
            LayerKind::Text { transform, .. } => Ok(*transform),
            _ => Err(EngineError::InvalidOperation(NOT_A_TEXT_LAYER.into())),
        }
    }

    /// Repère texte -> repère document, pour placer le curseur et pointer dans le texte.
    fn text_to_document(&self, id: LayerId) -> Result<Matrix, EngineError> {
        let (ox, oy) = self.layer(id)?.offset;
        Ok(Matrix::translation(f64::from(ox), f64::from(oy)).then_after(&self.text_transform(id)?))
    }

    /// Curseur et sélection d'un calque de texte en cours d'édition, en repère document :
    /// les deux extrémités du trait du curseur, et un quadrilatère par ligne sélectionnée.
    /// `start`, `end` : positions en unités UTF-16, comme dans un champ de saisie.
    #[allow(clippy::type_complexity)]
    pub fn text_caret(&self, id: LayerId, start: usize, end: usize) -> Result<([(f64, f64); 2], Vec<[(f64, f64); 4]>), EngineError> {
        let data = self.layer(id)?.text().ok_or_else(|| EngineError::InvalidOperation(NOT_A_TEXT_LAYER.into()))?;
        let placed = self.text_to_document(id)?;
        let geometry = text::caret_geometry(data, start, end)?;
        let map = |(x, y): (f32, f32)| placed.apply(f64::from(x), f64::from(y)).unwrap_or((f64::from(x), f64::from(y)));
        let quads = geometry.highlights.iter().map(|(x, y, w, h)| [map((*x, *y)), map((x + w, *y)), map((x + w, y + h)), map((*x, y + h))]);
        Ok(([map(geometry.caret.0), map(geometry.caret.1)], quads.collect()))
    }

    /// Position du curseur (UTF-16) la plus proche du point `(x, y)` du document.
    pub fn text_hit(&self, id: LayerId, x: f64, y: f64) -> Result<usize, EngineError> {
        let data = self.layer(id)?.text().ok_or_else(|| EngineError::InvalidOperation(NOT_A_TEXT_LAYER.into()))?;
        let inverse = self.text_to_document(id)?.inverse().ok_or_else(|| EngineError::InvalidOperation("texte aplati".into()))?;
        let (tx, ty) = inverse.apply(x, y).unwrap_or((x, y));
        text::hit_test(data, tx as f32, ty as f32)
    }

    /// Pixellise un calque de texte, de forme ou un objet dynamique : il devient un calque de pixels ordinaire
    /// (peinture, transformation…) avec son rendu actuel, et n'est plus éditable comme avant.
    pub fn rasterize_layer(&mut self, id: LayerId) -> Result<(), EngineError> {
        let layer = self.layer_mut(id)?;
        let (LayerKind::Text { raster, .. } | LayerKind::Shape { raster, .. } | LayerKind::Smart { raster, .. }) = &layer.kind else {
            return Err(EngineError::InvalidOperation("seul un calque de texte, de forme ou un objet dynamique peut être pixellisé".into()));
        };
        layer.kind = LayerKind::Pixel { pixels: Arc::clone(raster) };
        Ok(())
    }

    // ------------------------------------------------------- objets dynamiques

    /// Pose un contenu sur un calque objet dynamique : `full` (repère contenu -> document) est séparée en position
    /// et transformation locale (voir `smart::normalize`), et le rendu recalculé depuis l'aplat du contenu. Le
    /// masque de fusion, qui vit dans le repère du calque, est recalé pour rester à sa place dans le document.
    fn place_smart(&mut self, id: LayerId, content: Arc<SmartContent>, full: &Matrix, quality: Quality) -> Result<(), EngineError> {
        let (offset, local) = smart::normalize(full);
        let raster = content.render(&local, quality)?;
        let layer = self.layer_mut(id)?;
        let (dx, dy) = (layer.offset.0 - offset.0, layer.offset.1 - offset.1);
        if let Some(mask) = layer.mask.as_mut().filter(|_| (dx, dy) != (0, 0)) {
            mask.pixels = Arc::new(mask.pixels.translated(dx, dy));
        }
        layer.offset = offset;
        layer.kind = LayerKind::Smart { content, transform: local, raster };
        Ok(())
    }

    /// Nouveau calque objet dynamique, de transformation `full` (repère contenu -> document).
    pub fn add_smart_layer(&mut self, content: Arc<SmartContent>, name: impl Into<String>, full: &Matrix, parent: Option<LayerId>, index: usize) -> Result<LayerId, EngineError> {
        let raster = Arc::clone(&content.flattened);
        let kind = LayerKind::Smart { content: Arc::clone(&content), transform: Matrix::IDENTITY, raster };
        let id = self.attach(LayerProps::named(name, BlendMode::Normal), kind, parent, index)?;
        self.place_smart(id, content, full, Quality::Final)?;
        Ok(id)
    }

    /// « Placer » : le contenu devient un nouveau calque, centré dans le document et réduit pour y tenir entièrement
    /// s'il est plus grand, comme dans Photoshop ; jamais agrandi (Ctrl+T le fera sans perte, l'original restant
    /// intact). À sa taille d'origine, il est placé sur des pixels entiers : aucun rééchantillonnage.
    pub fn place_smart_object(&mut self, content: SmartContent, parent: Option<LayerId>, index: usize) -> Result<LayerId, EngineError> {
        let (w, h) = (f64::from(content.width()), f64::from(content.height()));
        let (dw, dh) = (f64::from(self.width), f64::from(self.height));
        let fit = (dw / w).min(dh / h).min(1.0);
        let (x, y) = (((dw - w * fit) / 2.0).round(), ((dh - h * fit) / 2.0).round());
        let full = Matrix::translation(x, y).then_after(&Matrix::scaling(fit));
        let name = content.name.clone();
        self.add_smart_layer(Arc::new(content), name, &full, parent, index)
    }

    /// « Convertir en objet dynamique » : les calques donnés (groupes, masques, effets et réglages compris) deviennent
    /// le contenu d'un objet dynamique, dans un document de la taille de leur emprise (effets compris, contenu hors
    /// cadre compris), qui prend leur place dans la pile. La transformation est la translation vers leur position :
    /// le rendu est le même qu'avant, au pixel près pour des calques en mode Normal. Retourne le nouveau calque.
    pub fn convert_to_smart(&mut self, ids: &[LayerId]) -> Result<LayerId, EngineError> {
        let members = self.top_level_selection(ids)?;
        let topmost = *members.last().ok_or_else(|| EngineError::InvalidOperation("aucun calque à convertir".into()))?;
        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        let mut stack = members.clone();
        while let Some(current) = stack.pop() {
            let layer = self.layer(current)?;
            stack.extend_from_slice(layer.children());
            let Some((x0, y0, x1, y1)) = layer.pixels().and_then(|p| p.pixel_bounds(|px| px[3] > 0)) else { continue };
            // Une ombre ou un halo débordent des pixels : l'emprise les englobe, sinon le contenu les couperait.
            let margin = layer.effects.iter().filter(|e| e.enabled()).map(LayerEffect::reach).fold(0.0f32, f32::max).ceil() as i32;
            let (ox, oy) = layer.offset;
            let here = (x0 + ox - margin, y0 + oy - margin, x1 + ox + margin, y1 + oy + margin);
            bounds = Some(bounds.map_or(here, |b| (b.0.min(here.0), b.1.min(here.1), b.2.max(here.2), b.3.max(here.3))));
        }
        let (x0, y0, x1, y1) = bounds.ok_or_else(|| EngineError::InvalidOperation("rien à convertir : les calques sélectionnés sont vides".into()))?;
        let name = self.layer(topmost)?.props.name.clone();
        let clip = self.copy_layers(&members)?;
        let mut inner = Document::new(name.clone(), self.width, self.height, self.dpi)?;
        inner.paste_layers(&clip, None, 0)?;
        // Le contenu n'a rien sous lui : un calque écrêté en bas de sa pile n'aurait pas de base. Il ne l'est plus, et
        // c'est l'objet dynamique qui s'écrête à sa place.
        let mut clipped = false;
        for (n, root) in inner.root.clone().into_iter().enumerate() {
            let layer = inner.layer_mut(root)?;
            if !layer.props.clipped {
                break;
            }
            layer.props.clipped = false;
            clipped |= n == 0;
        }
        inner.crop(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)?;
        let content = SmartContent::new(smart::new_content_id(), name.clone(), Arc::new(inner), SmartSource::Embedded { original: None });
        let (parent, index) = self.slot_above(topmost)?;
        let id = self.add_smart_layer(Arc::new(content), name, &Matrix::translation(f64::from(x0), f64::from(y0)), parent, index)?;
        // L'objet prend la place des calques : base d'un groupe d'écrêtage s'ils l'étaient, écrêté s'ils l'étaient.
        self.layer_mut(id)?.props.clipped = clipped;
        self.remove_layers(&members)?;
        Ok(id)
    }

    fn smart_content(&self, id: LayerId) -> Result<Arc<SmartContent>, EngineError> {
        self.layer(id)?.smart().cloned().ok_or_else(|| EngineError::InvalidOperation(NOT_A_SMART_OBJECT.into()))
    }

    /// Instances d'un contenu : les calques objets dynamiques qui le partagent, dans l'ordre du panneau.
    pub fn smart_instances(&self, content: ContentId) -> Vec<LayerId> {
        self.walk_top_down().into_iter().filter(|(_, l)| l.smart().is_some_and(|c| c.id == content)).map(|(_, l)| l.id).collect()
    }

    /// Géométrie affichable d'un objet dynamique (échelles, angle, coins).
    pub fn smart_geometry(&self, id: LayerId) -> Option<SmartGeometry> {
        let layer = self.layer(id).ok()?;
        let content = layer.smart()?;
        Some(SmartGeometry::of(&layer.smart_transform()?, content.width(), content.height()))
    }

    /// Remplace le contenu de TOUTES les instances de `content.id` et refait leur rendu. Si les dimensions changent,
    /// `fit` dit comment la transformation de chacune s'adapte. Un contenu au même aplat (lien incorporé, contenu
    /// marqué manquant) n'est pas rendu de nouveau. Retourne le nombre d'instances mises à jour.
    pub fn update_smart_content(&mut self, content: Arc<SmartContent>, fit: SmartFit) -> Result<usize, EngineError> {
        let instances = self.smart_instances(content.id);
        if instances.is_empty() {
            return Err(EngineError::InvalidOperation("plus aucun calque n'utilise ce contenu".into()));
        }
        for id in &instances {
            let layer = self.layer(*id)?;
            let (old, full) = (self.smart_content(*id)?, layer.smart_transform().unwrap_or(Matrix::IDENTITY));
            if Arc::ptr_eq(&old.flattened, &content.flattened) {
                if let LayerKind::Smart { content: current, .. } = &mut self.layer_mut(*id)?.kind {
                    *current = Arc::clone(&content);
                }
                continue;
            }
            let (w0, h0, w1, h1) = (f64::from(old.width()), f64::from(old.height()), f64::from(content.width()), f64::from(content.height()));
            let adjusted = if (w0, h0) == (w1, h1) {
                full
            } else {
                match fit {
                    // Le centre reste en place, l'échelle aussi ; décalage arrondi au pixel pour rester net.
                    SmartFit::Centre => full.then_after(&Matrix::translation(((w0 - w1) / 2.0).round(), ((h0 - h1) / 2.0).round())),
                    // Le nouveau contenu occupe la place de l'ancien : même centre, mis à l'échelle pour y tenir.
                    SmartFit::DisplayedSize => {
                        let k = (w0 / w1).min(h0 / h1);
                        full.then_after(&Matrix::translation(w0 / 2.0, h0 / 2.0)).then_after(&Matrix::scaling(k)).then_after(&Matrix::translation(-w1 / 2.0, -h1 / 2.0))
                    }
                }
            };
            self.place_smart(*id, Arc::clone(&content), &adjusted, Quality::Final)?;
        }
        Ok(instances.len())
    }

    /// « Remplacer le contenu » : le nouveau contenu prend l'identifiant de l'ancien, donc la place de celui-ci dans
    /// toutes les instances, avec leur transformation ; s'il n'a pas les mêmes dimensions, il est mis à l'échelle
    /// pour garder la taille affichée (comme Photoshop). Le nom des calques ne change pas.
    pub fn replace_smart_content(&mut self, id: LayerId, content: SmartContent) -> Result<usize, EngineError> {
        if self.layer(id)?.props.locks.all {
            return Err(EngineError::InvalidOperation("ce calque est verrouillé".into()));
        }
        let old = self.smart_content(id)?;
        let renamed = SmartContent::with_flattened(old.id, content.name, content.document, content.source, content.flattened);
        self.update_smart_content(Arc::new(renamed), SmartFit::DisplayedSize)
    }

    /// « Réinitialiser la transformation » : taille réelle (100 %), sans rotation ni déformation, centré là où se
    /// trouvait le centre de l'objet, sur des pixels entiers.
    pub fn reset_smart_transform(&mut self, id: LayerId) -> Result<(), EngineError> {
        self.ensure_movable(id)?;
        let content = self.smart_content(id)?;
        let full = self.layer(id)?.smart_transform().unwrap_or(Matrix::IDENTITY);
        let (w, h) = (f64::from(content.width()), f64::from(content.height()));
        let (cx, cy) = full.apply(w / 2.0, h / 2.0).unwrap_or((w / 2.0, h / 2.0));
        self.place_smart(id, content, &Matrix::translation((cx - w / 2.0).round(), (cy - h / 2.0).round()), Quality::Final)
    }

    /// « Nouvel objet dynamique par copie » : copie placée juste au-dessus, dont le contenu est indépendant (nouvel
    /// identifiant). Pixels et document restent partagés jusqu'à la première modification de l'un des deux.
    pub fn new_smart_by_copy(&mut self, id: LayerId) -> Result<LayerId, EngineError> {
        let content = self.smart_content(id)?;
        let (parent, index) = self.slot_above(id)?;
        let copy = self.clone_subtree(id, parent, true)?;
        self.children_mut(parent)?.insert(index, copy);
        if let LayerKind::Smart { content: own, .. } = &mut self.layer_mut(copy)?.kind {
            *own = Arc::new(content.copy_with_new_id());
        }
        Ok(copy)
    }

    /// « Incorporer le lien » : le contenu lié devient incorporé, pour toutes ses instances. Le rendu ne change pas.
    pub fn embed_smart_link(&mut self, id: LayerId) -> Result<usize, EngineError> {
        let content = self.smart_content(id)?;
        if !content.is_linked() {
            return Err(EngineError::InvalidOperation("cet objet dynamique est déjà incorporé".into()));
        }
        self.update_smart_content(Arc::new(content.embedded()), SmartFit::Centre)
    }

    /// Vérifie les fichiers liés des objets dynamiques du document : un fichier modifié (ou réapparu) est relu et
    /// toutes ses instances refaites ; un fichier introuvable marque l'objet « manquant », son dernier rendu restant
    /// affiché. Les contenus imbriqués dans d'autres contenus ne sont pas vérifiés.
    pub fn refresh_smart_links(&mut self) -> SmartLinksReport {
        let mut report = SmartLinksReport::default();
        let mut seen = HashSet::new();
        let linked: Vec<Arc<SmartContent>> = self.nodes.values().filter_map(|l| l.smart()).filter(|c| c.is_linked() && seen.insert(c.id)).cloned().collect();
        for content in linked {
            match content.reload() {
                None => {}
                Some(Ok(fresh)) => match self.update_smart_content(Arc::new(fresh), SmartFit::Centre) {
                    Ok(_) => report.updated.push(content.id),
                    Err(error) => report.failed.push(format!("{} : {error}", content.name)),
                },
                Some(Err(error)) => report.failed.push(format!("{} : {error}", content.name)),
            }
        }
        let mut seen = HashSet::new();
        report.missing = self.nodes.values().filter_map(|l| l.smart()).filter(|c| c.missing() && seen.insert(c.id)).map(|c| c.id).collect();
        report
    }

    // ------------------------------------------------------------- déplacement

    fn ensure_movable(&self, id: LayerId) -> Result<(), EngineError> {
        let locks = self.layer(id)?.props.locks;
        if locks.all || locks.position {
            return Err(EngineError::InvalidOperation("la position de ce calque est verrouillée".into()));
        }
        Ok(())
    }

    /// Outil Déplacement : décale des calques. Un groupe entraîne tout son contenu.
    pub fn translate_layers(&mut self, ids: &[LayerId], dx: i32, dy: i32) -> Result<(), EngineError> {
        let mut stack = self.top_level_selection(ids)?;
        let mut moved = Vec::new();
        while let Some(id) = stack.pop() {
            self.ensure_movable(id)?;
            stack.extend_from_slice(self.layer(id)?.children());
            moved.push(id);
        }
        for id in moved {
            let layer = self.layer_mut(id)?;
            layer.offset = (layer.offset.0 + dx, layer.offset.1 + dy);
        }
        Ok(())
    }

    /// Déplace un calque à masque en choisissant ce qui bouge : les pixels seuls (le masque reste en place dans le
    /// document), le masque seul, ou les deux ensemble. Sans masque, c'est un déplacement ordinaire. Un groupe bouge
    /// toujours d'un bloc : son contenu est décalé avec lui et son masque, qui vit dans son repère, suit sa position
    /// sans être recopié (une copie décalée du masque obligeait à recharger sa texture à chaque pas du geste).
    pub fn translate_parts(&mut self, id: LayerId, dx: i32, dy: i32, pixels: bool, mask: bool) -> Result<(), EngineError> {
        let layer = self.layer(id)?;
        if layer.mask.is_none() || (pixels && mask) || layer.is_group() {
            return self.translate_layers(&[id], dx, dy);
        }
        self.ensure_movable(id)?;
        let layer = self.layer_mut(id)?;
        let Some(current) = layer.mask.as_mut() else { return Ok(()) };
        if pixels {
            // Le calque se décale ; le masque, lui, est décalé d'autant en sens inverse dans le repère du calque :
            // il reste donc au même endroit du document.
            layer.offset = (layer.offset.0 + dx, layer.offset.1 + dy);
            current.pixels = Arc::new(current.pixels.translated(-dx, -dy));
        } else if mask {
            current.pixels = Arc::new(current.pixels.translated(dx, dy));
        }
        Ok(())
    }

    /// Outil Déplacement avec une sélection active : les pixels sélectionnés sont
    /// découpés, décalés, puis reposés par-dessus le reste du calque. La sélection suit.
    pub fn move_selected_pixels(&mut self, id: LayerId, dx: i32, dy: i32) -> Result<(), EngineError> {
        self.ensure_movable(id)?;
        if self.ensure_paintable(id)?.transparency {
            return Err(EngineError::InvalidOperation("les pixels transparents de ce calque sont verrouillés".into()));
        }
        let Some(selection) = self.selection.clone() else { return self.translate_layers(&[id], dx, dy) };
        let (ox, oy) = self.layer(id)?.offset;
        let source = Arc::clone(
            self.layer(id)?.pixels().ok_or_else(|| EngineError::InvalidOperation("un groupe n'a pas de pixels".into()))?,
        );
        let (cut, mut rest) = split_by_selection(&source, (ox, oy), &selection);
        paste_over(&mut rest, &cut.translated(dx, dy));
        *self.pixels_mut(id)? = rest;
        self.selection = Some(Arc::new(selection.translated(dx, dy)));
        Ok(())
    }

    /// Cadre de transformation : étendue exacte, en repère document, de la sélection
    /// s'il y en a une, sinon du contenu des calques donnés (groupes développés).
    pub fn content_bounds(&self, ids: &[LayerId]) -> Result<Option<(i32, i32, i32, i32)>, EngineError> {
        if let Some(selection) = &self.selection {
            let (w, h) = (self.width as i32, self.height as i32);
            let bounds = if selection.background() == [255] { Some((0, 0, w, h)) } else { selection.pixel_bounds(|px| px[0] > 0) };
            return Ok(bounds.map(|(x0, y0, x1, y1)| (x0.max(0), y0.max(0), x1.min(w), y1.min(h))));
        }
        let mut stack = self.top_level_selection(ids)?;
        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        while let Some(id) = stack.pop() {
            let layer = self.layer(id)?;
            stack.extend_from_slice(layer.children());
            let Some((x0, y0, x1, y1)) = layer.pixels().and_then(|p| p.pixel_bounds(|px| px[3] > 0)) else { continue };
            let (ox, oy) = layer.offset;
            let here = (x0 + ox, y0 + oy, x1 + ox, y1 + oy);
            bounds = Some(bounds.map_or(here, |b| (b.0.min(here.0), b.1.min(here.1), b.2.max(here.2), b.3.max(here.3))));
        }
        Ok(bounds)
    }

    /// Ctrl+T : applique `matrix` (repère document) aux calques donnés, masques de fusion
    /// compris. Avec une sélection active et un seul calque de pixels, seuls les pixels
    /// sélectionnés sont transformés, puis reposés sur le reste ; la sélection suit.
    pub fn transform_layers(&mut self, ids: &[LayerId], matrix: &Matrix, quality: Quality) -> Result<(), EngineError> {
        let mut stack = self.top_level_selection(ids)?;
        let mut targets = Vec::new();
        while let Some(id) = stack.pop() {
            self.ensure_movable(id)?;
            let layer = self.layer(id)?;
            stack.extend_from_slice(layer.children());
            if layer.is_group() {
                continue;
            }
            // Un calque de texte se transforme sans être pixellisé : seuls ses réglages changent.
            if layer.smart().is_some() {
                if self.selection.is_some() {
                    return Err(EngineError::InvalidOperation(
                        "désélectionnez (Ctrl+D) pour transformer un objet dynamique : une sélection ne peut pas le découper".into(),
                    ));
                }
            } else if layer.text().is_none() && layer.shape().is_none() {
                self.ensure_paintable(id)?;
            } else if self.selection.is_some() {
                return Err(EngineError::InvalidOperation(
                    "désélectionnez (Ctrl+D) pour transformer un calque de texte : une sélection ne peut pas le découper".into(),
                ));
            }
            targets.push(id);
        }

        if let (Some(selection), [id]) = (self.selection.clone(), targets.as_slice()) {
            let (offset, source) = {
                let layer = self.layer(*id)?;
                (layer.offset, Arc::clone(layer.pixels().ok_or_else(|| EngineError::UnknownLayer(*id))?))
            };
            let (cut, mut rest) = split_by_selection(&source, offset, &selection);
            // Le morceau découpé est en repère local : on le ramène au repère document avant de transformer.
            let placed = matrix.then_after(&Matrix::translation(f64::from(offset.0), f64::from(offset.1)));
            let back = Matrix::translation(-f64::from(offset.0), -f64::from(offset.1)).then_after(&placed);
            paste_over(&mut rest, &transform_store(&cut, &back, quality)?);
            *self.pixels_mut(*id)? = rest;
            self.selection = Some(Arc::new(transform_store(&selection, matrix, quality)?));
            return Ok(());
        }

        for id in targets {
            let (offset, pixels, mask) = {
                let layer = self.layer(id)?;
                (layer.offset, layer.pixels().cloned(), layer.mask.as_ref().map(|m| Arc::clone(&m.pixels)))
            };
            // Local -> document, puis la transformation : le résultat vit en repère document.
            let placed = matrix.then_after(&Matrix::translation(f64::from(offset.0), f64::from(offset.1)));
            if let LayerKind::Shape { shape, .. } = &self.layer(id)?.kind {
                // Une forme reste une forme, quoi qu'on lui fasse. Redimensionnée droite : sa taille et sa
                // position sont recalculées, arrondi et contour gardent leur épaisseur. Tournée, inclinée
                // ou mise en perspective : la transformation s'ajoute à la sienne et elle est redessinée
                // exactement — jamais pixellisée.
                let m = matrix.0;
                let plain = [m[1], m[3], m[6], m[7]].iter().all(|v| v.abs() < 1e-9) && m[0] > 0.0 && m[4] > 0.0 && (m[8] - 1.0).abs() < 1e-9;
                let next = if plain && shape.transform == Matrix::IDENTITY {
                    ShapeData { width: (f64::from(shape.width) * m[0]) as f32, height: (f64::from(shape.height) * m[4]) as f32, ..shape.clone() }
                } else {
                    ShapeData { transform: placed.then_after(&shape.transform), ..shape.clone() }
                };
                let (next, (dx, dy)) = next.normalized();
                let origin = if plain && shape.transform == Matrix::IDENTITY { placed.apply(0.0, 0.0).unwrap_or((0.0, 0.0)) } else { (0.0, 0.0) };
                let rendered = Arc::new(shape::render(&next)?);
                let layer = self.layer_mut(id)?;
                layer.offset = (origin.0.round() as i32 + dx, origin.1.round() as i32 + dy);
                layer.kind = LayerKind::Shape { shape: next, raster: rendered };
                continue;
            }
            if let LayerKind::Smart { content, transform, .. } = &self.layer(id)?.kind {
                // Un objet dynamique reste paramétrique : sa transformation s'ajoute à la sienne et il est rendu de
                // nouveau depuis l'aplat de son contenu, jamais depuis son rendu précédent.
                let content = Arc::clone(content);
                let full = placed.then_after(transform);
                self.place_smart(id, content, &full, quality)?;
                // Le masque (repère du calque) suit : ancien repère -> document -> transformation -> nouveau repère.
                if let Some(mask) = mask {
                    let (nx, ny) = self.layer(id)?.offset;
                    let back = Matrix::translation(-f64::from(nx), -f64::from(ny)).then_after(&placed);
                    let moved = transform_store(&mask, &back, quality)?;
                    if let Some(current) = &mut self.layer_mut(id)?.mask {
                        current.pixels = Arc::new(moved);
                    }
                }
                continue;
            }
            if let LayerKind::Text { text, transform, .. } = &self.layer(id)?.kind {
                let combined = placed.then_after(transform);
                let rendered = Arc::new(text::render_transformed(text, &combined, quality)?);
                let layer = self.layer_mut(id)?;
                layer.offset = (0, 0);
                if let LayerKind::Text { transform, raster, .. } = &mut layer.kind {
                    (*transform, *raster) = (combined, rendered);
                }
                continue;
            }
            let new_pixels = pixels.map(|p| transform_store(&p, &placed, quality)).transpose()?;
            let new_mask = mask.map(|m| transform_store(&m, &placed, quality)).transpose()?;
            let layer = self.layer_mut(id)?;
            layer.offset = (0, 0);
            if let (LayerKind::Pixel { pixels }, Some(new)) = (&mut layer.kind, new_pixels) {
                *pixels = Arc::new(new);
            }
            if let (Some(mask), Some(new)) = (&mut layer.mask, new_mask) {
                mask.pixels = Arc::new(new);
            }
        }
        Ok(())
    }

    /// Peut-on afficher le déplacement de ces calques sans recomposer ? Oui si l'image se
    /// reconstitue exactement en superposant « dessous », « déplacé » et « dessus » en mode
    /// Normal. Retourne alors les calques de pixels de chaque partie, sinon `None` (modes de
    /// fusion, écrêtage, masque ou opacité de groupe en jeu : le moteur recompose à chaque pas).
    pub fn move_preview_plan(&self, ids: &[LayerId]) -> Option<MovePlan> {
        if self.selection.is_some() {
            return None;
        }
        let mut stack = self.top_level_selection(ids).ok()?;
        let mut moving = HashSet::new();
        while let Some(id) = stack.pop() {
            self.ensure_movable(id).ok()?;
            let layer = self.layer(id).ok()?;
            stack.extend_from_slice(layer.children());
            if !layer.is_group() {
                moving.insert(id);
            }
        }
        // Calques de pixels du bas vers le haut ; les déplacés doivent y être contigus.
        let leaves: Vec<&Layer> = self.walk_top_down().into_iter().rev().map(|(_, l)| l).filter(|l| !l.is_group()).collect();
        let first = leaves.iter().position(|l| moving.contains(&l.id))?;
        let last = leaves.iter().rposition(|l| moving.contains(&l.id))?;
        if last - first + 1 != moving.len() {
            return None;
        }
        let plain_group = |group: &Layer| {
            let p = &group.props;
            let masked = group.mask.as_ref().is_some_and(|m| m.enabled);
            p.visible && p.opacity >= 1.0 && p.fill >= 1.0 && !p.clipped && !masked && matches!(p.blend, BlendMode::PassThrough | BlendMode::Normal)
        };
        for leaf in &leaves[first..] {
            // Un réglage dépend de ce qu'il y a dessous : impossible de le recomposer à part.
            // Un calque à effets déborde de ses pixels (ombre, halo) : pas d'aperçu décomposé non plus.
            if leaf.props.blend != BlendMode::Normal || leaf.props.clipped || leaf.adjustment().is_some() || crate::effects::any_enabled(&leaf.effects) {
                return None;
            }
            let mut parent = leaf.parent;
            while let Some(id) = parent {
                let group = self.layer(id).ok()?;
                if !plain_group(group) {
                    return None;
                }
                parent = group.parent;
            }
        }
        let ids_of = |range: &[&Layer]| range.iter().map(|l| l.id).collect();
        Some(MovePlan { below: ids_of(&leaves[..first]), moving: ids_of(&leaves[first..=last]), above: ids_of(&leaves[last + 1..]) })
    }

    /// Étendue (repère document) de chaque calque visible qui a du contenu, sauf `exclude` et leurs
    /// descendants : ce sont les « objets » auxquels s'aimante ce qu'on déplace.
    pub fn other_bounds(&self, exclude: &[LayerId]) -> Vec<(i32, i32, i32, i32)> {
        let mut skipped: HashSet<LayerId> = HashSet::new();
        let mut stack: Vec<LayerId> = exclude.to_vec();
        while let Some(id) = stack.pop() {
            if skipped.insert(id) {
                stack.extend_from_slice(self.layer(id).map(Layer::children).unwrap_or_default());
            }
        }
        self.walk_top_down()
            .into_iter()
            .map(|(_, layer)| layer)
            .filter(|layer| !skipped.contains(&layer.id) && layer.props.visible)
            .filter_map(|layer| {
                let (x0, y0, x1, y1) = layer.pixels()?.pixel_bounds(|px| px[3] > 0)?;
                let (ox, oy) = layer.offset;
                // Un calque qui couvre tout le document (un fond) n'apporte rien : ses bords sont ceux du document.
                let covers = x0 + ox <= 0 && y0 + oy <= 0 && x1 + ox >= self.width as i32 && y1 + oy >= self.height as i32;
                (!covers).then_some((x0 + ox, y0 + oy, x1 + ox, y1 + oy))
            })
            .collect()
    }

    /// Outil Texte : calque de texte visible le plus haut dont le rectangle contient le point,
    /// **même s'il est recouvert** par d'autres calques et quel que soit le calque actif. Survoler
    /// un texte existant doit toujours permettre d'y entrer, jamais d'en créer un second par-dessus.
    pub fn pick_text_layer(&self, x: i32, y: i32) -> Option<LayerId> {
        self.walk_top_down().into_iter().map(|(_, layer)| layer).find(|layer| {
            let (Some(_), Some(pixels)) = (layer.text(), layer.pixels()) else { return false };
            let (lx, ly) = (x - layer.offset.0, y - layer.offset.1);
            layer.props.visible && pixels.pixel_bounds(|px| px[3] > 0).is_some_and(|(x0, y0, x1, y1)| lx >= x0 && lx < x1 && ly >= y0 && ly < y1)
        }).map(|layer| layer.id)
    }

    /// Sélection automatique : calque de pixels visible le plus haut qui n'est pas
    /// transparent au point `(x, y)` du document.
    pub fn pick_layer(&self, x: i32, y: i32) -> Option<LayerId> {
        let visible = |layer: &Layer| {
            let mut current = Some(layer);
            while let Some(l) = current {
                if !l.props.visible {
                    return false;
                }
                current = l.parent.and_then(|p| self.nodes.get(&p)).map(Arc::as_ref);
            }
            true
        };
        // Un calque verrouillé (cadenas ou position), ou rangé dans un dossier verrouillé, ne se saisit pas à la souris :
        // le clic le traverse vers ce qui est dessous. Il se sélectionne toujours dans le panneau Calques.
        let fixed = |layer: &Layer| {
            let mut current = Some(layer);
            while let Some(l) = current {
                if l.props.locks.all || l.props.locks.position {
                    return true;
                }
                current = l.parent.and_then(|p| self.nodes.get(&p)).map(Arc::as_ref);
            }
            false
        };
        self.walk_top_down().into_iter().map(|(_, layer)| layer).find(|layer| {
            let Some(pixels) = layer.pixels() else { return false };
            let (lx, ly) = (x - layer.offset.0, y - layer.offset.1);
            if !visible(layer) || fixed(layer) {
                return false;
            }
            // Un texte se saisit par son rectangle : viser l'intérieur d'un « o » ou l'espace
            // entre deux lettres doit suffire, comme dans Photoshop.
            if layer.text().is_some() {
                return pixels.pixel_bounds(|px| px[3] > 0).is_some_and(|(x0, y0, x1, y1)| lx >= x0 && lx < x1 && ly >= y0 && ly < y1);
            }
            Sampler::new(pixels).get(lx, ly)[3] > 8
        })
        .map(|layer| self.clipping_base(layer.id))
        // Calque écrêté sur une base verrouillée : c'est la base qu'on saisirait, elle ne bouge pas.
        .filter(|id| self.nodes.get(id).is_none_or(|base| !fixed(base)))
    }

    /// Base d'un groupe d'écrêtage : un calque écrêté n'apparaît qu'à travers le calque du dessous,
    /// c'est donc celui-là qu'on saisit sur l'image. Sans quoi, cliquer sur un texte habillé d'un
    /// aplat écrêté sélectionnerait l'aplat, et une sélection multiple serait défaite au premier clic.
    fn clipping_base(&self, id: LayerId) -> LayerId {
        let Some(layer) = self.nodes.get(&id) else { return id };
        let Ok(siblings) = self.children_of(layer.parent) else { return id };
        let Some(mut index) = siblings.iter().position(|sibling| *sibling == id) else { return id };
        while index > 0 && self.nodes.get(&siblings[index]).is_some_and(|l| l.props.clipped) {
            index -= 1;
        }
        siblings[index]
    }

    // --------------------------------------------------------------- structure

    fn is_descendant_or_self(&self, candidate: LayerId, ancestor: LayerId) -> bool {
        let mut current = Some(candidate);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = self.nodes.get(&id).and_then(|l| l.parent);
        }
        false
    }

    /// Retire de la sélection les calques déjà contenus dans un groupe sélectionné,
    /// et trie le reste dans l'ordre du document (du bas vers le haut).
    fn top_level_selection(&self, ids: &[LayerId]) -> Result<Vec<LayerId>, EngineError> {
        for id in ids {
            self.layer(*id)?;
        }
        let set: HashSet<LayerId> = ids.iter().copied().collect();
        let mut ordered: Vec<LayerId> = self
            .walk_top_down()
            .into_iter()
            .rev()
            .map(|(_, layer)| layer.id)
            .filter(|id| set.contains(id))
            .filter(|id| {
                let parent = self.nodes.get(id).and_then(|l| l.parent);
                !parent.is_some_and(|p| set.iter().any(|s| self.is_descendant_or_self(p, *s)))
            })
            .collect();
        ordered.dedup();
        Ok(ordered)
    }

    /// Déplace des calques (glisser-déposer) vers `parent`, à partir de `index`
    /// compté dans la liste **avant** le retrait des calques déplacés.
    pub fn move_layers(&mut self, ids: &[LayerId], parent: Option<LayerId>, index: usize) -> Result<(), EngineError> {
        let moved = self.top_level_selection(ids)?;
        if let Some(target) = parent {
            if moved.iter().any(|id| self.is_descendant_or_self(target, *id)) {
                return Err(EngineError::InvalidOperation("un groupe ne peut pas être déplacé dans lui-même".into()));
            }
        }
        let siblings = self.children_of(parent)?;
        let index = index.min(siblings.len());
        // Les calques retirés avant le point d'insertion décalent celui-ci d'autant.
        let shift = siblings[..index].iter().filter(|id| moved.contains(id)).count();

        for id in &moved {
            let old_parent = self.layer(*id)?.parent;
            self.children_mut(old_parent)?.retain(|c| c != id);
            self.layer_mut(*id)?.parent = parent;
        }
        let target = self.children_mut(parent)?;
        let at = index - shift;
        target.splice(at..at, moved);
        Ok(())
    }

    /// Supprime des calques et leurs descendants. Un document garde toujours au moins un calque.
    pub fn remove_layers(&mut self, ids: &[LayerId]) -> Result<(), EngineError> {
        let removed = self.top_level_selection(ids)?;
        let mut doomed = HashSet::new();
        let mut stack = removed.clone();
        while let Some(id) = stack.pop() {
            doomed.insert(id);
            stack.extend_from_slice(self.layer(id)?.children());
        }
        if doomed.len() >= self.nodes.len() {
            return Err(EngineError::InvalidOperation("impossible de supprimer le dernier calque".into()));
        }
        // Le cadenas « Tout verrouiller » protège aussi de la suppression, comme dans Photoshop.
        if doomed.iter().any(|id| self.nodes.get(id).is_some_and(|l| l.props.locks.all)) {
            return Err(EngineError::InvalidOperation("ce calque est verrouillé : ouvrez le cadenas pour le supprimer".into()));
        }
        for id in &removed {
            let parent = self.layer(*id)?.parent;
            self.children_mut(parent)?.retain(|c| c != id);
        }
        self.nodes.retain(|id, _| !doomed.contains(id));
        Ok(())
    }

    /// Ctrl+G : regroupe les calques à l'emplacement du plus haut d'entre eux.
    pub fn group_layers(&mut self, ids: &[LayerId], name: impl Into<String>) -> Result<LayerId, EngineError> {
        let members = self.top_level_selection(ids)?;
        let topmost = *members.last().ok_or_else(|| EngineError::InvalidOperation("aucun calque à grouper".into()))?;
        let (parent, index) = self.slot_above(topmost)?;
        let group = self.add_group(name, parent, index)?;
        self.move_layers(&members, Some(group), 0)?;
        Ok(group)
    }

    /// Ctrl+J : copie placée juste au-dessus. Les pixels restent partagés jusqu'à la première retouche.
    pub fn duplicate_layer(&mut self, id: LayerId) -> Result<LayerId, EngineError> {
        let (parent, index) = self.slot_above(id)?;
        let copy = self.clone_subtree(id, parent, true)?;
        self.children_mut(parent)?.insert(index, copy);
        Ok(copy)
    }

    fn clone_subtree(&mut self, id: LayerId, parent: Option<LayerId>, rename: bool) -> Result<LayerId, EngineError> {
        let mut layer = self.layer(id)?.clone();
        let new_id = self.next_id;
        self.next_id += 1;
        layer.id = new_id;
        layer.parent = parent;
        if rename {
            layer.props.name.push_str(" copie");
        }
        if let LayerKind::Group { children, .. } = &mut layer.kind {
            let originals = std::mem::take(children);
            for child in originals {
                let cloned = self.clone_subtree(child, Some(new_id), false)?;
                children.push(cloned);
            }
        }
        self.nodes.insert(new_id, Arc::new(layer));
        Ok(new_id)
    }

    /// Vrai si les deux documents ont la même structure et les mêmes propriétés,
    /// pixels exclus. Sert à décider si tout l'affichage doit être recomposé.
    pub(crate) fn same_structure(&self, other: &Self) -> bool {
        self.same_structure_with(other, true)
    }

    /// Avec `positions` à faux, la position des calques est ignorée : sert à reconnaître un
    /// simple déplacement, qui ne périme que les tuiles survolées et non toute l'image.
    pub(crate) fn same_structure_with(&self, other: &Self, positions: bool) -> bool {
        // Un changement de taille (recadrage) périme tout, quelle que soit la suite.
        (self.width, self.height) == (other.width, other.height)
            && self.root == other.root
            && self.nodes.len() == other.nodes.len()
            && self.nodes.iter().all(|(id, a)| {
                other.nodes.get(id).is_some_and(|b| {
                    Arc::ptr_eq(a, b)
                        || (a.parent == b.parent
                            && (!positions || a.offset == b.offset)
                            && a.adjustment() == b.adjustment()
                            && a.effects == b.effects
                            && a.props == b.props
                            && a.children() == b.children()
                            && a.is_group() == b.is_group()
                            && a.mask.as_ref().map(|m| m.enabled) == b.mask.as_ref().map(|m| m.enabled))
                })
            })
    }

    pub(crate) fn nodes(&self) -> impl Iterator<Item = &Arc<Layer>> {
        self.nodes.values()
    }

    /// Stockage des pixels affichés d'un calque (pixels, ou rendu en cache d'un texte ou d'une forme), à remplacer
    /// tel quel : sert au disque de travail de l'historique (module `scratch`), qui retire puis remet des tuiles
    /// sans rien recalculer. `None` pour un groupe, un calque de réglage ou un calque inconnu.
    pub(crate) fn pixels_slot(&mut self, id: LayerId) -> Option<&mut Arc<TileStore>> {
        match &mut self.layer_mut(id).ok()?.kind {
            LayerKind::Pixel { pixels } => Some(pixels),
            LayerKind::Text { raster, .. } | LayerKind::Shape { raster, .. } | LayerKind::Smart { raster, .. } => Some(raster),
            LayerKind::Group { .. } | LayerKind::Adjustment { .. } => None,
        }
    }

    /// Stockage du masque de fusion d'un calque, même usage que [`Self::pixels_slot`].
    pub(crate) fn mask_slot(&mut self, id: LayerId) -> Option<&mut Arc<MaskStore>> {
        self.layer_mut(id).ok()?.mask.as_mut().map(|mask| &mut mask.pixels)
    }

    /// Stockage de la sélection, même usage que [`Self::pixels_slot`].
    pub(crate) fn selection_slot(&mut self) -> Option<&mut Arc<MaskStore>> {
        self.selection.as_mut()
    }
}

const TEXT_IS_NOT_PAINTABLE: &str = "calque de texte : pixellisez-le (clic droit > Pixelliser le calque) pour le peindre";
const SHAPE_IS_NOT_PAINTABLE: &str = "calque de forme : pixellisez-le (clic droit > Pixelliser le calque) pour le peindre";
const NOT_A_TEXT_LAYER: &str = "ce calque n'est pas un calque de texte";
const NOT_A_SMART_OBJECT: &str = "ce calque n'est pas un objet dynamique";

/// Adaptation de la transformation d'un objet dynamique dont le contenu change de dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartFit {
    /// Même échelle, même centre (contenu modifié).
    Centre,
    /// Même taille affichée, même centre (contenu remplacé).
    DisplayedSize,
}

/// Bilan d'une vérification des liens ([`Document::refresh_smart_links`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SmartLinksReport {
    /// Contenus relus (fichier modifié ou réapparu).
    pub updated: Vec<ContentId>,
    /// Contenus dont le fichier est introuvable.
    pub missing: Vec<ContentId>,
    /// Fichiers devenus illisibles (le contenu précédent reste en place), avec la raison.
    pub failed: Vec<String>,
}
const NO_CLOSED_PATH: &str = "le tracé doit compter au moins trois points";

/// Nom automatique d'un calque de texte : le début de son contenu.
fn text_layer_name(text: &TextData) -> String {
    let first_line = text.content.lines().next().unwrap_or_default().trim();
    if first_line.is_empty() {
        return "Texte".into();
    }
    let mut name: String = first_line.chars().take(30).collect();
    if first_line.chars().count() > 30 {
        name.push('…');
    }
    name
}

/// Partage les pixels d'un calque placé en `offset` en « découpé » (dans la sélection,
/// pondéré par elle) et « restant ». Les deux sont en repère local.
fn split_by_selection(source: &TileStore, offset: (i32, i32), selection: &MaskStore) -> (TileStore, TileStore) {
    let (mut cut, mut rest) = (TileStore::new(), TileStore::new());
    let mut mask = Sampler::new(selection);
    let size = TILE_SIZE as i32;
    for (coord, tile) in source.iter() {
        let (mut taken, mut kept) = (Tile::transparent(), (**tile).clone());
        let (lx, ly) = coord.origin();
        for (index, px) in kept.bytes_mut().chunks_exact_mut(4).enumerate() {
            let force = mask.get(lx + index as i32 % size + offset.0, ly + index as i32 / size + offset.1)[0];
            if force == 0 {
                continue;
            }
            for c in 0..4 {
                taken.bytes_mut()[index * 4 + c] = mul_div_255(px[c], force);
                px[c] = mul_div_255(px[c], 255 - force);
            }
        }
        cut.insert(coord, taken);
        rest.insert(coord, kept);
    }
    (cut, rest)
}

/// Pose `top` par-dessus `under` (fusion Normal, alpha prémultiplié).
fn paste_over(under: &mut TileStore, top: &TileStore) {
    for (coord, tile) in top.iter() {
        let target = under.get_mut(coord);
        for (dst, src) in target.bytes_mut().chunks_exact_mut(4).zip(tile.bytes().chunks_exact(4)) {
            let out = crate::pixel::over([src[0], src[1], src[2], src[3]], [dst[0], dst[1], dst[2], dst[3]]);
            dst.copy_from_slice(&out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        Document::with_background("x", 300, 300, 72.0, Background::White).expect("document")
    }

    fn add(doc: &mut Document, name: &str, parent: Option<LayerId>) -> LayerId {
        let index = doc.children_of(parent).expect("parent").len();
        doc.add_pixel_layer(name, TileStore::new(), parent, index).expect("calque")
    }

    fn names_top_down(doc: &Document) -> Vec<String> {
        doc.walk_top_down().iter().map(|(depth, l)| format!("{}{}", "  ".repeat(*depth), l.props.name)).collect()
    }

    #[test]
    fn rejects_empty_and_oversized_documents() {
        assert!(Document::new("x", 0, 10, 72.0).is_err());
        assert!(Document::new("x", 10, MAX_DIMENSION + 1, 72.0).is_err());
        assert!(Document::new("x", 8000, 8000, 300.0).is_ok());
    }

    #[test]
    fn new_groups_are_pass_through_and_layers_are_normal() {
        let mut d = doc();
        let g = d.add_group("G", None, 1).expect("groupe");
        let l = add(&mut d, "L", Some(g));
        assert_eq!(d.layer(g).expect("g").props.blend, BlendMode::PassThrough);
        assert_eq!(d.layer(l).expect("l").props.blend, BlendMode::Normal);
        assert!(d.edit_props(l, |p| p.blend = BlendMode::PassThrough).is_err(), "Transfert refusé hors groupe");
        assert_eq!(d.layer(l).expect("l").props.blend, BlendMode::Normal, "propriétés inchangées après refus");
    }

    #[test]
    fn group_layers_wraps_selection_at_topmost_position() {
        let mut d = doc();
        let a = add(&mut d, "A", None);
        let _b = add(&mut d, "B", None);
        let c = add(&mut d, "C", None);
        let _e = add(&mut d, "E", None);
        d.group_layers(&[c, a], "Groupe 1").expect("groupe");
        assert_eq!(names_top_down(&d), ["E", "Groupe 1", "  C", "  A", "B", "Arrière-plan"]);
        d.validate().expect("arbre cohérent");
    }

    #[test]
    fn move_layers_reorders_and_reparents() {
        let mut d = doc();
        let a = add(&mut d, "A", None);
        let b = add(&mut d, "B", None);
        let g = d.add_group("G", None, 3).expect("groupe");
        // Déplacement dans la même liste, vers le haut : l'index tient compte du retrait.
        d.move_layers(&[a], None, 3).expect("réordonner");
        assert_eq!(names_top_down(&d), ["G", "A", "B", "Arrière-plan"]);
        d.move_layers(&[a, b], Some(g), 0).expect("entrer dans le groupe");
        assert_eq!(names_top_down(&d), ["G", "  A", "  B", "Arrière-plan"]);
        assert!(d.move_layers(&[g], Some(g), 0).is_err(), "pas de cycle");
        d.validate().expect("arbre cohérent");
    }

    #[test]
    fn remove_layers_takes_descendants_but_never_the_last_layer() {
        let mut d = doc();
        let g = d.add_group("G", None, 1).expect("groupe");
        add(&mut d, "dans G", Some(g));
        d.remove_layers(&[g]).expect("suppression");
        assert_eq!(d.layer_count(), 1);
        let last = d.walk_top_down()[0].1.id;
        assert!(d.remove_layers(&[last]).is_err());
    }

    #[test]
    fn duplicate_shares_pixels_and_copies_subtrees() {
        let mut d = doc();
        let g = d.add_group("G", None, 1).expect("groupe");
        let inner = add(&mut d, "L", Some(g));
        d.pixels_mut(inner).expect("pixels").fill(300, 300, [9, 9, 9, 255]);
        let copy = d.duplicate_layer(g).expect("copie");
        assert_eq!(names_top_down(&d), ["G copie", "  L", "G", "  L", "Arrière-plan"]);
        let copied_inner = d.layer(copy).expect("copie").children()[0];
        let (a, b) = (d.layer(inner).expect("a"), d.layer(copied_inner).expect("b"));
        assert!(Arc::ptr_eq(a.pixels().expect("pixels"), b.pixels().expect("pixels")));
        d.validate().expect("arbre cohérent");
    }

    #[test]
    fn fill_respects_the_transparency_lock() {
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.pixels_mut(l).expect("pixels").get_mut(crate::tiles::TileCoord::new(0, 0)).set_pixel(1, 1, [0, 0, 100, 100]);
        d.edit_props(l, |p| p.locks.transparency = true).expect("verrou");
        d.fill_layer(l, [255, 0, 0]).expect("remplissage");
        let tile = d.layer(l).expect("l").pixels().expect("pixels").get(crate::tiles::TileCoord::new(0, 0)).expect("tuile").clone();
        assert_eq!(tile.pixel(1, 1), [100, 0, 0, 100], "couleur changée, alpha conservé");
        assert_eq!(tile.pixel(2, 2), [0; 4], "les pixels transparents le restent");
        d.edit_props(l, |p| p.locks.pixels = true).expect("verrou");
        assert!(d.fill_layer(l, [0, 0, 0]).is_err());
    }

    #[test]
    fn fill_and_clear_are_limited_to_the_selection() {
        use crate::selection::Shape;
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.translate_layers(&[l], -3, 4).expect("décalage"); // la sélection est en repère document, pas calque
        d.set_selection(Some(Shape::Rect { x: 10.0, y: 10.0, width: 20.0, height: 20.0 }.rasterize(300, 300)));
        d.fill_layer(l, [0, 255, 0]).expect("remplissage");
        let pixel = |d: &Document, x: i32, y: i32| {
            let layer = d.layer(l).expect("l");
            Sampler::new(layer.pixels().expect("pixels")).get(x - layer.offset.0, y - layer.offset.1)
        };
        assert_eq!((pixel(&d, 15, 15), pixel(&d, 5, 5), pixel(&d, 30, 30)), ([0, 255, 0, 255], [0; 4], [0; 4]));

        d.set_selection(Some(Shape::Rect { x: 10.0, y: 10.0, width: 5.0, height: 5.0 }.rasterize(300, 300)));
        d.clear_selected_pixels(l).expect("effacement");
        assert_eq!((pixel(&d, 12, 12), pixel(&d, 20, 20)), ([0; 4], [0, 255, 0, 255]));
    }

    #[test]
    fn moving_layers_keeps_off_canvas_content_and_drags_group_children() {
        let mut d = doc();
        let g = d.add_group("G", None, 1).expect("groupe");
        let inner = add(&mut d, "L", Some(g));
        d.pixels_mut(inner).expect("pixels").fill(300, 300, [7, 7, 7, 255]);
        let before = Arc::clone(d.layer(inner).expect("l").pixels().expect("pixels"));

        d.translate_layers(&[g], -500, 20).expect("déplacement du groupe");
        assert_eq!(d.layer(inner).expect("l").offset, (-500, 20), "l'enfant suit le groupe");
        assert!(Arc::ptr_eq(&before, d.layer(inner).expect("l").pixels().expect("pixels")), "aucun pixel touché");
        d.translate_layers(&[g], 500, -20).expect("retour");
        assert_eq!(d.pick_layer(10, 10), Some(inner), "contenu intact après un passage hors cadre");

        d.edit_props(inner, |p| p.locks.position = true).expect("verrou");
        assert!(d.translate_layers(&[g], 1, 1).is_err());
        assert_eq!(d.layer(g).expect("g").offset, (0, 0), "échec = rien n'a bougé");
    }

    #[test]
    fn moving_selected_pixels_cuts_pastes_and_moves_the_selection() {
        use crate::selection::Shape;
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.pixels_mut(l).expect("pixels").fill(300, 300, [0, 0, 255, 255]);
        d.set_selection(Some(Shape::Rect { x: 0.0, y: 0.0, width: 50.0, height: 50.0 }.rasterize(300, 300)));
        d.move_selected_pixels(l, 100, 0).expect("déplacement");

        let at = |d: &Document, x, y| Sampler::new(d.layer(l).expect("l").pixels().expect("pixels")).get(x, y);
        assert_eq!(at(&d, 10, 10), [0; 4], "trou laissé par la découpe");
        assert_eq!(at(&d, 110, 10), [0, 0, 255, 255], "pixels reposés");
        assert_eq!(at(&d, 60, 10), [0, 0, 255, 255], "le reste est intact");
        let mut selection = Sampler::new(d.selection().expect("sélection"));
        assert_eq!((selection.get(10, 10), selection.get(110, 10)), ([0], [255]), "la sélection a suivi");
    }

    #[test]
    fn content_bounds_are_exact_and_follow_the_layer() {
        let mut d = doc();
        let l = add(&mut d, "L", None);
        assert_eq!(d.content_bounds(&[l]).expect("bornes"), None, "calque vide");
        d.pixels_mut(l).expect("pixels").get_mut(TileCoord::new(0, 0)).set_pixel(10, 20, [1, 1, 1, 1]);
        d.pixels_mut(l).expect("pixels").get_mut(TileCoord::new(1, 0)).set_pixel(4, 30, [1, 1, 1, 255]);
        d.translate_layers(&[l], -5, 7).expect("déplacement");
        assert_eq!(d.content_bounds(&[l]).expect("bornes"), Some((5, 27, 256, 38)));
    }

    #[test]
    fn transform_scales_a_layer_about_a_point_and_resets_its_offset() {
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.pixels_mut(l).expect("pixels").fill(50, 50, [0, 255, 0, 255]);
        d.translate_layers(&[l], 100, 100).expect("position"); // carré vert en (100..150)²
        // Doublement autour du coin (100, 100) : attendu en (100..200)².
        let scale = Matrix([2.0, 0.0, -100.0, 0.0, 2.0, -100.0, 0.0, 0.0, 1.0]);
        d.transform_layers(&[l], &scale, Quality::Final).expect("transformation");
        assert_eq!(d.layer(l).expect("l").offset, (0, 0));
        let at = |x, y| Sampler::new(d.layer(l).expect("l").pixels().expect("pixels")).get(x, y);
        assert_eq!((at(103, 103), at(150, 150), at(196, 196), at(95, 150), at(205, 150)), ([0, 255, 0, 255], [0, 255, 0, 255], [0, 255, 0, 255], [0; 4], [0; 4]));
        // L'agrandissement bicubique adoucit le bord : l'étendue peut déborder de deux pixels, pas plus.
        let (x0, y0, x1, y1) = d.content_bounds(&[l]).expect("bornes").expect("contenu");
        assert!((98..=100).contains(&x0) && (98..=100).contains(&y0) && (200..=202).contains(&x1) && (200..=202).contains(&y1));
    }

    #[test]
    fn transform_with_a_selection_only_affects_selected_pixels() {
        use crate::selection::Shape;
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.pixels_mut(l).expect("pixels").fill(300, 300, [0, 0, 255, 255]);
        d.set_selection(Some(Shape::Rect { x: 0.0, y: 0.0, width: 40.0, height: 40.0 }.rasterize(300, 300)));
        d.transform_layers(&[l], &Matrix::translation(200.0, 0.0), Quality::Final).expect("transformation");
        let at = |d: &Document, x, y| Sampler::new(d.layer(l).expect("l").pixels().expect("pixels")).get(x, y);
        assert_eq!((at(&d, 10, 10), at(&d, 100, 10), at(&d, 210, 10)), ([0; 4], [0, 0, 255, 255], [0, 0, 255, 255]));
        assert_eq!(d.content_bounds(&[l]).expect("bornes"), Some((200, 0, 240, 40)), "la sélection a suivi");
    }

    #[test]
    fn move_preview_plan_accepts_plain_stacks_and_rejects_blending_tricks() {
        let mut d = doc();
        let a = add(&mut d, "A", None);
        let b = add(&mut d, "B", None);
        let c = add(&mut d, "C", None);
        let background = d.walk_top_down().last().expect("fond").1.id;
        assert_eq!(d.move_preview_plan(&[b]), Some(MovePlan { below: vec![background, a], moving: vec![b], above: vec![c] }));
        assert!(d.move_preview_plan(&[a, c]).is_none(), "calques déplacés non contigus");

        d.edit_props(c, |p| p.blend = BlendMode::Multiply).expect("mode");
        assert!(d.move_preview_plan(&[b]).is_none(), "un mode de fusion au-dessus dépend du fond");
        assert!(d.move_preview_plan(&[c]).is_none());
        assert!(d.move_preview_plan(&[a]).is_none());
        d.edit_props(c, |p| (p.blend, p.clipped) = (BlendMode::Normal, true)).expect("écrêtage");
        assert!(d.move_preview_plan(&[b]).is_none(), "écrêtage en jeu");
        d.edit_props(c, |p| p.clipped = false).expect("écrêtage");

        let g = d.group_layers(&[b, c], "G").expect("groupe");
        assert!(d.move_preview_plan(&[g]).is_some(), "groupe simple");
        d.edit_props(g, |p| p.opacity = 0.5).expect("opacité");
        assert!(d.move_preview_plan(&[g]).is_none(), "l'opacité d'un groupe ne se décompose pas");
    }

    #[test]
    fn text_layers_stay_editable_until_rasterized() {
        let mut d = doc();
        let text = TextData { content: "Bonjour".into(), size: 32.0, ..TextData::default() };
        let id = d.add_text_layer(text.clone(), (40, 50), None, 1).expect("calque de texte");
        assert_eq!(d.layer(id).expect("texte").props.name, "Bonjour");
        assert!(d.content_bounds(&[id]).expect("bornes").is_some_and(|b| b.0 >= 36 && b.1 >= 50), "rendu à l'ancre");
        assert!(d.pixels_mut(id).is_err() && d.fill_layer(id, [0; 3]).is_err(), "pas de peinture sur du texte");
        d.translate_layers(&[id], 10, 0).expect("un calque de texte se déplace");
        assert_eq!(d.pick_layer(60, 60), Some(id), "un texte se saisit par son rectangle, pas lettre par lettre");
        let cover = add(&mut d, "cache", None);
        d.pixels_mut(cover).expect("pixels").fill(300, 300, [9, 9, 9, 255]);
        assert_eq!(d.pick_layer(60, 60), Some(cover));
        assert_eq!(d.pick_text_layer(60, 60), Some(id), "un texte recouvert reste accessible à l'outil Texte");
        assert_eq!(d.pick_text_layer(250, 250), None);
        d.remove_layers(&[cover]).expect("retrait");

        // Ctrl+T : doublé autour de son ancre (50, 50), le texte reste un texte.
        let before = d.content_bounds(&[id]).expect("bornes").expect("contenu");
        let double = Matrix([2.0, 0.0, -50.0, 0.0, 2.0, -50.0, 0.0, 0.0, 1.0]);
        d.transform_layers(&[id], &double, Quality::Final).expect("transformation d'un texte");
        let after = d.content_bounds(&[id]).expect("bornes").expect("contenu");
        assert!(((after.2 - after.0) - 2 * (before.2 - before.0)).abs() <= 6, "deux fois plus large : {before:?} -> {after:?}");
        assert!(d.layer(id).expect("texte").text().is_some(), "toujours éditable");
        // Le curseur suit la transformation : à la fin du texte, il est deux fois plus loin de l'ancre.
        let ([top, bottom], _) = d.text_caret(id, 7, 7).expect("curseur");
        assert!((bottom.1 - top.1 - 2.0 * 32.0 * 1.2).abs() < 1.0, "hauteur de curseur doublée");
        assert_eq!(d.text_hit(id, top.0 + 1.0, (top.1 + bottom.1) / 2.0).expect("pointage"), 7);

        d.set_text(id, TextData { content: "Au revoir\nligne 2".into(), ..text.clone() }).expect("édition");
        assert_eq!(d.layer(id).expect("texte").props.name, "Au revoir", "le nom automatique suit le contenu");
        d.edit_props(id, |p| p.name = "Titre".into()).expect("renommer");
        d.set_text(id, TextData { content: "Autre".into(), ..text }).expect("édition");
        assert_eq!(d.layer(id).expect("texte").props.name, "Titre", "un nom choisi est conservé");

        d.rasterize_layer(id).expect("pixellisation");
        assert!(d.layer(id).expect("calque").text().is_none() && d.pixels_mut(id).is_ok());
        assert!(d.rasterize_layer(id).is_err());
    }

    #[test]
    fn a_path_becomes_a_selection_a_fill_or_a_brush_stroke() {
        use crate::path::Anchor;
        let corner = |x: f32, y: f32| Anchor { point: [x, y], handle_in: [x, y], handle_out: [x, y] };
        let mut d = doc();
        let l = add(&mut d, "L", None);
        assert!(d.path_to_selection(CombineMode::Replace).is_err(), "pas de tracé");
        d.paths = vec![SubPath { anchors: vec![corner(20.0, 20.0), corner(120.0, 20.0), corner(120.0, 120.0), corner(20.0, 120.0)], closed: true }];
        let at = |d: &Document, x, y| Sampler::new(d.layer(l).expect("l").pixels().expect("pixels")).get(x, y);

        let square = d.paths.clone();
        d.path_to_selection(CombineMode::Replace).expect("sélection");
        assert_eq!(d.content_bounds(&[l]).expect("bornes"), Some((20, 20, 120, 120)));
        assert!(d.paths.is_empty(), "le tracé disparaît une fois converti");
        d.set_selection(None);
        d.paths = square;

        d.fill_path(l, [0, 200, 0]).expect("fond");
        assert_eq!((at(&d, 70, 70), at(&d, 10, 10)), ([0, 200, 0, 255], [0; 4]));
        assert!(d.selection().is_none(), "le fond du tracé ne touche pas à la sélection");

        let brush = Brush { size: 6.0, hardness: 1.0, opacity: 1.0, color: [255, 0, 0], mode: crate::paint::BrushMode::Paint, pressure_size: false, pressure_opacity: false };
        d.stroke_path(l, brush).expect("contour");
        assert_eq!((at(&d, 70, 20), at(&d, 20, 70), at(&d, 70, 70)), ([255, 0, 0, 255], [255, 0, 0, 255], [0, 200, 0, 255]), "le contour suit les quatre côtés");
    }

    #[test]
    fn mask_button_with_a_selection_hides_everything_else_without_destroying_it() {
        use crate::render::TileRenderer;
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.pixels_mut(l).expect("pixels").fill(300, 300, [0, 0, 255, 255]);
        d.translate_layers(&[l], 10, 0).expect("position");
        d.set_selection(Some(Shape::Rect { x: 50.0, y: 50.0, width: 40.0, height: 40.0 }.rasterize(300, 300)));
        d.add_mask(l, true).expect("masque depuis la sélection");
        assert!(d.selection().is_none(), "la sélection est consommée");
        let tile = TileRenderer::new().tile(&d, 0, TileCoord::new(0, 0)).expect("tuile");
        assert_eq!((tile.pixel(60, 60), tile.pixel(40, 60), tile.pixel(95, 60)), ([0, 0, 255, 255], [255; 4], [255; 4]));
        d.set_mask_enabled(l, false).expect("désactiver");
        assert_eq!(TileRenderer::new().tile(&d, 0, TileCoord::new(0, 0)).expect("tuile").pixel(40, 60), [0, 0, 255, 255], "rien n'a été détruit");
    }

    #[test]
    fn copy_cut_and_paste_keep_pixels_in_place() {
        let mut d = doc();
        let l = add(&mut d, "L", None);
        assert!(d.copy_pixels(l).is_err(), "calque vide");
        d.pixels_mut(l).expect("pixels").fill(300, 300, [0, 0, 255, 255]);
        d.translate_layers(&[l], 5, 5).expect("position");
        d.set_selection(Some(Shape::Rect { x: 100.0, y: 100.0, width: 20.0, height: 20.0 }.rasterize(300, 300)));
        let clip = d.copy_pixels(l).expect("copie");
        d.clear_selected_pixels(l).expect("couper");
        let pasted = d.paste(&clip, "Calque 2", None, 2).expect("coller");
        assert_eq!(d.content_bounds(&[pasted]).expect("bornes"), Some((100, 100, 120, 120)), "collé au même endroit (la sélection borne le résultat)");
        d.set_selection(None);
        assert_eq!(d.content_bounds(&[pasted]).expect("bornes"), Some((100, 100, 120, 120)));
        let at = |id, x, y| Sampler::new(d.layer(id).expect("calque").pixels().expect("pixels")).get(x - 5, y - 5);
        assert_eq!((at(l, 110, 110), at(l, 90, 110), at(pasted, 110, 110)), ([0; 4], [0, 0, 255, 255], [0, 0, 255, 255]));
    }

    #[test]
    fn bucket_and_gradient_fills() {
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.pixels_mut(l).expect("pixels").fill(300, 300, [255, 255, 255, 255]);
        d.set_selection(Some(Shape::Rect { x: 100.0, y: 0.0, width: 100.0, height: 300.0 }.rasterize(300, 300)));
        d.fill_layer(l, [0, 0, 0]).expect("bande noire");
        d.set_selection(None);
        let at = |d: &Document, x, y| Sampler::new(d.layer(l).expect("l").pixels().expect("pixels")).get(x, y);

        // Pot de peinture à gauche : la bande noire fait barrage, la zone de droite reste blanche.
        d.bucket_fill(l, (10, 10), [255, 0, 0], 10, true).expect("pot de peinture");
        assert_eq!((at(&d, 50, 150), at(&d, 150, 150), at(&d, 250, 150)), ([255, 0, 0, 255], [0, 0, 0, 255], [255; 4]));
        d.bucket_fill(l, (250, 10), [0, 255, 0], 10, false).expect("non contigu");
        assert_eq!(at(&d, 250, 150), [0, 255, 0, 255]);

        // Dégradé horizontal noir -> blanc sur toute la largeur.
        let stop = |position, color| GradientStop { position, color };
        d.gradient_fill(l, (0.0, 0.0), (300.0, 0.0), &[stop(0.0, [0, 0, 0, 255]), stop(1.0, [255, 255, 255, 255])], false).expect("dégradé");
        let (left, middle, right) = (at(&d, 1, 100)[0], at(&d, 150, 100)[0], at(&d, 298, 100)[0]);
        assert!(left < 5 && (120..=136).contains(&middle) && right > 250, "{left} {middle} {right}");
        assert_eq!(at(&d, 150, 100)[3], 255);
        // Vers le transparent : ce qui est dessous transparaît de plus en plus.
        d.gradient_fill(l, (0.0, 0.0), (300.0, 0.0), &[stop(0.0, [255, 0, 0, 255]), stop(1.0, [255, 0, 0, 0])], false).expect("dégradé transparent");
        assert!(at(&d, 1, 100)[0] > 250 && at(&d, 298, 100)[1] > 240, "rouge à gauche, fond intact à droite");
        assert!(d.gradient_fill(l, (5.0, 5.0), (5.2, 5.0), &[stop(0.0, [0; 4]), stop(1.0, [0; 4])], false).is_err());
        // Trois arrêts, donnés dans le désordre : bleu au milieu.
        let three = [stop(1.0, [255, 0, 0, 255]), stop(0.5, [0, 0, 255, 255]), stop(0.0, [0, 255, 0, 255])];
        d.gradient_fill(l, (0.0, 0.0), (300.0, 0.0), &three, false).expect("trois arrêts");
        let middle = at(&d, 150, 100);
        assert!(middle[2] > 245 && middle[0] < 10 && middle[1] < 10, "{middle:?}");
        assert!(at(&d, 75, 100)[1] > 110 && at(&d, 75, 100)[2] > 110, "à mi-chemin du vert au bleu");
        assert!(d.gradient_fill(l, (0.0, 0.0), (300.0, 0.0), &three[..1], false).is_err());
    }

    #[test]
    fn adjustment_layers_correct_what_is_below_without_touching_pixels() {
        use crate::render::TileRenderer;
        let mut d = doc();
        let below = add(&mut d, "dessous", None);
        d.pixels_mut(below).expect("pixels").fill(300, 300, [200, 100, 0, 255]);
        let adjustment = d.add_adjustment_layer(Adjustment::Invert, None, 2, false).expect("réglage");
        let above = add(&mut d, "dessus", None);
        d.pixels_mut(above).expect("pixels").fill(50, 50, [10, 20, 30, 255]);
        let shown = |d: &Document, x, y| TileRenderer::new().tile(d, 0, TileCoord::new(0, 0)).expect("tuile").pixel(x, y);
        let shown_below = shown(&d, 100, 100);
        // Inverser (courbe mesurée, interpolée par le compositeur) : 200 → ~52, 100 → ~145, 0 → 255.
        assert!((i32::from(shown_below[0]) - 52).abs() <= 2 && (i32::from(shown_below[1]) - 145).abs() <= 2 && shown_below[2] == 255, "ce qui est dessous est inversé : {shown_below:?}");
        assert_eq!(shown(&d, 10, 10), [10, 20, 30, 255], "ce qui est dessus ne l'est pas");
        assert_eq!(Sampler::new(d.layer(below).expect("calque").pixels().expect("pixels")).get(100, 100), [200, 100, 0, 255], "pixels intacts");

        // Opacité à 50 % : à mi-chemin ; masqué : sans effet ; invisible : sans effet.
        d.edit_props(adjustment, |p| p.opacity = 0.5).expect("opacité");
        let half = shown(&d, 100, 100);
        // À mi-chemin entre (200, 100) et (52, 145) : (126, 123).
        assert!(half[0].abs_diff(126) <= 2 && half[1].abs_diff(123) <= 2, "{half:?}");
        d.edit_props(adjustment, |p| (p.opacity, p.visible) = (1.0, false)).expect("masquer");
        assert_eq!(shown(&d, 100, 100), [200, 100, 0, 255]);

        d.edit_props(adjustment, |p| p.visible = true).expect("afficher");
        assert!(d.set_adjustment(adjustment, Adjustment::Levels { black: 0.0, gamma: 1.0, white: 255.0 }).is_err(), "le type ne change pas");
        assert!(d.set_adjustment(below, Adjustment::Invert).is_err());
        assert!(d.move_preview_plan(&[below]).is_none(), "un réglage au-dessus interdit l'aperçu décomposé");

        // Réglage écrêté (le cas par défaut) : seul le calque juste dessous est corrigé.
        let mut d = doc();
        let bottom = add(&mut d, "bas", None);
        d.pixels_mut(bottom).expect("pixels").fill(300, 300, [200, 100, 0, 255]);
        let target = add(&mut d, "cible", None);
        d.pixels_mut(target).expect("pixels").fill(50, 50, [10, 20, 30, 255]);
        let clipped = d.add_adjustment_layer(Adjustment::Invert, None, 3, true).expect("réglage écrêté");
        assert!(d.layer(clipped).expect("réglage").props.clipped);
        // Inverser n'est pas le négatif strict : 10 → 245 (extrême quasi exact), 20 → 233 et 30 → 222 (légèrement plus sombres que 235 et 225).
        let table = crate::adjust::invert_table();
        assert_eq!(shown(&d, 10, 10), [table[10], table[20], table[30], 255], "le calque ciblé est inversé");
        assert!(shown(&d, 10, 10)[0] >= 244 && shown(&d, 10, 10)[2] < 225, "courbe mesurée, pas 255 - v : {:?}", shown(&d, 10, 10));
        assert_eq!(shown(&d, 100, 100), [200, 100, 0, 255], "le calque d'en dessous ne l'est pas");

        // Ctrl+I : inversion directe des pixels, limitée à la sélection.
        d.set_selection(Some(Shape::Rect { x: 0.0, y: 0.0, width: 20.0, height: 300.0 }.rasterize(300, 300)));
        d.invert_layer(bottom).expect("inversion");
        let at = |x| Sampler::new(d.layer(bottom).expect("bas").pixels().expect("pixels")).get(x, 200);
        assert_eq!((at(10), at(30)), ([table[200], table[100], table[0], 255], [200, 100, 0, 255]));
        assert_eq!(table[0], 255, "le noir devient blanc");
    }

    #[test]
    fn a_padlocked_layer_cannot_be_deleted() {
        let mut d = doc();
        let a = add(&mut d, "a", None);
        let b = add(&mut d, "b", None);
        d.edit_props(b, |p| p.locks.all = true).expect("cadenas");
        assert!(d.remove_layers(&[b]).is_err(), "le cadenas protège de la suppression");
        d.edit_props(b, |p| p.locks.all = false).expect("cadenas ouvert");
        d.remove_layers(&[b]).expect("déverrouillé : supprimé");
        assert!(d.layer(b).is_err() && d.layer(a).is_ok());
    }

    #[test]
    fn filters_respect_the_selection() {
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.pixels_mut(l).expect("pixels").fill(300, 300, [255, 255, 255, 255]);
        d.set_selection(Some(Shape::Rect { x: 0.0, y: 0.0, width: 150.0, height: 300.0 }.rasterize(300, 300)));
        d.fill_layer(l, [0, 0, 0]).expect("moitié noire");
        // Flou limité à une bande horizontale : la frontière noir/blanc n'est adoucie que dans cette bande.
        d.set_selection(Some(Shape::Rect { x: 0.0, y: 100.0, width: 300.0, height: 100.0 }.rasterize(300, 300)));
        d.apply_filter(l, &Filter::GaussianBlur { radius: 8.0 }).expect("flou");
        let at = |x, y| Sampler::new(d.layer(l).expect("l").pixels().expect("pixels")).get(x, y)[0];
        assert!((60..=200).contains(&at(150, 150)), "frontière adoucie dans la sélection : {}", at(150, 150));
        assert_eq!((at(149, 50), at(150, 50)), (0, 255), "frontière nette hors sélection");
    }

    #[test]
    fn crop_moves_everything_and_destroys_nothing() {
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.pixels_mut(l).expect("pixels").fill(300, 300, [0, 0, 255, 255]);
        d.set_selection(Some(Shape::Rect { x: 50.0, y: 50.0, width: 100.0, height: 100.0 }.rasterize(300, 300)));
        d.guides.push(Guide { orientation: Orientation::Vertical, position: 120.0 });
        d.guides.push(Guide { orientation: Orientation::Vertical, position: 20.0 });

        d.crop(100, 80, 150, 120).expect("recadrage");
        assert_eq!((d.width(), d.height()), (150, 120));
        assert_eq!(d.layer(l).expect("calque").offset, (-100, -80), "le calque est décalé, pas rogné");
        assert_eq!(Sampler::new(d.layer(l).expect("calque").pixels().expect("pixels")).get(5, 5), [0, 0, 255, 255], "pixels hors cadre conservés");
        let selection = d.selection().expect("sélection conservée");
        assert_eq!(selection.pixel_bounds(|v| v[0] > 0), Some((0, 0, 50, 70)), "sélection recalée et limitée au cadre");
        assert_eq!(d.guides.len(), 1, "le repère sorti du cadre disparaît");
        assert!((d.guides[0].position - 20.0).abs() < 1e-6);

        // Agrandir la zone de travail : origine négative.
        d.crop(-50, -50, 250, 220).expect("agrandissement");
        assert_eq!(d.layer(l).expect("calque").offset, (-50, -30));
        assert!(d.crop(0, 0, 0, 10).is_err());
    }

    #[test]
    fn shape_layers_stay_adjustable_and_resize_without_losing_their_nature() {
        use crate::shape::{ShapeData, ShapeKind};
        let mut d = doc();
        let data = ShapeData { kind: ShapeKind::Rect, width: 100.0, height: 60.0, radius: 10.0, fill: Some([0, 200, 0, 255]), stroke: Some([0, 0, 0, 255]), stroke_width: 4.0, sides: 3, transform: Default::default() };
        let id = d.add_shape_layer(data.clone(), (30, 40), None, 1).expect("forme");
        assert_eq!(d.content_bounds(&[id]).expect("bornes"), Some((30, 40, 130, 100)));
        assert!(d.pixels_mut(id).is_err() && d.fill_layer(id, [0; 3]).is_err(), "pas de peinture sur une forme");

        d.set_shape(id, ShapeData { radius: 30.0, ..data.clone() }).expect("arrondi");
        assert_eq!(d.layer(id).expect("forme").shape().expect("forme").radius, 30.0);

        // Ctrl+T, échelle x2 autour du coin haut gauche : la forme double, l'arrondi et le contour restent.
        let double = Matrix([2.0, 0.0, -30.0, 0.0, 2.0, -40.0, 0.0, 0.0, 1.0]);
        d.transform_layers(&[id], &double, Quality::Final).expect("redimensionnement");
        let shape = d.layer(id).expect("forme").shape().expect("toujours une forme").clone();
        assert_eq!((shape.width, shape.height, shape.radius, shape.stroke_width), (200.0, 120.0, 30.0, 4.0));
        assert_eq!(d.content_bounds(&[id]).expect("bornes"), Some((30, 40, 230, 160)));

        // Quart de tour autour du centre de la forme (130, 100) : elle reste une forme, redessinée droite
        // dans son nouveau repère, sa boîte 200 x 120 devenue 120 x 200 autour du même centre.
        let turn = Matrix([0.0, -1.0, 230.0, 1.0, 0.0, -30.0, 0.0, 0.0, 1.0]);
        d.transform_layers(&[id], &turn, Quality::Final).expect("rotation d'une forme");
        let layer = d.layer(id).expect("forme");
        let shape = layer.shape().expect("toujours une forme, pas des pixels").clone();
        assert_eq!((shape.width, shape.height, shape.radius, shape.stroke_width), (200.0, 120.0, 30.0, 4.0), "la forme garde ses réglages");
        assert!(shape.transform != Matrix::IDENTITY, "la rotation vit dans sa transformation");
        let (x0, y0, x1, y1) = d.content_bounds(&[id]).expect("bornes").expect("pixels");
        assert!((x0 - 70).abs() <= 2 && (y0 - 0).abs() <= 2 && (x1 - 190).abs() <= 2 && (y1 - 200).abs() <= 2, "boîte tournée : {:?}", (x0, y0, x1, y1));
        // Le panneau Propriétés modifie la couleur : la rotation est conservée.
        d.set_shape(id, ShapeData { fill: Some([255, 0, 0, 255]), ..shape.clone() }).expect("couleur");
        assert!(d.layer(id).expect("forme").shape().expect("forme").transform == shape.transform);
        assert_eq!(d.content_bounds(&[id]).expect("bornes").expect("pixels"), (x0, y0, x1, y1), "et rien ne bouge");
        // Redimensionnée par son cadre englobant (resté droit) : la boîte suit, en restant tournée.
        d.place_shape(id, 70, 0, 60.0, 100.0).expect("placement");
        let (px0, py0, px1, py1) = d.content_bounds(&[id]).expect("bornes").expect("pixels");
        assert!((px0 - 70).abs() <= 2 && py0.abs() <= 2 && (px1 - 130).abs() <= 3 && (py1 - 100).abs() <= 3, "{:?}", (px0, py0, px1, py1));
        d.rasterize_layer(id).expect("pixellisation");
        assert!(d.layer(id).expect("calque").shape().is_none() && d.pixels_mut(id).is_ok());
    }

    #[test]
    fn snapping_targets_are_the_other_visible_objects() {
        let mut d = doc();
        let a = add(&mut d, "A", None);
        d.pixels_mut(a).expect("pixels").fill(20, 10, [1, 1, 1, 255]);
        d.translate_layers(&[a], 30, 40).expect("position");
        let b = add(&mut d, "B", None);
        d.pixels_mut(b).expect("pixels").fill(5, 5, [1, 1, 1, 255]);
        assert_eq!(d.other_bounds(&[b]), vec![(30, 40, 50, 50)], "ni le calque déplacé, ni le fond qui couvre tout le document");
        d.edit_props(a, |p| p.visible = false).expect("masquer");
        assert!(d.other_bounds(&[b]).is_empty());
    }

    #[test]
    fn alt_clicking_an_eye_isolates_the_layer_then_restores_everything() {
        let mut d = doc();
        let group = d.add_group("G", None, 1).expect("groupe");
        let inside = add(&mut d, "dedans", Some(group));
        let other = add(&mut d, "autre", None);
        let visible = |d: &Document, id| d.layer(id).expect("calque").props.visible;
        d.solo(inside).expect("isoler");
        assert!(visible(&d, inside) && visible(&d, group), "le calque et son groupe restent visibles");
        assert!(!visible(&d, other) && !visible(&d, d.root[0]), "tout le reste est masqué");
        d.solo(inside).expect("rétablir");
        assert!(visible(&d, other) && visible(&d, d.root[0]), "second Alt + clic : tout revient");
    }

    #[test]
    fn a_layer_created_inside_a_clipping_group_joins_it() {
        let mut d = doc();
        let base = add(&mut d, "base", None);
        let clipped = add(&mut d, "écrêté", None);
        d.edit_props(clipped, |p| p.clipped = true).expect("écrêtage");
        // Nouveau calque juste au-dessus de la base, donc entre la base et le calque écrêté.
        let (parent, index) = d.slot_above(base).expect("emplacement");
        let between = d.add_pixel_layer("entre", TileStore::new(), parent, index).expect("calque");
        assert!(d.layer(between).expect("calque").props.clipped, "il rejoint le groupe d'écrêtage");
        assert!(d.layer(clipped).expect("calque").props.clipped, "et le calque du dessus reste écrêté sur la même base");
        // Au-dessus du groupe d'écrêtage, ou ailleurs : calque ordinaire.
        let (parent, index) = d.slot_above(clipped).expect("emplacement");
        let on_top = d.add_pixel_layer("dessus", TileStore::new(), parent, index).expect("calque");
        assert!(!d.layer(on_top).expect("calque").props.clipped);
    }

    #[test]
    fn select_all_takes_the_layer_content_not_the_whole_canvas() {
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.pixels_mut(l).expect("pixels").fill(40, 30, [255, 0, 0, 255]);
        d.translate_layers(&[l], 100, 50).expect("position");
        d.select_layer_content(l).expect("sélection");
        assert_eq!(d.selection().expect("sélection").pixel_bounds(|v| v[0] > 0), Some((100, 50, 140, 80)), "le contenu seul, à sa place");
        // Calque vide : tout le document.
        let empty = add(&mut d, "vide", None);
        d.select_layer_content(empty).expect("sélection");
        let all = d.selection().expect("sélection");
        assert_eq!((Sampler::new(all).get(0, 0)[0], Sampler::new(all).get(299, 299)[0]), (255, 255));
        // Un calque qui dépasse du document : la sélection s'arrête au bord.
        d.translate_layers(&[l], 180, 0).expect("position");
        d.select_layer_content(l).expect("sélection");
        assert_eq!(d.selection().expect("sélection").pixel_bounds(|v| v[0] > 0), Some((280, 50, 300, 80)));
    }

    #[test]
    fn groups_can_be_merged_copied_and_pasted_elsewhere() {
        let mut d = doc();
        let group = d.add_group("Logo", None, 1).expect("groupe");
        let red = add(&mut d, "rouge", Some(group));
        d.pixels_mut(red).expect("pixels").fill(100, 100, [255, 0, 0, 255]);
        let blue = add(&mut d, "bleu", Some(group));
        d.pixels_mut(blue).expect("pixels").fill(50, 50, [0, 0, 255, 255]);
        d.edit_props(group, |p| p.opacity = 0.5).expect("opacité");

        // Copier / coller le groupe dans un AUTRE document : toute la structure suit, avec de nouveaux identifiants.
        let clip = d.copy_layers(&[group]).expect("copie");
        let mut other = doc();
        let pasted = other.paste_layers(&clip, None, 1).expect("collage");
        assert_eq!(pasted.len(), 1);
        let copy = other.layer(pasted[0]).expect("groupe collé");
        assert_eq!((copy.props.name.as_str(), copy.children().len(), copy.props.opacity), ("Logo", 2, 0.5));
        assert!(copy.children().iter().all(|child| other.layer(*child).expect("enfant").parent == Some(pasted[0])));

        // Fusionner le groupe : un seul calque, qui garde le nom et l'opacité du groupe ; le rendu ne change pas.
        let before = crate::render::TileRenderer::new().tile(&d, 0, TileCoord::new(0, 0)).expect("tuile").pixel(20, 20);
        let merged = d.merge_group(group).expect("fusion");
        let layer = d.layer(merged).expect("calque fusionné");
        assert!(!layer.is_group() && layer.props.name == "Logo" && layer.props.opacity == 0.5 && layer.props.blend == BlendMode::Normal);
        assert!(d.layer(group).is_err() && d.layer(red).is_err(), "le groupe et son contenu ont disparu");
        assert_eq!(Sampler::new(layer.pixels().expect("pixels")).get(20, 20), [0, 0, 255, 255], "contenu rendu à pleine opacité : c'est le calque qui porte les 50 %");
        let after = crate::render::TileRenderer::new().tile(&d, 0, TileCoord::new(0, 0)).expect("tuile").pixel(20, 20);
        assert_eq!(before, after, "l'image est la même avant et après");
    }

    #[test]
    fn gradients_can_be_drawn_in_a_mask_and_masks_copied_between_layers() {
        let mut d = doc();
        let a = add(&mut d, "A", None);
        d.add_mask(a, true).expect("masque");
        let black_to_white = [GradientStop { position: 0.0, color: [0, 0, 0, 255] }, GradientStop { position: 1.0, color: [255, 255, 255, 255] }];
        d.gradient_mask(a, (0.0, 0.0), (300.0, 0.0), &black_to_white, false).expect("dégradé");
        let value = |d: &Document, id, x, y| Sampler::new(&d.layer(id).expect("calque").mask.as_ref().expect("masque").pixels).get(x, y)[0];
        assert!(value(&d, a, 2, 100) < 8 && value(&d, a, 297, 100) > 247, "noir à gauche, blanc à droite");
        assert!(value(&d, a, 150, 100).abs_diff(128) <= 3, "gris au milieu");

        // Copie vers un calque placé ailleurs : le masque reste au même endroit DU DOCUMENT.
        let b = add(&mut d, "B", None);
        d.translate_layers(&[b], 40, 0).expect("position");
        d.copy_mask(a, b, false).expect("copie");
        assert!(d.layer(a).expect("A").mask.is_some(), "copie : la source garde son masque");
        assert!(value(&d, b, 150 - 40, 100).abs_diff(128) <= 3, "aligné sur le document, pas sur le calque");
        let c = add(&mut d, "C", None);
        d.copy_mask(a, c, true).expect("déplacement");
        assert!(d.layer(a).expect("A").mask.is_none() && d.layer(c).expect("C").mask.is_some(), "déplacement : la source le perd");
    }

    #[test]
    fn a_simple_adjustment_layer_splits_the_image_for_its_instant_preview() {
        let mut d = doc();
        let under = add(&mut d, "dessous", None);
        let adjustment = d.add_adjustment_layer(crate::adjust::Adjustment::Invert, None, 2, false).expect("réglage");
        let over = add(&mut d, "dessus", None);
        let (below, target, above) = d.adjustment_preview_plan(adjustment).expect("réglage simple");
        assert!(target.is_empty(), "réglage libre : il corrige tout ce qui est dessous");
        assert!(below.contains(&under) && !below.contains(&over) && above.contains(&over) && !above.contains(&under));
        // Écrêté sur « dessous » : la cible est ce calque seul.
        d.edit_props(adjustment, |p| p.clipped = true).expect("écrêtage");
        let (below, target, above) = d.adjustment_preview_plan(adjustment).expect("réglage écrêté");
        assert_eq!(target, vec![under]);
        assert!(!below.contains(&under) && above.contains(&over));
        d.edit_props(adjustment, |p| p.clipped = false).expect("écrêtage");
        // Réglage masqué, écrêté ou dans un autre mode : pas d'aperçu instantané (le moteur rend comme avant).
        d.edit_props(adjustment, |p| p.blend = BlendMode::Multiply).expect("mode");
        assert!(d.adjustment_preview_plan(adjustment).is_none());
        assert!(d.adjustment_preview_plan(under).is_none(), "un calque de pixels n'est pas un réglage");
    }

    #[test]
    fn several_layers_merge_into_one_pixel_layer() {
        use crate::shape::{ShapeData, ShapeKind};
        let mut d = doc();
        let red = add(&mut d, "rouge", None);
        d.pixels_mut(red).expect("pixels").fill(100, 100, [255, 0, 0, 255]);
        let data = ShapeData { kind: ShapeKind::Rect, width: 40.0, height: 40.0, radius: 0.0, fill: Some([0, 0, 255, 255]), stroke: None, stroke_width: 0.0, sides: 3, transform: Default::default() };
        let shape = d.add_shape_layer(data, (80, 80), None, 2).expect("forme");
        let before = crate::render::TileRenderer::new().tile(&d, 0, TileCoord::new(0, 0)).expect("tuile");
        let merged = d.merge_layers(&[red, shape]).expect("fusion");
        let layer = d.layer(merged).expect("calque fusionné");
        assert!(layer.shape().is_none() && layer.text().is_none() && !layer.is_group(), "un calque de pixels");
        assert_eq!(layer.props.name, "Rectangle", "il prend le nom du calque le plus haut");
        assert!(d.layer(red).is_err() && d.layer(shape).is_err(), "les originaux ont disparu");
        let after = crate::render::TileRenderer::new().tile(&d, 0, TileCoord::new(0, 0)).expect("tuile");
        assert_eq!((before.pixel(50, 50), before.pixel(90, 90)), (after.pixel(50, 50), after.pixel(90, 90)), "même image");
        assert!(d.merge_layers(&[merged]).is_err(), "un seul calque : rien à fusionner");
    }

    #[test]
    fn a_resized_shape_is_redrawn_with_its_corners_intact() {
        use crate::shape::{ShapeData, ShapeKind};
        let mut d = doc();
        let data = ShapeData { kind: ShapeKind::Rect, width: 100.0, height: 60.0, radius: 20.0, fill: Some([0, 0, 0, 255]), stroke: None, stroke_width: 0.0, sides: 3, transform: Default::default() };
        let id = d.add_shape_layer(data, (10, 10), None, 1).expect("forme");
        // Allongée vers la droite seulement : 100 -> 200 px de large, même hauteur, même coin haut gauche.
        d.place_shape(id, 10, 10, 200.0, 60.0).expect("redimensionnement");
        let layer = d.layer(id).expect("calque");
        let shape = layer.shape().expect("toujours une forme");
        assert_eq!((shape.width, shape.height, shape.radius, layer.offset), (200.0, 60.0, 20.0, (10, 10)), "coins de 20 px conservés");
        // Le coin arrondi a la même courbure qu'avant, au bout droit de la forme.
        let pixels = layer.pixels().expect("pixels");
        assert_eq!(Sampler::new(pixels).get(199, 0)[3], 0, "coin haut droit toujours arrondi (vide)");
        assert_eq!(Sampler::new(pixels).get(150, 0)[3], 255, "bord haut plein");
    }

    #[test]
    fn pixels_and_mask_can_move_separately_or_together() {
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.pixels_mut(l).expect("pixels").fill(20, 20, [255, 0, 0, 255]);
        d.add_mask(l, false).expect("masque noir");
        d.layer_mut(l).expect("calque").mask.as_mut().map(|m| Arc::make_mut(&mut m.pixels).get_mut(TileCoord::new(0, 0)).set_pixel(5, 5, [255]));
        let mask_at = |d: &Document, x: i32, y: i32| {
            let layer = d.layer(l).expect("calque");
            Sampler::new(&layer.mask.as_ref().expect("masque").pixels).get(x - layer.offset.0, y - layer.offset.1)[0]
        };
        // Pixels seuls : le calque avance, le masque reste où il était DANS LE DOCUMENT.
        d.translate_parts(l, 30, 10, true, false).expect("pixels");
        assert_eq!(d.layer(l).expect("calque").offset, (30, 10));
        assert_eq!(mask_at(&d, 5, 5), 255, "le point blanc du masque n'a pas bougé");
        // Masque seul : le calque reste, le masque avance.
        d.translate_parts(l, 7, 0, false, true).expect("masque");
        assert_eq!((d.layer(l).expect("calque").offset, mask_at(&d, 12, 5), mask_at(&d, 5, 5)), ((30, 10), 255, 0));
        // Les deux : ensemble.
        d.translate_parts(l, 0, 4, true, true).expect("les deux");
        assert_eq!((d.layer(l).expect("calque").offset, mask_at(&d, 12, 9)), ((30, 14), 255));
    }

    #[test]
    fn moving_part_of_a_masked_group_moves_the_whole_group_without_copying_its_mask() {
        let mut d = doc();
        let g = d.add_group("G", None, 0).expect("groupe");
        let child = add(&mut d, "L", Some(g));
        d.pixels_mut(child).expect("pixels").fill(20, 20, [255, 0, 0, 255]);
        d.add_mask(g, false).expect("masque noir");
        d.layer_mut(g).expect("groupe").mask.as_mut().map(|m| Arc::make_mut(&mut m.pixels).get_mut(TileCoord::new(0, 0)).set_pixel(5, 5, [255]));
        let mask_arc = Arc::clone(&d.layer(g).expect("groupe").mask.as_ref().expect("masque").pixels);
        let mask_at = |d: &Document, x: i32, y: i32| {
            let layer = d.layer(g).expect("groupe");
            Sampler::new(&layer.mask.as_ref().expect("masque").pixels).get(x - layer.offset.0, y - layer.offset.1)[0]
        };
        // « Pixels seuls » sur un groupe : tout le groupe bouge, enfants compris, et le masque suit par la position.
        d.translate_parts(g, 30, 10, true, false).expect("groupe");
        assert_eq!(d.layer(g).expect("groupe").offset, (30, 10), "le groupe est décalé");
        assert_eq!(d.layer(child).expect("enfant").offset, (30, 10), "l'enfant suit le groupe");
        assert!(Arc::ptr_eq(&mask_arc, &d.layer(g).expect("groupe").mask.as_ref().expect("masque").pixels), "le masque n'est pas recopié");
        assert_eq!((mask_at(&d, 35, 15), mask_at(&d, 5, 5)), (255, 0), "le point blanc du masque a suivi le groupe");
        // « Masque seul » : pareil, le groupe bouge d'un bloc.
        d.translate_parts(g, 0, 4, false, true).expect("groupe");
        assert_eq!((d.layer(g).expect("groupe").offset, d.layer(child).expect("enfant").offset), ((30, 14), (30, 14)));
        assert!(Arc::ptr_eq(&mask_arc, &d.layer(g).expect("groupe").mask.as_ref().expect("masque").pixels), "toujours le même masque");
    }

    #[test]
    fn ctrl_i_on_a_mask_swaps_hidden_and_revealed() {
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.add_mask(l, true).expect("masque blanc");
        d.invert_mask(l).expect("inversion");
        let value = |d: &Document, x, y| Sampler::new(&d.layer(l).expect("calque").mask.as_ref().expect("masque").pixels).get(x, y)[0];
        assert_eq!((value(&d, 10, 10), value(&d, 5000, 5000)), (0, 0), "tout le masque passe au noir, fond compris");
        d.set_selection(Some(Shape::Rect { x: 0.0, y: 0.0, width: 50.0, height: 50.0 }.rasterize(300, 300)));
        d.invert_mask(l).expect("inversion");
        assert_eq!((value(&d, 10, 10), value(&d, 100, 100)), (255, 0), "avec une sélection : seulement dedans");
    }

    #[test]
    fn delete_key_on_a_mask_hides_the_selection() {
        let mut d = doc();
        let l = add(&mut d, "L", None);
        d.add_mask(l, true).expect("masque blanc");
        let value = |d: &Document, x, y| Sampler::new(&d.layer(l).expect("calque").mask.as_ref().expect("masque").pixels).get(x, y)[0];
        d.set_selection(Some(Shape::Rect { x: 0.0, y: 0.0, width: 50.0, height: 50.0 }.rasterize(300, 300)));
        d.clear_selected_mask(l).expect("effacement dans le masque");
        assert_eq!((value(&d, 10, 10), value(&d, 100, 100)), (0, 255), "la sélection est masquée, le reste intact");
        // Le calque n'a pas de masque : refus, et ses pixels ne sont pas touchés.
        let bare = add(&mut d, "Sans masque", None);
        assert!(d.clear_selected_mask(bare).is_err());
        d.set_selection(None);
        d.clear_selected_mask(l).expect("tout le masque");
        assert_eq!((value(&d, 10, 10), value(&d, 100, 100), value(&d, 5000, 5000)), (0, 0, 0), "sans sélection : tout le masque, fond compris");
    }

    #[test]
    fn filtering_a_shape_rasterizes_it_first() {
        use crate::shape::{ShapeData, ShapeKind};
        let mut d = doc();
        let data = ShapeData { kind: ShapeKind::Rect, width: 100.0, height: 100.0, radius: 0.0, fill: Some([255, 0, 0, 255]), stroke: None, stroke_width: 0.0, sides: 3, transform: Default::default() };
        let shape = d.add_shape_layer(data, (50, 50), None, 1).expect("forme");
        d.apply_filter(shape, &Filter::GaussianBlur { radius: 8.0 }).expect("le filtre s'applique à une forme");
        let layer = d.layer(shape).expect("calque");
        assert!(layer.shape().is_none(), "la forme est devenue un calque de pixels");
        let edge = Sampler::new(layer.pixels().expect("pixels")).get(-4, 50)[3];
        assert!(edge > 0 && edge < 255, "et son bord est bien flouté : {edge}");
    }

    #[test]
    fn picking_a_clipped_layer_selects_the_layer_it_is_clipped_on() {
        // Un texte, et par-dessus un aplat écrêté dessus : cliquer sur l'image doit désigner le texte.
        let mut d = doc();
        let base = add(&mut d, "texte", None);
        d.pixels_mut(base).expect("pixels").get_mut(TileCoord::new(0, 0)).set_pixel(5, 5, [255, 0, 0, 255]);
        let paint = add(&mut d, "aplat écrêté", None);
        d.pixels_mut(paint).expect("pixels").get_mut(TileCoord::new(0, 0)).set_pixel(5, 5, [0, 255, 0, 255]);
        d.edit_props(paint, |props| props.clipped = true).expect("écrêtage");
        assert_eq!(d.pick_layer(5, 5), Some(base), "c'est le calque du dessous qu'on saisit, pas l'aplat écrêté");
        // Détaché, l'aplat redevient un objet à part entière.
        d.edit_props(paint, |props| props.clipped = false).expect("détachement");
        assert_eq!(d.pick_layer(5, 5), Some(paint));
    }

    #[test]
    fn pick_layer_ignores_hidden_and_transparent_layers() {
        let mut d = doc();
        let top = add(&mut d, "haut", None);
        d.pixels_mut(top).expect("pixels").get_mut(TileCoord::new(0, 0)).set_pixel(5, 5, [255, 0, 0, 255]);
        let background = d.walk_top_down().last().expect("fond").1.id;
        assert_eq!(d.pick_layer(5, 5), Some(top));
        assert_eq!(d.pick_layer(6, 5), Some(background), "transparent : on traverse");
        d.edit_props(top, |p| p.visible = false).expect("masquer");
        assert_eq!(d.pick_layer(5, 5), Some(background));
        assert_eq!(d.pick_layer(-10, 5), None, "hors de tout contenu");
    }
    #[test]
    fn a_locked_layer_is_not_picked_by_the_mouse() {
        let mut d = doc();
        let top = add(&mut d, "haut", None);
        d.pixels_mut(top).expect("pixels").get_mut(TileCoord::new(0, 0)).set_pixel(5, 5, [255, 0, 0, 255]);
        let background = d.walk_top_down().last().expect("fond").1.id;
        assert_eq!(d.pick_layer(5, 5), Some(top));
        d.edit_props(top, |props| props.locks.all = true).expect("cadenas");
        assert_eq!(d.pick_layer(5, 5), Some(background), "le cadenas fait traverser le clic");
        d.edit_props(top, |props| (props.locks.all, props.locks.position) = (false, true)).expect("position");
        assert_eq!(d.pick_layer(5, 5), Some(background), "le verrou de position aussi");
    }



    #[test]
    fn clone_is_shallow_and_edits_do_not_leak() {
        let mut d = doc();
        let l = add(&mut d, "L", None);
        let snapshot = d.clone();
        d.edit_props(l, |p| p.opacity = 0.5).expect("édition");
        d.add_mask(l, true).expect("masque");
        assert_eq!(snapshot.layer(l).expect("l").props.opacity, 1.0);
        assert!(snapshot.layer(l).expect("l").mask.is_none());
        assert!(!d.same_structure(&snapshot));
        assert!(d.same_structure(&d.clone()));
    }
}
