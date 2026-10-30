//! Modes colorimétriques du document (Image › Mode), à la manière de Photoshop — sans moteur multicouche.
//!
//! Le moteur garde TOUJOURS ses pixels en RGBA8 prémultiplié (sRGB) : un mode est une règle posée sur le document,
//! tenue à trois endroits.
//!
//! 1. **À la conversion** ([`Document::convert_color_mode`], une seule étape d'historique) :
//!    - *Niveaux de gris* : chaque calque de pixels passe en gris ; textes, formes, calques de remplissage et effets
//!      de calque voient leurs couleurs converties (et sont redessinés) ; les masques le sont déjà. Les calques de
//!      réglage sont GARDÉS tels quels : leur effet sur la luminosité reste visible, leur effet sur la teinte est
//!      neutralisé par le rendu final (voir 2.) ; repasser en RVB leur rend tout leur effet, comme dans Photoshop où
//!      un réglage Teinte/Saturation reste dans la pile mais ne colore plus rien. Le contenu des objets dynamiques
//!      incorporés est converti lui aussi (une fois par contenu, toutes ses instances suivent) ; celui d'un objet lié
//!      ne l'est pas (son fichier fait foi) : son rendu passe en gris comme le reste.
//!    - *Bitmap* : l'image est aplatie (le dialogue de l'interface le demande d'abord), posée sur du blanc, passée en
//!      gris, rééchantillonnée à la résolution de sortie, puis réduite au noir et blanc par la méthode choisie : seuil
//!      à 50 %, diffusion d'erreur de Floyd–Steinberg (balayage en serpentin, moins de « vers » que le balayage
//!      simple), ou trame de demi-teintes (fréquence en lignes par pouce, angle, point rond « euclidien »). Le
//!      document ne contient alors plus qu'un calque opaque de pixels 0 ou 255.
//!    - *Couleurs RVB* : rien n'est recalculé — les couleurs perdues ne reviennent pas. Quitter les niveaux de gris
//!      convertit de nouveau tout le contenu (ce qui a été peint ou collé en couleur entre-temps devient gris : on
//!      retrouve exactement ce qu'on voyait) ; quitter le bitmap aplatit l'image telle qu'elle s'affiche (seuillée).
//!    - *Couleurs CMJN* : mode de TRAVAIL. Les pixels restent en RVB ; le document retient un profil de presse
//!      (FOGRA39…), l'interface garde l'épreuve écran de ce profil allumée tant que dure le mode, signale les couleurs
//!      hors gamme et propose l'export en CMJN. Les pixels ne sont pas « écrasés » dans la gamme : repasser en RVB ne
//!      perd rien.
//! 2. **Au rendu** ([`finish_tile`], appelé par `TileRenderer` sur chaque tuile composée, et la même règle dans la
//!    composition sur carte graphique de l'interface, `viewport/gpu-layers.ts`) : en gris, l'image finale est
//!    désaturée ; en bitmap, elle est seuillée (couleur ET transparence, à 50 %) à pleine résolution, et seulement
//!    désaturée dans les niveaux réduits (zoom arrière : un seuillage d'image réduite n'aurait aucun sens). Tout ce
//!    qu'on peint, colle ou importe dans le mode est donc garanti gris (ou noir et blanc) à l'écran, à l'export et
//!    à l'aplatissement, quelle que soit la façon dont il est arrivé — l'interface ramène en plus les couleurs
//!    choisies au gris (ou au noir et blanc). C'est le choix fait pour le bitmap : les outils restent utilisables
//!    (Photoshop en interdit la plupart), leurs gris et leurs bords adoucis sont seuillés au rendu.
//! 3. **À l'export** (`io.rs`) : PNG, TIFF et JPEG en vrais fichiers gris 8 bits ; PNG et TIFF 1 bit en bitmap.
//!
//! Luminance : celle de la conversion « Niveaux de gris » classique, Rec. 601 (0,299 R + 0,587 G + 0,114 B, la
//! « luminosité » du transfert de dégradé du moteur et des barres du sélecteur de couleur), en entiers exacts :
//! `(77 R + 150 G + 29 B + 128) >> 8`. Photoshop passe, lui, par son espace de travail gris (gain de point 20 %) ;
//! Rec. 601 en est l'approximation d'usage, sans profil. Le calcul porte directement sur les valeurs prémultipliées
//! (il est linéaire) : il ne dépasse jamais l'alpha, et un gris converti ne bouge plus (idempotence).

use std::sync::Arc;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cmyk::{CmykProfile, CmykSpace, RenderIntent};
use crate::document::{Document, LayerId, LayerKind};
use crate::effects::LayerEffect;
use crate::error::EngineError;
use crate::fill::FillContent;
use crate::render::TileRenderer;
use crate::tiles::{tiles_for, Tile, TileCoord, TileStore, MAX_DIMENSION, TILE_SIZE};

/// Mode colorimétrique d'un document. Enregistré dans le `.graf` (absent des anciens fichiers : RVB).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ColorMode {
    #[default]
    Rgb,
    Grayscale,
    Bitmap,
    /// Travail en CMJN : pixels RVB, épreuve écran et export dans cet espace CMJN (profil, intention, compensation du
    /// point noir : ceux des Paramètres de couleur au moment de la conversion, gardés avec le document comme
    /// Photoshop garde le profil d'un document CMJN). Les fichiers de la 1.2.0 n'ont que le profil : relatif, avec
    /// compensation.
    CmykWorkflow {
        profile: CmykProfile,
        #[serde(default)]
        intent: RenderIntent,
        #[serde(default = "yes")]
        bpc: bool,
    },
}

fn yes() -> bool {
    true
}

impl ColorMode {
    /// Mode Couleurs CMJN dans cet espace.
    pub fn cmyk(space: CmykSpace) -> Self {
        Self::CmykWorkflow { profile: space.profile, intent: space.intent, bpc: space.bpc }
    }

    /// Espace CMJN du mode Couleurs CMJN (`None` dans les autres modes).
    pub fn cmyk_space(self) -> Option<CmykSpace> {
        match self {
            Self::CmykWorkflow { profile, intent, bpc } => Some(CmykSpace { profile, intent, bpc }),
            _ => None,
        }
    }

    /// Libellé de l'état d'historique de la conversion vers ce mode.
    pub fn history_label(self) -> &'static str {
        match self {
            Self::Rgb => "Mode Couleurs RVB",
            Self::Grayscale => "Mode Niveaux de gris",
            Self::Bitmap => "Mode Bitmap",
            Self::CmykWorkflow { .. } => "Mode Couleurs CMJN",
        }
    }
}

