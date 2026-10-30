//! Effets de calque (« fx ») : ombre portée, ombre interne, lueurs, contour, incrustations (couleur, dégradé,
//! motif), satin, biseautage et estampage.
//!
//! Non destructifs : le calque garde ses pixels et la liste de ses effets. Le compositeur ne voit
//! que le résultat, un « calque stylé » — effets du dessous, pixels, effets du dessus — calculé une
//! fois puis gardé en cache tant que ni les pixels ni les effets ne changent.
//!
//! Simplification assumée par rapport à Photoshop : les effets sont posés en mode Normal à
//! l'intérieur du calque stylé, qui est ensuite fusionné d'un bloc avec le mode du calque. Une ombre
//! noire ou une lueur claire donnent le même résultat ; seuls des cas rares (ombre colorée en
//! Produit sur un fond vif) diffèrent.

use crate::blend::BlendMode;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::filters::gaussian;
use crate::tiles::TileStore;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokePosition {
    #[default]
    Outside,
    Inside,
    Center,
}

/// Style du biseautage (comme Photoshop) : où se trouve le relief par rapport au bord du calque.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BevelStyle {
    /// Le relief est taillé à l'extérieur du calque (le calque semble posé sur un socle).
    OuterBevel,
    /// Le relief est taillé dans le calque, depuis son bord vers l'intérieur.
    #[default]
    InnerBevel,
    /// Le relief est à cheval sur le bord : le calque semble sortir du fond.
    Emboss,
    /// Estampage paysager : le bord s'enfonce dans le fond, comme une couture de coussin.
    PillowEmboss,
}

/// Technique du biseautage : profil arrondi (Lisse) ou taillé net (Ciselé dur).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BevelTechnique {
    #[default]
    Smooth,
    ChiselHard,
}

/// Direction du relief : bombé vers le haut (vers la lumière) ou creusé vers le bas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BevelDirection {
    #[default]
    Up,
    Down,
}

/// Identifiant d'un motif de la bibliothèque (`crate::patterns`), gardé dans un tableau fixe pour que les effets
/// restent copiables (`Copy`) ; enregistré dans les fichiers comme une simple chaîne.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PatternId {
    bytes: [u8; 24],
    len: u8,
}

impl PatternId {
    /// Identifiant `id` (tronqué à 24 octets, à une frontière de caractère : aucun motif livré n'en approche).
    pub fn new(id: &str) -> Self {
        let mut end = id.len().min(24);
        while !id.is_char_boundary(end) {
            end -= 1;
        }
        let mut bytes = [0u8; 24];
        bytes[..end].copy_from_slice(&id.as_bytes()[..end]);
        Self { bytes, len: end as u8 }
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

impl Default for PatternId {
    fn default() -> Self {
        Self::new("pois")
    }
}

impl std::fmt::Debug for PatternId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl Serialize for PatternId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PatternId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Ok(Self::new(&id))
    }
}

const fn hundred() -> f32 {
    100.0
}

