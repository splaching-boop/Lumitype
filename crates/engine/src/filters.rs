//! Filtres destructifs appliqués aux pixels d'un calque : flou gaussien et accentuation.
//!
//! Un filtre a besoin du voisinage de chaque pixel, y compris par-dessus les frontières de
//! tuiles : le calque est donc d'abord mis à plat dans un tampon dense, filtré, puis redécoupé.
//! Tout se fait en alpha prémultiplié, ce qui évite les franges sombres au bord des zones transparentes.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::tiles::TileStore;

/// Pastille du flou de profondeur de champ : position relative au contenu du calque (0 à 1 sur
/// chaque axe) et force du flou à cet endroit, en pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlurPin {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Filter {
    /// Fluidité : déformation libre décrite par une grille de `cols x rows` nœuds répartis sur le
    /// contenu du calque. Chaque nœud donne, en fraction de la largeur et de la hauteur du contenu,
    /// le décalage vers le point de l'image d'ORIGINE qui doit s'afficher là (déformation inverse).
    Liquify { cols: u32, rows: u32, dx: Vec<f32>, dy: Vec<f32> },
    /// Flou de profondeur de champ : le flou varie continûment d'une pastille à l'autre. Une seule
    /// pastille floute toute l'image uniformément ; une seconde, réglée autrement, crée un dégradé
    /// de netteté entre les deux ; et ainsi de suite.
    FieldBlur { pins: Vec<BlurPin> },
    /// `radius` en pixels (écart-type du flou), de 0,1 à 250.
    GaussianBlur { radius: f32 },
    /// Masque flou : `amount` de 1 à 500 %, `radius` en pixels.
    Sharpen { radius: f32, amount: f32 },
    /// Flou directionnel : `angle` en degrés, `distance` en pixels (1 à 500).
    MotionBlur { angle: f32, distance: f32 },
    /// Flou d'objectif (bokeh) : noyau en disque de `radius` pixels (1 à 100) ; `highlights`
    /// (0 à 100) fait ressortir les hautes lumières en pastilles, comme un vrai objectif.
    /// Avec une sélection, c'est l'outil du « flou de profondeur de champ » manuel : on
    /// sélectionne l'arrière-plan, et lui seul part dans le flou.
    LensBlur { radius: f32, highlights: f32 },
    /// Bruit : `amount` de 0 à 100 % ; `monochrome` = même grain sur les trois couches.
    Noise { amount: f32, monochrome: bool },
    /// Tramage ordonné (matrice de Bayer 8 x 8) : `levels` tons par couche (2 à 8), points de
    /// `scale` pixels (1 à 16) ; `monochrome` = noir et blanc.
    Dither { scale: u32, levels: u32, monochrome: bool },
    /// Verre (Photoshop : Déformation > Verre) : l'image est vue à travers une plaque de verre dont le
    /// relief est donné par une texture ; là où le relief penche, l'image est déviée. `distortion` de
    /// 0 à 20, `smoothness` de 1 à 15 (adoucit le relief), `scale` de 50 à 200 % (taille de la texture),
    /// `invert` : creux et bosses échangés.
    Glass { distortion: f32, smoothness: f32, texture: GlassTexture, scale: f32, invert: bool },
    /// Présence, comme dans Lightroom, avec un vrai calcul de voisinage (ce qu'un calque de réglage,
    /// qui travaille pixel par pixel, ne peut pas faire). Chaque curseur va de -100 à 100.
    /// `texture` : contraste des détails moyens (grain de peau, feuillage) ; `clarity` : contraste
    /// local des tons moyens, à grande échelle ; `dehaze` : suppression du voile atmosphérique.
    Presence { texture: f32, clarity: f32, dehaze: f32 },
    /// Bruit gaussien : le flou granuleux des affiches contemporaines. L'image est floutée (`radius`,
    /// 0 à 250 px), puis chaque pixel va chercher sa couleur à une petite distance tirée au sort selon
    /// une loi normale (`scatter`, 0 à 100 px) : les transitions se dissolvent en poussière au lieu de
    /// se fondre en douceur. Un grain gaussien (`amount`, 0 à 100 %) est enfin ajouté, de la taille
    /// `grain` (1 à 8 px), monochrome ou coloré.
    GaussianNoise { radius: f32, scatter: f32, amount: f32, grain: u32, monochrome: bool },
    /// Tramage couleur : seule la luminosité est tramée (matrice de Bayer 16 x 16) ; chaque point
    /// allumé garde la teinte de l'image, le reste est noir. `scale` : taille du point (1 à 16 px),
    /// `contrast` (0 à 200 %, 100 = neutre), `threshold` (0 à 100, 50 = neutre : plus haut = moins
    /// de points), `strength` (0 à 100 % : mélange avec l'image d'origine).
    ColorDither { scale: u32, contrast: f32, threshold: f32, strength: f32 },
    /// Aberration chromatique : le rouge et le bleu sont décalés en sens opposés, de `amount` pixels
    /// (0 à 100). `radial` : le décalage part du centre et grandit vers les bords, comme un objectif ;
    /// sinon il est uniforme, dans la direction `angle` (degrés).
    ChromaticAberration { amount: f32, angle: f32, radial: bool },
    /// Image recomposée en caractères : cellules de `cell` pixels de haut (6 à 64) ;
    /// `color` = chaque caractère prend la couleur de sa cellule, sinon blanc sur noir.
    Ascii { cell: u32, color: bool },
}

impl Filter {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Liquify { .. } => "Fluidité",
            Self::FieldBlur { .. } => "Flou de profondeur de champ",
            Self::GaussianBlur { .. } => "Flou gaussien",
            Self::Sharpen { .. } => "Accentuation",
            Self::MotionBlur { .. } => "Flou directionnel",
            Self::LensBlur { .. } => "Flou d'objectif",
            Self::Noise { .. } => "Ajout de bruit",
            Self::Dither { .. } => "Tramage",
            Self::Glass { .. } => "Verre",
            Self::Presence { .. } => "Texture, clarté, voile",
            Self::GaussianNoise { .. } => "Bruit gaussien",
            Self::ColorDither { .. } => "Tramage couleur",
            Self::ChromaticAberration { .. } => "Aberration chromatique",
            Self::Ascii { .. } => "ASCII",
        }
    }
}

impl Filter {
    /// Le même filtre pour une image réduite d'un facteur `k` (aperçu) : toutes les longueurs suivent.
    pub fn scaled(&self, k: f32) -> Self {
        match self.clone() {
            // Décalages relatifs au contenu : ils ne dépendent pas de l'échelle.
            liquify @ Self::Liquify { .. } => liquify,
            Self::FieldBlur { pins } => Self::FieldBlur { pins: pins.into_iter().map(|p| BlurPin { blur: p.blur * k, ..p }).collect() },
            Self::GaussianBlur { radius } => Self::GaussianBlur { radius: radius * k },
            Self::Sharpen { radius, amount } => Self::Sharpen { radius: radius * k, amount },
            Self::MotionBlur { angle, distance } => Self::MotionBlur { angle, distance: distance * k },
            Self::LensBlur { radius, highlights } => Self::LensBlur { radius: radius * k, highlights },
            Self::Noise { amount, monochrome } => Self::Noise { amount, monochrome },
            Self::Dither { scale, levels, monochrome } => Self::Dither { scale: ((scale as f32 * k).round() as u32).max(1), levels, monochrome },
            // Les rayons se déduisent de la taille de l'image : rien à mettre à l'échelle.
            presence @ Self::Presence { .. } => presence,
            // La déviation et le relief suivent l'image ; la taille de la texture aussi (via `scale`, hors de sa plage habituelle).
            Self::Glass { distortion, smoothness, texture, scale, invert } => Self::Glass { distortion: distortion * k, smoothness: (smoothness * k).max(0.5), texture, scale: scale * k, invert },
            Self::GaussianNoise { radius, scatter, amount, grain, monochrome } => {
                Self::GaussianNoise { radius: radius * k, scatter: scatter * k, amount, grain: ((grain as f32 * k).round() as u32).max(1), monochrome }
            }
            Self::ColorDither { scale, contrast, threshold, strength } => Self::ColorDither { scale: ((scale as f32 * k).round() as u32).max(1), contrast, threshold, strength },
            Self::ChromaticAberration { amount, angle, radial } => Self::ChromaticAberration { amount: amount * k, angle, radial },
            Self::Ascii { cell, color } => Self::Ascii { cell: ((cell as f32 * k).round() as u32).max(6), color },
        }
    }
}

/// Plus grand tampon de travail accepté, en pixels.
const MAX_AREA: i64 = 80_000_000;

/// Mémoire que les filtres s'autorisent, en octets (0 = pas de limite). La coque la règle d'après la
/// mémoire de la machine : mieux vaut un message clair qu'une application tuée par le système.
static MEMORY_BUDGET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Mémoire de travail par pixel, au plus fort : l'image convertie en flottants (16 octets), le
/// résultat, et les tampons de flou. Estimation volontairement prudente.
const BYTES_PER_PIXEL: u64 = 48;

pub fn set_memory_budget(bytes: u64) {
    MEMORY_BUDGET.store(bytes, std::sync::atomic::Ordering::Relaxed);
}

pub fn memory_budget() -> u64 {
    MEMORY_BUDGET.load(std::sync::atomic::Ordering::Relaxed)
}

