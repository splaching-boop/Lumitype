//! Composition de l'arbre de calques en tuiles affichables.
//!
//! Niveau 0 = pleine résolution ; chaque niveau divise la résolution par deux.
//! La composition se fait **au niveau demandé**, à partir des mips de chaque
//! calque : recomposer la vue après un changement d'opacité coûte un travail
//! proportionnel à l'écran, pas à l'image. Contrepartie connue : pour les modes
//! non linéaires, composer des mips n'est pas identique à réduire la composition
//! (écart visible seulement en zoom arrière ; l'export utilise toujours le niveau 0).
//!
//! Un calque a sa propre position : ses tuiles (et leurs mips) vivent dans son repère
//! local, et la fenêtre correspondant à une tuile du document en est extraite à la
//! volée (`gather`). Au niveau 0 l'extraction est exacte ; aux niveaux réduits, la
//! position est arrondie au pixel de mip le plus proche, soit moins d'un pixel d'écran.
//!
//! Le parcours de l'arbre produit un **programme** (voir `plan.rs`) : la suite des opérations à
//! exécuter pour cette tuile. Le processeur l'exécute ici même ; la carte graphique exécute le même
//! programme dans le crate `gpu`, et doit produire les mêmes tuiles à ±1/255.
//!
//! Règles de composition (celles de Photoshop) :
//! - groupe en mode Transfert : ses enfants fusionnent directement avec le fond,
//!   puis opacité et masque du groupe interpolent entre « avant » et « après » ;
//! - groupe dans un autre mode : composé à part sur fond transparent, puis
//!   fusionné comme un calque ;
//! - masque d'écrêtage : les calques écrêtés fusionnent avec la couleur de leur
//!   base sans en changer l'alpha, puis l'ensemble est fusionné avec le mode,
//!   l'opacité et le masque de la base. Une base invisible masque tout le lot.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::blend::BlendMode;
use crate::color_mode::{finish_tile, ColorMode};
use crate::document::{Document, Layer, LayerId, LayerKind};
use crate::effects::LayerEffect;
use crate::pixel::unpremultiply;
use crate::plan::{Builder, Coverage, Op, Program, Shape, Source};
use crate::tiles::{tiles_for, MaskTile, Store, Tile, TileBuf, TileCoord, TileStore, TILE_SIZE};

/// Identifiant réservé à la sélection dans le cache de mips (les calques commencent à 1).
pub const SELECTION_ID: LayerId = 0;
/// Les réductions des pixels BRUTS d'un calque à effets sont rangées sous son identifiant marqué de ce
/// bit (ses pixels « stylés », effets compris, gardent l'identifiant nu). Voir `Pass::raw_pixels`.
const RAW_PIXELS: LayerId = 1 << 63;
/// Effets du dessous fondus dans l'image avec leur propre mode (ombre portée, lueur externe) : leurs réductions
/// sont rangées sous l'identifiant du calque marqué de ce bit, plus le rang de l'effet (bits 40 et suivants).
const UNDERLAY: LayerId = 1 << 62;
const UNDERLAY_SHIFT: u32 = 40;

/// Calque stylé en cache : le calque et ses effets, plus les effets du dessous à fondre à part dans l'image.
/// Tout est dans le repère du calque (même position que ses pixels).
#[derive(Debug)]
pub struct StyledStores {
    /// Le calque et ceux de ses effets qui s'y fondent directement.
    pub main: Arc<TileStore>,
    /// Effets du dessous fondus à part dans l'image, chacun avec son mode (ombre portée, lueur externe).
    pub under: Vec<(BlendMode, Arc<TileStore>)>,
    /// Effets du dessus fondus à part dans l'image (rangs 64 et suivants des clés de réduction).
    pub over: Vec<(BlendMode, Arc<TileStore>)>,
}

/// Rang de départ des effets du dessus dans les clés de réduction (ceux du dessous commencent à 0).
const OVER_RANK: u64 = 64;

/// Budget FIXE du cache de rendu de chaque document, en octets (0 = aucun). Il ne sert plus qu'aux tests et à
/// la compatibilité : en usage normal, le budget découle du plafond de mémoire (voir [`cache_budget_for`]).
static CACHE_BUDGET: AtomicU64 = AtomicU64::new(0);

pub fn set_cache_budget(bytes: u64) {
    CACHE_BUDGET.store(bytes, Ordering::Relaxed);
}

pub fn cache_budget() -> u64 {
    CACHE_BUDGET.load(Ordering::Relaxed)
}

/// Budget du cache d'un document à cet instant, en octets (0 = illimité), pour un cache qui occupe déjà
/// `own_cache` octets. Sous un plafond de mémoire, le cache n'a droit qu'à ce que le plafond laisse une fois
/// comptées toutes les autres tuiles vivantes (documents, historique, et caches des autres documents, qui
/// pèsent autant sur la machine), avec un minimum de [`crate::memory::MIN_CACHE_BUDGET`]. Le cache de ce
/// document est retiré du compte, sinon il se limiterait lui-même deux fois. Un budget fixe, s'il y en a
/// un, borne en plus.
pub fn cache_budget_for(own_cache: u64) -> u64 {
    cache_budget_under(crate::memory::memory_cap(), cache_budget(), crate::tiles::resident_bytes().saturating_sub(own_cache))
}

/// Le calcul de [`cache_budget_for`], sans les compteurs globaux : `cap` le plafond de mémoire (0 = aucun),
/// `fixed` le budget fixe (0 = aucun), `others` les octets de toutes les tuiles vivantes hors ce cache.
pub fn cache_budget_under(cap: u64, fixed: u64, others: u64) -> u64 {
    if cap == 0 {
        return fixed;
    }
    let dynamic = cap.saturating_sub(others).max(crate::memory::MIN_CACHE_BUDGET);
    if fixed == 0 { dynamic } else { dynamic.min(fixed) }
}

type MipCache<const C: usize> = Mutex<Counted<(LayerId, u8, TileCoord), Arc<TileBuf<C>>>>;

/// Table de cache qui compte ses entrées remplies (`Some`) au fil des insertions et retraits : la mémoire occupée
/// se lit d'un coup, sans parcourir la table. Avant, chaque tuile ajoutée faisait recompter les cinq tables du
/// cache (plusieurs fois) pour savoir si le budget était dépassé. Les lectures passent par `Deref`.
#[derive(Debug)]
struct Counted<K, V> {
    map: HashMap<K, Option<V>>,
    filled: usize,
}

impl<K, V> Default for Counted<K, V> {
    fn default() -> Self {
        Self { map: HashMap::new(), filled: 0 }
    }
}

impl<K: Eq + std::hash::Hash, V> Counted<K, V> {
    fn insert(&mut self, key: K, value: Option<V>) {
        let added = value.is_some();
        if self.map.insert(key, value).is_some_and(|old| old.is_some()) {
            self.filled -= 1;
        }
        if added {
            self.filled += 1;
        }
    }

    fn remove(&mut self, key: &K) {
        if self.map.remove(key).is_some_and(|old| old.is_some()) {
            self.filled -= 1;
        }
    }

    fn retain(&mut self, mut keep: impl FnMut(&K, &Option<V>) -> bool) {
        let filled = &mut self.filled;
        self.map.retain(|key, value| {
            let kept = keep(key, value);
            if !kept && value.is_some() {
                *filled -= 1;
            }
            kept
        });
    }

    fn clear(&mut self) {
        self.map.clear();
        self.filled = 0;
    }

    /// Entrées remplies (les `None`, tuiles vides, ne pèsent rien).
    fn filled(&self) -> usize {
        self.filled
    }
}

impl<K, V> std::ops::Deref for Counted<K, V> {
    type Target = HashMap<K, Option<V>>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

/// Versions gardées d'une tuile de groupe : l'empreinte de ce qui l'a produite, et la tuile.
type GroupVersions = Vec<(Arc<Vec<Print>>, Option<Arc<Tile>>)>;

/// Contenus figés des groupes, par tuile de leur repère (voir [`GroupCache`]), avec le compte des tuiles remplies.
#[derive(Debug, Default)]
struct CountedGroups {
    map: HashMap<GroupKey, GroupVersions>,
    filled: usize,
}

impl CountedGroups {
    /// Version de `key` composée avec cette empreinte, remise en tête (la version servie est la plus récente).
    fn find(&mut self, key: &GroupKey, print: &[PrintRef<'_>]) -> Option<Option<Arc<Tile>>> {
        let versions = self.map.get_mut(key)?;
        let at = versions.iter().position(|(cached, _)| same_print(cached, print))?;
        let hit = versions.remove(at);
        let tile = hit.1.clone();
        versions.insert(0, hit);
        Some(tile)
    }

    /// Nouvelle version en tête ; au-delà de [`GROUP_VERSIONS`], les plus anciennes partent.
    fn push(&mut self, key: GroupKey, print: Arc<Vec<Print>>, tile: Option<Arc<Tile>>) {
        let versions = self.map.entry(key).or_default();
        self.filled += usize::from(tile.is_some());
        versions.insert(0, (print, tile));
        while versions.len() > GROUP_VERSIONS {
            if let Some((_, Some(_))) = versions.pop() {
                self.filled -= 1;
            }
        }
    }

    fn retain(&mut self, mut keep: impl FnMut(&GroupKey, &GroupVersions) -> bool) {
        let filled = &mut self.filled;
        self.map.retain(|key, versions| {
            let kept = keep(key, versions);
            if !kept {
                *filled -= versions.iter().filter(|(_, tile)| tile.is_some()).count();
            }
            kept
        });
    }

    fn filled(&self) -> usize {
        self.filled
    }
}

impl std::ops::Deref for CountedGroups {
    type Target = HashMap<GroupKey, GroupVersions>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Les caches ne contiennent que des données recalculables : un verrou
    // empoisonné reste utilisable.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Un calque dans l'empreinte d'un groupe (voir `Pass::fingerprint`). Pixels et masque sont repérés par la VERSION
/// de leur contenu (voir `tiles::Store`) et non par leur adresse : un état relu du disque de travail (Ctrl+Z sur un
/// gros document) a de nouveaux stockages aux mêmes pixels, et son contenu figé reste bon.
///
/// Les effets sont EMPRUNTÉS au document (`E = &[LayerEffect]`) le temps de consulter le cache, et copiés
/// (`E = Vec<LayerEffect>`) seulement pour une entrée nouvelle : une empreinte est prise à chaque tuile, un cache
/// qui répond ne doit pas copier les effets de tous les calques du groupe pour rien.
#[derive(Debug, Clone, PartialEq)]
struct Print<E = Vec<LayerEffect>> {
    id: LayerId,
    at: (i32, i32),
    visible: bool,
    blend: BlendMode,
    opacity: u32,
    fill: u32,
    clipped: bool,
    range: Option<crate::blend::BlendIf>,
    pixels: Option<u64>,
    mask: Option<(u64, bool)>,
    effects: E,
    children: usize,
}

/// Empreinte empruntée (voir [`Print`]).
type PrintRef<'a> = Print<&'a [LayerEffect]>;

impl PrintRef<'_> {
    fn to_owned(&self) -> Print {
        Print { id: self.id, at: self.at, visible: self.visible, blend: self.blend, opacity: self.opacity, fill: self.fill, clipped: self.clipped, range: self.range, pixels: self.pixels, mask: self.mask, effects: self.effects.to_vec(), children: self.children }
    }
}

impl Print {
    fn same(&self, probe: &PrintRef<'_>) -> bool {
        self.id == probe.id
            && self.at == probe.at
            && self.visible == probe.visible
            && self.blend == probe.blend
            && self.opacity == probe.opacity
            && self.fill == probe.fill
            && self.clipped == probe.clipped
            && self.range == probe.range
            && self.pixels == probe.pixels
            && self.mask == probe.mask
            && self.effects.as_slice() == probe.effects
            && self.children == probe.children
    }
}

/// Même empreinte, comparée sans copie.
fn same_print(cached: &[Print], probe: &[PrintRef<'_>]) -> bool {
    cached.len() == probe.len() && cached.iter().zip(probe).all(|(a, b)| a.same(b))
}

/// Clé du cache des groupes : le groupe, le premier calque de la série figée (le groupe lui-même pour tout son
/// contenu, voir `Pass::flat_run`), le niveau et la tuile, dans le repère du groupe.
type GroupKey = (LayerId, LayerId, u8, TileCoord);
/// Contenus figés gardés par tuile, du plus récent au plus ancien : déplacer puis annuler (Ctrl+Z), ou passer d'un
/// état à l'autre dans l'Historique, retrouve le contenu d'avant au lieu de le recomposer.
const GROUP_VERSIONS: usize = 3;
/// Contenu composé d'un groupe (ou d'une série de ses calques), par tuile de son repère : l'empreinte de ce qui l'a
/// produit, et la tuile — quelques versions par tuile (voir [`GROUP_VERSIONS`]).
type GroupCache = Mutex<CountedGroups>;

/// Clé des couches fixes de l'aperçu de déplacement : empreinte du contenu, niveau, tuile.
type SliceKey = (u64, u8, TileCoord);

/// Les caches de tuiles d'un document : ce que le plafond de mémoire peut vider. Ils sont partagés (`Arc`)
/// entre le [`TileRenderer`] qui les remplit et le registre du module `memory`, qui peut les serrer tous
/// quand une session grossit, même ceux des documents qui ne sont pas à l'écran.
/// Tuiles composées, par niveau de mip et coordonnée (`None` : tuile entièrement transparente).
type CompositeMap = Counted<(u8, TileCoord), Arc<Tile>>;
/// Calque stylé en cache : les pixels d'origine et les effets qui ont servi, puis le résultat.
type StyledEntry = (Arc<TileStore>, Vec<LayerEffect>, Arc<StyledStores>);

#[derive(Debug, Default)]
pub struct Caches {
    composite: Mutex<CompositeMap>,
    pixel_mips: MipCache<4>,
    mask_mips: MipCache<1>,
    /// Contenu composé des groupes, dans leur repère : un groupe déplacé d'un bloc n'est pas recomposé.
    groups: GroupCache,
    /// Couches fixes de l'aperçu de déplacement (« dessous », « dessus »), par empreinte de leur contenu (voir
    /// `MoveSlice::fingerprint`), niveau et tuile : un deuxième Ctrl+T sur le même groupe, ou un nouveau geste
    /// après un déplacement, les retrouve au lieu de les recomposer (leur contenu n'a pas changé).
    move_slices: Mutex<Counted<SliceKey, Arc<Tile>>>,
    /// Tuiles de calque entièrement opaques, par version de contenu et tuile locale (voir `covered`).
    opaque_tiles: Mutex<HashMap<(u64, TileCoord), bool>>,
}

impl Caches {
    /// Mémoire occupée par les caches, en octets (estimation : une tuile RGBA = 256 Kio, une tuile de masque 64 Kio).
    /// Les tables comptent leurs entrées remplies au fil de l'eau (voir [`Counted`]) : lecture immédiate.
    pub fn bytes(&self) -> u64 {
        let rgba = (TILE_SIZE * TILE_SIZE * 4) as u64;
        let composite = lock(&self.composite).filled() as u64 * rgba;
        let pixels = lock(&self.pixel_mips).filled() as u64 * rgba;
        let masks = lock(&self.mask_mips).filled() as u64 * rgba / 4;
        let groups = lock(&self.groups).filled() as u64 * rgba;
        let slices = lock(&self.move_slices).filled() as u64 * rgba;
        composite + pixels + masks + groups + slices
    }

    /// Respecte le budget (appelé à chaque tuile ajoutée, et quand le budget ou le plafond de mémoire change) :
    /// au-delà, les caches sont vidés (ils se reconstruisent à la demande). Ce sont des caches purs : les vider
    /// ne perd rien, cela coûte seulement un recalcul. Le budget est recalculé à chaque passage : sous un plafond
    /// de mémoire, il rétrécit à mesure que documents et historique grossissent (voir [`cache_budget_for`]).
    pub fn trim(&self) {
        self.trim_to(cache_budget_for(self.bytes()));
    }

    /// Serre les caches s'ils dépassent `budget` octets (0 = illimité) : seulement une partie des entrées est
    /// retirée, jusqu'à repasser sous les trois quarts du budget. Tout vider d'un coup, comme avant, faisait
    /// que le cache plein se vidait à CHAQUE tuile ajoutée (la suivante le remettait au-dessus) : l'image
    /// entière se recomposait sans cesse, d'où des gels brutaux à l'usage dès que la mémoire était juste.
    /// Avec cette marge, le cache se vide un peu, puis se remplit tranquillement avant la prochaine coupe.
    pub fn trim_to(&self, budget: u64) {
        if budget == 0 {
            return;
        }
        let used = self.bytes();
        if used <= budget {
            return;
        }
        let target = budget / 4 * 3;
        let mut excess = used - target;
        let rgba = (TILE_SIZE * TILE_SIZE * 4) as u64;
        // D'abord les réductions par calque (vite recalculées depuis les pixels), puis les tuiles composées
        // (plus coûteuses : toute la pile de calques). Les entrées « None » (tuiles vides) ne pèsent rien.
        fn evict<K: Eq + std::hash::Hash, V>(map: &mut Counted<K, V>, each: u64, excess: &mut u64) {
            if *excess == 0 {
                return;
            }
            let wanted = (*excess).div_ceil(each) as usize;
            let mut dropped = 0usize;
            map.retain(|_, tile| {
                if dropped >= wanted || tile.is_none() {
                    return true;
                }
                dropped += 1;
                false
            });
            *excess = excess.saturating_sub(dropped as u64 * each);
        }
        evict(&mut lock(&self.move_slices), rgba, &mut excess);
        evict(&mut lock(&self.pixel_mips), rgba, &mut excess);
        evict(&mut lock(&self.mask_mips), rgba / 4, &mut excess);
        if excess > 0 {
            let mut groups = lock(&self.groups);
            let wanted = excess.div_ceil(rgba) as usize;
            let mut dropped = 0usize;
            groups.retain(|_, versions| {
                let filled = versions.iter().filter(|(_, tile)| tile.is_some()).count();
                if dropped >= wanted || filled == 0 {
                    return true;
                }
                dropped += filled;
                false
            });
            excess = excess.saturating_sub(dropped as u64 * rgba);
        }
        evict(&mut lock(&self.composite), rgba, &mut excess);
    }
}

/// Recalcul local des effets tant que les tuiles changées ne dépassent pas ce quart des tuiles du calque (et au moins
/// [`LOCAL_RESTYLE_MIN`] tuiles) ; au-delà, tout le calque, c'est moins cher.
const LOCAL_RESTYLE_SHARE: usize = 4;
const LOCAL_RESTYLE_MIN: usize = 16;

/// Rendu stylé `cached` (fait pour les pixels `known`) mis à jour pour `pixels` en ne recalculant les effets
/// qu'autour des tuiles qui diffèrent (élargies de la portée des effets). `None` quand cela ne se prête pas :
/// effet tendu sur tout le calque, trop de tuiles changées, rendu précédent sans effets (échec), ou calcul impossible.
fn restyled_locally(known: &Arc<TileStore>, pixels: &TileStore, effects: &[LayerEffect], cached: &StyledStores) -> Option<StyledStores> {
    if !crate::effects::locally_recomputable(effects) || Arc::ptr_eq(&cached.main, known) {
        return None;
    }
    let changed = known.changed_coords(pixels);
    let tiles = pixels.tile_count().max(known.tile_count());
    if changed.is_empty() || changed.len() > (tiles / LOCAL_RESTYLE_SHARE).max(LOCAL_RESTYLE_MIN) {
        return None;
    }
    let size = TILE_SIZE as i32;
    let reach = crate::effects::reach_enabled(effects);
    let (x0, y0, x1, y1) = changed.iter().fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |(a, b, c, d), t| {
        let (ox, oy) = t.origin();
        (a.min(ox), b.min(oy), c.max(ox + size), d.max(oy + size))
    });
    let region = (x0 - reach, y0 - reach, x1 + reach, y1 + reach);
    let fresh = crate::effects::restyle_region(pixels, effects, region).ok()?;
    if fresh.under.len() != cached.under.len() || fresh.over.len() != cached.over.len() {
        return None;
    }
    let patched = |store: &Arc<TileStore>, part: &TileStore| {
        let mut store = TileStore::clone(store);
        store.paste(part, region);
        Arc::new(store)
    };
    Some(StyledStores {
        main: patched(&cached.main, &fresh.main),
        under: cached.under.iter().zip(&fresh.under).map(|((mode, store), (_, part))| (*mode, patched(store, part))).collect(),
        over: cached.over.iter().zip(&fresh.over).map(|((mode, store), (_, part))| (*mode, patched(store, part))).collect(),
    })
}

/// Calcule et met en cache les tuiles affichables. Les méthodes `invalidate_*`
/// doivent être appelées à chaque modification du document (voir `session`).
#[derive(Debug)]
pub struct TileRenderer {
    /// Caches de tuiles, inscrits au registre du plafond de mémoire à la création.
    caches: Arc<Caches>,
    /// Calques stylés (pixels + effets), par calque : pixels d'origine et effets pour lesquels le rendu a
    /// été fait. Tant que ni l'un ni l'autre ne change, il est réutilisé. Les pixels d'origine sont GARDÉS
    /// (et non leur seule adresse) : une adresse libérée peut être réattribuée à d'autres pixels, et le
    /// cache rendait alors l'ancien rendu — une forme redimensionnée gardait autour d'elle le fantôme de
    /// son ancienne silhouette, pris pour une ombre « blanche et nette ».
    styled: Mutex<HashMap<LayerId, StyledEntry>>,
    /// Incrémenté à chaque invalidation : un calcul commencé avant n'est pas mis en cache.
    generation: AtomicU64,
}

impl Default for TileRenderer {
    fn default() -> Self {
        let caches = Arc::new(Caches::default());
        crate::memory::register_caches(&caches);
        Self { caches, styled: Mutex::default(), generation: AtomicU64::new(0) }
    }
}

impl TileRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Dernier niveau utile : celui où le document tient dans une seule tuile.
    pub fn max_level(doc: &Document) -> u8 {
        let mut tiles = tiles_for(doc.width().max(doc.height()));
        let mut level = 0;
        while tiles > 1 {
            tiles = tiles.div_ceil(2);
            level += 1;
        }
        level
    }

