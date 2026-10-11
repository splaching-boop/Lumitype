//! Session d'édition : document courant, historique et cache de rendu.
//!
//! # Annulation
//!
//! Chaque état de l'historique est un [`Document`] complet. Cela ne copie aucun
//! pixel : nœuds, stockages et tuiles sont partagés par `Arc`, et un nouvel état
//! ne possède en propre que ce qui a changé (quelques tuiles pour un trait, un
//! nœud pour une opacité). Avantages sur des commandes inverses écrites à la main :
//! tout est annulable par construction, y compris les futures opérations sur le
//! texte et les tracés, et annuler ne peut pas diverger de refaire.
//!
//! Les tuiles à rafraîchir se déduisent en comparant deux états par pointeurs.
//!
//! Au-delà du plafond de mémoire, les états anciens partent sur le disque de travail (module `scratch`) et
//! sont relus quand on y revient : l'état courant et ses voisins proches restent toujours en mémoire.

use crate::document::{Document, LayerId};
use crate::error::EngineError;
use crate::paint::{Brush, BrushDynamics, BrushKind, Stroke, StrokePoint};
use crate::render::TileRenderer;
use crate::tiles::TileCoord;
use std::sync::{Arc, Mutex};

/// Nombre d'états conservés, comme le réglage par défaut de Photoshop.
const MAX_STATES: usize = 50;
/// Nombre d'états d'historique gardés, réglable par les préférences (de 5 à 500).
static HISTORY_STATES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(MAX_STATES);

pub fn set_history_states(count: usize) {
    HISTORY_STATES.store(count.clamp(5, 500), std::sync::atomic::Ordering::Relaxed);
}

fn history_states() -> usize {
    HISTORY_STATES.load(std::sync::atomic::Ordering::Relaxed)
}

/// Ce que l'affichage doit recharger après une opération.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dirty {
    /// Structure ou propriétés modifiées : toutes les tuiles affichées sont périmées.
    pub all: bool,
    /// Sinon, tuiles de niveau 0 dont les pixels ont changé.
    pub tiles: Vec<TileCoord>,
    /// Calques dont les pixels ou le masque ont changé : leurs miniatures sont à refaire.
    pub layers: Vec<LayerId>,
    /// La sélection a changé : son contour est à recharger.
    pub selection: bool,
    /// Calques dont les PIXELS ou le masque ont changé (et non la seule position) : la composition sur carte
    /// graphique, qui garde chaque calque en texture, recharge ceux-là et rien d'autre.
    pub repainted: Vec<LayerId>,
    /// Zones repeintes de ces calques, dans LEUR repère et sans limite : `tiles` s'arrête aux bords du document,
    /// alors qu'un calque peut déborder (photo plus grande que le document, rotation). La carte graphique, qui garde
    /// les calques entiers, met à jour d'après ces zones-ci.
    pub areas: Vec<LayerArea>,
}

/// Zone repeinte d'un calque (ses pixels, ou son masque), en pixels de son repère : `[x, y, largeur, hauteur]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerArea {
    pub layer: LayerId,
    pub mask: bool,
    pub rect: [i32; 4],
}

/// Rectangle couvert par des tuiles du repère d'un calque, ou rien.
fn tiles_area(layer: LayerId, mask: bool, coords: &[TileCoord]) -> Option<LayerArea> {
    let size = crate::tiles::TILE_SIZE as i32;
    let (x0, y0, x1, y1) = coords.iter().fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |(a, b, c, d), t| (a.min(t.x), b.min(t.y), c.max(t.x), d.max(t.y)));
    (x0 <= x1).then(|| LayerArea { layer, mask, rect: [x0 * size, y0 * size, (x1 - x0 + 1) * size, (y1 - y0 + 1) * size] })
}

struct Entry {
    label: String,
    state: State,
    /// Deux opérations successives de même clé (glissement d'un curseur) ne font qu'un état.
    coalesce: Option<String>,
}

/// Un état d'historique est en mémoire, ou (état ancien, plafond de mémoire atteint) sur le disque de travail.
enum State {
    Resident(Document),
    Spilled(crate::scratch::SpilledState),
}

impl Entry {
    fn new(label: impl Into<String>, document: Document, coalesce: Option<String>) -> Self {
        Self { label: label.into(), state: State::Resident(document), coalesce }
    }

    /// Le document d'un état en mémoire. L'état courant et ses voisins proches le sont toujours ; un état plus
    /// lointain est relu par `Session::ensure_resident` avant tout usage.
    fn doc(&self) -> &Document {
        match &self.state {
            State::Resident(document) => document,
            State::Spilled(_) => unreachable!("état d'historique sur le disque utilisé sans avoir été relu"),
        }
    }

    fn resident(&self) -> Option<&Document> {
        match &self.state {
            State::Resident(document) => Some(document),
            State::Spilled(_) => None,
        }
    }
}

/// États voisins de l'état courant (de part et d'autre) qui ne partent jamais sur le disque : Ctrl+Z et Ctrl+Y
/// immédiats restent instantanés. Avec l'état courant, cela fait les `MIN_HISTORY` états que l'élagage garde.
const RECENT_STATES: usize = crate::memory::MIN_HISTORY - 1;

/// Issue d'un passage du disque de travail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spill {
    /// Sous le plafond, ou plus rien à écrire : l'historique est gardé tel quel.
    Done,
    /// Disque de travail désactivé, plein (quota) ou en erreur : il faut élaguer comme avant.
    Unavailable,
}

pub struct Session {
    entries: Vec<Entry>,
    cursor: usize,
    renderer: TileRenderer,
    /// Document en cours de modification par un trait non terminé.
    live: Option<(Document, Stroke)>,
    /// Augmente à chaque changement visible. Une tuile calculée à une révision
    /// antérieure à celle d'un `Dirty` reçu par l'affichage est périmée.
    revision: u64,
    /// Ce que chaque révision récente a changé (révision atteinte, tuiles de niveau 0 ou tout, sélection) :
    /// un affichage qui garde ses propres tuiles (fenêtre native) ne refait que celles-là. Borné : au-delà,
    /// `changes_since` répond « tout ».
    changes: std::collections::VecDeque<Change>,
    /// Fichier d'échange des états partis sur le disque, créé à la première écriture.
    scratch: Option<crate::scratch::SharedScratch>,
    /// Réglages du disque de travail propres à cette session (tests) ; `None` : ceux de l'application.
    scratch_config: Option<crate::scratch::Config>,
}

/// Ce qu'une révision a changé.
#[derive(Clone, Debug)]
struct Change {
    revision: u64,
    /// `None` : tout le document (structure, propriétés, effets).
    tiles: Option<Vec<TileCoord>>,
    selection: bool,
}

/// Réponse de [`Session::changes_since`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Tuiles de niveau 0 dont la composition a changé ; `None` : toutes (ou historique trop ancien).
    pub tiles: Option<Vec<TileCoord>>,
    pub selection: bool,
}

/// Nombre de révisions dont les changements sont gardés.
const CHANGES_KEPT: usize = 512;

impl Session {
    /// Historique complet à cet instant (états et état courant), pour la sauvegarde automatique. Les documents
    /// partagent leurs pixels avec la session (copie à l'écriture) : l'instantané est immédiat, et rien de ce qui
    /// sera fait ensuite ne s'y glisse. Un trait en cours n'en fait pas partie : il n'est pas encore une action.
    ///
    /// Les états partis sur le disque de travail n'en font pas partie : les relire tous ferait exploser la mémoire
    /// au moment même où elle manque. La récupération perd donc ces annulations anciennes, pas le travail.
    pub fn snapshot(&self) -> crate::io::History {
        let before = self.entries[..self.cursor].iter().filter(|e| e.resident().is_none()).count();
        let states = self.entries.iter().filter_map(|e| e.resident().map(|doc| (e.label.clone(), doc.clone()))).collect();
        crate::io::History { states, cursor: self.cursor - before }
    }

