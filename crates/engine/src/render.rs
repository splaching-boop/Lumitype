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

type MipCache<const C: usize> = Mutex<HashMap<(LayerId, u8, TileCoord), Option<Arc<TileBuf<C>>>>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Les caches ne contiennent que des données recalculables : un verrou
    // empoisonné reste utilisable.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Les caches de tuiles d'un document : ce que le plafond de mémoire peut vider. Ils sont partagés (`Arc`)
/// entre le [`TileRenderer`] qui les remplit et le registre du module `memory`, qui peut les serrer tous
/// quand une session grossit, même ceux des documents qui ne sont pas à l'écran.
#[derive(Debug, Default)]
pub struct Caches {
    composite: Mutex<HashMap<(u8, TileCoord), Option<Arc<Tile>>>>,
    pixel_mips: MipCache<4>,
    mask_mips: MipCache<1>,
}

impl Caches {
    /// Mémoire occupée par les caches, en octets (estimation : une tuile RGBA = 256 Kio, une tuile de masque 64 Kio).
    pub fn bytes(&self) -> u64 {
        let rgba = (TILE_SIZE * TILE_SIZE * 4) as u64;
        let filled = |cache: &HashMap<(u8, TileCoord), Option<Arc<Tile>>>| cache.values().filter(|t| t.is_some()).count() as u64;
        let composite = filled(&lock(&self.composite)) * rgba;
        let pixels = lock(&self.pixel_mips).values().filter(|t| t.is_some()).count() as u64 * rgba;
        let masks = lock(&self.mask_mips).values().filter(|t| t.is_some()).count() as u64 * rgba / 4;
        composite + pixels + masks
    }

    /// Respecte le budget (appelé à chaque tuile ajoutée, et quand le budget ou le plafond de mémoire change) :
    /// au-delà, les caches sont vidés (ils se reconstruisent à la demande). Ce sont des caches purs : les vider
    /// ne perd rien, cela coûte seulement un recalcul. Le budget est recalculé à chaque passage : sous un plafond
    /// de mémoire, il rétrécit à mesure que documents et historique grossissent (voir [`cache_budget_for`]).
    pub fn trim(&self) {
        self.trim_to(cache_budget_for(self.bytes()));
    }

