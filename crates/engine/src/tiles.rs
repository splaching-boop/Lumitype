//! Stockage creux en tuiles avec copie à l'écriture.
//!
//! Chaque tuile est partagée par `Arc` : cloner un [`Store`] ne copie aucun
//! pixel, et seule une tuile réellement modifiée est dupliquée. L'historique
//! repose sur cette propriété : deux états successifs partagent toutes les
//! tuiles qui n'ont pas changé.
//!
//! Le nombre de canaux est un paramètre : 4 pour les calques (RGBA
//! prémultiplié), 1 pour les masques (niveaux de gris).
//!
//! Un stockage est **sans limites** : les coordonnées de tuile sont signées et
//! rien ne borne l'étendue. Un calque peut donc déborder du document dans toutes
//! les directions (contenu conservé quand on le déplace hors cadre), et c'est la
//! position du calque, pas son stockage, qui le situe dans le document.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rayon::prelude::*;

/// Octets de toutes les tuiles vivantes du processus, quel que soit leur propriétaire (documents, états
/// d'annulation, caches de rendu, tampons de travail). Tenu à jour à la création et à la destruction de
/// chaque [`TileBuf`] : une tuile partagée par `Arc` entre plusieurs états n'est comptée qu'une fois,
/// puisqu'elle n'est allouée qu'une fois. C'est la mesure sur laquelle repose le plafond de mémoire
/// (voir le module `memory`).
static RESIDENT_BYTES: AtomicU64 = AtomicU64::new(0);

/// Mémoire occupée par les tuiles actuellement allouées, en octets.
pub fn resident_bytes() -> u64 {
    RESIDENT_BYTES.load(Ordering::Relaxed)
}

/// Côté d'une tuile en pixels.
pub const TILE_SIZE: u32 = 256;
const TILE_PIXELS: usize = (TILE_SIZE * TILE_SIZE) as usize;
/// Taille d'une tuile RGBA8 en octets.
pub const TILE_BYTES: usize = TILE_PIXELS * 4;
/// Plus grand côté accepté pour un document.
pub const MAX_DIMENSION: u32 = 30_000;

/// Position d'une tuile dans la grille (en tuiles, pas en pixels).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileCoord {
    pub x: i32,
    pub y: i32,
}

impl TileCoord {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// Tuile contenant le pixel `(x, y)`, et position du pixel dans cette tuile.
    /// Division euclidienne : correcte aussi pour les coordonnées négatives.
    pub const fn of_pixel(x: i32, y: i32) -> (Self, u32, u32) {
        let size = TILE_SIZE as i32;
        (Self::new(x.div_euclid(size), y.div_euclid(size)), x.rem_euclid(size) as u32, y.rem_euclid(size) as u32)
    }

    /// Coordonnées du pixel haut gauche de la tuile.
    pub const fn origin(self) -> (i32, i32) {
        (self.x * TILE_SIZE as i32, self.y * TILE_SIZE as i32)
    }
}

/// Bloc de `TILE_SIZE x TILE_SIZE` pixels à `C` canaux de 8 bits, toujours complet.
///
/// `Clone` et `Drop` sont écrits à la main (et non dérivés) pour tenir le compteur global
/// [`resident_bytes`] : chaque tuile allouée s'y ajoute, chaque tuile libérée s'en retire.
#[derive(PartialEq, Eq)]
pub struct TileBuf<const C: usize> {
    data: Box<[u8]>,
}

/// Tuile de calque : RGBA, alpha prémultiplié.
pub type Tile = TileBuf<4>;
/// Tuile de masque : 0 = masqué, 255 = visible.
pub type MaskTile = TileBuf<1>;

impl<const C: usize> TileBuf<C> {
    /// Octets d'une tuile de ce type (tous ses pixels sont stockés, même uniformes).
    pub const BYTES: u64 = (TILE_PIXELS * C) as u64;

    pub fn filled(px: [u8; C]) -> Self {
        let mut data = vec![0u8; TILE_PIXELS * C];
        if px != [0; C] {
            for chunk in data.chunks_exact_mut(C) {
                chunk.copy_from_slice(&px);
            }
        }
        RESIDENT_BYTES.fetch_add(Self::BYTES, Ordering::Relaxed);
        Self { data: data.into_boxed_slice() }
    }