    /// Programme de composition d'une tuile : la suite d'opérations à exécuter, que ce soit par le
    /// processeur ou par la carte graphique (crate `gpu`). Les tuiles source y sont déjà résolues.
    pub fn program(&self, doc: &Document, level: u8, coord: TileCoord) -> Program {
        if level > Self::max_level(doc) {
            return Program::Ready(None);
        }
        let generation = self.generation.load(Ordering::Acquire);
        let program = Pass { renderer: self, doc, level, coord, generation, include: None, opaque: None, replace: None, shift: (0, 0) }.program();
        // Mode gris ou bitmap : le rendu final (voir `color_mode::finish_tile`) suit la composition. La carte graphique
        // l'applique à la présentation de l'image (`viewport/gpu-layers.ts`) ; ici, la tuile est livrée finie.
        match doc.color_mode() {
            ColorMode::Rgb | ColorMode::CmykWorkflow { .. } => program,
            mode => Program::Ready(finish_tile(mode, level, program.run_on_cpu())),
        }
    }

    /// Tuile composée au niveau demandé, ou `None` si elle est entièrement transparente.
    pub fn tile(&self, doc: &Document, level: u8, coord: TileCoord) -> Option<Arc<Tile>> {
        if level > Self::max_level(doc) {
            return None;
        }
        if let Some(hit) = lock(&self.caches.composite).get(&(level, coord)) {
            return hit.clone();
        }
        let generation = self.generation.load(Ordering::Acquire);
        let pass = Pass { renderer: self, doc, level, coord, generation, include: None, opaque: None, replace: None, shift: (0, 0) };
        let tile = finish_tile(doc.color_mode(), level, pass.run());
        if self.generation.load(Ordering::Acquire) == generation {
            lock(&self.caches.composite).insert((level, coord), tile.clone());
            self.trim();
        }
        tile
    }

    /// Tuile composée sans passer par le cache des tuiles composées (qui ne garde rien d'elle) : pour les parcours
    /// d'un seul tenant (export, aplat), où chaque tuile n'est demandée qu'une fois. Les caches par calque
    /// (réductions, effets) servent toujours, c'est eux qui évitent de recalculer les effets à chaque tuile.
    pub fn tile_uncached(&self, doc: &Document, level: u8, coord: TileCoord) -> Option<Arc<Tile>> {
        if level > Self::max_level(doc) {
            return None;
        }
        if let Some(hit) = lock(&self.caches.composite).get(&(level, coord)) {
            return hit.clone();
        }
        let generation = self.generation.load(Ordering::Acquire);
        let pass = Pass { renderer: self, doc, level, coord, generation, include: None, opaque: None, replace: None, shift: (0, 0) };
        finish_tile(doc.color_mode(), level, pass.run())
    }

    /// Mémoire occupée par les caches de ce document, en octets (voir [`Caches::bytes`]).
    pub fn cache_bytes(&self) -> u64 {
        self.caches.bytes()
    }

    /// Fait respecter au cache son budget du moment (voir [`Caches::trim`]).
    pub fn trim(&self) {
        self.caches.trim();
    }

    /// Calque tel que la composition le fond : ses pixels et ses effets, séparés comme dans `Pass::composite_list`
    /// (ce que la carte graphique de l'interface reçoit pour composer elle-même). Sans effet actif : ses pixels
    /// seuls. `None` : le calque n'a pas de pixels (groupe, réglage).
    pub fn styled_layer(&self, layer: &Layer) -> Option<Arc<StyledStores>> {
        let pixels = layer.pixels()?;
        if !crate::effects::any_enabled(&layer.effects) {
            return Some(Arc::new(StyledStores { main: Arc::clone(pixels), under: Vec::new(), over: Vec::new() }));
        }
        Some(self.styled_stores(layer.id, pixels, &layer.effects))
    }

    /// Calque stylé en cache, recalculé si les pixels ou les effets ont changé. Le calcul se fait
    /// sous le verrou : plusieurs tuiles demandées en même temps n'en déclenchent qu'un seul.
    fn styled_store(&self, id: LayerId, pixels: &Arc<TileStore>, effects: &[LayerEffect]) -> Arc<TileStore> {
        Arc::clone(&self.styled_stores(id, pixels, effects).main)
    }

    fn styled_stores(&self, id: LayerId, pixels: &Arc<TileStore>, effects: &[LayerEffect]) -> Arc<StyledStores> {
        let mut cache = lock(&self.styled);
        let mut previous = None;
        if let Some((known, for_effects, stores)) = cache.get(&id) {
            // Même version : mêmes pixels (un état relu du disque de travail a de nouveaux stockages, pas de nouveaux pixels).
            if (Arc::ptr_eq(known, pixels) || known.version() == pixels.version()) && for_effects == effects {
                return Arc::clone(stores);
            }
            if for_effects == effects {
                previous = Some((Arc::clone(known), Arc::clone(stores)));
            }
        }
        // Mêmes effets, quelques tuiles changées (trait de pinceau en cours) : les effets ne sont recalculés
        // qu'autour d'elles, et le reste du rendu précédent est gardé. Sinon, tout le calque. En cas d'échec
        // (calque démesuré), le calque s'affiche sans ses effets plutôt que pas du tout.
        let local = previous.and_then(|(known, stores)| restyled_locally(&known, pixels, effects, &stores));
        let stores = Arc::new(match local {
            Some(stores) => stores,
            None => match crate::effects::styled_parts(pixels, effects) {
                Ok(parts) => StyledStores {
                    main: Arc::new(parts.main),
                    under: parts.under.into_iter().map(|(mode, store)| (mode, Arc::new(store))).collect(),
                    over: parts.over.into_iter().map(|(mode, store)| (mode, Arc::new(store))).collect(),
                },
                Err(_) => StyledStores { main: Arc::clone(pixels), under: Vec::new(), over: Vec::new() },
            },
        });
        cache.insert(id, (Arc::clone(pixels), effects.to_vec(), Arc::clone(&stores)));
        // Les réductions déjà calculées pour ce calque (et ses effets du dessous) viennent de l'ancien rendu.
        lock(&self.caches.pixel_mips).retain(|(layer, _, _), _| (*layer & !(UNDERLAY | (0xFF << UNDERLAY_SHIFT))) != id || (*layer & RAW_PIXELS) != 0);
        stores
    }

    /// Composition d'une tuile où les pixels du calque `id` sont remplacés par `store`, déjà à l'échelle du
    /// niveau demandé et en repère document. Sert à l'aperçu d'un filtre AVEC tous les calques : rien n'est
    /// mis en cache (le remplaçant change à chaque réglage), mais les réductions des autres calques le sont.
    pub fn tile_replacing(&self, doc: &Document, level: u8, coord: TileCoord, id: LayerId, store: &TileStore) -> Option<Arc<Tile>> {
        let generation = self.generation.load(Ordering::Acquire);
        let level = level.min(Self::max_level(doc));
        finish_tile(doc.color_mode(), level, Pass { renderer: self, doc, level, coord, generation, include: None, opaque: None, replace: Some((id, store)), shift: (0, 0) }.run())
    }

    /// Composition restreinte aux calques de pixels retenus par `include`, les autres étant
    /// ignorés. Sert à l'aperçu de déplacement : l'interface recompose elle-même « dessous »,
    /// « déplacé » et « dessus ». Non mis en cache : chaque tuile n'est demandée qu'une fois par geste.
    pub fn tile_filtered(
        &self,
        doc: &Document,
        level: u8,
        coord: TileCoord,
        include: &(dyn Fn(LayerId) -> bool + Sync),
        opaque: Option<LayerId>,
    ) -> Option<Arc<Tile>> {
        self.tile_part_replacing(doc, level, coord, include, opaque, None)
    }

    /// Comme `tile_filtered`, mais les pixels d'un calque peuvent être remplacés par d'autres, déjà à
    /// l'échelle du niveau et en repère document. Sert à l'aperçu d'un filtre sur la zone de travail :
    /// l'interface affiche « dessous », le calque **filtré**, puis « dessus », sans rien écrire dans
    /// le document. Rien n'est mis en cache : le remplaçant change à chaque réglage.
    pub fn tile_part_replacing(
        &self,
        doc: &Document,
        level: u8,
        coord: TileCoord,
        include: &(dyn Fn(LayerId) -> bool + Sync),
        opaque: Option<LayerId>,
        replace: Option<(LayerId, &TileStore)>,
    ) -> Option<Arc<Tile>> {
        if level > Self::max_level(doc) {
            return None;
        }
        let generation = self.generation.load(Ordering::Acquire);
        Pass { renderer: self, doc, level, coord, generation, include: Some(include), opaque, replace, shift: (0, 0) }.run()
    }

