//! Export PDF : une page, une image — l'équivalent de « Enregistrer en Photoshop PDF » pour une image aplatie.
//!
//! Le fichier est écrit à la main (aucune dépendance) : c'est un PDF 1.6 minimal et parfaitement standard.
//!
//! ```text
//! 1  Catalog      /Pages 2, /OutputIntents (CMJN avec profil), /Metadata (XMP, s'il y a des métadonnées)
//! 2  Pages        une seule page
//! 3  Page         /MediaBox = taille du document en points (px × 72 / ppp), /TrimBox identique en CMJN
//! 4  Contents     « q L 0 0 H 0 0 cm /Im0 Do Q » : l'image remplit la page
//! 5  Image        RVB ou CMJN 8 bits, FlateDecode (sans perte, prédicteurs PNG) ou DCTDecode (JPEG)
//! …  SMask        alpha de l'image (RVB seulement ; en CMJN la transparence est posée sur du blanc)
//! …  ICC          profil incorporé (/ICCBased), partagé par l'image et l'intention de sortie
//! …  OutputIntent /S /GTS_PDFX, /DestOutputProfile = le profil de presse (comme un PDF d'impression)
//! …  Info         auteur, sujet, mots-clés, logiciel, date
//! ```
//!
//! **PDF/X-4** (ISO 15930-7, la norme d'impression actuelle) quand `print` est donné : CMJN obligatoire avec le profil
//! de presse incorporé en intention de sortie, `/TrimBox` (format fini) et `/BleedBox` (fonds perdus) déclarées,
//! traits de coupe facultatifs dans une marge ajoutée autour, métadonnées XMP avec `pdfxid:GTS_PDFXVersion`,
//! `pdf:Trapped` et le titre, encrage limité à la séparation.
//!
//! En CMJN JPEG, l'encodeur écrit la convention d'Adobe (segment APP14) : encres inversées (255 = pas d'encre).
//! Un lecteur de PDF ignore ce segment ; l'image porte donc `/Decode [1 0 1 0 1 0 1 0]`, qui inverse de nouveau.

use std::fmt::Write as _;

use crate::cmyk::{self, CmykSpace};
use crate::error::EngineError;
use crate::metadata::ImageMetadata;

/// Réglages d'un PDF.
#[derive(Clone, Copy, Debug)]
pub struct PdfSettings<'a> {
    /// Taille de la page en points (1/72 de pouce).
    pub page: [f32; 2],
    /// CMJN (avec ce profil) ou RVB (`None`).
    pub cmyk: Option<CmykSpace>,
    /// Incorporer le profil ICC (sRGB en RVB, le profil de presse en CMJN, avec l'intention de sortie).
    pub embed_profile: bool,
    /// Qualité JPEG (DCTDecode), ou `None` : sans perte (FlateDecode).
    pub jpeg_quality: Option<u8>,
    pub metadata: Option<&'a ImageMetadata>,
    /// Niveaux de gris (`DeviceGray`, document en mode Gris ou Bitmap) : `Some(8)` gris 8 bits (avec son masque de
    /// transparence, JPEG possible), `Some(1)` noir et blanc 1 bit sur blanc (toujours sans perte). Prime sur `cmyk`.
    /// Pas de profil ICC en gris : aucun profil gris n'est livré, l'image est en `DeviceGray` simple.
    pub gray_bits: Option<u8>,
    /// Impression (PDF/X-4), voir l'en-tête.
    pub print: Option<&'a PdfPrint>,
}

/// Réglages d'impression d'un PDF/X-4.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfPrint {
    /// Fonds perdus du document, en mm (l'image les contient).
    pub bleed_mm: f32,
    /// Garder les fonds perdus dans le PDF (sinon l'image est rognée au format fini).
    #[serde(default = "yes")]
    pub include_bleed: bool,
    /// Traits de coupe dans une marge ajoutée autour de la page.
    #[serde(default)]
    pub marks: bool,
    /// Limite d'encrage en % appliquée à la séparation (0 : aucune).
    #[serde(default)]
    pub ink_limit: u16,
    /// Titre du document (exigé par PDF/X-4).
    #[serde(default)]
    pub title: String,
    /// Noirs à l'impression (voir `print::apply_black_rules`) : noir pur en 100 % N, aplats en noir riche.
    #[serde(default = "yes")]
    pub pure_black: bool,
    #[serde(default = "yes")]
    pub rich_black: bool,
    /// Résolution de l'image, pour la largeur du bord des aplats (0,5 mm).
    #[serde(default)]
    pub dpi: f32,
}

fn yes() -> bool {
    true
}

