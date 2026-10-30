//! Aperçu d'un déplacement de calques par images en cache (comme le cache de projection de Krita ou de Photoshop).
//!
//! Déplacer un groupe de cinquante calques ne doit rien recomposer pendant le geste. L'image est découpée en
//! couches, chacune composée UNE fois par le moteur au début du geste, au niveau de zoom affiché :
//! - « dessous » : tout ce qui est composé avant les calques déplacés ;
//! - « déplacé » : les calques déplacés, composés à part (un groupe : son contenu aplati, ses effets, son opacité
//!   et son masque compris), qui suit la souris d'un bloc ;
//! - « dessus » : ce qui vient après, en une ou quelques couches, chacune fondue avec son mode — et les calques de
//!   réglage du dessus, appliqués par la carte graphique à tout ce qui est dessous, comme dans le moteur.
//!
//! Aucune couche ne dépend de la position des calques déplacés : elles restent bonnes tout le geste, et le moteur
//! ne reçoit qu'un seul déplacement, au relâchement, suivi d'une recomposition exacte.
//!
//! Le découpage est exact quand ce qui est déplacé et ce qui est au-dessus se fondent « normalement » les uns sur
//! les autres (le cas d'un groupe ordinaire). Quelques cas ne le sont qu'à peu près pendant le geste (`notes`) :
//! un groupe en Transfert dont le contenu a des modes de fusion ou des réglages (il est montré isolé), des effets
//! dans un autre mode que Normal (montrés comme s'ils étaient en Normal sur un fond vide), un dossier parent avec
//! opacité, masque ou mode, trop de couches au-dessus. Le rendu final, au relâchement, est toujours celui du moteur.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use serde::Serialize;

use crate::adjust::Adjustment;
use crate::blend::BlendMode;
use crate::document::{Document, Layer, LayerId};

/// Nombre maximal de couches au-dessus des calques déplacés (chacune est une image de la taille de l'écran sur la
/// carte graphique) : au-delà, le reste est réuni en une couche fondue normalement (approximation).
pub const MAX_ABOVE: usize = 4;

/// Nombre maximal de paquets pour le contenu d'un groupe en Transfert déplacé seul (voir `move_layout`) : au-delà,
/// le reste rejoint le dernier paquet, fondu normalement (approximation notée). Le mode portable de l'interface en
/// demande moins (`move_layout_packs`).
pub const MAX_PACKS: usize = 12;

/// Modes qui se regroupent sans rien changer à l'image : deux calques consécutifs dans l'un d'eux, composés ensemble
/// sur fond transparent puis fondus d'un bloc dans ce mode, donnent exactement les deux fusions successives, couverture
/// partielle comprise (pour Produit, (1 − a + A)(1 − b + B) se développe en la fusion de B sur A puis sur le fond ;
/// Superposition est son symétrique). Les autres modes (Incrustation, Densité…) dépendent de ce qui est dessous :
/// chacun garde sa couche.
fn groupable(mode: BlendMode) -> bool {
    matches!(mode, BlendMode::Normal | BlendMode::Multiply | BlendMode::Screen)
}

/// Genre d'une couche de l'aperçu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SliceKind {
    /// Ce qui est sous les calques déplacés.
    Below,
    /// Les calques déplacés.
    Moving,
    /// Des calques au-dessus, composés ensemble sur fond transparent.
    Layers,
    /// Un calque de réglage au-dessus : appliqué par la carte graphique à tout ce qui est dessous.
    Adjust,
}

/// Une couche de l'aperçu, du bas vers le haut.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveSlice {
    pub kind: SliceKind,
    /// Mode de fusion de la couche sur ce qui est dessous (Normal pour les couches composées ensemble).
    pub mode: BlendMode,
    /// Calques (hors groupes) composés dans la couche, réglages compris.
    #[serde(skip)]
    pub leaves: Vec<LayerId>,
    /// Calque de réglage (`Adjust`) ; calque ou groupe déplacé seul (`Moving`).
    pub layer: Option<LayerId>,
    /// Réglage (`Adjust`) : ses valeurs, son opacité × fond, et la valeur de son masque hors de l'image chargée.
    pub adjustment: Option<Adjustment>,
    pub opacity: f32,
    /// Réglage à masque actif : l'image de la couche est la couverture du masque (sinon, aucune image).
    pub masked: bool,
    pub mask_background: f32,
    /// Couche qui suit les calques déplacés (le « déplacé », ou un paquet du contenu d'un groupe en Transfert) :
    /// la carte graphique la décale (son image, ou le masque d'un réglage).
    pub moving: bool,
    /// Empreinte du contenu d'une couche fixe (0 pour une couche qui bouge) : mêmes calques, mêmes pixels (par
    /// version), mêmes réglages, mêmes positions, même image. Le moteur et l'interface s'en servent pour réutiliser
    /// une couche déjà composée d'un geste à l'autre. Envoyée en texte (un nombre de 64 bits ne tient pas en JavaScript).
    #[serde(serialize_with = "as_text")]
    pub fingerprint: u64,
}

fn as_text<S: serde::Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    if *value == 0 { serializer.serialize_none() } else { serializer.serialize_str(&format!("{value:016x}")) }
}