/// Un effet. `opacity` de 0 à 1 ; longueurs en pixels ; `angle` en degrés : direction d'où vient la
/// lumière (120° = en haut à gauche, l'ombre part donc en bas à droite).
///
/// Champs en camelCase dans les fichiers et l'interface (`highlightOpacity`…) ; les anciens champs, d'un seul mot,
/// ne changent pas : les fichiers d'avant s'ouvrent tels quels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum LayerEffect {
    // `blend` : mode de fusion de l'effet (Normal pour les fichiers d'avant son apparition). Ombre portée et lueur
    // externe se fondent dans ce qui est SOUS le calque ; les autres, dans le calque lui-même (comme Photoshop).
    DropShadow { enabled: bool, color: [u8; 3], opacity: f32, angle: f32, distance: f32, size: f32, #[serde(default)] blend: BlendMode },
    InnerShadow { enabled: bool, color: [u8; 3], opacity: f32, angle: f32, distance: f32, size: f32, #[serde(default)] blend: BlendMode },
    OuterGlow { enabled: bool, color: [u8; 3], opacity: f32, size: f32, #[serde(default)] blend: BlendMode },
    InnerGlow { enabled: bool, color: [u8; 3], opacity: f32, size: f32, #[serde(default)] blend: BlendMode },
    Stroke { enabled: bool, color: [u8; 3], opacity: f32, width: f32, position: StrokePosition, #[serde(default)] blend: BlendMode },
    ColorOverlay { enabled: bool, color: [u8; 3], opacity: f32, #[serde(default)] blend: BlendMode },
    /// Incrustation de dégradé : de `from` à `to` à travers le contenu du calque, dans la direction
    /// `angle` (0° = vers la droite, 90° = vers le haut), ou du centre vers les bords si `radial`.
    GradientOverlay { enabled: bool, from: [u8; 3], to: [u8; 3], opacity: f32, angle: f32, radial: bool, #[serde(default)] blend: BlendMode },
    /// Incrustation de motif : un motif bicolore de la bibliothèque (`crate::patterns`), `color` pour le dessin et
    /// `background` pour le fond, à l'échelle `scale` (en %, 100 = taille d'origine), lié au calque (il le suit
    /// quand on le déplace), limité à sa forme.
    PatternOverlay {
        enabled: bool,
        pattern: PatternId,
        #[serde(default = "hundred")]
        scale: f32,
        color: [u8; 3],
        background: [u8; 3],
        opacity: f32,
        #[serde(default)]
        blend: BlendMode,
    },
    /// Satin : ombrage intérieur ondulé, différence de deux copies floutées de la forme décalées de part et d'autre
    /// (`distance` dans la direction `angle`), limité à la forme ; `invert` en prend le complément.
    Satin { enabled: bool, color: [u8; 3], opacity: f32, angle: f32, distance: f32, size: f32, #[serde(default)] invert: bool, #[serde(default)] blend: BlendMode },
    /// Biseautage et estampage : relief tiré de la forme du calque (carte de hauteur sur `size` pixels autour du
    /// bord), éclairé depuis `angle` (degrés) et `altitude` (degrés au-dessus de l'horizon) ; les pentes tournées
    /// vers la lumière reçoivent les hautes lumières (`highlight`, en Écran par défaut), les autres les ombres
    /// (`shadow`, en Produit). `depth` : 1 = 100 % (pente de 45° pour un profil droit). `soften` adoucit le relief.
    BevelEmboss {
        enabled: bool,
        #[serde(default)]
        style: BevelStyle,
        #[serde(default)]
        technique: BevelTechnique,
        depth: f32,
        #[serde(default)]
        direction: BevelDirection,
        size: f32,
        #[serde(default)]
        soften: f32,
        angle: f32,
        altitude: f32,
        highlight: [u8; 3],
        highlight_opacity: f32,
        highlight_blend: BlendMode,
        shadow: [u8; 3],
        shadow_opacity: f32,
        shadow_blend: BlendMode,
    },
}

impl LayerEffect {
    pub fn blend(&self) -> BlendMode {
        match *self {
            Self::DropShadow { blend, .. }
            | Self::InnerShadow { blend, .. }
            | Self::OuterGlow { blend, .. }
            | Self::InnerGlow { blend, .. }
            | Self::Stroke { blend, .. }
            | Self::ColorOverlay { blend, .. }
            | Self::GradientOverlay { blend, .. }
            | Self::PatternOverlay { blend, .. }
            | Self::Satin { blend, .. } => blend,
            // Deux modes (hautes lumières, ombres) : celui des hautes lumières représente l'effet.
            Self::BevelEmboss { highlight_blend, .. } => highlight_blend,
        }
    }

    /// Effet posé sous le calque, qui se fond dans ce qui est dessous avec un mode autre que Normal : il est
    /// rendu à part, et le compositeur le fond dans l'image (voir `Styled::under`).
    fn blends_into_backdrop(&self) -> bool {
        matches!(self, Self::DropShadow { .. } | Self::OuterGlow { .. }) && !matches!(self.blend(), BlendMode::Normal | BlendMode::PassThrough)
    }

    pub(crate) fn enabled(&self) -> bool {
        match *self {
            Self::DropShadow { enabled, .. }
            | Self::InnerShadow { enabled, .. }
            | Self::OuterGlow { enabled, .. }
            | Self::InnerGlow { enabled, .. }
            | Self::Stroke { enabled, .. }
            | Self::ColorOverlay { enabled, .. }
            | Self::GradientOverlay { enabled, .. }
            | Self::PatternOverlay { enabled, .. }
            | Self::Satin { enabled, .. }
            | Self::BevelEmboss { enabled, .. } => enabled,
        }
    }

    /// Marge de calcul nécessaire autour du contenu du calque.
    pub(crate) fn reach(&self) -> f32 {
        match *self {
            // Les effets internes ne débordent pas, mais leur calcul a besoin de « voir » le vide autour du
            // calque sur la même distance : sans cette marge, le bord du tampon passerait pour de la matière.
            Self::DropShadow { distance, size, .. } | Self::InnerShadow { distance, size, .. } => distance.abs() + size * 1.6 + 2.0,
            Self::OuterGlow { size, .. } | Self::InnerGlow { size, .. } => size * 1.6 + 2.0,
            Self::Stroke { width, position, .. } if position != StrokePosition::Inside => width + 2.0,
            // Le satin floute la forme décalée : comme une ombre interne, il doit voir le vide autour.
            Self::Satin { distance, size, .. } => distance.abs() + size * 1.6 + 2.0,
            // Biseau interne : le relief reste dans la forme. Les autres styles la débordent de toute leur taille, plus
            // le flou qui arrondit le profil et celui de l'adoucissement.
            Self::BevelEmboss { style: BevelStyle::InnerBevel, .. } => 2.0,
            Self::BevelEmboss { size, soften, .. } => size * 1.5 + soften * 1.6 + 2.0,
            _ => 0.0,
        }
    }
}

/// Vrai si au moins un effet est actif : sinon le calque se compose tel quel, sans aucun surcoût.
pub fn any_enabled(effects: &[LayerEffect]) -> bool {
    effects.iter().any(LayerEffect::enabled)
}

/// De combien de pixels, au plus, les effets actifs débordent du contenu du calque (0 sans effet actif).
/// Sert à borner la zone à recomposer quand un calque à effets se déplace sans changer.
pub fn reach_enabled(effects: &[LayerEffect]) -> i32 {
    effects.iter().filter(|e| e.enabled()).map(|e| e.reach().ceil() as i32).max().unwrap_or(0)
}

const MAX_AREA: i64 = 80_000_000;

/// Flou d'une carte à un canal (on réutilise le flou RGBA du module des filtres).
fn blur(map: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma < 0.3 {
        return map.to_vec();
    }
    let packed: Vec<f32> = map.iter().flat_map(|v| [*v, 0.0, 0.0, 0.0]).collect();
    gaussian(&packed, w, h, sigma).chunks_exact(4).map(|px| px[0]).collect()
}

/// Carte décalée de `(dx, dy)` pixels ; ce qui entre par les bords vaut `outside`.
fn shifted(map: &[f32], w: usize, h: usize, dx: i32, dy: i32, outside: f32) -> Vec<f32> {
    let mut out = vec![outside; map.len()];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let sy = y as i32 - dy;
        if sy < 0 || sy >= h as i32 {
            return;
        }
        for (x, slot) in row.iter_mut().enumerate() {
            let sx = x as i32 - dx;
            if sx >= 0 && sx < w as i32 {
                *slot = map[sy as usize * w + sx as usize];
            }
        }
    });
    out
}

/// Distance euclidienne (en pixels) de chaque pixel au plus proche pixel « plein » (`inside` vrai),
/// par la transformée exacte en deux passes de Felzenszwalb et Huttenlocher.
pub(crate) fn distance_to(inside: &[bool], w: usize, h: usize) -> Vec<f32> {
    const FAR: f32 = 1e12;
    // Passe d'une ligne : enveloppe inférieure de paraboles.
    fn pass(f: &[f32], out: &mut [f32]) {
        let n = f.len();
        let (mut v, mut z) = (vec![0usize; n], vec![0.0f32; n + 1]);
        let mut k = 0;
        (z[0], z[1]) = (f32::NEG_INFINITY, f32::INFINITY);
        for q in 1..n {
            loop {
                let p = v[k];
                let s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * (q as f32 - p as f32));
                if s <= z[k] && k > 0 {
                    k -= 1;
                } else {
                    k += 1;
                    v[k] = q;
                    z[k] = s;
                    z[k + 1] = f32::INFINITY;
                    break;
                }
            }
        }
        k = 0;
        for (q, slot) in out.iter_mut().enumerate() {
            while z[k + 1] < q as f32 {
                k += 1;
            }
            let p = v[k];
            *slot = (q as f32 - p as f32).powi(2) + f[p];
        }
    }
    // Colonnes d'abord…
    let mut columns = vec![0.0f32; w * h];
    let seeds: Vec<f32> = inside.iter().map(|full| if *full { 0.0 } else { FAR }).collect();
    let by_column: Vec<Vec<f32>> = (0..w)
        .into_par_iter()
        .map(|x| {
            let f: Vec<f32> = (0..h).map(|y| seeds[y * w + x]).collect();
            let mut out = vec![0.0; h];
            pass(&f, &mut out);
            out
        })
        .collect();
    for (x, column) in by_column.iter().enumerate() {
        for (y, value) in column.iter().enumerate() {
            columns[y * w + x] = *value;
        }
    }
    // …puis lignes.
    let mut squared = vec![0.0f32; w * h];
    squared.par_chunks_mut(w).zip(columns.par_chunks(w)).for_each(|(out, row)| pass(row, out));
    squared.into_iter().map(f32::sqrt).collect()
}

/// Pose `color` avec la couverture `coverage` (0..1) sur un tampon prémultiplié 0..255 (« par-dessus »).
fn paint_over(target: &mut [f32], coverage: &[f32], color: [u8; 3], opacity: f32) {
    target.par_chunks_mut(4).zip(coverage.par_iter()).for_each(|(px, c)| {
        let a = (c * opacity).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        for i in 0..3 {
            px[i] = f32::from(color[i]) * a + px[i] * (1.0 - a);
        }
        px[3] = 255.0 * a + px[3] * (1.0 - a);
    });
}

/// Pose `color` avec la couverture `coverage` sur un tampon prémultiplié 0..255, selon le mode de fusion `mode`
/// (contre ce que le tampon contient déjà : le calque et ses effets posés avant).
fn paint_blend(target: &mut [f32], coverage: &[f32], color: [u8; 3], opacity: f32, mode: BlendMode) {
    if matches!(mode, BlendMode::Normal | BlendMode::PassThrough) {
        return paint_over(target, coverage, color, opacity);
    }
    target.par_chunks_mut(4).zip(coverage.par_iter()).for_each(|(px, c)| {
        let a = (c * opacity).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let src = [f32::from(color[0]) / 255.0 * a, f32::from(color[1]) / 255.0 * a, f32::from(color[2]) / 255.0 * a, a];
        let dst = [px[0] / 255.0, px[1] / 255.0, px[2] / 255.0, px[3] / 255.0];
        let out = crate::blend::blend_premultiplied(mode, src, dst);
        for i in 0..4 {
            px[i] = out[i] * 255.0;
        }
    });
}

/// Liste des effets rendus à part, si cet effet doit l'être : mode autre que Normal, et rendu en parties.
fn apart<'a>(blend: BlendMode, over: &'a mut Option<&mut Vec<(BlendMode, TileStore)>>) -> Option<&'a mut Vec<(BlendMode, TileStore)>> {
    if matches!(blend, BlendMode::Normal | BlendMode::PassThrough) {
        return None;
    }
    over.as_deref_mut()
}

/// Pose un effet du dessus : dans le calque en mode Normal ; seul, dans son propre tampon, dans un autre mode.
fn place(
    out: &mut [f32],
    over: &mut Option<&mut Vec<(BlendMode, TileStore)>>,
    coverage: &[f32],
    color: [u8; 3],
    opacity: f32,
    blend: BlendMode,
    to_store: &dyn Fn(&[f32]) -> Result<TileStore, EngineError>,
) -> Result<(), EngineError> {
    match apart(blend, over) {
        Some(list) => {
            let mut alone = vec![0.0f32; out.len()];
            paint_over(&mut alone, coverage, color, opacity);
            list.push((blend, to_store(&alone)?));
        }
        None => paint_blend(out, coverage, color, opacity, blend),
    }
    Ok(())
}

/// Pose un effet du dessus dont la couleur varie d'un pixel à l'autre (`colour_at`, 0..255), à travers `coverage` :
/// dans le calque selon son mode, ou seul dans son propre tampon dans un mode autre que Normal (comme `place`).
fn place_varying(
    out: &mut [f32],
    over: &mut Option<&mut Vec<(BlendMode, TileStore)>>,
    coverage: &[f32],
    opacity: f32,
    blend: BlendMode,
    colour_at: &(dyn Fn(usize) -> [f32; 3] + Sync),
    to_store: &dyn Fn(&[f32]) -> Result<TileStore, EngineError>,
) -> Result<(), EngineError> {
    let separate = apart(blend, over).is_some();
    let mode = if separate || blend == BlendMode::PassThrough { BlendMode::Normal } else { blend };
    let mut alone = if separate { vec![0.0f32; out.len()] } else { Vec::new() };
    let target: &mut [f32] = if separate { &mut alone } else { out };
    target.par_chunks_mut(4).enumerate().for_each(|(i, px)| {
        let a = (coverage[i] * opacity).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let colour = colour_at(i);
        let src = [colour[0] / 255.0 * a, colour[1] / 255.0 * a, colour[2] / 255.0 * a, a];
        let dst = [px[0] / 255.0, px[1] / 255.0, px[2] / 255.0, px[3] / 255.0];
        let mixed = crate::blend::blend_premultiplied(mode, src, dst);
        for c in 0..4 {
            px[c] = mixed[c] * 255.0;
        }
    });
    if separate {
        let store = to_store(&alone)?;
        if let Some(list) = over.as_deref_mut() {
            list.push((blend, store));
        }
    }
    Ok(())
}

/// Couverture du satin : deux copies de la forme floutées (écart-type `size / 2`), décalées de `shift` de part et
/// d'autre ; leur différence, limitée à la forme (son complément si `invert`).
fn satin_coverage(alpha: &[f32], w: usize, h: usize, (dx, dy): (i32, i32), size: f32, invert: bool) -> Vec<f32> {
    let one = blur(&shifted(alpha, w, h, dx, dy, 0.0), w, h, size / 2.0);
    let other = blur(&shifted(alpha, w, h, -dx, -dy, 0.0), w, h, size / 2.0);
    one.par_iter()
        .zip(other.par_iter())
        .zip(alpha.par_iter())
        .map(|((a, b), inside)| {
            let d = (a - b).abs().min(1.0);
            (if invert { 1.0 - d } else { d }) * inside
        })
        .collect()
}

/// Ombrage du biseautage : couvertures (hautes lumières, ombres), de 0 à 1, pour chaque pixel du tampon.
///
/// 1. Distance signée au bord (positive dans la forme), tirée des distances exactes (`distance_to`), affinée sur
///    les pixels du bord par leur alpha (un bord antialiasé donne un relief lisse, sans escaliers).
/// 2. Profil selon le style : de 0 à 1 sur `size` pixels, dans la forme (biseau interne), hors de la forme (biseau
///    externe), à cheval sur le bord (estampage) ou en creux sur le bord (estampage paysager). Lisse : profil en
///    quart de cercle (raide au bord, plat au sommet) puis arrondi par un flou ; Ciselé dur : pente droite, arêtes nettes.
/// 3. Hauteur = profil × taille × profondeur (négative vers le Bas) ; `soften` la floute encore.
/// 4. Normale de la surface par différences centrées ; éclairage de Lambert depuis la direction de la lumière. Une
///    surface plane reçoit l'éclairage de référence (sinus de l'altitude) : ce qui l'excède devient haute lumière,
///    ce qui lui manque devient ombre, rapportés à l'écart maximal possible.
fn bevel_shading(effect: &LayerEffect, alpha: &[f32], solid: &[bool], inside: &[f32], outside: &[f32], w: usize, h: usize) -> (Vec<f32>, Vec<f32>) {
    let LayerEffect::BevelEmboss { style, technique, depth, direction, size, soften, angle, altitude, .. } = *effect else {
        return (Vec::new(), Vec::new());
    };
    let size = size.max(1.0);
    let smooth = technique == BevelTechnique::Smooth;
    let profile = |t: f32| -> f32 {
        let t = t.clamp(0.0, 1.0);
        if smooth { (t * std::f32::consts::FRAC_PI_2).sin() } else { t }
    };
    let sign = if direction == BevelDirection::Up { 1.0 } else { -1.0 };
    let height: Vec<f32> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let a = alpha[i];
            // Distance signée au bord, positive dedans (même convention de demi-pixel que le contour).
            let mut s = if solid[i] { inside[i] - 0.5 } else { 0.5 - outside[i] };
            if a > 0.0 && a < 1.0 && s.abs() < 1.0 {
                s = a - 0.5;
            }
            let t = match style {
                BevelStyle::InnerBevel => profile(s / size),
                BevelStyle::OuterBevel => profile((s + size) / size),
                BevelStyle::Emboss => profile((s + size / 2.0) / size),
                // Creux sur le bord, plat de part et d'autre : le plat est à hauteur 0 (le flou du profil, nul hors du
                // tampon, ne doit pas faire apparaître une marche sur le bord du tampon).
                BevelStyle::PillowEmboss => profile(s.abs() * 2.0 / size) - 1.0,
            };
            t * size * depth.max(0.0) * sign
        })
        .collect();
    // Lisse : le profil est arrondi (arêtes adoucies) ; Ciselé dur : juste de quoi éviter les escaliers.
    let round = if smooth { (size / 8.0).max(0.8) } else { 0.6 };
    let sigma = (round * round + (soften / 2.0).powi(2)).sqrt();
    let height = blur(&height, w, h, sigma);

    let (angle, altitude) = (angle.to_radians(), altitude.clamp(0.0, 90.0).to_radians());
    // Vers la lumière, l'axe vertical de l'écran pointant vers le bas.
    let light = [altitude.cos() * angle.cos(), -altitude.cos() * angle.sin(), altitude.sin()];
    let gain = 1.0 / (1.0 - light[2]).max(0.15);
    let at = |x: usize, y: usize| height[y * w + x];
    let shading: Vec<(f32, f32)> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let gx = (at((x + 1).min(w - 1), y) - at(x.saturating_sub(1), y)) / 2.0;
            let gy = (at(x, (y + 1).min(h - 1)) - at(x, y.saturating_sub(1))) / 2.0;
            let lambert = (-gx * light[0] - gy * light[1] + light[2]) / (gx * gx + gy * gy + 1.0).sqrt();
            let shade = (lambert - light[2]) * gain;
            // Où l'ombrage s'applique : dans la forme (biseau interne), hors d'elle (biseau externe), partout sinon.
            let mask = match style {
                BevelStyle::InnerBevel => alpha[i],
                BevelStyle::OuterBevel => 1.0 - alpha[i],
                BevelStyle::Emboss | BevelStyle::PillowEmboss => 1.0,
            };
            (shade.clamp(0.0, 1.0) * mask, (-shade).clamp(0.0, 1.0) * mask)
        })
        .collect();
    shading.into_iter().unzip()
}

