//! Métadonnées inscrites dans les images exportées : auteur, copyright, description, mots-clés, logiciel, date.
//!
//! Chaque format a ses conteneurs ; on écrit partout ce qu'il sait porter, après l'encodage des pixels :
//! - PNG : morceaux `iTXt` (UTF-8) aux mots-clés normalisés (Author, Copyright, Description, Software, Creation Time)
//!   et un paquet XMP (`XML:com.adobe.xmp`), insérés juste après `IHDR` ;
//! - JPEG : segments APP1 EXIF (IFD0 : ImageDescription, Software, DateTime, Artist, Copyright) et APP1 XMP,
//!   insérés après le segment JFIF ;
//! - WebP : conteneur étendu (`VP8X` avec les drapeaux EXIF et XMP), morceaux `EXIF` et `XMP ` après l'image ;
//! - TIFF : les mêmes entrées EXIF ajoutées à l'IFD principal (réécrit en fin de fichier), plus le XMP (tag 700) ;
//!   la réécriture d'IFD sert aussi, hors métadonnées, à déclarer l'alpha du TIFF (`tiff_with_unassociated_alpha`).
//!
//! Le XMP porte aussi ce que l'EXIF n'a pas : les mots-clés (`dc:subject`), le site ou contact
//! (`xmpRights:WebStatement`) et la date au format ISO.
use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::io::ExportFormat;

/// Ce qui est inscrit dans une image exportée. Les champs vides ne sont pas écrits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageMetadata {
    /// Auteur (prénom et nom) : EXIF Artist, PNG Author, XMP dc:creator.
    pub author: String,
    /// Mention de copyright : EXIF Copyright, PNG Copyright, XMP dc:rights.
    pub copyright: String,
    /// Description : EXIF ImageDescription, PNG Description, XMP dc:description.
    pub description: String,
    /// Mots-clés : XMP dc:subject (et PNG Keywords).
    pub keywords: Vec<String>,
    /// Site ou contact : XMP xmpRights:WebStatement.
    pub contact: String,
    /// Logiciel : EXIF Software, PNG Software, XMP xmp:CreatorTool.
    pub software: String,
    /// Date de création, forme EXIF `AAAA:MM:JJ HH:MM:SS` (vide : non écrite).
    pub created: String,
    /// Date de création, forme ISO 8601 pour le XMP (`AAAA-MM-JJTHH:MM:SS`).
    pub created_iso: String,
}

impl ImageMetadata {
    pub fn is_empty(&self) -> bool {
        self.author.is_empty() && self.copyright.is_empty() && self.description.is_empty() && self.keywords.is_empty() && self.contact.is_empty() && self.software.is_empty() && self.created.is_empty()
    }

    /// Entrées EXIF de l'IFD principal : (tag, texte), triées par tag comme l'exige le TIFF.
    fn exif_entries(&self) -> Vec<(u16, &str)> {
        let mut out = Vec::new();
        if !self.description.is_empty() {
            out.push((0x010E, self.description.as_str()));
        }
        if !self.software.is_empty() {
            out.push((0x0131, self.software.as_str()));
        }
        if !self.created.is_empty() {
            out.push((0x0132, self.created.as_str()));
        }
        if !self.author.is_empty() {
            out.push((0x013B, self.author.as_str()));
        }
        if !self.copyright.is_empty() {
            out.push((0x8298, self.copyright.as_str()));
        }
        out
    }