/// Découpage d'un déplacement en couches (voir le module).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveLayout {
    /// Calques déplacés de plus haut niveau (frères), du bas vers le haut.
    pub moving: Vec<LayerId>,
    /// Couches, du bas vers le haut : « dessous », « déplacé », puis celles du dessus.
    pub slices: Vec<MoveSlice>,
    /// Vrai si l'aperçu est l'image exacte du document déplacé (aux arrondis de la réduction près).
    pub exact: bool,
    /// Raisons pour lesquelles il ne l'est pas, pour l'indicateur de diagnostic.
    pub notes: Vec<String>,
    /// Paquets au plus pour le contenu d'un groupe en Transfert : à redonner pour les images des couches (même découpage).
    pub max_packs: usize,
    /// Calques opaques fixes du dessus (Normal, pleine opacité, sans masque ni effet, parents neutres) : là où l'un
    /// d'eux couvre une tuile, le « dessous » n'a pas à être composé (voir `TileRenderer::covered`).
    #[serde(skip)]
    pub occluders: Vec<LayerId>,
    /// Groupe déplacé seul dont le contenu aplati peut remplacer tout son sous-programme sur la carte graphique
    /// (même image : groupe isolé, ou en Transfert sur des calques normaux).
    pub collapsible: Option<LayerId>,
    /// Étendue (repère document, `x0, y0, x1, y1`) de ce qui est déplacé, effets compris (par tuiles : large).
    pub bounds: Option<(i32, i32, i32, i32)>,
}

impl MoveSlice {
    fn layers(kind: SliceKind, mode: BlendMode, leaves: Vec<LayerId>, layer: Option<LayerId>) -> Self {
        Self { kind, mode, leaves, layer, adjustment: None, opacity: 1.0, masked: false, mask_background: 1.0, moving: kind == SliceKind::Moving, fingerprint: 0 }
    }
}

/// Mode dans lequel un calque se fond sur ce qui est dessous (un groupe en Transfert : Normal).
fn fused_mode(layer: &Layer) -> BlendMode {
    if layer.props.blend == BlendMode::PassThrough { BlendMode::Normal } else { layer.props.blend }
}

fn masked(layer: &Layer) -> bool {
    layer.mask.as_ref().is_some_and(|m| m.enabled)
}

/// Groupe ou calque sans opacité réduite ni masque actif.
fn plain(layer: &Layer) -> bool {
    layer.props.opacity * layer.props.fill >= 1.0 && !masked(layer)
}

/// Effets fondus à part dans un autre mode que Normal (ombre en Produit…) : sur un fond vide, ils ne sont plus justes.
fn odd_effects(layer: &Layer) -> bool {
    layer.effects.iter().any(|e| e.enabled() && e.blend() != BlendMode::Normal)
}

impl Document {
    /// Calques (hors groupes) dans l'ordre de la composition, du bas vers le haut.
    fn composite_leaves(&self) -> Vec<LayerId> {
        self.walk_top_down().into_iter().rev().map(|(_, l)| l).filter(|l| !l.is_group()).map(|l| l.id).collect()
    }

    /// Calques (hors groupes) de ce calque et de tout son contenu.
    fn leaves_of(&self, id: LayerId, out: &mut Vec<LayerId>) {
        let Ok(layer) = self.layer(id) else { return };
        if layer.is_group() {
            for child in layer.children() {
                self.leaves_of(*child, out);
            }
        } else {
            out.push(id);
        }
    }

    /// Vrai si le contenu de ce groupe se compose à part puis se pose d'un bloc sans rien changer à l'image :
    /// rien qu'en mode Normal, sans réglage ni effet dans un autre mode, sous-groupes en Transfert sans opacité ni
    /// masque (même règle que `Pass::flattenable` du rendu, sans le minimum de deux calques).
    fn normal_content(&self, group: &Layer) -> bool {
        group.children().iter().filter_map(|id| self.layer(*id).ok()).filter(|l| l.props.visible).all(|layer| {
            if layer.is_group() {
                matches!(layer.props.blend, BlendMode::PassThrough | BlendMode::Normal)
                    && (layer.props.blend != BlendMode::PassThrough || plain(layer))
                    && self.normal_content(layer)
            } else {
                layer.adjustment().is_none() && layer.props.blend == BlendMode::Normal && !odd_effects(layer)
            }
        })
    }

    /// Découpage du déplacement de `ids` en couches (voir le module), ou `None` si l'aperçu par couches ne convient
    /// pas : sélection de pixels active (seuls ses pixels bougeraient), calque verrouillé, calques déplacés qui ne
    /// sont pas des frères voisins, écrêtage à cheval sur ce qui bouge et ce qui reste, calque de réglage déplacé.
    pub fn move_layout(&self, ids: &[LayerId]) -> Option<MoveLayout> {
        self.move_layout_packs(ids, MAX_PACKS)
    }

