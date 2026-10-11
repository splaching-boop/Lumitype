//! Programme de composition d'une tuile : la liste des opérations à exécuter, sans dire qui les exécute.
//!
//! `render.rs` parcourt l'arbre des calques et en tire ce programme ; celui-ci est ensuite exécuté
//! soit par le processeur (`Program::run_on_cpu`, la référence), soit par la carte graphique (crate
//! `gpu`). Les deux suivent exactement la même suite d'opérations, dans le même ordre : c'est ce qui
//! permet de comparer leurs résultats pixel par pixel dans les tests.
//!
//! Le programme ne dépend d'aucune donnée d'image : les tuiles source, les masques et les réglages y
//! sont déjà résolus, et les seules valeurs calculées sont les couleurs. Les tampons de travail sont
//! numérotés ; le tampon 0 contient l'image finale.

use std::sync::Arc;

use crate::adjust::Adjustment;
use crate::blend::{BlendMode, blend_function, blend_premultiplied};
use crate::tiles::{MaskTile, TILE_SIZE, Tile, TileCoord};

/// Nombre de pixels d'une tuile.
pub const TILE_PIXELS: usize = (TILE_SIZE * TILE_SIZE) as usize;

/// Tampon de travail : une tuile en RGBA prémultiplié flottant, valeurs dans `0..=1`.
pub type Buf = Vec<[f32; 4]>;

/// Ce qui est fusionné : une tuile de calque, ou un tampon déjà composé (contenu d'un groupe).
#[derive(Clone, Debug)]
pub enum Source {
    Tile(Arc<Tile>),
    Buffer(u16),
}

/// Force d'une opération, pixel par pixel : le masque du calque, ou rien.
#[derive(Clone, Debug)]
pub enum Coverage {
    Uniform(f32),
    Mask(Arc<MaskTile>),
}

impl Coverage {
    #[inline]
    pub fn at(&self, index: usize) -> f32 {
        match self {
            Self::Uniform(v) => *v,
            Self::Mask(tile) => f32::from(tile.bytes()[index]) / 255.0,
        }
    }

    /// Vrai si la couverture vaut 1 partout (aucune opération à faire pour une interpolation).
    fn is_full(&self) -> bool {
        matches!(self, Self::Uniform(v) if *v >= 1.0)
    }
}

/// Forme de la base d'un écrêtage : ce que les calques écrêtés ont le droit de couvrir.
#[derive(Clone, Debug)]
pub enum Shape {
    /// L'alpha de la base elle-même (cas habituel).
    BaseAlpha,
    /// L'alpha des pixels bruts de la base, qui porte des effets débordant de sa forme.
    Raw(Arc<Tile>),
    /// Base à effets dont les pixels bruts sont absents sur cette tuile : rien n'est couvert.
    Empty,
}

/// Une opération de composition. `target` désigne toujours un tampon de travail.
#[derive(Clone, Debug)]
pub enum Op {
    /// Remet un tampon à transparent.
    Clear { target: u16 },
    /// Charge une tuile de calque dans un tampon (il est entièrement réécrit).
    Load { target: u16, source: Arc<Tile> },
    /// Recopie un tampon dans un autre (mémorise l'état avant un groupe en Transfert).
    Copy { target: u16, source: u16 },
    /// Fusionne une source sur le tampon, avec un mode, une opacité et un masque.
    Blend { target: u16, source: Source, mode: BlendMode, opacity: f32, mask: Coverage },
    /// Interpole le tampon vers l'état mémorisé : opacité et masque d'un groupe en Transfert.
    Mix { target: u16, before: u16, opacity: f32, mask: Coverage },
    /// Applique un calque de réglage au tampon (l'alpha ne change pas).
    Adjust { target: u16, adjustment: Adjustment, mode: BlendMode, opacity: f32, mask: Coverage },
    /// Fusionne un calque écrêté avec la couleur du tampon, sans toucher à son alpha.
    Clip { target: u16, source: Source, mode: BlendMode, opacity: f32, mask: Coverage, shape: Shape },
}

/// Ce qu'il faut faire pour obtenir une tuile composée.
#[derive(Clone, Debug)]
pub enum Program {
    /// Rien à calculer : la tuile d'un seul calque ordinaire, ou rien du tout (tuile vide).
    Ready(Option<Arc<Tile>>),
    /// Suite d'opérations ; le résultat est dans le tampon 0, qui part transparent.
    Ops { coord: TileCoord, buffers: u16, ops: Vec<Op> },
}

impl Program {
    /// Exécution par le processeur : c'est la référence à laquelle le GPU doit se conformer.
    pub fn run_on_cpu(&self) -> Option<Arc<Tile>> {
        match self {
            Self::Ready(tile) => tile.clone(),
            Self::Ops { coord, buffers, ops } => {
                let mut state = Machine { coord: *coord, buffers: vec![Vec::new(); *buffers as usize] };
                state.buffers[0] = vec![[0.0; 4]; TILE_PIXELS];
                for op in ops {
                    state.apply(op);
                }
                to_tile(&state.buffers[0])
            }
        }
    }