    /// Tuile reconstituée à partir de ses octets bruts (relecture du disque de travail) ; `None` si la longueur
    /// n'est pas celle d'une tuile de ce type.
    pub fn from_vec(data: Vec<u8>) -> Option<Self> {
        if data.len() != TILE_PIXELS * C {
            return None;
        }
        RESIDENT_BYTES.fetch_add(Self::BYTES, Ordering::Relaxed);
        Some(Self { data: data.into_boxed_slice() })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; C] {
        let i = (y * TILE_SIZE + x) as usize * C;
        let mut px = [0; C];
        px.copy_from_slice(&self.data[i..i + C]);
        px
    }

    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, px: [u8; C]) {
        let i = (y * TILE_SIZE + x) as usize * C;
        self.data[i..i + C].copy_from_slice(&px);
    }

    /// Vrai si tous les pixels valent `px` (la tuile peut alors ne pas être stockée).
    pub fn is_uniform(&self, px: [u8; C]) -> bool {
        self.data.chunks_exact(C).all(|chunk| chunk == px)
    }
}

impl Tile {
    pub fn transparent() -> Self {
        Self::filled([0; 4])
    }

    pub fn is_transparent(&self) -> bool {
        self.data.chunks_exact(4).all(|px| px[3] == 0)
    }

    /// Vrai si tous les pixels sont opaques (la composition peut ignorer ce qui est dessous).
    pub fn is_opaque(&self) -> bool {
        self.data.chunks_exact(4).all(|px| px[3] == 255)
    }
}

impl<const C: usize> Clone for TileBuf<C> {
    /// Copie à l'écriture (`Arc::make_mut`) : la copie est une nouvelle allocation, comptée comme telle.
    fn clone(&self) -> Self {
        RESIDENT_BYTES.fetch_add(Self::BYTES, Ordering::Relaxed);
        Self { data: self.data.clone() }
    }
}

impl<const C: usize> Drop for TileBuf<C> {
    fn drop(&mut self) {
        RESIDENT_BYTES.fetch_sub(Self::BYTES, Ordering::Relaxed);
    }
}

impl<const C: usize> std::fmt::Debug for TileBuf<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TileBuf<{C}>")
    }
}

/// Nombre de tuiles nécessaires pour couvrir `pixels` pixels.
pub const fn tiles_for(pixels: u32) -> u32 {
    pixels.div_ceil(TILE_SIZE)
}

/// Image en tuiles, creuse et sans limites : une tuile absente vaut uniformément `background`.
#[derive(Clone, Debug)]
pub struct Store<const C: usize> {
    background: [u8; C],
    tiles: HashMap<TileCoord, Arc<TileBuf<C>>>,
}

/// Pixels d'un calque. Tuile absente = transparente.
pub type TileStore = Store<4>;
/// Masque de fusion. Tuile absente = valeur de fond (255 « tout révéler » ou 0 « tout masquer »).
pub type MaskStore = Store<1>;

impl<const C: usize> Store<C> {
    pub fn with_background(background: [u8; C]) -> Self {
        Self { background, tiles: HashMap::new() }
    }

    pub fn background(&self) -> [u8; C] {
        self.background
    }