/// Forme du point de trame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HalftoneShape {
    /// Point rond « euclidien » : rond dans les tons clairs, en damier à 50 %, blanc rond dans les tons sombres.
    #[default]
    Round,
}

/// Méthode de passage au noir et blanc.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum BitmapMethod {
    /// Seuil à 50 % : noir sous 128, blanc au-dessus.
    Threshold,
    /// Diffusion d'erreur de Floyd–Steinberg.
    Diffusion,
    /// Trame de demi-teintes : `frequency` en lignes par pouce, `angle` en degrés.
    Halftone {
        frequency: f32,
        angle: f32,
        #[serde(default)]
        shape: HalftoneShape,
    },
}

/// Réglages de la fenêtre Bitmap. `output_dpi` : résolution de sortie (l'image est rééchantillonnée si elle diffère
/// de celle du document, comme dans Photoshop).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BitmapOptions {
    pub method: BitmapMethod,
    pub output_dpi: f32,
}

// ------------------------------------------------------------------ pixels

/// Luminance 8 bits (Rec. 601, entiers exacts ; voir l'en-tête du module).
#[inline]
pub fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((77 * u32::from(r) + 150 * u32::from(g) + 29 * u32::from(b) + 128) >> 8) as u8
}

/// Couleur RVB ramenée au gris de même luminance.
#[inline]
pub fn gray_rgb([r, g, b]: [u8; 3]) -> [u8; 3] {
    let y = luma(r, g, b);
    [y, y, y]
}

/// Couleur RVB ramenée au noir ou au blanc (seuil à 50 % de la luminance).
#[inline]
pub fn bitmap_rgb(rgb: [u8; 3]) -> [u8; 3] {
    if gray_rgb(rgb)[0] >= 128 { [255; 3] } else { [0; 3] }
}

/// Pixel prémultiplié en gris (l'alpha ne change pas).
#[inline]
pub fn gray_px([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    let y = luma(r, g, b);
    [y, y, y, a]
}

/// Pixel prémultiplié seuillé : transparent sous 50 % d'opacité, sinon noir ou blanc opaque selon la luminance de
/// la couleur (comparée sans division : `Y × 255 ≥ 128 × alpha`, exactement la règle de la carte graphique).
#[inline]
pub fn bitmap_px(px: [u8; 4]) -> [u8; 4] {
    if px[3] < 128 {
        return [0; 4];
    }
    let y = u32::from(luma(px[0], px[1], px[2]));
    if y * 255 >= 128 * u32::from(px[3]) { [255; 4] } else { [0, 0, 0, 255] }
}

/// Même image, tuile par tuile (une tuile absente vaut une tuile transparente).
fn same_pixels(a: &TileStore, b: &TileStore) -> bool {
    let mut coords = a.sorted_coords();
    coords.extend(b.sorted_coords());
    coords.sort_unstable();
    coords.dedup();
    coords.into_iter().all(|coord| match (a.get(coord), b.get(coord)) {
        (Some(x), Some(y)) => Arc::ptr_eq(x, y) || x.bytes() == y.bytes(),
        (Some(t), None) | (None, Some(t)) => t.is_transparent(),
        (None, None) => true,
    })
}

fn map_tile(tile: &Tile, f: impl Fn([u8; 4]) -> [u8; 4]) -> Tile {
    let mut out = tile.clone();
    for px in out.bytes_mut().chunks_exact_mut(4) {
        px.copy_from_slice(&f([px[0], px[1], px[2], px[3]]));
    }
    out
}

/// Toutes les tuiles d'un stockage transformées pixel par pixel, en parallèle.
fn map_store(store: &TileStore, f: impl Fn([u8; 4]) -> [u8; 4] + Sync) -> TileStore {
    let tiles: Vec<(TileCoord, &Arc<Tile>)> = store.iter().collect();
    let mapped: Vec<(TileCoord, Tile)> = tiles.into_par_iter().map(|(coord, tile)| (coord, map_tile(tile, &f))).collect();
    let mut out = TileStore::new();
    for (coord, tile) in mapped {
        out.insert(coord, tile);
    }
    out
}

/// Rendu final d'une tuile composée selon le mode du document (voir l'en-tête du module, point 2). `level` : niveau
/// de la pyramide (0 = pleine résolution) ; le bitmap n'est seuillé qu'au niveau 0.
pub fn finish_tile(mode: ColorMode, level: u8, tile: Option<Arc<Tile>>) -> Option<Arc<Tile>> {
    let tile = tile?;
    match mode {
        ColorMode::Rgb | ColorMode::CmykWorkflow { .. } => Some(tile),
        ColorMode::Grayscale => Some(Arc::new(map_tile(&tile, gray_px))),
        ColorMode::Bitmap if level > 0 => Some(Arc::new(map_tile(&tile, gray_px))),
        ColorMode::Bitmap => {
            let out = map_tile(&tile, bitmap_px);
            (!out.is_transparent()).then(|| Arc::new(out))
        }
    }
}

// ------------------------------------------------------------------ noir et blanc

/// Réduit une image grise (un octet par pixel, `width × height`) au noir et blanc : chaque valeur devient 0 ou 255.
/// `dpi` : résolution de l'image (la trame en dépend) ; `origin` : position de l'image dans l'image complète, pour
/// qu'un extrait (aperçu) reçoive exactement la même trame que l'image entière.
pub fn dither(gray: &[u8], width: u32, height: u32, method: BitmapMethod, dpi: f32, origin: (i64, i64)) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    debug_assert_eq!(gray.len(), w * h);
    match method {
        BitmapMethod::Threshold => gray.par_iter().map(|&v| if v >= 128 { 255 } else { 0 }).collect(),
        BitmapMethod::Diffusion => diffuse(gray, w, h),
        BitmapMethod::Halftone { frequency, angle, shape: HalftoneShape::Round } => {
            // Taille d'une cellule de trame en pixels : résolution / linéature (au moins 2 px).
            let cell = (f64::from(dpi.max(1.0)) / f64::from(frequency.max(1.0))).max(2.0);
            let (sin, cos) = f64::from(angle).to_radians().sin_cos();
            let tau = std::f64::consts::TAU;
            let mut out = vec![0u8; w * h];
            out.par_chunks_mut(w.max(1)).enumerate().for_each(|(y, row)| {
                let py = (origin.1 + y as i64) as f64 + 0.5;
                for (x, dst) in row.iter_mut().enumerate() {
                    let v = gray[y * w + x];
                    *dst = match v {
                        0 => 0,
                        255 => 255,
                        _ => {
                            let px = (origin.0 + x as i64) as f64 + 0.5;
                            let u = (px * cos + py * sin) / cell;
                            let t = (-px * sin + py * cos) / cell;
                            let (fu, ft) = (u - u.floor() - 0.5, t - t.floor() - 0.5);
                            // Fonction de point euclidienne : 1 au centre de la cellule, −1 aux coins.
                            let spot = ((tau * fu).cos() + (tau * ft).cos()) * 0.5;
                            let darkness = 1.0 - f64::from(v) / 255.0;
                            if spot > 1.0 - 2.0 * darkness { 0 } else { 255 }
                        }
                    };
                }
            });
            out
        }
    }
}