/// Marge autour des fonds perdus quand il y a des traits de coupe, en points (7 mm).
const MARKS_PAD: f32 = 20.0;
/// Écart entre les fonds perdus et le début d'un trait de coupe, et longueur du trait, en points.
const MARK_OFFSET: f32 = 3.0;
const MARK_LENGTH: f32 = 14.0;

/// Image RGBA (alpha droit) → fichier PDF d'une page.
pub fn encode(width: u32, height: u32, rgba: &[u8], settings: &PdfSettings) -> Result<Vec<u8>, EngineError> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return Err(EngineError::Corrupt("PDF : tampon de taille inattendue".into()));
    }
    if settings.gray_bits.is_some() && settings.print.is_none() {
        return encode_gray(width, height, rgba, settings);
    }
    if let Some(print) = settings.print {
        return encode_x4(width, height, rgba, settings, print);
    }
    // Pixels de l'image et masque de transparence éventuel.
    let (components, pixels, alpha): (usize, Vec<u8>, Option<Vec<u8>>) = match settings.cmyk {
        Some(space) => (4, cmyk::rgba_to_cmyk(rgba, space)?, None),
        None => {
            let opaque = rgba.chunks_exact(4).all(|px| px[3] == 255);
            // En JPEG, les pixels entièrement transparents deviennent blancs : sinon le noir qu'ils ont souvent
            // (couleur indéfinie) baverait en halo sombre autour des bords détourés, malgré le masque.
            let lossy = settings.jpeg_quality.is_some();
            let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|px| if lossy && px[3] == 0 { [255, 255, 255] } else { [px[0], px[1], px[2]] }).collect();
            let alpha = (!opaque).then(|| rgba.chunks_exact(4).map(|px| px[3]).collect());
            (3, rgb, alpha)
        }
    };

    let mut pdf = Writer::new();
    // Numéros d'objets : les cinq premiers sont fixes, les suivants n'existent que si nécessaire.
    let (catalog, pages, page, contents, image) = (1, 2, 3, 4, 5);
    let mut next = 6;
    let mut take = || {
        next += 1;
        next - 1
    };
    let smask = alpha.as_ref().map(|_| take());
    let icc = settings.embed_profile.then(&mut take);
    let intent = (settings.embed_profile && settings.cmyk.is_some()).then(&mut take);
    let xmp = settings.metadata.filter(|m| !m.is_empty()).map(|_| take());
    let info = take();
    let count = next;

    // ---- Catalogue, pages, page, contenu
    let mut extra = String::new();
    if let Some(intent) = intent {
        let _ = write!(extra, " /OutputIntents [{intent} 0 R]");
    }
    if let Some(xmp) = xmp {
        let _ = write!(extra, " /Metadata {xmp} 0 R");
    }
    pdf.object(catalog, &format!("<< /Type /Catalog /Pages {pages} 0 R{extra} >>"));
    pdf.object(pages, &format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>"));
    let (pw, ph) = (number(settings.page[0].max(1.0)), number(settings.page[1].max(1.0)));
    let trim = if settings.cmyk.is_some() { format!(" /TrimBox [0 0 {pw} {ph}]") } else { String::new() };
    pdf.object(page, &format!("<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 {pw} {ph}]{trim} /Resources << /XObject << /Im0 {image} 0 R >> >> /Contents {contents} 0 R >>"));
    pdf.stream(contents, "", format!("q\n{pw} 0 0 {ph} 0 0 cm\n/Im0 Do\nQ\n").as_bytes());

    // ---- Image
    let color_space = match (icc, settings.cmyk) {
        (Some(icc), _) => format!("[/ICCBased {icc} 0 R]"),
        (None, Some(_)) => "/DeviceCMYK".to_owned(),
        (None, None) => "/DeviceRGB".to_owned(),
    };
    let mut dict = format!("/Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {color_space} /BitsPerComponent 8");
    if let Some(smask) = smask {
        let _ = write!(dict, " /SMask {smask} 0 R");
    }
    let data = match settings.jpeg_quality {
        Some(quality) => {
            dict.push_str(" /Filter /DCTDecode");
            if components == 4 {
                dict.push_str(" /Decode [1 0 1 0 1 0 1 0]");
                crate::io::encode_jpeg_cmyk(&pixels, width, height, quality, None)?
            } else {
                crate::io::encode_jpeg_rgb(&pixels, width, height, quality, None)?
            }
        }
        None => {
            let _ = write!(dict, " /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors {components} /BitsPerComponent 8 /Columns {width} >>");
            crate::png_parallel::zlib_stream(width, height, &pixels, components, 6)?
        }
    };
    pdf.stream(image, &dict, &data);
    if let (Some(smask), Some(alpha)) = (smask, &alpha) {
        let data = crate::png_parallel::zlib_stream(width, height, alpha, 1, 6)?;
        pdf.stream(
            smask,
            &format!("/Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 1 /BitsPerComponent 8 /Columns {width} >>"),
            &data,
        );
    }

    // ---- Profil et intention de sortie
    if let Some(icc) = icc {
        let (profile, n, alternate) = match settings.cmyk {
            Some(space) => (space.profile.icc_bytes()?, 4, "/DeviceCMYK"),
            None => (std::sync::Arc::from(cmyk::srgb_icc()), 3, "/DeviceRGB"),
        };
        pdf.stream(icc, &format!("/N {n} /Alternate {alternate} /Filter /FlateDecode"), &deflate(&profile));
    }
    if let (Some(intent), Some(icc), Some(space)) = (intent, icc, settings.cmyk) {
        let (name, condition) = (space.profile.display_name()?, space.profile.condition()?);
        pdf.object(
            intent,
            &format!(
                "<< /Type /OutputIntent /S /GTS_PDFX /OutputCondition {} /OutputConditionIdentifier {} /RegistryName (http://www.color.org) /Info {} /DestOutputProfile {icc} 0 R >>",
                literal(&name),
                literal(&condition),
                literal(&name),
            ),
        );
    }

    // ---- Métadonnées
    if let (Some(xmp), Some(meta)) = (xmp, settings.metadata) {
        pdf.stream(xmp, "/Type /Metadata /Subtype /XML", meta.xmp_packet().as_bytes());
    }
    let mut fields = String::from("/Producer (Lumitype)");
    if let Some(meta) = settings.metadata {
        for (key, value) in [("Author", meta.author.clone()), ("Subject", meta.description.clone()), ("Keywords", meta.keywords.join(", ")), ("Creator", meta.software.clone())] {
            if !value.is_empty() {
                let _ = write!(fields, " /{key} {}", text(&value));
            }
        }
        if let Some(date) = pdf_date(&meta.created_iso) {
            let _ = write!(fields, " /CreationDate ({date}) /ModDate ({date})");
        }
    }
    pdf.object(info, &format!("<< {fields} >>"));

    // Identifiant du fichier (requis par PDF/X, utile partout) : empreinte de l'image et de la taille de page.
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&data);
    hasher.update(format!("{pw} {ph} {width} {height}").as_bytes());
    let crc = hasher.finalize();
    let id: String = (0..4).map(|i| format!("{:08X}", crc.rotate_left(i * 8) ^ (data.len() as u32).wrapping_mul(i + 1))).collect();
    Ok(pdf.finish(count, catalog, info, &id))
}

