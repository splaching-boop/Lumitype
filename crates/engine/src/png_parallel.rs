//! Encodeur PNG multi-cœurs, pour l'export.
//!
//! Un encodeur PNG classique compresse toute l'image d'un seul tenant, sur un seul cœur : c'était 85 % du temps
//! d'export d'une photo 8K. Ici, l'image est découpée en bandes de lignes compressées EN PARALLÈLE, puis mises bout à
//! bout, comme le fait `pigz` pour gzip :
//!
//! - chaque ligne reçoit le filtre PNG qui la rend la plus compressible (choix « adaptatif », comme les encodeurs
//!   usuels) ; le filtre d'une ligne ne dépend que de la ligne au-dessus, déjà connue : les bandes sont
//!   indépendantes ;
//! - chaque bande devient un flux deflate « brut » terminé par un vidage synchrone (bloc vide, non final), sauf la
//!   dernière, terminée normalement : leur simple concaténation forme un flux deflate valide ;
//! - la somme Adler-32 du flux zlib est assemblée à partir de celles des bandes (formule de `adler32_combine`).
//!
//! Le fichier est un PNG parfaitement standard, lisible partout. Coût : chaque bande repart sans dictionnaire, soit
//! quelques dixièmes de pour cent de fichier en plus.

use flate2::{Compress, Compression, FlushCompress, Status};
use rayon::prelude::*;

use crate::error::EngineError;

/// Lignes par bande, visées pour ~1 Mo de données : assez pour bien compresser, assez nombreuses pour occuper tous
/// les cœurs.
fn rows_per_strip(row_bytes: usize, height: usize) -> usize {
    let threads = rayon::current_num_threads().max(1);
    let by_size = (1 << 20) / row_bytes.max(1);
    let by_threads = height.div_ceil(threads * 4);
    by_size.min(by_threads).clamp(16, height.max(16))
}

