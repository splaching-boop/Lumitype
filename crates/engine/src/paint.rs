//! Moteur de pinceau.
//!
//! Un trait accumule une **couverture** (masque 8 bits, maximum des empreintes),
//! puis chaque tuile touchée est recalculée depuis l'instantané d'avant le trait :
//! `résultat = mélange(instantané, couleur, couverture x opacité)`. Repasser sur
//! soi-même pendant un même trait ne renforce donc pas l'opacité, comme dans
//! Photoshop, et annuler le trait revient à reprendre l'instantané.
//!
//! Lissage : la souris ou le stylet ne donnent que des points espacés ; les relier par des
//! droites fait un trait anguleux dès qu'on va vite. Le trait passe donc par les MILIEUX des
//! points reçus, chaque point servant de point de contrôle d'une courbe quadratique : le tracé
//! est lisse, sans retard perceptible, et la pression est adoucie de la même façon.

use std::collections::BTreeSet;

use rayon::prelude::*;
use std::sync::Arc;

use crate::brushes::BrushTip;
use crate::document::{Document, LayerId};
use crate::error::EngineError;
use crate::pixel::{mul_div_255, premultiply};
use crate::tiles::{MaskStore, MaskTile, Sampler, Tile, TileCoord, TileStore, TILE_SIZE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushMode {
    Paint,
    Erase,
}

#[derive(Clone, Copy, Debug)]
pub struct Brush {
    /// Diamètre en pixels.
    pub size: f32,
    /// Dureté `0..=1` : part du rayon entièrement couverte.
    pub hardness: f32,
    /// Opacité du trait `0..=1`.
    pub opacity: f32,
    /// Couleur RGB à alpha droit. Sur un masque, seule sa luminosité compte.
    pub color: [u8; 3],
    pub mode: BrushMode,
    /// La pression du stylet module le diamètre.
    pub pressure_size: bool,
    /// La pression du stylet module l'opacité : on appuie peu, on dépose peu.
    pub pressure_opacity: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    /// `0..=1` ; 1 pour une souris.
    pub pressure: f32,
}

/// Forme du pinceau.
#[derive(Clone, Debug, Default)]
pub enum BrushKind {
    /// Rond, net ou flou selon la dureté : le trait est tracé d'un seul tenant.
    #[default]
    Round,
    /// Aérographe : nuage de fines gouttelettes, qui se densifie à chaque passage. Fait pour fondre des couleurs.
    Spray,
    /// Mélangeur : le pinceau part chargé de sa couleur, qui se mêle peu à peu à celles qu'il traverse.
    Mixer,
    /// Pointe importée (ABR, GBR, image), tamponnée le long du trait.
    Tip(Arc<BrushTip>),
}

/// Dynamique de forme : comment les empreintes varient le long du trait. Les valeurs nulles
/// donnent le comportement par défaut de chaque forme.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BrushDynamics {
    /// Espacement des empreintes, en fraction du diamètre ; 0 = celui de la forme.
    pub spacing: f32,
    /// Angle de la pointe, en degrés.
    pub angle: f32,
    /// La pointe s'oriente dans le sens du trait (l'angle s'y ajoute).
    pub follow_direction: bool,
    /// Dispersion : écart aléatoire des empreintes autour du trait, en fraction du diamètre (0 à 3).
    pub scatter: f32,
    /// Variation aléatoire de la taille (0 à 1 : jusqu'à -100 %).
    pub size_jitter: f32,
    /// Variation aléatoire de l'angle (0 à 1 : jusqu'à un tour complet).
    pub angle_jitter: f32,
    /// Spray : densité des gouttelettes (1 = normale). Mélangeur : densité de la peinture, c'est-à-dire
    /// la force avec laquelle chaque empreinte recouvre la toile (0 = valeur par défaut).
    pub amount: f32,
    /// Flux (0 à 1 ; 0 = 100 %) : en dessous de 100 %, les empreintes qui se recouvrent s'additionnent
    /// au lieu de se confondre. Le trait se construit en repassant, comme une peinture diluée.
    pub flow: f32,
    /// Mélangeur : quantité de peinture (0 à 1). À 0 le pinceau n'apporte aucune couleur et ne fait
    /// qu'étaler celles de la toile ; à 1 il peint sa couleur presque pure.
    pub paint: f32,
    /// Mélangeur : étirement des couleurs (0 à 1). Plus il est fort, plus les couleurs ramassées
    /// sont traînées loin avant de céder la place à celles que le pinceau rencontre.
    pub stretch: f32,
}

enum Snapshot {
    Pixels(Arc<TileStore>),
    Mask(Arc<MaskStore>),
    /// Masque rapide : le trait peint la sélection elle-même (repère document).
    Selection(Arc<MaskStore>),
}

/// Trait en cours sur un calque ou sur son masque.
pub struct Stroke {
    layer: LayerId,
    /// Position du calque au début du trait : la couverture est tenue en repère local.
    offset: (i32, i32),
    brush: Brush,
    snapshot: Snapshot,
    lock_transparency: bool,
    /// Sélection active au début du trait : la peinture ne déborde pas.
    selection: Option<Arc<MaskStore>>,
    coverage: MaskStore,
    /// Dernier point reçu (point de contrôle de la prochaine courbe) et dernier point réellement tracé.
    last: Option<StrokePoint>,
    drawn: Option<StrokePoint>,
    kind: BrushKind,
    /// Trait esquissé à l'écran par la carte graphique : le calque n'est recomposé qu'à la fin du trait,
    /// en une fois. Pendant le geste, seule la couverture est tenue à jour — c'est de loin le moins coûteux.
    deferred: bool,
    waiting: BTreeSet<TileCoord>,
    dynamics: BrushDynamics,
    /// Direction du trait à l'empreinte courante, en radians.
    heading: f32,
    /// Distance parcourue depuis la dernière empreinte (formes tamponnées).
    travelled: f32,
    /// Générateur pseudo-aléatoire du spray : déterministe, donc un même geste redonne le même grain.
    seed: u32,
    /// Mélangeur : ce que le pinceau porte — un carré de couleurs prémultipliées ramassées sur la toile,
    /// qui voyage avec lui — et les empreintes en attente d'être appliquées au document.
    carried: Option<Patch>,
    mixing: Vec<StrokePoint>,
}

impl Stroke {
    pub fn begin(doc: &Document, layer: LayerId, on_mask: bool, brush: Brush) -> Result<Self, EngineError> {
        let target = doc.layer(layer)?;
        if target.shape().is_some() && !on_mask {
            return Err(EngineError::InvalidOperation("calque de forme : pixellisez-le pour le peindre".into()));
        }
        if target.text().is_some() && !on_mask {
            return Err(EngineError::InvalidOperation("calque de texte : pixellisez-le pour le peindre".into()));
        }
        // Le masque de fusion d'un objet dynamique se peint normalement ; son contenu, non.
        if target.smart().is_some() && !on_mask {
            return Err(EngineError::InvalidOperation(crate::smart::NOT_PAINTABLE.into()));
        }
        if target.props.locks.all || target.props.locks.pixels {
            return Err(EngineError::InvalidOperation("les pixels de ce calque sont verrouillés".into()));
        }
        let snapshot = if on_mask {
            let mask = target.mask.as_ref().ok_or_else(|| EngineError::InvalidOperation("ce calque n'a pas de masque".into()))?;
            Snapshot::Mask(Arc::clone(&mask.pixels))
        } else {
            let pixels = target.pixels().ok_or_else(|| EngineError::InvalidOperation("impossible de peindre sur un groupe".into()))?;
            Snapshot::Pixels(Arc::clone(pixels))
        };
        Ok(Self {
            layer,
            offset: target.offset,
            brush,
            snapshot,
            lock_transparency: target.props.locks.transparency,
            selection: doc.selection().cloned(),
            coverage: MaskStore::with_background([0]),
            last: None,
            drawn: None,
            kind: BrushKind::Round,
            deferred: false,
            waiting: BTreeSet::new(),
            dynamics: BrushDynamics::default(),
            heading: 0.0,
            travelled: 0.0,
            seed: 0x9E37_79B9,
            carried: None,
            mixing: Vec::new(),
        })
    }

    /// Choisit la forme du pinceau, à faire juste après `begin`. Le mélangeur travaille sur les
    /// couleurs d'un calque : sur un masque ou une sélection, il se comporte en pinceau rond.
    /// Voir le champ `deferred`. Sans effet pour le mélangeur, qui écrit dans le calque à chaque empreinte.
    pub fn with_deferred(mut self, deferred: bool) -> Self {
        self.deferred = deferred && !matches!(self.kind, BrushKind::Mixer);
        self
    }

    pub fn with_dynamics(mut self, dynamics: BrushDynamics) -> Self {
        self.dynamics = dynamics;
        self
    }

    pub fn with_kind(mut self, kind: BrushKind) -> Self {
        let on_pixels = matches!(self.snapshot, Snapshot::Pixels(_));
        self.kind = if matches!(kind, BrushKind::Mixer) && (!on_pixels || self.brush.mode == BrushMode::Erase) { BrushKind::Round } else { kind };
        self
    }

    /// Masque rapide : on peint la sélection au pinceau. Mode Peinture = ajouter à la sélection,
    /// mode Gomme = retirer. Dureté et opacité du pinceau donnent une sélection progressive.
    pub fn begin_on_selection(doc: &Document, brush: Brush) -> Self {
        let before = doc.selection().cloned().unwrap_or_else(|| Arc::new(MaskStore::with_background([0])));
        Self {
            layer: LayerId::default(),
            offset: (0, 0),
            brush,
            snapshot: Snapshot::Selection(before),
            lock_transparency: false,
            selection: None, // la sélection est la cible : elle ne se limite pas elle-même
            coverage: MaskStore::with_background([0]),
            last: None,
            drawn: None,
            kind: BrushKind::Round,
            deferred: false,
            waiting: BTreeSet::new(),
            dynamics: BrushDynamics::default(),
            heading: 0.0,
            travelled: 0.0,
            seed: 0x9E37_79B9,
            carried: None,
            mixing: Vec::new(),
        }
    }

    pub fn on_selection(&self) -> bool {
        matches!(self.snapshot, Snapshot::Selection(_))
    }

    pub fn layer(&self) -> LayerId {
        self.layer
    }

    pub fn on_mask(&self) -> bool {
        matches!(self.snapshot, Snapshot::Mask(_))
    }