/// Applique le filtre à tout le contenu d'un calque (repère local). Le flou s'étale : le
/// résultat peut être plus grand que l'original.
pub fn apply(pixels: &TileStore, filter: &Filter) -> Result<TileStore, EngineError> {
    let filter = filter.clone();
    let Some((x0, y0, x1, y1)) = pixels.pixel_bounds(|px| px[3] > 0) else { return Ok(TileStore::new()) };
    // Marge : de combien le résultat peut déborder du contenu d'origine.
    let margin = match &filter {
        // Une déformation peut pousser de la matière hors du contenu : la marge couvre le plus grand décalage.
        Filter::Liquify { dx, dy, .. } => {
            let reach = |field: &[f32], extent: i32| field.iter().fold(0.0f32, |m, v| m.max(v.abs())) * extent as f32;
            (reach(dx, x1 - x0).max(reach(dy, y1 - y0)).ceil() as i32 + 2).clamp(2, 4096)
        }
        Filter::FieldBlur { pins } => (pins.iter().map(|p| p.blur).fold(0.0f32, f32::max).clamp(0.0, 250.0) * 1.5).ceil() as i32 + 1,
        Filter::GaussianBlur { radius } => (radius.clamp(0.1, 250.0) * 3.0).ceil() as i32 + 1,
        Filter::MotionBlur { distance, .. } => (distance.clamp(1.0, 500.0) / 2.0).ceil() as i32 + 1,
        Filter::LensBlur { radius, .. } => radius.clamp(1.0, 100.0).ceil() as i32 + 1,
        Filter::ChromaticAberration { amount, .. } => amount.clamp(0.0, 100.0).ceil() as i32 + 1,
        // Aucune marge : le verre ne dévie qu'à l'intérieur du contenu (les bords lisent leur propre pixel).
        Filter::Glass { .. } => 0,
        Filter::GaussianNoise { radius, scatter, .. } => (radius.clamp(0.0, 250.0) * 3.0 + scatter.clamp(0.0, 100.0) * 3.0).ceil() as i32 + 1,
        _ => 0,
    };
    let (left, top) = (x0 - margin, y0 - margin);
    let (w, h) = ((x1 - x0 + 2 * margin) as usize, (y1 - y0 + 2 * margin) as usize);
    if (w as i64) * (h as i64) > MAX_AREA {
        return Err(EngineError::InvalidOperation("calque trop grand pour ce filtre : réduisez la zone ou le rayon".into()));
    }
    // Refus net plutôt que mémoire épuisée : le système tue l'application sans prévenir.
    let needed = (w as u64) * (h as u64) * BYTES_PER_PIXEL;
    let budget = memory_budget();
    if budget > 0 && needed > budget {
        let giga = |bytes: u64| bytes as f64 / 1_073_741_824.0;
        return Err(EngineError::InvalidOperation(format!(
            "ce filtre demanderait environ {:.1} Go de mémoire, pour {:.1} Go disponibles : réduisez la zone sélectionnée ou le rayon",
            giga(needed),
            giga(budget)
        )));
    }
    let source: Vec<f32> = pixels.translated(-left, -top).to_premultiplied(w as u32, h as u32).iter().map(|v| f32::from(*v)).collect();
    let quantize = |values: &[f32]| -> Vec<u8> { values.iter().map(|v| (v + 0.5).clamp(0.0, 255.0) as u8).collect() };
    let result: Vec<u8> = match filter {
        Filter::Liquify { cols, rows, dx, dy } => {
            let nodes = (cols as usize) * (rows as usize);
            if cols < 2 || rows < 2 || dx.len() != nodes || dy.len() != nodes {
                return Err(EngineError::InvalidOperation("grille de déformation incohérente".into()));
            }
            let content = ((x1 - x0) as f32, (y1 - y0) as f32);
            quantize(&warp(&source, w, h, margin as f32, content, (cols as usize, rows as usize), &dx, &dy))
        }
        Filter::FieldBlur { pins } => {
            // Les pastilles sont données par rapport au contenu ; le tampon a une marge autour.
            let content = ((x1 - x0) as f32, (y1 - y0) as f32);
            let placed: Vec<(f32, f32, f32)> = pins.iter().map(|p| (margin as f32 + p.x * content.0, margin as f32 + p.y * content.1, p.blur.clamp(0.0, 250.0))).collect();
            field(&source, w, h, &placed)
        }
        Filter::GaussianBlur { radius } => quantize(&gaussian(&source, w, h, radius.clamp(0.1, 250.0))),
        Filter::Sharpen { radius, amount } => {
            // Masque flou : original + quantité x (original - flou). L'alpha n'est pas accentué,
            // et la couleur prémultipliée ne doit jamais le dépasser.
            let blurred = gaussian(&source, w, h, radius.clamp(0.1, 250.0));
            let k = amount.clamp(1.0, 500.0) / 100.0;
            source
                .chunks_exact(4)
                .zip(blurred.chunks_exact(4))
                .flat_map(|(px, soft)| {
                    let alpha = px[3];
                    let channel = |c: usize| (px[c] + (px[c] - soft[c]) * k).clamp(0.0, alpha);
                    [channel(0), channel(1), channel(2), alpha].map(|v| (v + 0.5) as u8)
                })
                .collect()
        }
        Filter::MotionBlur { angle, distance } => quantize(&motion(&source, w, h, angle, distance.clamp(1.0, 500.0))),
        Filter::LensBlur { radius, highlights } => quantize(&lens(&source, w, h, radius.clamp(1.0, 100.0), highlights.clamp(0.0, 100.0) / 100.0)),
        Filter::Noise { amount, monochrome } => noise(&source, w, amount.clamp(0.0, 100.0) / 100.0, monochrome),
        Filter::Dither { scale, levels, monochrome } => dither(&source, w, scale.clamp(1, 16) as usize, levels.clamp(2, 8), monochrome),
        Filter::Glass { distortion, smoothness, texture, scale, invert } => {
            let custom = if texture == GlassTexture::Custom { glass_texture() } else { None };
            if texture == GlassTexture::Custom && custom.is_none() {
                return Err(EngineError::InvalidOperation("aucune texture chargée : choisissez-en une avec « Charger une texture… »".into()));
            }
            quantize(&glass(&source, w, h, distortion.clamp(0.0, 20.0), smoothness.clamp(0.5, 15.0), texture, custom.as_deref(), scale.clamp(5.0, 400.0) / 100.0, invert))
        }
        Filter::Presence { texture, clarity, dehaze } => {
            let unit = |v: f32| v.clamp(-100.0, 100.0) / 100.0;
            quantize(&presence(&source, w, h, unit(texture), unit(clarity), unit(dehaze)))
        }
        Filter::GaussianNoise { radius, scatter, amount, grain, monochrome } => {
            let blurred = if radius >= 0.1 { gaussian(&source, w, h, radius.clamp(0.1, 250.0)) } else { source.clone() };
            gaussian_noise(&blurred, w, h, margin as usize, scatter.clamp(0.0, 100.0), amount.clamp(0.0, 100.0) / 100.0, grain.clamp(1, 8) as usize, monochrome)
        }
        Filter::ColorDither { scale, contrast, threshold, strength } => {
            color_dither(&source, w, scale.clamp(1, 16) as usize, contrast.clamp(0.0, 200.0) / 100.0, threshold.clamp(0.0, 100.0) / 100.0, strength.clamp(0.0, 100.0) / 100.0)
        }
        Filter::ChromaticAberration { amount, angle, radial } => quantize(&aberration(&source, w, h, amount.clamp(0.0, 100.0), angle, radial)),
        Filter::Ascii { cell, color } => ascii(&source, w, h, cell.clamp(6, 64) as usize, color)?,
    };
    let store = TileStore::from_premultiplied(w as u32, h as u32, &result).ok_or_else(|| EngineError::Corrupt("tampon de filtre de taille inattendue".into()))?;
    Ok(store.translated(left, top))
}

/// Déformation inverse : chaque pixel d'arrivée va chercher sa couleur dans l'image d'origine, au
/// point désigné par la grille (interpolée bilinéairement), avec un échantillonnage bilinéaire.
#[allow(clippy::too_many_arguments)]
fn warp(source: &[f32], w: usize, h: usize, margin: f32, content: (f32, f32), (cols, rows): (usize, usize), dx: &[f32], dy: &[f32]) -> Vec<f32> {
    let sample = |x: f32, y: f32, c: usize| -> f32 {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let at = |ix: f32, iy: f32| if ix < 0.0 || iy < 0.0 || ix >= w as f32 || iy >= h as f32 { 0.0 } else { source[(iy as usize * w + ix as usize) * 4 + c] };
        let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1.0, y0) * tx;
        let bottom = at(x0, y0 + 1.0) * (1.0 - tx) + at(x0 + 1.0, y0 + 1.0) * tx;
        top * (1.0 - ty) + bottom * ty
    };
    let mut out = vec![0.0f32; source.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            // Position dans la grille ; hors du contenu, la déformation du bord se prolonge.
            let u = (((x as f32 + 0.5 - margin) / content.0).clamp(0.0, 1.0)) * (cols - 1) as f32;
            let v = (((y as f32 + 0.5 - margin) / content.1).clamp(0.0, 1.0)) * (rows - 1) as f32;
            let (i, j) = ((u as usize).min(cols - 2), (v as usize).min(rows - 2));
            let (tu, tv) = (u - i as f32, v - j as f32);
            let node = |field: &[f32]| {
                let (a, b, c, d) = (field[j * cols + i], field[j * cols + i + 1], field[(j + 1) * cols + i], field[(j + 1) * cols + i + 1]);
                (a * (1.0 - tu) + b * tu) * (1.0 - tv) + (c * (1.0 - tu) + d * tu) * tv
            };
            let (sx, sy) = (x as f32 + 0.5 + node(dx) * content.0, y as f32 + 0.5 + node(dy) * content.1);
            for c in 0..4 {
                row[x * 4 + c] = sample(sx, sy, c);
            }
        }
    });
    out
}

/// Flou variable : la force du flou en chaque pixel est interpolée entre les pastilles (pondération
/// par l'inverse du carré de la distance), puis l'image est recomposée à partir de quelques flous
/// gaussiens de forces croissantes, chaque pixel mélangeant les deux niveaux qui encadrent la sienne.
/// Les niveaux sont calculés l'un après l'autre : jamais plus de trois tampons en mémoire.
/// Flou par pastilles, **bande par bande**.
///
/// Chaque bande de lignes est floutée aux sept niveaux, avec autour d'elle une marge égale à la
/// portée exacte du plus grand flou : le résultat est celui qu'on obtiendrait sur l'image entière,
/// mais les tampons de travail font la taille d'une bande, pas celle de l'image. Sur 12 mégapixels,
/// le calcul est ainsi passé de 1,5 Go à quelques centaines de mégaoctets.
///
/// Le résultat sort directement en 8 bits : garder un tampon flottant de plus (16 octets par pixel)
/// n'apportait rien.
fn field(source: &[f32], w: usize, h: usize, pins: &[(f32, f32, f32)]) -> Vec<u8> {
    // Bande d'environ 16 Mo de pixels : assez grande pour amortir la marge, assez petite pour la mémoire.
    field_banded(source, w, h, pins, (4_000_000 / w.max(1)).clamp(32, 2048))
}