    /// Comme `move_layout`, le contenu d'un groupe en Transfert en `max_packs` paquets au plus (au moins un).
    pub fn move_layout_packs(&self, ids: &[LayerId], max_packs: usize) -> Option<MoveLayout> {
        let max_packs = max_packs.max(1);
        if self.selection().is_some() {
            return None;
        }
        // Comparaison (options de fusion) : le calque dépend de l'image exacte sous lui, que les couches de l'aperçu
        // ne reproduisent pas. Le déplacement est alors rendu normalement.
        if self.walk_top_down().iter().any(|(_, l)| l.props.visible && l.props.range().is_some()) {
            return None;
        }
        let top = self.top_level_selection(ids).ok()?;
        let first_id = *top.first()?;
        let parent = self.layer(first_id).ok()?.parent;
        let mut notes: Vec<String> = Vec::new();
        // Tout ce qui bouge : déplaçable, et pas de réglage déplacé seul (il n'a rien à montrer, son masque si).
        let mut stack = top.clone();
        while let Some(id) = stack.pop() {
            self.ensure_movable(id).ok()?;
            let layer = self.layer(id).ok()?;
            stack.extend_from_slice(layer.children());
        }
        for id in &top {
            let layer = self.layer(*id).ok()?;
            if layer.parent != parent || layer.adjustment().is_some() {
                return None;
            }
        }
        // Frères voisins, dans l'ordre de la pile.
        let siblings = self.children_of(parent).ok()?;
        let first = siblings.iter().position(|id| *id == first_id)?;
        let last = first + top.len() - 1;
        if siblings.get(first..=last) != Some(&top[..]) {
            return None;
        }
        // Écrêtage : un calque déplacé écrêté sur un calque qui reste, ou un calque qui reste écrêté sur un déplacé.
        if self.layer(siblings[first]).ok()?.props.clipped || siblings.get(last + 1).is_some_and(|id| self.layer(*id).is_ok_and(|l| l.props.clipped)) {
            return None;
        }

        let mut moving_leaves = Vec::new();
        for id in &top {
            self.leaves_of(*id, &mut moving_leaves);
        }
        let moving_set: HashSet<LayerId> = moving_leaves.iter().copied().collect();
        let order = self.composite_leaves();
        let start = order.iter().position(|id| moving_set.contains(id)).unwrap_or(order.len());
        let below: Vec<LayerId> = order[..start].to_vec();

        // Dossiers parents : l'aperçu pose les couches les unes sur les autres comme si les calques déplacés étaient
        // au premier niveau ; un parent avec opacité, masque ou mode le contredit.
        let mut ancestor = parent;
        while let Some(id) = ancestor {
            let group = self.layer(id).ok()?;
            if !plain(group) || !matches!(group.props.blend, BlendMode::PassThrough | BlendMode::Normal) {
                notes.push(format!("dossier parent « {} » avec opacité, masque ou mode de fusion", group.props.name));
                break;
            }
            ancestor = group.parent;
        }

        // Couche déplacée : un calque ou un groupe seul garde son mode ; plusieurs calques sont fondus normalement.
        let single = (top.len() == 1).then(|| self.layer(first_id).ok()).flatten();
        let mode = single.map_or(BlendMode::Normal, fused_mode);
        let mut collapsible = None;
        for id in &top {
            let layer = self.layer(*id).ok()?;
            if !layer.props.visible {
                continue;
            }
            if layer.is_group() {
                let isolated = layer.props.blend != BlendMode::PassThrough;
                let normal = self.normal_content(layer);
                if !isolated && !normal {
                    notes.push(format!("contenu du groupe « {} » fondu à part pendant le geste (modes de fusion ou réglages)", layer.props.name));
                }
                if top.len() == 1 && (isolated || normal) {
                    collapsible = Some(layer.id);
                }
            } else if odd_effects(layer) {
                notes.push(format!("effets de « {} » dans un autre mode que Normal", layer.props.name));
            }
            if top.len() > 1 && layer.props.blend != BlendMode::Normal && layer.props.blend != BlendMode::PassThrough {
                notes.push(format!("mode de fusion de « {} » appliqué à part", layer.props.name));
            }
        }

        // Groupe en Transfert déplacé seul dont le contenu ne se pose pas d'un bloc (modes, réglages, écrêtages) :
        // optimisé par paquets. Ses calques se fondent directement sur ce qui est dessous, comme dans le document ;
        // les calques normaux qui se suivent sont réunis en une couche, les autres gardent la leur (mode, réglage) —
        // toutes décalées ensemble. Même image que le document, couche par couche.
        let packs = single.filter(|g| {
            g.props.visible && g.is_group() && g.props.blend == BlendMode::PassThrough && plain(g) && !self.normal_content(g)
        });
        let mut slices = vec![MoveSlice::layers(SliceKind::Below, BlendMode::Normal, below, None)];
        if let Some(group) = packs {
            let note = format!("contenu du groupe « {} » fondu à part pendant le geste (modes de fusion ou réglages)", group.props.name);
            notes.retain(|n| *n != note);
            let mut units = Units { slices: Vec::new(), notes: &mut notes, kind: SliceKind::Moving, limit: max_packs, place: "dans le groupe déplacé" };
            self.units(group.children(), &mut units);
            slices.append(&mut units.slices);
        } else {
            slices.push(MoveSlice::layers(SliceKind::Moving, mode, moving_leaves.clone(), single.map(|l| l.id)));
        }
        // Couches du dessus : les frères qui suivent, puis ceux de chaque dossier parent, jusqu'au premier niveau.
        let mut above = Units { slices: Vec::new(), notes: &mut notes, kind: SliceKind::Layers, limit: MAX_ABOVE, place: "au-dessus" };
        let mut container = parent;
        let mut from = last + 1;
        loop {
            let list = self.children_of(container).ok()?;
            self.units(&list[from.min(list.len())..], &mut above);
            let Some(id) = container else { break };
            let group = self.layer(id).ok()?;
            container = group.parent;
            from = self.children_of(container).ok()?.iter().position(|c| *c == id)? + 1;
        }
        slices.append(&mut above.slices);

        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        for id in &moving_leaves {
            let layer = self.layer(*id).ok()?;
            // Réglage déplacé avec son groupe : c'est son masque qui bouge (ses tuiles peintes ; ailleurs, son fond).
            if layer.adjustment().is_some() {
                if let Some((x0, y0, x1, y1)) = layer.mask.as_ref().filter(|m| m.enabled).and_then(|m| m.pixels.bounds()) {
                    let (ox, oy) = layer.offset;
                    let here = (x0 + ox, y0 + oy, x1 + ox, y1 + oy);
                    bounds = Some(bounds.map_or(here, |b| (b.0.min(here.0), b.1.min(here.1), b.2.max(here.2), b.3.max(here.3))));
                }
                continue;
            }
            let Some((x0, y0, x1, y1)) = layer.pixels().and_then(|p| p.bounds()) else { continue };
            let reach = crate::effects::reach_enabled(&layer.effects);
            let (ox, oy) = layer.offset;
            let here = (x0 + ox - reach, y0 + oy - reach, x1 + ox + reach, y1 + oy + reach);
            bounds = Some(bounds.map_or(here, |b| (b.0.min(here.0), b.1.min(here.1), b.2.max(here.2), b.3.max(here.3))));
        }
        notes.dedup();
        for slice in &mut slices {
            if !slice.moving {
                slice.fingerprint = self.slice_fingerprint(slice);
            }
        }
        let occluders = slices.iter().filter(|s| s.kind == SliceKind::Layers && s.mode == BlendMode::Normal).flat_map(|s| s.leaves.iter().copied()).filter(|id| self.occludes(*id)).collect();
        Some(MoveLayout { moving: top, slices, exact: notes.is_empty(), notes, max_packs, occluders, collapsible, bounds })
    }