    /// Bloc EXIF complet (en-tête TIFF petit-boutiste, IFD0, chaînes), sans le préfixe `Exif\0\0` du JPEG.
    pub fn exif_tiff(&self) -> Vec<u8> {
        let entries = self.exif_entries();
        let mut out = Vec::with_capacity(64 + entries.len() * 40);
        out.extend_from_slice(b"II\x2A\x00");
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        // Les chaînes suivent l'IFD : 2 (compte) + 12 par entrée + 4 (IFD suivante).
        let mut data_offset = 8 + 2 + entries.len() * 12 + 4;
        let mut data = Vec::new();
        for (tag, text) in &entries {
            let mut bytes = text.as_bytes().to_vec();
            bytes.push(0);
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&2u16.to_le_bytes()); // ASCII
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            if bytes.len() <= 4 {
                let mut inline = [0u8; 4];
                inline[..bytes.len()].copy_from_slice(&bytes);
                out.extend_from_slice(&inline);
            } else {
                out.extend_from_slice(&(data_offset as u32).to_le_bytes());
                data_offset += bytes.len();
                data.extend_from_slice(&bytes);
            }
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    /// Paquet XMP (RDF/XML), UTF-8.
    pub fn xmp_packet(&self) -> String {
        let mut body = String::new();
        if !self.author.is_empty() {
            body.push_str(&format!("<dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>", escape(&self.author)));
        }
        if !self.copyright.is_empty() {
            body.push_str(&format!("<dc:rights><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:rights>", escape(&self.copyright)));
            body.push_str("<xmpRights:Marked>True</xmpRights:Marked>");
        }
        if !self.description.is_empty() {
            body.push_str(&format!("<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>", escape(&self.description)));
        }
        if !self.keywords.is_empty() {
            let items: String = self.keywords.iter().map(|k| format!("<rdf:li>{}</rdf:li>", escape(k))).collect();
            body.push_str(&format!("<dc:subject><rdf:Bag>{items}</rdf:Bag></dc:subject>"));
        }
        if !self.contact.is_empty() {
            body.push_str(&format!("<xmpRights:WebStatement>{}</xmpRights:WebStatement>", escape(&self.contact)));
        }
        if !self.software.is_empty() {
            body.push_str(&format!("<xmp:CreatorTool>{}</xmp:CreatorTool>", escape(&self.software)));
        }
        if !self.created_iso.is_empty() {
            body.push_str(&format!("<xmp:CreateDate>{0}</xmp:CreateDate><xmp:MetadataDate>{0}</xmp:MetadataDate>", escape(&self.created_iso)));
        }
        format!(
            "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Lumitype\">\
<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmlns:xmpRights=\"http://ns.adobe.com/xap/1.0/rights/\">\
{body}\
</rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>"
        )
    }

    /// Morceaux texte PNG (mot-clé, texte), hors XMP.
    fn png_texts(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if !self.author.is_empty() {
            out.push(("Author", self.author.clone()));
        }
        if !self.copyright.is_empty() {
            out.push(("Copyright", self.copyright.clone()));
        }
        if !self.description.is_empty() {
            out.push(("Description", self.description.clone()));
        }
        if !self.keywords.is_empty() {
            out.push(("Keywords", self.keywords.join(", ")));
        }
        if !self.contact.is_empty() {
            out.push(("Source", self.contact.clone()));
        }
        if !self.software.is_empty() {
            out.push(("Software", self.software.clone()));
        }
        if !self.created_iso.is_empty() {
            out.push(("Creation Time", self.created_iso.clone()));
        }
        out
    }
}

/// Inscrit les métadonnées dans une image déjà encodée. Sans métadonnée, les octets sont rendus tels quels.
pub fn embed(format: ExportFormat, bytes: Vec<u8>, metadata: &ImageMetadata) -> Result<Vec<u8>, EngineError> {
    if metadata.is_empty() {
        return Ok(bytes);
    }
    match format {
        ExportFormat::Png => Ok(embed_png(bytes, metadata)),
        ExportFormat::Jpeg => embed_jpeg(bytes, metadata),
        ExportFormat::Webp => embed_webp(bytes, metadata),
        ExportFormat::Tiff => embed_tiff(bytes, metadata),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn corrupt(what: &str) -> EngineError {
    EngineError::Corrupt(format!("métadonnées : {what}"))
}

// ------------------------------------------------------------------ PNG

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(kind);
    hasher.update(data);
    out.extend_from_slice(&hasher.finalize().to_be_bytes());
}

/// `iTXt` : mot-clé, drapeau de compression (0), méthode (0), étiquette de langue, mot-clé traduit, texte UTF-8.
fn itxt(keyword: &str, text: &str) -> Vec<u8> {
    let mut data = Vec::with_capacity(keyword.len() + text.len() + 5);
    data.extend_from_slice(keyword.as_bytes());
    data.extend_from_slice(&[0, 0, 0, 0, 0]);
    data.extend_from_slice(text.as_bytes());
    data
}

fn embed_png(bytes: Vec<u8>, metadata: &ImageMetadata) -> Vec<u8> {
    // Signature (8) + IHDR (4 + 4 + 13 + 4) : les textes s'insèrent juste après, avant les données.
    const AFTER_IHDR: usize = 8 + 25;
    if bytes.len() < AFTER_IHDR || &bytes[12..16] != b"IHDR" {
        return bytes;
    }
    let mut out = Vec::with_capacity(bytes.len() + 2048);
    out.extend_from_slice(&bytes[..AFTER_IHDR]);
    for (keyword, text) in metadata.png_texts() {
        png_chunk(&mut out, b"iTXt", &itxt(keyword, &text));
    }
    png_chunk(&mut out, b"iTXt", &itxt("XML:com.adobe.xmp", &metadata.xmp_packet()));
    out.extend_from_slice(&bytes[AFTER_IHDR..]);
    out
}

// ------------------------------------------------------------------ JPEG

fn embed_jpeg(bytes: Vec<u8>, metadata: &ImageMetadata) -> Result<Vec<u8>, EngineError> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return Err(corrupt("JPEG inattendu"));
    }
    // Après SOI, et après un éventuel APP0 (JFIF) : l'EXIF se place avant tout le reste.
    let mut at = 2;
    if bytes.len() >= at + 4 && bytes[at] == 0xFF && bytes[at + 1] == 0xE0 {
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        at += 2 + length;
        if at > bytes.len() {
            return Err(corrupt("segment JFIF tronqué"));
        }
    }
    let mut segments = Vec::new();
    let exif = {
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend(metadata.exif_tiff());
        payload
    };
    segments.push(exif);
    let mut xmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    xmp.extend_from_slice(metadata.xmp_packet().as_bytes());
    segments.push(xmp);
    let mut out = Vec::with_capacity(bytes.len() + 4096);
    out.extend_from_slice(&bytes[..at]);
    for payload in segments {
        if payload.len() + 2 > 65_535 {
            continue; // trop long pour un segment : omis plutôt que de corrompre le fichier
        }
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&payload);
    }
    out.extend_from_slice(&bytes[at..]);
    Ok(out)
}