/// Le calcul lui-même, à hauteur de bande imposée : les tests vérifient ainsi qu'une image découpée
/// en petites bandes donne la même image qu'en une seule.
fn field_banded(source: &[f32], w: usize, h: usize, pins: &[(f32, f32, f32)], band: usize) -> Vec<u8> {
    let quantize = |values: &[f32], out: &mut [u8]| {
        for (slot, value) in out.iter_mut().zip(values) {
            *slot = (value + 0.5).clamp(0.0, 255.0) as u8;
        }
    };
    let strongest = pins.iter().map(|p| p.2).fold(0.0f32, f32::max);
    let mut out = vec![0u8; source.len()];
    if pins.is_empty() || strongest < 0.05 {
        quantize(source, &mut out);
        return out;
    }
    // Force du flou (écart-type) voulue en un pixel : la moitié de la valeur de la pastille, comme un rayon.
    let wanted_at = |x: usize, y: usize| {
        let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
        let (mut sum, mut weights) = (0.0f32, 0.0f32);
        for (px, py, blur) in pins {
            let d2 = (x - px) * (x - px) + (y - py) * (y - py);
            if d2 < 1.0 {
                return blur / 2.0;
            }
            sum += blur / d2;
            weights += 1.0 / d2;
        }
        sum / weights / 2.0
    };
    // Niveaux resserrés vers les faibles flous, là où l'œil distingue le mieux les écarts.
    const LEVELS: usize = 7;
    let sigmas: Vec<f32> = (0..LEVELS).map(|i| strongest / 2.0 * (i as f32 / (LEVELS - 1) as f32).powf(1.6)).collect();
    // Portée exacte du plus grand flou : la somme des demi-largeurs de ses trois moyennes glissantes.
    // Au-delà, un pixel ne peut plus influencer la bande, et les bords de bande sont donc exacts.
    let halo = box_sizes(sigmas[LEVELS - 1]).iter().map(|size| size / 2).sum::<usize>() + 1;
    let rows_max = (band + 2 * halo).min(h);
    let (mut blurred, mut scratch) = (vec![0.0f32; rows_max * w * 4], vec![0.0f32; rows_max * w * 4]);
    let mut mixed = vec![0.0f32; band.min(h) * w * 4];

    let mut y = 0;
    while y < h {
        let stop = (y + band).min(h);
        let (top, bottom) = (y.saturating_sub(halo), (stop + halo).min(h));
        let (rows, span) = (bottom - top, (stop - y) * w * 4);
        let slice = &source[top * w * 4..bottom * w * 4];
        let inside = (y - top) * w * 4;
        let wanted: Vec<f32> = (0..(stop - y) * w).into_par_iter().map(|i| wanted_at(i % w, y + i / w)).collect();
        mixed[..span].fill(0.0);

        for (index, sigma) in sigmas.iter().enumerate() {
            let level: &[f32] = if *sigma < 0.1 {
                &slice[inside..inside + span]
            } else {
                gaussian_into(slice, w, rows, *sigma, &mut blurred[..slice.len()], &mut scratch[..slice.len()]);
                &blurred[inside..inside + span]
            };
            let (below, above) = (index.checked_sub(1).map(|i| sigmas[i]), sigmas.get(index + 1).copied());
            mixed[..span].par_chunks_mut(4).zip(level.par_chunks(4)).zip(wanted.par_iter()).for_each(|((px, blurred), want)| {
                // Poids « en toit » : 1 quand le flou voulu vaut celui du niveau, 0 aux niveaux voisins.
                let weight = match (below, above) {
                    _ if (*want - sigma).abs() < 1e-6 => 1.0,
                    (Some(lo), _) if *want < *sigma => ((*want - lo) / (sigma - lo)).clamp(0.0, 1.0),
                    (_, Some(hi)) if *want > *sigma => ((hi - *want) / (hi - sigma)).clamp(0.0, 1.0),
                    _ => 0.0,
                };
                if weight > 0.0 {
                    for c in 0..4 {
                        px[c] += blurred[c] * weight;
                    }
                }
            });
        }
        quantize(&mixed[..span], &mut out[y * w * 4..stop * w * 4]);
        y = stop;
    }
    out
}

/// Flou directionnel : moyenne des pixels le long d'un segment centré, d'un pas d'un pixel.
fn motion(source: &[f32], w: usize, h: usize, angle: f32, distance: f32) -> Vec<f32> {
    let (dx, dy) = (angle.to_radians().cos(), -angle.to_radians().sin());
    let taps = distance.round().max(1.0) as i32;
    let mut out = vec![0.0; source.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let mut sum = [0.0f32; 4];
            for tap in 0..taps {
                let offset = tap as f32 - (taps - 1) as f32 / 2.0;
                let (sx, sy) = ((x as f32 + dx * offset).round() as isize, (y as f32 + dy * offset).round() as isize);
                if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                    let i = (sy as usize * w + sx as usize) * 4;
                    for c in 0..4 {
                        sum[c] += source[i + c];
                    }
                }
            }
            for c in 0..4 {
                row[x * 4 + c] = sum[c] / taps as f32;
            }
        }
    });
    out
}

/// Flou d'objectif : moyenne sur un disque, les hautes lumières pesant plus lourd que le reste
/// (c'est ce qui dessine les pastilles du bokeh). Chaque ligne du disque est sommée en temps
/// constant grâce à des sommes cumulées : le coût est proportionnel au rayon, pas à sa surface.
fn lens(source: &[f32], w: usize, h: usize, radius: f32, highlights: f32) -> Vec<f32> {
    // Cinq canaux cumulés par ligne : R, V, B, A pondérés, et le poids lui-même.
    let stride = w + 1;
    let mut prefix = vec![0.0f64; stride * h * 5];
    prefix.par_chunks_mut(stride * 5).enumerate().for_each(|(y, line)| {
        for x in 0..w {
            let px = &source[(y * w + x) * 4..(y * w + x) * 4 + 4];
            let luma = if px[3] > 0.0 { (0.299 * px[0] + 0.587 * px[1] + 0.114 * px[2]) / px[3] } else { 0.0 };
            let bright = ((luma - 0.75) / 0.25).clamp(0.0, 1.0);
            let weight = f64::from(1.0 + highlights * 24.0 * bright * bright);
            for c in 0..4 {
                line[(x + 1) * 5 + c] = line[x * 5 + c] + f64::from(px[c]) * weight;
            }
            line[(x + 1) * 5 + 4] = line[x * 5 + 4] + weight;
        }
    });
    let reach = radius.ceil() as isize;
    let halves: Vec<isize> = (-reach..=reach).map(|dy| (radius * radius - (dy * dy) as f32).max(0.0).sqrt().round() as isize).collect();
    let mut out = vec![0.0; source.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let mut sum = [0.0f64; 5];
            let mut area = 0.0f64;
            for (k, half) in halves.iter().enumerate() {
                let sy = y as isize + k as isize - reach;
                area += (2 * half + 1) as f64;
                if sy < 0 || sy as usize >= h {
                    continue;
                }
                let (from, to) = ((x as isize - half).clamp(0, w as isize) as usize, (x as isize + half + 1).clamp(0, w as isize) as usize);
                let line = &prefix[sy as usize * stride * 5..];
                for c in 0..5 {
                    sum[c] += line[to * 5 + c] - line[from * 5 + c];
                }
            }
            // Hors image = transparent, de poids 1 : les bords s'estompent comme pour un flou ordinaire.
            let total = sum[4] + (area - count_inside(x, y, w, h, &halves, reach));
            for c in 0..4 {
                row[x * 4 + c] = (sum[c] / total.max(1e-9)) as f32;
            }
        }
    });
    out
}

/// Nombre de pixels du disque centré en `(x, y)` qui tombent dans l'image.
fn count_inside(x: usize, y: usize, w: usize, h: usize, halves: &[isize], reach: isize) -> f64 {
    let mut count = 0.0;
    for (k, half) in halves.iter().enumerate() {
        let sy = y as isize + k as isize - reach;
        if sy >= 0 && (sy as usize) < h {
            count += ((x as isize + half + 1).clamp(0, w as isize) - (x as isize - half).clamp(0, w as isize)) as f64;
        }
    }
    count
}

/// Valeur pseudo-aléatoire déterministe dans `[0, 1)` : le même filtre redonne le même grain.
fn hash(x: usize, y: usize, salt: u32) -> f32 {
    let mut v = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77) ^ salt.wrapping_mul(0xC2B2_AE3D);
    v ^= v >> 15;
    v = v.wrapping_mul(0x2C1B_3C6D);
    v ^= v >> 12;
    (v >> 8) as f32 / (1u32 << 24) as f32
}

/// Bruit à répartition en cloche (somme de deux tirages), proportionnel à l'alpha : le transparent reste transparent.
fn noise(source: &[f32], w: usize, amount: f32, monochrome: bool) -> Vec<u8> {
    let mut out = vec![0u8; source.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let px = &source[(y * w + x) * 4..(y * w + x) * 4 + 4];
            let alpha = px[3];
            let grain = |salt: u32| (hash(x, y, salt) + hash(x, y, salt + 7) - 1.0) * amount * alpha;
            let shared = grain(1);
            for c in 0..3 {
                let delta = if monochrome { shared } else { grain(c as u32 + 1) };
                row[x * 4 + c] = ((px[c] + delta).clamp(0.0, alpha) + 0.5) as u8;
            }
            row[x * 4 + 3] = (alpha + 0.5) as u8;
        }
    });
    out
}

