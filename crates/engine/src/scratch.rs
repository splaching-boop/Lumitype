//! Disque de travail de l'historique, comme le « disque de travail » de Photoshop.
//!
//! Au-delà du plafond de mémoire vive (module `memory`), une fois les caches de rendu serrés, les états
//! d'annulation ANCIENS partent sur le disque au lieu d'être abandonnés : leurs tuiles qui ne sont retenues par
//! aucun état resté en mémoire (l'état courant, les états récents) sont écrites, compressées, dans un fichier
//! d'échange propre à la session, puis libérées. L'état garde en mémoire son squelette (calques, propriétés,
//! textes : quelques octets) et, pour chaque tuile retirée, sa place dans le fichier. Revenir à cet état (Ctrl+Z,
//! clic dans l'Historique) relit ses tuiles à ce moment-là seulement.
//!
//! # Fichier d'échange
//!
//! Un par session, créé à la première écriture dans le dossier réglé ([`set_scratch`], par défaut
//! `<cache>/dev.imageeditor.app/scratch/`), supprimé à la fermeture du document ; les restes d'une session qui
//! s'est mal terminée sont effacés au démarrage ([`clean_leftovers`]). Format : un en-tête de 8 octets
//! ([`MAGIC`]), puis les tuiles compressées en LZ4 (bloc brut, sans en-tête), écrites en ajout. L'index
//! (position, longueur, nombre de canaux, nombre d'états qui s'en servent) n'existe qu'en mémoire : le fichier ne
//! survit pas à la session, il n'a pas à se relire seul.
//!
//! Une tuile partagée par plusieurs états n'est écrite qu'une fois : le fichier se souvient, par adresse, des
//! tuiles qu'il a déjà (référence faible, pour qu'une adresse libérée ne soit jamais confondue avec une autre),
//! et une tuile relue est rendue à tous les états qui la demandent ensuite. Les emplacements que plus aucun état
//! n'utilise sont récupérés en compactant le fichier quand ils dépassent la moitié de sa taille, et le fichier
//! revient à son seul en-tête quand plus aucun état n'est sur le disque.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use crate::document::{Document, LayerId};
use crate::tiles::{MaskTile, Tile, TileBuf, TileCoord};

/// En-tête du fichier d'échange : format 1.
pub const MAGIC: &[u8; 8] = b"LUMISWP1";
/// Extension des fichiers d'échange : c'est elle que le nettoyage au démarrage reconnaît.
pub const EXTENSION: &str = "lumiswap";
/// Suffixe du fichier temporaire d'un compactage (`.lumiswap.tmp`), lui aussi effacé au démarrage.
const TEMP_SUFFIX: &str = "tmp";
/// Quota par défaut : 20 Gio.
pub const DEFAULT_QUOTA: u64 = 20 * 1024 * 1024 * 1024;
/// En dessous, on ne compacte pas : récupérer quelques mégaoctets ne vaut pas une réécriture.
const COMPACT_MIN_GARBAGE: u64 = 64 * 1024 * 1024;

static ENABLED: AtomicBool = AtomicBool::new(true);
static QUOTA: AtomicU64 = AtomicU64::new(DEFAULT_QUOTA);
static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Octets de tous les fichiers d'échange du processus (en-têtes compris).
static DISK_BYTES: AtomicU64 = AtomicU64::new(0);
/// Numéro des fichiers d'échange, unique dans le processus.
static NEXT_FILE: AtomicU64 = AtomicU64::new(1);

/// Réglages du disque de travail : actif ou non, dossier (`None` : celui par défaut) et quota en octets, partagé
/// par tous les documents ouverts. Appliqués aux écritures suivantes ; un fichier déjà ouvert reste où il est.
pub fn set_scratch(enabled: bool, dir: Option<PathBuf>, quota: u64) {
    ENABLED.store(enabled, Ordering::Relaxed);
    QUOTA.store(quota.max(1), Ordering::Relaxed);
    *DIR.lock().unwrap_or_else(PoisonError::into_inner) = dir;
}

/// Dossier par défaut, fixé une fois par l'application (dossier de cache) ; il sert quand aucun dossier n'est
/// réglé. Sans lui (tests, moteur seul), le dossier temporaire du système.
static DEFAULT_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_default_dir(dir: PathBuf) {
    *DEFAULT_DIR.lock().unwrap_or_else(PoisonError::into_inner) = Some(dir);
}