    /// Session reconstituée depuis un historique (récupération après une fermeture imprévue).
    pub fn from_history(history: crate::io::History) -> Self {
        let cursor = history.cursor.min(history.states.len().saturating_sub(1));
        Self {
            entries: history.states.into_iter().map(|(label, document)| Entry::new(label, document, None)).collect(),
            cursor,
            renderer: TileRenderer::new(),
            live: None,
            revision: 0,
            changes: std::collections::VecDeque::new(),
            scratch: None,
            scratch_config: None,
        }
    }

    pub fn new(document: Document, label: impl Into<String>) -> Self {
        Self {
            entries: vec![Entry::new(label, document, None)],
            cursor: 0,
            renderer: TileRenderer::new(),
            live: None,
            revision: 0,
            changes: std::collections::VecDeque::new(),
            scratch: None,
            scratch_config: None,
        }
    }

    /// Renomme le document dans tout l'historique, sans créer d'état : le nom suit le fichier
    /// (Enregistrer sous) et une annulation ne doit pas le faire revenir en arrière.
    pub fn rename(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        for entry in &mut self.entries {
            match &mut entry.state {
                State::Resident(document) => document.name = name.to_owned(),
                State::Spilled(spilled) => spilled.skeleton_mut().name = name.to_owned(),
            }
        }
        if let Some((document, _)) = &mut self.live {
            document.name = name.to_owned();
        }
    }

    pub fn document(&self) -> &Document {
        match &self.live {
            Some((document, _)) => document,
            None => self.entries[self.cursor].doc(),
        }
    }

    pub fn renderer(&self) -> &TileRenderer {
        &self.renderer
    }

    /// Le calque `id` réduit au niveau demandé puis filtré, replacé dans le repère du document à cette
    /// échelle. C'est la brique commune de l'aperçu d'un filtre : dans la fenêtre du filtre comme sur la
    /// zone de travail, on compose ensuite l'image en substituant ces pixels à ceux du calque.
    pub fn filtered_layer_at_level(&self, id: LayerId, filter: &crate::filters::Filter, level: u8) -> Result<crate::tiles::TileStore, EngineError> {
        use crate::transform::{Matrix, Quality, transform_store};
        let doc = self.document();
        let layer = doc.layer(id)?;
        if layer.smart().is_some() {
            return Err(EngineError::InvalidOperation(crate::smart::NOT_PAINTABLE.into()));
        }
        let pixels = layer.pixels().ok_or_else(|| EngineError::InvalidOperation("un groupe ne se filtre pas".into()))?;
        let Some((x0, y0, _, _)) = pixels.pixel_bounds(|px| px[3] > 0) else {
            return Err(EngineError::InvalidOperation("ce calque est vide".into()));
        };
        let k = 1.0 / f64::from(1u32 << level);
        let origin = pixels.translated(-x0, -y0);
        let small = if level > 0 { transform_store(&origin, &Matrix::scaling(k), Quality::Final)? } else { origin };
        let filtered = crate::filters::apply(&small, &filter.scaled(k as f32))?;
        let (ox, oy) = layer.offset;
        Ok(filtered.translated((f64::from(x0 + ox) * k).round() as i32, (f64::from(y0 + oy) * k).round() as i32))
    }

    /// Niveau de réduction (puissance de deux) tel que le plus grand côté du document tienne dans `max_side`.
    pub fn level_for_side(&self, max_side: u32) -> u8 {
        let doc = self.document();
        let longest = doc.width().max(doc.height());
        let mut level = 0u8;
        while (longest >> level) > max_side.max(64) && level < TileRenderer::max_level(doc) {
            level += 1;
        }
        level
    }

    /// Aperçu d'un filtre DANS l'image : le calque `id` filtré, recomposé avec tous les autres calques, à une
    /// échelle réduite (puissance de deux) telle que le plus grand côté tienne dans `max_side`. Retourne
    /// largeur, hauteur et pixels RGBA à alpha droit. Le document n'est pas modifié.
    pub fn filter_preview_in_context(&self, id: LayerId, filter: &crate::filters::Filter, max_side: u32) -> Result<(u32, u32, Vec<u8>), EngineError> {
        use crate::tiles::{TileCoord, TileStore, tiles_for};
        let doc = self.document();
        let level = self.level_for_side(max_side);
        let placed = self.filtered_layer_at_level(id, filter, level)?;

        let (w, h) = ((doc.width() >> level).max(1), (doc.height() >> level).max(1));
        let mut flat = TileStore::new();
        for ty in 0..tiles_for(h) as i32 {
            for tx in 0..tiles_for(w) as i32 {
                let coord = TileCoord::new(tx, ty);
                flat.restore(coord, self.renderer.tile_replacing(doc, level, coord, id, &placed));
            }
        }
        let mut rgba = flat.to_premultiplied(w, h);
        for px in rgba.chunks_exact_mut(4) {
            let straight = crate::pixel::unpremultiply([px[0], px[1], px[2], px[3]]);
            px.copy_from_slice(&straight);
        }
        Ok((w, h, rgba))
    }