    /// Vide les caches s'ils dépassent `budget` octets (0 = illimité).
    pub fn trim_to(&self, budget: u64) {
        if budget == 0 || self.bytes() <= budget {
            return;
        }
        // D'abord les réductions par calque, les plus volumineuses ; puis, si cela ne suffit pas, les tuiles composées.
        lock(&self.pixel_mips).clear();
        lock(&self.mask_mips).clear();
        if self.bytes() > budget {
            lock(&self.composite).clear();
        }
    }
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
    styled: Mutex<HashMap<LayerId, (Arc<TileStore>, Vec<LayerEffect>, Arc<StyledStores>)>>,
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
        Pass { renderer: self, doc, level, coord, generation, include: None, opaque: None, replace: None }.program()
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
        let pass = Pass { renderer: self, doc, level, coord, generation, include: None, opaque: None, replace: None };
        let tile = pass.run();
        if self.generation.load(Ordering::Acquire) == generation {
            lock(&self.caches.composite).insert((level, coord), tile.clone());
            self.trim();
        }
        tile
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
        if let Some((known, for_effects, stores)) = cache.get(&id) {
            if Arc::ptr_eq(known, pixels) && for_effects == effects {
                return Arc::clone(stores);
            }
        }
        // En cas d'échec (calque démesuré), le calque s'affiche sans ses effets plutôt que pas du tout.
        let stores = Arc::new(match crate::effects::styled_parts(pixels, effects) {
            Ok(parts) => StyledStores {
                main: Arc::new(parts.main),
                under: parts.under.into_iter().map(|(mode, store)| (mode, Arc::new(store))).collect(),
                over: parts.over.into_iter().map(|(mode, store)| (mode, Arc::new(store))).collect(),
            },
            Err(_) => StyledStores { main: Arc::clone(pixels), under: Vec::new(), over: Vec::new() },
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
        Pass { renderer: self, doc, level: level.min(Self::max_level(doc)), coord, generation, include: None, opaque: None, replace: Some((id, store)) }.run()
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
        Pass { renderer: self, doc, level, coord, generation, include: Some(include), opaque, replace }.run()
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
            let tile =
                gather(&self.caches.pixel_mips, &self.generation, generation, layer.pixels()?, layer.id, layer.offset, level, origin);
            for y in 0..h {
                for x in 0..w {
                    out.extend_from_slice(&tile.as_ref().map_or([0; 4], |t| unpremultiply(t.pixel(x, y))));
                }
            }
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
}

impl Pass<'_> {
    fn run(&self) -> Option<Arc<Tile>> {
        self.program().run_on_cpu()
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
            return gather(&r.caches.pixel_mips, &r.generation, self.generation, pixels, layer.id, layer.offset, self.level, self.coord);
        }
        // Calque à effets : le compositeur travaille sur le calque stylé, pas sur les pixels bruts.
        let styled = r.styled_store(layer.id, pixels, &layer.effects);
        gather(&r.caches.pixel_mips, &r.generation, self.generation, &styled, layer.id, layer.offset, self.level, self.coord)
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
                gather(&r.caches.pixel_mips, &r.generation, self.generation, store, key, layer.offset, self.level, self.coord).map(|tile| (*mode, tile))
            })
            .collect()
    }

    /// Pixels du calque SANS ses effets. Un calque écrêté se limite à la forme de sa base, pas à son
    /// ombre portée ni à sa lueur : c'est cet alpha-là qui sert de gabarit.
    fn raw_pixels(&self, layer: &Layer) -> Option<Arc<Tile>> {
        let r = self.renderer;
        gather(&r.caches.pixel_mips, &r.generation, self.generation, layer.pixels()?, layer.id | RAW_PIXELS, layer.offset, self.level, self.coord)
    }

    fn mask_factor(&self, layer: &Layer) -> Coverage {
        let Some(mask) = layer.mask.as_ref().filter(|m| m.enabled) else { return Coverage::Uniform(1.0) };
        let r = self.renderer;
        match gather(&r.caches.mask_mips, &r.generation, self.generation, &mask.pixels, layer.id, layer.offset, self.level, self.coord) {
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
            let plain = p.visible && self.opacity(layer) >= 1.0 && !p.clipped && layer.mask.is_none();
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
                    builder.push(Op::Blend { target, source: Source::Tile(tile), mode, opacity: base.props.opacity, mask: self.mask_factor(base) });
                }
            };
            fuse(self.underlays(base), builder);
            builder.push(Op::Blend { target, source: content, mode: base.props.blend, opacity: self.opacity(base), mask: self.mask_factor(base) });
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
            LayerKind::Pixel { .. } | LayerKind::Text { .. } | LayerKind::Shape { .. } | LayerKind::Smart { .. } => self.pixels(layer).map(|tile| (Source::Tile(tile), None)),
            LayerKind::Adjustment { .. } => None,
            LayerKind::Group { children, .. } => {
                let buffer = builder.take();
                builder.push(Op::Clear { target: buffer });
                self.composite_list(children, buffer, builder);
                Some((Source::Buffer(buffer), Some(buffer)))
            }
        }
    }

    fn pass_through(&self, group: &Layer, target: u16, builder: &mut Builder) {
        let opacity = self.opacity(group);
        let mask = self.mask_factor(group);
        if opacity >= 1.0 && matches!(mask, Coverage::Uniform(v) if v >= 1.0) {
            self.composite_list(group.children(), target, builder);
            return;
        }
        // Opacité ou masque : le groupe interpole entre l'image d'avant et celle d'après.
        let before = builder.take();
        builder.push(Op::Copy { target: before, source: target });
        self.composite_list(group.children(), target, builder);
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
    use crate::document::Background;
    use crate::plan::TILE_PIXELS;
    use crate::tiles::TileStore;

    const ORIGIN: TileCoord = TileCoord::new(0, 0);

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
        let others = crate::tiles::resident_bytes() - own;
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