/// Dossier où le prochain fichier d'échange sera créé.
pub fn scratch_dir() -> PathBuf {
    if let Some(dir) = DIR.lock().unwrap_or_else(PoisonError::into_inner).clone() {
        return dir;
    }
    DEFAULT_DIR.lock().unwrap_or_else(PoisonError::into_inner).clone().unwrap_or_else(|| std::env::temp_dir().join("lumitype-scratch"))
}

/// Octets occupés sur le disque par les fichiers d'échange de toutes les sessions.
pub fn scratch_bytes() -> u64 {
    DISK_BYTES.load(Ordering::Relaxed)
}

/// Efface les fichiers d'échange laissés par une session précédente qui ne s'est pas fermée proprement. À appeler
/// au démarrage, avant tout document. Retourne le nombre de fichiers effacés. (Un fichier encore ouvert par une
/// autre instance ne peut pas être effacé sous Windows ; ailleurs, l'instance garde son descripteur et continue.)
pub fn clean_leftovers(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_scratch_file(path))
        .filter(|path| std::fs::remove_file(path).is_ok())
        .count()
}

/// Fichier d'échange (`.lumiswap`) ou temporaire d'un compactage interrompu (`.lumiswap.tmp`).
fn is_scratch_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    name.ends_with(&format!(".{EXTENSION}")) || name.ends_with(&format!(".{EXTENSION}.{TEMP_SUFFIX}"))
}

/// Réglages en vigueur pour une session (les tests en fixent d'autres, propres à leur session).
#[derive(Clone, Debug)]
pub struct Config {
    pub enabled: bool,
    pub dir: PathBuf,
    pub quota: u64,
}

impl Config {
    pub fn current() -> Self {
        Self { enabled: ENABLED.load(Ordering::Relaxed), dir: scratch_dir(), quota: QUOTA.load(Ordering::Relaxed) }
    }
}

/// Où se trouve un stockage de tuiles dans un document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum StoreId {
    Pixels(LayerId),
    Mask(LayerId),
    Selection,
}

/// Une tuile de calque (4 canaux) ou de masque (1 canal).
#[derive(Clone)]
enum AnyTile {
    Rgba(Arc<Tile>),
    Gray(Arc<MaskTile>),
}

#[derive(Clone)]
enum AnyWeak {
    Rgba(Weak<Tile>),
    Gray(Weak<MaskTile>),
}

impl AnyTile {
    fn address(&self) -> usize {
        match self {
            Self::Rgba(tile) => Arc::as_ptr(tile) as usize,
            Self::Gray(tile) => Arc::as_ptr(tile) as usize,
        }
    }

    fn bytes(&self) -> &[u8] {
        match self {
            Self::Rgba(tile) => tile.bytes(),
            Self::Gray(tile) => tile.bytes(),
        }
    }

    fn channels(&self) -> u8 {
        match self {
            Self::Rgba(_) => 4,
            Self::Gray(_) => 1,
        }
    }

    fn downgrade(&self) -> AnyWeak {
        match self {
            Self::Rgba(tile) => AnyWeak::Rgba(Arc::downgrade(tile)),
            Self::Gray(tile) => AnyWeak::Gray(Arc::downgrade(tile)),
        }
    }
}

impl AnyWeak {
    fn upgrade(&self) -> Option<AnyTile> {
        match self {
            Self::Rgba(weak) => weak.upgrade().map(AnyTile::Rgba),
            Self::Gray(weak) => weak.upgrade().map(AnyTile::Gray),
        }
    }

    fn alive(&self) -> bool {
        match self {
            Self::Rgba(weak) => weak.strong_count() > 0,
            Self::Gray(weak) => weak.strong_count() > 0,
        }
    }

    /// Même allocation que `tile` (la référence faible empêche toute réutilisation de l'adresse).
    fn is(&self, tile: &AnyTile) -> bool {
        match (self, tile) {
            (Self::Rgba(weak), AnyTile::Rgba(tile)) => std::ptr::eq(weak.as_ptr(), Arc::as_ptr(tile)),
            (Self::Gray(weak), AnyTile::Gray(tile)) => std::ptr::eq(weak.as_ptr(), Arc::as_ptr(tile)),
            _ => false,
        }
    }
}

