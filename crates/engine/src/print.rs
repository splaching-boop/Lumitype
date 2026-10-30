//! Impression : format fini, fonds perdus, zone de sécurité et limite d'encrage (Image › Format d'impression…).
//!
//! - **Fonds perdus** : le document les CONTIENT. Sa zone de travail est le format fini agrandi des fonds perdus de
//!   chaque côté (une carte 85 × 55 mm avec 3 mm de fonds perdus fait 91 × 61 mm). Le format fini (« zone de coupe »)
//!   est donc le rectangle intérieur, en retrait de [`Document::bleed_px`] ; l'export PDF/X-4 le déclare en
//!   `/TrimBox`, la zone entière en `/BleedBox`.
//! - **Zone de sécurité** : retrait depuis le format fini, à l'intérieur duquel garder textes et éléments importants
//!   (la coupe a une tolérance d'un à deux millimètres). Un simple repère, affiché et vérifié.
//! - **Limite d'encrage** : somme maximale C + M + J + N, en % (300 % sur papier couché offset, le plafond de FOGRA39).
//!   Au-delà, l'encre sèche mal, décharge sur la feuille suivante et les noirs bouchent. [`ink_report`] situe les zones
//!   qui dépassent (l'alerte d'encrage et la vérification), [`Document::correct_ink`] les ramène sous la limite et
//!   [`limit_ink`] est appliquée à la séparation de l'export, en sécurité.
//!
//! Correction de l'encrage : pour une couleur qui dépasse, on remplace d'abord du cyan + magenta + jaune par du noir
//! (« GCR », Gray Component Replacement : la partie grise du mélange passe au noir seul, la couleur reste la même),
//! puis, si cela ne suffit pas, on réduit proportionnellement les trois couleurs. La couleur RVB corrigée est le
//! retour à l'écran de ces encres ; comme les pixels restent en RVB, on vérifie que leur séparation par le profil
//! reste bien sous la limite (sinon on resserre la cible). Le tout est calculé une fois sur les 33³ nœuds de la
//! table du profil, puis appliqué par interpolation aux seuls pixels qui dépassent.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cmyk::{self, CmykSpace, N};
use crate::document::{Document, LayerId};
use crate::error::EngineError;
use crate::pixel::{premultiply, unpremultiply};
use crate::tiles::{Sampler, TileCoord};

pub const MM_PER_INCH: f32 = 25.4;

/// Format d'impression d'un document. Enregistré avec lui (absent des anciens fichiers).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintSetup {
    /// Fonds perdus, en mm (0 : aucun).
    pub bleed_mm: f32,
    /// Zone de sécurité, en mm depuis le format fini (0 : aucune).
    pub safe_mm: f32,
    /// Limite d'encrage en % (0 : pas de limite).
    pub ink_limit: u16,
}

impl Default for PrintSetup {
    fn default() -> Self {
        Self { bleed_mm: 3.0, safe_mm: 4.0, ink_limit: 300 }
    }
}

impl PrintSetup {
    /// Valeurs ramenées dans des bornes raisonnables (une valeur non finie compte pour 0).
    pub fn sanitized(self) -> Self {
        let mm = |v: f32, max: f32| if v.is_finite() { v.clamp(0.0, max) } else { 0.0 };
        Self { bleed_mm: mm(self.bleed_mm, 50.0), safe_mm: mm(self.safe_mm, 100.0), ink_limit: if self.ink_limit == 0 { 0 } else { self.ink_limit.clamp(150, 400) } }
    }
}

/// Millimètres → pixels à cette résolution (arrondi).
pub fn mm_to_px(mm: f32, dpi: f32) -> u32 {
    let dpi = if dpi.is_finite() && dpi > 0.0 { dpi } else { 72.0 };
    (mm.max(0.0) / MM_PER_INCH * dpi).round() as u32
}

/// Total d'encre d'une couleur CMJN 8 bits, en unités 8 bits (255 = 100 %).
#[inline]
pub fn ink_total(c: [u8; 4]) -> u32 {
    c.iter().map(|&v| u32::from(v)).sum()
}

/// Limite en % → unités 8 bits.
#[inline]
pub fn limit_units(percent: u16) -> u32 {
    (u32::from(percent) * 255 + 50) / 100
}