    /// Tuiles du document à recomposer sans que les pixels d'aucun calque aient changé
    /// (un calque a été déplacé) : seules les tuiles composées en cache sont concernées.
    pub fn invalidate_doc_tiles(&self, doc: &Document, coords: &[TileCoord]) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let max_level = Self::max_level(doc);
        let mut composite = lock(&self.caches.composite);
        for coord in coords {
            for level in 0..=max_level {
                composite.remove(&(level, TileCoord::new(coord.x >> level, coord.y >> level)));
            }
        }
    }

    /// Propriétés ou structure modifiées : toutes les tuiles composées sont à refaire.
    pub fn invalidate_composites(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        lock(&self.caches.composite).clear();
    }

    /// Pixels (ou masque) d'un calque modifiés sur ses tuiles **locales** `coords` de
    /// niveau 0. Retourne les tuiles du **document** à recharger par l'affichage.
    pub fn invalidate_tiles(&self, doc: &Document, layer: LayerId, mask: bool, coords: &[TileCoord]) -> Vec<TileCoord> {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let max_level = Self::max_level(doc);
        let offset = doc.layer(layer).map_or((0, 0), |l| l.offset);
        let size = TILE_SIZE as i32;
        let (tiles_x, tiles_y) = (tiles_for(doc.width()) as i32, tiles_for(doc.height()) as i32);

        let mut affected = Vec::new();
        for coord in coords {
            for level in 0..=max_level {
                let at = TileCoord::new(coord.x >> level, coord.y >> level);
                if mask {
                    lock(&self.caches.mask_mips).remove(&(layer, level, at));
                } else {
                    let mut mips = lock(&self.caches.pixel_mips);
                    mips.remove(&(layer, level, at));
                    mips.remove(&(layer | RAW_PIXELS, level, at));
                    for rank in (0..8u64).chain(OVER_RANK..OVER_RANK + 8) {
                        mips.remove(&(layer | UNDERLAY | (rank << UNDERLAY_SHIFT), level, at));
                    }
                }
            }
            // Une tuile locale décalée recouvre jusqu'à quatre tuiles du document.
            let (x0, y0) = (coord.x * size + offset.0, coord.y * size + offset.1);
            let (first, _, _) = TileCoord::of_pixel(x0, y0);
            let (last, _, _) = TileCoord::of_pixel(x0 + size - 1, y0 + size - 1);
            for ty in first.y.max(0)..=last.y.min(tiles_y - 1) {
                for tx in first.x.max(0)..=last.x.min(tiles_x - 1) {
                    affected.push(TileCoord::new(tx, ty));
                }
            }
        }
        affected.sort_unstable();
        affected.dedup();
        let mut composite = lock(&self.caches.composite);
        for coord in &affected {
            for level in 0..=max_level {
                composite.remove(&(level, TileCoord::new(coord.x >> level, coord.y >> level)));
            }
        }
        affected
    }

    /// Couche `index` de l'aperçu de déplacement `layout` (voir `move_preview`), tuile `coord` du document au niveau
    /// `level`, en RGBA prémultiplié. `own` : pour la couche déplacée, l'opacité et le masque du groupe (ou du
    /// calque) déplacé seul sont compris ; sans eux (`own` faux), c'est le contenu aplati du groupe, que la carte
    /// graphique fond elle-même avec l'opacité et le masque du groupe (voir `GpuPlanOptions.collapsed`). Couche de
    /// réglage : la couverture de son masque, dans les quatre canaux.
    pub fn move_slice_tile(&self, doc: &Document, layout: &crate::move_preview::MoveLayout, index: usize, level: u8, coord: TileCoord, own: bool) -> Option<Arc<Tile>> {
        use crate::move_preview::SliceKind;
        let slice = layout.slices.get(index)?;
        if level > Self::max_level(doc) {
            return None;
        }
        let generation = self.generation.load(Ordering::Acquire);
        // « Dessous » entièrement caché par un calque opaque fixe du dessus : rien à composer (ce qui bouge passe
        // aussi sous ce calque, donc reste caché).
        if slice.kind == SliceKind::Below && self.covered(doc, &layout.occluders, level, coord) {
            return None;
        }
        // Couche fixe déjà composée pour ce contenu : reprise telle quelle.
        let fixed = !slice.moving && slice.kind != SliceKind::Adjust && slice.fingerprint != 0;
        let key = (slice.fingerprint, level, coord);
        if fixed {
            if let Some(hit) = lock(&self.caches.move_slices).get(&key) {
                return hit.clone();
            }
        }
        let pass = Pass { renderer: self, doc, level, coord, generation, include: None, opaque: None, replace: None, shift: (0, 0) };
        match slice.kind {
            SliceKind::Adjust => {
                let layer = doc.layer(slice.layer?).ok()?;
                Some(Arc::new(match pass.mask_factor(layer) {
                    Coverage::Uniform(v) => Tile::filled([(v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8; 4]),
                    Coverage::Mask(mask) => {
                        let mut tile = Tile::transparent();
                        for (px, v) in tile.bytes_mut().chunks_exact_mut(4).zip(mask.bytes()) {
                            px.copy_from_slice(&[*v; 4]);
                        }
                        tile
                    }
                }))
            }
            SliceKind::Moving if slice.layer.and_then(|id| doc.layer(id).ok()).is_some_and(Layer::is_group) => {
                let group = doc.layer(slice.layer?).ok()?;
                if !group.props.visible {
                    return None;
                }
                // Contenu du groupe composé à part : celui du cache des groupes quand il s'y prête (empreinte
                // indépendante de la position : valable tout le geste, et déjà prêt pour le rendu du relâchement).
                let content = match pass.flat_group(group) {
                    Some(tile) => tile,
                    None => {
                        let mut builder = Builder::new();
                        pass.composite_children(group, 0, &mut builder);
                        builder.finish(coord).run_on_cpu()
                    }
                };
                if own { attenuate(content, pass.opacity(group), &pass.mask_factor(group)) } else { content }
            }
            _ => {
                let set: std::collections::HashSet<LayerId> = slice.leaves.iter().copied().collect();
                let include = |id: LayerId| set.contains(&id);
                let tile = Pass { include: Some(&include), ..pass }.run();
                if fixed && self.generation.load(Ordering::Acquire) == generation {
                    lock(&self.caches.move_slices).insert(key, tile.clone());
                    self.caches.trim();
                }
                tile
            }
        }
    }

    /// Vrai si la tuile `coord` du niveau `level` (limitée au document) est entièrement recouverte par l'un des
    /// calques `occluders` (calques opaques fixes du dessus, voir `MoveLayout::occluders`) : chacune de ses tuiles
    /// locales sur cette zone est présente et opaque. Prudent : une zone couverte par plusieurs calques à la fois,
    /// ou par le bord d'un calque, n'est pas reconnue (elle est simplement composée).
    fn covered(&self, doc: &Document, occluders: &[LayerId], level: u8, coord: TileCoord) -> bool {
        if occluders.is_empty() {
            return false;
        }
        let span = (TILE_SIZE as i32) << level;
        let (x0, y0) = (coord.x * span, coord.y * span);
        let (x1, y1) = ((x0 + span).min(doc.width() as i32), (y0 + span).min(doc.height() as i32));
        if x0 >= x1 || y0 >= y1 {
            return false;
        }
        let size = TILE_SIZE as i32;
        occluders.iter().any(|id| {
            let Ok(layer) = doc.layer(*id) else { return false };
            let Some(store) = layer.pixels() else { return false };
            let (ox, oy) = layer.offset;
            let (tx0, ty0) = ((x0 - ox).div_euclid(size), (y0 - oy).div_euclid(size));
            let (tx1, ty1) = ((x1 - 1 - ox).div_euclid(size), (y1 - 1 - oy).div_euclid(size));
            // Une grande tuile de réduction couvre beaucoup de tuiles locales : au-delà de 64, on compose.
            if (tx1 - tx0 + 1) * (ty1 - ty0 + 1) > 64 {
                return false;
            }
            let version = store.version();
            (ty0..=ty1).all(|ty| {
                (tx0..=tx1).all(|tx| {
                    let local = TileCoord::new(tx, ty);
                    if let Some(known) = lock(&self.caches.opaque_tiles).get(&(version, local)) {
                        return *known;
                    }
                    let opaque = store.get(local).is_some_and(|t| t.is_opaque());
                    let mut memo = lock(&self.caches.opaque_tiles);
                    if memo.len() > 65_536 {
                        memo.clear();
                    }
                    memo.insert((version, local), opaque);
                    opaque
                })
            })
        })
    }

    /// Image de la couche `index` de l'aperçu de déplacement sur le rectangle `(x, y, largeur, hauteur)` du niveau
    /// `level` (pixels de ce niveau, repère document), RGBA prémultiplié ligne par ligne. Tuiles composées en
    /// parallèle.
    pub fn move_slice_region(&self, doc: &Document, layout: &crate::move_preview::MoveLayout, index: usize, level: u8, (x, y, w, h): (i32, i32, u32, u32), own: bool) -> Vec<u8> {
        use rayon::prelude::*;
        let mut out = vec![0u8; w as usize * h as usize * 4];
        if w == 0 || h == 0 {
            return out;
        }
        let size = TILE_SIZE as i32;
        let level = level.min(Self::max_level(doc));
        // Pas de limite au document : la couche déplacée peut venir de dehors (contenu qui déborde du cadre).
        let (x1, y1) = (x + w as i32, y + h as i32);
        let (tx0, ty0) = (x.div_euclid(size), y.div_euclid(size));
        let (tx1, ty1) = ((x1 - 1).div_euclid(size), (y1 - 1).div_euclid(size));
        let coords: Vec<TileCoord> = (ty0..=ty1).flat_map(|ty| (tx0..=tx1).map(move |tx| TileCoord::new(tx, ty))).collect();
        let tiles: Vec<(TileCoord, Option<Arc<Tile>>)> = coords.into_par_iter().map(|c| (c, self.move_slice_tile(doc, layout, index, level, c, own))).collect();
        for (coord, tile) in tiles {
            let Some(tile) = tile else { continue };
            let (ox, oy) = coord.origin();
            let (ix0, iy0, ix1, iy1) = (x.max(ox), y.max(oy), x1.min(ox + size), y1.min(oy + size));
            if ix0 >= ix1 || iy0 >= iy1 {
                continue;
            }
            let width = (ix1 - ix0) as usize * 4;
            for row in iy0..iy1 {
                let src = ((row - oy) * size + (ix0 - ox)) as usize * 4;
                let dst = ((row - y) as usize * w as usize + (ix0 - x) as usize) * 4;
                out[dst..dst + width].copy_from_slice(&tile.bytes()[src..src + width]);
            }
        }
        out
    }

    /// Tuile de la sélection au niveau demandé, pour l'affichage du contour animé.
    /// `None` = uniformément égale au fond de la sélection.
    pub fn selection_tile(&self, doc: &Document, level: u8, coord: TileCoord) -> Option<Arc<MaskTile>> {
        let selection = doc.selection()?;
        let generation = self.generation.load(Ordering::Acquire);
        mip(&self.caches.mask_mips, &self.generation, generation, selection, SELECTION_ID, level.min(Self::max_level(doc)), coord)
    }

    /// Calque apparu ou disparu : ses mips en cache ne sont plus fiables.
    pub fn forget_layer(&self, layer: LayerId) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        lock(&self.styled).remove(&layer);
        lock(&self.caches.pixel_mips).retain(|(id, _, _), _| *id != layer && *id != (layer | RAW_PIXELS));
        lock(&self.caches.mask_mips).retain(|(id, _, _), _| *id != layer);
        lock(&self.caches.groups).retain(|(id, _, _, _), _| *id != layer);
    }

    /// Miniature d'un calque (ou de son masque) : RGBA à alpha droit, au plus 256 px de côté.
    pub fn thumbnail(&self, doc: &Document, layer: LayerId, mask: bool) -> Option<(u32, u32, Vec<u8>)> {
        let layer = doc.layer(layer).ok()?;
        let level = Self::max_level(doc);
        let (w, h) = ((doc.width() >> level).max(1), (doc.height() >> level).max(1));
        let generation = self.generation.load(Ordering::Acquire);
        let origin = TileCoord::new(0, 0);
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        // La miniature est cadrée sur le document, comme dans Photoshop : le calque y apparaît à sa position.
        if mask {
            let store = &layer.mask.as_ref()?.pixels;
            let tile = gather(&self.caches.mask_mips, &self.generation, generation, store, layer.id, layer.offset, level, origin);
            let background = store.background()[0];
            for y in 0..h {
                for x in 0..w {
                    let v = tile.as_ref().map_or(background, |t| t.pixel(x, y)[0]);
                    out.extend_from_slice(&[v, v, v, 255]);
                }
            }
        } else {
            let store = layer.pixels()?;
            let tile = gather(&self.caches.pixel_mips, &self.generation, generation, store, layer.id, layer.offset, level, origin);
            // Sans tuile stockée, le calque vaut partout son fond (un remplissage uniforme n'a que lui) : la miniature
            // était vide (damier) pour un fond de plan blanc ou un remplissage noir relus d'une copie de secours.
            let background = unpremultiply(store.background());
            for y in 0..h {
                for x in 0..w {
                    out.extend_from_slice(&tile.as_ref().map_or(background, |t| unpremultiply(t.pixel(x, y))));
                }
            }
        }
        Some((w, h, out))
    }

    /// Miniature d'un calque cadrée comme dans le panneau : sur son plan de travail s'il est dans un plan (le calque
    /// lui-même, son plus proche parent qui est un plan), sinon sur le document. Le masque d'un plan reste cadré sur le
    /// document (il découpe tout l'espace de travail).
    pub fn board_thumbnail(&self, doc: &Document, layer: LayerId, mask: bool) -> Option<(u32, u32, Vec<u8>)> {
        let (full_w, full_h, full) = self.thumbnail(doc, layer, mask)?;
        let own = doc.layer(layer).ok()?;
        let mut current = Some(own);
        let mut board = None;
        while let Some(l) = current {
            if let Some(b) = &l.artboard {
                board = Some(b.clone());
                break;
            }
            current = l.parent.and_then(|p| doc.layer(p).ok());
        }
        let Some(board) = board.filter(|_| !(mask && own.artboard.is_some())) else { return Some((full_w, full_h, full)) };
        // Même réduction que la miniature entière : un pixel de miniature couvre `2^niveau` pixels du document.
        let step = f64::from(1u32 << Self::max_level(doc));
        let x0 = ((f64::from(board.x) / step).floor().max(0.0) as u32).min(full_w - 1);
        let y0 = ((f64::from(board.y) / step).floor().max(0.0) as u32).min(full_h - 1);
        let x1 = ((f64::from(board.x + board.width as i32) / step).ceil() as u32).clamp(x0 + 1, full_w);
        let y1 = ((f64::from(board.y + board.height as i32) / step).ceil() as u32).clamp(y0 + 1, full_h);
        let (w, h) = (x1 - x0, y1 - y0);
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in y0..y1 {
            let start = ((y * full_w + x0) * 4) as usize;
            out.extend_from_slice(&full[start..start + (w * 4) as usize]);
        }
        Some((w, h, out))
    }
}