const BAYER: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// Tramage ordonné : l'image est ramenée à quelques tons, le seuil variant selon une matrice de Bayer.
fn dither(source: &[f32], w: usize, scale: usize, levels: u32, monochrome: bool) -> Vec<u8> {
    let steps = (levels - 1) as f32;
    let mut out = vec![0u8; source.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            // Un « point » de tramage couvre `scale` pixels : on lit la couleur en son coin.
            let (bx, by) = (x / scale, y / scale);
            let i = ((by * scale).min(y) * w + (bx * scale).min(x)) * 4;
            let alpha = source[i + 3];
            let here = source[(y * w + x) * 4 + 3];
            if alpha <= 0.0 || here <= 0.0 {
                continue;
            }
            let threshold = (f32::from(BAYER[by % 8][bx % 8]) + 0.5) / 64.0 - 0.5;
            let straight = [source[i] / alpha, source[i + 1] / alpha, source[i + 2] / alpha];
            let tone = |v: f32| ((v * steps + threshold).round() / steps).clamp(0.0, 1.0);
            let result = if monochrome { [tone(0.299 * straight[0] + 0.587 * straight[1] + 0.114 * straight[2]); 3] } else { straight.map(tone) };
            for c in 0..3 {
                row[x * 4 + c] = (result[c] * here + 0.5) as u8;
            }
            row[x * 4 + 3] = (here + 0.5) as u8;
        }
    });
    out
}

/// Relief du filtre Verre.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GlassTexture {
    /// Verre dépoli : relief irrégulier et doux.
    #[default]
    Frosted,
    /// Pavés de verre carrés, bombés.
    Blocks,
    /// Trame de toile.
    Canvas,
    /// Petites lentilles rondes serrées.
    TinyLens,
    /// Image chargée par l'utilisateur ; sa clarté sert de relief, répétée pour couvrir l'image.
    Custom,
}

/// Carte de relief chargée par l'utilisateur : clarté de 0 à 1, ligne par ligne.
#[derive(Debug)]
pub struct HeightMap {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f32>,
}

impl HeightMap {
    /// Depuis une image quelconque (PNG, JPEG, WebP…) : sa luminance devient le relief.
    pub fn from_image(bytes: &[u8]) -> Result<Self, EngineError> {
        let image = image::load_from_memory(bytes).map_err(|e| EngineError::Corrupt(format!("texture illisible : {e}")))?;
        // Une texture démesurée n'apporte rien : elle est ramenée à 2048 px de côté au plus.
        let image = if image.width().max(image.height()) > 2048 { image.thumbnail(2048, 2048) } else { image }.into_luma8();
        let (width, height) = (image.width() as usize, image.height() as usize);
        if width < 2 || height < 2 {
            return Err(EngineError::InvalidOperation("texture trop petite".into()));
        }
        Ok(Self { width, height, values: image.into_raw().into_iter().map(|v| f32::from(v) / 255.0).collect() })
    }

    /// Relief en `(x, y)`, la texture étant répétée à l'infini, avec interpolation.
    fn at(&self, x: f32, y: f32) -> f32 {
        let (fx, fy) = (x.rem_euclid(self.width as f32), y.rem_euclid(self.height as f32));
        let (x0, y0) = (fx as usize % self.width, fy as usize % self.height);
        let (x1, y1) = ((x0 + 1) % self.width, (y0 + 1) % self.height);
        let (tx, ty) = (fx.fract(), fy.fract());
        let v = |x: usize, y: usize| self.values[y * self.width + x];
        (v(x0, y0) * (1.0 - tx) + v(x1, y0) * tx) * (1.0 - ty) + (v(x0, y1) * (1.0 - tx) + v(x1, y1) * tx) * ty
    }
}

/// Texture chargée pour le filtre Verre, commune à l'application (comme dans Photoshop, on en charge une à la fois).
static GLASS_TEXTURE: std::sync::RwLock<Option<std::sync::Arc<HeightMap>>> = std::sync::RwLock::new(None);

pub fn set_glass_texture(map: Option<HeightMap>) {
    *GLASS_TEXTURE.write().unwrap_or_else(std::sync::PoisonError::into_inner) = map.map(std::sync::Arc::new);
}

pub fn glass_texture() -> Option<std::sync::Arc<HeightMap>> {
    GLASS_TEXTURE.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// Bruit lissé : interpolation douce entre les nœuds d'une grille de pas `cell`.
fn value_noise(x: f32, y: f32, cell: f32, salt: u32) -> f32 {
    let (gx, gy) = (x / cell, y / cell);
    let (x0, y0) = (gx.floor(), gy.floor());
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, ty) = (ease(gx - x0), ease(gy - y0));
    let at = |dx: f32, dy: f32| hash((x0 + dx + 4096.0) as usize, (y0 + dy + 4096.0) as usize, salt);
    (at(0.0, 0.0) * (1.0 - tx) + at(1.0, 0.0) * tx) * (1.0 - ty) + (at(0.0, 1.0) * (1.0 - tx) + at(1.0, 1.0) * tx) * ty
}

/// Relief d'une texture intégrée en `(x, y)`, coordonnées déjà divisées par l'échelle.
fn glass_height(texture: GlassTexture, x: f32, y: f32) -> f32 {
    match texture {
        GlassTexture::Frosted => value_noise(x, y, 9.0, 1) * 0.6 + value_noise(x, y, 3.5, 2) * 0.4,
        GlassTexture::Blocks => {
            // Pavé de 48 px : un dôme aplati, avec un joint en creux.
            let (u, v) = ((x.rem_euclid(48.0) / 48.0) * 2.0 - 1.0, (y.rem_euclid(48.0) / 48.0) * 2.0 - 1.0);
            let edge = u.abs().max(v.abs());
            (1.0 - edge.powi(4)) * (1.0 - 0.35 * (u * u + v * v))
        }
        GlassTexture::Canvas => {
            // Fils de chaîne et de trame qui passent alternativement dessus et dessous.
            let (a, b) = ((x * std::f32::consts::PI / 4.0).sin(), (y * std::f32::consts::PI / 4.0).sin());
            let over = ((x / 8.0).floor() + (y / 8.0).floor()).rem_euclid(2.0) < 1.0;
            0.5 + 0.25 * if over { a.abs() - 0.5 * b.abs() } else { b.abs() - 0.5 * a.abs() }
        }
        GlassTexture::TinyLens => {
            // Lentilles de 14 px en quinconce.
            let row = (y / 12.0).floor();
            let sx = x + if row.rem_euclid(2.0) < 1.0 { 0.0 } else { 7.0 };
            let (u, v) = ((sx.rem_euclid(14.0) - 7.0) / 7.0, (y.rem_euclid(12.0) - 6.0) / 6.0);
            (1.0 - (u * u + v * v)).max(0.0).sqrt()
        }
        GlassTexture::Custom => 0.5,
    }
}

/// Filtre Verre : relief -> pente -> déviation du regard.
#[allow(clippy::too_many_arguments)]
fn glass(source: &[f32], w: usize, h: usize, distortion: f32, smoothness: f32, texture: GlassTexture, custom: Option<&HeightMap>, scale: f32, invert: bool) -> Vec<f32> {
    // 1. Relief, à l'échelle demandée, puis adouci : sans cela chaque grain du relief donnerait un éclat dur.
    let mut relief = vec![0.0f32; w * h * 4];
    relief.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let (tx, ty) = (x as f32 / scale, y as f32 / scale);
            let height = custom.map_or_else(|| glass_height(texture, tx, ty), |map| map.at(tx, ty));
            row[x * 4] = if invert { 1.0 - height } else { height };
        }
    });
    let relief = gaussian(&relief, w, h, smoothness * 0.6);
    let height_at = |x: usize, y: usize| relief[(y.min(h - 1) * w + x.min(w - 1)) * 4];

    // 2. La pente du relief dévie le regard : on lit l'image un peu plus loin dans le sens de la pente.
    let strength = distortion * 14.0;
    let sample = |x: f32, y: f32, c: usize| -> f32 {
        let (fx, fy) = (x.clamp(0.0, w as f32 - 1.0), y.clamp(0.0, h as f32 - 1.0));
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |ix: usize, iy: usize| source[(iy * w + ix) * 4 + c];
        (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty
    };
    let mut out = vec![0.0f32; source.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let dx = (height_at(x + 1, y) - height_at(x.saturating_sub(1), y)) * strength;
            let dy = (height_at(x, y + 1) - height_at(x, y.saturating_sub(1))) * strength;
            for c in 0..4 {
                row[x * 4 + c] = sample(x as f32 + dx, y as f32 + dy, c);
            }
        }
    });
    out
}