// ------------------------------------------------------------------ WebP

fn riff_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

fn embed_webp(bytes: Vec<u8>, metadata: &ImageMetadata) -> Result<Vec<u8>, EngineError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(corrupt("WebP inattendu"));
    }
    // Morceaux existants (VP8, VP8L, VP8X, ALPH…), dans l'ordre.
    let mut chunks: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let kind: [u8; 4] = bytes[at..at + 4].try_into().expect("quatre octets");
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().expect("quatre octets")) as usize;
        let end = at + 8 + size;
        if end > bytes.len() {
            return Err(corrupt("morceau WebP tronqué"));
        }
        chunks.push((kind, bytes[at + 8..end].to_vec()));
        at = end + (size % 2);
    }
    let (width, height, alpha) = webp_geometry(&chunks)?;
    let mut flags = 0u8;
    let mut body: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for (kind, data) in chunks {
        match &kind {
            b"VP8X" => {
                flags |= data.first().copied().unwrap_or(0);
            }
            b"EXIF" | b"XMP " => {}
            _ => body.push((kind, data)),
        }
    }
    if alpha {
        flags |= 0x10;
    }
    flags |= 0x08 | 0x04; // EXIF et XMP
    let mut vp8x = vec![flags, 0, 0, 0];
    vp8x.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    vp8x.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
    let mut content = Vec::with_capacity(bytes.len() + 4096);
    content.extend_from_slice(b"WEBP");
    riff_chunk(&mut content, b"VP8X", &vp8x);
    for (kind, data) in &body {
        riff_chunk(&mut content, kind, data);
    }
    riff_chunk(&mut content, b"EXIF", &metadata.exif_tiff());
    riff_chunk(&mut content, b"XMP ", metadata.xmp_packet().as_bytes());
    let mut out = Vec::with_capacity(content.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(content.len() as u32).to_le_bytes());
    out.extend_from_slice(&content);
    Ok(out)
}