    /// Change la valeur de tout ce qui n'a pas de tuile (pour un masque : tout le plan hors des zones peintes).
    pub fn set_background(&mut self, value: [u8; C]) {
        self.background = value;
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    pub fn get(&self, coord: TileCoord) -> Option<&Arc<TileBuf<C>>> {
        self.tiles.get(&coord)
    }

    /// Accès en écriture. La tuile est créée (remplie du fond) si elle n'existe
    /// pas et dupliquée si elle est partagée (copie à l'écriture).
    pub fn get_mut(&mut self, coord: TileCoord) -> &mut TileBuf<C> {
        let background = self.background;
        let entry = self.tiles.entry(coord).or_insert_with(|| Arc::new(TileBuf::filled(background)));
        Arc::make_mut(entry)
    }

    /// Insère une tuile ; une tuile uniformément égale au fond supprime l'entrée.
    pub fn insert(&mut self, coord: TileCoord, tile: TileBuf<C>) {
        if tile.is_uniform(self.background) {
            self.tiles.remove(&coord);
        } else {
            self.tiles.insert(coord, Arc::new(tile));
        }
    }

    /// Remet une tuile à l'état partagé fourni (`None` = fond). Sert à restaurer un instantané.
    pub fn restore(&mut self, coord: TileCoord, tile: Option<Arc<TileBuf<C>>>) {
        match tile {
            Some(tile) => self.tiles.insert(coord, tile),
            None => self.tiles.remove(&coord),
        };
    }

    pub fn iter(&self) -> impl Iterator<Item = (TileCoord, &Arc<TileBuf<C>>)> {
        self.tiles.iter().map(|(coord, tile)| (*coord, tile))
    }

    /// Octets des tuiles de ce stockage qui ne figurent pas encore dans `seen` (adresses des tuiles déjà
    /// comptées), et les y ajoute. Plusieurs stockages partagent des tuiles (copie à l'écriture, tuiles pleines
    /// de `fill`) : en passant le même ensemble à chacun, on obtient ce qu'ils occupent réellement ensemble.
    pub fn unique_tile_bytes(&self, seen: &mut HashSet<usize>) -> u64 {
        self.tiles.values().filter(|tile| seen.insert(Arc::as_ptr(tile) as usize)).count() as u64 * TileBuf::<C>::BYTES
    }

    /// Coordonnées triées : ordre stable pour la sérialisation et les tests.
    pub fn sorted_coords(&self) -> Vec<TileCoord> {
        let mut coords: Vec<_> = self.tiles.keys().copied().collect();
        coords.sort_unstable_by_key(|c| (c.y, c.x));
        coords
    }

    /// Tuiles qui diffèrent entre deux états. La comparaison se fait sur les
    /// pointeurs `Arc` : coût proportionnel au nombre de tuiles, pas de pixels.
    pub fn changed_coords(&self, other: &Self) -> Vec<TileCoord> {
        let keys: HashSet<TileCoord> = self.tiles.keys().chain(other.tiles.keys()).copied().collect();
        keys.into_iter()
            .filter(|coord| match (self.tiles.get(coord), other.tiles.get(coord)) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            })
            .collect()
    }

    /// Rectangle englobant des tuiles stockées, en pixels : `(x0, y0, x1, y1)`, bornes hautes exclues.
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let size = TILE_SIZE as i32;
        self.tiles.keys().fold(None, |acc, c| {
            let (x0, y0) = c.origin();
            Some(match acc {
                None => (x0, y0, x0 + size, y0 + size),
                Some((ax0, ay0, ax1, ay1)) => (ax0.min(x0), ay0.min(y0), ax1.max(x0 + size), ay1.max(y0 + size)),
            })
        })
    }

    /// Rectangle englobant exact des pixels retenus par `counts` (par exemple « alpha non nul »),
    /// en pixels : `(x0, y0, x1, y1)`, bornes hautes exclues. Sert au cadre de transformation.
    pub fn pixel_bounds(&self, counts: impl Fn([u8; C]) -> bool) -> Option<(i32, i32, i32, i32)> {
        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        for (coord, tile) in &self.tiles {
            let (ox, oy) = coord.origin();
            for y in 0..TILE_SIZE {
                for x in 0..TILE_SIZE {
                    if !counts(tile.pixel(x, y)) {
                        continue;
                    }
                    let (px, py) = (ox + x as i32, oy + y as i32);
                    bounds = Some(match bounds {
                        None => (px, py, px + 1, py + 1),
                        Some((x0, y0, x1, y1)) => (x0.min(px), y0.min(py), x1.max(px + 1), y1.max(py + 1)),
                    });
                }
            }
        }
        bounds
    }

    /// Copie décalée de `(dx, dy)` pixels. Chaque tuile source se répartit sur quatre tuiles au plus.
    pub fn translated(&self, dx: i32, dy: i32) -> Self {
        let size = TILE_SIZE as i32;
        let mut out = Self::with_background(self.background);
        if dx.rem_euclid(size) == 0 && dy.rem_euclid(size) == 0 {
            // Décalage d'un nombre entier de tuiles : aucune copie de pixels.
            for (coord, tile) in &self.tiles {
                out.tiles.insert(TileCoord::new(coord.x + dx / size, coord.y + dy / size), Arc::clone(tile));
            }
            return out;
        }
        for (coord, tile) in &self.tiles {
            let (ox, oy) = coord.origin();
            let (x0, y0) = (ox + dx, oy + dy);
            let (first, _, _) = TileCoord::of_pixel(x0, y0);
            for ty in first.y..=first.y + 1 {
                for tx in first.x..=first.x + 1 {
                    let target = TileCoord::new(tx, ty);
                    let (tox, toy) = target.origin();
                    // Intersection de la tuile source décalée avec la tuile cible.
                    let (ix0, iy0) = (x0.max(tox), y0.max(toy));
                    let (ix1, iy1) = ((x0 + size).min(tox + size), (y0 + size).min(toy + size));
                    if ix0 >= ix1 || iy0 >= iy1 {
                        continue;
                    }
                    let width = (ix1 - ix0) as usize * C;
                    let destination = out.get_mut(target);
                    for y in iy0..iy1 {
                        let src = ((y - y0) * size + (ix0 - x0)) as usize * C;
                        let dst = ((y - toy) * size + (ix0 - tox)) as usize * C;
                        destination.bytes_mut()[dst..dst + width].copy_from_slice(&tile.bytes()[src..src + width]);
                    }
                }
            }
        }
        let coords: Vec<TileCoord> = out.tiles.keys().copied().collect();
        for coord in coords {
            if out.tiles.get(&coord).is_some_and(|t| t.is_uniform(out.background)) {
                out.tiles.remove(&coord);
            }
        }
        out
    }
}