/// Ramène une couleur CMJN sous `max` (unités 8 bits) : GCR d'abord (le gris du mélange C+M+J passe au noir), puis
/// réduction proportionnelle de C, M et J. Une couleur déjà sous la limite est rendue telle quelle.
pub fn limit_ink(c: [u8; 4], max: u32) -> [u8; 4] {
    let total = ink_total(c);
    if total <= max {
        return c;
    }
    let [mut cy, mut ma, mut ye, mut k] = c.map(u32::from);
    // GCR : retirer g de chacune des trois couleurs et l'ajouter au noir fait baisser le total de 2 g.
    let gray = cy.min(ma).min(ye).min(255 - k);
    let g = gray.min((total - max).div_ceil(2));
    cy -= g;
    ma -= g;
    ye -= g;
    k += g;
    let colors = cy + ma + ye;
    if colors + k > max && colors > 0 {
        let room = max.saturating_sub(k);
        let scale = |v: u32| v * room / colors;
        (cy, ma, ye) = (scale(cy), scale(ma), scale(ye));
    }
    if cy + ma + ye + k > max {
        k = max.saturating_sub(cy + ma + ye).min(255);
    }
    [cy as u8, ma as u8, ye as u8, k as u8]
}

/// Table de correction (33³ × 3) : chaque nœud sRGB donne sa couleur corrigée (lui-même s'il ne dépasse pas).
fn correction_table(space: CmykSpace, percent: u16) -> Result<Vec<u8>, EngineError> {
    let max = limit_units(percent);
    let level = |i: usize| ((i * 255 * 2 + (N - 1)) / (2 * (N - 1))) as u8;
    let mut nodes = Vec::with_capacity(N * N * N);
    for b in 0..N {
        for g in 0..N {
            for r in 0..N {
                nodes.push([level(r), level(g), level(b)]);
            }
        }
    }
    let separate = |[r, g, b]: [u8; 3]| cmyk::rgb_to_cmyk_pixel(space, r, g, b);
    let mut out: Vec<[u8; 3]> = nodes.clone();
    // Nœuds à corriger, avec leur cible courante.
    let mut pending: Vec<(usize, [u8; 4], u32)> = Vec::new();
    for (i, rgb) in nodes.iter().enumerate() {
        let inks = separate(*rgb)?;
        if ink_total(inks) > max {
            pending.push((i, inks, max));
        }
    }
    for _ in 0..8 {
        if pending.is_empty() {
            break;
        }
        let limited: Vec<[u8; 4]> = pending.iter().map(|&(_, inks, target)| limit_ink(inks, target)).collect();
        let back = cmyk::cmyk_to_rgb(space, &limited)?;
        let mut still = Vec::new();
        for (&(i, inks, target), rgb) in pending.iter().zip(back) {
            out[i] = rgb;
            let got = ink_total(separate(rgb)?);
            if got > max && target > 40 {
                still.push((i, inks, target.saturating_sub(got - max + 3)));
            }
        }
        pending = still;
    }
    Ok(out.into_iter().flatten().collect())
}

impl Document {
    /// Format d'impression du document, s'il en a un.
    pub fn print_setup(&self) -> Option<PrintSetup> {
        self.print
    }

    /// Fonds perdus en pixels (0 sans format d'impression).
    pub fn bleed_px(&self) -> u32 {
        self.print.map_or(0, |p| mm_to_px(p.bleed_mm, self.dpi)).min(self.width().min(self.height()).saturating_sub(1) / 2)
    }

    /// Format fini `[x, y, largeur, hauteur]` en pixels : le document moins ses fonds perdus.
    pub fn trim_rect(&self) -> [u32; 4] {
        let b = self.bleed_px();
        [b, b, self.width() - 2 * b, self.height() - 2 * b]
    }

    /// Nouveau document d'impression : `trim_width` × `trim_height` est le FORMAT FINI ; la zone de travail y ajoute
    /// les fonds perdus (le fond blanc les couvre). `cmyk` : travail en CMJN dans cet espace.
    pub fn new_print(
        name: impl Into<String>,
        trim_width: u32,
        trim_height: u32,
        dpi: f32,
        background: crate::document::Background,
        setup: PrintSetup,
        cmyk: Option<CmykSpace>,
    ) -> Result<Self, EngineError> {
        let setup = setup.sanitized();
        let b = mm_to_px(setup.bleed_mm, dpi);
        let mut doc = Self::with_background(name, trim_width.saturating_add(2 * b), trim_height.saturating_add(2 * b), dpi, background)?;
        doc.print = Some(setup);
        if let Some(space) = cmyk {
            doc.color_mode = crate::color_mode::ColorMode::CmykWorkflow { profile: space.profile, intent: space.intent, bpc: space.bpc };
        }
        Ok(doc)
    }