    /// Nombre d'opérations (mesures et tests).
    pub fn len(&self) -> usize {
        match self {
            Self::Ready(_) => 0,
            Self::Ops { ops, .. } => ops.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Bruit déterministe dans `0..1` pour le mode Dissoudre : stable d'un rendu à l'autre, et
/// reproduit à l'identique par le shader de la carte graphique.
#[inline]
pub fn dissolve_noise(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Alpha effectif de la source, Dissoudre compris : source prémultipliée prête à fusionner, ou rien.
#[inline]
fn prepare_source(coord: TileCoord, mode: BlendMode, px: [f32; 4], k: f32, index: usize) -> Option<[f32; 4]> {
    if px[3] <= 0.0 || k <= 0.0 {
        return None;
    }
    if mode != BlendMode::Dissolve {
        return Some([px[0] * k, px[1] * k, px[2] * k, px[3] * k]);
    }
    // Dissoudre : le pixel est soit entièrement présent, soit absent.
    let x = coord.x as u32 * TILE_SIZE + index as u32 % TILE_SIZE;
    let y = coord.y as u32 * TILE_SIZE + index as u32 / TILE_SIZE;
    (dissolve_noise(x, y) < px[3] * k).then(|| [px[0] / px[3], px[1] / px[3], px[2] / px[3], 1.0])
}

/// Convertit un tampon flottant en tuile 8 bits. `None` si tout est transparent.
pub fn to_tile(buf: &Buf) -> Option<Arc<Tile>> {
    if buf.iter().all(|px| px[3] <= 0.0) {
        return None;
    }
    let mut tile = Tile::transparent();
    for (dst, px) in tile.bytes_mut().chunks_exact_mut(4).zip(buf) {
        for c in 0..4 {
            dst[c] = (px[c] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
    Some(Arc::new(tile))
}

/// Exécution sur le processeur.
struct Machine {
    coord: TileCoord,
    buffers: Vec<Buf>,
}

impl Machine {
    fn ensure(&mut self, id: u16) {
        let slot = &mut self.buffers[id as usize];
        if slot.is_empty() {
            *slot = vec![[0.0; 4]; TILE_PIXELS];
        }
    }

    /// Parcourt les pixels d'une source et du tampon cible, sans jamais recopier l'un ni l'autre.
    fn walk(&mut self, target: u16, source: &Source, mut body: impl FnMut(usize, [f32; 4], &mut [f32; 4])) {
        self.ensure(target);
        let mut out = std::mem::take(&mut self.buffers[target as usize]);
        match source {
            Source::Tile(tile) => {
                for (index, (dst, px)) in out.iter_mut().zip(tile.bytes().chunks_exact(4)).enumerate() {
                    body(index, [px[0], px[1], px[2], px[3]].map(|v| f32::from(v) / 255.0), dst);
                }
            }
            Source::Buffer(id) => {
                self.ensure(*id);
                let content = std::mem::take(&mut self.buffers[*id as usize]);
                for (index, (dst, px)) in out.iter_mut().zip(&content).enumerate() {
                    body(index, *px, dst);
                }
                self.buffers[*id as usize] = content;
            }
        }
        self.buffers[target as usize] = out;
    }

    fn apply(&mut self, op: &Op) {
        match op {
            Op::Clear { target } => {
                self.ensure(*target);
                self.buffers[*target as usize].iter_mut().for_each(|px| *px = [0.0; 4]);
            }
            Op::Load { target, source } => {
                self.walk(*target, &Source::Tile(Arc::clone(source)), |_, px, dst| *dst = px);
            }
            Op::Copy { target, source } => {
                self.walk(*target, &Source::Buffer(*source), |_, px, dst| *dst = px);
            }
            Op::Blend { target, source, mode, opacity, mask } => {
                let (coord, mode, opacity) = (self.coord, *mode, *opacity);
                self.walk(*target, source, |index, px, dst| {
                    if let Some(src) = prepare_source(coord, mode, px, opacity * mask.at(index), index) {
                        *dst = blend_premultiplied(mode, src, *dst);
                    }
                });
            }
            Op::Mix { target, before, opacity, mask } => {
                let opacity = *opacity;
                self.walk(*target, &Source::Buffer(*before), |index, before, after| {
                    let k = opacity * mask.at(index);
                    for c in 0..4 {
                        after[c] = before[c] + (after[c] - before[c]) * k;
                    }
                });
            }
            Op::Adjust { target, adjustment, mode, opacity, mask } => {
                let prepared = adjustment.prepare();
                self.ensure(*target);
                for (index, px) in self.buffers[*target as usize].iter_mut().enumerate() {
                    let alpha = px[3];
                    let force = opacity * mask.at(index);
                    if alpha <= 0.0 || force <= 0.0 {
                        continue;
                    }
                    let base = [px[0] / alpha, px[1] / alpha, px[2] / alpha];
                    let corrected = prepared.apply_at(base, index);
                    let mixed = if *mode == BlendMode::Normal { corrected } else { blend_function(*mode, base, corrected) };
                    for c in 0..3 {
                        px[c] = (base[c] + (mixed[c] - base[c]) * force) * alpha;
                    }
                }
            }
            Op::Clip { target, source, mode, opacity, mask, shape } => {
                let (coord, mode, opacity) = (self.coord, *mode, *opacity);
                self.walk(*target, source, |index, px, dst| {
                    let base_alpha = dst[3];
                    if base_alpha <= 0.0 {
                        return;
                    }
                    let Some(src) = prepare_source(coord, mode, px, opacity * mask.at(index), index) else { return };
                    // Part de ce pixel qui appartient à la forme de la base (le reste est de l'ombre ou du halo).
                    let within = match shape {
                        Shape::BaseAlpha => 1.0,
                        Shape::Empty => 0.0,
                        Shape::Raw(tile) => (f32::from(tile.bytes()[index * 4 + 3]) / 255.0 / base_alpha).clamp(0.0, 1.0),
                    };
                    if within <= 0.0 {
                        return;
                    }
                    let sa = src[3] * within;
                    let s = [src[0] / src[3], src[1] / src[3], src[2] / src[3]];
                    let b = [dst[0] / base_alpha, dst[1] / base_alpha, dst[2] / base_alpha];
                    let mixed = blend_function(mode, b, s);
                    for c in 0..3 {
                        dst[c] = (b[c] + (mixed[c] - b[c]) * sa) * base_alpha;
                    }
                });
            }
        }
    }
}

/// Aide à la construction d'un programme : numérote les tampons et empile les opérations.
#[derive(Debug, Default)]
pub struct Builder {
    ops: Vec<Op>,
    /// Prochain tampon libre ; les tampons se prennent et se rendent en pile.
    next: u16,
    /// Plus grand nombre de tampons utilisés en même temps.
    peak: u16,
}

impl Builder {
    pub fn new() -> Self {
        // Le tampon 0 est l'image finale : toujours pris.
        Self { ops: Vec::new(), next: 1, peak: 1 }
    }

    /// Prend un tampon de travail (à rendre avec `release`).
    pub fn take(&mut self) -> u16 {
        let id = self.next;
        self.next += 1;
        self.peak = self.peak.max(self.next);
        id
    }

    pub fn release(&mut self, id: u16) {
        debug_assert_eq!(self.next, id + 1, "les tampons se rendent dans l'ordre inverse");
        self.next = id;
    }

    pub fn push(&mut self, op: Op) {
        self.ops.push(op);
    }

    /// Interpolation vers un état mémorisé, sans rien faire si la force vaut 1 partout.
    pub fn mix(&mut self, target: u16, before: u16, opacity: f32, mask: Coverage) {
        if opacity >= 1.0 && mask.is_full() {
            return;
        }
        self.push(Op::Mix { target, before, opacity, mask });
    }

    pub fn finish(self, coord: TileCoord) -> Program {
        Program::Ops { coord, buffers: self.peak, ops: self.ops }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(px: [u8; 4]) -> Arc<Tile> {
        Arc::new(Tile::filled(px))
    }

    #[test]
    fn a_program_blends_in_order_and_rounds_like_the_engine() {
        let mut builder = Builder::new();
        builder.push(Op::Blend { target: 0, source: Source::Tile(filled([255; 4])), mode: BlendMode::Normal, opacity: 1.0, mask: Coverage::Uniform(1.0) });
        builder.push(Op::Blend { target: 0, source: Source::Tile(filled([0, 0, 0, 255])), mode: BlendMode::Normal, opacity: 0.5, mask: Coverage::Uniform(1.0) });
        let tile = builder.finish(TileCoord::new(0, 0)).run_on_cpu().expect("tuile");
        assert_eq!(tile.pixel(4, 4), [128, 128, 128, 255]);
    }

    #[test]
    fn buffers_are_reused_in_a_stack() {
        let mut builder = Builder::new();
        let first = builder.take();
        let second = builder.take();
        builder.release(second);
        builder.release(first);
        let again = builder.take();
        assert_eq!((first, second, again), (1, 2, 1));
        assert!(matches!(builder.finish(TileCoord::new(0, 0)), Program::Ops { buffers: 3, .. }), "trois tampons au plus fort");
    }

    #[test]
    fn an_empty_program_gives_no_tile() {
        assert!(Builder::new().finish(TileCoord::new(0, 0)).run_on_cpu().is_none());
        assert!(Program::Ready(None).run_on_cpu().is_none());
    }
}