/// Emplacement d'une tuile dans le fichier.
#[derive(Clone, Copy, Debug)]
struct Slot {
    offset: u64,
    len: u32,
    channels: u8,
    /// Nombre d'états sur le disque qui s'en servent ; à zéro, l'emplacement est récupérable.
    refs: u32,
    /// Faux après un compactage qui l'a écarté : l'emplacement n'existe plus.
    valid: bool,
}

/// Fichier d'échange d'une session.
pub(crate) struct ScratchFile {
    path: PathBuf,
    file: File,
    /// Longueur du fichier (fin de la dernière écriture).
    end: u64,
    slots: Vec<Slot>,
    /// Tuiles déjà écrites, par adresse : une tuile partagée n'est écrite qu'une fois.
    by_address: HashMap<usize, (AnyWeak, u32)>,
    /// Tuiles relues (ou encore vivantes), par emplacement : une tuile relue est partagée, pas relue deux fois.
    by_slot: HashMap<u32, AnyWeak>,
    /// Octets des emplacements encore utilisés.
    live: u64,
}

pub(crate) type SharedScratch = Arc<Mutex<ScratchFile>>;

/// Le disque de travail ne peut pas prendre cet état : quota atteint, ou erreur d'écriture (disque plein, dossier
/// supprimé…). Dans les deux cas la session élague comme sans disque de travail : le détail ne change rien.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SpillError {
    Quota,
    Io,
}

impl From<io::Error> for SpillError {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

impl ScratchFile {
    /// Crée le fichier d'échange d'une session dans `dir` (créé s'il le faut).
    pub(crate) fn create(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let name = format!("historique-{}-{}.{EXTENSION}", std::process::id(), NEXT_FILE.fetch_add(1, Ordering::Relaxed));
        let path = dir.join(name);
        let mut file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&path)?;
        file.write_all(MAGIC)?;
        DISK_BYTES.fetch_add(MAGIC.len() as u64, Ordering::Relaxed);
        Ok(Self { path, file, end: MAGIC.len() as u64, slots: Vec::new(), by_address: HashMap::new(), by_slot: HashMap::new(), live: 0 })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Emplacement de `tile` : celui où elle est déjà, ou un nouveau à la fin du fichier.
    fn slot_for(&mut self, tile: &AnyTile, quota: u64) -> Result<u32, SpillError> {
        if let Some((weak, slot)) = self.by_address.get(&tile.address()) {
            if weak.is(tile) && self.slots[*slot as usize].valid {
                return Ok(*slot);
            }
        }
        let packed = lz4_flex::block::compress(tile.bytes());
        if scratch_bytes() + packed.len() as u64 > quota {
            return Err(SpillError::Quota);
        }
        self.file.seek(SeekFrom::Start(self.end))?;
        self.file.write_all(&packed)?;
        let slot = self.slots.len() as u32;
        self.slots.push(Slot { offset: self.end, len: packed.len() as u32, channels: tile.channels(), refs: 0, valid: true });
        self.end += packed.len() as u64;
        DISK_BYTES.fetch_add(packed.len() as u64, Ordering::Relaxed);
        self.by_address.insert(tile.address(), (tile.downgrade(), slot));
        self.by_slot.insert(slot, tile.downgrade());
        Ok(slot)
    }

    fn retain(&mut self, slot: u32) {
        let entry = &mut self.slots[slot as usize];
        if entry.refs == 0 {
            self.live += u64::from(entry.len);
        }
        entry.refs += 1;
    }

    fn release(&mut self, slot: u32) {
        let entry = &mut self.slots[slot as usize];
        entry.refs = entry.refs.saturating_sub(1);
        if entry.refs == 0 {
            self.live = self.live.saturating_sub(u64::from(entry.len));
        }
    }