    /// Image composée (tous les calques visibles), en tuiles, en repère document. S'appuie sur le
    /// cache du compositeur : après un premier affichage, la plupart des tuiles sont déjà prêtes.
    pub fn flat_store(&self) -> crate::tiles::TileStore {
        let doc = self.document();
        let mut flat = crate::tiles::TileStore::new();
        for ty in 0..crate::tiles::tiles_for(doc.height()) as i32 {
            for tx in 0..crate::tiles::tiles_for(doc.width()) as i32 {
                let coord = crate::tiles::TileCoord::new(tx, ty);
                flat.restore(coord, self.renderer.tile(doc, 0, coord));
            }
        }
        flat
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Ce qui a changé depuis `revision` (exclue) jusqu'à maintenant. `tiles: None` : tout est à refaire —
    /// changement de structure, ou révision trop ancienne pour être encore dans le journal.
    pub fn changes_since(&self, revision: u64) -> Changes {
        if revision >= self.revision {
            return Changes { tiles: Some(Vec::new()), selection: false };
        }
        // Le journal doit couvrir chaque révision de `revision + 1` à la courante, sans trou.
        let covered = self.changes.front().is_some_and(|first| first.revision <= revision + 1);
        if !covered {
            return Changes { tiles: None, selection: true };
        }
        let mut out = Changes { tiles: Some(Vec::new()), selection: false };
        for change in self.changes.iter().filter(|c| c.revision > revision) {
            out.selection |= change.selection;
            match (&mut out.tiles, &change.tiles) {
                (Some(all), Some(tiles)) => all.extend_from_slice(tiles),
                (tiles @ Some(_), None) => *tiles = None,
                (None, _) => {}
            }
        }
        if let Some(tiles) = &mut out.tiles {
            tiles.sort_unstable();
            tiles.dedup();
        }
        out
    }

    /// Nouvelle révision, et ce qu'elle a changé.
    fn bump(&mut self, dirty: &Dirty) {
        self.revision += 1;
        let tiles = if dirty.all { None } else { Some(dirty.tiles.clone()) };
        self.changes.push_back(Change { revision: self.revision, tiles, selection: dirty.selection });
        while self.changes.len() > CHANGES_KEPT {
            self.changes.pop_front();
        }
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    /// Libellés de l'historique et position courante.
    pub fn history(&self) -> (Vec<&str>, usize) {
        (self.entries.iter().map(|e| e.label.as_str()).collect(), self.cursor)
    }

    /// Applique une modification. En cas d'erreur, rien ne change.
    pub fn apply<T>(
        &mut self,
        label: impl Into<String>,
        coalesce: Option<String>,
        edit: impl FnOnce(&mut Document) -> Result<T, EngineError>,
    ) -> Result<(T, Dirty), EngineError> {
        self.apply_with(label, coalesce, false, edit)
    }

    /// Geste continu (glisser un calque, une sélection) : chaque appel de même clé
    /// repart de l'état **d'avant le geste** et remplace l'état produit par l'appel
    /// précédent. L'interface envoie donc le déplacement total depuis le début du
    /// geste, sans cumul d'erreurs, et le geste entier ne fait qu'un état d'historique.
    pub fn apply_gesture<T>(
        &mut self,
        label: impl Into<String>,
        gesture: String,
        edit: impl FnOnce(&mut Document) -> Result<T, EngineError>,
    ) -> Result<(T, Dirty), EngineError> {
        self.apply_with(label, Some(gesture), true, edit)
    }

    fn apply_with<T>(
        &mut self,
        label: impl Into<String>,
        coalesce: Option<String>,
        rebase: bool,
        edit: impl FnOnce(&mut Document) -> Result<T, EngineError>,
    ) -> Result<(T, Dirty), EngineError> {
        self.finish_stroke();
        let top = self.cursor + 1 == self.entries.len();
        let merge = top && coalesce.is_some() && self.entries[self.cursor].coalesce == coalesce;
        // Un état fusionnable a toujours un prédécesseur : il a été créé par un appel précédent.
        let base = if merge && rebase { self.cursor.saturating_sub(1) } else { self.cursor };
        self.ensure_resident(base)?;
        let mut next = self.entries[base].doc().clone();
        let value = edit(&mut next)?;
        let dirty = self.refresh(self.entries[self.cursor].doc(), &next);
        self.bump(&dirty);

        if merge {
            self.entries[self.cursor].state = State::Resident(next);
        } else {
            self.entries.truncate(self.cursor + 1); // l'avenir annulé est abandonné
            self.entries.push(Entry::new(label, next, coalesce));
            while self.entries.len() > history_states() {
                self.entries.remove(0);
            }
            self.cursor = self.entries.len() - 1;
        }
        self.enforce_memory_cap();
        Ok((value, dirty))
    }

    /// Fait respecter le plafond de mémoire (voir le module `memory`) après une édition, dans l'ordre :
    /// 1. les caches de rendu de TOUS les documents ouverts se serrent sur ce que le plafond leur laisse
    ///    (celui de ce document l'a déjà fait à chaque tuile ajoutée ; les autres n'ont pas vu la session
    ///    grossir, et le plafond vient peut-être de changer) ;
    /// 2. si documents et historique dépassent à eux seuls le plafond, caches mis à part, les états anciens de
    ///    CETTE session partent sur le disque de travail (module `scratch`), du plus éloigné de l'état courant au
    ///    plus proche, jusqu'à repasser dessous ; l'état courant et ses `RECENT_STATES` voisins restent ;
    /// 3. seulement si le disque de travail est désactivé, plein (quota) ou en erreur : les plus anciens états
    ///    sont abandonnés, un par un, jusqu'à repasser dessous — jamais moins de `MIN_HISTORY` états, jamais
    ///    l'état courant.
    ///
    /// C'est la session modifiée qui libère, et non la plus gourmande : c'est elle qui grossit, et chercher la
    /// plus gourmande demanderait de parcourir tout l'historique de chacune à chaque trait. Retourne le nombre
    /// d'états abandonnés.
    pub fn enforce_memory_cap(&mut self) -> usize {
        let cap = crate::memory::memory_cap();
        if cap == 0 {
            return 0;
        }
        crate::memory::trim_caches();
        let over = move |_: &Self| crate::tiles::resident_bytes().saturating_sub(crate::memory::cache_bytes_total()) > cap;
        self.fit_history(&over)
    }

    /// Étapes 2 et 3 de [`Self::enforce_memory_cap`], `over` disant si l'on est encore au-dessus du plafond.
    fn fit_history(&mut self, over: &dyn Fn(&Self) -> bool) -> usize {
        if !over(self) {
            return 0;
        }
        match self.spill_history(over) {
            Spill::Done => 0,
            Spill::Unavailable => self.prune_history_while(over),
        }
    }

    /// Abandonne les états les plus anciens tant que les tuiles vivantes hors caches de rendu (documents et
    /// historique) dépassent `cap` octets. Libérer un état libère aussitôt les tuiles qu'il était seul à
    /// retenir : le compteur global est relu après chaque retrait.
    #[cfg(test)]
    fn prune_history_to(&mut self, cap: u64) -> usize {
        self.prune_history_while(&|_: &Self| crate::tiles::resident_bytes().saturating_sub(crate::memory::cache_bytes_total()) > cap)
    }

    fn prune_history_while(&mut self, over: &dyn Fn(&Self) -> bool) -> usize {
        let mut removed = 0;
        while over(self) && self.entries.len() > crate::memory::MIN_HISTORY && self.cursor > 0 {
            self.entries.remove(0);
            self.cursor -= 1;
            removed += 1;
        }
        removed
    }

    /// Réglages du disque de travail de cette session.
    fn scratch_config(&self) -> crate::scratch::Config {
        self.scratch_config.clone().unwrap_or_else(crate::scratch::Config::current)
    }

    /// Envoie sur le disque de travail les états éloignés de l'état courant, du plus lointain au plus proche, tant
    /// que `over` le demande. Seules partent les tuiles qu'aucun état resté en mémoire ne retient : écrire les
    /// autres ne libérerait rien.
    fn spill_history(&mut self, over: &dyn Fn(&Self) -> bool) -> Spill {
        let config = self.scratch_config();
        if !config.enabled {
            return Spill::Unavailable;
        }
        let near = self.cursor.saturating_sub(RECENT_STATES)..=self.cursor + RECENT_STATES;
        let mut candidates: Vec<usize> = (0..self.entries.len()).filter(|i| !near.contains(i) && self.entries[*i].resident().is_some()).collect();
        if candidates.is_empty() {
            return Spill::Done;
        }
        candidates.sort_by_key(|i| std::cmp::Reverse(i.abs_diff(self.cursor)));
        // Tuiles retenues par ce qui reste en mémoire quoi qu'il arrive : états proches, trait en cours.
        let (mut keep, mut stores) = (std::collections::HashSet::new(), std::collections::HashSet::new());
        for (i, entry) in self.entries.iter().enumerate() {
            if let (true, Some(doc)) = (near.contains(&i), entry.resident()) {
                crate::scratch::collect_addresses(doc, &mut keep, &mut stores);
            }
        }
        if let Some((doc, _)) = &self.live {
            crate::scratch::collect_addresses(doc, &mut keep, &mut stores);
        }
        let file = match &self.scratch {
            Some(file) => Arc::clone(file),
            None => match crate::scratch::ScratchFile::create(&config.dir) {
                Ok(file) => Arc::clone(self.scratch.insert(Arc::new(Mutex::new(file)))),
                Err(_) => return Spill::Unavailable,
            },
        };
        if crate::scratch::tidy(&file).is_err() {
            return Spill::Unavailable;
        }
        for i in candidates {
            if !over(self) {
                break;
            }
            match crate::scratch::spill(&file, self.entries[i].doc(), &keep, config.quota) {
                Ok(Some(spilled)) => self.entries[i].state = State::Spilled(spilled),
                Ok(None) => {}
                Err(_) => return Spill::Unavailable,
            }
        }
        Spill::Done
    }

    /// Relit depuis le disque de travail l'état `index` s'il y est parti.
    fn ensure_resident(&mut self, index: usize) -> Result<(), EngineError> {
        let State::Spilled(spilled) = &self.entries[index].state else { return Ok(()) };
        let path = self.scratch.as_ref().map(|file| file.lock().unwrap_or_else(std::sync::PoisonError::into_inner).path().to_owned()).unwrap_or_default();
        let document = spilled.restore().map_err(EngineError::io(path))?;
        self.entries[index].state = State::Resident(document);
        Ok(())
    }

    /// Octets des tuiles des états en mémoire de cette session (une tuile partagée compte une fois).
    #[cfg(test)]
    fn resident_tile_bytes(&self) -> u64 {
        let mut seen = std::collections::HashSet::new();
        self.entries.iter().filter_map(Entry::resident).map(|doc| crate::memory::document_tile_bytes(doc, &mut seen)).sum()
    }

    /// Octets du fichier d'échange de cette session.
    pub fn scratch_file_bytes(&self) -> u64 {
        self.scratch.as_ref().and_then(|file| std::fs::metadata(file.lock().unwrap_or_else(std::sync::PoisonError::into_inner).path()).ok()).map_or(0, |m| m.len())
    }

    /// Échap pendant un geste (transformation libre) : l'état produit par le geste est
    /// retiré de l'historique, comme s'il n'avait jamais commencé. Sans effet sinon.
    pub fn cancel_gesture(&mut self, gesture: &str) -> Dirty {
        let top = self.cursor + 1 == self.entries.len();
        if !top || self.cursor == 0 || self.entries[self.cursor].coalesce.as_deref() != Some(gesture) {
            return Dirty::default();
        }
        if self.ensure_resident(self.cursor - 1).is_err() {
            return Dirty::default();
        }
        let dirty = self.refresh(self.entries[self.cursor].doc(), self.entries[self.cursor - 1].doc());
        self.entries.pop();
        self.cursor -= 1;
        self.bump(&dirty);
        dirty
    }

    /// Modification hors historique (repli d'un groupe dans le panneau).
    pub fn apply_silently(&mut self, edit: impl FnOnce(&mut Document) -> Result<(), EngineError>) -> Result<(), EngineError> {
        let mut next = self.entries[self.cursor].doc().clone();
        edit(&mut next)?;
        self.entries[self.cursor].state = State::Resident(next);
        Ok(())
    }

    pub fn undo(&mut self) -> Dirty {
        self.jump(self.cursor.saturating_sub(1))
    }

    pub fn redo(&mut self) -> Dirty {
        self.jump(self.cursor + 1)
    }

    /// Clic sur un état du panneau Historique.
    pub fn jump(&mut self, index: usize) -> Dirty {
        self.finish_stroke();
        let index = index.min(self.entries.len() - 1);
        if index == self.cursor {
            return Dirty::default();
        }
        // État parti sur le disque de travail : relu maintenant. Illisible (fichier effacé, disque retiré) : il
        // est retiré de l'historique, et l'on reste où l'on est.
        if self.ensure_resident(index).is_err() {
            self.entries.remove(index);
            if index < self.cursor {
                self.cursor -= 1;
            }
            return Dirty::default();
        }
        let dirty = self.refresh(self.entries[self.cursor].doc(), self.entries[index].doc());
        self.bump(&dirty);
        self.cursor = index;
        // Un curseur repris après une annulation ouvre un nouvel état.
        self.entries[index].coalesce = None;
        // Relire a pu faire repasser au-dessus du plafond : les états désormais éloignés repartent sur le disque.
        // Sans élagage ici : on ne supprime pas l'historique que l'on est en train de parcourir.
        let cap = crate::memory::memory_cap();
        if cap > 0 {
            let over = move |_: &Self| crate::tiles::resident_bytes().saturating_sub(crate::memory::cache_bytes_total()) > cap;
            if over(self) {
                self.spill_history(&over);
            }
        }
        dirty
    }

    // ------------------------------------------------------------------ pinceau

    pub fn begin_stroke(&mut self, layer: LayerId, on_mask: bool, brush: Brush, kind: BrushKind, dynamics: BrushDynamics) -> Result<(), EngineError> {
        self.begin_stroke_with(layer, on_mask, brush, kind, dynamics, false)
    }

    /// `deferred` : le trait est esquissé à l'écran par la carte graphique ; le calque n'est recomposé qu'à la fin.
    pub fn begin_stroke_with(&mut self, layer: LayerId, on_mask: bool, brush: Brush, kind: BrushKind, dynamics: BrushDynamics, deferred: bool) -> Result<(), EngineError> {
        self.finish_stroke();
        let document = self.entries[self.cursor].doc().clone();
        let stroke = Stroke::begin(&document, layer, on_mask, brush)?.with_kind(kind).with_dynamics(dynamics).with_deferred(deferred);
        self.live = Some((document, stroke));
        Ok(())
    }

    /// Masque rapide : le trait peint la sélection du document au lieu d'un calque.
    pub fn begin_selection_stroke(&mut self, brush: Brush) {
        self.finish_stroke();
        let document = self.entries[self.cursor].doc().clone();
        let stroke = Stroke::begin_on_selection(&document, brush);
        self.live = Some((document, stroke));
    }

    pub fn extend_stroke(&mut self, points: &[StrokePoint]) -> Result<Dirty, EngineError> {
        let (document, stroke) =
            self.live.as_mut().ok_or_else(|| EngineError::InvalidOperation("aucun trait en cours".into()))?;
        let local = stroke.extend(document, points)?;
        if stroke.on_selection() {
            // Seul le contour de sélection change : aucune tuile de l'image n'est à recomposer.
            let dirty = Dirty { selection: true, ..Dirty::default() };
            self.bump(&dirty);
            return Ok(dirty);
        }
        let tiles = self.renderer.invalidate_tiles(document, stroke.layer(), stroke.on_mask(), &local);
        let areas = tiles_area(stroke.layer(), stroke.on_mask(), &local).into_iter().collect();
        let dirty = Dirty { all: false, tiles, layers: vec![stroke.layer()], selection: false, repainted: vec![stroke.layer()], areas };
        self.bump(&dirty);
        Ok(dirty)
    }

    /// Termine le trait : il devient un état de l'historique. Sans effet s'il n'y en a pas.
    /// Retourne les tuiles du dernier bout de trait (le lissage garde un demi-segment d'avance).
    pub fn finish_stroke(&mut self) -> Dirty {
        let Some((mut document, mut stroke)) = self.live.take() else { return Dirty::default() };
        // Une erreur ici ne peut venir que d'un calque disparu entre-temps : le trait déjà peint est conservé.
        let local = stroke.finish(&mut document).unwrap_or_default();
        let dirty = if stroke.on_selection() {
            if document.selection().is_some_and(|s| s.tile_count() == 0) {
                document.set_selection(None); // tout a été gommé : plus de sélection du tout
            }
            Dirty { selection: true, ..Dirty::default() }
        } else {
            let tiles = self.renderer.invalidate_tiles(&document, stroke.layer(), stroke.on_mask(), &local);
            let areas = tiles_area(stroke.layer(), stroke.on_mask(), &local).into_iter().collect();
            Dirty { all: false, tiles, layers: vec![stroke.layer()], selection: false, repainted: vec![stroke.layer()], areas }
        };
        self.bump(&dirty);
        let label = if stroke.on_selection() { "Masque rapide" } else if stroke.on_mask() { "Pinceau (masque)" } else { "Pinceau" };
        self.entries.truncate(self.cursor + 1);
        self.entries.push(Entry::new(label, document, None));
        while self.entries.len() > history_states() {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len() - 1;
        self.enforce_memory_cap();
        dirty
    }

    // ---------------------------------------------------------------- interne

    /// Compare deux états, invalide le cache de rendu et décrit ce qui a changé.
    fn refresh(&self, old: &Document, new: &Document) -> Dirty {
        let mut dirty = Dirty { all: !old.same_structure(new), ..Dirty::default() };
        // Simple déplacement : seules les tuiles survolées avant et après sont à recomposer.
        let mut moved_tiles = Vec::new();
        if dirty.all && old.same_structure_with(new, false) {
            dirty.all = false;
            for layer in new.nodes() {
                let Ok(previous) = old.layer(layer.id) else { continue };
                if previous.offset == layer.offset {
                    continue;
                }
                // Un calque à effets déborde de ses pixels (ombre, halo) : on ne sait pas borner la zone touchée.
                if crate::effects::any_enabled(&layer.effects) {
                    dirty.all = true;
                }
                // Emprise des pixels, et celle du masque : il suit la position du calque, donc ce qu'il cache
                // change de place aussi. Un groupe n'a pas de pixels (ses enfants, décalés avec lui, ont les
                // leurs) : seule l'emprise de son masque compte.
                let pixels = layer.pixels().and_then(|p| p.bounds());
                let mask = layer.mask.as_ref().and_then(|m| m.pixels.bounds());
                for bounds in pixels.into_iter().chain(mask) {
                    for offset in [previous.offset, layer.offset] {
                        moved_tiles.extend(covered_tiles(new, bounds, offset));
                    }
                }
            }
            moved_tiles.sort_unstable();
            moved_tiles.dedup();
            self.renderer.invalidate_doc_tiles(new, &moved_tiles);
        }

        for layer in new.nodes() {
            let Ok(previous) = old.layer(layer.id) else {
                self.renderer.forget_layer(layer.id);
                continue;
            };
            if previous.effects != layer.effects {
                self.renderer.forget_layer(layer.id);
            }
            if previous.offset != layer.offset {
                dirty.layers.push(layer.id); // miniature cadrée sur le document : elle change avec la position
            }
            if let (Some(a), Some(b)) = (previous.pixels(), layer.pixels()) {
                if !std::sync::Arc::ptr_eq(a, b) {
                    // Pixels modifiés sous des effets : l'ombre ou le halo changent aussi, bien au-delà des tuiles peintes.
                    if crate::effects::any_enabled(&layer.effects) {
                        self.renderer.forget_layer(layer.id);
                        dirty.all = true;
                    }
                    let coords = a.changed_coords(b);
                    dirty.tiles.extend(self.renderer.invalidate_tiles(new, layer.id, false, &coords));
                    dirty.areas.extend(tiles_area(layer.id, false, &coords));
                    dirty.layers.push(layer.id);
                    dirty.repainted.push(layer.id);
                }
            }
            match (&previous.mask, &layer.mask) {
                (Some(a), Some(b)) if a.pixels.background() != b.pixels.background() => {
                    self.renderer.forget_layer(layer.id);
                    dirty.all = true;
                    dirty.layers.push(layer.id);
                    dirty.repainted.push(layer.id);
                }
                (Some(a), Some(b)) if !std::sync::Arc::ptr_eq(&a.pixels, &b.pixels) => {
                    let coords = a.pixels.changed_coords(&b.pixels);
                    dirty.tiles.extend(self.renderer.invalidate_tiles(new, layer.id, true, &coords));
                    dirty.areas.extend(tiles_area(layer.id, true, &coords));
                    dirty.layers.push(layer.id);
                    dirty.repainted.push(layer.id);
                }
                (None, None) | (Some(_), Some(_)) => {}
                _ => self.renderer.forget_layer(layer.id),
            }
        }
        dirty.selection = match (old.selection(), new.selection()) {
            (None, None) => false,
            (Some(a), Some(b)) => !std::sync::Arc::ptr_eq(a, b),
            _ => true,
        };
        if dirty.selection {
            self.renderer.forget_layer(crate::render::SELECTION_ID);
        }
        for layer in old.nodes().filter(|l| new.layer(l.id).is_err()) {
            self.renderer.forget_layer(layer.id);
        }

        dirty.tiles.extend(moved_tiles);
        if dirty.all {
            self.renderer.invalidate_composites();
            dirty.tiles.clear();
        } else {
            dirty.tiles.sort_unstable();
            dirty.tiles.dedup();
        }
        dirty.layers.sort_unstable();
        dirty.layers.dedup();
        dirty.repainted.sort_unstable();
        dirty.repainted.dedup();
        dirty
    }
}

/// Tuiles du document recouvertes par le rectangle local `bounds` d'un calque placé en `offset`.
fn covered_tiles(doc: &Document, (x0, y0, x1, y1): (i32, i32, i32, i32), offset: (i32, i32)) -> Vec<TileCoord> {
    let (first, _, _) = TileCoord::of_pixel((x0 + offset.0).max(0), (y0 + offset.1).max(0));
    let (last, _, _) = TileCoord::of_pixel((x1 + offset.0).min(doc.width() as i32) - 1, (y1 + offset.1).min(doc.height() as i32) - 1);
    (first.y..=last.y).flat_map(|ty| (first.x..=last.x).map(move |tx| TileCoord::new(tx, ty))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Background;
    use crate::paint::BrushMode;
    use crate::tiles::TileStore;

    const ORIGIN: TileCoord = TileCoord::new(0, 0);

    fn session() -> (Session, LayerId) {
        let document = Document::with_background("x", 512, 256, 72.0, Background::White).expect("document");
        let mut session = Session::new(document, "Nouveau");
        let (id, _) = session
            .apply("Nouveau calque", None, |doc| doc.add_pixel_layer("peinture", TileStore::new(), None, 1))
            .expect("calque");
        (session, id)
    }

    fn red() -> Brush {
        Brush { size: 10.0, hardness: 1.0, opacity: 1.0, color: [255, 0, 0], mode: BrushMode::Paint, pressure_size: false, pressure_opacity: false }
    }

    fn rendered(session: &Session, x: u32, y: u32) -> [u8; 4] {
        session.renderer().tile(session.document(), 0, ORIGIN).map_or([0; 4], |t| t.pixel(x, y))
    }

    #[test]
    fn changes_since_reports_touched_tiles_then_everything_when_the_log_is_too_old() {
        let (mut session, id) = session();
        let start = session.revision();
        // Un trait sur une tuile : seule celle-là change.
        session.begin_stroke(id, false, red(), BrushKind::Round, BrushDynamics::default()).expect("trait");
        session.extend_stroke(&[StrokePoint { x: 20.0, y: 20.0, pressure: 1.0 }]).expect("trait");
        session.finish_stroke();
        let changes = session.changes_since(start);
        assert_eq!(changes.tiles.as_deref(), Some(&[ORIGIN][..]), "le trait n'a touché que la première tuile");
        assert!(!changes.selection);
        assert_eq!(session.changes_since(session.revision()), Changes { tiles: Some(Vec::new()), selection: false }, "rien depuis maintenant");
        // Un changement de propriété : tout le document.
        let mid = session.revision();
        session.apply("Opacité", None, |doc| doc.edit_props(id, |p| p.opacity = 0.3)).expect("opacité");
        assert_eq!(session.changes_since(mid).tiles, None);
        assert_eq!(session.changes_since(start).tiles, None, "l'union contient un « tout »");
        // Journal dépassé : on ne sait plus, donc tout.
        for _ in 0..CHANGES_KEPT {
            session.apply("Opacité", None, |doc| doc.edit_props(id, |p| p.opacity = 0.5)).expect("opacité");
            session.apply("Opacité", None, |doc| doc.edit_props(id, |p| p.opacity = 0.6)).expect("opacité");
        }
        assert_eq!(session.changes_since(start), Changes { tiles: None, selection: true });
    }

    #[test]
    fn property_change_is_undoable_and_marks_everything_dirty() {
        let (mut session, id) = session();
        let (_, dirty) = session.apply("Opacité", None, |doc| doc.edit_props(id, |p| p.opacity = 0.3)).expect("opacité");
        assert!(dirty.all);
        assert_eq!(session.document().layer(id).expect("calque").props.opacity, 0.3);
        assert!(session.undo().all);
        assert_eq!(session.document().layer(id).expect("calque").props.opacity, 1.0);
        session.redo();
        assert_eq!(session.document().layer(id).expect("calque").props.opacity, 0.3);
    }

    #[test]
    fn slider_drags_coalesce_into_one_state() {
        let (mut session, id) = session();
        for value in [0.9, 0.7, 0.5] {
            let key = Some(format!("opacity:{id}"));
            session.apply("Opacité", key, |doc| doc.edit_props(id, |p| p.opacity = value)).expect("opacité");
        }
        assert_eq!(session.history().0, ["Nouveau", "Nouveau calque", "Opacité"]);
        session.undo();
        assert_eq!(session.document().layer(id).expect("calque").props.opacity, 1.0, "un seul Ctrl+Z suffit");
    }

    #[test]
    fn stroke_is_one_history_state_with_precise_dirty_tiles() {
        let (mut session, id) = session();
        session.begin_stroke(id, false, red(), BrushKind::Round, BrushDynamics::default()).expect("début");
        let point = |x| StrokePoint { x, y: 50.0, pressure: 1.0 };
        let dirty = session.extend_stroke(&[point(20.0), point(60.0)]).expect("trait");
        let area = vec![LayerArea { layer: id, mask: false, rect: [0, 0, 256, 256] }];
        assert_eq!(dirty, Dirty { all: false, tiles: vec![ORIGIN], layers: vec![id], selection: false, repainted: vec![id], areas: area.clone() });
        assert_eq!(rendered(&session, 40, 50), [255, 0, 0, 255], "visible pendant le trait");
        session.extend_stroke(&[point(100.0)]).expect("suite");
        session.finish_stroke();
        assert_eq!(session.history().0.last().copied(), Some("Pinceau"));

        let undone = session.undo();
        assert_eq!(undone, Dirty { all: false, tiles: vec![ORIGIN], layers: vec![id], selection: false, repainted: vec![id], areas: area });
        assert_eq!(rendered(&session, 40, 50), [255; 4], "le cache de rendu suit l'annulation");
        session.redo();
        assert_eq!(rendered(&session, 40, 50), [255, 0, 0, 255]);
    }

    #[test]
    fn a_low_memory_cap_prunes_the_oldest_states_but_never_below_the_minimum() {
        let (mut session, id) = session();
        // Vingt états, chacun avec une tuile en propre (un pixel modifié : copie à l'écriture).
        for i in 0..20 {
            session.apply("Pinceau", None, |doc| doc.pixels_mut(id).map(|p| p.get_mut(ORIGIN).set_pixel(i, 0, [255, 0, 0, 255]))).expect("pixel");
        }
        assert_eq!(session.history().0.len(), 22);
        let (_, cursor) = session.history();
        assert_eq!(cursor, 21);
        // Le plafond n'est pas touché (il est global au processus et d'autres tests tournent en même temps) :
        // on élague à un plafond donné. Sans pression, rien ne bouge.
        assert_eq!(session.prune_history_to(u64::MAX), 0);
        assert_eq!(session.history().0.len(), 22);
        // Plafond impossible à tenir : on descend au minimum, en gardant l'état courant et ce qu'il affiche.
        let removed = session.prune_history_to(1);
        assert_eq!(removed, 22 - crate::memory::MIN_HISTORY);
        let (labels, cursor) = session.history();
        assert_eq!(labels.len(), crate::memory::MIN_HISTORY);
        assert_eq!(cursor, crate::memory::MIN_HISTORY - 1, "le curseur suit les états retirés");
        assert_eq!(session.document().layer(id).expect("calque").pixels().expect("pixels").get(ORIGIN).expect("tuile").pixel(19, 0), [255, 0, 0, 255]);
        assert!(session.can_undo(), "les quatre états restants s'annulent encore");
        session.undo();
        assert_eq!(session.document().layer(id).expect("calque").pixels().expect("pixels").get(ORIGIN).expect("tuile").pixel(19, 0), [0; 4]);
    }

    /// Session dont chaque état peint sa propre bande sur quatre tuiles (copie à l'écriture : chaque état possède
    /// ses tuiles) ; le disque de travail est réglé pour CETTE session (dossier propre, quota donné). Retourne
    /// aussi les pixels attendus du calque à chaque état, pour comparer après relecture.
    fn spilling_session(enabled: bool, quota: u64) -> (Session, LayerId, Vec<Vec<u8>>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let (mut session, id) = session();
        session.scratch_config = Some(crate::scratch::Config { enabled, dir: dir.path().to_owned(), quota });
        let pixels = |s: &Session| s.document().layer(id).expect("calque").pixels().expect("pixels").to_premultiplied(512, 512);
        let mut expected = vec![pixels(&session), pixels(&session)]; // « Nouveau » (sans le calque : vide aussi) et « Nouveau calque »
        for i in 0..20u32 {
            session
                .apply("Pinceau", None, |doc| {
                    let store = doc.pixels_mut(id)?;
                    for (tx, ty) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        for x in 0..256 {
                            // Motif propre à l'état : une ligne différente à chaque fois, et des pixels peu compressibles.
                            let value = ((x * 7 + i * 13) % 251) as u8;
                            store.get_mut(TileCoord::new(tx, ty)).set_pixel(x, i, [value, 255 - value, (i * 9) as u8, 255]);
                        }
                    }
                    Ok(())
                })
                .expect("pinceau");
            expected.push(pixels(&session));
        }
        (session, id, expected, dir)
    }

    #[test]
    fn old_states_go_to_the_scratch_disk_and_come_back_pixel_exact() {
        let (mut session, id, expected, dir) = spilling_session(true, crate::scratch::DEFAULT_QUOTA);
        let all = session.resident_tile_bytes();
        // Plafond : un tiers de ce que l'historique occupe.
        let cap = all / 3;
        let over = move |s: &Session| s.resident_tile_bytes() > cap;
        assert_eq!(session.fit_history(&over), 0, "aucun état abandonné : le disque les garde");
        assert_eq!(session.history().0.len(), 22, "tout l'historique est encore là");
        assert!(session.resident_tile_bytes() <= cap, "mémoire résidente sous le plafond : {} > {cap}", session.resident_tile_bytes());
        let spilled = session.entries.iter().filter(|e| e.resident().is_none()).count();
        assert!(spilled > 0, "des états sont partis sur le disque");
        assert!(session.entries[session.cursor.saturating_sub(RECENT_STATES)..].iter().all(|e| e.resident().is_some()), "l'état courant et ses voisins restent en mémoire");
        let file = std::fs::read_dir(dir.path()).expect("dossier").flatten().map(|e| e.path()).next().expect("un fichier d'échange");
        assert!(session.scratch_file_bytes() > crate::scratch::MAGIC.len() as u64, "des tuiles y sont écrites");
        let header = std::fs::read(&file).expect("lecture");
        assert_eq!(&header[..8], crate::scratch::MAGIC);

        // Ctrl+Z jusqu'au plus ancien : chaque état revient à l'identique.
        let pixels = |s: &Session| s.document().layer(id).ok().and_then(|l| l.pixels().map(|p| p.to_premultiplied(512, 512)));
        for index in (1..22).rev() {
            assert_eq!(pixels(&session).as_deref(), Some(expected[index].as_slice()), "état {index}");
            session.undo();
        }
        assert!(session.document().layer(id).is_err(), "premier état : le calque n'existe pas encore");
        assert!(!session.can_undo());
        // Et l'on revient au dernier : rien n'a été perdu en route.
        session.jump(21);
        assert_eq!(pixels(&session).as_deref(), Some(expected[21].as_slice()));
        // Le rendu suit aussi.
        assert_eq!(session.renderer().tile(session.document(), 0, ORIGIN).expect("tuile").pixel(5, 19), {
            let px = &expected[21][(19 * 512 + 5) * 4..(19 * 512 + 5) * 4 + 4];
            [px[0], px[1], px[2], px[3]]
        });

        // Fermeture du document : le fichier d'échange disparaît.
        drop(session);
        assert!(!file.exists(), "fichier d'échange supprimé à la fermeture");
    }

    #[test]
    fn without_scratch_disk_old_states_are_pruned_as_before() {
        let (mut session, _, _, dir) = spilling_session(false, crate::scratch::DEFAULT_QUOTA);
        let removed = session.fit_history(&|_: &Session| true);
        assert_eq!(removed, 22 - crate::memory::MIN_HISTORY, "élagué jusqu'au minimum");
        assert_eq!(session.history().0.len(), crate::memory::MIN_HISTORY);
        assert_eq!(std::fs::read_dir(dir.path()).expect("dossier").count(), 0, "aucun fichier d'échange");
    }

    #[test]
    fn a_full_scratch_disk_falls_back_to_pruning() {
        // Quota de quelques octets : la première tuile ne passe pas.
        let (mut session, _, _, _dir) = spilling_session(true, 16);
        let removed = session.fit_history(&|_: &Session| true);
        assert_eq!(removed, 22 - crate::memory::MIN_HISTORY, "quota atteint : élagage comme avant");
        assert!(session.entries.iter().all(|e| e.resident().is_some()), "aucun état à moitié écrit");
    }

    #[test]
    fn an_unreadable_scratch_state_is_dropped_instead_of_breaking_undo() {
        let (mut session, _, _, dir) = spilling_session(true, crate::scratch::DEFAULT_QUOTA);
        let all = session.resident_tile_bytes();
        session.fit_history(&move |s: &Session| s.resident_tile_bytes() > all / 3);
        // Le fichier disparaît sous nos pieds (disque retiré…) : vidé.
        let file = std::fs::read_dir(dir.path()).expect("dossier").flatten().map(|e| e.path()).next().expect("fichier");
        std::fs::OpenOptions::new().write(true).open(&file).expect("ouverture").set_len(8).expect("troncature");
        let before = session.history().0.len();
        session.jump(2);
        assert_eq!(session.history().0.len(), before - 1, "l'état illisible est retiré");
        assert_eq!(session.history().1, 20, "et l'on reste sur l'état courant");
    }

    #[test]
    fn the_recovery_snapshot_skips_states_on_the_scratch_disk() {
        let (mut session, _, _, _dir) = spilling_session(true, crate::scratch::DEFAULT_QUOTA);
        let all = session.resident_tile_bytes();
        session.fit_history(&move |s: &Session| s.resident_tile_bytes() > all / 3);
        let spilled = session.entries.iter().filter(|e| e.resident().is_none()).count();
        let snapshot = session.snapshot();
        assert_eq!(snapshot.states.len(), 22 - spilled);
        assert_eq!(snapshot.cursor, snapshot.states.len() - 1, "le curseur désigne toujours l'état courant");
    }

    #[test]
    fn rename_survives_undo_and_adds_no_history_state() {
        let (mut session, id) = session();
        session.apply("Opacité", None, |doc| doc.edit_props(id, |p| p.opacity = 0.3)).expect("opacité");
        session.rename("Design NFS");
        assert_eq!(session.document().name, "Design NFS");
        assert_eq!(session.history().0, ["Nouveau", "Nouveau calque", "Opacité"]);
        session.undo();
        session.undo();
        assert_eq!(session.document().name, "Design NFS");
        session.rename("   ");
        assert_eq!(session.document().name, "Design NFS");
    }

    #[test]
    fn new_action_after_undo_discards_the_redo_branch() {
        let (mut session, id) = session();
        session.apply("A", None, |doc| doc.edit_props(id, |p| p.opacity = 0.5)).expect("A");
        session.undo();
        session.apply("B", None, |doc| doc.edit_props(id, |p| p.fill = 0.5)).expect("B");
        assert_eq!(session.history().0, ["Nouveau", "Nouveau calque", "B"]);
        assert_eq!(session.redo(), Dirty::default());
    }

    #[test]
    fn failed_edit_leaves_history_untouched() {
        let (mut session, _) = session();
        let before = session.history().0.len();
        assert!(session.apply("Erreur", None, |doc| doc.remove_mask(999)).is_err());
        assert_eq!(session.history().0.len(), before);
    }

    #[test]
    fn deleting_and_restoring_a_layer_keeps_rendering_correct() {
        let (mut session, id) = session();
        session.apply("Remplir", None, |doc| doc.pixels_mut(id).map(|p| p.fill(512, 256, [0, 0, 255, 255]))).expect("remplir");
        assert_eq!(rendered(&session, 5, 5), [0, 0, 255, 255]);
        session.apply("Supprimer", None, |doc| doc.remove_layers(&[id])).expect("supprimer");
        assert_eq!(rendered(&session, 5, 5), [255; 4]);
        session.undo();
        assert_eq!(rendered(&session, 5, 5), [0, 0, 255, 255]);
    }

    #[test]
    fn a_drag_gesture_is_one_state_and_never_accumulates() {
        let (mut session, id) = session();
        let states = session.history().0.len();
        // L'interface envoie le déplacement TOTAL depuis le début du geste.
        for total in [5, 12, 40] {
            session.apply_gesture("Déplacer", "geste-1".into(), |doc| doc.translate_layers(&[id], total, 0)).expect("glisser");
        }
        assert_eq!(session.document().layer(id).expect("calque").offset, (40, 0), "40, pas 5 + 12 + 40");
        assert_eq!(session.history().0.len(), states + 1);

        session.apply_gesture("Déplacer", "geste-2".into(), |doc| doc.translate_layers(&[id], 10, 0)).expect("second geste");
        assert_eq!(session.document().layer(id).expect("calque").offset, (50, 0), "un nouveau geste part de l'état courant");
        session.undo();
        assert_eq!(session.document().layer(id).expect("calque").offset, (40, 0));
        session.undo();
        assert_eq!(session.document().layer(id).expect("calque").offset, (0, 0));
    }

    #[test]
    fn moving_a_small_layer_only_dirties_the_tiles_it_leaves_and_enters() {
        let document = Document::with_background("x", 2048, 2048, 72.0, Background::White).expect("document");
        let mut session = Session::new(document, "Nouveau");
        let mut pixels = TileStore::new();
        pixels.fill(100, 100, [255, 0, 0, 255]);
        let (id, _) = session.apply("Calque", None, |doc| doc.add_pixel_layer("rouge", pixels, None, 1)).expect("calque");
        let tile = |session: &Session, x, y| session.renderer().tile(session.document(), 0, TileCoord::new(x, y)).expect("tuile").pixel(10, 10);
        assert_eq!(tile(&session, 0, 0), [255, 0, 0, 255]);

        let (_, dirty) = session.apply("Déplacer", None, |doc| doc.translate_layers(&[id], 1024, 512)).expect("déplacement");
        assert!(!dirty.all, "un déplacement ne périme pas toute l'image");
        assert_eq!(dirty.tiles, vec![TileCoord::new(0, 0), TileCoord::new(4, 2)]);
        assert_eq!((tile(&session, 0, 0), tile(&session, 4, 2)), ([255; 4], [255, 0, 0, 255]), "cache de rendu à jour");
    }

    #[test]
    fn moving_a_masked_group_shifts_children_and_mask_without_repainting_anything() {
        let document = Document::with_background("x", 2048, 2048, 72.0, Background::White).expect("document");
        let mut session = Session::new(document, "Nouveau");
        let mut pixels = TileStore::new();
        pixels.fill(100, 100, [255, 0, 0, 255]);
        let (group, _) = session.apply("Groupe", None, |doc| doc.add_group("G", None, 1)).expect("groupe");
        let (child, _) = session.apply("Calque", None, |doc| doc.add_pixel_layer("rouge", pixels, Some(group), 0)).expect("enfant");
        // Masque blanc avec un point noir sur la tuile (1, 1) : c'est elle qui fait l'emprise du masque.
        session
            .apply("Masque", None, |doc| {
                doc.add_mask(group, true)?;
                doc.mask_mut(group)?.get_mut(TileCoord::new(1, 1)).set_pixel(44, 44, [0]);
                Ok(())
            })
            .expect("masque");
        let child_pixels = std::sync::Arc::clone(session.document().layer(child).expect("enfant").pixels().expect("pixels"));
        let mask_pixels = std::sync::Arc::clone(&session.document().layer(group).expect("groupe").mask.as_ref().expect("masque").pixels);

        // L'outil Déplacement vise « les pixels » du groupe : tout le groupe bouge, masque compris, par la position.
        let (_, dirty) = session.apply("Déplacer", None, |doc| doc.translate_parts(group, 1024, 512, true, false)).expect("déplacement");
        let doc = session.document();
        assert_eq!((doc.layer(group).expect("groupe").offset, doc.layer(child).expect("enfant").offset), ((1024, 512), (1024, 512)));
        assert!(std::sync::Arc::ptr_eq(&child_pixels, doc.layer(child).expect("enfant").pixels().expect("pixels")), "pixels de l'enfant partagés");
        assert!(std::sync::Arc::ptr_eq(&mask_pixels, &doc.layer(group).expect("groupe").mask.as_ref().expect("masque").pixels), "masque non recopié");
        assert!(!dirty.all, "un déplacement ne périme pas toute l'image");
        assert!(dirty.repainted.is_empty(), "rien n'a été repeint : les textures restent valables");
        assert!(dirty.areas.is_empty());
        // Tuiles sales : l'emprise des pixels de l'enfant et celle du masque du groupe, avant et après.
        let expected = [TileCoord::new(0, 0), TileCoord::new(1, 1), TileCoord::new(4, 2), TileCoord::new(5, 3)];
        assert_eq!(dirty.tiles, expected);
        assert_eq!(dirty.layers, vec![group, child], "miniatures cadrées sur le document : à refaire, mais sans repeinte");
    }

    #[test]
    fn the_whole_history_survives_a_recovery_round_trip() {
        let mut document = Document::with_background("x", 300, 200, 72.0, Background::White).expect("document");
        let layer = document.add_pixel_layer("L", crate::tiles::TileStore::new(), None, 1).expect("calque");
        let mut session = Session::new(document, "Nouveau");
        session.apply("Remplir", None, |doc| {
            doc.pixels_mut(layer)?.fill(100, 100, [255, 0, 0, 255]);
            Ok(())
        }).expect("action");
        session.apply("Déplacer", None, |doc| doc.translate_layers(&[layer], 50, 0)).expect("action");
        let _ = session.undo();
        // Instantané, puis une action faite PENDANT la sauvegarde : elle ne doit pas s'y trouver.
        let snapshot = session.snapshot();
        session.apply("Déplacer encore", None, |doc| doc.translate_layers(&[layer], 0, 30)).expect("action");
        let dir = std::env::temp_dir().join(format!("grafism-recuperation-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dossier");
        let path = dir.join("essai.graf");
        crate::io::save_recovery(&snapshot, None, &path, &|_| {}).expect("écriture");
        let (history, project) = crate::io::load_recovery(&path).expect("lecture");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(project.is_none());
        let labels: Vec<&str> = history.states.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["Nouveau", "Remplir", "Déplacer"], "tous les états, sans l'action faite pendant la sauvegarde");
        assert_eq!(history.cursor, 1, "l'état courant est celui d'avant « Déplacer » (annulé)");
        let restored = Session::from_history(history);
        assert_eq!(restored.document().layer(layer).expect("calque").offset, (0, 0));
        assert_eq!(restored.document().layer(layer).expect("calque").pixels().expect("pixels").pixel_bounds(|p| p[3] > 0), Some((0, 0, 100, 100)));
    }

    #[test]
    fn a_filter_can_be_previewed_with_all_the_other_layers() {
        use crate::filters::Filter;
        // Fond blanc, calque rouge de 400 px au milieu, et un calque vert opaque PAR-DESSUS dans un coin.
        let mut document = Document::with_background("x", 1024, 1024, 72.0, Background::White).expect("document");
        let red = document.add_pixel_layer("rouge", crate::tiles::TileStore::new(), None, 1).expect("calque");
        document.pixels_mut(red).expect("pixels").fill(400, 400, [255, 0, 0, 255]);
        document.translate_layers(&[red], 300, 300).expect("position");
        let green = document.add_pixel_layer("vert", crate::tiles::TileStore::new(), None, 2).expect("calque");
        document.pixels_mut(green).expect("pixels").fill(200, 200, [0, 255, 0, 255]);
        document.translate_layers(&[green], 300, 300).expect("position");
        let session = Session::new(document, "Nouveau");

        let (w, h, rgba) = session.filter_preview_in_context(red, &Filter::GaussianBlur { radius: 40.0 }, 512).expect("aperçu");
        assert_eq!((w, h), (512, 512), "document réduit de moitié");
        let at = |x: usize, y: usize| [rgba[(y * 512 + x) * 4], rgba[(y * 512 + x) * 4 + 1], rgba[(y * 512 + x) * 4 + 2]];
        assert_eq!(at(10, 10), [255, 255, 255], "le fond est là");
        assert_eq!(at(200, 200), [0, 255, 0], "le calque du dessus recouvre le calque filtré");
        let core = at(300, 300);
        assert!(core[0] == 255 && core[1] < 4 && core[2] < 4, "cœur du calque filtré : {core:?}");
        let edge = at(350, 300);
        assert!(edge[1] > 40 && edge[1] < 215, "bord du rouge flouté sur le blanc : {edge:?}");
        // Le document, lui, n'a pas bougé.
        assert_eq!(session.renderer().tile(session.document(), 0, TileCoord::new(2, 1)).expect("tuile").pixel(138, 100), [255, 0, 0, 255], "pixel (650, 356) : toujours net");
    }

    #[test]
    fn a_filter_applied_as_a_single_gesture_changes_the_image() {
        // Cas de la fenêtre de filtre validée sans aperçu sur l'image : un seul appel, avec une clé de geste.
        use crate::filters::Filter;
        let document = Document::with_background("x", 300, 300, 72.0, Background::White).expect("document");
        let background = document.walk_top_down().last().expect("fond").1.id;
        let mut session = Session::new(document, "Nouveau");
        let (_, dirty) = session
            .apply_gesture("Ajout de bruit", "filtre-1".into(), |doc| doc.apply_filter(background, &Filter::Noise { amount: 50.0, monochrome: true }))
            .expect("filtre");
        assert!(dirty.all || !dirty.tiles.is_empty(), "l'interface doit être prévenue des tuiles modifiées : {dirty:?}");
        assert_eq!(session.history().0.len(), 2, "un état d'historique");
        let tile = session.renderer().tile(session.document(), 0, TileCoord::new(0, 0)).expect("tuile");
        assert!((0..100).any(|x| tile.pixel(x, 10) != [255; 4]), "l'image a changé");
    }

    #[test]
    fn a_dropped_selection_comes_back_with_undo() {
        use crate::selection::Shape;
        let (mut session, _) = session();
        session.apply("Sélection", None, |doc| {
            doc.set_selection(Some(Shape::Rect { x: 10.0, y: 10.0, width: 50.0, height: 50.0 }.rasterize(600, 400)));
            Ok(())
        }).expect("sélection");
        session.apply("Désélectionner", None, |doc| {
            doc.set_selection(None);
            Ok(())
        }).expect("Ctrl+D");
        assert!(session.document().selection().is_none());
        let dirty = session.undo();
        assert!(session.document().selection().is_some(), "la sélection est revenue");
        assert!(dirty.selection, "l'interface est prévenue : le contour doit réapparaître");
    }

    #[test]
    fn cancelling_a_gesture_removes_it_from_history() {
        let (mut session, id) = session();
        let states = session.history().0.len();
        session.apply_gesture("Transformation", "t-1".into(), |doc| doc.translate_layers(&[id], 30, 0)).expect("geste");
        assert!(session.cancel_gesture("autre").tiles.is_empty(), "mauvais geste : rien ne se passe");
        session.cancel_gesture("t-1");
        assert_eq!(session.document().layer(id).expect("calque").offset, (0, 0));
        assert_eq!(session.history().0.len(), states);
        assert!(!session.can_redo(), "un geste annulé ne peut pas être rétabli");
    }

    #[test]
    fn history_is_bounded() {
        let (mut session, id) = session();
        for i in 0..80 {
            session.apply(format!("Étape {i}"), None, |doc| doc.edit_props(id, |p| p.opacity = i as f32 / 80.0)).expect("étape");
        }
        assert_eq!(session.history().0.len(), MAX_STATES);
    }
}