/// Floyd–Steinberg en serpentin (lignes paires de gauche à droite, impaires de droite à gauche).
fn diffuse(gray: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    // Erreurs de la ligne en cours et de la suivante, avec une case de marge de chaque côté.
    let mut current = vec![0f32; w + 2];
    let mut next = vec![0f32; w + 2];
    for y in 0..h {
        let forward = y % 2 == 0;
        for i in 0..w {
            let x = if forward { i } else { w - 1 - i };
            let value = f32::from(gray[y * w + x]) + current[x + 1];
            let ink = if value >= 127.5 { 255u8 } else { 0 };
            out[y * w + x] = ink;
            let error = value - f32::from(ink);
            // Voisins dans le sens du balayage : devant (7/16), dessous derrière (3/16), dessous (5/16), dessous devant (1/16).
            let (ahead, behind) = if forward { (x + 2, x) } else { (x, x + 2) };
            current[ahead] += error * 7.0 / 16.0;
            next[behind] += error * 3.0 / 16.0;
            next[x + 1] += error * 5.0 / 16.0;
            next[ahead] += error * 1.0 / 16.0;
        }
        std::mem::swap(&mut current, &mut next);
        next.iter_mut().for_each(|e| *e = 0.0);
    }
    out
}

/// Pixel prémultiplié posé sur du blanc, en gris.
#[inline]
fn gray_on_white(px: &[u8]) -> u8 {
    let lift = 255 - px[3];
    luma(px[0].saturating_add(lift), px[1].saturating_add(lift), px[2].saturating_add(lift))
}

/// Image composée (niveau 0, mode du document compris) du rectangle `(x, y, largeur, hauteur)`, en gris sur blanc.
fn composite_gray(doc: &Document, renderer: &TileRenderer, (x, y, w, h): (i32, i32, u32, u32)) -> Vec<u8> {
    let size = TILE_SIZE as i32;
    let (tx0, ty0) = (x.div_euclid(size), y.div_euclid(size));
    let (tx1, ty1) = ((x + w as i32 - 1).div_euclid(size), (y + h as i32 - 1).div_euclid(size));
    let coords: Vec<TileCoord> = (ty0..=ty1).flat_map(|ty| (tx0..=tx1).map(move |tx| TileCoord::new(tx, ty))).collect();
    let tiles: Vec<(TileCoord, Option<Arc<Tile>>)> = coords.into_par_iter().map(|c| (c, renderer.tile(doc, 0, c))).collect();
    let mut flat = TileStore::new();
    for (coord, tile) in tiles {
        flat.restore(coord, tile);
    }
    flat.read_rect(x, y, w, h).par_chunks_exact(4).map(gray_on_white).collect()
}

/// Rééchantillonne une image grise (Lanczos 3).
fn resize_gray(gray: Vec<u8>, width: u32, height: u32, new_width: u32, new_height: u32) -> Result<Vec<u8>, EngineError> {
    if (width, height) == (new_width, new_height) {
        return Ok(gray);
    }
    use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images::Image};
    let failed = |e: &dyn std::fmt::Display| EngineError::Corrupt(format!("rééchantillonnage impossible : {e}"));
    let source = Image::from_vec_u8(width, height, gray, PixelType::U8).map_err(|e| failed(&e))?;
    let mut target = Image::new(new_width, new_height, PixelType::U8);
    Resizer::new()
        .resize(&source, &mut target, &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)))
        .map_err(|e| failed(&e))?;
    Ok(target.into_vec())
}

/// Taille de l'image à la résolution de sortie.
fn output_size(doc: &Document, output_dpi: f32) -> Result<(u32, u32, f64), EngineError> {
    let input = if doc.dpi.is_finite() && doc.dpi > 0.0 { doc.dpi } else { 72.0 };
    let output = if output_dpi.is_finite() && output_dpi > 0.0 { output_dpi } else { input };
    let k = f64::from(output) / f64::from(input);
    let (w, h) = (((f64::from(doc.width()) * k).round() as u32).max(1), ((f64::from(doc.height()) * k).round() as u32).max(1));
    if w > MAX_DIMENSION || h > MAX_DIMENSION {
        return Err(EngineError::InvalidDimensions { width: w, height: h, max: MAX_DIMENSION });
    }
    Ok((w, h, k))
}

/// Aperçu de la fenêtre Bitmap : un extrait d'au plus `max_width × max_height` pixels DE SORTIE, au centre de
/// l'image, avant (gris) et après (noir et blanc) la conversion. Renvoie (largeur, hauteur, avant, après).
pub fn bitmap_preview(doc: &Document, renderer: &TileRenderer, options: &BitmapOptions, (max_width, max_height): (u32, u32)) -> Result<(u32, u32, Vec<u8>, Vec<u8>), EngineError> {
    let (out_w, out_h, k) = output_size(doc, options.output_dpi)?;
    let (w, h) = (max_width.clamp(1, out_w), max_height.clamp(1, out_h));
    // Extrait de sortie centré, et le rectangle du document qui le produit.
    let (ox, oy) = ((out_w - w) / 2, (out_h - h) / 2);
    let x0 = (f64::from(ox) / k).floor() as i32;
    let y0 = (f64::from(oy) / k).floor() as i32;
    let sw = ((f64::from(w) / k).ceil() as u32).clamp(1, doc.width());
    let sh = ((f64::from(h) / k).ceil() as u32).clamp(1, doc.height());
    let source = composite_gray(doc, renderer, (x0, y0, sw, sh));
    let before = resize_gray(source, sw, sh, w, h)?;
    let after = dither(&before, w, h, options.method, options.output_dpi, (i64::from(ox), i64::from(oy)));
    Ok((w, h, before, after))
}

// ------------------------------------------------------------------ conversions du document

/// Couleurs d'un texte, d'une forme, d'un remplissage ou d'un effet ramenées au gris.
fn gray_text(text: &crate::text::TextData) -> crate::text::TextData {
    let mut out = text.clone();
    out.color = gray_rgb(out.color);
    for run in &mut out.runs {
        run.color = run.color.map(gray_rgb);
    }
    out
}