    /// Image › Format d'impression… : pose (ou retire, `None`) le format d'impression. Un changement de fonds perdus
    /// agrandit ou réduit la zone de travail d'autant de chaque côté (le format fini ne bouge pas). `extend` : les
    /// calques de pixels qui touchent l'ancien bord y sont prolongés (bord recopié), comme on le fait à la main
    /// pour une photo pleine page.
    pub fn set_print_setup(&mut self, setup: Option<PrintSetup>, extend: bool) -> Result<(), EngineError> {
        self.set_print_format(setup, extend, None)
    }

    /// Comme [`Document::set_print_setup`], en changeant aussi la résolution (`dpi`) : les pixels ne bougent pas,
    /// seule la taille imprimée change (3840 px à 72 ppp = 1354,7 mm, à 300 ppp = 325,1 mm). Les fonds perdus, en
    /// mm, sont recalculés en pixels à la nouvelle résolution.
    pub fn set_print_format(&mut self, setup: Option<PrintSetup>, extend: bool, dpi: Option<f32>) -> Result<(), EngineError> {
        let setup = setup.map(PrintSetup::sanitized);
        let old = self.bleed_px() as i32;
        if let Some(dpi) = dpi.filter(|d| d.is_finite() && *d >= 1.0 && *d <= 9600.0) {
            self.dpi = dpi;
        }
        let new = setup.map_or(0, |p| mm_to_px(p.bleed_mm, self.dpi)) as i32;
        let delta = new - old;
        if delta != 0 {
            let (w, h) = (self.width() as i32 + 2 * delta, self.height() as i32 + 2 * delta);
            if w < 1 || h < 1 {
                return Err(EngineError::InvalidDimensions { width: w.max(0) as u32, height: h.max(0) as u32, max: crate::tiles::MAX_DIMENSION });
            }
            self.crop(-delta, -delta, w as u32, h as u32)?;
        }
        self.print = setup;
        if extend && delta > 0 {
            let inner = [delta, delta, self.width() as i32 - delta, self.height() as i32 - delta];
            let ids: Vec<LayerId> = self.layer_ids_bottom_up();
            for id in ids {
                let layer = self.layer(id)?;
                if layer.pixels().is_none() || self.ensure_paintable(id).is_err() {
                    continue;
                }
                self.extend_edges(id, inner)?;
            }
        }
        Ok(())
    }

    /// Tous les calques (groupes compris), dans un ordre stable.
    fn layer_ids_bottom_up(&self) -> Vec<LayerId> {
        let mut out = Vec::new();
        let mut stack: Vec<LayerId> = self.children_of(None).map(<[LayerId]>::to_vec).unwrap_or_default();
        stack.reverse();
        while let Some(id) = stack.pop() {
            out.push(id);
            if let Ok(children) = self.children_of(Some(id)) {
                stack.extend(children.iter().rev().copied());
            }
        }
        out
    }