    /// Prolonge le trait (points en repère document) et applique le résultat au
    /// document. Retourne les tuiles **locales** du calque qui ont été modifiées.
    pub fn extend(&mut self, doc: &mut Document, points: &[StrokePoint]) -> Result<Vec<TileCoord>, EngineError> {
        let mut touched = BTreeSet::new();
        let local = |p: &StrokePoint| StrokePoint { x: p.x - self.offset.0 as f32, y: p.y - self.offset.1 as f32, ..*p };
        for point in &points.iter().map(local).collect::<Vec<_>>() {
            let (Some(control), Some(from)) = (self.last, self.drawn) else {
                self.dab(*point, &mut touched);
                (self.last, self.drawn) = (Some(*point), Some(*point));
                continue;
            };
            // Points quasi confondus : du bruit de capteur, qui ferait trembler la courbe.
            if (point.x - control.x).hypot(point.y - control.y) < 0.75 {
                continue;
            }
            // La pression brute varie par à-coups : moyenne glissante.
            let point = StrokePoint { pressure: (control.pressure + point.pressure) / 2.0, ..*point };
            let middle = StrokePoint { x: (control.x + point.x) / 2.0, y: (control.y + point.y) / 2.0, pressure: (control.pressure + point.pressure) / 2.0 };
            self.curve(from, control, middle, &mut touched);
            (self.last, self.drawn) = (Some(point), Some(middle));
        }
        if self.deferred {
            self.waiting.extend(touched);
            return Ok(Vec::new());
        }
        self.apply(doc, touched)
    }

    /// Fin du trait : le lissage garde un demi-segment d'avance, qu'il faut tracer jusqu'au dernier point reçu.
    pub fn finish(&mut self, doc: &mut Document) -> Result<Vec<TileCoord>, EngineError> {
        let mut touched = BTreeSet::new();
        if let (Some(from), Some(to)) = (self.drawn, self.last) {
            self.segment(from, to, &mut touched);
            self.drawn = Some(to);
        }
        touched.extend(std::mem::take(&mut self.waiting));
        self.apply(doc, touched)
    }

    /// Recompose les tuiles touchées (en parallèle : elles sont indépendantes) puis les pose dans le document.
    fn apply(&mut self, doc: &mut Document, mut touched: BTreeSet<TileCoord>) -> Result<Vec<TileCoord>, EngineError> {
        for at in std::mem::take(&mut self.mixing) {
            self.mix(doc, at, &mut touched)?;
        }
        let touched: Vec<TileCoord> = touched.into_iter().collect();
        let resolved: Vec<(TileCoord, Resolved)> = touched.par_iter().filter_map(|coord| self.resolve(*coord).map(|tile| (*coord, tile))).collect();
        for (coord, tile) in resolved {
            match tile {
                Resolved::Pixels(tile) => doc.pixels_mut(self.layer)?.insert(coord, tile),
                Resolved::Mask(tile) if self.on_selection() => doc.selection_mut().insert(coord, tile),
                Resolved::Mask(tile) => doc.mask_mut(self.layer)?.insert(coord, tile),
            }
        }
        Ok(touched)
    }