fn gray_rgba([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    let [y, _, _] = gray_rgb([r, g, b]);
    [y, y, y, a]
}

fn gray_shape(shape: &crate::shape::ShapeData) -> crate::shape::ShapeData {
    let mut out = shape.clone();
    out.fill = out.fill.map(gray_rgba);
    out.stroke = out.stroke.map(gray_rgba);
    out
}

fn gray_fill(fill: &FillContent) -> FillContent {
    let mut out = fill.clone();
    match &mut out {
        FillContent::SolidColor { color } => *color = gray_rgb(*color),
        FillContent::Gradient { stops, .. } => stops.iter_mut().for_each(|stop| stop.color = gray_rgba(stop.color)),
        FillContent::Pattern { color, background, .. } => {
            *color = gray_rgb(*color);
            *background = gray_rgb(*background);
        }
    }
    out
}

/// Énumération exhaustive : un nouvel effet de calque oblige à dire ici ce que deviennent ses couleurs.
fn gray_effect(effect: &LayerEffect) -> LayerEffect {
    let mut out = *effect;
    match &mut out {
        LayerEffect::DropShadow { color, .. }
        | LayerEffect::InnerShadow { color, .. }
        | LayerEffect::OuterGlow { color, .. }
        | LayerEffect::InnerGlow { color, .. }
        | LayerEffect::Stroke { color, .. }
        | LayerEffect::ColorOverlay { color, .. }
        | LayerEffect::Satin { color, .. } => *color = gray_rgb(*color),
        LayerEffect::GradientOverlay { from, to, .. } => {
            *from = gray_rgb(*from);
            *to = gray_rgb(*to);
        }
        LayerEffect::PatternOverlay { color, background, .. } => {
            *color = gray_rgb(*color);
            *background = gray_rgb(*background);
        }
        LayerEffect::BevelEmboss { highlight, shadow, .. } => {
            *highlight = gray_rgb(*highlight);
            *shadow = gray_rgb(*shadow);
        }
    }
    out
}

impl Document {
    /// Image › Mode : convertit le document vers `target` (voir l'en-tête du module). `bitmap` : réglages de la
    /// fenêtre Bitmap, exigés pour ce mode. À appeler dans UNE opération de session : la conversion entière forme un
    /// seul état d'historique, et Annuler rend le mode d'avant avec les pixels d'avant.
    pub fn convert_color_mode(&mut self, target: ColorMode, bitmap: Option<&BitmapOptions>) -> Result<(), EngineError> {
        let from = self.color_mode;
        if from == target {
            return Ok(());
        }
        if target == ColorMode::Bitmap && bitmap.is_none() {
            return Err(EngineError::InvalidOperation("réglages du mode Bitmap manquants".into()));
        }
        // On quitte un mode en figeant ce qu'il montrait : le bitmap aplati tel qu'il s'affiche (seuillé), les
        // niveaux de gris en convertissant de nouveau tout ce qui a pu arriver en couleur depuis.
        match from {
            ColorMode::Bitmap => self.flatten_displayed()?,
            ColorMode::Grayscale => self.gray_all_layers()?,
            ColorMode::Rgb | ColorMode::CmykWorkflow { .. } => {}
        }
        match target {
            ColorMode::Rgb | ColorMode::CmykWorkflow { .. } => {}
            ColorMode::Grayscale => {
                if from != ColorMode::Grayscale {
                    self.gray_all_layers()?;
                }
            }
            ColorMode::Bitmap => {
                // Le bitmap part d'une image en gris : rendu du document sans règle de mode, converti ci-dessous.
                self.color_mode = ColorMode::Rgb;
                if let Some(options) = bitmap {
                    self.flatten_to_bitmap(options)?;
                }
            }
        }
        self.color_mode = target;
        Ok(())
    }

    /// Calque par calque : pixels, textes, formes, remplissages et effets en gris (voir l'en-tête du module).
    fn gray_all_layers(&mut self) -> Result<(), EngineError> {
        let ids: Vec<LayerId> = self.walk_top_down().iter().map(|(_, layer)| layer.id).collect();
        for id in ids {
            // La conversion de mode passe outre les verrous, comme dans Photoshop : ils sont rendus tels quels après.
            let locks = self.layer(id)?.props.locks;
            self.edit_props(id, |props| props.locks.all = false)?;
            let result = self.gray_layer(id);
            self.edit_props(id, |props| props.locks = locks)?;
            result?;
        }
        self.gray_smart_contents()
    }

    /// Contenu des objets dynamiques INCORPORÉS : son document passe en gris lui aussi (calque par calque, objets
    /// imbriqués compris), une fois par contenu, et toutes ses instances suivent. Le mode du document contenu ne
    /// change pas (« Modifier le contenu » ouvre un document RVB aux pixels gris, comme ce qu'on voyait). Un contenu
    /// lié n'est pas touché (son fichier fait foi) : seul son rendu passe en gris au rendu final (`finish_tile`). Un
    /// contenu déjà gris est laissé tel quel, image d'origine comprise (rien à convertir).
    fn gray_smart_contents(&mut self) -> Result<(), EngineError> {
        let mut seen = std::collections::HashSet::new();
        let contents: Vec<Arc<crate::smart::SmartContent>> =
            self.walk_top_down().iter().filter_map(|(_, layer)| layer.smart().cloned()).filter(|content| !content.is_linked() && seen.insert(content.id)).collect();
        for content in contents {
            let mut inner = Document::clone(&content.document);
            inner.gray_all_layers()?;
            let grayed = content.edited(inner);
            if same_pixels(&grayed.flattened, &content.flattened) {
                continue;
            }
            self.update_smart_content(Arc::new(grayed), crate::document::SmartFit::Centre)?;
        }
        Ok(())
    }

    fn gray_layer(&mut self, id: LayerId) -> Result<(), EngineError> {
        let layer = self.layer(id)?;
        let effects: Vec<LayerEffect> = layer.effects.iter().map(gray_effect).collect();
        let effects_changed = effects != layer.effects;
        enum Change {
            Pixels,
            Text(crate::text::TextData),
            Shape(crate::shape::ShapeData),
            Fill(FillContent),
            None,
        }
        let change = match &layer.kind {
            LayerKind::Pixel { .. } => Change::Pixels,
            LayerKind::Text { text, .. } => Some(gray_text(text)).filter(|t| t != text).map_or(Change::None, Change::Text),
            LayerKind::Shape { shape, .. } => Some(gray_shape(shape)).filter(|s| s != shape).map_or(Change::None, Change::Shape),
            LayerKind::Fill { fill, .. } => Some(gray_fill(fill)).filter(|f| f != fill).map_or(Change::None, Change::Fill),
            LayerKind::Group { .. } | LayerKind::Adjustment { .. } | LayerKind::Smart { .. } => Change::None,
        };
        match change {
            // Filtres dynamiques : l'origine passe en gris et les filtres sont refaits dessus.
            Change::Pixels if layer.filters.is_some() => self.edit_filter_source(id, |store| map_store(store, gray_px))?,
            Change::Pixels => {
                let store = self.pixels_mut(id)?;
                *store = map_store(store, gray_px);
            }
            Change::Text(text) => self.set_text(id, text)?,
            Change::Shape(shape) => self.set_shape(id, shape)?,
            Change::Fill(fill) => self.set_fill(id, fill)?,
            Change::None => {}
        }
        if effects_changed {
            self.set_effects(id, effects)?;
        }
        Ok(())
    }

    /// Composition du document entier au niveau 0 (règle de mode comprise), en RGBA prémultiplié tuile par tuile.
    fn composite_store(&self) -> TileStore {
        let renderer = TileRenderer::new();
        let (tw, th) = (tiles_for(self.width()) as i32, tiles_for(self.height()) as i32);
        let coords: Vec<TileCoord> = (0..th).flat_map(|ty| (0..tw).map(move |tx| TileCoord::new(tx, ty))).collect();
        let tiles: Vec<(TileCoord, Option<Arc<Tile>>)> = coords.into_par_iter().map(|c| (c, renderer.tile(self, 0, c))).collect();
        let mut flat = TileStore::new();
        for (coord, tile) in tiles {
            // Rien hors du cadre : la composition ne garde que le document.
            flat.restore(coord, tile);
        }
        flat
    }

    /// Aplatit le document tel qu'il s'affiche (mode compris) en un seul calque « Arrière-plan ».
    fn flatten_displayed(&mut self) -> Result<(), EngineError> {
        let flat = self.composite_store();
        self.replace_all_layers("Arrière-plan", flat)?;
        Ok(())
    }

    /// Aplatit sur du blanc, passe en gris, rééchantillonne à la résolution de sortie et réduit au noir et blanc.
    fn flatten_to_bitmap(&mut self, options: &BitmapOptions) -> Result<(), EngineError> {
        let (width, height) = (self.width(), self.height());
        let (out_w, out_h, k) = output_size(self, options.output_dpi)?;
        let gray = composite_gray(self, &TileRenderer::new(), (0, 0, width, height));
        let gray = resize_gray(gray, width, height, out_w, out_h)?;
        let bits = dither(&gray, out_w, out_h, options.method, options.output_dpi, (0, 0));
        let rgba: Vec<u8> = bits.par_iter().flat_map_iter(|&v| [v, v, v, 255]).collect();
        let pixels = TileStore::from_premultiplied(out_w, out_h, &rgba).ok_or_else(|| EngineError::Corrupt("tampon de taille inattendue".into()))?;
        self.set_selection(None);
        self.replace_all_layers("Arrière-plan", pixels)?;
        if (out_w, out_h) != (width, height) {
            // Repères et tracé suivent le rééchantillonnage ; puis le cadre prend la nouvelle taille (aucun calque
            // à décaler : le seul calque est à l'origine).
            let k = k as f32;
            for guide in &mut self.guides {
                guide.position *= k;
            }
            for anchor in self.paths.iter_mut().flat_map(|sub| sub.anchors.iter_mut()) {
                for point in [&mut anchor.point, &mut anchor.handle_in, &mut anchor.handle_out] {
                    *point = [point[0] * k, point[1] * k];
                }
            }
            self.crop(0, 0, out_w, out_h)?;
        }
        if options.output_dpi.is_finite() && options.output_dpi > 0.0 {
            self.dpi = options.output_dpi;
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ export

/// Image RGBA (alpha droit) en gris : un octet par pixel, et l'alpha à part s'il n'est pas opaque partout.
pub(crate) fn gray_planes(rgba: &[u8]) -> (Vec<u8>, Option<Vec<u8>>) {
    let gray: Vec<u8> = rgba.par_chunks_exact(4).map(|px| luma(px[0], px[1], px[2])).collect();
    let alpha = rgba.par_chunks_exact(4).any(|px| px[3] != 255).then(|| rgba.par_chunks_exact(4).map(|px| px[3]).collect());
    (gray, alpha)
}

/// Image RGBA (alpha droit) en gris sur du blanc (formats sans transparence).
pub(crate) fn gray_on_white_straight(rgba: &[u8]) -> Vec<u8> {
    rgba.par_chunks_exact(4)
        .map(|px| {
            let a = u32::from(px[3]);
            let y = u32::from(luma(px[0], px[1], px[2]));
            ((y * a + 255 * (255 - a) + 127) / 255) as u8
        })
        .collect()
}

/// Image RGBA (alpha droit) en noir et blanc sur du blanc : `true` = blanc.
pub(crate) fn bilevel(rgba: &[u8]) -> Vec<bool> {
    gray_on_white_straight(rgba).into_par_iter().map(|v| v >= 128).collect()
}

/// Lignes de bits (1 = blanc), complétées à l'octet, le bit de poids fort à gauche : PNG 1 bit et TIFF 1 bit.
pub(crate) fn pack_bits_rows(white: &[bool], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let stride = w.div_ceil(8);
    let mut out = vec![0u8; stride * h];
    out.par_chunks_mut(stride.max(1)).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            if white[y * w + x] {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
    });
    out
}

/// PNG en niveaux de gris : 8 bits (avec son alpha s'il n'est pas opaque partout), ou 1 bit (noir et blanc).
pub(crate) fn encode_png_gray(width: u32, height: u32, rgba: &[u8], bits: u8) -> Result<Vec<u8>, EngineError> {
    if bits == 1 {
        let packed = pack_bits_rows(&bilevel(rgba), width, height);
        return crate::png_parallel::encode_raw(width, height, width.div_ceil(8), &packed, 1, 1, 0, 6);
    }
    match gray_planes(rgba) {
        (gray, None) => crate::png_parallel::encode_raw(width, height, width, &gray, 1, 8, 0, 6),
        (gray, Some(alpha)) => {
            let interleaved: Vec<u8> = gray.iter().zip(&alpha).flat_map(|(g, a)| [*g, *a]).collect();
            crate::png_parallel::encode_raw(width, height, width, &interleaved, 2, 8, 4, 6)
        }
    }
}

/// Compression PackBits (TIFF, compression 32773) d'une ligne : répétitions et suites littérales de 128 octets au plus.
fn packbits(row: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < row.len() {
        let mut run = 1;
        while i + run < row.len() && run < 128 && row[i + run] == row[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((1i16 - run as i16) as i8 as u8);
            out.push(row[i]);
            i += run;
            continue;
        }
        let start = i;
        while i < row.len() && i - start < 128 && !(i + 1 < row.len() && row[i + 1] == row[i]) {
            i += 1;
        }
        if i == start {
            i += 1;
        }
        out.push((i - start - 1) as u8);
        out.extend_from_slice(&row[start..i]);
    }
}

/// TIFF 1 bit (noir et blanc), une bande, compression PackBits. La bibliothèque `tiff` n'écrit ni le 1 bit ni le
/// CCITT G4 : l'en-tête est donc écrit ici (TIFF de base, petit-boutiste, `BlackIsZero`). PackBits fait partie du
/// TIFF de base : tout lecteur l'ouvre.
pub(crate) fn encode_tiff_bilevel(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let stride = width.div_ceil(8) as usize;
    let packed = pack_bits_rows(&bilevel(rgba), width, height);
    let mut data = Vec::with_capacity(packed.len() / 4);
    for row in packed.chunks_exact(stride.max(1)) {
        packbits(row, &mut data);
    }
    // Disposition : en-tête (8), données, puis IFD (aligné sur 2 octets).
    let mut out = Vec::with_capacity(data.len() + 200);
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42u16.to_le_bytes());
    let data_at = 8u32;
    let ifd_at = (data_at + data.len() as u32 + 1) & !1;
    out.extend_from_slice(&ifd_at.to_le_bytes());
    out.extend_from_slice(&data);
    out.resize(ifd_at as usize, 0);
    // Étiquettes, par numéro croissant : (étiquette, type, valeur). Type 3 = SHORT, 4 = LONG.
    let entries: [(u16, u16, u32); 10] = [
        (256, 4, width),             // ImageWidth
        (257, 4, height),            // ImageLength
        (258, 3, 1),                 // BitsPerSample
        (259, 3, 32773),             // Compression : PackBits
        (262, 3, 1),                 // PhotometricInterpretation : BlackIsZero
        (273, 4, data_at),           // StripOffsets
        (277, 3, 1),                 // SamplesPerPixel
        (278, 4, height),            // RowsPerStrip
        (279, 4, data.len() as u32), // StripByteCounts
        (284, 3, 1),                 // PlanarConfiguration
    ];
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, kind, value) in entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        if kind == 3 {
            out.extend_from_slice(&(value as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
        } else {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // pas d'IFD suivant
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Background;
    use crate::session::Session;

    /// Dégradé horizontal du noir au blanc.
    fn ramp(width: u32, height: u32) -> Vec<u8> {
        (0..height).flat_map(|_| (0..width).map(move |x| (x * 255 / (width - 1)) as u8)).collect()
    }

    fn render(doc: &Document) -> Vec<u8> {
        doc.composite_store().to_premultiplied(doc.width(), doc.height())
    }

    #[test]
    fn luma_is_rec601_exact_and_idempotent() {
        assert_eq!(luma(255, 255, 255), 255);
        assert_eq!(luma(0, 0, 0), 0);
        assert_eq!(luma(255, 0, 0), 77);
        assert_eq!(luma(0, 255, 0), 149);
        assert_eq!(luma(0, 0, 255), 29);
        for v in 0..=255u8 {
            assert_eq!(luma(v, v, v), v, "un gris ne bouge pas");
        }
        // Prémultiplié : jamais au-dessus de l'alpha.
        for a in [0u8, 1, 77, 200, 255] {
            assert!(gray_px([a, a, a, a])[0] <= a);
        }
    }

    #[test]
    fn grayscale_conversion_covers_every_layer_kind() {
        let mut doc = Document::with_background("gris", 64, 64, 72.0, Background::White).expect("document");
        let mut red = TileStore::new();
        red.fill(32, 32, [255, 0, 0, 255]);
        let pixel = doc.add_pixel_layer("rouge", red, None, 1).expect("calque");
        let shape = crate::shape::ShapeData {
            kind: crate::shape::ShapeKind::Rect,
            width: 20.0,
            height: 10.0,
            radius: 0.0,
            fill: Some([0, 0, 255, 255]),
            stroke: Some([0, 255, 0, 128]),
            stroke_width: 2.0,
            sides: 3,
            transform: crate::transform::Matrix::IDENTITY,
        };
        let shape_id = doc.add_shape_layer(shape, (30, 30), None, 2).expect("forme");
        let fill_id = doc.add_fill_layer(FillContent::SolidColor { color: [0, 255, 0] }, "vert", None, 3).expect("remplissage");
        doc.set_effects(pixel, vec![LayerEffect::ColorOverlay { enabled: true, color: [255, 0, 0], opacity: 0.0, blend: crate::blend::BlendMode::Normal }]).expect("effet");
        doc.edit_props(pixel, |p| p.locks.all = true).expect("verrou");
        let hue = doc.add_adjustment_layer(crate::adjust::Adjustment::HueSaturation { hue: 90.0, saturation: 50.0, lightness: 0.0, range_start: 0.0, range_end: 0.0, feather_in: 0.0, feather_out: 0.0, range_center: 0.0, range_width: 0.0 }, None, 4, false).expect("réglage");
        doc.convert_color_mode(ColorMode::Grayscale, None).expect("conversion");
        assert_eq!(doc.color_mode(), ColorMode::Grayscale);
        // Pixels : rouge -> 77, même verrouillé ; le verrou est rendu.
        let layer = doc.layer(pixel).expect("calque");
        assert!(layer.props.locks.all);
        assert_eq!(layer.pixels().expect("pixels").get(TileCoord::new(0, 0)).expect("tuile").pixel(5, 5), [77, 77, 77, 255]);
        assert!(matches!(layer.effects[0], LayerEffect::ColorOverlay { color: [77, 77, 77], .. }));
        let shape = doc.layer(shape_id).expect("forme").shape().cloned().expect("forme");
        assert_eq!(shape.fill, Some([29, 29, 29, 255]));
        assert_eq!(shape.stroke, Some([149, 149, 149, 128]));
        assert_eq!(doc.layer(fill_id).expect("remplissage").fill_content(), Some(&FillContent::SolidColor { color: [149, 149, 149] }));
        // Le réglage de teinte est gardé ; le rendu final reste gris malgré lui.
        assert!(doc.layer(hue).expect("réglage").adjustment().is_some());
        for px in render(&doc).chunks_exact(4) {
            assert!(px[0] == px[1] && px[1] == px[2], "rendu gris : {px:?}");
        }
    }

    #[test]
    fn colors_painted_in_grayscale_render_gray_and_are_baked_when_leaving() {
        let mut doc = Document::with_background("gris", 40, 40, 72.0, Background::White).expect("document");
        doc.convert_color_mode(ColorMode::Grayscale, None).expect("gris");
        // Peinture en couleur arrivée après la conversion (collage, import…).
        let mut red = TileStore::new();
        red.fill(10, 10, [255, 0, 0, 255]);
        let id = doc.add_pixel_layer("collé", red, None, 1).expect("calque");
        assert_eq!(&render(&doc)[..4], &[77, 77, 77, 255], "affiché en gris");
        doc.convert_color_mode(ColorMode::Rgb, None).expect("rvb");
        assert_eq!(doc.color_mode(), ColorMode::Rgb);
        assert_eq!(doc.layer(id).expect("calque").pixels().expect("pixels").get(TileCoord::new(0, 0)).expect("tuile").pixel(0, 0), [77, 77, 77, 255], "les couleurs ne reviennent pas");
    }

    #[test]
    fn grayscale_final_render_and_bitmap_threshold_follow_the_level() {
        let tile = Arc::new(Tile::filled([200, 40, 40, 255]));
        assert_eq!(finish_tile(ColorMode::Rgb, 0, Some(tile.clone())).expect("tuile").pixel(0, 0), [200, 40, 40, 255]);
        let y = luma(200, 40, 40);
        assert_eq!(finish_tile(ColorMode::Grayscale, 0, Some(tile.clone())).expect("tuile").pixel(0, 0), [y, y, y, 255]);
        assert_eq!(finish_tile(ColorMode::Bitmap, 0, Some(tile.clone())).expect("tuile").pixel(0, 0), [0, 0, 0, 255], "sombre : noir");
        assert_eq!(finish_tile(ColorMode::Bitmap, 1, Some(tile)).expect("tuile").pixel(0, 0), [y, y, y, 255], "niveau réduit : gris");
        // Transparence partielle : seuillée.
        assert_eq!(bitmap_px([100, 100, 100, 100]), [0; 4]);
        assert_eq!(bitmap_px([150, 150, 150, 160]), [255; 4]);
        assert!(finish_tile(ColorMode::Bitmap, 0, Some(Arc::new(Tile::filled([10, 10, 10, 60])))).is_none());
    }

    #[test]
    fn bitmap_methods_draw_the_expected_patterns_on_a_ramp() {
        let (w, h) = (256u32, 64u32);
        let gray = ramp(w, h);
        let black_share = |bits: &[u8], x0: u32, x1: u32| {
            let mut black = 0;
            for y in 0..h {
                for x in x0..x1 {
                    black += usize::from(bits[(y * w + x) as usize] == 0);
                }
            }
            black as f64 / f64::from((x1 - x0) * h)
        };
        // Seuil : tout noir à gauche du milieu, tout blanc à droite.
        let threshold = dither(&gray, w, h, BitmapMethod::Threshold, 300.0, (0, 0));
        assert!(threshold.iter().all(|&v| v == 0 || v == 255));
        assert_eq!(black_share(&threshold, 0, 128), 1.0);
        assert_eq!(black_share(&threshold, 128, 256), 0.0);
        // Diffusion et trame : la part de noir suit le ton (dégradé), à quelques pour cent près, et décroît.
        for method in [BitmapMethod::Diffusion, BitmapMethod::Halftone { frequency: 50.0, angle: 45.0, shape: HalftoneShape::Round }] {
            let bits = dither(&gray, w, h, method, 300.0, (0, 0));
            assert!(bits.iter().all(|&v| v == 0 || v == 255));
            let mut previous = 1.1;
            for band in 0..8 {
                let (x0, x1) = (band * 32, band * 32 + 32);
                let share = black_share(&bits, x0, x1);
                let expected = 1.0 - (f64::from(x0 + x1) / 2.0) / 255.0;
                assert!((share - expected).abs() < 0.12, "{method:?} bande {band} : {share:.3} au lieu de {expected:.3}");
                assert!(share <= previous + 0.02, "{method:?} : la part de noir décroît");
                previous = share;
            }
            // Noir pur et blanc pur restent purs.
            assert_eq!(bits[0], 0);
            assert_eq!(bits[(w - 1) as usize], 255);
        }
        // Diffusion : un gris moyen uniforme donne un mélange fin (aucune grande plage d'une seule couleur).
        let mid = dither(&vec![128u8; 64 * 64], 64, 64, BitmapMethod::Diffusion, 300.0, (0, 0));
        assert!(mid.chunks(8).all(|run| run.contains(&0) && run.contains(&255)), "motif fin");
        // Trame : périodique, de période cellule (300 / 50 = 6 px) le long des axes tournés : à 0°, 6 px exactement.
        let flat = vec![100u8; 60 * 60];
        let screen = dither(&flat, 60, 60, BitmapMethod::Halftone { frequency: 50.0, angle: 0.0, shape: HalftoneShape::Round }, 300.0, (0, 0));
        for y in 0..54 {
            for x in 0..54 {
                assert_eq!(screen[y * 60 + x], screen[y * 60 + x + 6]);
                assert_eq!(screen[y * 60 + x], screen[(y + 6) * 60 + x]);
            }
        }
        // Un extrait reçoit la même trame que l'image entière.
        let part = dither(&flat[..60 * 10], 60, 10, BitmapMethod::Halftone { frequency: 50.0, angle: 0.0, shape: HalftoneShape::Round }, 300.0, (0, 20));
        assert_eq!(&part[..], &screen[20 * 60..30 * 60]);
    }

    #[test]
    fn bitmap_conversion_flattens_resamples_and_is_undone_in_one_step() {
        let mut doc = Document::with_background("bitmap", 100, 50, 100.0, Background::White).expect("document");
        let mut dark = TileStore::new();
        dark.fill(50, 50, [30, 30, 30, 255]);
        doc.add_pixel_layer("sombre", dark, None, 1).expect("calque");
        let mut session = Session::new(doc, "Ouvrir");
        let options = BitmapOptions { method: BitmapMethod::Threshold, output_dpi: 200.0 };
        session.apply(ColorMode::Bitmap.history_label(), None, |doc| doc.convert_color_mode(ColorMode::Bitmap, Some(&options))).expect("bitmap");
        let doc = session.document();
        assert_eq!(doc.color_mode(), ColorMode::Bitmap);
        assert_eq!((doc.width(), doc.height(), doc.dpi), (200, 100, 200.0), "rééchantillonné à la résolution de sortie");
        assert_eq!(doc.layer_count(), 1, "aplati");
        let image = render(doc);
        assert_eq!(&image[..4], &[0, 0, 0, 255]);
        assert_eq!(&image[(150 * 4)..(150 * 4 + 4)], &[255; 4]);
        assert!(image.chunks_exact(4).all(|px| px == [0, 0, 0, 255] || px == [255; 4]));
        session.undo();
        let doc = session.document();
        assert_eq!(doc.color_mode(), ColorMode::Rgb);
        assert_eq!((doc.width(), doc.height(), doc.layer_count()), (100, 50, 2), "Annuler rend tout");
        assert_eq!(session.history().0.len(), 2, "une seule étape d'historique");
    }

    #[test]
    fn painting_in_bitmap_mode_is_thresholded_and_baked_when_leaving() {
        let mut doc = Document::with_background("bitmap", 32, 32, 72.0, Background::White).expect("document");
        doc.convert_color_mode(ColorMode::Bitmap, Some(&BitmapOptions { method: BitmapMethod::Diffusion, output_dpi: 72.0 })).expect("bitmap");
        let mut stroke = TileStore::new();
        stroke.fill(8, 8, [90, 90, 90, 140]); // gris à demi transparent : bord adouci d'un pinceau
        doc.add_pixel_layer("trait", stroke, None, 1).expect("calque");
        let image = render(&doc);
        assert!(image.chunks_exact(4).all(|px| px == [0, 0, 0, 255] || px == [255; 4]), "seuillé au rendu");
        doc.convert_color_mode(ColorMode::Grayscale, None).expect("gris");
        assert_eq!(doc.layer_count(), 1, "aplati tel qu'affiché");
        assert_eq!(render(&doc), image);
    }

    #[test]
    fn grayscale_converts_embedded_smart_contents_once_for_all_instances() {
        let mut doc = Document::with_background("gris", 64, 64, 72.0, Background::White).expect("document");
        let mut red = TileStore::new();
        red.fill(20, 20, [200, 40, 40, 255]);
        let layer = doc.add_pixel_layer("rouge", red, None, 1).expect("calque");
        let smart = doc.convert_to_smart(&[layer]).expect("objet dynamique");
        let twin = doc.duplicate_layer(smart).expect("instance");
        doc.convert_color_mode(ColorMode::Grayscale, None).expect("gris");
        let content = Arc::clone(doc.layer(smart).expect("objet").smart().expect("contenu"));
        let y = luma(200, 40, 40);
        let inner = content.document.walk_top_down().iter().find_map(|(_, l)| l.pixels().cloned()).expect("pixels du contenu");
        assert_eq!(crate::tiles::Sampler::new(&inner).get(5, 5), [y, y, y, 255], "le document contenu est converti");
        assert_eq!(content.document.color_mode(), ColorMode::Rgb, "son mode ne change pas");
        assert_eq!(crate::tiles::Sampler::new(&content.flattened).get(5, 5), [y, y, y, 255]);
        let other = doc.layer(twin).expect("instance").smart().expect("contenu");
        assert!(Arc::ptr_eq(other, &content), "les instances partagent toujours le même contenu");
        // Repasser en RVB ne recolore rien ; une seconde conversion ne refait rien (contenu déjà gris).
        doc.convert_color_mode(ColorMode::Rgb, None).expect("rvb");
        let again = doc.layer(smart).expect("objet").smart().expect("contenu");
        assert!(Arc::ptr_eq(&again.flattened, &content.flattened), "contenu déjà gris laissé tel quel");
    }

    #[test]
    fn cmyk_workflow_keeps_the_pixels_and_switches_profiles() {
        let mut doc = Document::with_background("cmjn", 16, 16, 72.0, Background::White).expect("document");
        let mut red = TileStore::new();
        red.fill(16, 16, [255, 0, 0, 255]);
        doc.add_pixel_layer("rouge", red, None, 1).expect("calque");
        let before = render(&doc);
        doc.convert_color_mode(ColorMode::cmyk(CmykProfile::Fogra39.into()), None).expect("cmjn");
        doc.convert_color_mode(ColorMode::cmyk(CmykProfile::Swop2006c3.into()), None).expect("autre profil");
        assert_eq!(doc.color_mode(), ColorMode::cmyk(CmykProfile::Swop2006c3.into()));
        assert_eq!(render(&doc), before, "pixels intacts");
        doc.convert_color_mode(ColorMode::Rgb, None).expect("rvb");
        assert_eq!(render(&doc), before);
    }

    #[test]
    fn mode_serialization_matches_the_interface() {
        assert_eq!(serde_json::to_string(&ColorMode::Grayscale).unwrap(), r#"{"kind":"grayscale"}"#);
        assert_eq!(serde_json::to_string(&ColorMode::cmyk(CmykProfile::Fogra51.into())).unwrap(), r#"{"kind":"cmykWorkflow","profile":"fogra51","intent":"relative","bpc":true}"#);
        // Fichiers de la 1.2.0 : le profil seul.
        let old: ColorMode = serde_json::from_str(r#"{"kind":"cmykWorkflow","profile":"fogra52"}"#).unwrap();
        assert_eq!(old.cmyk_space(), Some(CmykProfile::Fogra52.into()));
        let options: BitmapOptions = serde_json::from_str(r#"{"method":{"kind":"halftone","frequency":53,"angle":45},"outputDpi":300}"#).unwrap();
        assert_eq!(options.method, BitmapMethod::Halftone { frequency: 53.0, angle: 45.0, shape: HalftoneShape::Round });
    }

    #[test]
    fn bilevel_tiff_and_png_read_back_with_the_image_library() {
        let (w, h) = (37u32, 9u32);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| if (i % w + i / w) % 3 == 0 { [0, 0, 0, 255] } else { [255; 4] }).collect();
        for bytes in [encode_tiff_bilevel(w, h, &rgba), encode_png_gray(w, h, &rgba, 1).expect("png")] {
            let image = image::load_from_memory(&bytes).expect("relu").into_luma8();
            assert_eq!(image.dimensions(), (w, h));
            for (i, v) in image.as_raw().iter().enumerate() {
                assert_eq!(*v, rgba[i * 4], "pixel {i}");
            }
        }
        let mut out = Vec::new();
        packbits(&[1, 1, 1, 2, 3, 4, 4], &mut out);
        assert_eq!(out, [254, 1, 1, 2, 3, 255, 4]);
    }
}