/// Texture, clarté et correction du voile, sur un tampon prémultiplié 0..255.
fn presence(source: &[f32], w: usize, h: usize, texture: f32, clarity: f32, dehaze: f32) -> Vec<f32> {
    let mut out = source.to_vec();
    let side = w.min(h) as f32;

    // Correction du voile (He et al., « dark channel prior », version allégée). Le voile est une
    // lumière ajoutée, d'autant plus forte que la scène est lointaine : on estime sa couleur, puis la
    // part de voile en chaque point d'après le canal le plus sombre du voisinage, et on la retire.
    if dehaze.abs() > 1e-3 {
        let dark: Vec<f32> = source.chunks_exact(4).flat_map(|px| [px[0].min(px[1]).min(px[2]), 0.0, 0.0, px[3]]).collect();
        let local = gaussian(&dark, w, h, (side / 40.0).clamp(2.0, 60.0));
        // Couleur du voile : moyenne des pixels dont le canal sombre est le plus élevé (le dixième supérieur).
        let mut levels: Vec<f32> = local.chunks_exact(4).map(|px| px[0]).collect();
        levels.sort_unstable_by(f32::total_cmp);
        let threshold = levels[levels.len() * 9 / 10];
        let (mut haze, mut count) = ([0.0f32; 3], 0.0f32);
        for (px, level) in source.chunks_exact(4).zip(local.chunks_exact(4)) {
            if level[0] >= threshold && px[3] > 0.0 {
                for c in 0..3 {
                    haze[c] += px[c] / px[3] * 255.0;
                }
                count += 1.0;
            }
        }
        let haze = haze.map(|v| (v / count.max(1.0)).clamp(30.0, 255.0));
        out.par_chunks_mut(4).zip(local.par_chunks(4)).for_each(|(px, level)| {
            let alpha = px[3];
            if alpha <= 0.0 {
                return;
            }
            let min_haze = haze[0].min(haze[1]).min(haze[2]);
            // Part de lumière transmise : 1 = pas de voile. Un réglage négatif en ajoute au lieu d'en retirer.
            let transmission = (1.0 - dehaze * 0.85 * (level[0] / level[3].max(1.0) * 255.0) / min_haze).clamp(0.12, 3.0);
            for c in 0..3 {
                let straight = px[c] / alpha * 255.0;
                px[c] = (((straight - haze[c]) / transmission + haze[c]).clamp(0.0, 255.0)) * alpha / 255.0;
            }
        });
    }

    // Texture et clarté : contraste local = écart entre l'image et sa version floutée, réinjecté.
    for (amount, sigma, midtones) in [(texture * 0.9, (side / 400.0).clamp(1.2, 6.0), false), (clarity * 1.0, (side / 28.0).clamp(6.0, 120.0), true)] {
        if amount.abs() <= 1e-3 {
            continue;
        }
        let base = out.clone();
        let blurred = gaussian(&base, w, h, sigma);
        out.par_chunks_mut(4).zip(base.par_chunks(4)).zip(blurred.par_chunks(4)).for_each(|((px, original), soft)| {
            let alpha = original[3];
            if alpha <= 0.0 {
                return;
            }
            // La clarté épargne les tons très sombres et très clairs, où elle ferait des halos et boucherait les noirs.
            let luma = (0.2126 * original[0] + 0.7152 * original[1] + 0.0722 * original[2]) / alpha;
            let weight = if midtones { (1.0 - (2.0 * luma - 1.0).powi(2)).max(0.0) } else { 1.0 };
            for c in 0..3 {
                px[c] = (original[c] + (original[c] - soft[c] * alpha / soft[3].max(1.0)) * amount * weight).clamp(0.0, alpha);
            }
        });
    }
    out
}

/// Tirage selon une loi normale centrée réduite (Box-Muller), déterministe pour un pixel et un sel donnés.
fn normal(x: usize, y: usize, salt: u32) -> f32 {
    let (u, v) = (hash(x, y, salt).max(1e-7), hash(x, y, salt.wrapping_add(101)));
    (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
}

/// Dissolution puis grain, tous deux gaussiens. `source` est l'image déjà floutée.
#[allow(clippy::too_many_arguments)]
fn gaussian_noise(source: &[f32], w: usize, h: usize, margin: usize, scatter: f32, amount: f32, grain: usize, monochrome: bool) -> Vec<u8> {
    // Les cases du grain sont calées sur le contenu, pas sur le tampon (qui a une marge variable) :
    // le grain ne glisse pas quand on change le flou.
    let shift = grain * margin.div_ceil(grain) - margin;
    let mut out = vec![0u8; source.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            // Dissolution : la couleur vient d'un voisin tiré au sort, d'autant plus loin que `scatter` est grand.
            let (sx, sy) = if scatter > 0.0 {
                let (dx, dy) = (normal(x, y, 11) * scatter, normal(x, y, 23) * scatter);
                (((x as f32 + dx).round() as isize).clamp(0, w as isize - 1) as usize, ((y as f32 + dy).round() as isize).clamp(0, h as isize - 1) as usize)
            } else {
                (x, y)
            };
            let px = &source[(sy * w + sx) * 4..(sy * w + sx) * 4 + 4];
            // Après un flou, l'alpha peut valoir -0,0000001 par erreur d'arrondi : borné, sinon `clamp` refuserait l'intervalle.
            let alpha = px[3].clamp(0.0, 255.0);
            // Grain : un même tirage pour tous les pixels d'une case de `grain` pixels de côté.
            let (gx, gy) = ((x + shift) / grain, (y + shift) / grain);
            let shared = normal(gx, gy, 37);
            for c in 0..3 {
                let noise = if monochrome { shared } else { normal(gx, gy, 37 + c as u32 * 7) };
                // Écart-type de 64 niveaux à 100 %, proportionnel à l'alpha : le transparent reste transparent.
                row[x * 4 + c] = ((px[c] + noise * amount * 64.0 * alpha / 255.0).clamp(0.0, alpha) + 0.5) as u8;
            }
            row[x * 4 + 3] = (alpha + 0.5) as u8;
        }
    });
    out
}

/// Seuil de la matrice de Bayer 16 x 16 en `(x, y)`, dans `]0, 1[` : construit par entrelacement
/// des bits de `x XOR y` et de `y`, du plus significatif au moins significatif.
fn bayer16(x: usize, y: usize) -> f32 {
    let (x, y) = (x & 15, y & 15);
    let mixed = x ^ y;
    let mut value = 0usize;
    for bit in (0..4).rev() {
        value = (value << 2) | (((mixed >> bit) & 1) << 1) | ((y >> bit) & 1);
    }
    // L'ordre des bits est inversé pour que les seuils voisins soient bien dispersés.
    let mut reversed = 0usize;
    for pair in 0..4 {
        reversed |= ((value >> (pair * 2)) & 3) << ((3 - pair) * 2);
    }
    (reversed as f32 + 0.5) / 256.0
}

/// Tramage couleur : un point s'allume quand la luminosité de la cellule dépasse le seuil de la
/// matrice. Allumé, il prend la teinte de la cellule portée à pleine luminosité : comme la part
/// de points allumés vaut la luminosité, l'image garde à l'œil ses couleurs et ses valeurs.
fn color_dither(source: &[f32], w: usize, scale: usize, contrast: f32, threshold: f32, strength: f32) -> Vec<u8> {
    let mut out = vec![0u8; source.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let here = (y * w + x) * 4;
            let alpha_here = source[here + 3];
            if alpha_here <= 0.0 {
                continue;
            }
            let (bx, by) = (x / scale, y / scale);
            let cell = (by * scale * w + bx * scale) * 4;
            let alpha = source[cell + 3];
            let straight = if alpha > 0.0 { [source[cell] / alpha, source[cell + 1] / alpha, source[cell + 2] / alpha] } else { [0.0; 3] };
            let luma = 0.299 * straight[0] + 0.587 * straight[1] + 0.114 * straight[2];
            let level = (luma - 0.5) * contrast + 0.5 + (0.5 - threshold);
            let peak = straight[0].max(straight[1]).max(straight[2]).max(1e-3);
            let lit = level > bayer16(bx, by);
            for c in 0..3 {
                let dot = if lit { straight[c] / peak } else { 0.0 };
                let original = source[here + c] / alpha_here;
                row[x * 4 + c] = ((original + (dot - original) * strength) * alpha_here + 0.5).clamp(0.0, alpha_here) as u8;
            }
            row[x * 4 + 3] = (alpha_here + 0.5) as u8;
        }
    });
    out
}

/// Aberration chromatique : le rouge est lu d'un côté, le bleu de l'autre, le vert reste en place.
fn aberration(source: &[f32], w: usize, h: usize, amount: f32, angle: f32, radial: bool) -> Vec<f32> {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let reach = cx.hypot(cy).max(1.0);
    let direction = (angle.to_radians().cos(), -angle.to_radians().sin());
    let sample = |x: f32, y: f32, c: usize| -> f32 {
        let (fx, fy) = ((x - 0.5).clamp(0.0, w as f32 - 1.0), (y - 0.5).clamp(0.0, h as f32 - 1.0));
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |ix: usize, iy: usize| source[(iy * w + ix) * 4 + c];
        (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty
    };
    let mut out = vec![0.0f32; source.len()];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // Radial : le décalage est nul au centre et vaut `amount` dans les angles.
            let (ox, oy) = if radial { ((px - cx) / reach * amount, (py - cy) / reach * amount) } else { (direction.0 * amount, direction.1 * amount) };
            let i = x * 4;
            row[i] = sample(px - ox, py - oy, 0);
            row[i + 1] = source[(y * w + x) * 4 + 1];
            row[i + 2] = sample(px + ox, py + oy, 2);
            // Alpha : le plus grand des trois, et aucune couleur prémultipliée ne le dépasse.
            let alpha = source[(y * w + x) * 4 + 3].max(sample(px - ox, py - oy, 3)).max(sample(px + ox, py + oy, 3));
            row[i + 3] = alpha;
        }
    });
    out
}

/// Du plus clair au plus dense.
const ASCII_RAMP: &str = " .,:;i1tfLCG08@";