/// Encode une image 8 bits (`channels` = 3 pour RGB, 4 pour RGBA ; pixels à alpha droit) en PNG. `level` : 0 à 9.
pub fn encode(width: u32, height: u32, pixels: &[u8], channels: usize, level: u32) -> Result<Vec<u8>, EngineError> {
    let (w, h) = (width as usize, height as usize);
    let row = w * channels;
    if w == 0 || h == 0 || pixels.len() != row * h || !matches!(channels, 3 | 4) {
        return Err(EngineError::Corrupt("encodage PNG : tampon de taille inattendue".into()));
    }
    let strip = rows_per_strip(row, h);
    let starts: Vec<usize> = (0..h).step_by(strip).collect();
    let last = starts.len() - 1;
    let parts: Vec<(Vec<u8>, u32, usize)> = starts
        .par_iter()
        .enumerate()
        .map(|(index, &first)| {
            let end = (first + strip).min(h);
            let filtered = filter_rows(pixels, row, channels, first, end);
            let adler = adler32(&filtered);
            let deflated = deflate_raw(&filtered, level, index == last)?;
            Ok((deflated, adler, filtered.len()))
        })
        .collect::<Result<_, EngineError>>()?;

    // Flux zlib : en-tête, bandes, Adler-32 de l'ensemble.
    let mut adler = 1u32;
    for (_, part, len) in &parts {
        adler = adler32_combine(adler, *part, *len);
    }
    let mut zlib = Vec::with_capacity(parts.iter().map(|p| p.0.len()).sum::<usize>() + 6);
    zlib.extend_from_slice(&[0x78, 0x9c]);
    for (data, _, _) in &parts {
        zlib.extend_from_slice(data);
    }
    zlib.extend_from_slice(&adler.to_be_bytes());

    let mut out = Vec::with_capacity(zlib.len() + 64);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, if channels == 4 { 6 } else { 2 }, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    // Données en morceaux de 1 Mio : taille habituelle, et bien en dessous de la limite de 2 Gio d'un morceau.
    for piece in zlib.chunks(1 << 20) {
        chunk(&mut out, b"IDAT", piece);
    }
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// Poids estimé du PNG : parmi les bandes qu'utiliserait `encode`, `samples` bandes réparties sur la hauteur sont
/// réellement filtrées et compressées ; leur poids est extrapolé à toutes les bandes. Même découpage que l'encodage
/// réel : l'estimation suit fidèlement le vrai poids.
pub fn estimate(width: u32, height: u32, pixels: &[u8], channels: usize, level: u32, samples: usize) -> Result<u64, EngineError> {
    let (w, h) = (width as usize, height as usize);
    let row = w * channels;
    if w == 0 || h == 0 || pixels.len() != row * h {
        return Err(EngineError::Corrupt("estimation PNG : tampon de taille inattendue".into()));
    }
    let strip = rows_per_strip(row, h);
    let starts: Vec<usize> = (0..h).step_by(strip).collect();
    let picked: Vec<usize> = if starts.len() <= samples { starts.clone() } else { (0..samples).map(|i| starts[i * (starts.len() - 1) / (samples - 1).max(1)]).collect() };
    let sizes: Vec<(u64, u64)> = picked
        .par_iter()
        .map(|&first| {
            let end = (first + strip).min(h);
            let filtered = filter_rows(pixels, row, channels, first, end);
            deflate_raw(&filtered, level, false).map(|d| (d.len() as u64, (end - first) as u64))
        })
        .collect::<Result<_, EngineError>>()?;
    let (bytes, rows) = sizes.iter().fold((0, 0), |(b, r), (sb, sr)| (b + sb, r + sr));
    // Données extrapolées à toute la hauteur, plus les en-têtes (signature, IHDR, IEND, 12 octets par IDAT).
    let data = bytes * h as u64 / rows.max(1);
    Ok(data + 57 + 12 * data.div_ceil(1 << 20))
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
}

/// Lignes `first..end`, chacune précédée de son octet de filtre. Filtre choisi par ligne : celui dont la somme des
/// valeurs (lues comme signées) est la plus petite, l'heuristique classique des encodeurs PNG.
fn filter_rows(pixels: &[u8], row: usize, bpp: usize, first: usize, end: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity((end - first) * (row + 1));
    let zeros = vec![0u8; row];
    let mut candidate = vec![0u8; row];
    let mut best = vec![0u8; row];
    for y in first..end {
        let current = &pixels[y * row..(y + 1) * row];
        let above = if y == 0 { &zeros[..] } else { &pixels[(y - 1) * row..y * row] };
        let mut best_kind = 0u8;
        let mut best_score = u64::MAX;
        for kind in 0..5u8 {
            let mut score = 0u64;
            for i in 0..row {
                let a = if i >= bpp { current[i - bpp] } else { 0 };
                let b = above[i];
                let c = if i >= bpp { above[i - bpp] } else { 0 };
                let x = current[i];
                let v = match kind {
                    0 => x,
                    1 => x.wrapping_sub(a),
                    2 => x.wrapping_sub(b),
                    3 => x.wrapping_sub(((u16::from(a) + u16::from(b)) / 2) as u8),
                    _ => x.wrapping_sub(paeth(a, b, c)),
                };
                candidate[i] = v;
                score += u64::from((v as i8).unsigned_abs());
            }
            if score < best_score {
                best_score = score;
                best_kind = kind;
                std::mem::swap(&mut best, &mut candidate);
            }
        }
        out.push(best_kind);
        out.extend_from_slice(&best);
    }
    out
}

#[inline]
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Flux deflate brut ; `last` : bloc final, sinon vidage synchrone (le flux suivant s'y accole).
fn deflate_raw(data: &[u8], level: u32, last: bool) -> Result<Vec<u8>, EngineError> {
    let failed = || EngineError::Corrupt("encodage PNG : compression impossible".into());
    let mut compress = Compress::new(Compression::new(level.min(9)), false);
    let mut out = Vec::with_capacity(data.len() / 2 + 1024);
    let flush = if last { FlushCompress::Finish } else { FlushCompress::Sync };
    loop {
        let consumed = compress.total_in() as usize;
        if out.capacity() - out.len() < 64 * 1024 {
            out.reserve(out.capacity().max(64 * 1024));
        }
        let status = compress.compress_vec(&data[consumed..], &mut out, flush).map_err(|_| failed())?;
        let done = compress.total_in() as usize == data.len();
        match status {
            Status::StreamEnd => break,
            // Vidage synchrone terminé : tout est lu et il restait de la place (sinon il faut rappeler).
            Status::Ok | Status::BufError if !last && done && out.len() < out.capacity() => break,
            Status::Ok | Status::BufError => {}
        }
    }
    Ok(out)
}

const ADLER_MOD: u32 = 65_521;

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    // Par blocs de 5 552 octets : la plus grande longueur sans débordement avant le modulo (comme zlib).
    for block in data.chunks(5552) {
        for &byte in block {
            a += u32::from(byte);
            b += a;
        }
        a %= ADLER_MOD;
        b %= ADLER_MOD;
    }
    (b << 16) | a
}

/// Adler-32 de la concaténation `A‖B`, connaissant celles de A et de B et la longueur de B (formule de zlib).
fn adler32_combine(first: u32, second: u32, second_len: usize) -> u32 {
    let rem = (second_len % ADLER_MOD as usize) as u64;
    let m = u64::from(ADLER_MOD);
    let (a1, b1) = (u64::from(first & 0xffff), u64::from(first >> 16));
    let (a2, b2) = (u64::from(second & 0xffff), u64::from(second >> 16));
    let a = (a1 + a2 + m - 1) % m;
    let b = (b1 + b2 + rem * a1 % m + m - rem) % m;
    ((b << 16) | a) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(w: u32, h: u32, channels: usize) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                let noise = (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)) >> 24;
                let px = [(x * 7) as u8 ^ noise as u8, (y * 3) as u8, ((x + y) / 2) as u8, 128 + (noise as u8 / 2)];
                px.into_iter().take(channels)
            })
            .collect()
    }

    #[test]
    fn a_parallel_png_decodes_to_exactly_the_same_pixels() {
        for (w, h, channels) in [(1, 1, 4), (7, 3, 3), (1000, 700, 4), (513, 1999, 3)] {
            let pixels = sample(w, h, channels);
            let png = encode(w, h, &pixels, channels, 6).expect("encodage");
            let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png).expect("PNG valide");
            let back = if channels == 4 { decoded.into_rgba8().into_raw() } else { decoded.into_rgb8().into_raw() };
            assert_eq!(back, pixels, "{w} × {h} × {channels}");
        }
    }

    #[test]
    fn combined_checksums_match_the_whole() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i * 31 % 251) as u8).collect();
        let (a, b) = data.split_at(77_777);
        assert_eq!(adler32_combine(adler32(a), adler32(b), b.len()), adler32(&data));
    }
}