/// PDF/X-4 d'une page : image CMJN avec profil incorporé, boîtes de coupe et de fonds perdus, traits de coupe.
fn encode_x4(width: u32, height: u32, rgba: &[u8], settings: &PdfSettings, print: &PdfPrint) -> Result<Vec<u8>, EngineError> {
    let space = settings.cmyk.unwrap_or_default();
    // Fonds perdus en points et en pixels (l'image couvre toute la page du document).
    let mut bleed = (print.bleed_mm.max(0.0) * 72.0 / 25.4).min(settings.page[0].min(settings.page[1]) / 2.0 - 1.0).max(0.0);
    let (mut w, mut h, mut page) = (width, height, settings.page);
    let cropped;
    let mut rgba = rgba;
    if !print.include_bleed && bleed > 0.0 {
        let bx = ((bleed / settings.page[0]) * width as f32).round() as u32;
        let by = ((bleed / settings.page[1]) * height as f32).round() as u32;
        if bx * 2 < width && by * 2 < height {
            let (cw, ch) = (width - 2 * bx, height - 2 * by);
            let mut out = Vec::with_capacity(cw as usize * ch as usize * 4);
            for row in by..by + ch {
                let start = (row as usize * width as usize + bx as usize) * 4;
                out.extend_from_slice(&rgba[start..start + cw as usize * 4]);
            }
            cropped = out;
            rgba = &cropped;
            (w, h) = (cw, ch);
            page = [page[0] - 2.0 * bleed, page[1] - 2.0 * bleed];
        }
        bleed = 0.0;
    }
    let mut pixels = cmyk::rgba_to_cmyk(rgba, space)?;
    crate::print::apply_black_rules(w, h, rgba, &mut pixels, print.pure_black, print.rich_black, crate::print::mm_to_px(0.5, print.dpi).max(2));
    if print.ink_limit > 0 {
        let max = crate::print::limit_units(print.ink_limit);
        use rayon::prelude::*;
        pixels.par_chunks_exact_mut(4).for_each(|px| {
            let fixed = crate::print::limit_ink([px[0], px[1], px[2], px[3]], max);
            px.copy_from_slice(&fixed);
        });
    }

    let mut pdf = Writer::new();
    let (catalog, pages, page_id, contents, image, icc, intent, xmp, info) = (1, 2, 3, 4, 5, 6, 7, 8, 9);
    let count = 10;
    pdf.object(catalog, &format!("<< /Type /Catalog /Pages {pages} 0 R /OutputIntents [{intent} 0 R] /Metadata {xmp} 0 R >>"));
    pdf.object(pages, &format!("<< /Type /Pages /Kids [{page_id} 0 R] /Count 1 >>"));
    let pad = if print.marks { MARKS_PAD } else { 0.0 };
    let (iw, ih) = (page[0].max(1.0), page[1].max(1.0));
    let (mw, mh) = (iw + 2.0 * pad, ih + 2.0 * pad);
    let n = number;
    let media = format!("[0 0 {} {}]", n(mw), n(mh));
    let bleed_box = format!("[{} {} {} {}]", n(pad), n(pad), n(pad + iw), n(pad + ih));
    let trim_box = format!("[{} {} {} {}]", n(pad + bleed), n(pad + bleed), n(pad + iw - bleed), n(pad + ih - bleed));
    pdf.object(page_id, &format!("<< /Type /Page /Parent {pages} 0 R /MediaBox {media} /BleedBox {bleed_box} /TrimBox {trim_box} /Resources << /XObject << /Im0 {image} 0 R >> >> /Contents {contents} 0 R >>"));
    let mut content = format!("q\n{} 0 0 {} {} {} cm\n/Im0 Do\nQ\n", n(iw), n(ih), n(pad), n(pad));
    if print.marks {
        // Traits de coupe : dans le prolongement des bords du format fini, hors des fonds perdus, en couleur de
        // repérage (les quatre encres) pour apparaître sur chaque plaque.
        let (left, right, bottom, top) = (pad + bleed, pad + iw - bleed, pad + bleed, pad + ih - bleed);
        let o = bleed + MARK_OFFSET;
        let l = MARK_LENGTH.min(pad + bleed - o).max(4.0);
        content.push_str("q\n0.25 w\n1 1 1 1 K\n");
        for (x, y, sx, sy) in [(left, bottom, -1.0, -1.0), (right, bottom, 1.0, -1.0), (left, top, -1.0, 1.0), (right, top, 1.0, 1.0)] {
            let _ = writeln!(content, "{} {} m {} {} l S", n(x + sx * o), n(y), n(x + sx * (o + l)), n(y));
            let _ = writeln!(content, "{} {} m {} {} l S", n(x), n(y + sy * o), n(x), n(y + sy * (o + l)));
        }
        content.push_str("Q\n");
    }
    pdf.stream(contents, "", content.as_bytes());

    let mut dict = format!("/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace [/ICCBased {icc} 0 R] /BitsPerComponent 8");
    let data = match settings.jpeg_quality {
        Some(quality) => {
            dict.push_str(" /Filter /DCTDecode /Decode [1 0 1 0 1 0 1 0]");
            crate::io::encode_jpeg_cmyk(&pixels, w, h, quality, None)?
        }
        None => {
            let _ = write!(dict, " /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 4 /BitsPerComponent 8 /Columns {w} >>");
            crate::png_parallel::zlib_stream(w, h, &pixels, 4, 6)?
        }
    };
    pdf.stream(image, &dict, &data);
    pdf.stream(icc, "/N 4 /Alternate /DeviceCMYK /Filter /FlateDecode", &deflate(&space.profile.icc_bytes()?));
    let (name, condition) = (space.profile.display_name()?, space.profile.condition()?);
    pdf.object(
        intent,
        &format!(
            "<< /Type /OutputIntent /S /GTS_PDFX /OutputCondition {} /OutputConditionIdentifier {} /RegistryName (http://www.color.org) /Info {} /DestOutputProfile {icc} 0 R >>",
            literal(&name),
            literal(&condition),
            literal(&name),
        ),
    );

    let title = if print.title.trim().is_empty() { "Sans titre" } else { print.title.trim() };
    let (pdf_now, iso_now) = now_dates();
    let meta = settings.metadata;
    let mut extra_xmp = String::new();
    let mut fields = format!("/Title {} /Producer (Lumitype) /Creator (Lumitype) /Trapped /False /GTS_PDFXVersion (PDF/X-4) /CreationDate ({pdf_now}) /ModDate ({pdf_now})", text(title));
    if let Some(meta) = meta {
        for (key, value) in [("Author", meta.author.clone()), ("Subject", meta.description.clone()), ("Keywords", meta.keywords.join(", "))] {
            if !value.is_empty() {
                let _ = write!(fields, " /{key} {}", text(&value));
            }
        }
        if !meta.author.is_empty() {
            let _ = write!(extra_xmp, "<dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>", xml(&meta.author));
        }
        if !meta.description.is_empty() {
            let _ = write!(extra_xmp, "<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>", xml(&meta.description));
        }
    }
    let packet = format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Lumitype\">\
<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\" xmlns:xmpMM=\"http://ns.adobe.com/xap/1.0/mm/\">\
<dc:format>application/pdf</dc:format>\
<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>{extra_xmp}\
<xmp:CreateDate>{iso_now}</xmp:CreateDate><xmp:ModifyDate>{iso_now}</xmp:ModifyDate><xmp:MetadataDate>{iso_now}</xmp:MetadataDate>\
<xmp:CreatorTool>Lumitype</xmp:CreatorTool>\
<pdf:Producer>Lumitype</pdf:Producer><pdf:Trapped>False</pdf:Trapped>\
<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>\
<xmpMM:DocumentID>uuid:{}</xmpMM:DocumentID><xmpMM:RenditionClass>default</xmpMM:RenditionClass><xmpMM:VersionID>1</xmpMM:VersionID>\
</rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>",
        xml(title),
        pseudo_uuid(&data, &iso_now),
    );
    pdf.stream(xmp, "/Type /Metadata /Subtype /XML", packet.as_bytes());
    pdf.object(info, &format!("<< {fields} >>"));

    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&data);
    hasher.update(format!("{mw} {mh} {w} {h}").as_bytes());
    let crc = hasher.finalize();
    let id: String = (0..4).map(|i| format!("{:08X}", crc.rotate_left(i * 8) ^ (data.len() as u32).wrapping_mul(i + 1))).collect();
    Ok(pdf.finish(count, catalog, info, &id))
}