/// Calque stylé : le calque et ses effets (`main`), plus à part les effets du dessous qui se fondent dans l'image
/// sous le calque avec un mode autre que Normal (`under`, de bas en haut), en repère local.
pub struct Styled {
    pub main: TileStore,
    pub under: Vec<(BlendMode, TileStore)>,
    /// Effets du dessus (contour, incrustations, lueur et ombre internes) dans un mode autre que Normal : rendus
    /// seuls, ils se fondent dans TOUT ce qui est dessous (le calque et l'image), là où ils sont dessinés.
    pub over: Vec<(BlendMode, TileStore)>,
}

/// Calque stylé en un seul tampon (effets du dessous inclus en mode Normal) : aperçus, tests.
pub fn styled(pixels: &TileStore, effects: &[LayerEffect]) -> Result<TileStore, EngineError> {
    let mut under = Vec::new();
    styled_into(pixels, effects, &mut under, None)
}

/// Les effets peuvent-ils se recalculer sur une partie seulement du calque ? Non pour l'incrustation en dégradé,
/// tendue sur l'emprise ENTIÈRE du contenu : un morceau du calque n'a pas la même emprise.
pub fn locally_recomputable(effects: &[LayerEffect]) -> bool {
    !effects.iter().any(|e| e.enabled() && matches!(e, LayerEffect::GradientOverlay { .. }))
}