/// Image recomposée en caractères à chasse fixe, sur fond noir : chaque cellule prend le
/// caractère dont la densité correspond à sa luminosité.
fn ascii(source: &[f32], w: usize, h: usize, cell: usize, color: bool) -> Result<Vec<u8>, EngineError> {
    let (cell_w, cell_h) = ((cell as f32 * 0.6).round().max(3.0) as usize, cell);
    // Chaque caractère de la rampe est dessiné une seule fois par le moteur de texte.
    let glyphs: Vec<Vec<f32>> = ASCII_RAMP
        .chars()
        .map(|ch| {
            let data = crate::text::TextData { content: ch.to_string(), font_family: "monospace".into(), size: cell as f32, line_height: 1.0, color: [255; 3], ..Default::default() };
            let raster = crate::text::render(&data)?.to_premultiplied(cell_w as u32, cell_h as u32);
            Ok(raster.chunks_exact(4).map(|px| f32::from(px[3]) / 255.0).collect())
        })
        .collect::<Result<_, EngineError>>()?;
    let mut out = vec![0u8; source.len()];
    out.par_chunks_mut(w * 4 * cell_h).enumerate().for_each(|(row_index, band)| {
        let y0 = row_index * cell_h;
        let rows = band.len() / (w * 4);
        for x0 in (0..w).step_by(cell_w) {
            let columns = cell_w.min(w - x0);
            // Couleur et couverture moyennes de la cellule.
            let mut sum = [0.0f32; 4];
            for y in y0..(y0 + rows).min(h) {
                for x in x0..x0 + columns {
                    for c in 0..4 {
                        sum[c] += source[(y * w + x) * 4 + c];
                    }
                }
            }
            if sum[3] <= 0.0 {
                continue;
            }
            let straight = [sum[0] / sum[3], sum[1] / sum[3], sum[2] / sum[3]];
            let luma = 0.299 * straight[0] + 0.587 * straight[1] + 0.114 * straight[2];
            let glyph = &glyphs[((luma * (glyphs.len() - 1) as f32).round() as usize).min(glyphs.len() - 1)];
            // En couleur, le caractère porte la teinte de la cellule à pleine luminosité : c'est sa densité qui rend la valeur.
            let peak = straight[0].max(straight[1]).max(straight[2]).max(1e-3);
            let ink = if color { straight.map(|v| v / peak) } else { [1.0; 3] };
            for y in 0..rows {
                for x in 0..columns {
                    let here = source[((y0 + y) * w + x0 + x) * 4 + 3];
                    let coverage = glyph[y * cell_w + x];
                    let at = (y * w + x0 + x) * 4;
                    for c in 0..3 {
                        band[at + c] = (ink[c] * coverage * here + 0.5) as u8;
                    }
                    band[at + 3] = (here + 0.5) as u8;
                }
            }
        }
    });
    Ok(out)
}

/// Flou gaussien approché par trois moyennes glissantes successives (écart imperceptible,
/// coût indépendant du rayon), en deux passes : lignes puis colonnes.
pub(crate) fn gaussian(source: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let mut out = vec![0.0; source.len()];
    let mut scratch = vec![0.0; source.len()];
    gaussian_into(source, w, h, sigma, &mut out, &mut scratch);
    out
}

/// Même flou, mais dans des tampons fournis par l'appelant : **deux** tampons en tout, réutilisables
/// d'un appel à l'autre. Le flou par pastilles en enchaîne sept ; en laissant `gaussian` allouer
/// ses quatre tampons pleine taille à chaque fois, une image de 12 mégapixels demandait 1,5 Go.
/// Le résultat est dans `out` ; `scratch` sert de brouillon (son contenu final n'a pas de sens).
pub(crate) fn gaussian_into(source: &[f32], w: usize, h: usize, sigma: f32, out: &mut [f32], scratch: &mut [f32]) {
    debug_assert!(out.len() == source.len() && scratch.len() == source.len());
    let sizes = box_sizes(sigma);
    // Lignes : source -> out -> brouillon -> out.
    blur_lines(source, out, w, sizes[0]);
    blur_lines(out, scratch, w, sizes[1]);
    blur_lines(scratch, out, w, sizes[2]);
    // Colonnes : on transpose, on refait la même passe, puis on transpose en retour.
    transpose_into(out, w, h, scratch);
    blur_lines(scratch, out, h, sizes[0]);
    blur_lines(out, scratch, h, sizes[1]);
    blur_lines(scratch, out, h, sizes[2]);
    transpose_into(out, h, w, scratch);
    out.copy_from_slice(scratch);
}

/// Largeurs (impaires) des trois moyennes glissantes équivalentes à un flou gaussien d'écart-type `sigma`.
fn box_sizes(sigma: f32) -> [usize; 3] {
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let mut lower = ideal.floor() as usize;
    if lower % 2 == 0 {
        lower = lower.saturating_sub(1).max(1);
    }
    let upper = lower + 2;
    let switch = ((12.0 * sigma * sigma - (3 * lower * lower + 12 * lower + 9) as f32) / (-4.0 * lower as f32 - 4.0)).round() as i32;
    [0, 1, 2].map(|i| if (i as i32) < switch { lower } else { upper })
}

/// Moyenne glissante de largeur `size` sur chaque ligne de `line` pixels RGBA ; hors image = transparent.
fn blur_lines(source: &[f32], target: &mut [f32], line: usize, size: usize) {
    let half = (size / 2) as isize;
    let scale = 1.0 / size as f32;
    target.par_chunks_mut(line * 4).zip(source.par_chunks(line * 4)).for_each(|(out, row)| {
        let at = |i: isize, c: usize| if i < 0 || i >= line as isize { 0.0 } else { row[i as usize * 4 + c] };
        for c in 0..4 {
            let mut sum: f32 = (-half..=half).map(|i| at(i, c)).sum();
            for x in 0..line as isize {
                out[x as usize * 4 + c] = sum * scale;
                sum += at(x + half + 1, c) - at(x - half, c);
            }
        }
    });
}