/// Date et heure courantes (UTC) : forme PDF (`D:AAAAMMJJHHMMSSZ`) et forme XMP (`AAAA-MM-JJTHH:MM:SSZ`), identiques
/// comme l'exige PDF/X.
fn now_dates() -> (String, String) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Jours depuis 1970 → date civile (algorithme de Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (hh, mm, ss) = (rem / 3600, rem % 3600 / 60, rem % 60);
    (format!("D:{y:04}{m:02}{d:02}{hh:02}{mm:02}{ss:02}Z"), format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z"))
}

/// Identifiant unique de document (forme UUID) tiré du contenu et de la date.
fn pseudo_uuid(data: &[u8], date: &str) -> String {
    let mut a = crc32fast::Hasher::new();
    a.update(data);
    let x = a.finalize();
    let mut b = crc32fast::Hasher::new();
    b.update(date.as_bytes());
    b.update(&x.to_le_bytes());
    let y = b.finalize();
    format!("{x:08x}-{:04x}-4{:03x}-a{:03x}-{y:08x}{:04x}", y >> 16, x & 0xfff, y & 0xfff, (x ^ y) & 0xffff)
}

fn xml(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// PDF en niveaux de gris (`DeviceGray`) : 8 bits avec masque de transparence éventuel, ou 1 bit (noir et blanc).
fn encode_gray(width: u32, height: u32, rgba: &[u8], settings: &PdfSettings) -> Result<Vec<u8>, EngineError> {
    let one_bit = settings.gray_bits == Some(1);
    let mut pdf = Writer::new();
    let (catalog, pages, page, contents, image) = (1, 2, 3, 4, 5);
    let mut next = 6;
    let mut take = || {
        next += 1;
        next - 1
    };
    let lossy = settings.jpeg_quality.filter(|_| !one_bit);
    let (gray, alpha) = if one_bit { (Vec::new(), None) } else { crate::color_mode::gray_planes(rgba) };
    let smask = alpha.as_ref().map(|_| take());
    let xmp = settings.metadata.filter(|m| !m.is_empty()).map(|_| take());
    let info = take();
    let count = next;
    let extra = xmp.map(|xmp| format!(" /Metadata {xmp} 0 R")).unwrap_or_default();
    pdf.object(catalog, &format!("<< /Type /Catalog /Pages {pages} 0 R{extra} >>"));
    pdf.object(pages, &format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>"));
    let (pw, ph) = (number(settings.page[0].max(1.0)), number(settings.page[1].max(1.0)));
    pdf.object(page, &format!("<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 {pw} {ph}] /Resources << /XObject << /Im0 {image} 0 R >> >> /Contents {contents} 0 R >>"));
    pdf.stream(contents, "", format!("q\n{pw} 0 0 {ph} 0 0 cm\n/Im0 Do\nQ\n").as_bytes());
    let data = if one_bit {
        // Lignes de bits (1 = blanc) : prédicteurs PNG sur des octets entiers, comme pour un PNG 1 bit.
        let packed = crate::color_mode::pack_bits_rows(&crate::color_mode::bilevel(rgba), width, height);
        let data = crate::png_parallel::zlib_stream(width.div_ceil(8), height, &packed, 1, 6)?;
        pdf.stream(image, &format!("/Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 1 /BitsPerComponent 1 /Columns {width} >>"), &data);
        data
    } else {
        let mut dict = format!("/Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceGray /BitsPerComponent 8");
        if let Some(smask) = smask {
            let _ = write!(dict, " /SMask {smask} 0 R");
        }
        let data = match lossy {
            Some(quality) => {
                dict.push_str(" /Filter /DCTDecode");
                // Pixels entièrement transparents en blanc, comme en RVB : pas de halo sombre au bord du détourage.
                let gray: Vec<u8> = gray.iter().zip(rgba.chunks_exact(4)).map(|(g, px)| if px[3] == 0 { 255 } else { *g }).collect();
                crate::io::encode_jpeg_gray(&gray, width, height, quality)?
            }
            None => {
                let _ = write!(dict, " /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 1 /BitsPerComponent 8 /Columns {width} >>");
                crate::png_parallel::zlib_stream(width, height, &gray, 1, 6)?
            }
        };
        pdf.stream(image, &dict, &data);
        if let (Some(smask), Some(alpha)) = (smask, &alpha) {
            let mask = crate::png_parallel::zlib_stream(width, height, alpha, 1, 6)?;
            pdf.stream(smask, &format!("/Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 1 /BitsPerComponent 8 /Columns {width} >>"), &mask);
        }
        data
    };
    if let (Some(xmp), Some(meta)) = (xmp, settings.metadata) {
        pdf.stream(xmp, "/Type /Metadata /Subtype /XML", meta.xmp_packet().as_bytes());
    }
    let mut fields = String::from("/Producer (Lumitype)");
    if let Some(meta) = settings.metadata {
        for (key, value) in [("Author", meta.author.clone()), ("Subject", meta.description.clone()), ("Keywords", meta.keywords.join(", ")), ("Creator", meta.software.clone())] {
            if !value.is_empty() {
                let _ = write!(fields, " /{key} {}", text(&value));
            }
        }
        if let Some(date) = pdf_date(&meta.created_iso) {
            let _ = write!(fields, " /CreationDate ({date}) /ModDate ({date})");
        }
    }
    pdf.object(info, &format!("<< {fields} >>"));
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&data);
    hasher.update(format!("{pw} {ph} {width} {height}").as_bytes());
    let crc = hasher.finalize();
    let id: String = (0..4).map(|i| format!("{:08X}", crc.rotate_left(i * 8) ^ (data.len() as u32).wrapping_mul(i + 1))).collect();
    Ok(pdf.finish(count, catalog, info, &id))
}

/// Écriture des objets numérotés et de la table des positions (xref).
struct Writer {
    out: Vec<u8>,
    /// Position de chaque objet, par numéro (0 : objet libre de tête).
    offsets: Vec<usize>,
}

impl Writer {
    fn new() -> Self {
        // L'en-tête, puis une ligne de commentaire à octets hauts : le fichier est reconnu comme binaire.
        let mut out = Vec::with_capacity(1 << 16);
        out.extend_from_slice(b"%PDF-1.6\n%\xE2\xE3\xCF\xD3\n");
        Self { out, offsets: Vec::new() }
    }

    fn begin(&mut self, id: usize) {
        if self.offsets.len() <= id {
            self.offsets.resize(id + 1, 0);
        }
        self.offsets[id] = self.out.len();
        self.out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
    }

    fn object(&mut self, id: usize, body: &str) {
        self.begin(id);
        self.out.extend_from_slice(body.as_bytes());
        self.out.extend_from_slice(b"\nendobj\n");
    }

    /// Flux : `dict` sans les chevrons ni la longueur, ajoutée ici.
    fn stream(&mut self, id: usize, dict: &str, data: &[u8]) {
        self.begin(id);
        let space = if dict.is_empty() { "" } else { " " };
        self.out.extend_from_slice(format!("<< {dict}{space}/Length {} >>\nstream\n", data.len()).as_bytes());
        self.out.extend_from_slice(data);
        self.out.extend_from_slice(b"\nendstream\nendobj\n");
    }

    fn finish(mut self, count: usize, root: usize, info: usize, id: &str) -> Vec<u8> {
        self.offsets.resize(count, 0);
        let xref = self.out.len();
        let mut table = format!("xref\n0 {count}\n0000000000 65535 f \n");
        for offset in &self.offsets[1..] {
            let _ = writeln!(table, "{offset:010} 00000 n ");
        }
        let _ = write!(table, "trailer\n<< /Size {count} /Root {root} 0 R /Info {info} 0 R /ID [<{id}> <{id}>] >>\nstartxref\n{xref}\n%%EOF\n");
        self.out.extend_from_slice(table.as_bytes());
        self.out
    }
}

/// Nombre PDF : au plus trois décimales, sans zéros inutiles.
fn number(value: f32) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() { "0".to_owned() } else { text.to_owned() }
}

/// Chaîne littérale ASCII, parenthèses et barres obliques inverses échappées.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('(');
    for c in text.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            _ => out.push('?'),
        }
    }
    out.push(')');
    out
}