    /// Courbe quadratique `from -> to` de point de contrôle `control`, découpée en petits segments.
    fn curve(&mut self, from: StrokePoint, control: StrokePoint, to: StrokePoint, touched: &mut BTreeSet<TileCoord>) {
        let length = (control.x - from.x).hypot(control.y - from.y) + (to.x - control.x).hypot(to.y - control.y);
        // Longueur des segments selon la taille du pinceau : un gros pinceau tolère des segments
        // plus longs (l'écart à la courbe reste bien inférieur au pixel de bord), ce qui limite
        // le nombre de gélules à tracer, chacune couvrant une grande surface.
        let radius = self.radius(from.pressure.max(to.pressure));
        // Très gros pinceau : jusqu'à un cinquième du rayon par segment. L'écart à la courbe reste invisible
        // à cette échelle, et chaque gélule évitée épargne des millions de pixels à calculer.
        let step = (2.0 * radius.sqrt()).clamp(3.0, 64.0_f32.max(radius * 0.2));
        let steps = ((length / step).ceil() as usize).clamp(1, 64);
        let mut previous = from;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let u = 1.0 - t;
            let blend = |a: f32, b: f32, c: f32| u * u * a + 2.0 * u * t * b + t * t * c;
            let next = StrokePoint {
                x: blend(from.x, control.x, to.x),
                y: blend(from.y, control.y, to.y),
                pressure: blend(from.pressure, control.pressure, to.pressure),
            };
            self.segment(previous, next, touched);
            previous = next;
        }
    }

    fn radius(&self, pressure: f32) -> f32 {
        let scale = if self.brush.pressure_size { pressure.clamp(0.0, 1.0) } else { 1.0 };
        (self.brush.size * 0.5 * scale).max(0.5)
    }

    /// Trace le segment `from -> to` d'un seul tenant : une « gélule » (cercle balayé le long du
    /// segment), et non une suite d'empreintes rondes. Des empreintes espacées laissent un bord
    /// festonné, d'autant plus visible que le pinceau est gros ; la gélule a un bord exact, et
    /// coûte moins cher : l'intérieur de chaque ligne est rempli d'un bloc, seule la frange du
    /// bord est calculée pixel par pixel, et les tuiles sont traitées en parallèle.
    /// Pinceau rond flou (dureté < 100 %) : tracé en empreintes rondes rapprochées (10 % du
    /// diamètre) qui S'ACCUMULENT (chacune se pose par-dessus les précédentes). Le cœur du trait devient plein
    /// et le bord garde un fondu court, alors qu'une seule empreinte (un clic) reste douce. Le pinceau net
    /// garde la gélule : son bord est exact, sans festons.
    fn soft_round(&self) -> bool {
        matches!(self.kind, BrushKind::Round) && self.brush.hardness < 1.0
    }

    fn segment(&mut self, from: StrokePoint, to: StrokePoint, touched: &mut BTreeSet<TileCoord>) {
        if !matches!(self.kind, BrushKind::Round) || self.soft_round() {
            self.stamps(from, to, touched);
            return;
        }
        let hardness = self.brush.hardness.clamp(0.0, 1.0);
        let mut capsule = Capsule::new((from.x, from.y, self.radius(from.pressure)), (to.x, to.y, self.radius(to.pressure)), hardness);
        if self.brush.pressure_opacity {
            capsule.strength = Some((from.pressure.clamp(0.0, 1.0), to.pressure.clamp(0.0, 1.0)));
        }
        let (x0, y0, x1, y1) = capsule.bounds();
        let (first, _, _) = TileCoord::of_pixel(x0, y0);
        let (last, _, _) = TileCoord::of_pixel(x1 - 1, y1 - 1);
        let coords: Vec<TileCoord> = (first.y..=last.y).flat_map(|ty| (first.x..=last.x).map(move |tx| TileCoord::new(tx, ty))).collect();
        let coverage = &self.coverage;
        let updated: Vec<(TileCoord, MaskTile)> = coords
            .into_par_iter()
            .filter_map(|coord| {
                let existing = coverage.get(coord);
                if existing.is_some_and(|tile| tile.is_uniform([255])) {
                    return None; // déjà entièrement couverte
                }
                let mut tile = existing.map_or_else(|| MaskTile::filled([0]), |t| (**t).clone());
                capsule.stamp(coord, &mut tile).then_some((coord, tile))
            })
            .collect();
        for (coord, tile) in updated {
            self.coverage.insert(coord, tile);
            touched.insert(coord);
        }
    }

    /// Premier point du trait : une gélule réduite à un cercle, ou une première empreinte.
    fn dab(&mut self, at: StrokePoint, touched: &mut BTreeSet<TileCoord>) {
        if matches!(self.kind, BrushKind::Round) && !self.soft_round() {
            self.segment(at, at, touched);
        } else {
            self.stamp(at, touched);
        }
    }

    /// Formes tamponnées : empreintes régulièrement espacées le long du segment.
    fn stamps(&mut self, from: StrokePoint, to: StrokePoint, touched: &mut BTreeSet<TileCoord>) {
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        let length = dx.hypot(dy);
        if length <= f32::EPSILON {
            return;
        }
        let mut position = 0.0;
        loop {
            let pressure = from.pressure + (to.pressure - from.pressure) * (position / length).min(1.0);
            let diameter = self.radius(pressure) * 2.0;
            let default = match &self.kind {
                BrushKind::Tip(tip) => tip.spacing,
                BrushKind::Spray => 0.08,
                // Pinceau rond flou : empreintes tous les 10 % du diamètre qui s'accumulent, comme le trait de Photoshop.
                BrushKind::Round => 0.10,
                // Mélangeur : empreintes très serrées, chacune très légère : le fondu est continu, sans paliers.
                _ => 0.025,
            };
            let spacing = (diameter * if self.dynamics.spacing > 0.0 { self.dynamics.spacing.clamp(0.01, 5.0) } else { default }).max(1.0);
            self.heading = dy.atan2(dx);
            let step = spacing - self.travelled;
            if position + step > length {
                self.travelled += length - position;
                return;
            }
            position += step;
            self.travelled = 0.0;
            let t = position / length;
            self.stamp(StrokePoint { x: from.x + dx * t, y: from.y + dy * t, pressure: from.pressure + (to.pressure - from.pressure) * t }, touched);
        }
    }

    fn stamp(&mut self, mut at: StrokePoint, touched: &mut BTreeSet<TileCoord>) {
        let dynamics = self.dynamics;
        // Dispersion : l'empreinte s'écarte du trait, dans une direction et à une distance tirées au sort.
        if dynamics.scatter > 0.0 {
            let (angle, reach) = (self.random() * std::f32::consts::TAU, self.random() * dynamics.scatter.clamp(0.0, 3.0) * self.radius(at.pressure) * 2.0);
            (at.x, at.y) = (at.x + angle.cos() * reach, at.y + angle.sin() * reach);
        }
        let scale = 1.0 - self.random() * dynamics.size_jitter.clamp(0.0, 1.0);
        match self.kind.clone() {
            BrushKind::Round => self.round_dab(at, touched),
            BrushKind::Mixer => self.mixing.push(at), // appliqué au document en fin de lot
            BrushKind::Spray => self.spray(at, touched),
            BrushKind::Tip(tip) => {
                let turn = dynamics.angle.to_radians() + if dynamics.follow_direction { self.heading } else { 0.0 } + (self.random() - 0.5) * dynamics.angle_jitter.clamp(0.0, 1.0) * std::f32::consts::TAU;
                self.tip(&tip, at, scale.max(0.05), turn, touched);
            }
        }
    }

    fn random(&mut self) -> f32 {
        // xorshift32 : bien assez bon pour disperser des gouttelettes.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Flux effectif : 1 = les empreintes se confondent (maximum), moins = elles s'additionnent.
    fn flow(&self) -> f32 {
        if self.dynamics.flow > 0.0 { self.dynamics.flow.clamp(0.01, 1.0) } else { 1.0 }
    }

    /// Ajoute une couverture en un pixel (repère local).
    fn cover(&mut self, x: i32, y: i32, value: u8, touched: &mut BTreeSet<TileCoord>) {
        let flow = self.flow();
        let (coord, px, py) = TileCoord::of_pixel(x, y);
        let tile = self.coverage.get_mut(coord);
        let merged = merge(tile.pixel(px, py)[0], value, flow);
        if merged != tile.pixel(px, py)[0] {
            tile.set_pixel(px, py, [merged]);
            touched.insert(coord);
        }
    }

    /// Aérographe : gouttelettes réparties en cloche autour du point (denses au centre, rares au bord).
    fn spray(&mut self, at: StrokePoint, touched: &mut BTreeSet<TileCoord>) {
        let radius = self.radius(at.pressure);
        let density = if self.dynamics.amount > 0.0 { self.dynamics.amount.clamp(0.1, 4.0) } else { 1.0 };
        let drops = ((radius * radius * 0.012 * density) as usize).clamp(2, 1500);
        let softness = 1.0 - self.brush.hardness.clamp(0.0, 1.0) * 0.6;
        for _ in 0..drops {
            let angle = self.random() * std::f32::consts::TAU;
            // Moyenne de deux tirages : répartition triangulaire, proche d'une cloche.
            let distance = radius * ((self.random() + self.random()) / 2.0).powf(softness * 1.4);
            let (cx, cy) = (at.x + angle.cos() * distance, at.y + angle.sin() * distance);
            let size = (0.45 + self.random() * 0.9).max(0.5) * (1.0 + radius / 220.0);
            let strength = (0.35 + self.random() * 0.65) * if self.brush.pressure_opacity { at.pressure.clamp(0.0, 1.0) } else { 1.0 };
            let reach = size.ceil() as i32 + 1;
            for y in (cy as i32 - reach)..=(cy as i32 + reach) {
                for x in (cx as i32 - reach)..=(cx as i32 + reach) {
                    let d = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
                    let value = ((size - d + 0.5).clamp(0.0, 1.0) * strength * 255.0) as u8;
                    if value > 0 {
                        self.cover(x, y, value, touched);
                    }
                }
            }
        }
    }

    /// Pointe tamponnée : sa carte de couverture, mise à l'échelle du diamètre (le plus grand côté
    /// de la pointe), éventuellement réduite (`scale`) et tournée de `turn` radians.
    fn tip(&mut self, tip: &BrushTip, at: StrokePoint, scale: f32, turn: f32, touched: &mut BTreeSet<TileCoord>) {
        let diameter = self.radius(at.pressure) * 2.0 * scale;
        let unit = diameter / tip.width.max(tip.height) as f32;
        let (w, h) = (tip.width as f32 * unit, tip.height as f32 * unit);
        if w < 0.5 || h < 0.5 {
            return;
        }
        // Boîte englobante de la pointe tournée.
        let (sin, cos) = turn.sin_cos();
        let (half_w, half_h) = ((w * cos.abs() + h * sin.abs()) / 2.0, (w * sin.abs() + h * cos.abs()) / 2.0);
        let flow = self.flow();
        let strength = if self.brush.pressure_opacity { at.pressure.clamp(0.0, 1.0) } else { 1.0 };
        let size = TILE_SIZE as i32;
        let (x0, y0, x1, y1) = ((at.x - half_w).floor() as i32, (at.y - half_h).floor() as i32, (at.x + half_w).ceil() as i32, (at.y + half_h).ceil() as i32);
        let (first, _, _) = TileCoord::of_pixel(x0, y0);
        let (last, _, _) = TileCoord::of_pixel(x1, y1);
        for ty in first.y..=last.y {
            for tx in first.x..=last.x {
                let coord = TileCoord::new(tx, ty);
                let (ox, oy) = coord.origin();
                let mut changed = false;
                let tile = self.coverage.get_mut(coord);
                for y in y0.max(oy)..(y1 + 1).min(oy + size) {
                    for x in x0.max(ox)..(x1 + 1).min(ox + size) {
                        // Pixel ramené dans le repère de la pointe (rotation inverse autour du centre).
                        let (dx, dy) = (x as f32 + 0.5 - at.x, y as f32 + 0.5 - at.y);
                        let (u, v) = ((dx * cos + dy * sin) / w + 0.5, (dy * cos - dx * sin) / h + 0.5);
                        let value = (tip.sample(u, v) * strength * 255.0 + 0.5) as u8;
                        let (px, py) = ((x - ox) as u32, (y - oy) as u32);
                        let merged = merge(tile.pixel(px, py)[0], value, flow);
                        if merged != tile.pixel(px, py)[0] {
                            tile.set_pixel(px, py, [merged]);
                            changed = true;
                        }
                    }
                }
                if changed {
                    touched.insert(coord);
                }
            }
        }
    }

    /// Empreinte ronde floue, posée PAR-DESSUS la couverture existante (1 - (1 - a)(1 - b)) : c'est cette
    /// accumulation, et non le maximum, qui donne au trait flou de Photoshop son cœur plein et son bord court.
    fn round_dab(&mut self, at: StrokePoint, touched: &mut BTreeSet<TileCoord>) {
        // Numéro de l'empreinte, unique dans toute la session : il fait varier le tramage d'une empreinte à l'autre
        // ET d'un trait à l'autre. Sans cela, dix clics au même endroit répéteraient dix fois le même motif de
        // tramage, qui s'additionnerait en grain visible au bord de la tache.
        static SERIAL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x9E37_79B9);
        let serial = SERIAL.fetch_add(0x6C8E_9CF5, std::sync::atomic::Ordering::Relaxed);
        let radius = self.radius(at.pressure);
        let hard = radius * self.brush.hardness.clamp(0.0, 1.0);
        let strength = if self.brush.pressure_opacity { at.pressure.clamp(0.0, 1.0) } else { 1.0 } * self.flow();
        let size = TILE_SIZE as i32;
        let (x0, y0, x1, y1) = ((at.x - radius - 1.0).floor() as i32, (at.y - radius - 1.0).floor() as i32, (at.x + radius + 1.0).ceil() as i32, (at.y + radius + 1.0).ceil() as i32);
        let (first, _, _) = TileCoord::of_pixel(x0, y0);
        let (last, _, _) = TileCoord::of_pixel(x1, y1);
        let coords: Vec<TileCoord> = (first.y..=last.y).flat_map(|ty| (first.x..=last.x).map(move |tx| TileCoord::new(tx, ty))).collect();
        let coverage = &self.coverage;
        let updated: Vec<(TileCoord, MaskTile)> = coords
            .into_par_iter()
            .filter_map(|coord| {
                let existing = coverage.get(coord);
                if existing.is_some_and(|tile| tile.is_uniform([255])) {
                    return None;
                }
                let mut tile = existing.map_or_else(|| MaskTile::filled([0]), |t| (**t).clone());
                let (ox, oy) = coord.origin();
                let mut changed = false;
                for y in y0.max(oy)..(y1 + 1).min(oy + size) {
                    let dy = y as f32 + 0.5 - at.y;
                    for x in x0.max(ox)..(x1 + 1).min(ox + size) {
                        let dx = x as f32 + 0.5 - at.x;
                        let distance = (dx * dx + dy * dy).sqrt();
                        if distance >= radius + 0.5 {
                            continue;
                        }
                        let value = soft_falloff(distance, radius, hard) * strength * SOFT_FLOW;
                        if value <= 0.0 {
                            continue;
                        }
                        let (px, py) = ((x - ox) as u32, (y - oy) as u32);
                        let before = tile.pixel(px, py)[0];
                        let old = f32::from(before) / 255.0;
                        // Arrondi TRAMÉ plutôt qu'au plus proche : sur 256 niveaux, les dépôts minuscules des dizaines
                        // d'empreintes du bord d'un trait flou seraient tous arrondis à zéro (paliers, anneaux visibles).
                        // Un seuil pseudo-aléatoire, différent à chaque pixel et à chaque empreinte, les conserve en
                        // moyenne : le dégradé garde tous ses niveaux, sans motif régulier.
                        let noise = {
                            let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77) ^ serial;
                            h ^= h >> 15;
                            h = h.wrapping_mul(0x2C1B_3C6D);
                            h ^= h >> 12;
                            (h >> 8) as f32 / (1u32 << 24) as f32
                        };
                        let new = ((old + value * (1.0 - old)) * 255.0 + noise).floor().clamp(0.0, 255.0) as u8;
                        let new = new.max(before); // une couverture ne recule jamais
                        if new != before {
                            tile.set_pixel(px, py, [new]);
                            changed = true;
                        }
                    }
                }
                changed.then_some((coord, tile))
            })
            .collect();
        for (coord, tile) in updated {
            self.coverage.insert(coord, tile);
            touched.insert(coord);
        }
    }

    /// Mélangeur (dans l'esprit du « mélange de couleurs » de Clip Studio Paint). Le pinceau porte un
    /// carré de couleurs ramassées sur la toile, qui voyage avec lui. À chaque empreinte :
    ///   1. il dépose ce qu'il porte, teinté par sa propre couleur selon la **quantité de peinture**,
    ///      avec une force donnée par la **densité** ;
    ///   2. il se recharge de ce qu'il y avait sous lui AVANT le dépôt ; l'**étirement** dit quelle part
    ///      de l'ancien contenu il garde, donc jusqu'où les couleurs sont traînées.
    /// Tout se fait en alpha prémultiplié : le transparent se traîne comme une couleur, on peut donc
    /// aussi estomper le bord d'une forme. Il écrit directement dans le calque, chaque empreinte
    /// dépendant du résultat de la précédente.
    fn mix(&mut self, doc: &mut Document, at: StrokePoint, touched: &mut BTreeSet<TileCoord>) -> Result<(), EngineError> {
        let radius = self.radius(at.pressure);
        let hard = radius * self.brush.hardness.clamp(0.0, 1.0);
        let dynamics = self.dynamics;
        let pressure = if self.brush.pressure_opacity { at.pressure.clamp(0.0, 1.0) } else { 1.0 };
        // La densité demandée est celle du TRAIT : répartie sur les nombreuses empreintes qui se recouvrent
        // (une quarantaine par diamètre), chacune n'en dépose qu'une petite part.
        let wanted = (if dynamics.amount > 0.0 { dynamics.amount.clamp(0.0, 1.0) } else { 0.7 }) * self.brush.opacity.clamp(0.0, 1.0) * pressure;
        let density = 1.0 - (1.0 - wanted.min(0.98)).powf(1.0 / 6.0);
        let (paint, stretch) = (dynamics.paint.clamp(0.0, 1.0), dynamics.stretch.clamp(0.0, 0.98));
        let ink = self.brush.color.map(|c| f32::from(c) / 255.0);
        let offset = self.offset;
        let selection = self.selection.clone();
        let keep_alpha = self.lock_transparency;

        // Le carré porté : une grille d'au plus 160 cases de côté, quelle que soit la taille du pinceau.
        let reach = radius.ceil() as i32 + 1;
        let cell = ((2 * reach) as f32 / 160.0).ceil().max(1.0) as i32;
        let side = ((2 * reach) / cell + 1) as usize;
        let fresh = self.carried.as_ref().is_none_or(|patch| patch.side != side || patch.cell != cell);
        let mut patch = if fresh { Patch { side, cell, colors: vec![[0.0; 4]; side * side] } } else { self.carried.take().unwrap_or(Patch { side, cell, colors: Vec::new() }) };

        let pixels = doc.pixels_mut(self.layer)?;
        let (x0, y0) = (at.x.round() as i32 - reach, at.y.round() as i32 - reach);
        // Moyenne de la toile dans chaque case, relevée AVANT tout dépôt.
        let mut before = vec![[0.0f32; 4]; side * side];
        {
            let mut sampler = Sampler::new(pixels);
            for (index, slot) in before.iter_mut().enumerate() {
                let (cx, cy) = ((index % side) as i32, (index / side) as i32);
                let px = sampler.get(x0 + cx * cell + cell / 2, y0 + cy * cell + cell / 2);
                *slot = px.map(|v| f32::from(v) / 255.0);
            }
        }
        if fresh {
            // Première empreinte : le pinceau se charge, sans rien déposer encore.
            patch.colors = before;
            self.carried = Some(patch);
            return Ok(());
        }

        let size = TILE_SIZE as i32;
        let (first, _, _) = TileCoord::of_pixel(x0, y0);
        let (last, _, _) = TileCoord::of_pixel(x0 + 2 * reach, y0 + 2 * reach);
        let mut mask = selection.as_deref().map(Sampler::new);
        for ty in first.y..=last.y {
            for tx in first.x..=last.x {
                let coord = TileCoord::new(tx, ty);
                let (ox, oy) = coord.origin();
                let tile = pixels.get_mut(coord);
                let mut changed = false;
                for y in y0.max(oy)..(y0 + 2 * reach + 1).min(oy + size) {
                    for x in x0.max(ox)..(x0 + 2 * reach + 1).min(ox + size) {
                        let distance = (x as f32 + 0.5 - at.x).hypot(y as f32 + 0.5 - at.y);
                        let selected = mask.as_mut().map_or(1.0, |m| f32::from(m.get(x + offset.0, y + offset.1)[0]) / 255.0);
                        let force = falloff(distance, radius, hard) * density * selected;
                        if force <= 0.0 {
                            continue;
                        }
                        // Lecture interpolée du carré porté : pas de marches d'escalier entre ses cases.
                        let (gx, gy) = (((x - x0) as f32 + 0.5) / cell as f32 - 0.5, ((y - y0) as f32 + 0.5) / cell as f32 - 0.5);
                        let (cx, cy) = (gx.max(0.0) as usize, gy.max(0.0) as usize);
                        let (nx, ny) = ((cx + 1).min(side - 1), (cy + 1).min(side - 1));
                        let (tx, ty) = ((gx - cx as f32).clamp(0.0, 1.0), (gy - cy as f32).clamp(0.0, 1.0));
                        let cell_at = |ix: usize, iy: usize| patch.colors[iy.min(side - 1) * side + ix.min(side - 1)];
                        let (c00, c10, c01, c11) = (cell_at(cx, cy), cell_at(nx, cy), cell_at(cx, ny), cell_at(nx, ny));
                        let carried = [0, 1, 2, 3].map(|c| (c00[c] * (1.0 - tx) + c10[c] * tx) * (1.0 - ty) + (c01[c] * (1.0 - tx) + c11[c] * tx) * ty);
                        // Ce que le pinceau dépose : ses couleurs ramassées, teintées par sa propre peinture (opaque).
                        let deposit = [0, 1, 2, 3].map(|c| carried[c] * (1.0 - paint) + if c < 3 { ink[c] } else { 1.0 } * paint);
                        let (px, py) = ((x - ox) as u32, (y - oy) as u32);
                        let old = tile.pixel(px, py).map(|v| f32::from(v) / 255.0);
                        let mut new = [0, 1, 2, 3].map(|c| old[c] + (deposit[c] - old[c]) * force);
                        if keep_alpha {
                            // Transparence verrouillée : l'alpha d'origine reste, la couleur s'y adapte.
                            let scale = if new[3] > 1e-4 { old[3] / new[3] } else { 0.0 };
                            new = [new[0] * scale, new[1] * scale, new[2] * scale, old[3]];
                        }
                        let out = [0, 1, 2, 3].map(|c| (new[c].min(if c < 3 { new[3] } else { 1.0 }).clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                        if out != tile.pixel(px, py) {
                            tile.set_pixel(px, py, out);
                            changed = true;
                        }
                    }
                }
                if changed {
                    touched.insert(coord);
                }
            }
        }
        // Rechargement : une part de l'ancien contenu est gardée (étirement), le reste vient de la toile.
        // L'étirement se compte par diamètre parcouru, pas par empreinte : il ne dépend pas de leur nombre.
        let keep = stretch.powf(1.0 / 6.0);
        for (slot, canvas) in patch.colors.iter_mut().zip(&before) {
            for c in 0..4 {
                slot[c] = slot[c] * keep + canvas[c] * (1.0 - keep);
            }
        }
        // Les couleurs portées se mêlent un peu entre elles à chaque pas, comme des pigments humides :
        // c'est ce qui adoucit les transitions au lieu de traîner des bandes nettes.
        let blend = 0.2;
        let snapshot = patch.colors.clone();
        for cy in 0..side {
            for cx in 0..side {
                let at = |ix: usize, iy: usize| snapshot[iy.min(side - 1) * side + ix.min(side - 1)];
                let around = [at(cx.saturating_sub(1), cy), at(cx + 1, cy), at(cx, cy.saturating_sub(1)), at(cx, cy + 1)];
                let slot = &mut patch.colors[cy * side + cx];
                for c in 0..4 {
                    slot[c] += ((around[0][c] + around[1][c] + around[2][c] + around[3][c]) / 4.0 - slot[c]) * blend;
                }
            }
        }
        self.carried = Some(patch);
        Ok(())
    }

    /// Recalcule une tuile de la cible depuis l'instantané et la couverture.
    fn resolve(&self, coord: TileCoord) -> Option<Resolved> {
        let coverage = self.coverage.get(coord)?;
        let opacity = (self.brush.opacity.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        // La sélection est en repère document : force du trait pour chaque pixel de la tuile locale.
        let (ox, oy) = coord.origin();
        let size = TILE_SIZE as i32;
        let mut selected = self.selection.as_deref().map(Sampler::new);
        let strengths: Vec<u8> = coverage
            .bytes()
            .iter()
            .enumerate()
            .map(|(index, covered)| {
                let base = mul_div_255(*covered, opacity);
                match (&mut selected, base) {
                    (_, 0) | (None, _) => base,
                    (Some(sampler), _) => {
                        let (x, y) = (ox + index as i32 % size + self.offset.0, oy + index as i32 / size + self.offset.1);
                        mul_div_255(base, sampler.get(x, y)[0])
                    }
                }
            })
            .collect();
        let strength = |index: usize| strengths[index];

        match &self.snapshot {
            Snapshot::Pixels(before) => {
                let [r, g, b] = self.brush.color;
                let mut tile = before.get(coord).map_or_else(Tile::transparent, |t| (**t).clone());
                for (index, px) in tile.bytes_mut().chunks_exact_mut(4).enumerate() {
                    let t = strength(index);
                    if t == 0 {
                        continue;
                    }
                    let keep = 255 - t;
                    let alpha = px[3];
                    let mix = |ink: u8, under: u8| mul_div_255(ink, t).saturating_add(mul_div_255(under, keep));
                    let out = match self.brush.mode {
                        BrushMode::Erase if self.lock_transparency => continue,
                        BrushMode::Erase => [0, 1, 2, 3].map(|c| mul_div_255(px[c], keep)),
                        BrushMode::Paint if self.lock_transparency => {
                            // L'alpha est conservé : l'encre est prémultipliée par l'alpha existant.
                            let ink = premultiply([r, g, b, alpha]);
                            [mix(ink[0], px[0]), mix(ink[1], px[1]), mix(ink[2], px[2]), alpha]
                        }
                        BrushMode::Paint => {
                            let ink = premultiply([r, g, b, 255]);
                            [mix(ink[0], px[0]), mix(ink[1], px[1]), mix(ink[2], px[2]), mix(255, alpha)]
                        }
                    };
                    px.copy_from_slice(&out);
                }
                Some(Resolved::Pixels(tile))
            }
            Snapshot::Mask(before) | Snapshot::Selection(before) => {
                let [r, g, b] = self.brush.color.map(u32::from);
                let gray = match (self.brush.mode, self.on_selection()) {
                    (BrushMode::Paint, true) => 255, // masque rapide : peindre sélectionne…
                    (BrushMode::Erase, true) => 0,   // …gommer désélectionne
                    (BrushMode::Paint, false) => ((r * 30 + g * 59 + b * 11 + 50) / 100) as u8,
                    (BrushMode::Erase, false) => 255,
                };
                let background = before.background();
                let mut tile = before.get(coord).map_or_else(|| MaskTile::filled(background), |t| (**t).clone());
                for (index, value) in tile.bytes_mut().iter_mut().enumerate() {
                    let t = strength(index);
                    *value = mul_div_255(gray, t) + mul_div_255(*value, 255 - t);
                }
                Some(Resolved::Mask(tile))
            }
        }
    }
}

/// Carré de couleurs prémultipliées (0..1) porté par le mélangeur : `side` cases de côté, chacune de `cell` pixels.
struct Patch {
    side: usize,
    cell: i32,
    colors: Vec<[f32; 4]>,
}

/// Profil d'une empreinte de pinceau flou : une cloche gaussienne `exp(-(t / 0,55)²)` sur la zone de fondu,
/// ramenée à zéro au bord (81 % au quart du rayon, 42 % à mi-rayon, 12 % aux trois quarts). Ajustée sur une
/// série de clics de référence fournie par l'auteur.
pub(crate) fn soft_falloff(distance: f32, radius: f32, hard: f32) -> f32 {
    const WIDTH: f32 = 0.55;
    let edge = (radius - distance + 0.5).clamp(0.0, 1.0);
    if distance <= hard || radius - hard < 1.0 {
        return edge;
    }
    let t = ((distance - hard) / (radius - hard)).clamp(0.0, 1.0) / WIDTH;
    let tail = (-(1.0 / (WIDTH * WIDTH))).exp();
    edge.min((((-(t * t)).exp() - tail) / (1.0 - tail)).max(0.0))
}

/// Force d'une empreinte de pinceau flou : un tiers. Un clic seul dépose 33 % au centre ; les clics
/// suivants s'ajoutent (56 %, 70 %, 80 %… 96 % au huitième), jusqu'à un disque plein au bord doux — la
/// progression de la série de référence. Un trait (empreintes tous les 10 % du diamètre) fait environ
/// 83 % au centre en un passage et se construit, lui aussi, en repassant.
pub(crate) const SOFT_FLOW: f32 = 0.33;

/// Réunit une couverture existante et une nouvelle : maximum à plein flux, addition progressive sinon.
fn merge(existing: u8, value: u8, flow: f32) -> u8 {
    if flow >= 1.0 {
        return existing.max(value);
    }
    let (e, v) = (f32::from(existing) / 255.0, f32::from(value) / 255.0 * flow);
    ((e + v * (1.0 - e)) * 255.0 + 0.5) as u8
}

/// Couverture d'un trait rond passant par `points` (déjà en repère local), sans lissage : sert aux
/// outils qui ont besoin de la ZONE peinte plutôt que de peinture (correcteur localisé).
pub fn stroke_mask(points: &[(f32, f32)], size: f32, hardness: f32) -> MaskStore {
    let mut mask = MaskStore::with_background([0]);
    let radius = (size * 0.5).max(0.5);
    let pairs: Vec<((f32, f32), (f32, f32))> = match points {
        [] => Vec::new(),
        [only] => vec![(*only, *only)],
        _ => points.windows(2).map(|w| (w[0], w[1])).collect(),
    };
    for (from, to) in pairs {
        let capsule = Capsule::new((from.0, from.1, radius), (to.0, to.1, radius), hardness.clamp(0.0, 1.0));
        let (x0, y0, x1, y1) = capsule.bounds();
        let (first, _, _) = TileCoord::of_pixel(x0, y0);
        let (last, _, _) = TileCoord::of_pixel(x1 - 1, y1 - 1);
        for ty in first.y..=last.y {
            for tx in first.x..=last.x {
                let coord = TileCoord::new(tx, ty);
                let mut tile = mask.get(coord).map_or_else(|| MaskTile::filled([0]), |t| (**t).clone());
                if capsule.stamp(coord, &mut tile) {
                    mask.insert(coord, tile);
                }
            }
        }
    }
    mask
}

enum Resolved {
    Pixels(Tile),
    Mask(MaskTile),
}

/// Cercle de rayon variable balayé le long d'un segment. Rayons et positions en pixels du calque.
struct Capsule {
    a: (f32, f32),
    d: (f32, f32),
    length_sq: f32,
    radius: (f32, f32),
    hardness: f32,
    /// Pression -> opacité : force du dépôt aux deux extrémités ; `None` = plein partout.
    strength: Option<(f32, f32)>,
}

impl Capsule {
    fn new(from: (f32, f32, f32), to: (f32, f32, f32), hardness: f32) -> Self {
        let d = (to.0 - from.0, to.1 - from.1);
        Self { a: (from.0, from.1), d, length_sq: d.0 * d.0 + d.1 * d.1, radius: (from.2, to.2), hardness, strength: None }
    }

    fn bounds(&self) -> (i32, i32, i32, i32) {
        let r = self.radius.0.max(self.radius.1) + 1.0;
        let (bx, by) = (self.a.0 + self.d.0, self.a.1 + self.d.1);
        ((self.a.0.min(bx) - r).floor() as i32, (self.a.1.min(by) - r).floor() as i32, (self.a.0.max(bx) + r).ceil() as i32, (self.a.1.max(by) + r).ceil() as i32)
    }

    /// Intervalle des abscisses de la ligne `y` situées à moins de `rho` du segment. La gélule
    /// étant convexe, c'est la réunion des intervalles de ses deux cercles et de sa bande centrale.
    fn row(&self, y: f32, rho: f32) -> Option<(f32, f32)> {
        if rho <= 0.0 {
            return None;
        }
        let mut span: Option<(f32, f32)> = None;
        let mut join = |lo: f32, hi: f32| {
            if lo <= hi {
                span = Some(span.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
            }
        };
        for (cx, cy) in [self.a, (self.a.0 + self.d.0, self.a.1 + self.d.1)] {
            let dy = y - cy;
            if dy.abs() <= rho {
                let half = (rho * rho - dy * dy).sqrt();
                join(cx - half, cx + half);
            }
        }
        if self.length_sq > 1e-6 {
            let length = self.length_sq.sqrt();
            let (dx, dy) = self.d;
            let v = y - self.a.1;
            // Projection sur le segment comprise entre 0 et 1…
            let (mut lo, mut hi) = (f32::MIN, f32::MAX);
            if dx.abs() > 1e-6 {
                let (t0, t1) = (self.a.0 - v * dy / dx, self.a.0 + (self.length_sq - v * dy) / dx);
                (lo, hi) = (t0.min(t1), t0.max(t1));
            } else if !(0.0..=self.length_sq).contains(&(v * dy)) {
                return span;
            }
            // …et distance à la droite inférieure à `rho`.
            if dy.abs() > 1e-6 {
                let (p0, p1) = (self.a.0 + (v * dx - rho * length) / dy, self.a.0 + (v * dx + rho * length) / dy);
                (lo, hi) = (lo.max(p0.min(p1)), hi.min(p0.max(p1)));
            } else if (v * dx).abs() > rho * length {
                return span;
            }
            join(lo, hi);
        }
        span
    }

    /// Ajoute la gélule à une tuile de couverture (maximum). Retourne vrai si la tuile a changé.
    fn stamp(&self, coord: TileCoord, tile: &mut MaskTile) -> bool {
        let (ox, oy) = coord.origin();
        let size = TILE_SIZE as i32;
        let (r_min, r_max) = (self.radius.0.min(self.radius.1), self.radius.0.max(self.radius.1));
        // En deçà de ce rayon, la couverture vaut 255 quel que soit le point du segment le plus proche.
        let soft = r_min * (1.0 - self.hardness) >= 1.0;
        // Avec une force variable, plus de remplissage par blocs : chaque pixel est calculé.
        let full = if self.strength.is_some() { -1.0 } else if soft { (r_min * self.hardness).min(r_min - 0.5) } else { r_min - 0.5 };
        let bytes = tile.bytes_mut();
        let mut changed = false;
        for py in 0..size {
            let y = (oy + py) as f32 + 0.5;
            let Some((lo, hi)) = self.row(y, r_max + 0.5) else { continue };
            // Indices des pixels dont le CENTRE est dans l'intervalle, bornés à la tuile.
            let clip = |v: f32| ((v - 0.5).ceil() as i32 - ox).clamp(0, size);
            let (start, end) = (clip(lo), (((hi - 0.5).floor() as i32) - ox + 1).clamp(0, size));
            if start >= end {
                continue;
            }
            let (inner_start, inner_end) = match self.row(y, full) {
                Some((lo, hi)) => (clip(lo).max(start), ((((hi - 0.5).floor() as i32) - ox + 1).clamp(0, size)).min(end)),
                None => (end, end),
            };
            let line = &mut bytes[(py * size) as usize..((py + 1) * size) as usize];
            if inner_start < inner_end {
                let inner = &mut line[inner_start as usize..inner_end as usize];
                if inner.iter().any(|v| *v != 255) {
                    inner.fill(255);
                    changed = true;
                }
            }
            let (left_end, right_start) = if inner_start < inner_end { (inner_start, inner_end) } else { (end, end) };
            for px in (start..left_end).chain(right_start..end) {
                // Déjà plein : rien à gagner, on s'épargne tout le calcul (cas très fréquent au cœur d'un trait flou repassé).
                if line[px as usize] == 255 {
                    continue;
                }
                let x = (ox + px) as f32 + 0.5;
                let (vx, vy) = (x - self.a.0, y - self.a.1);
                let t = if self.length_sq > 1e-6 { ((vx * self.d.0 + vy * self.d.1) / self.length_sq).clamp(0.0, 1.0) } else { 0.0 };
                // Racine simple plutôt que `hypot`, dont les précautions contre le débordement coûtent cher ici, pixel par pixel.
                let (ex, ey) = (vx - self.d.0 * t, vy - self.d.1 * t);
                let distance = (ex * ex + ey * ey).sqrt();
                let radius = self.radius.0 + (self.radius.1 - self.radius.0) * t;
                let strength = self.strength.map_or(1.0, |(a, b)| a + (b - a) * t);
                let value = (falloff(distance, radius, radius * self.hardness) * strength * 255.0 + 0.5) as u8;
                if value > line[px as usize] {
                    line[px as usize] = value;
                    changed = true;
                }
            }
        }
        changed
    }
}

/// Profil d'une empreinte : plein jusqu'au rayon dur, décroissance en cosinus
/// ensuite, et un pixel d'antialiasing au bord d'un pinceau dur.
fn falloff(distance: f32, radius: f32, hard: f32) -> f32 {
    let edge = (radius - distance + 0.5).clamp(0.0, 1.0);
    if distance <= hard || radius - hard < 1.0 {
        return edge;
    }
    let t = ((distance - hard) / (radius - hard)).clamp(0.0, 1.0);
    // Profil en cloche (polynôme « smoothstep », sans fonction trigonométrique : c'est la ligne la plus
    // exécutée du pinceau flou) : une empreinte isolée est pleine au centre et vaut la moitié à mi-chemin du bord.
    edge.min(1.0 - t * t * (3.0 - 2.0 * t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Background;

    fn brush(opacity: f32) -> Brush {
        Brush { size: 20.0, hardness: 1.0, opacity, color: [255, 0, 0], mode: BrushMode::Paint, pressure_size: false, pressure_opacity: false }
    }

    fn point(x: f32, y: f32) -> StrokePoint {
        StrokePoint { x, y, pressure: 1.0 }
    }

    fn doc_with_layer() -> (Document, LayerId) {
        let mut doc = Document::with_background("x", 600, 300, 72.0, Background::White).expect("document");
        let id = doc.add_pixel_layer("peinture", TileStore::new(), None, 1).expect("calque");
        (doc, id)
    }

    /// Pixel du calque vu depuis le document (position du calque comprise).
    fn pixel(doc: &Document, id: LayerId, x: i32, y: i32) -> [u8; 4] {
        let layer = doc.layer(id).expect("calque");
        Sampler::new(layer.pixels().expect("pixels")).get(x - layer.offset.0, y - layer.offset.1)
    }

    #[test]
    fn stroke_paints_a_continuous_line_across_tiles() {
        let (mut doc, id) = doc_with_layer();
        let mut stroke = Stroke::begin(&doc, id, false, brush(1.0)).expect("trait");
        let mut touched = stroke.extend(&mut doc, &[point(200.0, 100.0), point(320.0, 100.0)]).expect("peinture");
        touched.extend(stroke.finish(&mut doc).expect("fin du trait"));
        touched.sort_unstable();
        touched.dedup();
        assert_eq!(touched, vec![TileCoord::new(0, 0), TileCoord::new(1, 0)]);
        for x in [200, 230, 255, 256, 290, 320] {
            assert_eq!(pixel(&doc, id, x, 100), [255, 0, 0, 255], "x = {x}");
        }
        assert_eq!(pixel(&doc, id, 260, 115), [0; 4], "hors du rayon");
    }

    #[test]
    fn overlapping_dabs_do_not_build_up_within_a_stroke() {
        let (mut doc, id) = doc_with_layer();
        let mut stroke = Stroke::begin(&doc, id, false, brush(0.5)).expect("trait");
        stroke.extend(&mut doc, &[point(50.0, 50.0), point(90.0, 50.0), point(50.0, 50.0)]).expect("aller-retour");
        stroke.finish(&mut doc).expect("fin du trait");
        assert_eq!(pixel(&doc, id, 70, 50), [128, 0, 0, 128]);
    }

    #[test]
    fn soft_brush_fades_towards_the_edge() {
        let (mut doc, id) = doc_with_layer();
        let soft = Brush { size: 40.0, hardness: 0.0, ..brush(1.0) };
        Stroke::begin(&doc, id, false, soft).expect("trait").extend(&mut doc, &[point(100.5, 100.5)]).expect("point");
        let (center, middle, edge) = (pixel(&doc, id, 100, 100)[3], pixel(&doc, id, 110, 100)[3], pixel(&doc, id, 119, 100)[3]);
        // Empreinte isolée : un tiers de la couleur au centre, 42 % de cela à mi-rayon, rien au bord.
        assert!(center.abs_diff(84) < 6 && middle.abs_diff(35) < 6 && edge < 4, "{center} {middle} {edge}");
    }

    #[test]
    fn eraser_and_transparency_lock() {
        let (mut doc, id) = doc_with_layer();
        doc.pixels_mut(id).expect("pixels").fill(600, 300, [0, 0, 255, 255]);
        let eraser = Brush { mode: BrushMode::Erase, ..brush(1.0) };
        Stroke::begin(&doc, id, false, eraser).expect("gomme").extend(&mut doc, &[point(50.5, 50.5)]).expect("gomme");
        assert_eq!(pixel(&doc, id, 50, 50), [0; 4]);

        doc.edit_props(id, |p| p.locks.transparency = true).expect("verrou");
        let mut stroke = Stroke::begin(&doc, id, false, brush(1.0)).expect("trait");
        stroke.extend(&mut doc, &[point(50.5, 50.5), point(80.5, 50.5)]).expect("peinture");
        stroke.finish(&mut doc).expect("fin du trait");
        assert_eq!(pixel(&doc, id, 50, 50), [0; 4], "la zone gommée reste transparente");
        assert_eq!(pixel(&doc, id, 80, 50), [255, 0, 0, 255], "la zone opaque est recolorée");
    }

    #[test]
    fn locked_pixels_refuse_painting() {
        let (mut doc, id) = doc_with_layer();
        doc.edit_props(id, |p| p.locks.pixels = true).expect("verrou");
        assert!(Stroke::begin(&doc, id, false, brush(1.0)).is_err());
    }

    #[test]
    fn painting_black_on_a_mask_hides_the_layer() {
        let (mut doc, id) = doc_with_layer();
        doc.add_mask(id, true).expect("masque");
        let black = Brush { color: [0, 0, 0], ..brush(1.0) };
        Stroke::begin(&doc, id, true, black).expect("trait").extend(&mut doc, &[point(30.5, 30.5)]).expect("masque");
        let mask = &doc.layer(id).expect("calque").mask.as_ref().expect("masque").pixels;
        let tile = mask.get(TileCoord::new(0, 0)).expect("tuile");
        assert_eq!(tile.pixel(30, 30), [0]);
        assert_eq!(tile.pixel(100, 100), [255]);
    }

    #[test]
    fn painting_stays_inside_the_selection() {
        use crate::selection::Shape;
        let (mut doc, id) = doc_with_layer();
        doc.set_selection(Some(Shape::Rect { x: 0.0, y: 0.0, width: 100.0, height: 300.0 }.rasterize(600, 300)));
        let mut stroke = Stroke::begin(&doc, id, false, brush(1.0)).expect("trait");
        stroke.extend(&mut doc, &[point(80.0, 50.0), point(130.0, 50.0)]).expect("peinture");
        stroke.finish(&mut doc).expect("fin du trait");
        assert_eq!(pixel(&doc, id, 95, 50), [255, 0, 0, 255]);
        assert_eq!(pixel(&doc, id, 105, 50), [0; 4], "hors sélection : intact");
    }

    #[test]
    fn painting_on_a_moved_layer_lands_under_the_cursor_and_may_leave_the_canvas() {
        let (mut doc, id) = doc_with_layer();
        doc.translate_layers(&[id], 37, -12).expect("déplacement");
        let mut stroke = Stroke::begin(&doc, id, false, brush(1.0)).expect("trait");
        let mut touched = stroke.extend(&mut doc, &[point(100.5, 100.5), point(-20.5, 100.5)]).expect("peinture");
        touched.extend(stroke.finish(&mut doc).expect("fin du trait"));
        assert_eq!(pixel(&doc, id, 100, 100), [255, 0, 0, 255], "sous le curseur, en repère document");
        assert_eq!(pixel(&doc, id, -20, 100), [255, 0, 0, 255], "hors cadre : conservé");
        assert!(touched.iter().any(|c| c.x < 0), "tuiles locales négatives : {touched:?}");
    }

    #[test]
    fn fast_strokes_are_rounded_instead_of_angular() {
        // Trois points à angle droit : un tracé en lignes brisées passerait par le coin (150, 50).
        let (mut doc, id) = doc_with_layer();
        let thin = Brush { size: 4.0, ..brush(1.0) };
        let mut stroke = Stroke::begin(&doc, id, false, thin).expect("trait");
        stroke.extend(&mut doc, &[point(50.0, 50.0), point(150.0, 50.0), point(150.0, 150.0)]).expect("peinture");
        stroke.finish(&mut doc).expect("fin du trait");
        assert_eq!(pixel(&doc, id, 149, 51)[3], 0, "le coin est arrondi : la pointe n'est pas peinte");
        assert!(pixel(&doc, id, 137, 62)[3] > 200, "la courbe passe à l'intérieur du coin, par (137,5 ; 62,5)");
        assert!(pixel(&doc, id, 51, 50)[3] > 200 && pixel(&doc, id, 150, 148)[3] > 200, "le trait part du premier point et atteint le dernier");
    }

    #[test]
    fn the_edge_of_a_large_stroke_is_perfectly_straight() {
        // Pinceau de 300 px tiré à l'horizontale sur 600 px : avec des empreintes espacées, le bord
        // supérieur ondulait de plus d'un pixel. Il doit maintenant être à la même hauteur partout.
        let (mut doc, id) = doc_with_layer();
        let big = Brush { size: 300.0, ..brush(1.0) };
        let mut stroke = Stroke::begin(&doc, id, false, big).expect("trait");
        stroke.extend(&mut doc, &[point(200.0, 300.0), point(500.0, 300.0), point(800.0, 300.0)]).expect("peinture");
        stroke.finish(&mut doc).expect("fin du trait");
        let top = |x: i32| (0..300).find(|y| pixel(&doc, id, x, *y)[3] > 127).expect("bord supérieur");
        let tops: Vec<i32> = (220..780).step_by(7).map(top).collect();
        assert!(tops.iter().all(|y| *y == 150), "bord ondulé : {tops:?}");
        assert_eq!(pixel(&doc, id, 500, 300), [255, 0, 0, 255], "intérieur plein");
        assert_eq!((pixel(&doc, id, 45, 300)[3], pixel(&doc, id, 955, 300)[3]), (0, 0), "extrémités rondes : rien au-delà du rayon");
        assert!(pixel(&doc, id, 52, 300)[3] > 200 && pixel(&doc, id, 948, 300)[3] > 200);
    }

    #[test]
    fn the_soft_edge_keeps_every_level_of_its_gradient() {
        // Bord d'un grand trait flou : une rampe douce, sans palier. On relève la couverture moyenne par tranche
        // de 4 px en s'éloignant de l'axe : elle doit décroître régulièrement, sans marche ni plateau figé.
        let (mut doc, id) = doc_with_layer();
        let soft = Brush { size: 400.0, hardness: 0.0, ..brush(1.0) };
        let mut stroke = Stroke::begin(&doc, id, false, soft).expect("trait");
        stroke.extend(&mut doc, &[point(100.0, 300.0), point(500.0, 300.0), point(900.0, 300.0), point(1300.0, 300.0)]).expect("peinture");
        stroke.finish(&mut doc).expect("fin");
        let band = |y0: i32| -> f32 { (y0..y0 + 4).flat_map(|y| (600..700).map(move |x| (x, y))).map(|(x, y)| f32::from(pixel(&doc, id, x, y)[3])).sum::<f32>() / 400.0 };
        let ramp: Vec<f32> = (0..40).map(|i| band(440 + i * 4)).collect();
        // Toute la zone faible (moins de 40 %) compte des valeurs distinctes à chaque tranche : aucun plateau.
        let faint: Vec<f32> = ramp.iter().copied().filter(|v| *v > 2.0 && *v < 100.0).collect();
        assert!(faint.len() >= 8, "une vraie zone de fondu : {ramp:?}");
        assert!(faint.windows(2).all(|w| w[1] < w[0]), "décroissance stricte, sans palier : {faint:?}");
    }

    #[test]
    fn soft_strokes_fade_with_the_distance_to_the_stroke() {
        let (mut doc, id) = doc_with_layer();
        let soft = Brush { size: 100.0, hardness: 0.0, ..brush(1.0) };
        let mut stroke = Stroke::begin(&doc, id, false, soft).expect("trait");
        stroke.extend(&mut doc, &[point(100.0, 200.0), point(300.0, 200.0), point(500.0, 200.0)]).expect("peinture");
        stroke.finish(&mut doc).expect("fin du trait");
        let alpha = |y| pixel(&doc, id, 300, y)[3];
        assert!(alpha(200) > 150 && alpha(175) > alpha(165) && alpha(165) > alpha(155) && alpha(149) == 0, "{} {} {} {}", alpha(200), alpha(175), alpha(165), alpha(155));
        // Empreintes espacées de 25 % : une légère ondulation du bord, comme dans Photoshop, mais pas de bosses.
        let profile: Vec<u8> = (150..450).step_by(3).map(|x| pixel(&doc, id, x, 170)[3]).collect();
        let (low, high) = (profile.iter().min().copied().unwrap_or(0), profile.iter().max().copied().unwrap_or(0));
        eprintln!("ondulation à 30 px de l'axe : {low}..{high}");
        assert!(high - low <= 24, "{profile:?}");
    }

    #[test]
    fn quick_mask_paints_and_erases_the_selection() {
        let (mut doc, _) = doc_with_layer();
        let value = |doc: &Document, x, y| doc.selection().map_or(0, |s| Sampler::new(s).get(x, y)[0]);
        let wide = Brush { size: 40.0, ..brush(1.0) };

        let mut add = Stroke::begin_on_selection(&doc, wide);
        add.extend(&mut doc, &[point(100.0, 100.0), point(200.0, 100.0), point(300.0, 100.0)]).expect("peinture");
        add.finish(&mut doc).expect("fin");
        assert_eq!((value(&doc, 200, 100), value(&doc, 200, 130), value(&doc, 50, 50)), (255, 0, 0), "la bande peinte est sélectionnée");

        let mut remove = Stroke::begin_on_selection(&doc, Brush { mode: BrushMode::Erase, ..wide });
        remove.extend(&mut doc, &[point(200.0, 60.0), point(200.0, 100.0), point(200.0, 140.0)]).expect("gomme");
        remove.finish(&mut doc).expect("fin");
        assert_eq!((value(&doc, 200, 100), value(&doc, 120, 100), value(&doc, 280, 100)), (0, 255, 255), "la gomme retire de la sélection");
    }

    #[test]
    fn spray_is_a_cloud_that_thickens_with_each_pass_and_fades_outwards() {
        let (mut doc, id) = doc_with_layer();
        let count = |doc: &Document, x0: i32, x1: i32| (x0..x1).flat_map(|x| (160..240).map(move |y| (x, y))).filter(|(x, y)| pixel(doc, id, *x, *y)[3] > 0).count();
        let wide = Brush { size: 120.0, hardness: 0.0, ..brush(1.0) };
        let mut stroke = Stroke::begin(&doc, id, false, wide).expect("trait").with_kind(BrushKind::Spray);
        stroke.extend(&mut doc, &[point(100.0, 200.0), point(300.0, 200.0), point(500.0, 200.0)]).expect("peinture");
        stroke.finish(&mut doc).expect("fin");
        let once = count(&doc, 200, 400);
        assert!(once > 300 && once < 80 * 200 * 8 / 10, "un nuage, pas un aplat : {once} pixels touchés sur 16000");
        let edge = (200..400).filter(|x| pixel(&doc, id, *x, 145)[3] > 0).count();
        let middle = (200..400).filter(|x| pixel(&doc, id, *x, 200)[3] > 0).count();
        assert!(middle > edge * 2, "dense au centre ({middle}), clairsemé au bord ({edge})");

        let mut again = Stroke::begin(&doc, id, false, wide).expect("trait").with_kind(BrushKind::Spray);
        again.seed = 12345; // un autre geste : d'autres gouttelettes
        again.extend(&mut doc, &[point(100.0, 200.0), point(300.0, 200.0), point(500.0, 200.0)]).expect("peinture");
        again.finish(&mut doc).expect("fin");
        assert!(count(&doc, 200, 400) > once * 13 / 10, "un second passage densifie le nuage");
    }

    #[test]
    fn imported_tips_are_stamped_at_the_brush_size() {
        // Pointe en croix 3 x 3 : on doit retrouver une croix de 90 px, pas un disque.
        let tip = Arc::new(BrushTip { name: "croix".into(), width: 3, height: 3, mask: vec![0, 255, 0, 255, 255, 255, 0, 255, 0], spacing: 5.0 });
        let (mut doc, id) = doc_with_layer();
        let mut stroke = Stroke::begin(&doc, id, false, Brush { size: 90.0, ..brush(1.0) }).expect("trait").with_kind(BrushKind::Tip(tip));
        stroke.extend(&mut doc, &[point(200.0, 200.0)]).expect("tampon");
        stroke.finish(&mut doc).expect("fin");
        let arms = [pixel(&doc, id, 200, 200)[3], pixel(&doc, id, 200, 165)[3], pixel(&doc, id, 235, 200)[3]];
        assert!(arms.iter().all(|a| *a > 240), "branches de la croix (la pointe est interpolée) : {arms:?}");
        assert_eq!((pixel(&doc, id, 165, 165)[3], pixel(&doc, id, 235, 235)[3]), (0, 0), "coins vides");
    }

    #[test]
    fn dynamics_rotate_scatter_and_space_the_stamps() {
        // Pointe en barre horizontale 8 x 1 : tournée de 90°, elle doit devenir verticale.
        let bar = Arc::new(BrushTip { name: "barre".into(), width: 8, height: 1, mask: vec![255; 8], spacing: 0.25 });
        let stamp = |dynamics: BrushDynamics, points: &[StrokePoint]| {
            let (mut doc, id) = doc_with_layer();
            let mut stroke = Stroke::begin(&doc, id, false, Brush { size: 80.0, ..brush(1.0) }).expect("trait").with_kind(BrushKind::Tip(Arc::clone(&bar))).with_dynamics(dynamics);
            stroke.extend(&mut doc, points).expect("tampon");
            stroke.finish(&mut doc).expect("fin");
            (doc, id)
        };
        let (flat, id) = stamp(BrushDynamics::default(), &[point(200.0, 200.0)]);
        assert!(pixel(&flat, id, 230, 200)[3] > 200 && pixel(&flat, id, 200, 230)[3] == 0, "barre horizontale");
        let (turned, id) = stamp(BrushDynamics { angle: 90.0, ..Default::default() }, &[point(200.0, 200.0)]);
        assert!(pixel(&turned, id, 200, 230)[3] > 200 && pixel(&turned, id, 230, 200)[3] == 0, "tournée de 90° : verticale");

        // Suivre la direction : sur un trait vertical, la barre s'aligne avec lui.
        let down = [point(200.0, 100.0), point(200.0, 300.0), point(200.0, 500.0)];
        let (follow, id) = stamp(BrushDynamics { follow_direction: true, ..Default::default() }, &down);
        assert_eq!(pixel(&follow, id, 225, 300)[3], 0, "la barre suit le trait au lieu de le barrer");

        // Espacement de 300 % : des empreintes séparées par du vide le long d'un trait horizontal.
        let across = [point(100.0, 200.0), point(400.0, 200.0), point(700.0, 200.0)];
        let (spaced, id) = stamp(BrushDynamics { spacing: 3.0, ..Default::default() }, &across);
        let gaps = (150..650).filter(|x| pixel(&spaced, id, *x, 200)[3] == 0).count();
        assert!(gaps > 250, "des vides entre les empreintes : {gaps}");

        // Dispersion : des empreintes s'écartent de l'axe du trait.
        let (scattered, id) = stamp(BrushDynamics { scatter: 1.5, ..Default::default() }, &across);
        let off_axis = (150..650).step_by(5).flat_map(|x| (60..340).step_by(5).map(move |y| (x, y))).filter(|(x, y)| (y - 200i32).abs() > 30 && pixel(&scattered, id, *x, *y)[3] > 0).count();
        assert!(off_axis > 20, "des empreintes doivent s'écarter de l'axe du trait : {off_axis}");
        let (straight, id) = stamp(BrushDynamics::default(), &across);
        assert!((150..650).all(|x| pixel(&straight, id, x, 240)[3] == 0), "sans dispersion, rien hors de l'axe");
    }

    /// Moitié gauche bleue, moitié droite jaune, le reste transparent.
    fn two_tone() -> (Document, LayerId) {
        let (mut doc, id) = doc_with_layer();
        let pixels = doc.pixels_mut(id).expect("pixels");
        for y in 100..300 {
            for x in 100..500 {
                let (coord, px, py) = TileCoord::of_pixel(x, y);
                pixels.get_mut(coord).set_pixel(px, py, if x < 300 { [0, 0, 255, 255] } else { [255, 255, 0, 255] });
            }
        }
        (doc, id)
    }

    fn mix_across(dynamics: BrushDynamics) -> (Document, LayerId) {
        let (mut doc, id) = two_tone();
        let white = Brush { size: 60.0, hardness: 0.6, color: [255, 255, 255], ..brush(1.0) };
        let mut stroke = Stroke::begin(&doc, id, false, white).expect("trait").with_kind(BrushKind::Mixer).with_dynamics(dynamics);
        stroke.extend(&mut doc, &[point(180.0, 200.0), point(260.0, 200.0), point(340.0, 200.0), point(420.0, 200.0)]).expect("mélange");
        stroke.finish(&mut doc).expect("fin");
        (doc, id)
    }

    #[test]
    fn the_mixer_smears_canvas_colours_like_wet_paint() {
        // Sans peinture : le pinceau ne fait qu'étaler. Le bleu est traîné dans le jaune, aucun blanc n'apparaît.
        let smear = BrushDynamics { amount: 0.9, paint: 0.0, stretch: 0.9, ..Default::default() };
        let (doc, id) = mix_across(smear);
        let dragged = pixel(&doc, id, 330, 200);
        assert!(dragged[2] > 90 && dragged[0] < 200, "du bleu entraîné au-delà de la frontière : {dragged:?}");
        let start = pixel(&doc, id, 200, 200);
        assert!(start[0] < 30 && start[2] > 220, "bleu sur bleu : rien ne change, aucune peinture n'est apportée : {start:?}");
        assert_eq!(pixel(&doc, id, 330, 260), [255, 255, 0, 255], "hors du trait, rien ne bouge");

        // Étirement faible : les couleurs sont abandonnées plus vite, le bleu va moins loin.
        let (short, id) = mix_across(BrushDynamics { stretch: 0.2, ..smear });
        assert!(pixel(&short, id, 380, 200)[2] < pixel(&doc, id, 380, 200)[2], "un étirement faible traîne moins loin");

        // Quantité de peinture : le pinceau apporte sa couleur (blanc), qui éclaircit le bleu traversé.
        let (painted, id) = mix_across(BrushDynamics { paint: 0.6, ..smear });
        let tinted = pixel(&painted, id, 240, 200);
        assert!(tinted[0] > 100 && tinted[1] > 100 && tinted[3] == 255, "le blanc du pinceau se mêle au bleu : {tinted:?}");
    }

    #[test]
    fn the_mixer_also_drags_transparency_unless_alpha_is_locked() {
        // Trait qui part du bleu et sort dans le vide, à gauche : le bleu est tiré hors de la forme.
        let smear = BrushDynamics { amount: 1.0, paint: 0.0, stretch: 0.9, ..Default::default() };
        let (mut doc, id) = two_tone();
        let tool = Brush { size: 60.0, hardness: 0.6, ..brush(1.0) };
        let mut stroke = Stroke::begin(&doc, id, false, tool).expect("trait").with_kind(BrushKind::Mixer).with_dynamics(smear);
        stroke.extend(&mut doc, &[point(180.0, 200.0), point(120.0, 200.0), point(60.0, 200.0), point(20.0, 200.0)]).expect("mélange");
        stroke.finish(&mut doc).expect("fin");
        let outside = pixel(&doc, id, 80, 200);
        assert!(outside[3] > 40 && outside[2] >= outside[0], "du bleu a été tiré dans le transparent : {outside:?}");
        assert!(outside[0] <= outside[3] && outside[2] <= outside[3], "alpha prémultiplié respecté");
    }

    #[test]
    fn flow_lets_overlapping_stamps_build_up() {
        // Pointe pleine à 40 % de couverture, empreintes très serrées : à plein flux elles se confondent
        // (40 % partout) ; à flux réduit elles s'additionnent et dépassent ce que donnerait une seule.
        let tip = Arc::new(BrushTip { name: "voile".into(), width: 2, height: 2, mask: vec![102; 4], spacing: 0.05 });
        let paint = |flow: f32| {
            let (mut doc, id) = doc_with_layer();
            let mut stroke = Stroke::begin(&doc, id, false, Brush { size: 60.0, ..brush(1.0) }).expect("trait").with_kind(BrushKind::Tip(Arc::clone(&tip))).with_dynamics(BrushDynamics { flow, ..Default::default() });
            stroke.extend(&mut doc, &[point(100.0, 200.0), point(200.0, 200.0), point(300.0, 200.0)]).expect("peinture");
            stroke.finish(&mut doc).expect("fin");
            pixel(&doc, id, 200, 200)[3]
        };
        assert!(paint(1.0).abs_diff(102) <= 2, "plein flux : les empreintes se confondent");
        assert!(paint(0.5) > 150, "flux à 50 % : elles s'additionnent ({})", paint(0.5));
        assert_eq!(merge(200, 100, 1.0), 200);
    }

    #[test]
    fn a_deferred_stroke_gives_the_same_picture_in_one_go_at_the_end() {
        let path = [point(60.0, 60.0), point(200.0, 90.0), point(340.0, 60.0), point(480.0, 200.0)];
        let paint = |deferred: bool| {
            let (mut doc, id) = doc_with_layer();
            let mut stroke = Stroke::begin(&doc, id, false, Brush { size: 90.0, hardness: 0.0, ..brush(0.8) }).expect("trait").with_deferred(deferred);
            let during = stroke.extend(&mut doc, &path).expect("peinture");
            let untouched = pixel(&doc, id, 200, 85)[3] == 0;
            stroke.finish(&mut doc).expect("fin");
            (doc.layer(id).expect("calque").pixels().expect("pixels").to_premultiplied(600, 400), during.is_empty(), untouched)
        };
        let (live, live_silent, _) = paint(false);
        let (deferred, silent, untouched) = paint(true);
        assert!(!live_silent && silent && untouched, "en différé, rien n'est recomposé pendant le geste");
        // Même image, au tramage près (il change d'un trait à l'autre) : quelques niveaux d'écart par endroits,
        // un écart moyen minuscule et aucun biais.
        let worst = live.iter().zip(&deferred).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        let mean = live.iter().zip(&deferred).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / live.len() as f64;
        let bias = live.iter().zip(&deferred).map(|(a, b)| f64::from(*a) - f64::from(*b)).sum::<f64>() / live.len() as f64;
        assert!(worst <= 8 && mean < 0.1 && bias.abs() < 0.02, "écart maximal {worst}, moyen {mean}, biais {bias}");
    }

    #[test]
    fn a_soft_brush_behaves_like_photoshop() {
        // Une empreinte isolée (un clic) : 90 % au quart du rayon, moitié à mi-rayon, faible aux trois quarts, rien au bord.
        assert!((soft_falloff(0.0, 100.0, 0.0) - 1.0).abs() < 1e-6 && (soft_falloff(25.0, 100.0, 0.0) - 0.806).abs() < 0.02);
        assert!((soft_falloff(50.0, 100.0, 0.0) - 0.416).abs() < 0.02 && (soft_falloff(75.0, 100.0, 0.0) - 0.124).abs() < 0.02);
        assert!(soft_falloff(99.0, 100.0, 0.0) < 0.01);
        // Dureté 50 % : plein jusqu'à mi-rayon, puis la même forme sur la moitié restante.
        assert!((soft_falloff(50.0, 100.0, 50.0) - 1.0).abs() < 1e-6 && (soft_falloff(62.5, 100.0, 50.0) - 0.806).abs() < 0.02);

        // Un trait : les empreintes, espacées de 25 % du diamètre, s'accumulent. À mi-rayon du trait, la
        // couverture dépasse nettement celle d'une empreinte seule ; au bord, elle reste faible.
        let (mut doc, id) = doc_with_layer();
        let soft = Brush { size: 200.0, hardness: 0.0, ..brush(1.0) };
        let mut stroke = Stroke::begin(&doc, id, false, soft).expect("trait");
        stroke.extend(&mut doc, &[point(100.0, 200.0), point(250.0, 200.0), point(400.0, 200.0), point(550.0, 200.0)]).expect("peinture");
        stroke.finish(&mut doc).expect("fin");
        let (core, edge) = (pixel(&doc, id, 300, 200)[3], pixel(&doc, id, 300, 290)[3]);
        // Cœur plein jusqu'à mi-rayon, fondu qui occupe le reste du cercle du pinceau.
        let (dense, fading) = (pixel(&doc, id, 300, 250)[3], pixel(&doc, id, 300, 275)[3]);
        eprintln!("trait : cœur {core}, 50 % du rayon {dense}, 75 % {fading}, 90 % {edge}");
        // Un vrai dégradé du cœur au bord : plein au centre, à peu près 60 % à mi-rayon, 20 % aux trois quarts.
        // Un passage : environ 83 % au centre, 47 % à mi-rayon, 15 % aux trois quarts (il se construit en repassant).
        assert!((195..235).contains(&core) && (95..145).contains(&dense) && (20..60).contains(&fading) && edge < 20, "{core} {dense} {fading} {edge}");

        // Un clic seul reste doux : moitié à mi-rayon.
        let (mut doc, id) = doc_with_layer();
        let mut click = Stroke::begin(&doc, id, false, soft).expect("clic");
        click.extend(&mut doc, &[point(300.0, 200.0)]).expect("clic");
        click.finish(&mut doc).expect("fin");
        let (quarter, half_way) = (pixel(&doc, id, 325, 200)[3], pixel(&doc, id, 350, 200)[3]);
        // Un clic seul : un tiers de la couleur au centre, comme la série de référence.
        let center = pixel(&doc, id, 300, 200)[3];
        assert!(center.abs_diff(84) < 6 && quarter.abs_diff(68) < 6 && half_way.abs_diff(35) < 6, "clic : {center} {quarter} {half_way}");
        // Pinceau net : inchangé, plein jusqu'au bord.
        assert!((falloff(90.0, 100.0, 100.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pressure_can_drive_opacity() {
        let (mut doc, id) = doc_with_layer();
        let pen = Brush { pressure_opacity: true, ..brush(1.0) };
        let mut stroke = Stroke::begin(&doc, id, false, pen).expect("trait");
        let soft = |x: f32, p: f32| StrokePoint { x, y: 100.0, pressure: p };
        stroke.extend(&mut doc, &[soft(50.0, 0.2), soft(150.0, 0.2), soft(250.0, 1.0), soft(350.0, 1.0)]).expect("peinture");
        stroke.finish(&mut doc).expect("fin");
        let (light, heavy) = (pixel(&doc, id, 70, 100)[3], pixel(&doc, id, 330, 100)[3]);
        // La pression est adoucie le long du trait : elle n'atteint pas tout à fait 100 % sur les derniers points.
        assert!(light < 90 && heavy > 150 && heavy > light * 3, "on appuie peu, on dépose peu : {light} puis {heavy}");
    }

    #[test]
    fn pressure_scales_the_diameter() {
        let (mut doc, id) = doc_with_layer();
        let pen = Brush { pressure_size: true, ..brush(1.0) };
        let light = StrokePoint { x: 100.5, y: 100.5, pressure: 0.25 };
        Stroke::begin(&doc, id, false, pen).expect("trait").extend(&mut doc, &[light]).expect("point");
        assert_eq!(pixel(&doc, id, 100, 100)[3], 255);
        assert_eq!(pixel(&doc, id, 106, 100)[3], 0, "rayon réduit à 2,5 px");
    }
}