/// Tuile d'un calque au niveau demandé. `None` = uniformément égale au fond du stockage.
fn mip<const C: usize>(
    cache: &MipCache<C>,
    current: &AtomicU64,
    generation: u64,
    store: &Store<C>,
    layer: LayerId,
    level: u8,
    coord: TileCoord,
) -> Option<Arc<TileBuf<C>>> {
    if level == 0 {
        return store.get(coord).cloned();
    }
    if let Some(hit) = lock(cache).get(&(layer, level, coord)) {
        return hit.clone();
    }
    let children = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| {
        mip(cache, current, generation, store, layer, level - 1, TileCoord::new(coord.x * 2 + dx, coord.y * 2 + dy))
    });
    let tile = children.iter().any(Option::is_some).then(|| {
        let half = TILE_SIZE / 2;
        let mut out = TileBuf::filled(store.background());
        for (index, child) in children.iter().enumerate() {
            let Some(child) = child else { continue };
            let (ox, oy) = ((index as u32 % 2) * half, (index as u32 / 2) * half);
            for y in 0..half {
                for x in 0..half {
                    out.set_pixel(ox + x, oy + y, box_filter(child, x * 2, y * 2));
                }
            }
        }
        Arc::new(out)
    });
    if current.load(Ordering::Acquire) == generation {
        lock(cache).insert((layer, level, coord), tile.clone());
    }
    tile
}

/// Fenêtre d'un calque correspondant à la tuile `coord` du **document** au niveau
/// demandé, compte tenu de la position du calque. `None` = uniformément égale au fond.
#[allow(clippy::too_many_arguments)]
fn gather<const C: usize>(
    cache: &MipCache<C>,
    current: &AtomicU64,
    generation: u64,
    store: &Store<C>,
    layer: LayerId,
    offset: (i32, i32),
    level: u8,
    coord: TileCoord,
) -> Option<Arc<TileBuf<C>>> {
    let size = TILE_SIZE as i32;
    let scale = 1i32 << level;
    // Position arrondie au pixel de mip le plus proche (exacte au niveau 0).
    let shift = |v: i32| (v + scale / 2).div_euclid(scale);
    let (x0, y0) = (coord.x * size - shift(offset.0), coord.y * size - shift(offset.1));
    let (first, fx, fy) = TileCoord::of_pixel(x0, y0);
    if fx == 0 && fy == 0 {
        // Grilles alignées (cas d'un calque jamais déplacé) : la tuile est renvoyée sans copie.
        return mip(cache, current, generation, store, layer, level, first);
    }
    let mut out: Option<TileBuf<C>> = None;
    for ty in first.y..=first.y + 1 {
        for tx in first.x..=first.x + 1 {
            let Some(source) = mip(cache, current, generation, store, layer, level, TileCoord::new(tx, ty)) else { continue };
            let (sx, sy) = (tx * size, ty * size);
            let (ix0, iy0) = (x0.max(sx), y0.max(sy));
            let (ix1, iy1) = ((x0 + size).min(sx + size), (y0 + size).min(sy + size));
            if ix0 >= ix1 || iy0 >= iy1 {
                continue;
            }
            let width = (ix1 - ix0) as usize * C;
            let target = out.get_or_insert_with(|| TileBuf::filled(store.background()));
            for y in iy0..iy1 {
                let src = ((y - sy) * size + (ix0 - sx)) as usize * C;
                let dst = ((y - y0) * size + (ix0 - x0)) as usize * C;
                target.bytes_mut()[dst..dst + width].copy_from_slice(&source.bytes()[src..src + width]);
            }
        }
    }
    out.map(Arc::new)
}

/// Tuile atténuée par une opacité et un masque (alpha prémultiplié : les quatre canaux), comme la fusion du moteur
/// les applique à la source. Inchangée à pleine opacité sans masque ; `None` si rien ne reste.
fn attenuate(tile: Option<Arc<Tile>>, opacity: f32, mask: &Coverage) -> Option<Arc<Tile>> {
    let tile = tile?;
    if opacity >= 1.0 && matches!(mask, Coverage::Uniform(v) if *v >= 1.0) {
        return Some(tile);
    }
    let mut out = Tile::transparent();
    let mut any = false;
    for (i, (dst, src)) in out.bytes_mut().chunks_exact_mut(4).zip(tile.bytes().chunks_exact(4)).enumerate() {
        let k = (opacity * mask.at(i)).clamp(0.0, 1.0);
        for c in 0..4 {
            dst[c] = (f32::from(src[c]) * k + 0.5) as u8;
        }
        any |= dst[3] > 0;
    }
    any.then(|| Arc::new(out))
}

/// Moyenne 2x2. En alpha prémultiplié, la moyenne directe des canaux est correcte.
#[inline]
fn box_filter<const C: usize>(tile: &TileBuf<C>, x: u32, y: u32) -> [u8; C] {
    let (a, b, c, d) = (tile.pixel(x, y), tile.pixel(x + 1, y), tile.pixel(x, y + 1), tile.pixel(x + 1, y + 1));
    let mut out = [0; C];
    for i in 0..C {
        out[i] = ((u16::from(a[i]) + u16::from(b[i]) + u16::from(c[i]) + u16::from(d[i]) + 2) / 4) as u8;
    }
    out
}

/// Composition d'une tuile donnée.
struct Pass<'a> {
    renderer: &'a TileRenderer,
    doc: &'a Document,
    level: u8,
    coord: TileCoord,
    generation: u64,
    /// Filtre des calques de pixels pris en compte (`None` = tous).
    include: Option<&'a (dyn Fn(LayerId) -> bool + Sync)>,
    /// Pixels de remplacement pour un calque (aperçu de filtre), à l'échelle du niveau, en repère document.
    replace: Option<(LayerId, &'a TileStore)>,
    /// Calque composé à pleine opacité, quelle que soit la sienne : l'interface applique
    /// elle-même l'opacité pendant qu'on glisse le curseur, sans rien redemander au moteur.
    opaque: Option<LayerId>,
    /// Origine du repère de composition, en pixels du document : (0, 0) pour le document ; la position d'un
    /// groupe quand on compose son contenu dans son propre repère (voir `group_tile`).
    shift: (i32, i32),
}