/// Lecture pixel par pixel avec mémorisation de la dernière tuile : des accès
/// voisins ne refont pas de recherche dans la table.
pub struct Sampler<'a, const C: usize> {
    store: &'a Store<C>,
    coord: Option<TileCoord>,
    tile: Option<&'a Arc<TileBuf<C>>>,
}

impl<'a, const C: usize> Sampler<'a, C> {
    pub fn new(store: &'a Store<C>) -> Self {
        Self { store, coord: None, tile: None }
    }

    #[inline]
    pub fn get(&mut self, x: i32, y: i32) -> [u8; C] {
        let (coord, px, py) = TileCoord::of_pixel(x, y);
        if self.coord != Some(coord) {
            self.coord = Some(coord);
            self.tile = self.store.get(coord);
        }
        self.tile.map_or(self.store.background, |t| t.pixel(px, py))
    }
}

impl Store<1> {
    /// Vrai si au moins un pixel du rectangle `(x0, y0, x1, y1)` (bornes hautes exclues) est non nul.
    /// Lit les tuiles par lignes entières : un rectangle d'une tuile hors de la sélection est écarté
    /// bien plus vite qu'en l'échantillonnant pixel par pixel.
    pub fn any_nonzero_in(&self, (x0, y0, x1, y1): (i32, i32, i32, i32)) -> bool {
        if x0 >= x1 || y0 >= y1 {
            return false;
        }
        let size = TILE_SIZE as i32;
        let (first, _, _) = TileCoord::of_pixel(x0, y0);
        let (last, _, _) = TileCoord::of_pixel(x1 - 1, y1 - 1);
        for ty in first.y..=last.y {
            for tx in first.x..=last.x {
                let coord = TileCoord::new(tx, ty);
                let Some(tile) = self.tiles.get(&coord) else {
                    if self.background != [0] {
                        return true;
                    }
                    continue;
                };
                let (ox, oy) = coord.origin();
                let (from, to) = ((x0.max(ox) - ox) as usize, (x1.min(ox + size) - ox) as usize);
                for y in y0.max(oy)..y1.min(oy + size) {
                    let row = (y - oy) as usize * TILE_SIZE as usize;
                    if tile.bytes()[row + from..row + to].iter().any(|v| *v != 0) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Valeurs du rectangle `width x height` d'origine (0, 0), ligne par ligne ; le fond vaut là où il n'y a pas de tuile.
    pub fn to_bytes(&self, width: u32, height: u32) -> Vec<u8> {
        let mut sampler = Sampler::new(self);
        let mut out = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                out.push(sampler.get(x, y)[0]);
            }
        }
        out
    }

    /// Rectangle `(x0, y0, largeur, hauteur)`, un octet par pixel, ligne par ligne, copié tuile par tuile ; le fond
    /// vaut là où il n'y a pas de tuile. Alimente la composition sur carte graphique, qui reçoit les masques entiers.
    pub fn read_rect(&self, x0: i32, y0: i32, width: u32, height: u32) -> Vec<u8> {
        let (w, h) = (width as usize, height as usize);
        let mut out = vec![self.background[0]; w * h];
        if w == 0 || h == 0 {
            return out;
        }
        let size = TILE_SIZE as i32;
        let (x1, y1) = (x0 + width as i32, y0 + height as i32);
        let (first, _, _) = TileCoord::of_pixel(x0, y0);
        let (last, _, _) = TileCoord::of_pixel(x1 - 1, y1 - 1);
        for ty in first.y..=last.y {
            for tx in first.x..=last.x {
                let coord = TileCoord::new(tx, ty);
                let Some(tile) = self.get(coord) else { continue };
                let bytes = tile.bytes();
                let (ox, oy) = coord.origin();
                let (cx0, cy0, cx1, cy1) = (x0.max(ox), y0.max(oy), x1.min(ox + size), y1.min(oy + size));
                let row = (cx1 - cx0) as usize;
                for y in cy0..cy1 {
                    let src = ((y - oy) * size + (cx0 - ox)) as usize;
                    let dst = ((y - y0) as usize) * w + (cx0 - x0) as usize;
                    out[dst..dst + row].copy_from_slice(&bytes[src..src + row]);
                }
            }
        }
        out
    }
}

impl Store<4> {
    /// Rectangle `(x0, y0, largeur, hauteur)` en RGBA8 prémultiplié, ligne par ligne, copié tuile par tuile
    /// (des rangées entières d'un coup) : c'est ce qui alimente la composition sur carte graphique, qui en
    /// reçoit des calques entiers. Hors des tuiles présentes, tout est transparent.
    pub fn read_rect(&self, x0: i32, y0: i32, width: u32, height: u32) -> Vec<u8> {
        let (w, h) = (width as usize, height as usize);
        let mut out = vec![0u8; w * h * 4];
        if w == 0 || h == 0 {
            return out;
        }
        let size = TILE_SIZE as i32;
        let (x1, y1) = (x0 + width as i32, y0 + height as i32);
        let (first, _, _) = TileCoord::of_pixel(x0, y0);
        let (last, _, _) = TileCoord::of_pixel(x1 - 1, y1 - 1);
        for ty in first.y..=last.y {
            for tx in first.x..=last.x {
                let coord = TileCoord::new(tx, ty);
                let Some(tile) = self.get(coord) else { continue };
                let bytes = tile.bytes();
                let (ox, oy) = coord.origin();
                // Partie de la tuile qui tombe dans le rectangle.
                let (cx0, cy0, cx1, cy1) = (x0.max(ox), y0.max(oy), x1.min(ox + size), y1.min(oy + size));
                let row_bytes = (cx1 - cx0) as usize * 4;
                for y in cy0..cy1 {
                    let src = (((y - oy) * size + (cx0 - ox)) * 4) as usize;
                    let dst = (((y - y0) as usize) * w + (cx0 - x0) as usize) * 4;
                    out[dst..dst + row_bytes].copy_from_slice(&bytes[src..src + row_bytes]);
                }
            }
        }
        out
    }
}

impl Default for TileStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TileStore {
    pub fn new() -> Self {
        Self::with_background([0; 4])
    }

    /// Remplace tout le contenu par un rectangle `width x height` (origine en 0, 0) d'une couleur prémultipliée.
    pub fn fill(&mut self, width: u32, height: u32, px: [u8; 4]) {
        self.tiles.clear();
        if px[3] == 0 {
            return;
        }
        let full = Arc::new(Tile::filled(px));
        for ty in 0..tiles_for(height) {
            for tx in 0..tiles_for(width) {
                let coord = TileCoord::new(tx as i32, ty as i32);
                let w = (width - tx * TILE_SIZE).min(TILE_SIZE);
                let h = (height - ty * TILE_SIZE).min(TILE_SIZE);
                if w == TILE_SIZE && h == TILE_SIZE {
                    // Les tuiles pleines partagent le même bloc mémoire.
                    self.tiles.insert(coord, Arc::clone(&full));
                } else {
                    let mut tile = Tile::transparent();
                    for y in 0..h {
                        for x in 0..w {
                            tile.set_pixel(x, y, px);
                        }
                    }
                    self.tiles.insert(coord, Arc::new(tile));
                }
            }
        }
    }

    /// Découpe un tampon RGBA8 **prémultiplié** de `width * height` pixels.
    /// Retourne `None` si la longueur du tampon ne correspond pas.
    pub fn from_premultiplied(width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        if rgba.len() != width as usize * height as usize * 4 {
            return None;
        }
        let (tw, th) = (tiles_for(width), tiles_for(height));
        let stride = width as usize * 4;
        let tiles: HashMap<_, _> = (0..tw * th)
            .into_par_iter()
            .filter_map(|i| {
                let coord = TileCoord::new((i % tw) as i32, (i / tw) as i32);
                let (x0, y0) = (((i % tw) * TILE_SIZE) as usize, ((i / tw) * TILE_SIZE) as usize);
                let w = (width as usize - x0).min(TILE_SIZE as usize);
                let h = (height as usize - y0).min(TILE_SIZE as usize);
                let mut tile = Tile::transparent();
                for row in 0..h {
                    let src = (y0 + row) * stride + x0 * 4;
                    let dst = row * TILE_SIZE as usize * 4;
                    tile.bytes_mut()[dst..dst + w * 4].copy_from_slice(&rgba[src..src + w * 4]);
                }
                (!tile.is_transparent()).then(|| (coord, Arc::new(tile)))
            })
            .collect();
        Some(Self { background: [0; 4], tiles })
    }

    /// Recopie dans ce stockage le rectangle `(x0, y0, x1, y1)` de `other` (mêmes coordonnées). Le
    /// reste n'est pas touché : les tuiles hors du rectangle restent partagées avec l'original.
    pub fn paste(&mut self, other: &Self, (x0, y0, x1, y1): (i32, i32, i32, i32)) {
        let mut source = Sampler::new(other);
        for y in y0..y1 {
            for x in x0..x1 {
                let (coord, px, py) = TileCoord::of_pixel(x, y);
                let value = source.get(x, y);
                if self.get(coord).is_some() || value != [0; 4] {
                    self.get_mut(coord).set_pixel(px, py, value);
                }
            }
        }
    }

    /// Tampon RGBA8 prémultiplié du rectangle `width x height` d'origine (0, 0). Réservé à l'export et aux tests.
    pub fn to_premultiplied(&self, width: u32, height: u32) -> Vec<u8> {
        let stride = width as usize * 4;
        let mut out = vec![0u8; stride * height as usize];
        for ty in 0..tiles_for(height) {
            for tx in 0..tiles_for(width) {
                let Some(tile) = self.get(TileCoord::new(tx as i32, ty as i32)) else { continue };
                let (x0, y0) = ((tx * TILE_SIZE) as usize, (ty * TILE_SIZE) as usize);
                let w = (width as usize - x0).min(TILE_SIZE as usize);
                let h = (height as usize - y0).min(TILE_SIZE as usize);
                for row in 0..h {
                    let src = row * TILE_SIZE as usize * 4;
                    let dst = (y0 + row) * stride + x0 * 4;
                    out[dst..dst + w * 4].copy_from_slice(&tile.bytes()[src..src + w * 4]);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod read_rect_tests {
    use super::*;

    #[test]
    fn read_rect_matches_the_pixel_by_pixel_copy_across_tile_borders() {
        let mut store = TileStore::new();
        for y in -40..600 {
            for x in -20..700 {
                let (coord, px, py) = TileCoord::of_pixel(x, y);
                store.get_mut(coord).set_pixel(px, py, [(x & 0xff) as u8, (y & 0xff) as u8, ((x + y) & 0xff) as u8, 255]);
            }
        }
        // Rectangle à cheval sur plusieurs tuiles, coordonnées négatives comprises, et débordant sur du vide.
        let (x0, y0, w, h) = (-30, -50, 800, 700);
        let fast = store.read_rect(x0, y0, w, h);
        let slow = store.translated(-x0, -y0).to_premultiplied(w, h);
        assert_eq!(fast, slow);
        assert!(store.read_rect(0, 0, 0, 10).is_empty());
    }

    #[test]
    fn mask_read_rect_matches_the_sampler_and_fills_with_the_background() {
        let mut mask = MaskStore::with_background([200]);
        for y in -10..300 {
            for x in 250..520 {
                let (coord, px, py) = TileCoord::of_pixel(x, y);
                mask.get_mut(coord).set_pixel(px, py, [((x * 7 + y) & 0xff) as u8]);
            }
        }
        let (x0, y0, w, h) = (-5, -20, 600, 350);
        let fast = mask.read_rect(x0, y0, w, h);
        let mut sampler = Sampler::new(&mask);
        let slow: Vec<u8> = (y0..y0 + h as i32).flat_map(|y| (x0..x0 + w as i32).map(move |x| (x, y))).map(|(x, y)| sampler.get(x, y)[0]).collect();
        assert_eq!(fast, slow);
        assert_eq!(fast[0], 200, "hors des tuiles : le fond du masque");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resident_bytes_follow_allocations_clones_and_drops() {
        // Le compteur est global au processus et les autres tests allouent en même temps : on ne peut
        // affirmer que ce qui ne dépend pas d'eux. Tant que nos tuiles vivent, le compteur vaut au moins
        // leur taille…
        let ours: Vec<Tile> = (0..16).map(|_| Tile::transparent()).collect();
        assert!(resident_bytes() >= 16 * Tile::BYTES);
        let mut store = TileStore::new();
        store.fill(512, 256, [255; 4]);
        let shared = store.clone();
        // Une seule allocation : la tuile pleine est partagée par les deux positions et par les deux stockages.
        assert!(resident_bytes() >= 17 * Tile::BYTES);
        drop(shared);
        drop(ours);
        // … et, pour les deltas exacts, on réessaie jusqu'à tomber sur une fenêtre sans allocation concurrente.
        let exact = (0..50).any(|_| {
            let before = resident_bytes();
            let tiles: Vec<MaskTile> = (0..8).map(|_| MaskTile::filled([0])).collect();
            let mut copy = tiles[0].clone(); // copie à l'écriture : une allocation de plus
            copy.set_pixel(0, 0, [1]);
            let during = resident_bytes();
            drop(copy);
            drop(tiles);
            let after = resident_bytes();
            during == before + 9 * MaskTile::BYTES && after == before
        });
        assert!(exact, "le compteur suit les allocations, les copies et les libérations");
    }

    #[test]
    fn unique_tile_bytes_counts_shared_tiles_once() {
        let mut store = TileStore::new();
        store.fill(600, 256, [255; 4]); // deux tuiles pleines partagées + une tuile de bord
        let copy = store.clone();
        let mut seen = HashSet::new();
        assert_eq!(store.unique_tile_bytes(&mut seen), 2 * Tile::BYTES);
        assert_eq!(copy.unique_tile_bytes(&mut seen), 0, "la copie partage toutes ses tuiles");
        let mut mask = MaskStore::with_background([255]);
        mask.get_mut(TileCoord::new(0, 0)).set_pixel(0, 0, [0]);
        assert_eq!(mask.unique_tile_bytes(&mut seen), MaskTile::BYTES);
    }

    #[test]
    fn new_store_is_empty() {
        let store = TileStore::new();
        assert_eq!(store.tile_count(), 0);
        assert!(store.get(TileCoord::new(0, 0)).is_none());
        assert!(store.bounds().is_none());
    }

    #[test]
    fn pixel_addressing_handles_negative_coordinates() {
        assert_eq!(TileCoord::of_pixel(0, 255), (TileCoord::new(0, 0), 0, 255));
        assert_eq!(TileCoord::of_pixel(-1, 256), (TileCoord::new(-1, 1), 255, 0));
        assert_eq!(TileCoord::of_pixel(-257, -256), (TileCoord::new(-2, -1), 255, 0));
    }

    #[test]
    fn clone_shares_tiles_until_written() {
        let mut a = TileStore::new();
        a.get_mut(TileCoord::new(0, 0)).set_pixel(1, 1, [255; 4]);
        let mut b = a.clone();
        let coord = TileCoord::new(0, 0);
        assert!(Arc::ptr_eq(a.get(coord).expect("tuile a"), b.get(coord).expect("tuile b")));
        assert!(a.changed_coords(&b).is_empty());

        b.get_mut(coord).set_pixel(2, 2, [9, 9, 9, 255]);
        assert!(!Arc::ptr_eq(a.get(coord).expect("tuile a"), b.get(coord).expect("tuile b")));
        assert_eq!(a.get(coord).expect("tuile a").pixel(2, 2), [0; 4], "l'original est intact");
        assert_eq!(a.changed_coords(&b), vec![coord]);
    }

    #[test]
    fn buffer_roundtrip_with_partial_edge_tiles() {
        let (w, h) = (300u32, 270u32);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, (i % 7) as u8, 3, 255]).collect();
        let store = TileStore::from_premultiplied(w, h, &rgba).expect("tampon valide");
        assert_eq!(store.tile_count(), 4);
        assert_eq!(store.bounds(), Some((0, 0, 512, 512)));
        assert_eq!(store.to_premultiplied(w, h), rgba);
    }

    #[test]
    fn transparent_regions_are_not_stored() {
        let (w, h) = (1024u32, 256u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        rgba[3] = 255; // un seul pixel opaque, dans la première tuile
        let store = TileStore::from_premultiplied(w, h, &rgba).expect("tampon valide");
        assert_eq!(store.tile_count(), 1);
    }

    #[test]
    fn wrong_buffer_length_is_rejected() {
        assert!(TileStore::from_premultiplied(10, 10, &[0; 8]).is_none());
    }

    #[test]
    fn fill_shares_full_tiles() {
        let mut store = TileStore::new();
        store.fill(600, 256, [255; 4]);
        assert_eq!(store.tile_count(), 3);
        let a = store.get(TileCoord::new(0, 0)).expect("tuile 0");
        let b = store.get(TileCoord::new(1, 0)).expect("tuile 1");
        assert!(Arc::ptr_eq(a, b));
        let edge = store.get(TileCoord::new(2, 0)).expect("tuile de bord");
        assert_eq!(edge.pixel(87, 0), [255; 4]);
        assert_eq!(edge.pixel(88, 0), [0; 4], "hors image : transparent");
    }

    #[test]
    fn mask_store_uses_its_background_for_missing_tiles() {
        let mut mask = MaskStore::with_background([255]);
        let coord = TileCoord::new(1, 1);
        assert_eq!(mask.get_mut(coord).pixel(0, 0), [255], "nouvelle tuile = fond");
        mask.get_mut(coord).set_pixel(3, 3, [0]);
        assert_eq!(mask.tile_count(), 1);
        mask.insert(coord, MaskTile::filled([255]));
        assert_eq!(mask.tile_count(), 0, "une tuile égale au fond n'est pas stockée");
    }

    #[test]
    fn translation_moves_pixels_across_tiles_and_into_negative_space() {
        let mut store = TileStore::new();
        store.get_mut(TileCoord::new(0, 0)).set_pixel(10, 20, [1, 2, 3, 255]);
        store.get_mut(TileCoord::new(0, 0)).set_pixel(255, 255, [9, 9, 9, 255]);

        let moved = store.translated(-30, 300);
        let mut sampler = Sampler::new(&moved);
        assert_eq!(sampler.get(-20, 320), [1, 2, 3, 255]);
        assert_eq!(sampler.get(225, 555), [9, 9, 9, 255]);
        assert_eq!(sampler.get(10, 20), [0; 4]);
        assert_eq!(moved.tile_count(), 2, "seules les tuiles non vides sont gardées");

        let whole = store.translated(256, -512);
        let original = store.get(TileCoord::new(0, 0)).expect("source");
        assert!(Arc::ptr_eq(original, whole.get(TileCoord::new(1, -2)).expect("cible")), "décalage entier : tuiles partagées");
    }
}