/// Largeur, hauteur et présence d'alpha d'un WebP, lues dans ses morceaux.
fn webp_geometry(chunks: &[([u8; 4], Vec<u8>)]) -> Result<(u32, u32, bool), EngineError> {
    for (kind, data) in chunks {
        match kind {
            b"VP8X" if data.len() >= 10 => {
                let w = u32::from_le_bytes([data[4], data[5], data[6], 0]) + 1;
                let h = u32::from_le_bytes([data[7], data[8], data[9], 0]) + 1;
                return Ok((w, h, data[0] & 0x10 != 0));
            }
            b"VP8L" if data.len() >= 5 && data[0] == 0x2F => {
                let bits = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                let w = (bits & 0x3FFF) + 1;
                let h = ((bits >> 14) & 0x3FFF) + 1;
                let alpha = (bits >> 28) & 1 == 1;
                return Ok((w, h, alpha));
            }
            // Flux avec perte : après l'étiquette d'image (3 octets) et le code de départ 9D 01 2A, largeur et
            // hauteur sur 14 bits (les 2 bits hauts sont une échelle, ignorée). L'alpha vit dans un morceau `ALPH`.
            b"VP8 " if data.len() >= 10 && data[3..6] == [0x9D, 0x01, 0x2A] => {
                let w = u32::from(u16::from_le_bytes([data[6], data[7]]) & 0x3FFF);
                let h = u32::from(u16::from_le_bytes([data[8], data[9]]) & 0x3FFF);
                let alpha = chunks.iter().any(|(kind, _)| kind == b"ALPH");
                return Ok((w, h, alpha));
            }
            _ => {}
        }
    }
    Err(corrupt("dimensions WebP introuvables"))
}

// ------------------------------------------------------------------ TIFF

/// Valeur d'une entrée d'IFD TIFF.
pub(crate) enum TiffValue {
    /// Type ASCII (2) : chaîne terminée par un zéro.
    Ascii(String),
    /// Type BYTE (1) : octets quelconques (XMP, par exemple).
    Bytes(Vec<u8>),
    /// Type SHORT (3) : un entier 16 bits.
    Short(u16),
}

/// Réécrit l'IFD principal d'un TIFF avec des entrées en plus (une entrée existante de même tag est remplacée) et
/// sans celles de `drop`. Le nouvel IFD et ses données sont ajoutés en fin de fichier ; l'en-tête pointe dessus,
/// l'ancien reste, inoffensif.
pub(crate) fn tiff_with_entries(mut bytes: Vec<u8>, extra: Vec<(u16, TiffValue)>, drop: &[u16]) -> Result<Vec<u8>, EngineError> {
    if bytes.len() < 8 {
        return Err(corrupt("TIFF inattendu"));
    }
    let little = match &bytes[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Err(corrupt("TIFF inattendu")),
    };
    let u16_at = |b: &[u8], at: usize| -> u16 { if little { u16::from_le_bytes([b[at], b[at + 1]]) } else { u16::from_be_bytes([b[at], b[at + 1]]) } };
    let u32_at = |b: &[u8], at: usize| -> u32 {
        let v = [b[at], b[at + 1], b[at + 2], b[at + 3]];
        if little { u32::from_le_bytes(v) } else { u32::from_be_bytes(v) }
    };
    let put_u16 = |v: u16| if little { v.to_le_bytes() } else { v.to_be_bytes() };
    let put_u32 = |v: u32| if little { v.to_le_bytes() } else { v.to_be_bytes() };
    let ifd = u32_at(&bytes, 4) as usize;
    if ifd + 2 > bytes.len() {
        return Err(corrupt("IFD TIFF introuvable"));
    }
    let count = usize::from(u16_at(&bytes, ifd));
    let entries_end = ifd + 2 + count * 12;
    if entries_end + 4 > bytes.len() {
        return Err(corrupt("IFD TIFF tronqué"));
    }
    let next_ifd = u32_at(&bytes, entries_end);
    // Entrées existantes (tag, 12 octets), sans celles qu'on réécrit.
    let mut entries: Vec<(u16, [u8; 12])> = (0..count)
        .map(|i| {
            let at = ifd + 2 + i * 12;
            (u16_at(&bytes, at), bytes[at..at + 12].try_into().expect("douze octets"))
        })
        .collect();
    let new_tags: Vec<u16> = extra.iter().map(|(tag, _)| *tag).collect();
    entries.retain(|(tag, _)| !new_tags.contains(tag) && !drop.contains(tag));
    // Données des nouvelles entrées, ajoutées en fin de fichier (alignées sur un mot).
    if bytes.len() % 2 == 1 {
        bytes.push(0);
    }
    for (tag, value) in extra {
        let (kind, count, data): (u16, u32, Vec<u8>) = match value {
            TiffValue::Ascii(text) => {
                let mut data = text.into_bytes();
                data.push(0);
                (2, data.len() as u32, data)
            }
            TiffValue::Bytes(data) => (1, data.len() as u32, data),
            TiffValue::Short(v) => (3, 1, put_u16(v).to_vec()),
        };
        let mut entry = [0u8; 12];
        entry[..2].copy_from_slice(&put_u16(tag));
        entry[2..4].copy_from_slice(&put_u16(kind));
        entry[4..8].copy_from_slice(&put_u32(count));
        if data.len() <= 4 {
            entry[8..8 + data.len()].copy_from_slice(&data);
        } else {
            entry[8..12].copy_from_slice(&put_u32(bytes.len() as u32));
            bytes.extend_from_slice(&data);
            if bytes.len() % 2 == 1 {
                bytes.push(0);
            }
        }
        entries.push((tag, entry));
    }
    entries.sort_by_key(|(tag, _)| *tag);
    let new_ifd = bytes.len();
    bytes.extend_from_slice(&put_u16(entries.len() as u16));
    for (_, entry) in &entries {
        bytes.extend_from_slice(entry);
    }
    bytes.extend_from_slice(&put_u32(next_ifd));
    bytes[4..8].copy_from_slice(&put_u32(new_ifd as u32));
    Ok(bytes)
}