    /// La tuile d'un emplacement : encore vivante (partagée), ou relue et décompressée.
    fn load(&mut self, slot: u32) -> io::Result<AnyTile> {
        if let Some(tile) = self.by_slot.get(&slot).and_then(AnyWeak::upgrade) {
            return Ok(tile);
        }
        let entry = *self.slots.get(slot as usize).filter(|s| s.valid).ok_or_else(|| io::Error::other("emplacement inconnu dans le fichier d'échange"))?;
        let mut packed = vec![0u8; entry.len as usize];
        self.file.seek(SeekFrom::Start(entry.offset))?;
        self.file.read_exact(&mut packed)?;
        let raw = (crate::tiles::TILE_SIZE * crate::tiles::TILE_SIZE) as usize * usize::from(entry.channels);
        let bytes = lz4_flex::block::decompress(&packed, raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("tuile illisible : {e}")))?;
        let corrupt = || io::Error::new(io::ErrorKind::InvalidData, "tuile de taille inattendue");
        let tile = match entry.channels {
            4 => AnyTile::Rgba(Arc::new(TileBuf::from_vec(bytes).ok_or_else(corrupt)?)),
            1 => AnyTile::Gray(Arc::new(TileBuf::from_vec(bytes).ok_or_else(corrupt)?)),
            _ => return Err(corrupt()),
        };
        self.by_slot.insert(slot, tile.downgrade());
        self.by_address.insert(tile.address(), (tile.downgrade(), slot));
        Ok(tile)
    }

    /// Ménage avant une série d'écritures : oubli des tuiles mortes, fichier remis à zéro s'il ne sert plus à
    /// rien, compactage si les emplacements inutiles dépassent la moitié du fichier.
    fn tidy(&mut self) -> io::Result<()> {
        self.by_address.retain(|_, (weak, _)| weak.alive());
        self.by_slot.retain(|_, weak| weak.alive());
        let header = MAGIC.len() as u64;
        if self.slots.iter().all(|slot| slot.refs == 0) {
            if self.end > header {
                self.file.set_len(header)?;
                DISK_BYTES.fetch_sub(self.end - header, Ordering::Relaxed);
                self.end = header;
            }
            self.slots.clear();
            self.by_address.clear();
            self.by_slot.clear();
            self.live = 0;
            return Ok(());
        }
        let garbage = self.end - header - self.live;
        if garbage > self.live && garbage >= COMPACT_MIN_GARBAGE {
            self.compact()?;
        }
        Ok(())
    }