    /// Empreinte d'une couche fixe (voir `MoveSlice::fingerprint`) : ses calques et leurs dossiers parents, tout ce qui
    /// change leur image (pixels et masque par version de contenu, propriétés, position, effets, réglage, remplissage,
    /// forme, texte, objet dynamique), et le format du document. Jamais 0.
    fn slice_fingerprint(&self, slice: &MoveSlice) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (self.width(), self.height(), format!("{:?}", self.color_mode()), format!("{:?}", slice.kind), format!("{:?}", slice.mode)).hash(&mut hasher);
        let mut seen = HashSet::new();
        for id in &slice.leaves {
            let mut current = Some(*id);
            while let Some(id) = current {
                if !seen.insert(id) {
                    break;
                }
                let Ok(layer) = self.layer(id) else { break };
                (layer.id, layer.offset, format!("{:?}", layer.props), format!("{:?}", layer.effects), format!("{:?}", layer.artboard)).hash(&mut hasher);
                layer.pixels().map(|p| p.version()).hash(&mut hasher);
                layer.mask.as_ref().map(|m| (m.pixels.version(), m.enabled, m.pixels.background())).hash(&mut hasher);
                format!("{:?} {:?} {:?} {:?}", layer.adjustment(), layer.fill_content(), layer.shape(), layer.text()).hash(&mut hasher);
                layer.smart().map(|s| std::sync::Arc::as_ptr(s) as usize).hash(&mut hasher);
                format!("{:?}", layer.smart_transform()).hash(&mut hasher);
                layer.children().len().hash(&mut hasher);
                current = layer.parent;
            }
        }
        hasher.finish().max(1)
    }

    /// Calque qui cache entièrement ce qui est sous lui là où ses pixels sont opaques : pixels en mode Normal, pleine
    /// opacité, ni masque, ni effet, ni écrêtage, visible, et des dossiers parents neutres (Transfert ou Normal, pleine
    /// opacité, sans masque, visibles).
    fn occludes(&self, id: LayerId) -> bool {
        let Ok(layer) = self.layer(id) else { return false };
        if layer.is_group() || layer.adjustment().is_some() || layer.pixels().is_none() || !layer.props.visible || layer.props.clipped {
            return false;
        }
        if layer.props.blend != BlendMode::Normal || !plain(layer) || crate::effects::any_enabled(&layer.effects) {
            return false;
        }
        let mut parent = layer.parent;
        while let Some(pid) = parent {
            let Ok(group) = self.layer(pid) else { return false };
            if !group.props.visible || !plain(group) || !matches!(group.props.blend, BlendMode::PassThrough | BlendMode::Normal) {
                return false;
            }
            parent = group.parent;
        }
        true
    }

    /// Range dans `above` les calques de cette liste de frères (du bas vers le haut), comme `composite_list` les
    /// compose : un calque et ceux qui sont écrêtés sur lui forment une unité. Sert au-dessus des calques déplacés,
    /// et au contenu d'un groupe en Transfert déplacé (ses paquets).
    fn units(&self, list: &[LayerId], above: &mut Units<'_>) {
        let mut i = 0;
        while i < list.len() {
            let Ok(base) = self.layer(list[i]) else {
                i += 1;
                continue;
            };
            let mut end = i + 1;
            while end < list.len() && self.layer(list[end]).is_ok_and(|l| l.props.clipped) {
                end += 1;
            }
            let clipped = &list[i + 1..end];
            i = end;
            if !base.props.visible {
                continue;
            }
            if let Some(adjustment) = base.adjustment() {
                above.adjust(base, adjustment.clone());
                continue;
            }
            // Dossier ordinaire en Transfert : transparent, son contenu se range comme s'il était au même niveau.
            if base.is_group() && base.props.blend == BlendMode::PassThrough && clipped.is_empty() && plain(base) {
                self.units(base.children(), above);
                continue;
            }
            let mut leaves = Vec::new();
            self.leaves_of(base.id, &mut leaves);
            for id in clipped {
                self.leaves_of(*id, &mut leaves);
            }
            // Composée seule sur fond transparent puis fondue dans son mode, l'unité donne la même image que dans le
            // document, sauf effets dans un autre mode, ou groupe en Transfert (atténué) aux contenus non normaux.
            if base.is_group() {
                if base.props.blend == BlendMode::PassThrough && !self.normal_content(base) {
                    let note = format!("contenu du groupe « {} » {} fondu à part", base.props.name, above.place);
                    above.notes.push(note);
                }
            } else if odd_effects(base) {
                let note = format!("effets de « {} » {} dans un autre mode que Normal", base.props.name, above.place);
                above.notes.push(note);
            }
            above.layers(fused_mode(base), leaves);
        }
    }
}

/// Couches en cours de constitution : celles du dessus (`Layers`), ou les paquets d'un groupe déplacé (`Moving`).
struct Units<'a> {
    slices: Vec<MoveSlice>,
    notes: &'a mut Vec<String>,
    /// Genre des couches de calques produites.
    kind: SliceKind,
    /// Nombre maximal de couches.
    limit: usize,
    /// Où elles sont, pour les notes (« au-dessus », « dans le groupe déplacé »).
    place: &'static str,
}