    /// Prolonge les bords du rectangle `inner` (x0, y0, x1, y1, coordonnées du document) vers l'extérieur : chaque
    /// pixel transparent hors du rectangle prend le pixel le plus proche du bord intérieur, s'il est visible.
    fn extend_edges(&mut self, id: LayerId, [x0, y0, x1, y1]: [i32; 4]) -> Result<(), EngineError> {
        let (ox, oy) = self.layer(id)?.offset;
        let Some(source) = self.layer(id)?.pixels().cloned() else { return Ok(()) };
        let (w, h) = (self.width() as i32, self.height() as i32);
        let mut read = Sampler::new(&source);
        let mut writes: Vec<(i32, i32, [u8; 4])> = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if x >= x0 && x < x1 && y >= y0 && y < y1 {
                    continue;
                }
                let (sx, sy) = (x.clamp(x0, x1 - 1), y.clamp(y0, y1 - 1));
                let from = read.get(sx - ox, sy - oy);
                if from[3] == 0 || read.get(x - ox, y - oy)[3] != 0 {
                    continue;
                }
                writes.push((x - ox, y - oy, from));
            }
        }
        if writes.is_empty() {
            return Ok(());
        }
        let store = self.pixels_mut(id)?;
        for (x, y, px) in writes {
            let (coord, lx, ly) = TileCoord::of_pixel(x, y);
            store.get_mut(coord).set_pixel(lx, ly, px);
        }
        Ok(())
    }

    /// Image › Corriger l'encrage… : ramène sous `percent` les pixels des calques de pixels qui dépassent (dans la
    /// sélection s'il y en a une), séparés dans `space`. Renvoie le nombre de calques modifiés. Les calques
    /// verrouillés, de texte, de forme, de remplissage et les objets dynamiques ne sont pas touchés.
    pub fn correct_ink(&mut self, space: CmykSpace, percent: u16, only: Option<LayerId>) -> Result<usize, EngineError> {
        let percent = percent.clamp(150, 400);
        let table = correction_table(space, percent)?;
        let tables = cmyk::tables(space)?;
        let max = limit_units(percent);
        let ids = match only {
            Some(id) => vec![id],
            None => self.layer_ids_bottom_up(),
        };
        let mut changed = 0;
        for id in ids {
            if self.layer(id)?.pixels().is_none() || self.ensure_paintable(id).is_err() {
                continue;
            }
            let over = |px: &[u8]| {
                let s = unpremultiply([px[0], px[1], px[2], px[3]]);
                px[3] > 0 && ink_total(cmyk::lookup::<4>(&tables.export, s[0], s[1], s[2])) > max
            };
            let touches = self.layer(id)?.pixels().is_some_and(|store| store.iter().any(|(_, tile)| tile.bytes().chunks_exact(4).any(over)));
            if !touches {
                continue;
            }
            let (export, table) = (&tables.export, &table);
            self.edit_selected_pixels(id, true, move |px, force, _| {
                if px[3] == 0 {
                    return;
                }
                let s = unpremultiply([px[0], px[1], px[2], px[3]]);
                if ink_total(cmyk::lookup::<4>(export, s[0], s[1], s[2])) <= max {
                    return;
                }
                let [r, g, b] = cmyk::lookup::<3>(table, s[0], s[1], s[2]);
                let f = u32::from(force);
                let mix = |new: u8, old: u8| ((u32::from(new) * f + u32::from(old) * (255 - f) + 127) / 255) as u8;
                let fixed = premultiply([mix(r, s[0]), mix(g, s[1]), mix(b, s[2]), px[3]]);
                px[..3].copy_from_slice(&fixed[..3]);
            })?;
            changed += 1;
        }
        Ok(changed)
    }
}

/// Résultat de l'analyse d'encrage d'un document.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InkReport {
    pub limit: u16,
    /// Encrage maximal rencontré, en %.
    pub max_percent: u16,
    /// Pixels au-delà de la limite.
    pub over_pixels: u64,
    /// Grille des dépassements : `cols` × `rows` cases de `cell` pixels, 1 = au moins un pixel dépasse.
    pub cell: u32,
    pub cols: u32,
    pub rows: u32,
    pub mask: Vec<u8>,
    /// Zones qui dépassent (cases voisines réunies), `[x, y, largeur, hauteur]` en pixels du document, les plus
    /// grandes d'abord.
    pub regions: Vec<[u32; 4]>,
}