/// Tag ExtraSamples (338) = 2, alpha non associé : l'encodeur TIFF de la bibliothèque `image` écrit quatre
/// composantes sans dire ce qu'est la quatrième ; libtiff (Photoshop, GIMP, ImageMagick…) s'en plaint et devine.
/// Il écrit aussi un tag Predictor (317) = 1 sans compression, que libtiff signale comme inconnu : retiré.
pub fn tiff_with_unassociated_alpha(bytes: Vec<u8>) -> Result<Vec<u8>, EngineError> {
    tiff_with_entries(bytes, vec![(338, TiffValue::Short(2))], &[317])
}

fn embed_tiff(bytes: Vec<u8>, metadata: &ImageMetadata) -> Result<Vec<u8>, EngineError> {
    let mut extra: Vec<(u16, TiffValue)> = metadata.exif_entries().into_iter().map(|(tag, text)| (tag, TiffValue::Ascii(text.to_owned()))).collect();
    extra.push((700, TiffValue::Bytes(metadata.xmp_packet().into_bytes())));
    tiff_with_entries(bytes, extra, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ImageMetadata {
        ImageMetadata {
            author: "Antoine Degardin".into(),
            copyright: "© 2026 Antoine Degardin".into(),
            description: "Essai <métadonnées>".into(),
            keywords: vec!["essai".into(), "lumitype".into()],
            contact: "https://exemple.fr".into(),
            software: "Lumitype".into(),
            created: "2026:10:03 02:00:00".into(),
            created_iso: "2026-10-03T02:00:00".into(),
        }
    }

    /// Relit un bloc EXIF (TIFF) : tag → texte des entrées ASCII.
    fn read_exif(tiff: &[u8]) -> Vec<(u16, String)> {
        assert_eq!(&tiff[..4], b"II\x2A\x00");
        let ifd = u32::from_le_bytes(tiff[4..8].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes(tiff[ifd..ifd + 2].try_into().unwrap()) as usize;
        (0..count)
            .map(|i| {
                let at = ifd + 2 + i * 12;
                let tag = u16::from_le_bytes(tiff[at..at + 2].try_into().unwrap());
                let len = u32::from_le_bytes(tiff[at + 4..at + 8].try_into().unwrap()) as usize;
                let text = if len <= 4 {
                    &tiff[at + 8..at + 8 + len]
                } else {
                    let off = u32::from_le_bytes(tiff[at + 8..at + 12].try_into().unwrap()) as usize;
                    &tiff[off..off + len]
                };
                (tag, String::from_utf8_lossy(&text[..len - 1]).into_owned())
            })
            .collect()
    }

    #[test]
    fn exif_block_holds_the_text_entries_in_tag_order() {
        let entries = read_exif(&sample().exif_tiff());
        assert_eq!(entries.iter().map(|(t, _)| *t).collect::<Vec<_>>(), vec![0x010E, 0x0131, 0x0132, 0x013B, 0x8298]);
        assert_eq!(entries[3].1, "Antoine Degardin");
        assert_eq!(entries[4].1, "© 2026 Antoine Degardin");
    }

    #[test]
    fn xmp_packet_escapes_and_lists_keywords() {
        let xmp = sample().xmp_packet();
        assert!(xmp.contains("<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">Essai &lt;métadonnées&gt;</rdf:li>"));
        assert!(xmp.contains("<rdf:li>essai</rdf:li><rdf:li>lumitype</rdf:li>"));
        assert!(xmp.contains("<xmpRights:WebStatement>https://exemple.fr</xmpRights:WebStatement>"));
        assert!(xmp.ends_with("<?xpacket end=\"w\"?>"));
    }

    #[test]
    fn png_jpeg_webp_and_tiff_keep_their_pixels_and_carry_the_metadata() {
        let (w, h) = (9u32, 5u32);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i * 7) as u8, (i * 13) as u8, 200, if i % 4 == 0 { 128 } else { 255 }]).collect();
        let meta = sample();
        for format in [ExportFormat::Png, ExportFormat::Jpeg, ExportFormat::Webp, ExportFormat::Tiff] {
            // WebP sans perte : la comparaison au pixel près vaut aussi pour lui.
            let plain = crate::io::encode_for_tests(w, h, &rgba, format, true);
            let with = embed(format, plain.clone(), &meta).expect("métadonnées");
            assert!(with.len() > plain.len(), "{format:?} : rien d'ajouté");
            // L'image se relit, aux mêmes dimensions (le JPEG approxime les couleurs, les autres sont exacts).
            let decoded = image::load_from_memory(&with).expect("relecture");
            assert_eq!((decoded.width(), decoded.height()), (w, h), "{format:?}");
            if format != ExportFormat::Jpeg {
                let before = image::load_from_memory(&plain).expect("relecture").to_rgba8();
                assert_eq!(decoded.to_rgba8().as_raw(), before.as_raw(), "{format:?} : pixels changés");
            }
            let text = String::from_utf8_lossy(&with);
            assert!(text.contains("Antoine Degardin"), "{format:?} : auteur absent");
            assert!(text.contains("xmpRights:WebStatement"), "{format:?} : XMP absent");
        }
        // Sans métadonnée : octets inchangés.
        let plain = crate::io::encode_for_tests(w, h, &rgba, ExportFormat::Png, false);
        assert_eq!(embed(ExportFormat::Png, plain.clone(), &ImageMetadata::default()).unwrap(), plain);
    }

    #[test]
    fn jpeg_segments_sit_right_after_jfif() {
        let rgba = vec![255u8; 4 * 4 * 4];
        let plain = crate::io::encode_for_tests(4, 4, &rgba, ExportFormat::Jpeg, false);
        let with = embed(ExportFormat::Jpeg, plain, &sample()).unwrap();
        // SOI, puis (APP0 ou directement) APP1 « Exif ».
        let mut at = 2;
        if with[at + 1] == 0xE0 {
            at += 2 + usize::from(u16::from_be_bytes([with[at + 2], with[at + 3]]));
        }
        assert_eq!(&with[at..at + 2], &[0xFF, 0xE1]);
        assert_eq!(&with[at + 4..at + 10], b"Exif\0\0");
        let exif = read_exif(&with[at + 10..]);
        assert_eq!(exif.iter().find(|(t, _)| *t == 0x8298).map(|(_, v)| v.as_str()), Some("© 2026 Antoine Degardin"));
    }

    #[test]
    fn webp_gets_an_extended_header_with_exif_and_xmp_flags() {
        let rgba: Vec<u8> = (0..16).flat_map(|i| [i * 16, 0, 0, if i % 2 == 0 { 255 } else { 100 }]).collect();
        let plain = crate::io::encode_for_tests(4, 4, &rgba, ExportFormat::Webp, true);
        let with = embed(ExportFormat::Webp, plain, &sample()).unwrap();
        assert_eq!(&with[12..16], b"VP8X");
        let flags = with[20];
        assert_eq!(flags & 0x1C, 0x1C, "alpha, EXIF et XMP annoncés");
        assert_eq!(u32::from_le_bytes(with[4..8].try_into().unwrap()) as usize, with.len() - 8, "taille RIFF");
        let text = String::from_utf8_lossy(&with);
        assert!(text.contains("EXIF") && text.contains("XMP "));
    }

    #[test]
    fn lossy_webp_keeps_its_vp8_stream_and_its_alpha() {
        // Image avec perte et transparence : morceaux VP8X + ALPH + « VP8  » (libwebp), ou « VP8  » seul sans alpha.
        let (w, h) = (37u32, 21u32);
        for (alpha, name) in [(true, "avec alpha"), (false, "opaque")] {
            let rgba: Vec<u8> = (0..w * h).flat_map(|i| [(i * 3) as u8, (i * 5) as u8, 90, if alpha && i % 3 == 0 { 80 } else { 255 }]).collect();
            let plain = crate::io::encode_for_tests(w, h, &rgba, ExportFormat::Webp, false);
            let text = String::from_utf8_lossy(&plain).into_owned();
            assert!(text.contains("VP8 ") && !text.contains("VP8L"), "{name} : flux avec perte attendu");
            assert_eq!(text.contains("ALPH"), alpha, "{name} : morceau alpha");
            let with = embed(ExportFormat::Webp, plain.clone(), &sample()).unwrap();
            assert_eq!(&with[12..16], b"VP8X");
            assert_eq!(with[20] & 0x10 != 0, alpha, "{name} : drapeau alpha du VP8X");
            assert_eq!(with[20] & 0x0C, 0x0C, "{name} : EXIF et XMP annoncés");
            assert_eq!(u32::from_le_bytes(with[4..8].try_into().unwrap()) as usize, with.len() - 8, "{name} : taille RIFF");
            // Les dimensions se lisent aussi dans l'en-tête de trame VP8 seul (fichier sans VP8X).
            let (mut at, mut vp8) = (12, None);
            while at + 8 <= plain.len() {
                let size = u32::from_le_bytes(plain[at + 4..at + 8].try_into().unwrap()) as usize;
                if &plain[at..at + 4] == b"VP8 " {
                    vp8 = Some(plain[at + 8..at + 8 + size].to_vec());
                }
                at += 8 + size + size % 2;
            }
            assert_eq!(webp_geometry(&[(*b"VP8 ", vp8.expect("morceau VP8"))]).unwrap(), (w, h, false), "{name} : en-tête de trame VP8");
            let before = image::load_from_memory(&plain).expect("relecture").to_rgba8();
            let decoded = image::load_from_memory(&with).expect("relecture");
            assert_eq!((decoded.width(), decoded.height()), (w, h), "{name}");
            assert_eq!(decoded.to_rgba8().as_raw(), before.as_raw(), "{name} : pixels changés par l'inscription");
            let text = String::from_utf8_lossy(&with);
            assert!(text.contains("Antoine Degardin") && text.contains("xmpRights:WebStatement"), "{name} : métadonnées absentes");
        }
    }

    #[test]
    fn tiff_ifd_is_rewritten_with_the_new_tags_in_order() {
        let rgba = vec![10u8; 3 * 3 * 4];
        let plain = crate::io::encode_for_tests(3, 3, &rgba, ExportFormat::Tiff, false);
        let with = embed(ExportFormat::Tiff, plain, &sample()).unwrap();
        let little = &with[..2] == b"II";
        let rd32 = |at: usize| { let v: [u8; 4] = with[at..at + 4].try_into().unwrap(); if little { u32::from_le_bytes(v) } else { u32::from_be_bytes(v) } };
        let rd16 = |at: usize| { let v: [u8; 2] = with[at..at + 2].try_into().unwrap(); if little { u16::from_le_bytes(v) } else { u16::from_be_bytes(v) } };
        let ifd = rd32(4) as usize;
        let count = rd16(ifd) as usize;
        let tags: Vec<u16> = (0..count).map(|i| rd16(ifd + 2 + i * 12)).collect();
        assert!(tags.windows(2).all(|w| w[0] < w[1]), "tags triés : {tags:?}");
        assert!(!tags.contains(&317), "Predictor retiré : {tags:?}");
        for tag in [0x010E, 0x0131, 0x0132, 0x013B, 338, 0x8298, 700] {
            assert!(tags.contains(&tag), "tag {tag} absent");
        }
        assert_eq!(image::load_from_memory(&with).expect("relecture TIFF").width(), 3);
    }
}