/// Effets recalculés sur la fenêtre `(x0, y0, x1, y1)` (repère local, bornes hautes exclues) seulement : les tuiles
/// du calque qui touchent la fenêtre élargie de la portée des effets sont stylées comme un petit calque à part.
/// Dans la fenêtre, le résultat est celui du calque entier (chaque effet ne voit pas plus loin que sa portée) ;
/// en dehors, il n'a pas de sens et n'est pas à reprendre. Voir [`locally_recomputable`].
pub fn restyle_region(pixels: &TileStore, effects: &[LayerEffect], (x0, y0, x1, y1): (i32, i32, i32, i32)) -> Result<Styled, EngineError> {
    let margin = reach_enabled(effects) + 2;
    let size = crate::tiles::TILE_SIZE as i32;
    let (wx0, wy0, wx1, wy1) = (x0 - margin, y0 - margin, x1 + margin, y1 + margin);
    let mut part = TileStore::new();
    for (coord, tile) in pixels.iter() {
        let (ox, oy) = coord.origin();
        if ox < wx1 && ox + size > wx0 && oy < wy1 && oy + size > wy0 {
            part.restore(coord, Some(Arc::clone(tile)));
        }
    }
    styled_parts(&part, effects)
}

/// Calque stylé : pixels du calque entourés de ses effets, en repère local.
pub fn styled_parts(pixels: &TileStore, effects: &[LayerEffect]) -> Result<Styled, EngineError> {
    let (mut under, mut over) = (Vec::new(), Vec::new());
    let main = styled_into(pixels, effects, &mut under, Some(&mut over))?;
    Ok(Styled { main, under, over })
}