impl Units<'_> {
    /// Plus de place pour une couche à part : le reste rejoint la dernière couche, fondu normalement.
    fn full(&mut self) -> bool {
        if self.slices.len() < self.limit {
            return false;
        }
        let note = format!("trop de couches {} : les dernières sont fondues normalement", self.place);
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
        true
    }

    /// Dernière couche, si elle est une couche de calques dans ce mode, qui se regroupe (`groupable`) : les calques
    /// qui suivent dans le même mode la rejoignent (même image, une couche de moins).
    fn last_in(&mut self, mode: BlendMode) -> Option<&mut MoveSlice> {
        let kind = self.kind;
        self.slices.last_mut().filter(|s| groupable(mode) && s.kind == kind && s.mode == mode && s.adjustment.is_none())
    }

    fn layers(&mut self, mode: BlendMode, leaves: Vec<LayerId>) {
        if let Some(last) = self.last_in(mode) {
            last.leaves.extend(leaves);
            return;
        }
        if self.full() {
            self.merge(leaves);
            return;
        }
        self.slices.push(MoveSlice::layers(self.kind, mode, leaves, None));
    }

    fn adjust(&mut self, layer: &Layer, adjustment: Adjustment) {
        if self.full() {
            self.merge(vec![layer.id]); // sans effet sur un fond vide : approximation notée par `full`
            return;
        }
        let mask = layer.mask.as_ref().filter(|m| m.enabled);
        self.slices.push(MoveSlice {
            kind: SliceKind::Adjust,
            mode: layer.props.blend,
            leaves: vec![layer.id],
            layer: Some(layer.id),
            adjustment: Some(adjustment),
            opacity: layer.props.opacity * layer.props.fill,
            masked: mask.is_some(),
            mask_background: mask.map_or(1.0, |m| f32::from(m.pixels.background()[0]) / 255.0),
            moving: self.kind == SliceKind::Moving,
            fingerprint: 0,
        });
    }

    /// Ajoute des calques à la dernière couche (couche pleine) ; la convertit en couche de calques si besoin.
    fn merge(&mut self, leaves: Vec<LayerId>) {
        let kind = self.kind;
        match self.slices.last_mut() {
            Some(last) if last.kind == kind => last.leaves.extend(leaves),
            _ => self.slices.push(MoveSlice::layers(kind, BlendMode::Normal, leaves, None)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::document::Background;
    use crate::plan::{Builder, Coverage, Op, Source};
    use crate::render::TileRenderer;
    use crate::tiles::{MaskTile, Tile, TileCoord, TileStore, TILE_SIZE};

    /// Calque d'un dégradé semi-transparent de `w` × `h` en `at` (contenu varié : un décalage se voit).
    fn add_patch(doc: &mut Document, parent: Option<LayerId>, at: (i32, i32), (w, h): (u32, u32), tint: [u8; 3]) -> LayerId {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let (x, y) = ((i as u32 % w) as u8, (i as u32 / w) as u8);
            let a = 120 + (x ^ y) % 120;
            let c = |v: u8| ((u16::from(v) * u16::from(a)) / 255) as u8;
            px.copy_from_slice(&[c(tint[0].wrapping_add(x)), c(tint[1].wrapping_add(y)), c(tint[2]), a]);
        }
        let pixels = TileStore::from_premultiplied(w, h, &rgba).expect("pixels");
        let index = doc.children_of(parent).expect("parent").len();
        let id = doc.add_pixel_layer("motif", pixels, parent, index).expect("calque");
        doc.translate_layers(&[id], at.0, at.1).expect("placé");
        id
    }

    /// Recompose l'image à partir des couches de l'aperçu prises AVANT le déplacement, la couche déplacée décalée de
    /// `(dx, dy)`, exactement comme la carte graphique les assemble : chaque couche fondue dans son mode, les
    /// réglages appliqués à ce qui est dessous.
    fn from_slices(r: &TileRenderer, doc: &Document, layout: &MoveLayout, coord: TileCoord, (dx, dy): (i32, i32)) -> Option<Arc<Tile>> {
        let mut builder = Builder::new();
        for (index, slice) in layout.slices.iter().enumerate() {
            // Couches qui suivent les calques déplacés (le « déplacé », les paquets d'un groupe, le masque d'un réglage
            // du groupe) : prises AVANT le déplacement, à l'endroit d'où vient ce qui est maintenant sur cette tuile.
            let tile = if slice.moving {
                let (ox, oy) = coord.origin();
                let bytes = r.move_slice_region(doc, layout, index, 0, (ox - dx, oy - dy, TILE_SIZE, TILE_SIZE), true);
                let tile = Tile::from_vec(bytes);
                if slice.kind == SliceKind::Adjust { tile.map(Arc::new) } else { tile.filter(|t| !t.is_transparent()).map(Arc::new) }
            } else {
                r.move_slice_tile(doc, layout, index, 0, coord, true)
            };
            match slice.kind {
                SliceKind::Adjust => {
                    let mask = tile.map(|t| MaskTile::from_vec(t.bytes().chunks_exact(4).map(|px| px[0]).collect()).expect("masque"));
                    let mask = mask.map_or(Coverage::Uniform(1.0), |m| Coverage::Mask(Arc::new(m)));
                    builder.push(Op::Adjust { target: 0, adjustment: slice.adjustment.clone().expect("réglage"), mode: slice.mode, opacity: slice.opacity, mask });
                }
                _ => {
                    if let Some(tile) = tile {
                        builder.push(Op::Blend { target: 0, source: Source::Tile(tile), mode: slice.mode, opacity: 1.0, mask: Coverage::Uniform(1.0), range: None });
                    }
                }
            }
        }
        builder.finish(coord).run_on_cpu()
    }

    fn worst(a: Option<Arc<Tile>>, b: Option<Arc<Tile>>) -> u8 {
        let (a, b) = (a.map(|t| t.bytes().to_vec()).unwrap_or_else(|| vec![0; TILE_SIZE as usize * TILE_SIZE as usize * 4]), b.map(|t| t.bytes().to_vec()).unwrap_or_else(|| vec![0; TILE_SIZE as usize * TILE_SIZE as usize * 4]));
        a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
    }

    /// Un document ordinaire : fond, un calque dessous, un groupe de calques, puis au-dessus un calque en Produit,
    /// un réglage masqué et un calque normal.
    fn sample() -> (Document, LayerId, LayerId) {
        let mut doc = Document::with_background("x", 600, 420, 72.0, Background::White).expect("document");
        add_patch(&mut doc, None, (30, 40), (400, 300), [200, 40, 10]);
        let group = doc.add_group("G", None, doc.children_of(None).expect("racine").len()).expect("groupe");
        for (k, at) in [(0, (100, 60)), (1, (180, 120)), (2, (260, 30))] {
            add_patch(&mut doc, Some(group), at, (220, 200), [k * 70, 120, 200 - k * 50]);
        }
        let multiply = add_patch(&mut doc, None, (300, 200), (260, 180), [90, 200, 120]);
        doc.edit_props(multiply, |p| p.blend = BlendMode::Multiply).expect("mode");
        let index = doc.children_of(None).expect("racine").len();
        let invert = doc.add_adjustment_layer(Adjustment::Invert, None, index, false).expect("réglage");
        doc.add_mask(invert, false).expect("masque");
        for y in 0..TILE_SIZE {
            for x in 0..TILE_SIZE {
                doc.mask_mut(invert).expect("masque").get_mut(TileCoord::new(0, 0)).set_pixel(x, y, [(x as u8) / 2 + 60]);
            }
        }
        doc.edit_props(invert, |p| p.opacity = 0.8).expect("opacité");
        add_patch(&mut doc, None, (20, 260), (300, 140), [10, 10, 240]);
        (doc, group, multiply)
    }

    #[test]
    fn the_cached_slices_rebuild_the_moved_document_exactly() {
        let (doc, group, _) = sample();
        let layout = doc.move_layout(&[group]).expect("découpage");
        assert!(layout.exact, "notes : {:?}", layout.notes);
        assert_eq!(layout.collapsible, Some(group));
        let kinds: Vec<SliceKind> = layout.slices.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [SliceKind::Below, SliceKind::Moving, SliceKind::Layers, SliceKind::Adjust, SliceKind::Layers]);
        assert_eq!(layout.slices[2].mode, BlendMode::Multiply, "le calque en Produit garde son mode, à part");
        assert!(layout.slices[3].masked);
        let r = TileRenderer::new();
        for d in [(0, 0), (37, -23), (-140, 95)] {
            let mut moved = doc.clone();
            moved.translate_layers(&[group], d.0, d.1).expect("déplacement");
            let truth = TileRenderer::new();
            for coord in [TileCoord::new(0, 0), TileCoord::new(1, 0), TileCoord::new(0, 1), TileCoord::new(2, 1)] {
                let gap = worst(from_slices(&r, &doc, &layout, coord, d), truth.tile(&moved, 0, coord));
                assert!(gap <= 3, "décalage {d:?}, tuile {coord:?} : écart {gap}");
            }
        }
    }

    /// Groupe en Transfert au contenu varié : un calque normal, un en Produit, un réglage masqué, puis un calque
    /// normal sur lequel un calque en Superposition est écrêté. Optimisé par paquets, il est rendu exactement, où qu'il
    /// soit déplacé.
    #[test]
    fn a_pass_through_group_moves_exactly_by_packs() {
        let (mut doc, group, _) = sample();
        let children = doc.layer(group).expect("groupe").children().to_vec();
        doc.edit_props(children[1], |p| p.blend = BlendMode::Multiply).expect("mode");
        let index = doc.children_of(Some(group)).expect("groupe").len();
        let invert = doc.add_adjustment_layer(Adjustment::Invert, Some(group), index, false).expect("réglage");
        doc.add_mask(invert, false).expect("masque");
        for y in 0..TILE_SIZE {
            for x in 0..TILE_SIZE {
                doc.mask_mut(invert).expect("masque").get_mut(TileCoord::new(0, 0)).set_pixel(x, y, [255 - (y as u8) / 2]);
            }
        }
        let base = add_patch(&mut doc, Some(group), (60, 150), (260, 160), [220, 220, 40]);
        let over = add_patch(&mut doc, Some(group), (40, 120), (300, 200), [40, 200, 220]);
        doc.edit_props(over, |p| {
            p.clipped = true;
            p.blend = BlendMode::Overlay;
        })
        .expect("écrêté");
        let layout = doc.move_layout(&[group]).expect("découpage");
        assert!(layout.exact, "notes : {:?}", layout.notes);
        assert!(layout.collapsible.is_none(), "un groupe en Transfert aux modes variés n'est jamais aplati");
        let moving: Vec<(SliceKind, BlendMode)> = layout.slices.iter().filter(|s| s.moving).map(|s| (s.kind, s.mode)).collect();
        assert_eq!(
            moving,
            [(SliceKind::Moving, BlendMode::Normal), (SliceKind::Moving, BlendMode::Multiply), (SliceKind::Moving, BlendMode::Normal), (SliceKind::Adjust, BlendMode::Normal), (SliceKind::Moving, BlendMode::Normal)],
            "paquets : normal, Produit, normal, réglage, calque normal avec son écrêtage"
        );
        assert!(layout.slices.iter().filter(|s| s.moving && s.kind == SliceKind::Moving).all(|s| s.layer.is_none()), "paquets : leurs calques, pas le groupe");
        let _ = base;
        let r = TileRenderer::new();
        for d in [(0, 0), (37, -23), (-140, 95)] {
            let mut moved = doc.clone();
            moved.translate_layers(&[group], d.0, d.1).expect("déplacement");
            let truth = TileRenderer::new();
            for coord in [TileCoord::new(0, 0), TileCoord::new(1, 0), TileCoord::new(0, 1), TileCoord::new(1, 1), TileCoord::new(2, 1)] {
                let gap = worst(from_slices(&r, &doc, &layout, coord, d), truth.tile(&moved, 0, coord));
                assert!(gap <= 3, "décalage {d:?}, tuile {coord:?} : écart {gap}");
            }
        }
    }

    /// Calques consécutifs en Produit (ou en Superposition) dans un groupe en Transfert : un seul paquet, même image.
    #[test]
    fn consecutive_multiply_layers_share_one_exact_pack() {
        let (mut doc, group, _) = sample();
        let a = add_patch(&mut doc, Some(group), (60, 150), (260, 160), [220, 120, 40]);
        let b = add_patch(&mut doc, Some(group), (140, 60), (300, 200), [40, 200, 220]);
        let c = add_patch(&mut doc, Some(group), (20, 220), (200, 140), [200, 40, 160]);
        let d = add_patch(&mut doc, Some(group), (90, 90), (240, 240), [90, 200, 60]);
        for id in [a, b] {
            doc.edit_props(id, |p| p.blend = BlendMode::Multiply).expect("mode");
        }
        for id in [c, d] {
            doc.edit_props(id, |p| p.blend = BlendMode::Screen).expect("mode");
        }
        let layout = doc.move_layout(&[group]).expect("découpage");
        assert!(layout.exact, "notes : {:?}", layout.notes);
        let moving: Vec<BlendMode> = layout.slices.iter().filter(|s| s.moving).map(|s| s.mode).collect();
        assert_eq!(moving, [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen], "deux Produit en un paquet, deux Superposition en un autre");
        let r = TileRenderer::new();
        for shift in [(0, 0), (37, -23), (-140, 95)] {
            let mut moved = doc.clone();
            moved.translate_layers(&[group], shift.0, shift.1).expect("déplacement");
            let truth = TileRenderer::new();
            for coord in [TileCoord::new(0, 0), TileCoord::new(1, 0), TileCoord::new(0, 1), TileCoord::new(1, 1), TileCoord::new(2, 1)] {
                let gap = worst(from_slices(&r, &doc, &layout, coord, shift), truth.tile(&moved, 0, coord));
                assert!(gap <= 3, "décalage {shift:?}, tuile {coord:?} : écart {gap}");
            }
        }
    }

    /// Un calque opaque fixe au-dessus : le « dessous » n'est pas composé là où il le couvre, et l'image reste exacte.
    #[test]
    fn an_opaque_layer_above_skips_the_hidden_below() {
        let (mut doc, group, _) = sample();
        let (w, h) = (2 * TILE_SIZE, TILE_SIZE);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, 90, 200, 255]).collect();
        let index = doc.children_of(None).expect("racine").len();
        let cover = doc.add_pixel_layer("opaque", TileStore::from_premultiplied(w, h, &rgba).expect("pixels"), None, index).expect("calque");
        let layout = doc.move_layout(&[group]).expect("découpage");
        assert!(layout.occluders.contains(&cover), "le calque opaque du dessus cache ce qui est dessous : {:?}", layout.occluders);
        let r = TileRenderer::new();
        assert!(r.move_slice_tile(&doc, &layout, 0, 0, TileCoord::new(0, 0), true).is_none(), "dessous caché : pas composé");
        assert!(r.move_slice_tile(&doc, &layout, 0, 0, TileCoord::new(0, 1), true).is_some(), "dessous visible : composé");
        for shift in [(0, 0), (37, -23), (-140, 95)] {
            let mut moved = doc.clone();
            moved.translate_layers(&[group], shift.0, shift.1).expect("déplacement");
            let truth = TileRenderer::new();
            for coord in [TileCoord::new(0, 0), TileCoord::new(1, 0), TileCoord::new(0, 1), TileCoord::new(1, 1), TileCoord::new(2, 1)] {
                let gap = worst(from_slices(&r, &doc, &layout, coord, shift), truth.tile(&moved, 0, coord));
                assert!(gap <= 3, "décalage {shift:?}, tuile {coord:?} : écart {gap}");
            }
        }
    }

    /// L'empreinte d'une couche fixe ne change que si son contenu change : un geste suivant la retrouve en cache.
    #[test]
    fn fixed_slices_keep_their_fingerprint_until_their_content_changes() {
        let (mut doc, group, _) = sample();
        let first = doc.move_layout(&[group]).expect("découpage");
        let prints = |l: &MoveLayout| l.slices.iter().map(|s| s.fingerprint).collect::<Vec<_>>();
        assert!(first.slices.iter().all(|s| s.moving == (s.fingerprint == 0)), "empreinte pour les couches fixes seulement");
        assert_eq!(prints(&first), prints(&doc.move_layout(&[group]).expect("découpage")), "même document, mêmes empreintes");
        doc.translate_layers(&[group], 40, 25).expect("déplacement");
        assert_eq!(prints(&first), prints(&doc.move_layout(&[group]).expect("découpage")), "le groupe a bougé : dessous et dessus inchangés");
        let below = first.slices[0].leaves[0];
        doc.edit_props(below, |p| p.opacity = 0.5).expect("opacité");
        let after = doc.move_layout(&[group]).expect("découpage");
        assert_ne!(first.slices[0].fingerprint, after.slices[0].fingerprint, "dessous retouché : nouvelle empreinte");
        // Couche en cache : la même tuile, sans recomposer.
        let r = TileRenderer::new();
        let a = r.move_slice_tile(&doc, &after, 0, 0, TileCoord::new(0, 0), true);
        let b = r.move_slice_tile(&doc, &after, 0, 0, TileCoord::new(0, 0), true);
        assert!(match (a, b) { (Some(a), Some(b)) => Arc::ptr_eq(&a, &b), (None, None) => true, _ => false }, "reprise du cache");
    }

    /// Mode portable : le contenu d'un groupe en Transfert tient dans le nombre de paquets demandé, et le découpage
    /// le rappelle (les images des couches sont demandées avec la même limite).
    #[test]
    fn the_pack_limit_is_respected_and_reported() {
        let (mut doc, group, _) = sample();
        for (i, mode) in [BlendMode::Overlay, BlendMode::SoftLight, BlendMode::Difference, BlendMode::ColorDodge, BlendMode::Overlay, BlendMode::Darken].into_iter().enumerate() {
            let id = add_patch(&mut doc, Some(group), (20 + 30 * i as i32, 40 + 20 * i as i32), (200, 160), [200, 40 * i as u8, 90]);
            doc.edit_props(id, |p| p.blend = mode).expect("mode");
        }
        let full = doc.move_layout(&[group]).expect("découpage");
        assert_eq!(full.max_packs, MAX_PACKS);
        assert!(full.slices.iter().filter(|s| s.moving).count() > 4, "sans limite : un paquet par mode");
        let light = doc.move_layout_packs(&[group], 4).expect("découpage");
        assert_eq!(light.max_packs, 4);
        assert_eq!(light.slices.iter().filter(|s| s.moving).count(), 4, "quatre paquets au plus");
        assert!(!light.exact, "le reste est fondu normalement : approximation notée");
    }

    #[test]
    fn the_flattened_group_image_is_the_group_render_wherever_it_is() {
        let (mut doc, group, _) = sample();
        doc.edit_props(group, |p| p.opacity = 0.6).expect("opacité");
        let layout = doc.move_layout(&[group]).expect("découpage");
        let r = TileRenderer::new();
        // Contenu aplati du groupe = le document réduit au seul groupe (rendu du moteur), opacité comprise.
        let mut alone = doc.clone();
        let others: Vec<LayerId> = alone.children_of(None).expect("racine").iter().copied().filter(|id| *id != group).collect();
        alone.remove_layers(&others).expect("retrait");
        for coord in [TileCoord::new(0, 0), TileCoord::new(1, 0), TileCoord::new(1, 1)] {
            let gap = worst(r.move_slice_tile(&doc, &layout, 1, 0, coord, true), TileRenderer::new().tile(&alone, 0, coord));
            assert!(gap <= 2, "tuile {coord:?} : écart {gap}");
        }
        // Sans opacité ni masque propres (aplat pour la carte graphique, qui les applique elle-même) : pleine opacité.
        let mut full = alone.clone();
        full.edit_props(group, |p| p.opacity = 1.0).expect("opacité");
        let gap = worst(r.move_slice_tile(&doc, &layout, 1, 0, TileCoord::new(1, 0), false), TileRenderer::new().tile(&full, 0, TileCoord::new(1, 0)));
        assert!(gap <= 2, "aplat sans opacité : écart {gap}");
        // Indépendant de la position : le groupe déplacé d'un bloc donne la même image, décalée d'autant.
        let before = r.move_slice_region(&doc, &layout, 1, 1, (20, 10, 200, 150), true);
        let mut moved = doc.clone();
        moved.translate_layers(&[group], 64, -32).expect("déplacement");
        let after_layout = moved.move_layout(&[group]).expect("découpage");
        let after = r.move_slice_region(&moved, &after_layout, 1, 1, (20 + 32, 10 - 16, 200, 150), true);
        let off: Vec<usize> = before.iter().zip(&after).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect();
        assert!(off.is_empty(), "au niveau 1, un décalage de 64 × -32 px = 32 × -16 pixels du niveau : {} écarts, premier {:?} ({:?})", off.len(), off.first().map(|i| ((i / 4) % 200, i / 800)), off.first().map(|i| (before[*i], after[*i])));
    }

    #[test]
    fn layouts_refuse_what_layers_cannot_show_and_note_approximations() {
        let (mut doc, group, multiply) = sample();
        let background = doc.walk_top_down().last().expect("fond").1.id;
        assert!(doc.move_layout(&[background, multiply]).is_none(), "calques non voisins");
        let index = doc.children_of(None).expect("racine").iter().position(|id| *id == multiply).expect("rang");
        let clipped = add_patch(&mut doc, None, (0, 0), (50, 50), [1, 2, 3]);
        doc.move_layers(&[clipped], None, index + 1).expect("rangé");
        doc.edit_props(clipped, |p| p.clipped = true).expect("écrêté");
        assert!(doc.move_layout(&[multiply]).is_none(), "un calque écrêté reste sur la base qui bouge");
        assert!(doc.move_layout(&[clipped]).is_none(), "un calque écrêté bouge sans sa base");
        assert!(doc.move_layout(&[multiply, clipped]).is_some(), "base et calque écrêté ensemble");
        // Contenu du groupe en Produit : optimisé par paquets (exact), jamais aplati pour la carte graphique.
        let child = doc.layer(group).expect("groupe").children()[1];
        doc.edit_props(child, |p| p.blend = BlendMode::Multiply).expect("mode");
        let layout = doc.move_layout(&[group]).expect("découpage");
        assert!(layout.exact && layout.collapsible.is_none(), "{layout:?}");
        assert!(layout.slices.iter().filter(|s| s.moving).count() >= 2, "paquets : {:?}", layout.slices);
        // Isolé (mode Normal au lieu de Transfert) : de nouveau exact, quel que soit son contenu.
        doc.edit_props(group, |p| p.blend = BlendMode::Normal).expect("mode");
        let layout = doc.move_layout(&[group]).expect("découpage");
        assert!(layout.collapsible == Some(group) && layout.notes.iter().all(|n| !n.contains("« G »")), "{:?}", layout.notes);
        doc.edit_props(group, |p| p.locks.position = true).expect("verrou");
        assert!(doc.move_layout(&[group]).is_none(), "verrouillé");
    }
}