/// Chaîne de texte quelconque (accents, émojis…) : UTF-16 gros-boutiste avec marque d'ordre, en hexadécimal.
fn text(value: &str) -> String {
    let mut out = String::from("<FEFF");
    for unit in value.encode_utf16() {
        let _ = write!(out, "{unit:04X}");
    }
    out.push('>');
    out
}

/// Date ISO (`AAAA-MM-JJTHH:MM:SS`) → date PDF (`D:AAAAMMJJHHMMSS`).
fn pdf_date(iso: &str) -> Option<String> {
    let digits: String = iso.chars().filter(char::is_ascii_digit).take(14).collect();
    (digits.len() >= 8).then(|| format!("D:{digits}"))
}

fn deflate(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::with_capacity(data.len() / 2), flate2::Compression::best());
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmyk::CmykProfile;

    fn sample(width: u32, height: u32, alpha: bool) -> Vec<u8> {
        (0..width * height)
            .flat_map(|i| {
                let (x, y) = (i % width, i / width);
                let a = if alpha && x < width / 4 { 0 } else { 255 };
                [(x * 255 / width) as u8, (y * 255 / height) as u8, 128, a]
            })
            .collect()
    }

    fn settings(cmyk: Option<CmykSpace>, embed: bool, jpeg: Option<u8>) -> PdfSettings<'static> {
        PdfSettings { page: [595.276, 841.89], cmyk, embed_profile: embed, jpeg_quality: jpeg, metadata: None, gray_bits: None, print: None }
    }

    #[test]
    fn gray_and_bitmap_pdfs_use_device_gray() {
        let rgba = sample(19, 7, true);
        for (bits, jpeg) in [(8u8, None), (8, Some(80)), (1, None), (1, Some(80))] {
            let pdf = encode(19, 7, &rgba, &PdfSettings { gray_bits: Some(bits), ..settings(None, true, jpeg) }).unwrap();
            check_structure(&pdf);
            assert!(find(&pdf, b"/ColorSpace /DeviceGray").is_some() && find(&pdf, b"/DeviceRGB").is_none() && find(&pdf, b"/ICCBased").is_none());
            assert_eq!(find(&pdf, format!("/BitsPerComponent {bits} /Filter").as_bytes()).is_some(), true, "{bits} bits");
            assert_eq!(find(&pdf, b"/DCTDecode").is_some(), bits == 8 && jpeg.is_some(), "le 1 bit reste sans perte");
        }
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    /// Vérifie l'en-tête, la fin, et que chaque entrée de la table xref pointe sur « N 0 obj ».
    fn check_structure(pdf: &[u8]) {
        assert!(pdf.starts_with(b"%PDF-1.6\n"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let tail = std::str::from_utf8(&pdf[pdf.len() - 40..]).unwrap();
        let start: usize = tail.split("startxref\n").nth(1).unwrap().lines().next().unwrap().parse().unwrap();
        assert!(pdf[start..].starts_with(b"xref\n"));
        let table = std::str::from_utf8(&pdf[start..]).unwrap();
        let mut lines = table.lines().skip(1);
        let count: usize = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
        assert_eq!(lines.next().unwrap(), "0000000000 65535 f ");
        for id in 1..count {
            let line = lines.next().unwrap();
            assert_eq!(line.len(), 19, "entrée de 20 octets avec la fin de ligne");
            let offset: usize = line[..10].parse().unwrap();
            assert!(pdf[offset..].starts_with(format!("{id} 0 obj\n").as_bytes()), "objet {id}");
        }
        assert!(table.contains(&format!("/Size {count} ")));
    }

    #[test]
    fn every_variant_is_a_well_formed_pdf() {
        let rgba = sample(37, 21, true);
        for cmyk in [None, Some(CmykProfile::Fogra39.into()), Some(CmykProfile::Swop2006c3.into())] {
            for embed in [false, true] {
                for jpeg in [None, Some(92), Some(60)] {
                    let pdf = encode(37, 21, &rgba, &settings(cmyk, embed, jpeg)).unwrap();
                    check_structure(&pdf);
                    assert_eq!(find(&pdf, b"/DCTDecode").is_some(), jpeg.is_some());
                    assert_eq!(find(&pdf, b"/DeviceCMYK").is_some(), cmyk.is_some());
                    assert_eq!(find(&pdf, b"/SMask").is_some(), cmyk.is_none(), "masque en RVB seulement");
                    assert_eq!(find(&pdf, b"/MediaBox [0 0 595.276 841.89]").is_some(), true);
                }
            }
        }
    }

    #[test]
    fn icc_profile_is_embedded_only_when_asked() {
        let rgba = sample(16, 16, false);
        let with = encode(16, 16, &rgba, &settings(Some(CmykProfile::Fogra39.into()), true, Some(90))).unwrap();
        let without = encode(16, 16, &rgba, &settings(Some(CmykProfile::Fogra39.into()), false, Some(90))).unwrap();
        assert!(find(&with, b"/ICCBased").is_some() && find(&with, b"/GTS_PDFX").is_some() && find(&with, b"/DestOutputProfile").is_some());
        assert!(find(&with, b"(FOGRA39)").is_some());
        assert!(find(&without, b"/ICCBased").is_none() && find(&without, b"/OutputIntents").is_none());
        assert!(with.len() > without.len() + 30_000, "profil compressé incorporé");
        // Le profil incorporé se relit tel quel.
        let start = find(&with, b"/N 4 /Alternate /DeviceCMYK /Filter /FlateDecode /Length ").unwrap();
        let stream = find(&with[start..], b"stream\n").unwrap() + start + 7;
        let mut decoded = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&with[stream..]), &mut decoded).unwrap();
        assert_eq!(&decoded[..], &*CmykProfile::Fogra39.icc_bytes().unwrap());

        let rgb = encode(16, 16, &rgba, &settings(None, true, None)).unwrap();
        assert!(find(&rgb, b"/N 3 /Alternate /DeviceRGB").is_some() && find(&rgb, b"/OutputIntents").is_none());
        let rgb_plain = encode(16, 16, &rgba, &settings(None, false, None)).unwrap();
        assert!(find(&rgb_plain, b"/ColorSpace /DeviceRGB").is_some() && find(&rgb_plain, b"/SMask").is_none(), "image opaque : pas de masque");
    }

    #[test]
    fn pdf_x4_declares_its_boxes_its_version_and_limits_the_ink() {
        let (w, h) = (91u32, 61u32);
        let rgba: Vec<u8> = (0..w * h).flat_map(|_| [0u8, 0, 0, 255]).collect();
        let print = PdfPrint { bleed_mm: 3.0, include_bleed: true, marks: true, ink_limit: 300, title: "Carte".into(), pure_black: true, rich_black: true, dpi: 25.4 };
        let page = [91.0 * 72.0 / 25.4, 61.0 * 72.0 / 25.4];
        let pdf = encode(w, h, &rgba, &PdfSettings { page, print: Some(&print), ..settings(Some(CmykProfile::Fogra39.into()), true, None) }).unwrap();
        check_structure(&pdf);
        for needle in [&b"/BleedBox [20 20 "[..], b"/TrimBox [28.504 28.504 ", b"<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>", b"/Trapped /False", b"/GTS_PDFX ", b"/ICCBased", b"1 1 1 1 K", b"/MediaBox [0 0 297.953 212.913]"] {
            assert!(find(&pdf, needle).is_some(), "{}", String::from_utf8_lossy(needle));
        }
        // Encrage : la séparation limitée ne dépasse jamais 300 %.
        let limited = crate::print::limit_ink(crate::cmyk::rgb_to_cmyk_pixel(CmykProfile::Fogra39.into(), 0, 0, 0).unwrap(), crate::print::limit_units(300));
        assert!(crate::print::ink_total(limited) <= crate::print::limit_units(300));
        // Sans fonds perdus : rognée au format fini, boîtes confondues.
        let print = PdfPrint { include_bleed: false, marks: false, ..print };
        let pdf = encode(w, h, &rgba, &PdfSettings { page, print: Some(&print), ..settings(Some(CmykProfile::Fogra39.into()), true, None) }).unwrap();
        check_structure(&pdf);
        assert!(find(&pdf, b"/Width 85 /Height 55").is_some());
        assert!(find(&pdf, b"/MediaBox [0 0 240.945 155.906] /BleedBox [0 0 240.945 155.906] /TrimBox [0 0 240.945 155.906]").is_some());
    }

    #[test]
    fn metadata_goes_to_info_and_xmp() {
        let meta = ImageMetadata { author: "Élodie".into(), description: "Affiche".into(), created_iso: "2026-10-03T02:00:00".into(), ..ImageMetadata::default() };
        let pdf = encode(8, 8, &sample(8, 8, false), &PdfSettings { metadata: Some(&meta), ..settings(None, false, None) }).unwrap();
        check_structure(&pdf);
        assert!(find(&pdf, b"/Author <FEFF00C9006C").is_some());
        assert!(find(&pdf, b"/CreationDate (D:20261003020000)").is_some());
        assert!(find(&pdf, b"/Type /Metadata /Subtype /XML").is_some());
    }
}