fn transpose_into(source: &[f32], w: usize, h: usize, out: &mut [f32]) {
    out.par_chunks_mut(h * 4).enumerate().for_each(|(x, column)| {
        for y in 0..h {
            column[y * 4..y * 4 + 4].copy_from_slice(&source[(y * w + x) * 4..(y * w + x) * 4 + 4]);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::Sampler;

    fn square() -> TileStore {
        let mut rgba = vec![0u8; 300 * 300 * 4];
        for y in 100..200 {
            for x in 100..200 {
                rgba[(y * 300 + x) * 4..(y * 300 + x) * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        TileStore::from_premultiplied(300, 300, &rgba).expect("tampon")
    }

    #[test]
    fn blur_softens_edges_spreads_outwards_and_preserves_the_total() {
        let source = square();
        let blurred = apply(&source, &Filter::GaussianBlur { radius: 6.0 }).expect("flou");
        let mut sampler = Sampler::new(&blurred);
        assert_eq!(sampler.get(150, 150), [255, 0, 0, 255], "loin des bords : inchangé");
        let edge = sampler.get(100, 150)[3];
        assert!((118..=138).contains(&edge), "sur le bord : à moitié couvert, {edge}");
        assert!(sampler.get(94, 150)[3] > 20 && sampler.get(75, 150)[3] == 0, "le flou déborde d'environ 3 sigma");
        // Un flou redistribue la matière sans en créer ni en perdre.
        let total = |store: &TileStore| store.translated(50, 50).to_premultiplied(400, 400).chunks_exact(4).map(|px| u64::from(px[3])).sum::<u64>();
        let (before, after) = (total(&source), total(&blurred));
        assert!(before.abs_diff(after) * 200 < before, "{before} -> {after}");
        assert_eq!(apply(&TileStore::new(), &Filter::GaussianBlur { radius: 5.0 }).expect("vide").tile_count(), 0);
    }

    #[test]
    fn sharpen_increases_local_contrast_without_touching_flat_areas_or_alpha() {
        // Deux aplats gris côte à côte : l'accentuation creuse le côté sombre et éclaircit le côté clair, près de la frontière.
        let mut rgba = vec![0u8; 200 * 50 * 4];
        for (index, px) in rgba.chunks_exact_mut(4).enumerate() {
            let v = if index % 200 < 100 { 100 } else { 160 };
            px.copy_from_slice(&[v, v, v, 255]);
        }
        let source = TileStore::from_premultiplied(200, 50, &rgba).expect("tampon");
        let sharp = apply(&source, &Filter::Sharpen { radius: 3.0, amount: 150.0 }).expect("accentuation");
        let mut sampler = Sampler::new(&sharp);
        assert!(sampler.get(98, 25)[0] < 95 && sampler.get(101, 25)[0] > 165, "{:?} {:?}", sampler.get(98, 25), sampler.get(101, 25));
        assert_eq!((sampler.get(50, 25), sampler.get(150, 25)), ([100, 100, 100, 255], [160, 160, 160, 255]), "aplats inchangés");
    }

    #[test]
    fn motion_blur_smears_along_its_angle_only() {
        let horizontal = apply(&square(), &Filter::MotionBlur { angle: 0.0, distance: 40.0 }).expect("flou directionnel");
        let mut sampler = Sampler::new(&horizontal);
        assert!(sampler.get(90, 150)[3] > 40 && sampler.get(210, 150)[3] > 0, "étalé à gauche et à droite");
        assert_eq!((sampler.get(150, 95)[3], sampler.get(150, 100)[3]), (0, 255), "bords haut et bas intacts");
        let vertical = apply(&square(), &Filter::MotionBlur { angle: 90.0, distance: 40.0 }).expect("flou vertical");
        assert!(Sampler::new(&vertical).get(150, 90)[3] > 40 && Sampler::new(&vertical).get(95, 150)[3] == 0);
    }

    #[test]
    fn lens_blur_is_a_flat_disc_and_highlights_bloom() {
        // Un point blanc sur gris sombre : le flou d'objectif en fait une pastille à bord net, pas une cloche.
        let mut rgba = vec![0u8; 200 * 200 * 4];
        for px in rgba.chunks_exact_mut(4) {
            px.copy_from_slice(&[40, 40, 40, 255]);
        }
        for y in 98..102 {
            for x in 98..102 {
                rgba[(y * 200 + x) * 4..(y * 200 + x) * 4 + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let source = TileStore::from_premultiplied(200, 200, &rgba).expect("tampon");
        let at = |store: &TileStore, x| Sampler::new(store).get(x, 100)[0];
        let plain = apply(&source, &Filter::LensBlur { radius: 20.0, highlights: 0.0 }).expect("flou d'objectif");
        let bloom = apply(&source, &Filter::LensBlur { radius: 20.0, highlights: 100.0 }).expect("bokeh");
        assert!(at(&plain, 100).abs_diff(at(&plain, 112)) <= 1, "disque plat : même valeur au centre et à mi-rayon");
        assert_eq!(at(&plain, 130), 40, "rien au-delà du rayon");
        assert!(at(&bloom, 110) > at(&plain, 110) + 10, "hautes lumières renforcées : {} contre {}", at(&bloom, 110), at(&plain, 110));
        assert!(at(&bloom, 117) > 45 && at(&bloom, 125) == 40, "pastille à bord franc");
    }

    #[test]
    fn noise_dither_and_ascii_keep_the_alpha_and_do_their_job() {
        let source = square();
        let alpha_at = |store: &TileStore, x, y| Sampler::new(store).get(x, y)[3];

        let noisy = apply(&source, &Filter::Noise { amount: 40.0, monochrome: true }).expect("bruit");
        let values: std::collections::BTreeSet<u8> = (100..200).map(|x| Sampler::new(&noisy).get(x, 150)[0]).collect();
        assert!(values.len() > 10, "le grain varie d'un pixel à l'autre");
        assert_eq!((alpha_at(&noisy, 150, 150), alpha_at(&noisy, 50, 50)), (255, 0));
        let again = apply(&source, &Filter::Noise { amount: 40.0, monochrome: true }).expect("bruit");
        assert_eq!(again.to_premultiplied(300, 300), noisy.to_premultiplied(300, 300), "déterministe");

        // Gris à 50 % tramé en noir et blanc : autant de pixels noirs que de blancs, rien entre les deux.
        let mut gray = vec![0u8; 64 * 64 * 4];
        gray.chunks_exact_mut(4).for_each(|px| px.copy_from_slice(&[128, 128, 128, 255]));
        let gray = TileStore::from_premultiplied(64, 64, &gray).expect("tampon");
        let dithered = apply(&gray, &Filter::Dither { scale: 1, levels: 2, monochrome: true }).expect("tramage").to_premultiplied(64, 64);
        let white = dithered.chunks_exact(4).filter(|px| px[0] == 255).count();
        assert!(dithered.chunks_exact(4).all(|px| px[0] == 0 || px[0] == 255) && (1900..=2200).contains(&white), "{white} pixels blancs sur 4096");

        let text = apply(&source, &Filter::Ascii { cell: 12, color: true }).expect("ascii");
        let inside = text.translated(-100, -100).to_premultiplied(100, 100);
        let (lit, dark) = (inside.chunks_exact(4).filter(|px| px[0] > 128).count(), inside.chunks_exact(4).filter(|px| px[0] < 32).count());
        assert!(lit > 300 && dark > 3000, "des caractères clairs sur fond noir : {lit} clairs, {dark} sombres");
        assert!(inside.chunks_exact(4).all(|px| px[1] == 0 && px[2] == 0), "couleur de la cellule conservée (rouge)");
        assert_eq!((alpha_at(&text, 150, 150), alpha_at(&text, 50, 50)), (255, 0));
    }

    #[test]
    fn the_bands_of_the_field_blur_give_the_same_image_as_one_pass() {
        // Même image, calculée en une seule bande puis en bandes de 24 lignes : les pixels doivent
        // coïncider. C'est ce qui garantit que la découpe (faite pour la mémoire) ne se voit pas.
        let (w, h) = (160usize, 120usize);
        let source: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let v = if ((i % w) / 3 + (i / w) / 3) % 2 == 0 { 255.0 } else { 20.0 };
                [v, v * 0.4, 255.0 - v, 255.0]
            })
            .collect();
        let pins = [(20.0f32, 20.0f32, 30.0f32), (140.0, 100.0, 2.0)];
        let whole = field_banded(&source, w, h, &pins, h);
        let sliced = field_banded(&source, w, h, &pins, 24);
        let worst = whole.iter().zip(&sliced).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert!(worst <= 1, "écart maximal entre une seule bande et plusieurs : {worst}");
    }

    #[test]
    fn field_blur_follows_its_pins() {
        // Damier fin : net, son contraste local est maximal ; flouté, il fond vers le gris.
        let mut rgba = vec![0u8; 400 * 100 * 4];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let v = if ((i % 400) / 2 + (i / 400) / 2) % 2 == 0 { 255 } else { 0 };
            px.copy_from_slice(&[v, v, v, 255]);
        }
        let source = TileStore::from_premultiplied(400, 100, &rgba).expect("tampon");
        let contrast = |store: &TileStore, x: i32| {
            let mut sampler = Sampler::new(store);
            let values: Vec<u8> = (x - 6..x + 6).map(|x| sampler.get(x, 50)[0]).collect();
            values.iter().max().expect("max") - values.iter().min().expect("min")
        };
        let pin = |x, blur| BlurPin { x, y: 0.5, blur };

        let uniform = apply(&source, &Filter::FieldBlur { pins: vec![pin(0.5, 20.0)] }).expect("une pastille");
        assert!(contrast(&uniform, 30) < 30 && contrast(&uniform, 370) < 30, "une seule pastille : toute l'image est floue");

        // Flou léger à droite (damier de 2 px : il s'efface vite) pour pouvoir mesurer la progression.
        let gradient = apply(&source, &Filter::FieldBlur { pins: vec![pin(0.05, 0.0), pin(0.95, 5.0)] }).expect("deux pastilles");
        let (left, middle, right) = (contrast(&gradient, 25), contrast(&gradient, 200), contrast(&gradient, 360));
        assert!(left > 200 && right < 60 && middle < left - 20 && middle > right, "net à gauche, flou à droite, progressif entre les deux : {left} {middle} {right}");

        let sharp = apply(&source, &Filter::FieldBlur { pins: vec![pin(0.5, 0.0)] }).expect("pastille à zéro");
        assert_eq!(sharp.to_premultiplied(400, 100), source.to_premultiplied(400, 100), "flou nul : image inchangée");
    }

    #[test]
    fn liquify_moves_pixels_where_the_mesh_says() {
        let source = square();
        let identity = Filter::Liquify { cols: 3, rows: 3, dx: vec![0.0; 9], dy: vec![0.0; 9] };
        assert_eq!(apply(&source, &identity).expect("identité").to_premultiplied(300, 300), source.to_premultiplied(300, 300), "grille nulle : image inchangée");

        // Tout le contenu va chercher sa couleur 20 % plus à droite : l'image glisse de 20 px vers la gauche.
        let shifted = apply(&source, &Filter::Liquify { cols: 2, rows: 2, dx: vec![0.2; 4], dy: vec![0.0; 4] }).expect("décalage");
        let mut sampler = Sampler::new(&shifted);
        assert_eq!((sampler.get(85, 150)[3], sampler.get(75, 150)[3], sampler.get(185, 150)[3]), (255, 0, 0), "décalé de 20 px vers la gauche");
        assert!(apply(&source, &Filter::Liquify { cols: 3, rows: 3, dx: vec![0.0; 4], dy: vec![0.0; 9] }).is_err());
    }

    #[test]
    fn gaussian_noise_dissolves_edges_into_dust_and_adds_bell_shaped_grain() {
        // Frontière noir / blanc nette. La dissolution seule (sans flou ni grain) doit donner, près de la
        // frontière, un mélange de pixels purement noirs et purement blancs : de la poussière, pas un dégradé.
        let mut rgba = vec![0u8; 200 * 60 * 4];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let v = if i % 200 < 100 { 0 } else { 255 };
            px.copy_from_slice(&[v, v, v, 255]);
        }
        let source = TileStore::from_premultiplied(200, 60, &rgba).expect("tampon");
        let dust = apply(&source, &Filter::GaussianNoise { radius: 0.0, scatter: 12.0, amount: 0.0, grain: 1, monochrome: true }).expect("dissolution");
        let mut sampler = Sampler::new(&dust);
        let near: Vec<u8> = (10..50).flat_map(|y| (92..108).map(move |x| (x, y))).map(|(x, y)| sampler.get(x, y)[0]).collect();
        assert!(near.iter().all(|v| *v == 0 || *v == 255), "aucune valeur intermédiaire : ce n'est pas un flou lisse");
        let white = near.iter().filter(|v| **v == 255).count() as f32 / near.len() as f32;
        assert!((0.3..0.7).contains(&white), "noir et blanc mêlés à la frontière : {white}");
        assert_eq!((sampler.get(30, 30)[0], sampler.get(170, 30)[0]), (0, 255), "loin de la frontière, rien ne change");

        // Grain seul sur un gris moyen : centré sur la valeur d'origine, en cloche (68 % des pixels à moins d'un écart-type).
        let mut gray = vec![0u8; 128 * 128 * 4];
        gray.chunks_exact_mut(4).for_each(|px| px.copy_from_slice(&[128, 128, 128, 255]));
        let gray = TileStore::from_premultiplied(128, 128, &gray).expect("tampon");
        let grainy = apply(&gray, &Filter::GaussianNoise { radius: 0.0, scatter: 0.0, amount: 50.0, grain: 1, monochrome: true }).expect("grain").to_premultiplied(128, 128);
        let values: Vec<f32> = grainy.chunks_exact(4).map(|px| f32::from(px[0])).collect();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        let within = values.iter().filter(|v| (**v - 128.0).abs() <= 32.0).count() as f32 / values.len() as f32;
        assert!((mean - 128.0).abs() < 1.5 && (0.64..0.73).contains(&within), "moyenne {mean}, part à moins d'un écart-type {within}");
        assert!(grainy.chunks_exact(4).all(|px| px[0] == px[1] && px[1] == px[2] && px[3] == 255), "monochrome, alpha intact");

        // Grain de 4 px : les pixels d'une même case partagent le même tirage.
        let coarse = apply(&gray, &Filter::GaussianNoise { radius: 0.0, scatter: 0.0, amount: 50.0, grain: 4, monochrome: false }).expect("gros grain");
        let mut sampler = Sampler::new(&coarse);
        assert_eq!(sampler.get(8, 8), sampler.get(11, 11));
        assert_ne!(sampler.get(8, 8), sampler.get(12, 8));
    }

    #[test]
    fn glass_bends_the_image_along_the_relief_and_accepts_a_loaded_texture() {
        // Bandes verticales nettes (période 20 px) : droites avant, ondulées après.
        let mut rgba = vec![0u8; 256 * 256 * 4];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let v = if (i % 256) / 10 % 2 == 0 { 0 } else { 255 };
            px.copy_from_slice(&[v, v, v, 255]);
        }
        let source = TileStore::from_premultiplied(256, 256, &rgba).expect("tampon");
        let glass = |distortion: f32, texture: GlassTexture| apply(&source, &Filter::Glass { distortion, smoothness: 3.0, texture, scale: 100.0, invert: false }).expect("verre").to_premultiplied(256, 256);
        // Une colonne d'une image à bandes verticales est uniforme ; après le verre, elle ne l'est plus.
        let column_varies = |bytes: &[u8]| (20..236).filter(|y| bytes[(y * 256 + 100) * 4] != bytes[((y + 1) * 256 + 100) * 4]).count();
        assert_eq!(glass(0.0, GlassTexture::Frosted), rgba, "distorsion nulle : image inchangée");
        for texture in [GlassTexture::Frosted, GlassTexture::Blocks, GlassTexture::Canvas, GlassTexture::TinyLens] {
            let bent = glass(8.0, texture);
            assert!(column_varies(&bent) > 4, "{texture:?} : les bandes doivent onduler ({})", column_varies(&bent));
            assert!(bent.chunks_exact(4).all(|px| px[3] == 255), "{texture:?} : l'opacité n'est pas touchée");
        }
        let changed = |bytes: &[u8]| bytes.iter().zip(&rgba).filter(|(a, b)| a != b).count();
        assert!(changed(&glass(16.0, GlassTexture::Frosted)) > changed(&glass(3.0, GlassTexture::Frosted)), "plus de distorsion, plus de pixels déviés");

        // Texture chargée : sans elle, erreur claire ; avec, elle est répétée sur toute l'image.
        set_glass_texture(None);
        assert!(apply(&source, &Filter::Glass { distortion: 8.0, smoothness: 3.0, texture: GlassTexture::Custom, scale: 100.0, invert: false }).is_err());
        let mut png = std::io::Cursor::new(Vec::new());
        image::GrayImage::from_fn(32, 32, |x, y| image::Luma([if (x / 8 + y / 8) % 2 == 0 { 30 } else { 220 }])).write_to(&mut png, image::ImageFormat::Png).expect("png");
        set_glass_texture(Some(HeightMap::from_image(png.get_ref()).expect("texture")));
        let custom = glass(10.0, GlassTexture::Custom);
        assert!(column_varies(&custom) > 10, "le damier chargé déforme l'image");
        let far = &custom[(200 * 256 + 200) * 4..][..4 * 40];
        assert!(far.chunks_exact(4).any(|px| px[0] != 0 && px[0] != 255) || column_varies(&custom) > 10, "répétée jusqu'au bout de l'image");
        set_glass_texture(None);
    }

    #[test]
    fn presence_works_on_the_neighbourhood_like_lightroom() {
        // Fines rayures (période 6 px) posées sur une grande marche sombre / claire.
        let mut rgba = vec![0u8; 400 * 200 * 4];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let x = i % 400;
            let v = (if x < 200 { 90 } else { 160 }) + if (x / 3) % 2 == 0 { 10 } else { -10 };
            px.copy_from_slice(&[v as u8, v as u8, v as u8, 255]);
        }
        let source = TileStore::from_premultiplied(400, 200, &rgba).expect("tampon");
        let row = |store: &TileStore| -> Vec<f32> { (0..400).map(|x| f32::from(Sampler::new(store).get(x, 100)[0])).collect() };
        // Amplitude des rayures, loin de la marche ; et hauteur de la marche juste autour d'elle (moyennes sur une période).
        let ripple = |r: &[f32]| (60..120).map(|x| (r[x] - r[x + 3]).abs()).sum::<f32>() / 60.0;
        let step = |r: &[f32]| r[206..218].iter().sum::<f32>() / 12.0 - r[182..194].iter().sum::<f32>() / 12.0;
        let base = row(&source);

        let textured = row(&apply(&source, &Filter::Presence { texture: 100.0, clarity: 0.0, dehaze: 0.0 }).expect("texture"));
        assert!(ripple(&textured) > ripple(&base) * 1.5, "la texture renforce les détails fins : {} -> {}", ripple(&base), ripple(&textured));
        assert!((step(&textured) - step(&base)).abs() < 8.0, "sans toucher aux grandes masses");

        let clear = row(&apply(&source, &Filter::Presence { texture: 0.0, clarity: 100.0, dehaze: 0.0 }).expect("clarté"));
        assert!(step(&clear) > step(&base) + 6.0, "la clarté creuse le contraste local à grande échelle : {} -> {}", step(&base), step(&clear));
        let softened = row(&apply(&source, &Filter::Presence { texture: -100.0, clarity: 0.0, dehaze: 0.0 }).expect("adoucir"));
        assert!(ripple(&softened) < ripple(&base) * 0.6, "texture négative : lissage des détails");

        // Voile : image délavée (tout entre 120 et 220). Le corriger étend le contraste vers le bas.
        let mut hazy = vec![0u8; 200 * 200 * 4];
        for (i, px) in hazy.chunks_exact_mut(4).enumerate() {
            let v = 120 + ((i % 200) / 2) as u8;
            px.copy_from_slice(&[v, v, v.saturating_add(10), 255]);
        }
        let hazy = TileStore::from_premultiplied(200, 200, &hazy).expect("tampon");
        let cleared = apply(&hazy, &Filter::Presence { texture: 0.0, clarity: 0.0, dehaze: 100.0 }).expect("voile");
        let (before, after) = (Sampler::new(&hazy).get(10, 100)[0], Sampler::new(&cleared).get(10, 100)[0]);
        assert!(after + 40 < before, "les zones sombres voilées retrouvent de la profondeur : {before} -> {after}");
        let neutral = apply(&source, &Filter::Presence { texture: 0.0, clarity: 0.0, dehaze: 0.0 }).expect("neutre");
        assert_eq!(neutral.to_premultiplied(400, 200), rgba, "réglages neutres : image inchangée");
    }

    #[test]
    fn bayer_16_is_a_well_spread_permutation() {
        let mut seen = std::collections::BTreeSet::new();
        for y in 0..16 {
            for x in 0..16 {
                seen.insert((bayer16(x, y) * 256.0) as u32);
            }
        }
        assert_eq!(seen.len(), 256, "chaque seuil apparaît une fois et une seule");
        // Bien dispersée : dans tout bloc de 2 x 2, les seuils couvrent les quatre quarts de l'échelle.
        let quarters: std::collections::BTreeSet<u32> = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|(x, y)| (bayer16(*x, *y) * 4.0) as u32).collect();
        assert_eq!(quarters.len(), 4);
    }

    #[test]
    fn color_dither_keeps_hue_and_perceived_brightness() {
        // Aplat orange sombre : une partie des points s'allume, en orange vif ; le reste est noir.
        let mut rgba = vec![0u8; 64 * 64 * 4];
        rgba.chunks_exact_mut(4).for_each(|px| px.copy_from_slice(&[120, 60, 0, 255]));
        let source = TileStore::from_premultiplied(64, 64, &rgba).expect("tampon");
        let out = apply(&source, &Filter::ColorDither { scale: 1, contrast: 100.0, threshold: 50.0, strength: 100.0 }).expect("tramage").to_premultiplied(64, 64);
        let lit: Vec<&[u8]> = out.chunks_exact(4).filter(|px| px[0] > 0).collect();
        assert!(out.chunks_exact(4).all(|px| (px[0] == 0 && px[1] == 0) || (px[0] == 255 && px[1].abs_diff(128) <= 2)), "noir, ou la teinte d'origine à pleine luminosité");
        let share = lit.len() as f32 / 4096.0;
        assert!((share - 0.276).abs() < 0.03, "part de points allumés = luminosité de l'aplat : {share}");
        // Seuil plus haut : moins de points. Force à 0 : image inchangée.
        let fewer = apply(&source, &Filter::ColorDither { scale: 1, contrast: 100.0, threshold: 70.0, strength: 100.0 }).expect("tramage").to_premultiplied(64, 64);
        assert!(fewer.chunks_exact(4).filter(|px| px[0] > 0).count() < lit.len());
        let untouched = apply(&source, &Filter::ColorDither { scale: 2, contrast: 100.0, threshold: 50.0, strength: 0.0 }).expect("tramage");
        assert_eq!(untouched.to_premultiplied(64, 64), rgba);
    }

    #[test]
    fn chromatic_aberration_splits_red_and_blue_apart() {
        // Trait blanc vertical sur fond noir, décalage horizontal de 6 px : rouge d'un côté, bleu de l'autre, vert en place.
        let mut rgba = vec![0u8; 100 * 40 * 4];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let v = if (48..52).contains(&(i % 100)) { 255 } else { 0 };
            px.copy_from_slice(&[v, v, v, 255]);
        }
        let source = TileStore::from_premultiplied(100, 40, &rgba).expect("tampon");
        let out = apply(&source, &Filter::ChromaticAberration { amount: 6.0, angle: 0.0, radial: false }).expect("aberration");
        let mut sampler = Sampler::new(&out);
        assert_eq!(sampler.get(56, 20), [255, 0, 0, 255], "le rouge part à droite");
        assert_eq!(sampler.get(44, 20), [0, 0, 255, 255], "le bleu part à gauche");
        assert_eq!(sampler.get(50, 20), [0, 255, 0, 255], "le vert reste en place");
        // Radial : rien ne bouge au centre.
        let radial = apply(&source, &Filter::ChromaticAberration { amount: 20.0, angle: 0.0, radial: true }).expect("aberration");
        assert_eq!(Sampler::new(&radial).get(50, 20), [255, 255, 255, 255]);
    }

    #[test]
    fn three_boxes_match_the_requested_sigma() {
        for sigma in [1.0f32, 4.0, 25.0] {
            let variance: f32 = box_sizes(sigma).iter().map(|s| ((s * s) as f32 - 1.0) / 12.0).sum();
            assert!((variance.sqrt() - sigma).abs() < sigma * 0.08 + 0.3, "sigma {sigma} -> {}", variance.sqrt());
        }
    }
}