impl Pass<'_> {
    fn run(&self) -> Option<Arc<Tile>> {
        self.program().run_on_cpu()
    }

    /// Position d'un calque dans le repère de cette composition.
    fn at(&self, offset: (i32, i32)) -> (i32, i32) {
        (offset.0 - self.shift.0, offset.1 - self.shift.1)
    }

    /// Contenu d'un groupe déjà composé dans son propre repère, ramené à la tuile en cours : `Some(tuile)`
    /// (ou `Some(None)` si rien n'y est dessiné). `None` quand le groupe ne s'y prête pas (voir
    /// `flattenable`) : il est alors composé par [`Self::composite_children`], qui fige ce qui peut l'être.
    fn flat_group(&self, group: &Layer) -> Option<Option<Arc<Tile>>> {
        if self.include.is_some() || self.replace.is_some() || self.opaque.is_some() || !self.flattenable(group) {
            return None;
        }
        Some(self.flat_run(group, group.children(), group.id))
    }

    /// Série `ids` de calques du groupe (tout son contenu, ou une partie, voir [`Self::cacheable_run`]) composée à
    /// part dans le repère du GROUPE, sur fond transparent, et ramenée à la tuile en cours.
    ///
    /// Le contenu figé est rangé par tuiles du repère du groupe, avec l'empreinte de ce qui le compose (calques,
    /// positions relatives, versions des pixels et des masques, effets, propriétés). Déplacer le groupe entier ne
    /// change rien à cette empreinte : les tuiles restent bonnes et sont seulement replacées, au lieu de recomposer
    /// chaque calque du groupe à chaque pas du déplacement. Quelques versions sont gardées par tuile : un Ctrl+Z
    /// retrouve celle d'avant. `first` distingue les séries d'un même groupe dans le cache.
    fn flat_run(&self, group: &Layer, ids: &[LayerId], first: LayerId) -> Option<Arc<Tile>> {
        let mut print = Vec::new();
        Self::fingerprint(self.doc, ids, group.offset, &mut print);
        // Copie des effets seulement pour une entrée nouvelle du cache, partagée entre les tuiles de ce passage.
        let owned: std::cell::OnceCell<Arc<Vec<Print>>> = std::cell::OnceCell::new();
        let r = self.renderer;
        let size = TILE_SIZE as i32;
        let scale = 1i32 << self.level;
        let round = |v: i32| (v + scale / 2).div_euclid(scale);
        let (x0, y0) = (self.coord.x * size - round(self.at(group.offset).0), self.coord.y * size - round(self.at(group.offset).1));
        let (first_tile, fx, fy) = TileCoord::of_pixel(x0, y0);
        let fetch = |coord: TileCoord| -> Option<Arc<Tile>> {
            let key = (group.id, first, self.level, coord);
            if let Some(tile) = lock(&r.caches.groups).find(&key, &print) {
                return tile;
            }
            let inner = Pass { coord, shift: group.offset, ..*self };
            let mut builder = Builder::new();
            inner.composite_list(ids, 0, &mut builder);
            let tile = builder.finish(coord).run_on_cpu();
            let owned = owned.get_or_init(|| Arc::new(print.iter().map(PrintRef::to_owned).collect()));
            lock(&r.caches.groups).push(key, Arc::clone(owned), tile.clone());
            r.trim();
            tile
        };
        if fx == 0 && fy == 0 {
            return fetch(first_tile);
        }
        let mut out: Option<Tile> = None;
        for ty in first_tile.y..=first_tile.y + 1 {
            for tx in first_tile.x..=first_tile.x + 1 {
                let Some(source) = fetch(TileCoord::new(tx, ty)) else { continue };
                let (sx, sy) = (tx * size, ty * size);
                let (ix0, iy0) = (x0.max(sx), y0.max(sy));
                let (ix1, iy1) = ((x0 + size).min(sx + size), (y0 + size).min(sy + size));
                if ix0 >= ix1 || iy0 >= iy1 {
                    continue;
                }
                let width = (ix1 - ix0) as usize * 4;
                let target = out.get_or_insert_with(|| TileBuf::filled([0; 4]));
                for y in iy0..iy1 {
                    let src = ((y - sy) * size + (ix0 - sx)) as usize * 4;
                    let dst = ((y - y0) * size + (ix0 - x0)) as usize * 4;
                    target.bytes_mut()[dst..dst + width].copy_from_slice(&source.bytes()[src..src + width]);
                }
            }
        }
        out.map(Arc::new)
    }

    /// Un groupe peut être composé à part puis posé d'un bloc, sans rien changer à l'image, si tout ce qu'il
    /// contient se fond normalement (voir [`Self::blendable`]) et qu'il y a au moins deux calques à composer (sinon
    /// il n'y a rien à gagner). Ses propres opacité et masque sont appliqués après : ils ne comptent pas.
    fn flattenable(&self, group: &Layer) -> bool {
        self.blendable(group.children()).is_some_and(|drawn| drawn >= 2)
    }

    /// Nombre de calques à dessiner dans `ids` (sous-groupes compris) si tout s'y fond normalement : pas de réglage
    /// (il corrigerait l'image dessous), pas de mode de fusion autre que Normal, pas d'effet fondu dans l'image
    /// dessous, des sous-groupes en Normal ou en Transfert sans opacité ni masque (sinon ils mélangent avec ce qui
    /// est sous eux), et pas de calque de remplissage : il couvre toujours le document, son rendu dans le repère du
    /// groupe change donc à chaque déplacement (on fige alors le reste du groupe, voir [`Self::cacheable_run`]).
    /// `None` sinon. Un sous-groupe d'un seul calque ne l'empêche pas (auparavant, il rendait tout le groupe
    /// inaplatissable).
    fn blendable(&self, ids: &[LayerId]) -> Option<usize> {
        let mut drawn = 0usize;
        let mut stack: Vec<LayerId> = ids.to_vec();
        while let Some(id) = stack.pop() {
            let Some(layer) = self.layer(id) else { continue };
            if !layer.props.visible {
                continue;
            }
            if layer.is_group() {
                let plain = self.opacity(layer) >= 1.0 && layer.mask.as_ref().is_none_or(|m| !m.enabled);
                if layer.props.range().is_some() {
                    return None;
                }
                match layer.props.blend {
                    BlendMode::Normal => {}
                    BlendMode::PassThrough if plain => {}
                    _ => return None,
                }
                stack.extend_from_slice(layer.children());
                continue;
            }
            if layer.adjustment().is_some() || layer.props.blend != BlendMode::Normal || layer.props.range().is_some() || layer.fill_content().is_some() {
                return None;
            }
            if layer.effects.iter().any(|e| e.enabled() && e.blend() != BlendMode::Normal) {
                return None;
            }
            drawn += 1;
        }
        Some(drawn)
    }

    /// La plus longue série de frères consécutifs de `ids` qui se fige (voir [`Self::blendable`]), avec au moins deux
    /// calques à dessiner : `(début, fin)`, fin exclue. Un calque écrêté reste avec sa base. Sert aux groupes qui
    /// ne se figent pas en entier — un calque de réglage au-dessus, un remplissage en fond, un calque en Produit :
    /// tout le reste du groupe est figé, seul ce calque-là est composé à chaque fois.
    fn cacheable_run(&self, ids: &[LayerId]) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize, usize)> = None;
        let mut run: Option<(usize, usize)> = None; // début, calques dessinés
        let close = |run: Option<(usize, usize)>, end: usize, best: &mut Option<(usize, usize, usize)>| {
            if let Some((start, drawn)) = run {
                if drawn >= 2 && best.is_none_or(|(_, _, d)| drawn > d) {
                    *best = Some((start, end, drawn));
                }
            }
        };
        let mut i = 0;
        while i < ids.len() {
            let mut end = i + 1;
            while end < ids.len() && self.layer(ids[end]).is_some_and(|l| l.props.clipped) {
                end += 1;
            }
            match self.blendable(&ids[i..end]) {
                Some(drawn) => run = Some(run.map_or((i, drawn), |(start, d)| (start, d + drawn))),
                None => {
                    close(run, i, &mut best);
                    run = None;
                }
            }
            i = end;
        }
        close(run, ids.len(), &mut best);
        best.map(|(start, end, _)| (start, end))
    }

    /// Compose les enfants d'un groupe dans `target`, en reprenant du cache la plus longue série qui se fige
    /// (voir [`Self::cacheable_run`]) : ce qui est dessous est composé, la série posée d'un bloc en mode Normal (la
    /// fusion normale est associative : même image), puis ce qui est dessus composé par-dessus.
    fn composite_children(&self, group: &Layer, target: u16, builder: &mut Builder) {
        let ids = group.children();
        let run = if self.include.is_some() || self.replace.is_some() || self.opaque.is_some() { None } else { self.cacheable_run(ids) };
        let Some((start, end)) = run else {
            self.composite_list(ids, target, builder);
            return;
        };
        self.composite_list(&ids[..start], target, builder);
        let key = if start == 0 && end == ids.len() { group.id } else { ids[start] };
        if let Some(tile) = self.flat_run(group, &ids[start..end], key) {
            builder.push(Op::Blend { target, source: Source::Tile(tile), mode: BlendMode::Normal, opacity: 1.0, mask: Coverage::Uniform(1.0), range: None });
        }
        self.composite_list(&ids[end..], target, builder);
    }

    /// Empreinte du contenu d'une série de calques, positions relatives à `origin` : deux empreintes égales donnent
    /// la même composition, où que soit le groupe.
    fn fingerprint<'d>(doc: &'d Document, ids: &[LayerId], origin: (i32, i32), out: &mut Vec<PrintRef<'d>>) {
        for id in ids {
            let Ok(layer) = doc.layer(*id) else { continue };
            let p = &layer.props;
            out.push(Print {
                id: layer.id,
                at: (layer.offset.0 - origin.0, layer.offset.1 - origin.1),
                visible: p.visible,
                blend: p.blend,
                opacity: p.opacity.to_bits(),
                fill: p.fill.to_bits(),
                clipped: p.clipped,
                range: p.range(),
                pixels: layer.pixels().map(|store| store.version()),
                mask: layer.mask.as_ref().map(|m| (m.pixels.version(), m.enabled)),
                effects: layer.effects.as_slice(),
                children: layer.children().len(),
            });
            if layer.is_group() {
                Self::fingerprint(doc, layer.children(), origin, out);
            }
        }
    }

    /// Traduit l'arbre des calques en programme de composition pour cette tuile.
    fn program(&self) -> Program {
        if let Some(shortcut) = self.single_plain_source() {
            return Program::Ready(shortcut);
        }
        let Ok(roots) = self.doc.children_of(None) else { return Program::Ready(None) };
        let mut builder = Builder::new();
        self.composite_list(roots, 0, &mut builder);
        builder.finish(self.coord)
    }

    fn pixels(&self, layer: &Layer) -> Option<Arc<Tile>> {
        if self.include.is_some_and(|include| !include(layer.id)) {
            return None;
        }
        if let Some((id, store)) = self.replace {
            if id == layer.id {
                return store.get(self.coord).cloned();
            }
        }
        let r = self.renderer;
        let pixels = layer.pixels()?;
        if !crate::effects::any_enabled(&layer.effects) {
            return gather(&r.caches.pixel_mips, &r.generation, self.generation, pixels, layer.id, self.at(layer.offset), self.level, self.coord);
        }
        // Calque à effets : le compositeur travaille sur le calque stylé, pas sur les pixels bruts.
        let styled = r.styled_store(layer.id, pixels, &layer.effects);
        gather(&r.caches.pixel_mips, &r.generation, self.generation, &styled, layer.id, self.at(layer.offset), self.level, self.coord)
    }

    /// Effets du dessous de ce calque qui se fondent dans l'image avec leur propre mode : (mode, tuile).
    fn underlays(&self, layer: &Layer) -> Vec<(BlendMode, Arc<Tile>)> {
        self.effect_parts(layer, false)
    }

    /// Effets rendus à part : ceux du dessous (`over` faux) ou ceux du dessus, en tuiles prêtes à fondre.
    fn effect_parts(&self, layer: &Layer, over: bool) -> Vec<(BlendMode, Arc<Tile>)> {
        if self.include.is_some_and(|include| !include(layer.id)) || !crate::effects::any_enabled(&layer.effects) {
            return Vec::new();
        }
        let r = self.renderer;
        let Some(pixels) = layer.pixels() else { return Vec::new() };
        let stores = r.styled_stores(layer.id, pixels, &layer.effects);
        let (list, first) = if over { (&stores.over, OVER_RANK) } else { (&stores.under, 0) };
        list
            .iter()
            .enumerate()
            .filter_map(|(rank, (mode, store))| {
                let key = layer.id | UNDERLAY | (((first + rank as LayerId) & 0xFF) << UNDERLAY_SHIFT);
                gather(&r.caches.pixel_mips, &r.generation, self.generation, store, key, self.at(layer.offset), self.level, self.coord).map(|tile| (*mode, tile))
            })
            .collect()
    }

    /// Pixels du calque SANS ses effets. Un calque écrêté se limite à la forme de sa base, pas à son
    /// ombre portée ni à sa lueur : c'est cet alpha-là qui sert de gabarit.
    fn raw_pixels(&self, layer: &Layer) -> Option<Arc<Tile>> {
        let r = self.renderer;
        gather(&r.caches.pixel_mips, &r.generation, self.generation, layer.pixels()?, layer.id | RAW_PIXELS, self.at(layer.offset), self.level, self.coord)
    }

    fn mask_factor(&self, layer: &Layer) -> Coverage {
        let Some(mask) = layer.mask.as_ref().filter(|m| m.enabled) else { return Coverage::Uniform(1.0) };
        let r = self.renderer;
        match gather(&r.caches.mask_mips, &r.generation, self.generation, &mask.pixels, layer.id, self.at(layer.offset), self.level, self.coord) {
            Some(tile) => Coverage::Mask(tile),
            None => Coverage::Uniform(f32::from(mask.pixels.background()[0]) / 255.0),
        }
    }

    /// Cas très fréquent (photo seule, ou un seul calque présent sur cette tuile) :
    /// la tuile du calque est renvoyée telle quelle, sans conversion ni copie.
    /// `None` = cas général ; `Some(None)` = tuile vide.
    fn single_plain_source(&self) -> Option<Option<Arc<Tile>>> {
        let mut found: Option<Arc<Tile>> = None;
        for (_, layer) in self.doc.walk_top_down() {
            let p = &layer.props;
            let plain = p.visible && self.opacity(layer) >= 1.0 && !p.clipped && layer.mask.is_none() && p.range().is_none();
            if layer.is_group() {
                if !(plain && matches!(p.blend, BlendMode::PassThrough | BlendMode::Normal)) {
                    return None;
                }
                continue;
            }
            // Un calque de réglage visible modifie l'image : pas de raccourci possible.
            if layer.adjustment().is_some() && p.visible {
                return None;
            }
            // Calque masqué : il ne compte pas, et ses pixels (réductions, effets) n'ont pas à être calculés.
            if !p.visible {
                continue;
            }
            let Some(tile) = self.pixels(layer) else { continue };
            if !plain || p.blend != BlendMode::Normal || found.is_some() {
                return None;
            }
            found = Some(tile);
        }
        Some(found)
    }

    fn opacity(&self, layer: &Layer) -> f32 {
        if self.opaque == Some(layer.id) {
            return 1.0;
        }
        layer.props.opacity * layer.props.fill
    }

    fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.doc.layer(id).ok()
    }

    /// Compose une liste de frères (du bas vers le haut) dans le tampon `target`.
    fn composite_list(&self, ids: &[LayerId], target: u16, builder: &mut Builder) {
        let mut i = 0;
        while i < ids.len() {
            let Some(base) = self.layer(ids[i]) else {
                i += 1;
                continue;
            };
            // Calques écrêtés sur `base` : ceux qui le suivent immédiatement.
            let mut end = i + 1;
            while end < ids.len() && self.layer(ids[end]).is_some_and(|l| l.props.clipped) {
                end += 1;
            }
            let clipped = &ids[i + 1..end];
            i = end;

            if !base.props.visible {
                continue;
            }
            if clipped.is_empty() && base.is_group() && base.props.blend == BlendMode::PassThrough {
                self.pass_through(base, target, builder);
                continue;
            }
            if base.adjustment().is_some() {
                // Réglage : corrige tout ce qui a été composé jusqu'ici dans ce groupe.
                self.adjust(base, target, builder);
                continue;
            }
            let Some((mut content, mut scratch)) = self.content(base, builder) else { continue };
            // Les calques écrêtés modifient le contenu de leur base : il lui faut alors un tampon à elle.
            if !clipped.is_empty() {
                if let Source::Tile(tile) = content {
                    let buffer = builder.take();
                    builder.push(Op::Load { target: buffer, source: tile });
                    (content, scratch) = (Source::Buffer(buffer), Some(buffer));
                }
            }
            // Base à effets : son contenu déborde de sa forme (ombre portée, lueur, contour extérieur). Les
            // calques écrêtés ne doivent couvrir que la forme elle-même : on relève son alpha brut.
            let shape = if clipped.is_empty() || !crate::effects::any_enabled(&base.effects) {
                Shape::BaseAlpha
            } else {
                self.raw_pixels(base).map_or(Shape::Empty, Shape::Raw)
            };
            if let Source::Buffer(into) = content {
                for id in clipped {
                    match self.layer(*id).filter(|l| l.props.visible) {
                        // Réglage écrêté : il ne corrige que le calque sur lequel il est écrêté.
                        Some(layer) if layer.adjustment().is_some() => self.adjust(layer, into, builder),
                        Some(layer) => self.clip_onto(layer, into, shape.clone(), builder),
                        None => {}
                    }
                }
            }
            // Ombre portée ou lueur externe dans un mode autre que Normal : elles se fondent d'abord dans l'image
            // sous le calque (avec l'opacité et le masque du calque, comme ses autres effets), puis le calque vient.
            let fuse = |parts: Vec<(BlendMode, Arc<Tile>)>, builder: &mut Builder| {
                for (mode, tile) in parts {
                    builder.push(Op::Blend { target, source: Source::Tile(tile), mode, opacity: base.props.opacity, mask: self.mask_factor(base), range: None });
                }
            };
            fuse(self.underlays(base), builder);
            builder.push(Op::Blend { target, source: content, mode: base.props.blend, opacity: self.opacity(base), mask: self.mask_factor(base), range: base.props.range() });
            // Effets du dessus dans un autre mode que Normal (contour en Incrustation…) : ils se fondent, là où ils
            // sont dessinés, dans tout ce qui est dessous — le calque et l'image.
            fuse(self.effect_parts(base, true), builder);
            if let Some(buffer) = scratch {
                builder.release(buffer);
            }
        }
    }

    /// Contenu d'un calque : sa tuile, ou pour un groupe un tampon où sa composition isolée est écrite
    /// (tampon à rendre au constructeur une fois utilisé).
    fn content(&self, layer: &Layer, builder: &mut Builder) -> Option<(Source, Option<u16>)> {
        match &layer.kind {
            LayerKind::Pixel { .. } | LayerKind::Text { .. } | LayerKind::Shape { .. } | LayerKind::Smart { .. } | LayerKind::Fill { .. } => self.pixels(layer).map(|tile| (Source::Tile(tile), None)),
            LayerKind::Adjustment { .. } => None,
            LayerKind::Group { .. } => {
                // Contenu figé du groupe (voir `flat_run`) : une seule tuile au lieu de toute sa pile.
                if let Some(flat) = self.flat_group(layer) {
                    return flat.map(|tile| (Source::Tile(tile), None));
                }
                let buffer = builder.take();
                builder.push(Op::Clear { target: buffer });
                self.composite_children(layer, buffer, builder);
                Some((Source::Buffer(buffer), Some(buffer)))
            }
        }
    }

    fn pass_through(&self, group: &Layer, target: u16, builder: &mut Builder) {
        let opacity = self.opacity(group);
        let mask = self.mask_factor(group);
        if opacity >= 1.0 && matches!(mask, Coverage::Uniform(v) if v >= 1.0) {
            // Groupe ordinaire de calques normaux : le composer d'abord à part puis le poser par-dessus donne
            // la même image (la fusion normale est associative), et ce contenu figé se réutilise tel quel
            // quand le groupe entier se déplace.
            match self.flat_group(group) {
                Some(Some(tile)) => builder.push(Op::Blend { target, source: Source::Tile(tile), mode: BlendMode::Normal, opacity: 1.0, mask: Coverage::Uniform(1.0), range: None }),
                Some(None) => {}
                None => self.composite_children(group, target, builder),
            }
            return;
        }
        // Opacité ou masque : le groupe interpole entre l'image d'avant et celle d'après.
        let before = builder.take();
        builder.push(Op::Copy { target: before, source: target });
        self.composite_children(group, target, builder);
        builder.mix(target, before, opacity, mask);
        builder.release(before);
    }

    /// Ajoute un calque de réglage : la couleur corrigée remplace la couleur d'origine à proportion de
    /// l'opacité et du masque du calque, à travers son mode de fusion. L'alpha ne change pas.
    fn adjust(&self, layer: &Layer, target: u16, builder: &mut Builder) {
        let Some(adjustment) = layer.adjustment() else { return };
        if self.include.is_some_and(|include| !include(layer.id)) {
            return; // composition partielle (aperçus) : ce réglage ne fait pas partie du lot
        }
        builder.push(Op::Adjust { target, adjustment: adjustment.clone(), mode: layer.props.blend, opacity: self.opacity(layer), mask: self.mask_factor(layer) });
    }

    /// Fusionne un calque écrêté avec la couleur de sa base, sans toucher à l'alpha de celle-ci.
    /// `shape` : forme de la base quand elle porte des effets (sinon, l'alpha de la base fait foi).
    fn clip_onto(&self, layer: &Layer, target: u16, shape: Shape, builder: &mut Builder) {
        let Some((source, scratch)) = self.content(layer, builder) else { return };
        builder.push(Op::Clip { target, source, mode: layer.props.blend, opacity: self.opacity(layer), mask: self.mask_factor(layer), shape });
        if let Some(buffer) = scratch {
            builder.release(buffer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les tables comptées suivent exactement leurs entrées remplies, quelles que soient les opérations.
    #[test]
    fn counted_tables_track_their_filled_entries() {
        let mut table: Counted<u32, Arc<Tile>> = Counted::default();
        let tile = || Some(Arc::new(Tile::transparent()));
        table.insert(1, tile());
        table.insert(2, None);
        table.insert(3, tile());
        table.insert(1, tile()); // remplacement : toujours une seule entrée
        table.insert(3, None); // vidée
        assert_eq!(table.filled(), 1);
        table.remove(&2);
        table.remove(&9);
        table.insert(4, tile());
        table.retain(|key, _| *key != 1);
        assert_eq!(table.filled(), 1);
        assert_eq!(table.len(), 2);
        table.clear();
        assert_eq!(table.filled(), 0);

        let mut groups = CountedGroups::default();
        let key = (1, 1, 0, TileCoord::new(0, 0));
        let probe = |n: usize| vec![PrintRef { id: n as LayerId, at: (0, 0), visible: true, blend: BlendMode::Normal, opacity: 255, fill: 255, clipped: false, range: None, pixels: None, mask: None, effects: &[], children: 0 }];
        let print = |n: usize| Arc::new(probe(n).iter().map(PrintRef::to_owned).collect::<Vec<Print>>());
        for n in 0..GROUP_VERSIONS + 2 {
            groups.push(key, print(n), if n == 1 { None } else { tile() });
        }
        let kept: usize = groups.get(&key).expect("versions").iter().filter(|(_, t)| t.is_some()).count();
        assert_eq!(groups.get(&key).expect("versions").len(), GROUP_VERSIONS);
        assert_eq!(groups.filled(), kept);
        assert!(groups.find(&key, &probe(GROUP_VERSIONS + 1)).is_some());
        assert!(groups.find(&key, &probe(0)).is_none(), "version trop ancienne, partie");
        groups.retain(|_, _| false);
        assert_eq!(groups.filled(), 0);
    }
    use crate::document::Background;
    use crate::plan::TILE_PIXELS;
    use crate::tiles::{Sampler, TileStore};

    const ORIGIN: TileCoord = TileCoord::new(0, 0);

    #[test]
    fn a_layer_thumbnail_is_framed_on_its_artboard() {
        use crate::document::Artboard;
        let mut d = Document::new("plans", 100, 100, 72.0).expect("document");
        let board = |x, y, w, h| Artboard { x, y, width: w, height: h, background: Some([255, 255, 255]), clip: true, preset: None, background_layer: None };
        let a = d.add_artboard("A", board(0, 0, 1200, 600)).expect("plan");
        let b = d.add_artboard("B", board(1300, 0, 400, 300)).expect("plan");
        d.fit_to_artboards().expect("ajuster");
        assert_eq!((d.width(), d.height()), (1700, 600));
        // Calque rouge à gauche du plan A, masque qui ne laisse voir que sa moitié gauche.
        let mut rgba = vec![0u8; 300 * 600 * 4];
        rgba.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&[255, 0, 0, 255]));
        let red = d.add_pixel_layer("Rouge", TileStore::from_premultiplied(300, 600, &rgba).expect("tampon"), Some(a), 1).expect("calque");
        d.add_mask(red, true).expect("masque");
        let renderer = TileRenderer::new();
        let level = TileRenderer::max_level(&d);
        let step = 1u32 << level;
        // Calque du plan A : cadré sur A (1200 × 600), le rouge occupe son quart gauche.
        let (w, h, px) = renderer.board_thumbnail(&d, red, false).expect("miniature");
        assert_eq!((w, h), (1200u32.div_ceil(step), 600u32.div_ceil(step)));
        assert_eq!(&px[0..4], &[255, 0, 0, 255]);
        assert_eq!(px[((w - 1) * 4 + 3) as usize], 0, "à droite du calque");
        let (_, _, mask) = renderer.board_thumbnail(&d, red, true).expect("masque");
        assert!(mask.chunks_exact(4).all(|p| p[0] == 255), "masque tout blanc");
        // Fond du plan B : cadré sur B, blanc partout, masque blanc.
        let bg = d.layer(b).expect("plan").artboard.as_ref().and_then(|x| x.background_layer).expect("fond");
        let (w, h, px) = renderer.board_thumbnail(&d, bg, false).expect("fond");
        assert_eq!((w, h), (400u32.div_ceil(step), 300u32.div_ceil(step)));
        assert!(px.chunks_exact(4).all(|p| p == [255, 255, 255, 255]), "fond blanc");
        let (_, _, mask) = renderer.board_thumbnail(&d, bg, true).expect("masque du fond");
        assert!(mask.chunks_exact(4).all(|p| p[0] == 255), "masque du fond blanc");
    }

    fn white_doc() -> Document {
        Document::with_background("x", 256, 256, 72.0, Background::White).expect("document")
    }

    /// Ajoute au sommet un calque uniformément rempli d'une couleur prémultipliée.
    fn add_fill(doc: &mut Document, parent: Option<LayerId>, px: [u8; 4]) -> LayerId {
        let mut pixels = TileStore::new();
        pixels.fill(doc.width(), doc.height(), px);
        let index = doc.children_of(parent).expect("parent").len();
        doc.add_pixel_layer("calque", pixels, parent, index).expect("calque")
    }

    fn render(doc: &Document) -> [u8; 4] {
        TileRenderer::new().tile(doc, 0, ORIGIN).map_or([0; 4], |t| t.pixel(10, 10))
    }

    /// Composition de référence, sans contenu figé des groupes (un filtre `include` le désactive).
    fn reference(doc: &Document, level: u8, coord: TileCoord) -> Option<Arc<Tile>> {
        let r = TileRenderer::new();
        let all = |_: LayerId| true;
        Pass { renderer: &r, doc, level, coord, generation: 0, include: Some(&all), opaque: None, replace: None, shift: (0, 0) }.run()
    }

    /// Au niveau 0, la composition par groupe figé est exacte (à l'arrondi près). Aux niveaux réduits, chaque
    /// position est arrondie au pixel de mip le plus proche, relativement au groupe : un calque peut s'y
    /// trouver décalé d'un pixel de mip, d'où une tolérance sur une petite part des pixels (les bords).
    fn assert_close(a: Option<Arc<Tile>>, b: Option<Arc<Tile>>, level: u8, what: &str) {
        let (a, b) = (a.map(|t| t.bytes().to_vec()).unwrap_or_default(), b.map(|t| t.bytes().to_vec()).unwrap_or_default());
        if a.is_empty() || b.is_empty() {
            assert!(a.iter().chain(b.iter()).all(|v| *v <= 2), "{what} : une tuile vide, l'autre non");
            return;
        }
        if level == 0 {
            let worst = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
            assert!(worst <= 2, "{what} : écart {worst}");
        } else {
            let off = a.iter().zip(&b).filter(|(x, y)| x.abs_diff(**y) > 2).count();
            assert!(off * 100 <= a.len() * 6, "{what} : {off} valeurs sur {} s'écartent", a.len());
        }
    }

    #[test]
    fn a_group_moved_as_a_block_reuses_its_composed_content_and_renders_the_same() {
        let mut doc = Document::with_background("x", 700, 520, 72.0, Background::White).expect("document");
        let group = doc.add_group("G", None, 1).expect("groupe");
        let nested = doc.add_group("N", Some(group), 0).expect("sous-groupe");
        let mut ids = Vec::new();
        for (k, parent) in [Some(group), Some(group), Some(nested), Some(nested)].into_iter().enumerate() {
            let (w, h) = (300u32, 220u32);
            let mut rgba = vec![0u8; (w * h * 4) as usize];
            for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
                let (x, y) = (i as u32 % w, i as u32 / w);
                let a = if (x / 20 + y / 20 + k as u32) % 2 == 0 { 255 } else { 120 };
                let c = |v: u32| (v * a / 255) as u8;
                px.copy_from_slice(&[c(40 + 50 * k as u32), c(x % 200), c(y % 200), a as u8]);
            }
            let store = TileStore::from_premultiplied(w, h, &rgba).expect("tampon").translated(37 + 61 * k as i32, 29 + 43 * k as i32);
            let index = doc.children_of(parent).expect("parent").len();
            ids.push(doc.add_pixel_layer("c", store, parent, index).expect("calque"));
        }
        doc.edit_props(ids[1], |p| p.opacity = 0.6).expect("opacité");
        let renderer = TileRenderer::new();
        let coords = |doc: &Document, level: u8| {
            let size = TILE_SIZE as u32;
            let (w, h) = ((doc.width() >> level).max(1).div_ceil(size), (doc.height() >> level).max(1).div_ceil(size));
            (0..h).flat_map(move |y| (0..w).map(move |x| TileCoord::new(x as i32, y as i32))).collect::<Vec<_>>()
        };
        let check = |doc: &Document, renderer: &TileRenderer, what: &str| {
            for level in 0..=TileRenderer::max_level(doc) {
                for coord in coords(doc, level) {
                    assert_close(renderer.tile(doc, level, coord), reference(doc, level, coord), level, &format!("{what}, niveau {level}, {coord:?}"));
                }
            }
        };
        check(&doc, &renderer, "avant");
        let filled = |r: &TileRenderer| lock(&r.caches.groups).iter().filter_map(|(k, versions)| versions.first().and_then(|(_, t)| t.clone()).map(|t| (*k, t))).collect::<HashMap<_, _>>();
        let before = filled(&renderer);
        assert!(!before.is_empty(), "le contenu du groupe est figé");
        // Déplacement du groupe entier, de biais : les tuiles figées sont reprises telles quelles.
        doc.translate_layers(&[group], 23, -11).expect("déplacement");
        renderer.invalidate_composites();
        check(&doc, &renderer, "après déplacement");
        let after = filled(&renderer);
        for (key, tile) in &before {
            assert!(after.get(key).is_some_and(|t| Arc::ptr_eq(t, tile)), "tuile figée {key:?} recalculée");
        }
        // Un calque du groupe déplacé seul : l'empreinte change, le contenu est recomposé et reste juste.
        doc.translate_layers(&[ids[2]], 40, 5).expect("déplacement d'un calque");
        renderer.invalidate_composites();
        check(&doc, &renderer, "après déplacement d'un calque du groupe");
        // Un mode de fusion dans le groupe : plus de contenu figé, toujours la même image.
        doc.edit_props(ids[0], |p| p.blend = BlendMode::Multiply).expect("mode");
        renderer.invalidate_composites();
        check(&doc, &renderer, "avec un mode Produit dans le groupe");
    }

    /// Groupe qui ne se fige pas en entier (remplissage en fond, réglage au-dessus, calque en Produit, en Transfert
    /// ou isolé, avec opacité) : le reste est figé et repris quand le groupe se déplace, puis après Ctrl+Z, et
    /// l'image reste celle de la composition calque par calque.
    #[test]
    fn a_partly_flattenable_group_keeps_the_rest_cached_through_moves_and_undo() {
        use crate::session::Session;
        let layer = |doc: &mut Document, parent: LayerId, k: i32| {
            let (w, h) = (260u32, 200u32);
            let rgba: Vec<u8> = (0..w * h)
                .flat_map(|i| {
                    let (x, y) = (i % w, i / w);
                    let a = if (x / 16 + y / 16 + k as u32) % 2 == 0 { 255 } else { 140 };
                    let c = |v: u32| (v * a / 255) as u8;
                    [c(30 + 40 * k as u32), c(x % 220), c(y % 220), a as u8]
                })
                .collect();
            let store = TileStore::from_premultiplied(w, h, &rgba).expect("tampon").translated(40 + 70 * k, 30 + 50 * k);
            let index = doc.children_of(Some(parent)).expect("parent").len();
            doc.add_pixel_layer("c", store, Some(parent), index).expect("calque")
        };
        type Setup = fn(&mut Document, LayerId, &dyn Fn(&mut Document, LayerId, i32) -> LayerId);
        let cases: [(&str, Setup); 4] = [
            ("remplissage en fond", |doc, g, add| {
                doc.add_fill_layer(crate::fill::FillContent::SolidColor { color: [210, 200, 40] }, "f", Some(g), 0).expect("remplissage");
                for k in 0..3 {
                    add(doc, g, k);
                }
            }),
            ("réglage au-dessus", |doc, g, add| {
                for k in 0..3 {
                    add(doc, g, k);
                }
                let index = doc.children_of(Some(g)).expect("parent").len();
                doc.add_adjustment_layer(crate::adjust::Adjustment::Invert, Some(g), index, false).expect("réglage");
            }),
            ("Produit au milieu, groupe isolé à 70 %", |doc, g, add| {
                add(doc, g, 0);
                let mid = add(doc, g, 1);
                doc.edit_props(mid, |p| p.blend = BlendMode::Multiply).expect("mode");
                for k in 2..5 {
                    add(doc, g, k);
                }
                doc.edit_props(g, |p| {
                    p.blend = BlendMode::Normal;
                    p.opacity = 0.7;
                })
                .expect("groupe");
            }),
            ("sous-groupes d'un seul calque", |doc, g, add| {
                for _ in 0..3 {
                    let index = doc.children_of(Some(g)).expect("parent").len();
                    let sub = doc.add_group("s", Some(g), index).expect("sous-groupe");
                    add(doc, sub, index as i32);
                }
            }),
        ];
        for (name, setup) in cases {
            let mut doc = Document::with_background("x", 700, 520, 72.0, Background::White).expect("document");
            let group = doc.add_group("G", None, 1).expect("groupe");
            setup(&mut doc, group, &layer);
            let mut session = Session::new(doc, "Ouvrir");
            let render_all = |s: &Session, what: &str| {
                let doc = s.document();
                for level in 0..=1u8 {
                    let size = TILE_SIZE as u32;
                    let (w, h) = ((doc.width() >> level).div_ceil(size), (doc.height() >> level).div_ceil(size));
                    for y in 0..h {
                        for x in 0..w {
                            let coord = TileCoord::new(x as i32, y as i32);
                            assert_close(s.renderer().tile(doc, level, coord), reference(doc, level, coord), level, &format!("{name}, {what}, niveau {level}, {coord:?}"));
                        }
                    }
                }
            };
            let cached = |s: &Session| lock(&s.renderer().caches.groups).iter().flat_map(|(k, v)| v.iter().filter_map(move |(_, t)| t.clone().map(|t| (*k, Arc::as_ptr(&t) as usize)))).collect::<std::collections::HashSet<_>>();
            render_all(&session, "au départ");
            let first = cached(&session);
            assert!(!first.is_empty(), "{name} : une partie du groupe est figée");
            session.apply_gesture("Déplacer", "g1".into(), |d| d.translate_layers(&[group], 37, 21)).expect("déplacement");
            session.renderer().invalidate_composites();
            render_all(&session, "après déplacement");
            let moved = cached(&session);
            assert!(first.iter().all(|t| moved.contains(t)), "{name} : contenu figé gardé au déplacement");
            session.undo();
            session.renderer().invalidate_composites();
            render_all(&session, "après Ctrl+Z");
            session.apply_gesture("Déplacer", "g2".into(), |d| d.translate_layers(&[group], -15, 40)).expect("déplacement");
            session.renderer().invalidate_composites();
            render_all(&session, "après un autre déplacement");
            assert_eq!(cached(&session).len(), first.len(), "{name} : rien n'a été recomposé");
        }
    }

    /// Un état d'historique relu du disque de travail (nouveaux stockages, mêmes pixels) retrouve le contenu figé
    /// de ses groupes : la version du contenu fait foi, pas l'adresse.
    #[test]
    fn a_store_keeps_its_version_through_clone_and_changes_it_on_write() {
        let mut a = TileStore::new();
        a.fill(300, 300, [10, 20, 30, 255]);
        let b = a.clone();
        assert_eq!(a.version(), b.version());
        let mut c = b.clone();
        c.get_mut(ORIGIN).set_pixel(0, 0, [1, 2, 3, 255]);
        assert_ne!(c.version(), b.version());
        assert_ne!(TileStore::new().version(), TileStore::new().version());
    }

    #[test]
    fn max_level_fits_document_in_one_tile() {
        let level_of = |w, h| TileRenderer::max_level(&Document::new("x", w, h, 72.0).expect("document"));
        assert_eq!(level_of(256, 256), 0);
        assert_eq!(level_of(257, 100), 1);
        assert_eq!(level_of(8000, 8000), 5);
    }

    #[test]
    fn single_plain_layer_is_not_copied() {
        let doc = white_doc();
        let rendered = TileRenderer::new().tile(&doc, 0, ORIGIN).expect("tuile");
        let id = doc.walk_top_down()[0].1.id;
        let stored = doc.layer(id).expect("calque").pixels().expect("pixels").get(ORIGIN).expect("tuile").clone();
        assert!(Arc::ptr_eq(&rendered, &stored));
    }

    #[test]
    fn opacity_fill_and_visibility() {
        let mut doc = white_doc();
        let black = add_fill(&mut doc, None, [0, 0, 0, 255]);
        assert_eq!(render(&doc), [0, 0, 0, 255]);
        doc.edit_props(black, |p| p.opacity = 0.5).expect("opacité");
        assert_eq!(render(&doc), [128, 128, 128, 255]);
        doc.edit_props(black, |p| p.fill = 0.5).expect("fond");
        assert_eq!(render(&doc), [191, 191, 191, 255], "opacité et fond se multiplient");
        doc.edit_props(black, |p| p.visible = false).expect("visibilité");
        assert_eq!(render(&doc), [255; 4]);
    }

    /// Comparaison : le calque disparaît là où sa valeur ou celle du dessous sort des plages, et apparaît en fondu
    /// entre les deux moitiés d'un triangle scindé. Une plage entière n'est pas gardée.
    #[test]
    fn blend_if_hides_by_this_layer_and_by_the_layer_below() {
        use crate::blend::{BlendIf, BlendIfChannel};
        let mut doc = white_doc();
        let top = add_fill(&mut doc, None, [100, 100, 100, 255]);
        let range = |this, under| Some(BlendIf { channel: BlendIfChannel::Gray, this, under });
        doc.edit_props(top, |p| p.blend_if = range([0, 0, 90, 90], [0, 0, 255, 255])).expect("comparaison");
        assert_eq!(render(&doc), [255; 4], "gris 100 au-dessus du blanc 90 : caché");
        doc.edit_props(top, |p| p.blend_if = range([0, 0, 255, 255], [0, 0, 200, 200])).expect("comparaison");
        assert_eq!(render(&doc), [255; 4], "dessous blanc hors plage : le calque disparaît");
        doc.edit_props(top, |p| p.blend_if = range([50, 150, 255, 255], [0, 0, 255, 255])).expect("comparaison");
        assert_eq!(render(&doc), [178, 178, 178, 255], "fondu à moitié (100 entre 50 et 150)");
        doc.edit_props(top, |p| p.blend_if = range([0, 0, 255, 255], [0, 0, 255, 255])).expect("comparaison");
        assert_eq!(doc.layer(top).expect("calque").props.blend_if, None, "plages entières : rien de gardé");
        assert_eq!(render(&doc), [100, 100, 100, 255]);
        doc.edit_props(top, |p| p.blend_if = Some(BlendIf { channel: BlendIfChannel::Red, this: [120, 120, 255, 255], under: [0, 0, 255, 255] })).expect("rouge");
        assert_eq!(render(&doc), [255; 4], "rouge 100 sous 120 : caché");
    }

    #[test]
    fn blend_mode_is_applied() {
        let mut doc = white_doc();
        add_fill(&mut doc, None, [128, 128, 128, 255]);
        let top = add_fill(&mut doc, None, [128, 128, 128, 255]);
        doc.edit_props(top, |p| p.blend = BlendMode::Multiply).expect("mode");
        assert_eq!(render(&doc), [64, 64, 64, 255]);
        doc.edit_props(top, |p| p.blend = BlendMode::Screen).expect("mode");
        assert_eq!(render(&doc), [192, 192, 192, 255]);
    }

    #[test]
    fn layer_mask_hides_and_can_be_disabled() {
        let mut doc = white_doc();
        let black = add_fill(&mut doc, None, [0, 0, 0, 255]);
        doc.add_mask(black, false).expect("masque noir");
        assert_eq!(render(&doc), [255; 4], "masque noir : calque invisible");
        doc.mask_mut(black).expect("masque").get_mut(ORIGIN).set_pixel(10, 10, [255]);
        assert_eq!(render(&doc), [0, 0, 0, 255], "pixel révélé");
        doc.set_mask_enabled(black, false).expect("désactivation");
        let tile = TileRenderer::new().tile(&doc, 0, ORIGIN).expect("tuile");
        assert_eq!(tile.pixel(200, 200), [0, 0, 0, 255], "masque désactivé : tout le calque est visible");
    }

    #[test]
    fn clipping_mask_restricts_to_base_alpha() {
        let mut doc = white_doc();
        // Base : un seul pixel rouge opaque. Calque écrêté : bleu partout.
        let mut base_pixels = TileStore::new();
        base_pixels.get_mut(ORIGIN).set_pixel(10, 10, [255, 0, 0, 255]);
        let base = doc.add_pixel_layer("base", base_pixels, None, 1).expect("base");
        let blue = add_fill(&mut doc, None, [0, 0, 255, 255]);
        assert_eq!(render(&doc), [0, 0, 255, 255], "sans écrêtage, le bleu couvre tout");

        doc.edit_props(blue, |p| p.clipped = true).expect("écrêtage");
        let tile = TileRenderer::new().tile(&doc, 0, ORIGIN).expect("tuile");
        assert_eq!(tile.pixel(10, 10), [0, 0, 255, 255], "bleu visible à travers la base");
        assert_eq!(tile.pixel(11, 10), [255; 4], "ailleurs, le bleu est écrêté");

        doc.edit_props(base, |p| p.visible = false).expect("base masquée");
        assert_eq!(render(&doc), [255; 4], "base invisible : le calque écrêté disparaît aussi");
    }

    #[test]
    fn pass_through_and_isolated_groups_differ() {
        // Fond blanc, groupe contenant un calque gris en Produit.
        let mut doc = white_doc();
        let group = doc.add_group("G", None, 1).expect("groupe");
        let gray = add_fill(&mut doc, Some(group), [128, 128, 128, 255]);
        doc.edit_props(gray, |p| p.blend = BlendMode::Multiply).expect("mode");
        add_fill(&mut doc, Some(group), [128, 128, 128, 255]);
        let second = doc.layer(group).expect("groupe").children()[1];
        doc.edit_props(second, |p| p.blend = BlendMode::Multiply).expect("mode");

        // Transfert : les deux Produit s'appliquent au fond blanc -> 255 * 0,5 * 0,5.
        assert_eq!(render(&doc), [64, 64, 64, 255]);
        // Groupe à 50 % en Transfert : interpolation entre avant (255) et après (64).
        doc.edit_props(group, |p| p.opacity = 0.5).expect("opacité");
        assert_eq!(render(&doc), [160, 160, 160, 255]);

        // Isolé (Normal) : le premier Produit n'a pas de fond dans le groupe.
        doc.edit_props(group, |p| (p.opacity, p.blend) = (1.0, BlendMode::Normal)).expect("mode");
        assert_eq!(render(&doc), [64, 64, 64, 255]);
        // Isolé en Produit sur blanc : contenu du groupe (128 x 0,5 = 64) x fond blanc.
        doc.edit_props(group, |p| p.blend = BlendMode::Multiply).expect("mode");
        assert_eq!(render(&doc), [64, 64, 64, 255]);
    }

    #[test]
    fn isolated_group_does_not_see_the_backdrop() {
        // Fond gris moyen ; groupe contenant un calque blanc en Différence.
        let mut doc = Document::new("x", 256, 256, 72.0).expect("document");
        add_fill(&mut doc, None, [128, 128, 128, 255]);
        let group = doc.add_group("G", None, 1).expect("groupe");
        let white = add_fill(&mut doc, Some(group), [255; 4]);
        doc.edit_props(white, |p| p.blend = BlendMode::Difference).expect("mode");
        assert_eq!(render(&doc), [127, 127, 127, 255], "Transfert : Différence avec le fond gris");
        doc.edit_props(group, |p| p.blend = BlendMode::Normal).expect("mode");
        assert_eq!(render(&doc), [255; 4], "isolé : le blanc ne voit pas le gris");
    }

    #[test]
    fn dissolve_produces_only_opaque_or_absent_pixels() {
        let mut doc = white_doc();
        let black = add_fill(&mut doc, None, [0, 0, 0, 255]);
        doc.edit_props(black, |p| (p.blend, p.opacity) = (BlendMode::Dissolve, 0.5)).expect("props");
        let tile = TileRenderer::new().tile(&doc, 0, ORIGIN).expect("tuile");
        let blacks = tile.bytes().chunks_exact(4).filter(|px| px[0] == 0).count();
        let whites = tile.bytes().chunks_exact(4).filter(|px| px[0] == 255).count();
        assert_eq!(blacks + whites, TILE_PIXELS, "aucune valeur intermédiaire");
        let ratio = blacks as f32 / TILE_PIXELS as f32;
        assert!((ratio - 0.5).abs() < 0.02, "environ la moitié des pixels : {ratio}");
    }

    #[test]
    fn compositing_at_a_mip_level_matches_downsampling_for_uniform_layers() {
        let mut doc = Document::with_background("x", 1024, 1024, 72.0, Background::White).expect("document");
        let black = add_fill(&mut doc, None, [0, 0, 0, 255]);
        doc.edit_props(black, |p| p.opacity = 0.5).expect("opacité");
        let tile = TileRenderer::new().tile(&doc, 2, ORIGIN).expect("mip");
        assert_eq!(tile.pixel(100, 100), [128, 128, 128, 255]);
    }

    #[test]
    fn invalidation_refreshes_cached_tiles() {
        let mut doc = white_doc();
        let top = add_fill(&mut doc, None, [0, 0, 0, 255]);
        let renderer = TileRenderer::new();
        assert_eq!(renderer.tile(&doc, 0, ORIGIN).expect("tuile").pixel(1, 1), [0, 0, 0, 255]);

        doc.pixels_mut(top).expect("pixels").get_mut(ORIGIN).set_pixel(1, 1, [255, 0, 0, 255]);
        assert_eq!(renderer.tile(&doc, 0, ORIGIN).expect("tuile").pixel(1, 1), [0, 0, 0, 255], "cache non invalidé");
        assert_eq!(renderer.invalidate_tiles(&doc, top, false, &[ORIGIN]), vec![ORIGIN]);
        assert_eq!(renderer.tile(&doc, 0, ORIGIN).expect("tuile").pixel(1, 1), [255, 0, 0, 255]);
    }

    #[test]
    fn moved_layer_is_composited_at_its_position_at_every_level() {
        let mut doc = Document::with_background("x", 1024, 1024, 72.0, Background::White).expect("document");
        let mut pixels = TileStore::new();
        pixels.fill(100, 100, [255, 0, 0, 255]);
        let red = doc.add_pixel_layer("rouge", pixels, None, 1).expect("calque");
        doc.translate_layers(&[red], 300, -40).expect("déplacement"); // à cheval sur deux tuiles, en partie hors cadre

        let renderer = TileRenderer::new();
        let right = renderer.tile(&doc, 0, TileCoord::new(1, 0)).expect("tuile");
        assert_eq!(right.pixel(300 - 256, 0), [255, 0, 0, 255], "coin visible du carré");
        assert_eq!(right.pixel(399 - 256, 59), [255, 0, 0, 255]);
        assert_eq!(right.pixel(400 - 256, 59), [255; 4]);
        assert_eq!(right.pixel(300 - 256, 60), [255; 4]);
        assert_eq!(renderer.tile(&doc, 0, ORIGIN).expect("tuile").pixel(255, 10), [255; 4]);

        // Niveau 2 : tout le document dans une tuile, carré attendu vers (75..100, 0..15).
        let mip = renderer.tile(&doc, 2, ORIGIN).expect("mip");
        assert_eq!(mip.pixel(87, 5), [255, 0, 0, 255]);
        assert_eq!(mip.pixel(110, 5), [255; 4]);

        // Une tuile locale modifiée invalide les tuiles du document qu'elle recouvre.
        assert_eq!(renderer.invalidate_tiles(&doc, red, false, &[ORIGIN]), vec![TileCoord::new(1, 0), TileCoord::new(2, 0)]);
    }

    #[test]
    fn filtered_parts_recompose_the_full_image_for_normal_layers() {
        let mut doc = white_doc();
        let middle = add_fill(&mut doc, None, [100, 0, 0, 128]); // prémultiplié : couleur <= alpha
        let top = add_fill(&mut doc, None, [0, 0, 60, 64]);
        let background = doc.walk_top_down().last().expect("fond").1.id;
        let renderer = TileRenderer::new();
        let part = |id| renderer.tile_filtered(&doc, 0, ORIGIN, &move |l| l == id, None).expect("partie").pixel(3, 3);
        let recomposed = crate::pixel::over(part(top), crate::pixel::over(part(middle), part(background)));
        let full = renderer.tile(&doc, 0, ORIGIN).expect("tuile").pixel(3, 3);
        for c in 0..4 {
            assert!(recomposed[c].abs_diff(full[c]) <= 1, "{recomposed:?} != {full:?}");
        }
    }

    #[test]
    fn a_layer_can_be_rendered_at_full_opacity_for_the_opacity_slider_preview() {
        let mut doc = white_doc();
        let red = add_fill(&mut doc, None, [255, 0, 0, 255]);
        doc.edit_props(red, |p| p.opacity = 0.25).expect("opacité");
        let renderer = TileRenderer::new();
        let part = renderer.tile_filtered(&doc, 0, ORIGIN, &move |l| l == red, Some(red)).expect("partie");
        assert_eq!(part.pixel(3, 3), [255, 0, 0, 255], "l'opacité du calque est ignorée : l'interface l'applique elle-même");
        let usual = renderer.tile_filtered(&doc, 0, ORIGIN, &move |l| l == red, None).expect("partie");
        assert_eq!(usual.pixel(3, 3), [64, 0, 0, 64]);
    }

    #[test]
    fn each_effect_has_its_own_blend_mode() {
        use crate::blend::BlendMode;
        let mut doc = white_doc();
        // Fond vert, puis un carré rouge avec une ombre portée rouge décalée.
        let green = doc.add_pixel_layer("vert", TileStore::new(), None, 1).expect("calque");
        doc.pixels_mut(green).expect("pixels").fill(256, 256, [0, 255, 0, 255]);
        let square = doc.add_pixel_layer("carré", TileStore::new(), None, 2).expect("calque");
        for y in 40..80 {
            for x in 40..80 {
                doc.pixels_mut(square).expect("pixels").get_mut(ORIGIN).set_pixel(x, y, [255, 0, 0, 255]);
            }
        }
        let shadow = |blend| LayerEffect::DropShadow { enabled: true, color: [255, 0, 0], opacity: 1.0, angle: 135.0, distance: 20.0, size: 0.0, blend };
        doc.set_effects(square, vec![shadow(BlendMode::Normal)]).expect("effets");
        assert_eq!(TileRenderer::new().tile(&doc, 0, ORIGIN).expect("tuile").pixel(88, 88), [255, 0, 0, 255], "Normal : l'ombre rouge couvre le vert");
        // Produit : rouge × vert = noir. L'ombre se fond dans ce qui est SOUS le calque.
        doc.set_effects(square, vec![shadow(BlendMode::Multiply)]).expect("effets");
        let multiplied = TileRenderer::new().tile(&doc, 0, ORIGIN).expect("tuile");
        assert_eq!(multiplied.pixel(88, 88), [0, 0, 0, 255], "Produit : l'ombre assombrit le fond");
        assert_eq!(multiplied.pixel(60, 60), [255, 0, 0, 255], "le calque lui-même ne change pas");
        // Incrustation de couleur en Produit : bleu × rouge = noir, sur le calque seulement.
        doc.set_effects(square, vec![LayerEffect::ColorOverlay { enabled: true, color: [0, 0, 255], opacity: 1.0, blend: BlendMode::Multiply }]).expect("effets");
        let overlaid = TileRenderer::new().tile(&doc, 0, ORIGIN).expect("tuile");
        assert_eq!((overlaid.pixel(60, 60), overlaid.pixel(120, 120)), ([0, 0, 0, 255], [0, 255, 0, 255]));
        // Contour extérieur en Produit : il ne s'applique que là où il est dessiné, en se fondant dans ce qui est
        // dessous (le fond vert). Blanc × vert = vert ; en Normal, il serait blanc.
        let stroke = |blend| LayerEffect::Stroke { enabled: true, color: [255, 255, 255], opacity: 1.0, width: 6.0, position: crate::effects::StrokePosition::Outside, blend };
        doc.set_effects(square, vec![stroke(BlendMode::Normal)]).expect("effets");
        assert_eq!(TileRenderer::new().tile(&doc, 0, ORIGIN).expect("tuile").pixel(83, 60), [255, 255, 255, 255], "Normal : contour blanc");
        doc.set_effects(square, vec![stroke(BlendMode::Multiply)]).expect("effets");
        let multiplied = TileRenderer::new().tile(&doc, 0, ORIGIN).expect("tuile");
        assert_eq!((multiplied.pixel(83, 60), multiplied.pixel(60, 60)), ([0, 255, 0, 255], [255, 0, 0, 255]), "Produit : le fond vert transparaît, le calque est intact");
    }

    #[test]
    fn clipped_layers_stop_at_the_base_shape_not_at_its_shadow() {
        let mut doc = white_doc();
        let base = doc.add_pixel_layer("base", TileStore::new(), None, 1).expect("calque");
        let pixels = doc.pixels_mut(base).expect("pixels");
        for y in 40..80 {
            for x in 40..80 {
                pixels.get_mut(ORIGIN).set_pixel(x, y, [255, 0, 0, 255]);
            }
        }
        doc.set_effects(base, vec![LayerEffect::DropShadow { enabled: true, color: [0, 0, 0], opacity: 1.0, angle: 135.0, distance: 12.0, size: 0.0, blend: crate::blend::BlendMode::Normal }]).expect("effets");
        // Calque écrêté : un aplat bleu bien plus grand que la base.
        let paint = doc.add_pixel_layer("peinture", TileStore::new(), None, 2).expect("calque");
        doc.pixels_mut(paint).expect("pixels").fill(200, 200, [0, 0, 255, 255]);
        doc.edit_props(paint, |p| p.clipped = true).expect("écrêtage");

        let tile = TileRenderer::new().tile(&doc, 0, ORIGIN).expect("tuile");
        assert_eq!(tile.pixel(60, 60), [0, 0, 255, 255], "dans la forme de la base : la peinture écrêtée");
        assert_eq!(tile.pixel(86, 86), [0, 0, 0, 255], "sur l'ombre portée : l'ombre, pas la peinture");
        assert_eq!(tile.pixel(120, 120), [255; 4], "ailleurs : le fond");
    }

    #[test]
    fn a_resized_shape_never_keeps_the_ghost_of_its_former_outline() {
        use crate::shape::{ShapeData, ShapeKind};
        let mut doc = white_doc();
        let data = |side: f32| ShapeData { kind: ShapeKind::Rect, width: side, height: side, radius: 0.0, fill: Some([0, 0, 255, 255]), stroke: None, stroke_width: 0.0, sides: 3, transform: Default::default() };
        let shape = doc.add_shape_layer(data(160.0), (20, 20), None, 1).expect("forme");
        doc.set_effects(shape, vec![LayerEffect::DropShadow { enabled: true, color: [0, 0, 0], opacity: 1.0, angle: 135.0, distance: 6.0, size: 0.0, blend: crate::blend::BlendMode::Normal }]).expect("effets");
        let renderer = TileRenderer::new();
        assert_eq!(renderer.tile(&doc, 0, ORIGIN).expect("tuile").pixel(170, 100), [0, 0, 255, 255]);
        // Redimensionnée bien des fois, comme en tirant un curseur : à chaque étape l'ancien dessin est libéré
        // et son adresse peut resservir. Jamais l'ancienne silhouette ne doit réapparaître.
        for step in 0..40 {
            let side = if step % 2 == 0 { 80.0 } else { 160.0 };
            doc.set_shape(shape, data(side)).expect("taille");
            renderer.invalidate_composites();
            lock(&renderer.caches.pixel_mips).clear();
            let px = renderer.tile(&doc, 0, ORIGIN).expect("tuile").pixel(170, 100);
            let expected = if side == 80.0 { [255; 4] } else { [0, 0, 255, 255] };
            assert_eq!(px, expected, "étape {step}, côté {side}");
        }
    }

    #[test]
    fn a_shape_shadow_stays_dark_and_soft_at_every_zoom_level() {
        use crate::shape::{ShapeData, ShapeKind};
        let mut doc = Document::with_background("x", 2048, 2048, 72.0, crate::document::Background::White).expect("document");
        let data = ShapeData { kind: ShapeKind::Rect, width: 900.0, height: 700.0, radius: 120.0, fill: Some([240, 240, 245, 255]), stroke: None, stroke_width: 0.0, sides: 3, transform: Default::default() };
        let shape = doc.add_shape_layer(data, (500, 600), None, 1).expect("forme");
        doc.set_effects(shape, vec![LayerEffect::DropShadow { enabled: true, color: [0, 0, 0], opacity: 0.55, angle: 120.0, distance: 40.0, size: 60.0, blend: crate::blend::BlendMode::Normal }]).expect("effets");
        let renderer = TileRenderer::new();
        for level in 0..=3u8 {
            // Un point sous la forme, à 30 px de son bord inférieur (dans l'ombre), et un point loin de tout.
            let scale = 1i32 << level;
            let (x, y) = (950 / scale, 1330 / scale);
            let coord = TileCoord::new(x / 256, y / 256);
            let tile = renderer.tile(&doc, level, coord).expect("tuile");
            let shadow = tile.pixel((x % 256) as u32, (y % 256) as u32);
            assert!(shadow[0] < 235 && shadow[0] > 60, "niveau {level} : ombre sombre et partielle, pas blanche ni pleine : {shadow:?}");
        }
    }

    #[test]
    fn layer_effects_are_composited_and_follow_pixel_changes() {
        let mut doc = white_doc();
        let layer = doc.add_pixel_layer("carré", TileStore::new(), None, 1).expect("calque");
        let paint = |doc: &mut Document, x0: i32| {
            let pixels = doc.pixels_mut(layer).expect("pixels");
            for y in 40..80 {
                for x in x0..x0 + 40 {
                    pixels.get_mut(ORIGIN).set_pixel(x as u32, y as u32, [255, 0, 0, 255]);
                }
            }
        };
        paint(&mut doc, 40);
        let shadow = |enabled| vec![LayerEffect::DropShadow { enabled, color: [0, 0, 0], opacity: 1.0, angle: 135.0, distance: 10.0, size: 0.0, blend: crate::blend::BlendMode::Normal }];
        doc.set_effects(layer, shadow(true)).expect("effets");
        let renderer = TileRenderer::new();
        let tile = renderer.tile(&doc, 0, ORIGIN).expect("tuile");
        assert_eq!((tile.pixel(60, 60), tile.pixel(84, 84), tile.pixel(30, 30)), ([255, 0, 0, 255], [0, 0, 0, 255], [255; 4]), "calque, son ombre, puis le fond");

        // Les pixels changent (un second carré) : le rendu stylé est refait, l'ombre suit.
        paint(&mut doc, 120);
        renderer.forget_layer(layer);
        renderer.invalidate_composites();
        assert_eq!(renderer.tile(&doc, 0, ORIGIN).expect("tuile").pixel(164, 84), [0, 0, 0, 255], "ombre du nouveau carré");
        // Effet désactivé : plus d'ombre.
        doc.set_effects(layer, shadow(false)).expect("effets");
        renderer.forget_layer(layer);
        renderer.invalidate_composites();
        assert_eq!(renderer.tile(&doc, 0, ORIGIN).expect("tuile").pixel(84, 84), [255; 4]);
    }

    /// Pendant un trait, les effets ne sont recalculés qu'autour des tuiles peintes : le rendu est celui d'un
    /// calcul complet (à l'arrondi du flou près), et la tuile où rien n'a changé garde ses pixels partagés.
    #[test]
    fn effects_are_restyled_locally_around_changed_tiles() {
        let mut doc = Document::with_background("x", 1024, 512, 72.0, crate::document::Background::White).expect("document");
        let mut pixels = TileStore::new();
        pixels.get_mut(ORIGIN).set_pixel(40, 40, [0, 0, 255, 255]);
        pixels.get_mut(TileCoord::new(3, 1)).set_pixel(100, 100, [0, 0, 255, 255]);
        let layer = doc.add_pixel_layer("calque", pixels, None, 1).expect("calque");
        let effects = vec![LayerEffect::DropShadow { enabled: true, color: [0, 0, 0], opacity: 1.0, angle: 135.0, distance: 8.0, size: 6.0, blend: BlendMode::Normal }, LayerEffect::OuterGlow { enabled: true, color: [255, 0, 0], opacity: 0.8, size: 10.0, blend: BlendMode::Normal }];
        doc.set_effects(layer, effects).expect("effets");
        let renderer = TileRenderer::new();
        let before = renderer.styled_layer(doc.layer(layer).expect("calque")).expect("stylé");
        // Un trait sur la première tuile seulement.
        doc.pixels_mut(layer).expect("pixels").get_mut(ORIGIN).set_pixel(200, 60, [0, 0, 255, 255]);
        renderer.invalidate_tiles(&doc, layer, false, &[ORIGIN]);
        let local = renderer.styled_layer(doc.layer(layer).expect("calque")).expect("stylé");
        let full = TileRenderer::new().styled_layer(doc.layer(layer).expect("calque")).expect("stylé");
        assert!(!Arc::ptr_eq(&local.main, &before.main));
        assert!(Arc::ptr_eq(local.main.get(TileCoord::new(3, 1)).expect("tuile"), before.main.get(TileCoord::new(3, 1)).expect("tuile")), "tuile loin du trait : gardée telle quelle");
        let (mut a, mut b) = (Sampler::new(&local.main), Sampler::new(&full.main));
        for y in -40..300 {
            for x in -40..560 {
                let (p, q) = (a.get(x, y), b.get(x, y));
                assert!(p.iter().zip(q).all(|(u, v)| (i32::from(*u) - i32::from(v)).abs() <= 2), "({x}, {y}) : local {p:?}, complet {q:?}");
            }
        }
        assert_eq!(a.get(200, 60), [0, 0, 255, 255]);
        assert!(a.get(206, 66)[3] > 0, "ombre du nouveau pixel");
    }

    #[test]
    fn the_cache_budget_bounds_memory_without_changing_the_picture() {
        let doc = Document::with_background("x", 2048, 2048, 72.0, crate::document::Background::White).expect("document");
        let renderer = TileRenderer::new();
        let render_all = || (0..8).flat_map(|y| (0..8).map(move |x| TileCoord::new(x, y))).map(|c| renderer.tile(&doc, 0, c).expect("tuile").pixel(1, 1)).collect::<Vec<_>>();
        set_cache_budget(0);
        let unlimited = render_all();
        assert!(renderer.cache_bytes() >= 64 * 256 * 1024, "sans budget, tout reste en cache");
        set_cache_budget(4 * 1024 * 1024);
        renderer.trim(); // ce que fait la coque quand la préférence change
        let limited = render_all();
        assert!(renderer.cache_bytes() <= 4 * 1024 * 1024 + 256 * 1024, "budget respecté : {}", renderer.cache_bytes());
        assert_eq!(limited, unlimited, "même image, seulement recalculée plus souvent");
        set_cache_budget(0);
    }

    #[test]
    fn the_dynamic_cache_respects_the_memory_cap() {
        use crate::memory::MIN_CACHE_BUDGET;
        const GIB: u64 = 1024 * 1024 * 1024;
        // Le budget découle du plafond : ce qu'il laisse une fois comptées les autres tuiles, jamais moins que le minimum.
        assert_eq!(cache_budget_under(0, 0, 3 * GIB), 0, "sans plafond ni budget fixe : illimité");
        assert_eq!(cache_budget_under(0, 64 * 1024 * 1024, 3 * GIB), 64 * 1024 * 1024, "sans plafond : le budget fixe");
        assert_eq!(cache_budget_under(4 * GIB, 0, GIB), 3 * GIB, "le plafond moins documents et historique");
        assert_eq!(cache_budget_under(4 * GIB, 0, 4 * GIB), MIN_CACHE_BUDGET, "plafond atteint : le minimum, pas zéro");
        assert_eq!(cache_budget_under(4 * GIB, 0, 10 * GIB), MIN_CACHE_BUDGET, "plafond dépassé : toujours le minimum");
        assert_eq!(cache_budget_under(4 * GIB, GIB, 0), GIB, "un budget fixe borne en plus");

        // Le plafond est global au processus et d'autres tests tournent en même temps : on le simule. Un document de
        // 2048² a 64 tuiles composées (16 Mio) ; avec 4 Mio de marge sous le plafond, le cache ne garde que ce qui tient.
        let doc = Document::with_background("x", 2048, 2048, 72.0, crate::document::Background::White).expect("document");
        let renderer = TileRenderer::new();
        let render_all = || (0..8).flat_map(|y| (0..8).map(move |x| TileCoord::new(x, y))).map(|c| renderer.tile(&doc, 0, c).expect("tuile").pixel(1, 1)).collect::<Vec<_>>();
        let unlimited = render_all();
        let own = renderer.cache_bytes();
        assert!(own >= 64 * 256 * 1024, "sans plafond, tout reste en cache");
        let others = crate::tiles::resident_bytes().saturating_sub(own);
        let cap = others + 4 * 1024 * 1024;
        let budget = cache_budget_under(cap, 0, others);
        assert_eq!(budget, MIN_CACHE_BUDGET.max(4 * 1024 * 1024), "la marge sous le plafond, au moins le minimum");
        renderer.caches.trim_to(4 * 1024 * 1024); // le minimum (128 Mio) ne mordrait pas ici : on serre à la marge nue
        assert!(renderer.cache_bytes() <= 4 * 1024 * 1024, "vidé sous le plafond : {}", renderer.cache_bytes());
        assert_eq!(render_all(), unlimited, "même image, seulement recalculée");
        // Le registre voit ce cache : il compte dans le total, et le serrer global le vide aussi.
        assert!(crate::memory::cache_bytes_total() >= renderer.cache_bytes());
        renderer.caches.trim_to(0);
        assert!(renderer.cache_bytes() > 0, "budget nul = illimité : rien n'est vidé");
    }

    #[test]
    fn thumbnails_cover_layers_and_masks() {
        let mut doc = Document::with_background("x", 1024, 512, 72.0, Background::White).expect("document");
        let id = doc.walk_top_down()[0].1.id;
        let renderer = TileRenderer::new();
        let (w, h, rgba) = renderer.thumbnail(&doc, id, false).expect("miniature");
        assert_eq!((w, h, rgba.len()), (256, 128, 256 * 128 * 4));
        assert_eq!(&rgba[..4], &[255; 4]);
        assert!(renderer.thumbnail(&doc, id, true).is_none(), "pas de masque");
        doc.add_mask(id, false).expect("masque");
        let (_, _, mask) = renderer.thumbnail(&doc, id, true).expect("miniature du masque");
        assert_eq!(&mask[..4], &[0, 0, 0, 255]);
    }
}