    /// Réécrit le fichier sans les emplacements inutilisés. Les numéros d'emplacement ne changent pas. En cas
    /// d'échec, le fichier d'origine reste tel quel (emplacements et descripteur), et le temporaire est supprimé.
    fn compact(&mut self) -> io::Result<()> {
        let temporary = self.path.with_extension(format!("{EXTENSION}.{TEMP_SUFFIX}"));
        let result = self.compact_into(&temporary);
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    fn compact_into(&mut self, temporary: &Path) -> io::Result<()> {
        let mut out = io::BufWriter::new(OpenOptions::new().read(true).write(true).create(true).truncate(true).open(temporary)?);
        out.write_all(MAGIC)?;
        let mut end = MAGIC.len() as u64;
        let mut moved = self.slots.clone();
        let mut buffer = Vec::new();
        for slot in &mut moved {
            if !slot.valid || slot.refs == 0 {
                slot.valid = false;
                continue;
            }
            buffer.resize(slot.len as usize, 0);
            self.file.seek(SeekFrom::Start(slot.offset))?;
            self.file.read_exact(&mut buffer)?;
            out.write_all(&buffer)?;
            slot.offset = end;
            end += u64::from(slot.len);
        }
        let file = out.into_inner().map_err(io::IntoInnerError::into_error)?;
        file.sync_data()?;
        // Sous Windows, un fichier ouvert ne peut pas être remplacé par un renommage : l'ancien descripteur est
        // fermé d'abord. Si le renommage échoue, l'ancien fichier, intact, est rouvert et rien d'autre ne change.
        let previous = std::mem::replace(&mut self.file, file);
        drop(previous);
        if let Err(error) = std::fs::rename(temporary, &self.path) {
            self.file = OpenOptions::new().read(true).write(true).open(&self.path)?;
            return Err(error);
        }
        DISK_BYTES.fetch_sub(self.end - end, Ordering::Relaxed);
        self.end = end;
        self.slots = moved;
        let slots = &self.slots;
        self.by_address.retain(|_, (_, slot)| slots[*slot as usize].valid);
        self.by_slot.retain(|slot, _| slots[*slot as usize].valid);
        Ok(())
    }
}

impl Drop for ScratchFile {
    /// Fermeture du document : le fichier d'échange disparaît avec lui.
    fn drop(&mut self) {
        DISK_BYTES.fetch_sub(self.end, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Un état d'historique sur le disque : son squelette (le document sans les tuiles écrites) et, pour chaque
/// tuile retirée, son stockage, sa position et son emplacement dans le fichier.
pub(crate) struct SpilledState {
    skeleton: Document,
    tiles: Vec<(StoreId, TileCoord, u32)>,
    file: SharedScratch,
}

impl SpilledState {
    /// Le squelette, pour ce qui ne touche pas aux pixels (renommer le document).
    pub(crate) fn skeleton_mut(&mut self) -> &mut Document {
        &mut self.skeleton
    }

    /// Le document complet, tuiles relues. Le fichier est verrouillé le temps de la lecture.
    pub(crate) fn restore(&self) -> io::Result<Document> {
        let mut doc = self.skeleton.clone();
        let mut file = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        for &(store, coord, slot) in &self.tiles {
            let tile = file.load(slot)?;
            let missing = || io::Error::other("stockage disparu du squelette d'un état d'historique");
            match (store, tile) {
                (StoreId::Pixels(id), AnyTile::Rgba(tile)) => put_back(doc.pixels_slot(id).ok_or_else(missing)?, coord, Some(tile)),
                (StoreId::Mask(id), AnyTile::Gray(tile)) => put_back(doc.mask_slot(id).ok_or_else(missing)?, coord, Some(tile)),
                (StoreId::Selection, AnyTile::Gray(tile)) => put_back(doc.selection_slot().ok_or_else(missing)?, coord, Some(tile)),
                _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "tuile du mauvais type dans le fichier d'échange")),
            }
        }
        Ok(doc)
    }
}

impl Drop for SpilledState {
    /// État abandonné ou relu : ses emplacements ne lui servent plus.
    fn drop(&mut self) {
        let mut file = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        for &(_, _, slot) in &self.tiles {
            file.release(slot);
        }
    }
}

/// Adresses des tuiles d'un document (toutes ses tuiles, partagées ou non), ajoutées à `into`. Les stockages déjà
/// vus (partagés d'un état à l'autre) sont sautés d'un coup.
pub(crate) fn collect_addresses(doc: &Document, into: &mut std::collections::HashSet<usize>, stores: &mut std::collections::HashSet<usize>) {
    for layer in doc.nodes() {
        if let Some(pixels) = layer.pixels() {
            if stores.insert(Arc::as_ptr(pixels) as usize) {
                into.extend(pixels.iter().map(|(_, tile)| Arc::as_ptr(tile) as usize));
            }
        }
        if let Some(mask) = &layer.mask {
            if stores.insert(Arc::as_ptr(&mask.pixels) as usize) {
                into.extend(mask.pixels.iter().map(|(_, tile)| Arc::as_ptr(tile) as usize));
            }
        }
    }
    if let Some(selection) = doc.selection() {
        if stores.insert(Arc::as_ptr(selection) as usize) {
            into.extend(selection.iter().map(|(_, tile)| Arc::as_ptr(tile) as usize));
        }
    }
}

/// Écrit sur le disque les tuiles de `doc` qu'aucun état resté en mémoire ne retient (`keep` : leurs adresses) et
/// retourne l'état allégé. `Ok(None)` : rien à écrire, tout est partagé avec un état en mémoire (l'écrire ne
/// libérerait rien). En cas d'erreur, rien n'a changé pour `doc` (des octets ont pu être ajoutés au fichier : ils
/// seront récupérés au prochain compactage).
pub(crate) fn spill(file: &SharedScratch, doc: &Document, keep: &std::collections::HashSet<usize>, quota: u64) -> Result<Option<SpilledState>, SpillError> {
    let mut gathered: Vec<(StoreId, TileCoord, AnyTile)> = Vec::new();
    for layer in doc.nodes() {
        if let Some(pixels) = layer.pixels() {
            gathered.extend(pixels.iter().filter(|(_, t)| !keep.contains(&(Arc::as_ptr(t) as usize))).map(|(c, t)| (StoreId::Pixels(layer.id), c, AnyTile::Rgba(Arc::clone(t)))));
        }
        if let Some(mask) = &layer.mask {
            gathered.extend(mask.pixels.iter().filter(|(_, t)| !keep.contains(&(Arc::as_ptr(t) as usize))).map(|(c, t)| (StoreId::Mask(layer.id), c, AnyTile::Gray(Arc::clone(t)))));
        }
    }
    if let Some(selection) = doc.selection() {
        gathered.extend(selection.iter().filter(|(_, t)| !keep.contains(&(Arc::as_ptr(t) as usize))).map(|(c, t)| (StoreId::Selection, c, AnyTile::Gray(Arc::clone(t)))));
    }
    if gathered.is_empty() {
        return Ok(None);
    }
    let mut guard = file.lock().unwrap_or_else(PoisonError::into_inner);
    let mut tiles = Vec::with_capacity(gathered.len());
    for (store, coord, tile) in &gathered {
        tiles.push((*store, *coord, guard.slot_for(tile, quota)?));
    }
    guard.file.flush()?;
    for &(_, _, slot) in &tiles {
        guard.retain(slot);
    }
    drop(guard);
    // Tout est écrit : le squelette perd ces tuiles.
    let mut skeleton = doc.clone();
    for (store, coord, _) in &gathered {
        match store {
            StoreId::Pixels(id) => {
                if let Some(slot) = skeleton.pixels_slot(*id) {
                    put_back(slot, *coord, None);
                }
            }
            StoreId::Mask(id) => {
                if let Some(slot) = skeleton.mask_slot(*id) {
                    put_back(slot, *coord, None);
                }
            }
            StoreId::Selection => {
                if let Some(slot) = skeleton.selection_slot() {
                    put_back(slot, *coord, None);
                }
            }
        }
    }
    Ok(Some(SpilledState { skeleton, tiles, file: Arc::clone(file) }))
}

/// Retire (`None`) ou remet une tuile d'un stockage SANS changer la version de son contenu : le squelette d'un état
/// et l'état relu représentent toujours le même contenu que l'état d'origine. Les caches de rendu, qui reconnaissent
/// un contenu à sa version (voir `tiles::Store`), resservent donc ce qu'ils avaient après un Ctrl+Z relu du disque.
fn put_back<const C: usize>(slot: &mut Arc<crate::tiles::Store<C>>, coord: TileCoord, tile: Option<Arc<TileBuf<C>>>) {
    let version = slot.version();
    let store = Arc::make_mut(slot);
    store.restore(coord, tile);
    store.set_version(version);
}

/// Ménage du fichier avant une série d'écritures (voir [`ScratchFile::tidy`]).
pub(crate) fn tidy(file: &SharedScratch) -> io::Result<()> {
    file.lock().unwrap_or_else(PoisonError::into_inner).tidy()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(seed: u8) -> AnyTile {
        let mut buf = TileBuf::<4>::filled([seed, 0, 0, 255]);
        buf.set_pixel(u32::from(seed) % crate::tiles::TILE_SIZE, 3, [1, 2, 3, 255]);
        AnyTile::Rgba(Arc::new(buf))
    }

    /// Le compactage garde les emplacements utilisés (relisibles ensuite), écarte les autres, ne laisse aucun
    /// temporaire ; le nettoyage au démarrage efface aussi un temporaire de compactage abandonné.
    #[test]
    fn compaction_keeps_live_slots_and_cleans_temporaries() {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let mut file = ScratchFile::create(dir.path()).expect("fichier d'échange");
        let kept = tile(7);
        let dropped = tile(9);
        let slot_kept = file.slot_for(&kept, u64::MAX).expect("écriture");
        let slot_dropped = file.slot_for(&dropped, u64::MAX).expect("écriture");
        file.retain(slot_kept);
        file.file.flush().expect("flush");
        drop(kept);
        drop(dropped);
        file.by_address.clear();
        file.by_slot.clear();
        file.compact().expect("compactage");
        assert!(file.slots[slot_kept as usize].valid);
        assert!(!file.slots[slot_dropped as usize].valid);
        assert!(file.load(slot_dropped).is_err());
        let back = file.load(slot_kept).expect("relecture");
        assert_eq!(back.bytes(), tile(7).bytes());
        assert_eq!(file.end, std::fs::metadata(file.path()).expect("taille").len());
        let temporary = file.path().with_extension(format!("{EXTENSION}.{TEMP_SUFFIX}"));
        assert!(!temporary.exists(), "le temporaire est renommé");
        // Nettoyage au démarrage : fichiers d'échange et temporaires de compactage, rien d'autre.
        std::fs::write(&temporary, b"x").expect("temporaire");
        std::fs::write(dir.path().join("note.txt"), b"x").expect("fichier");
        drop(file);
        assert_eq!(clean_leftovers(dir.path()), 1);
        assert!(!temporary.exists());
        assert!(dir.path().join("note.txt").exists());
    }
}