/// Analyse l'encrage de l'image aplatie (straight RGBA) dans `space`, pour la limite `percent`.
pub fn ink_report_of(width: u32, height: u32, rgba: &[u8], space: CmykSpace, percent: u16) -> Result<InkReport, EngineError> {
    let inks = cmyk::rgba_to_cmyk(rgba, space)?;
    let max = limit_units(percent.max(1));
    let cell = width.max(height).div_ceil(256).max(1);
    let (cols, rows) = (width.div_ceil(cell), height.div_ceil(cell));
    // Par rangée de pixels : maximum, dépassements, cases touchées.
    let per_row: Vec<(u32, u64, Vec<u32>)> = inks
        .par_chunks_exact(width as usize * 4)
        .map(|row| {
            let (mut top, mut count, mut cells) = (0u32, 0u64, Vec::new());
            for (x, px) in row.chunks_exact(4).enumerate() {
                let t = ink_total([px[0], px[1], px[2], px[3]]);
                top = top.max(t);
                if t > max {
                    count += 1;
                    let c = x as u32 / cell;
                    if cells.last() != Some(&c) {
                        cells.push(c);
                    }
                }
            }
            (top, count, cells)
        })
        .collect();
    let mut mask = vec![0u8; (cols * rows) as usize];
    let (mut top, mut over) = (0u32, 0u64);
    for (y, (t, count, cells)) in per_row.into_iter().enumerate() {
        top = top.max(t);
        over += count;
        let row = y as u32 / cell;
        for c in cells {
            mask[(row * cols + c) as usize] = 1;
        }
    }
    // Zones : composantes connexes (8-voisinage) de la grille.
    let mut seen = vec![false; mask.len()];
    let mut regions: Vec<([u32; 4], u32)> = Vec::new();
    for start in 0..mask.len() {
        if mask[start] == 0 || seen[start] {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (u32::MAX, u32::MAX, 0, 0, 0);
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let (cx, cy) = (i as u32 % cols, i as u32 / cols);
            (x0, y0, x1, y1, n) = (x0.min(cx), y0.min(cy), x1.max(cx), y1.max(cy), n + 1);
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (cx as i32 + dx, cy as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= cols as i32 || ny >= rows as i32 {
                        continue;
                    }
                    let j = (ny as u32 * cols + nx as u32) as usize;
                    if mask[j] == 1 && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        let (px, py) = (x0 * cell, y0 * cell);
        let (pw, ph) = (((x1 + 1) * cell).min(width) - px, ((y1 + 1) * cell).min(height) - py);
        regions.push(([px, py, pw, ph], n));
    }
    regions.sort_by_key(|r| std::cmp::Reverse(r.1));
    Ok(InkReport {
        limit: percent,
        max_percent: ((top * 100 + 127) / 255) as u16,
        over_pixels: over,
        cell,
        cols,
        rows,
        mask,
        regions: regions.into_iter().map(|(r, _)| r).collect(),
    })
}

/// Analyse d'encrage d'un document (aplati à pleine résolution).
pub fn ink_report(doc: &Document, space: CmykSpace, percent: u16) -> Result<InkReport, EngineError> {
    let flat = crate::io::flatten_for_export(doc, 1.0)?;
    ink_report_of(flat.width, flat.height, &flat.rgba, space, percent)
}

/// Noirs à l'impression, appliqués à la séparation (PDF/X-4) :
/// - `pure_black` : le noir pur (#000) devient 100 % N seul — textes et filets restent nets même si les plaques sont
///   un peu décalées (sur quatre plaques, un décalage ferait un liseré coloré) ;
/// - `rich_black` : l'intérieur des grands aplats noirs (à plus de `inset` pixels d'un bord) devient un noir riche
///   C60 M40 J40 N100 (240 %), plus profond ; leur bord reste en noir seul, ce qui évite le même liseré.
///
/// `rgba` : l'image aplatie (alpha droit), `inks` : sa séparation, modifiée en place.
pub fn apply_black_rules(width: u32, height: u32, rgba: &[u8], inks: &mut [u8], pure_black: bool, rich_black: bool, inset: u32) {
    if !pure_black && !rich_black {
        return;
    }
    let (w, h) = (width as usize, height as usize);
    let black: Vec<bool> = rgba.chunks_exact(4).map(|px| px[3] == 255 && px[0] <= 6 && px[1] <= 6 && px[2] <= 6).collect();
    if !black.iter().any(|&b| b) {
        return;
    }
    // Érosion du masque (carré de côté 2 × inset + 1) en deux passes séparables : un pixel « intérieur » n'a que du
    // noir autour de lui.
    let interior: Vec<bool> = if rich_black && inset > 0 {
        let r = inset as usize;
        let erode_rows = |src: &[bool]| -> Vec<bool> {
            let mut out = vec![false; src.len()];
            for y in 0..h {
                let row = &src[y * w..(y + 1) * w];
                let mut run = 0usize; // noirs consécutifs jusqu'à x inclus
                let mut runs = vec![0usize; w];
                for x in 0..w {
                    run = if row[x] { run + 1 } else { 0 };
                    runs[x] = run;
                }
                for x in 0..w {
                    let right = (x + r).min(w - 1);
                    out[y * w + x] = x >= r && x + r < w && runs[right] > 2 * r;
                }
            }
            out
        };
        let rows = erode_rows(&black);
        let mut out = vec![false; rows.len()];
        for x in 0..w {
            let mut run = 0usize;
            let mut runs = vec![0usize; h];
            for y in 0..h {
                run = if rows[y * w + x] { run + 1 } else { 0 };
                runs[y] = run;
            }
            for y in 0..h {
                let bottom = (y + r).min(h - 1);
                out[y * w + x] = y >= r && y + r < h && runs[bottom] > 2 * r;
            }
        }
        out
    } else {
        vec![false; black.len()]
    };
    for (i, ink) in inks.chunks_exact_mut(4).enumerate() {
        if !black[i] {
            continue;
        }
        if interior[i] {
            ink.copy_from_slice(&[153, 102, 102, 255]);
        } else if pure_black {
            ink.copy_from_slice(&[0, 0, 0, 255]);
        }
    }
}

/// Vérification avant impression (Fichier › Vérifier avant impression…).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    pub ink: InkReport,
    /// Fonds perdus remplis, par côté (haut, droite, bas, gauche) : le contenu va jusqu'au bord de la zone de travail
    /// au lieu de s'arrêter au format fini. `None` sans fonds perdus.
    pub bleed_filled: Option<[bool; 4]>,
    /// Calques de texte visibles qui sortent de la zone de sécurité : (calque, nom, `[x, y, l, h]`).
    pub texts_outside: Vec<(LayerId, String, [i32; 4])>,
}

/// Vérifie le document : encrage, fonds perdus, textes hors de la zone de sécurité.
pub fn preflight(doc: &Document, space: CmykSpace, percent: u16) -> Result<Preflight, EngineError> {
    let flat = crate::io::flatten_for_export(doc, 1.0)?;
    let ink = ink_report_of(flat.width, flat.height, &flat.rgba, space, percent)?;
    let b = doc.bleed_px() as usize;
    let (w, h) = (flat.width as usize, flat.height as usize);
    let bleed_filled = (b > 0 && w > 2 * b && h > 2 * b).then(|| {
        let px = |x: usize, y: usize| {
            let i = (y * w + x) * 4;
            let p = &flat.rgba[i..i + 4];
            // Posé sur du blanc : c'est ce qui s'imprime.
            let a = u32::from(p[3]);
            [0, 1, 2].map(|c| ((u32::from(p[c]) * a + 255 * (255 - a)) / 255) as i32)
        };
        // Papier nu : rien d'imprimé (transparent ou blanc). Un côté est « vide » quand, le long de ce côté, le dessin
        // touche la coupe mais que la bande des fonds perdus derrière lui reste papier nu (plus de 3 % des points) :
        // après la coupe, un liseré blanc apparaîtrait. Un contenu qui change dans les fonds perdus (photo, cercle
        // qui déborde, dégradé) est normal et n'est pas signalé.
        let paper = |c: [i32; 3]| c.iter().all(|&v| v >= 245);
        let side = |pairs: &mut dyn Iterator<Item = ((usize, usize), (usize, usize))>| {
            let (mut n, mut bad) = (0, 0);
            for (outer, inner) in pairs {
                n += 1;
                if paper(px(outer.0, outer.1)) && !paper(px(inner.0, inner.1)) {
                    bad += 1;
                }
            }
            bad * 100 <= n * 3
        };
        [
            side(&mut (b..w - b).map(|x| ((x, 0), (x, b)))),
            side(&mut (b..h - b).map(|y| ((w - 1, y), (w - 1 - b, y)))),
            side(&mut (b..w - b).map(|x| ((x, h - 1), (x, h - 1 - b)))),
            side(&mut (b..h - b).map(|y| ((0, y), (b, y)))),
        ]
    });
    let mut texts_outside = Vec::new();
    if let Some(setup) = doc.print.filter(|p| p.safe_mm > 0.0) {
        let s = mm_to_px(setup.safe_mm, doc.dpi) as i32;
        let [tx, ty, tw, th] = doc.trim_rect().map(|v| v as i32);
        let (x0, y0, x1, y1) = (tx + s, ty + s, tx + tw - s, ty + th - s);
        for id in doc.layer_ids_bottom_up() {
            let layer = doc.layer(id)?;
            if layer.text().is_none() || !layer.props.visible {
                continue;
            }
            let Some(bounds) = layer.pixels().and_then(|p| p.pixel_bounds(|v| v[3] > 0)) else { continue };
            let (ox, oy) = layer.offset;
            let (bx0, by0, bx1, by1) = (bounds.0 + ox, bounds.1 + oy, bounds.2 + ox, bounds.3 + oy);
            if bx0 < x0 || by0 < y0 || bx1 > x1 || by1 > y1 {
                texts_outside.push((id, layer.props.name.clone(), [bx0, by0, bx1 - bx0, by1 - by0]));
            }
        }
    }
    Ok(Preflight { ink, bleed_filled, texts_outside })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmyk::CmykProfile;
    use crate::document::Background;
    use crate::tiles::TileStore;

    #[test]
    fn limit_ink_prefers_black_then_scales_colors() {
        let max = limit_units(300);
        let rich = [255, 255, 255, 255];
        let fixed = limit_ink(rich, max);
        assert!(ink_total(fixed) <= max, "{fixed:?}");
        assert_eq!(fixed[3], 255, "le noir reste plein");
        let ok = [100, 80, 60, 20];
        assert_eq!(limit_ink(ok, max), ok, "sous la limite : intact");
        // GCR seul suffit : le total baisse sans toucher l'équilibre plus que nécessaire.
        let dark = [230, 220, 210, 200];
        let fixed = limit_ink(dark, max);
        assert!(ink_total(fixed) <= max && fixed[3] > 150, "{fixed:?}");
        for c in [[255, 0, 255, 255], [255, 255, 0, 200], [0, 0, 0, 255]] {
            assert!(ink_total(limit_ink(c, max)) <= max.max(255));
        }
    }

    #[test]
    fn black_rules_keep_edges_in_black_only_and_enrich_large_flats() {
        let (w, h) = (40u32, 20u32);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| if i % w < 30 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }).collect();
        let mut inks = cmyk::rgba_to_cmyk(&rgba, CmykProfile::Fogra39.into()).unwrap();
        apply_black_rules(w, h, &rgba, &mut inks, true, true, 3);
        let at = |x: u32, y: u32| &inks[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize];
        assert_eq!(at(0, 0), [0, 0, 0, 255], "bord : noir seul");
        assert_eq!(at(10, 10), [153, 102, 102, 255], "intérieur : noir riche");
        assert_eq!(at(29, 10), [0, 0, 0, 255], "bord droit de l'aplat");
        assert_eq!(at(35, 10), [0, 0, 0, 0], "blanc : pas d'encre");
    }

    #[test]
    fn preflight_finds_unfilled_bleed_and_texts_near_the_edge() {
        let mut doc = Document::new("photo", 100, 60, 254.0).unwrap();
        let mut pixels = TileStore::new();
        pixels.fill(100, 60, [20, 120, 200, 255]);
        doc.add_pixel_layer("photo", pixels, None, 0).unwrap();
        doc.set_print_setup(Some(PrintSetup::default()), false).unwrap();
        let report = preflight(&doc, CmykProfile::Fogra39.into(), 300).unwrap();
        assert_eq!(report.bleed_filled, Some([false; 4]), "le bleu s'arrête à la coupe");
        let mut doc2 = Document::new("photo", 100, 60, 254.0).unwrap();
        let mut pixels = TileStore::new();
        pixels.fill(100, 60, [20, 120, 200, 255]);
        doc2.add_pixel_layer("photo", pixels, None, 0).unwrap();
        doc2.set_print_setup(Some(PrintSetup::default()), true).unwrap();
        assert_eq!(preflight(&doc2, CmykProfile::Fogra39.into(), 300).unwrap().bleed_filled, Some([true; 4]));
        // Un dessin qui change dans les fonds perdus (ici, une bande sombre tout en haut) n'est pas une erreur.
        let mut pixels = TileStore::new();
        pixels.fill(160, 8, [10, 10, 10, 255]);
        doc2.add_pixel_layer("bande", pixels, None, 1).unwrap();
        assert_eq!(preflight(&doc2, CmykProfile::Fogra39.into(), 300).unwrap().bleed_filled, Some([true; 4]));
    }

    #[test]
    fn changing_the_resolution_keeps_the_pixels_and_rescales_the_bleed() {
        let mut doc = Document::new("écran", 3840, 2160, 72.0).unwrap();
        doc.set_print_format(Some(PrintSetup { bleed_mm: 3.0, safe_mm: 4.0, ink_limit: 300 }), false, Some(300.0)).unwrap();
        assert_eq!(doc.dpi, 300.0);
        assert_eq!(doc.trim_rect(), [35, 35, 3840, 2160], "format fini intact, fonds perdus à 300 ppp");
        doc.set_print_format(Some(PrintSetup { bleed_mm: 3.0, safe_mm: 4.0, ink_limit: 300 }), false, Some(150.0)).unwrap();
        assert_eq!(doc.trim_rect(), [18, 18, 3840, 2160]);
    }

    #[test]
    fn mm_and_pixels() {
        assert_eq!(mm_to_px(25.4, 300.0), 300);
        assert_eq!(mm_to_px(3.0, 300.0), 35);
        assert_eq!(mm_to_px(85.0, 300.0), 1004);
    }

    #[test]
    fn a_print_document_contains_its_bleed_and_keeps_its_trim() {
        let doc = Document::new_print("carte", 1004, 650, 300.0, Background::White, PrintSetup::default(), Some(CmykProfile::Fogra39.into())).unwrap();
        assert_eq!((doc.width(), doc.height()), (1004 + 70, 650 + 70));
        assert_eq!(doc.trim_rect(), [35, 35, 1004, 650]);
        assert!(matches!(doc.color_mode(), crate::color_mode::ColorMode::CmykWorkflow { .. }));
    }

    #[test]
    fn adding_bleed_grows_the_canvas_and_extends_edge_content() {
        let mut doc = Document::new("photo", 100, 60, 254.0).unwrap(); // 10 px par mm
        let mut pixels = TileStore::new();
        pixels.fill(100, 60, [200, 10, 10, 255]);
        let id = doc.add_pixel_layer("photo", pixels, None, 0).unwrap();
        doc.set_print_setup(Some(PrintSetup { bleed_mm: 3.0, safe_mm: 4.0, ink_limit: 300 }), true).unwrap();
        assert_eq!((doc.width(), doc.height()), (160, 120));
        assert_eq!(doc.trim_rect(), [30, 30, 100, 60]);
        let store = doc.layer(id).unwrap().pixels().unwrap().clone();
        let (ox, oy) = doc.layer(id).unwrap().offset;
        let mut s = Sampler::new(&store);
        assert_eq!(s.get(0 - ox, 0 - oy), [200, 10, 10, 255], "coin des fonds perdus rempli");
        assert_eq!(s.get(159 - ox, 60 - oy), [200, 10, 10, 255]);
        // Retirer le format d'impression rend la taille d'origine.
        doc.set_print_setup(None, false).unwrap();
        assert_eq!((doc.width(), doc.height()), (100, 60));
        // Sans prolongement, les fonds perdus restent vides.
        let mut plain = Document::new("photo", 100, 60, 254.0).unwrap();
        let mut pixels = TileStore::new();
        pixels.fill(100, 60, [200, 10, 10, 255]);
        let id = plain.add_pixel_layer("photo", pixels, None, 0).unwrap();
        plain.set_print_setup(Some(PrintSetup::default()), false).unwrap();
        let store = plain.layer(id).unwrap().pixels().unwrap().clone();
        let (ox, oy) = plain.layer(id).unwrap().offset;
        assert_eq!(Sampler::new(&store).get(-ox, -oy)[3], 0);
    }

    #[test]
    fn correcting_ink_brings_every_pixel_under_the_limit() {
        let space: CmykSpace = CmykProfile::Fogra39.into();
        let mut doc = Document::new("noir", 64, 64, 300.0).unwrap();
        let mut pixels = TileStore::new();
        pixels.fill(64, 64, [0, 0, 0, 255]);
        doc.add_pixel_layer("aplat", pixels, None, 0).unwrap();
        let mut pixels = TileStore::new();
        pixels.fill(32, 64, [20, 10, 60, 255]);
        doc.add_pixel_layer("bleu nuit", pixels, None, 0).unwrap();
        let before = ink_report(&doc, space, 280).unwrap();
        assert!(before.over_pixels > 0 && before.max_percent > 280, "{} %", before.max_percent);
        assert!(!before.regions.is_empty());
        assert!(doc.correct_ink(space, 280, None).unwrap() >= 1);
        let after = ink_report(&doc, space, 280).unwrap();
        assert_eq!(after.over_pixels, 0, "encore {} % au maximum", after.max_percent);
        assert!(after.max_percent > 240, "le noir reste un noir profond : {} %", after.max_percent);
    }
}