/// `over` : `None` = tous les effets du dessus fondus dans le calque (rendu en un seul tampon).
fn styled_into(pixels: &TileStore, effects: &[LayerEffect], under: &mut Vec<(BlendMode, TileStore)>, mut over: Option<&mut Vec<(BlendMode, TileStore)>>) -> Result<TileStore, EngineError> {
    let Some((x0, y0, x1, y1)) = pixels.pixel_bounds(|px| px[3] > 0) else { return Ok(TileStore::new()) };
    let margin = effects.iter().filter(|e| e.enabled()).map(LayerEffect::reach).fold(0.0f32, f32::max).ceil() as i32 + 1;
    let (left, top) = (x0 - margin, y0 - margin);
    let (w, h) = ((x1 - x0 + 2 * margin) as usize, (y1 - y0 + 2 * margin) as usize);
    if (w as i64) * (h as i64) > MAX_AREA {
        return Err(EngineError::InvalidOperation("calque trop grand pour des effets de calque".into()));
    }
    let source: Vec<f32> = pixels.translated(-left, -top).to_premultiplied(w as u32, h as u32).iter().map(|v| f32::from(*v)).collect();
    let alpha: Vec<f32> = source.chunks_exact(4).map(|px| px[3] / 255.0).collect();
    // Direction de l'ombre : à l'opposé de la lumière (l'axe vertical de l'écran pointe vers le bas).
    let offset = |angle: f32, distance: f32| ((-angle.to_radians().cos() * distance).round() as i32, (angle.to_radians().sin() * distance).round() as i32);
    // Distances au bord, calculées seulement si un contour le demande.
    let needs_distance = effects.iter().any(|e| matches!(e, LayerEffect::Stroke { enabled: true, .. } | LayerEffect::BevelEmboss { enabled: true, .. }));
    let solid: Vec<bool> = alpha.iter().map(|a| *a >= 0.5).collect();
    let (outside, inside) = if needs_distance { (distance_to(&solid, w, h), distance_to(&solid.iter().map(|s| !s).collect::<Vec<_>>(), w, h)) } else { (Vec::new(), Vec::new()) };

    // Dessous du calque : ombre portée, lueur externe. En mode Normal, elles vont dans le tampon du calque ; dans un
    // autre mode, chacune est rendue dans son propre tampon, que le compositeur fondra dans l'image dessous.
    let to_store = |buffer: &[f32]| -> Result<TileStore, EngineError> {
        let bytes: Vec<u8> = buffer
            .chunks_exact(4)
            .flat_map(|px| {
                let a = px[3].clamp(0.0, 255.0);
                [px[0].clamp(0.0, a), px[1].clamp(0.0, a), px[2].clamp(0.0, a), a].map(|v| (v + 0.5) as u8)
            })
            .collect();
        let store = TileStore::from_premultiplied(w as u32, h as u32, &bytes).ok_or_else(|| EngineError::Corrupt("tampon d'effets de taille inattendue".into()))?;
        Ok(store.translated(left, top))
    };
    let mut out = vec![0.0f32; source.len()];
    for effect in effects.iter().filter(|e| e.enabled()) {
        let (coverage, color, opacity) = match *effect {
            LayerEffect::DropShadow { color, opacity, angle, distance, size, .. } => {
                let (dx, dy) = offset(angle, distance);
                (blur(&shifted(&alpha, w, h, dx, dy, 0.0), w, h, size / 2.0), color, opacity)
            }
            // Halo renforcé près du bord, comme une source de lumière derrière le calque.
            LayerEffect::OuterGlow { color, opacity, size, .. } => (blur(&alpha, w, h, size / 2.0).iter().map(|v| (v * 1.8).min(1.0)).collect(), color, opacity),
            _ => continue,
        };
        if effect.blends_into_backdrop() {
            let mut alone = vec![0.0f32; source.len()];
            paint_over(&mut alone, &coverage, color, opacity);
            under.push((effect.blend(), to_store(&alone)?));
        } else {
            paint_over(&mut out, &coverage, color, opacity);
        }
    }
    // Le calque lui-même.
    out.par_chunks_mut(4).zip(source.par_chunks(4)).for_each(|(px, layer)| {
        let keep = 1.0 - layer[3] / 255.0;
        for i in 0..4 {
            px[i] = layer[i] + px[i] * keep;
        }
    });
    // Dessus du calque, limités à sa forme, dans l'ordre de Photoshop quel que soit celui de la liste (de bas en
    // haut) : incrustation de motif, incrustations de couleur et de dégradé (entre elles, dans l'ordre de la liste),
    // satin, lueur interne, ombre interne ; puis les contours, et le biseautage tout en haut.
    let empty: Vec<f32> = alpha.iter().map(|a| 1.0 - a).collect();
    for pass in 0..5 {
        for effect in effects.iter().filter(|e| e.enabled()) {
            match (pass, *effect) {
                (0, LayerEffect::PatternOverlay { pattern, scale, color, background, opacity, blend, .. }) => {
                    // Motif inconnu (fichier venu d'une version plus récente) : l'effet est ignoré.
                    let Some(pattern) = crate::patterns::find(pattern.as_str()) else { continue };
                    let scale = (scale / 100.0).clamp(0.01, 100.0);
                    // Lié au calque : le motif est ancré sur l'origine du repère du calque, il le suit quand on le déplace.
                    let colour_at = |i: usize| -> [f32; 3] {
                        let (x, y) = ((i % w) as i32 + left, (i / w) as i32 + top);
                        let k = pattern.coverage(x as f32, y as f32, scale);
                        std::array::from_fn(|c| f32::from(background[c]) + (f32::from(color[c]) - f32::from(background[c])) * k)
                    };
                    place_varying(&mut out, &mut over, &alpha, opacity, blend, &colour_at, &to_store)?;
                }
                (1, LayerEffect::ColorOverlay { color, opacity, blend, .. }) => {
                    if let Some(list) = apart(blend, &mut over) {
                        let mut alone = vec![0.0f32; source.len()];
                        paint_over(&mut alone, &alpha, color, opacity);
                        list.push((blend, to_store(&alone)?));
                    } else {
                        paint_blend(&mut out, &alpha, color, opacity, blend);
                    }
                }
                (1, LayerEffect::GradientOverlay { from, to, opacity, angle, radial, blend, .. }) => {
                    let separate = apart(blend, &mut over).is_some();
                    let mut alone = if separate { vec![0.0f32; source.len()] } else { Vec::new() };
                    let target: &mut Vec<f32> = if separate { &mut alone } else { &mut out };
                    let blend = if separate { BlendMode::Normal } else { blend };
                    // Le dégradé couvre exactement le contenu du calque (sans la marge de calcul).
                    let (cw, ch) = ((x1 - x0) as f32, (y1 - y0) as f32);
                    let (cx, cy) = (margin as f32 + cw / 2.0, margin as f32 + ch / 2.0);
                    let (dx, dy) = (angle.to_radians().cos(), -angle.to_radians().sin());
                    let half = (dx.abs() * cw + dy.abs() * ch) / 2.0;
                    target.par_chunks_mut(4).enumerate().for_each(|(i, px)| {
                        let a = (alpha[i] * opacity).clamp(0.0, 1.0);
                        if a <= 0.0 {
                            return;
                        }
                        let (x, y) = ((i % w) as f32 + 0.5 - cx, (i / w) as f32 + 0.5 - cy);
                        let t = if radial { (x / (cw / 2.0)).hypot(y / (ch / 2.0)) } else { (x * dx + y * dy) / half.max(1.0) * 0.5 + 0.5 }.clamp(0.0, 1.0);
                        let colour: [f32; 3] = std::array::from_fn(|c| f32::from(from[c]) + (f32::from(to[c]) - f32::from(from[c])) * t);
                        let src = [colour[0] / 255.0 * a, colour[1] / 255.0 * a, colour[2] / 255.0 * a, a];
                        let dst = [px[0] / 255.0, px[1] / 255.0, px[2] / 255.0, px[3] / 255.0];
                        let mode = if blend == BlendMode::PassThrough { BlendMode::Normal } else { blend };
                        let mixed = crate::blend::blend_premultiplied(mode, src, dst);
                        for i in 0..4 {
                            px[i] = mixed[i] * 255.0;
                        }
                    });
                    if separate {
                        let store = to_store(&alone)?;
                        if let (LayerEffect::GradientOverlay { blend, .. }, Some(list)) = (*effect, over.as_deref_mut()) {
                            list.push((blend, store));
                        }
                    }
                }
                (2, LayerEffect::Satin { color, opacity, angle, distance, size, invert, blend, .. }) => {
                    let satin = satin_coverage(&alpha, w, h, offset(angle, distance), size, invert);
                    place(&mut out, &mut over, &satin, color, opacity, blend, &to_store)?;
                }
                (3, LayerEffect::InnerGlow { color, opacity, size, blend, .. }) => {
                    let glow: Vec<f32> = blur(&empty, w, h, size / 2.0).iter().zip(&alpha).map(|(g, a)| (g * 1.8).min(1.0) * a).collect();
                    place(&mut out, &mut over, &glow, color, opacity, blend, &to_store)?;
                }
                (4, LayerEffect::InnerShadow { color, opacity, angle, distance, size, blend, .. }) => {
                    let (dx, dy) = offset(angle, distance);
                    let shadow: Vec<f32> = blur(&shifted(&empty, w, h, dx, dy, 1.0), w, h, size / 2.0).iter().zip(&alpha).map(|(s, a)| s * a).collect();
                    place(&mut out, &mut over, &shadow, color, opacity, blend, &to_store)?;
                }
                _ => {}
            }
        }
    }
    // Plusieurs contours : le premier de la liste est dessiné en dernier, donc au-dessus (comme dans
    // Photoshop). Pour un contour double, on liste donc le plus fin d'abord, le plus large ensuite.
    for effect in effects.iter().rev().filter(|e| e.enabled()) {
        if let LayerEffect::Stroke { color, opacity, width, position, blend, .. } = *effect {
            let (out_w, in_w) = match position {
                StrokePosition::Outside => (width, 0.0),
                StrokePosition::Inside => (0.0, width),
                StrokePosition::Center => (width / 2.0, width / 2.0),
            };
            // Distance signée au bord : négative dedans. Un pixel d'antialiasing de chaque côté.
            let band: Vec<f32> = (0..w * h)
                .into_par_iter()
                .map(|i| {
                    let d = if solid[i] { 0.5 - inside[i] } else { outside[i] - 0.5 };
                    ((out_w - d + 0.5).clamp(0.0, 1.0)).min((d + in_w + 0.5).clamp(0.0, 1.0))
                })
                .collect();
            place(&mut out, &mut over, &band, color, opacity, blend, &to_store)?;
        }
    }
    // Biseautage et estampage, au-dessus de tout : ombres puis hautes lumières, chacune dans son mode.
    for effect in effects.iter().filter(|e| e.enabled()) {
        if let LayerEffect::BevelEmboss { highlight, highlight_opacity, highlight_blend, shadow, shadow_opacity, shadow_blend, .. } = *effect {
            let (lit, dark) = bevel_shading(effect, &alpha, &solid, &inside, &outside, w, h);
            place(&mut out, &mut over, &dark, shadow, shadow_opacity, shadow_blend, &to_store)?;
            place(&mut out, &mut over, &lit, highlight, highlight_opacity, highlight_blend, &to_store)?;
        }
    }

    to_store(&out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::Sampler;

    /// Carré rouge de 100 px, de (100, 100) à (200, 200).
    fn square() -> TileStore {
        let mut store = TileStore::new();
        for y in 100..200 {
            for x in 100..200 {
                let (coord, px, py) = crate::tiles::TileCoord::of_pixel(x, y);
                store.get_mut(coord).set_pixel(px, py, [255, 0, 0, 255]);
            }
        }
        store
    }

    fn at(store: &TileStore, x: i32, y: i32) -> [u8; 4] {
        Sampler::new(store).get(x, y)
    }

    #[test]
    fn no_enabled_effect_means_no_work() {
        let off = [LayerEffect::OuterGlow { enabled: false, color: [255; 3], opacity: 1.0, size: 20.0, blend: crate::blend::BlendMode::Normal }];
        assert!(!any_enabled(&off) && !any_enabled(&[]));
        let styled = styled(&square(), &off).expect("rendu");
        assert_eq!((at(&styled, 150, 150), at(&styled, 90, 150)), ([255, 0, 0, 255], [0; 4]));
    }

    #[test]
    fn drop_shadow_falls_away_from_the_light_and_stays_under_the_layer() {
        // Lumière en haut à gauche (135°) : l'ombre part en bas à droite.
        let shadow = [LayerEffect::DropShadow { enabled: true, color: [0, 0, 0], opacity: 1.0, angle: 135.0, distance: 20.0, size: 0.0, blend: crate::blend::BlendMode::Normal }];
        let styled = styled(&square(), &shadow).expect("rendu");
        assert_eq!(at(&styled, 150, 150), [255, 0, 0, 255], "le calque recouvre son ombre");
        assert_eq!(at(&styled, 208, 208), [0, 0, 0, 255], "ombre visible en bas à droite");
        assert_eq!(at(&styled, 92, 92), [0; 4], "rien en haut à gauche");
        let soft = [LayerEffect::DropShadow { enabled: true, color: [0, 0, 0], opacity: 0.5, angle: 135.0, distance: 20.0, size: 16.0, blend: crate::blend::BlendMode::Normal }];
        let edge = at(&styled2(&soft), 214, 160)[3];
        assert!((40..=90).contains(&edge), "bord flouté, à demi-opacité : {edge}");
    }

    fn styled2(effects: &[LayerEffect]) -> TileStore {
        styled(&square(), effects).expect("rendu")
    }

    #[test]
    fn strokes_follow_the_edge_at_the_requested_position() {
        let stroke = |position| styled2(&[LayerEffect::Stroke { enabled: true, color: [0, 0, 255], opacity: 1.0, width: 10.0, position, blend: crate::blend::BlendMode::Normal }]);
        let outside = stroke(StrokePosition::Outside);
        assert_eq!((at(&outside, 95, 150), at(&outside, 105, 150), at(&outside, 88, 150)[3]), ([0, 0, 255, 255], [255, 0, 0, 255], 0));
        // Angles arrondis : à 10 px en diagonale du coin, on est hors du contour (il faudrait 14 px pour un angle droit).
        assert_eq!(at(&outside, 91, 91)[3], 0, "angle arrondi, pas carré");
        let inside = stroke(StrokePosition::Inside);
        assert_eq!((at(&inside, 105, 150), at(&inside, 115, 150), at(&inside, 95, 150)[3]), ([0, 0, 255, 255], [255, 0, 0, 255], 0));
        let center = stroke(StrokePosition::Center);
        assert_eq!((at(&center, 97, 150), at(&center, 103, 150), at(&center, 108, 150)), ([0, 0, 255, 255], [0, 0, 255, 255], [255, 0, 0, 255]));
    }

    #[test]
    fn inner_effects_never_leave_the_shape_and_glows_surround_it() {
        let inner = styled2(&[
            LayerEffect::InnerShadow { enabled: true, color: [0, 0, 0], opacity: 1.0, angle: 135.0, distance: 12.0, size: 0.0, blend: crate::blend::BlendMode::Normal },
            LayerEffect::InnerGlow { enabled: true, color: [255, 255, 0], opacity: 1.0, size: 12.0, blend: crate::blend::BlendMode::Normal },
        ]);
        assert_eq!(inner.pixel_bounds(|px| px[3] > 0), Some((100, 100, 200, 200)), "rien ne déborde");
        assert_eq!(at(&inner, 105, 105), [0, 0, 0, 255], "ombre interne contre les bords éclairés (haut et gauche)");
        assert_eq!(at(&inner, 150, 150), [255, 0, 0, 255], "le centre garde sa couleur");
        assert!(at(&inner, 197, 150)[1] > 80, "lueur interne jaune le long du bord opposé : {:?}", at(&inner, 197, 150));

        let glow = styled2(&[LayerEffect::OuterGlow { enabled: true, color: [0, 255, 255], opacity: 1.0, size: 24.0, blend: crate::blend::BlendMode::Normal }]);
        let (near, far) = (at(&glow, 96, 150), at(&glow, 80, 150));
        assert!(near[3] > far[3] && far[3] > 0 && near[0] == 0 && near[1] > 0, "halo cyan qui s'éteint en s'éloignant : {near:?} puis {far:?}");
        assert_eq!(at(&glow, 150, 150), [255, 0, 0, 255]);

        let overlay = styled2(&[LayerEffect::ColorOverlay { enabled: true, color: [0, 255, 0], opacity: 1.0, blend: crate::blend::BlendMode::Normal }]);
        assert_eq!((at(&overlay, 150, 150), at(&overlay, 90, 150)[3]), ([0, 255, 0, 255], 0));
    }

    #[test]
    fn effects_stack_several_strokes_and_a_gradient_overlay() {
        // Double contour : 4 px blanc listé d'abord (au-dessus), 10 px noir ensuite (dessous, il dépasse).
        let stroke = |color, width| LayerEffect::Stroke { enabled: true, color, opacity: 1.0, width, position: StrokePosition::Outside, blend: crate::blend::BlendMode::Normal };
        let double = styled2(&[stroke([255, 255, 255], 4.0), stroke([0, 0, 0], 10.0)]);
        assert_eq!((at(&double, 98, 150), at(&double, 93, 150), at(&double, 88, 150)[3]), ([255, 255, 255, 255], [0, 0, 0, 255], 0));

        let gradient = styled2(&[LayerEffect::GradientOverlay { enabled: true, from: [0, 0, 255], to: [255, 255, 0], opacity: 1.0, angle: 0.0, radial: false, blend: crate::blend::BlendMode::Normal }]);
        let (left, right) = (at(&gradient, 102, 150), at(&gradient, 197, 150));
        assert!(left[2] > 240 && left[0] < 15 && right[0] > 240 && right[2] < 15, "bleu à gauche, jaune à droite : {left:?} {right:?}");
        assert_eq!(gradient.pixel_bounds(|px| px[3] > 0), Some((100, 100, 200, 200)), "limité à la forme du calque");
        let up = styled2(&[LayerEffect::GradientOverlay { enabled: true, from: [0, 0, 0], to: [255, 255, 255], opacity: 1.0, angle: 90.0, radial: false, blend: crate::blend::BlendMode::Normal }]);
        assert!(at(&up, 150, 103)[0] > 240 && at(&up, 150, 197)[0] < 15, "90° : vers le haut");
    }

    /// Biseau par défaut de la fenêtre des effets (celui de Photoshop) : interne, lisse, 100 %, 5 px, lumière à 120°.
    fn bevel(style: BevelStyle, enabled: bool) -> LayerEffect {
        LayerEffect::BevelEmboss {
            enabled,
            style,
            technique: BevelTechnique::Smooth,
            depth: 1.0,
            direction: BevelDirection::Up,
            size: 10.0,
            soften: 0.0,
            angle: 120.0,
            altitude: 30.0,
            highlight: [255, 255, 255],
            highlight_opacity: 0.75,
            highlight_blend: BlendMode::Screen,
            shadow: [0, 0, 0],
            shadow_opacity: 0.75,
            shadow_blend: BlendMode::Multiply,
        }
    }

    fn satin(enabled: bool) -> LayerEffect {
        LayerEffect::Satin { enabled, color: [0, 0, 0], opacity: 1.0, angle: 19.0, distance: 11.0, size: 14.0, invert: false, blend: BlendMode::Normal }
    }

    fn pattern(enabled: bool) -> LayerEffect {
        LayerEffect::PatternOverlay { enabled, pattern: PatternId::new("damier"), scale: 100.0, color: [0, 0, 0], background: [255, 255, 255], opacity: 1.0, blend: BlendMode::Normal }
    }

    #[test]
    fn new_effects_disabled_change_nothing() {
        let plain = square();
        for effect in [bevel(BevelStyle::InnerBevel, false), bevel(BevelStyle::Emboss, false), satin(false), pattern(false)] {
            assert!(!any_enabled(&[effect]) && reach_enabled(&[effect]) == 0);
            let styled = styled2(&[effect]);
            assert_eq!(styled.pixel_bounds(|px| px[3] > 0), plain.pixel_bounds(|px| px[3] > 0));
            for (x, y) in [(100, 100), (150, 150), (199, 120), (90, 150)] {
                assert_eq!(at(&styled, x, y), at(&plain, x, y), "{effect:?} en ({x}, {y})");
            }
        }
    }

    #[test]
    fn bevel_lights_the_edge_facing_the_light_and_darkens_the_other() {
        // Lumière à 120° (en haut à gauche) : bords haut et gauche éclaircis, bas et droit assombris ; centre intact.
        let parts = styled_parts(&square(), &[bevel(BevelStyle::InnerBevel, true)]).expect("rendu");
        // Hautes lumières en Écran, ombres en Produit : rendues à part (ombres d'abord), fondues par le compositeur.
        assert_eq!(parts.over.iter().map(|(m, _)| *m).collect::<Vec<_>>(), [BlendMode::Multiply, BlendMode::Screen]);
        let (dark, lit) = (&parts.over[0].1, &parts.over[1].1);
        // La lumière vient surtout d'en haut (120°) : le bord du haut reçoit plus que celui de gauche.
        assert!(at(lit, 150, 102)[3] > at(lit, 102, 150)[3], "haut plus éclairé que gauche");
        for (x, y) in [(102, 150), (150, 102)] {
            assert!(at(lit, x, y)[3] > 40 && at(dark, x, y)[3] < 10, "bord éclairé en ({x}, {y}) : {:?} / {:?}", at(lit, x, y), at(dark, x, y));
        }
        for (x, y) in [(197, 150), (150, 197)] {
            assert!(at(dark, x, y)[3] > 40 && at(lit, x, y)[3] < 10, "bord à l'ombre en ({x}, {y}) : {:?} / {:?}", at(lit, x, y), at(dark, x, y));
        }
        assert_eq!((at(lit, 150, 150)[3], at(dark, 150, 150)[3]), (0, 0), "le plat du centre ne bouge pas");
        // Biseau interne : rien hors de la forme. En un seul tampon (Normal), le bord éclairé est plus clair que l'autre.
        assert!(lit.pixel_bounds(|px| px[3] > 0).is_some_and(|(x0, y0, x1, y1)| x0 >= 100 && y0 >= 100 && x1 <= 200 && y1 <= 200));
        let flat = styled2(&[bevel(BevelStyle::InnerBevel, true)]);
        let (bright, shade) = (at(&flat, 150, 102), at(&flat, 150, 197));
        assert!(bright[1] > 100 && shade[0] < 120, "haut-gauche clair, bas-droit sombre : {bright:?} {shade:?}");
        // Vers le Bas : l'inverse.
        let mut down = bevel(BevelStyle::InnerBevel, true);
        if let LayerEffect::BevelEmboss { direction, .. } = &mut down {
            *direction = BevelDirection::Down;
        }
        let sunk = styled2(&[down]);
        assert!(at(&sunk, 150, 102)[0] < 120 && at(&sunk, 150, 197)[1] > 100, "vers le bas, les bords s'inversent");
        // Biseau externe et estampage débordent de la forme ; leur portée le dit (invalidation).
        let outer = styled_parts(&square(), &[bevel(BevelStyle::OuterBevel, true)]).expect("rendu");
        assert!(at(&outer.over[1].1, 96, 150)[3] > 50 && at(&outer.over[0].1, 204, 150)[3] > 50, "socle éclairé à gauche, ombré à droite");
        assert!(reach_enabled(&[bevel(BevelStyle::Emboss, true)]) >= 10 && reach_enabled(&[bevel(BevelStyle::InnerBevel, true)]) <= 2);
        for style in [BevelStyle::Emboss, BevelStyle::PillowEmboss] {
            let parts = styled_parts(&square(), &[bevel(style, true)]).expect("rendu");
            assert!(parts.over.iter().any(|(_, s)| at(s, 96, 150)[3] > 20) && parts.over.iter().any(|(_, s)| at(s, 104, 150)[3] > 20), "{style:?} : des deux côtés du bord");
        }
    }

    #[test]
    fn satin_stays_inside_the_shape() {
        let styled = styled2(&[satin(true)]);
        assert_eq!(styled.pixel_bounds(|px| px[3] > 0), Some((100, 100, 200, 200)), "rien ne déborde");
        let darkened = (100..200).flat_map(|y| (100..200).map(move |x| (x, y))).filter(|(x, y)| at(&styled, *x, *y)[0] < 200).count();
        assert!(darkened > 200, "des bandes sombres dans la forme : {darkened} pixels");
        assert_eq!(at(&styled, 150, 150), [255, 0, 0, 255], "loin des bords, les deux copies se compensent");
        // Inversé : le centre s'assombrit.
        let LayerEffect::Satin { color, opacity, angle, distance, size, blend, .. } = satin(true) else { unreachable!() };
        let inverted = styled2(&[LayerEffect::Satin { enabled: true, color, opacity, angle, distance, size, invert: true, blend }]);
        assert_eq!(at(&inverted, 150, 150), [0, 0, 0, 255]);
        assert_eq!(inverted.pixel_bounds(|px| px[3] > 0), Some((100, 100, 200, 200)));
    }

    #[test]
    fn pattern_overlay_tiles_inside_the_shape() {
        // Damier de cases de 8 px, ancré sur l'origine du calque : noir sur les cases paires, blanc sur les autres.
        let styled = styled2(&[pattern(true)]);
        assert_eq!(styled.pixel_bounds(|px| px[3] > 0), Some((100, 100, 200, 200)), "limité à la forme");
        assert_eq!((at(&styled, 104, 104), at(&styled, 112, 104)), ([0, 0, 0, 255], [255, 255, 255, 255]));
        for (x, y) in [(103, 101), (110, 117), (125, 130)] {
            assert_eq!(at(&styled, x, y), at(&styled, x + 16, y), "se répète tous les 16 px ({x}, {y})");
            assert_eq!(at(&styled, x, y), at(&styled, x + 32, y + 48));
        }
        // Échelle 200 % : des cases de 16 px.
        let big = styled2(&[LayerEffect::PatternOverlay { enabled: true, pattern: PatternId::new("damier"), scale: 200.0, color: [0, 0, 0], background: [255, 255, 255], opacity: 1.0, blend: BlendMode::Normal }]);
        assert_eq!((at(&big, 104, 104), at(&big, 108, 104), at(&big, 120, 104), at(&big, 136, 104)), ([0, 0, 0, 255], [0, 0, 0, 255], [255; 4], [0, 0, 0, 255]));
        // Motif inconnu : ignoré.
        let unknown = styled2(&[LayerEffect::PatternOverlay { enabled: true, pattern: PatternId::new("inconnu"), scale: 100.0, color: [0; 3], background: [0; 3], opacity: 1.0, blend: BlendMode::Normal }]);
        assert_eq!(at(&unknown, 150, 150), [255, 0, 0, 255]);
    }

    #[test]
    fn new_effects_round_trip_and_old_files_still_open() {
        let list = vec![bevel(BevelStyle::PillowEmboss, true), satin(true), pattern(true)];
        let text = serde_json::to_string(&list).expect("json");
        assert!(text.contains("\"type\":\"bevelEmboss\"") && text.contains("\"highlightOpacity\"") && text.contains("\"pillow_emboss\"") && text.contains("\"pattern\":\"damier\""), "{text}");
        assert_eq!(serde_json::from_str::<Vec<LayerEffect>>(&text).expect("relu"), list);
        // Un fichier d'avant (champs d'un mot, sans mode) s'ouvre toujours.
        let old = r#"[{"type":"dropShadow","enabled":true,"color":[0,0,0],"opacity":0.5,"angle":120,"distance":5,"size":5}]"#;
        assert!(matches!(serde_json::from_str::<Vec<LayerEffect>>(old).expect("ancien")[0], LayerEffect::DropShadow { blend: BlendMode::Normal, .. }));
    }

    #[test]
    fn exact_distance_transform() {
        // Un seul pixel plein au centre d'une grille 9 x 9.
        let mut grid = vec![false; 81];
        grid[4 * 9 + 4] = true;
        let d = distance_to(&grid, 9, 9);
        assert_eq!(d[4 * 9 + 4], 0.0);
        assert!((d[4 * 9 + 7] - 3.0).abs() < 1e-4 && (d[0] - 32.0f32.sqrt()).abs() < 1e-4 && (d[7 * 9 + 8] - 5.0).abs() < 1e-4);
    }
}
