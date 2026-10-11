//! Pointes de pinceau importées : fichiers Photoshop (.abr), GIMP (.gbr) et simples images.
//!
//! Une pointe est une carte de couverture en niveaux de gris (255 = dépose toute la couleur).
//! Seules les pointes « échantillonnées » (dessinées) d'un fichier ABR sont reprises : les
//! pinceaux « calculés » de Photoshop (ronds paramétriques) ont leur équivalent natif ici, et la
//! dynamique de forme (dispersion, variation…) n'est pas décrite de façon exploitable dans le fichier.

use std::path::Path;

use crate::error::EngineError;

/// Plus grand côté conservé pour une pointe ; au-delà elle est réduite (elle sera de toute façon mise à l'échelle du pinceau).
const MAX_TIP_SIDE: u32 = 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct BrushTip {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Couverture par pixel, ligne par ligne.
    pub mask: Vec<u8>,
    /// Espacement des empreintes, en fraction du diamètre (0,25 = 25 %).
    pub spacing: f32,
}

impl BrushTip {
    fn new(name: String, width: u32, height: u32, mask: Vec<u8>, spacing: f32) -> Option<Self> {
        if width == 0 || height == 0 || mask.len() != (width as usize) * (height as usize) || mask.iter().all(|v| *v == 0) {
            return None;
        }
        let tip = Self { name, width, height, mask, spacing: if spacing.is_finite() { spacing.clamp(0.01, 2.0) } else { 0.25 } };
        Some(tip.limited())
    }

    /// Réduit une pointe démesurée par moyenne de blocs.
    fn limited(self) -> Self {
        let factor = self.width.max(self.height).div_ceil(MAX_TIP_SIDE).max(1);
        if factor == 1 {
            return self;
        }
        let (w, h) = (self.width.div_ceil(factor), self.height.div_ceil(factor));
        let mut mask = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let (mut sum, mut count) = (0u32, 0u32);
                for sy in y * factor..((y + 1) * factor).min(self.height) {
                    for sx in x * factor..((x + 1) * factor).min(self.width) {
                        sum += u32::from(self.mask[(sy * self.width + sx) as usize]);
                        count += 1;
                    }
                }
                mask.push((sum / count.max(1)) as u8);
            }
        }
        Self { width: w, height: h, mask, ..self }
    }

    /// Couverture interpolée en `(u, v)`, coordonnées de 0 à 1 dans la pointe ; 0 en dehors.
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
            return 0.0;
        }
        let (fx, fy) = ((u * self.width as f32 - 0.5).max(0.0), (v * self.height as f32 - 0.5).max(0.0));
        let (x0, y0) = (fx as u32, fy as u32);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |x: u32, y: u32| f32::from(self.mask[(y * self.width + x) as usize]);
        ((at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty) / 255.0
    }
}

/// Valeur pseudo-aléatoire déterministe dans `[0, 1)`.
fn noise(x: u32, y: u32, salt: u32) -> f32 {
    let mut v = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ salt.wrapping_mul(0xC2B2_AE3D);
    v ^= v >> 15;
    v = v.wrapping_mul(0x2C1B_3C6D);
    v ^= v >> 12;
    (v >> 8) as f32 / (1u32 << 24) as f32
}

/// Bruit lissé (interpolation entre les nœuds d'une grille de pas `cell`) : donne du grain « à grande échelle ».
fn smooth_noise(x: f32, y: f32, cell: f32, salt: u32) -> f32 {
    let (gx, gy) = (x / cell, y / cell);
    let (x0, y0) = (gx.floor(), gy.floor());
    let (tx, ty) = (gx - x0, gy - y0);
    let at = |dx: f32, dy: f32| noise((x0 + dx) as u32, (y0 + dy) as u32, salt);
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, ty) = (ease(tx), ease(ty));
    (at(0.0, 0.0) * (1.0 - tx) + at(1.0, 0.0) * tx) * (1.0 - ty) + (at(0.0, 1.0) * (1.0 - tx) + at(1.0, 1.0) * tx) * ty
}

/// Pointes fournies avec le logiciel, dessinées par le calcul (aucun fichier tiers) : de quoi
/// peindre avec de la matière sans rien importer.
pub fn builtin_tips() -> Vec<BrushTip> {
    const SIDE: u32 = 256;
    let centre = SIDE as f32 / 2.0;
    let make = |name: &str, spacing: f32, coverage: &dyn Fn(f32, f32, u32, u32) -> f32| {
        let mask = (0..SIDE * SIDE)
            .map(|i| {
                let (x, y) = (i % SIDE, i / SIDE);
                // Coordonnées centrées, de -1 à 1.
                let (u, v) = ((x as f32 + 0.5 - centre) / centre, (y as f32 + 0.5 - centre) / centre);
                (coverage(u, v, x, y).clamp(0.0, 1.0) * 255.0 + 0.5) as u8
            })
            .collect();
        BrushTip { name: name.to_owned(), width: SIDE, height: SIDE, mask, spacing }
    };
    let disc = |u: f32, v: f32, softness: f32| ((1.0 - u.hypot(v)) / softness).clamp(0.0, 1.0);
    vec![
        // Craie : disque au bord irrégulier, troué d'un grain fin — la couleur n'accroche que les reliefs du papier.
        make("Craie", 0.12, &|u, v, x, y| {
            let grain = noise(x, y, 1) * 0.55 + smooth_noise(x as f32, y as f32, 9.0, 2) * 0.45;
            disc(u, v, 0.18 + smooth_noise(x as f32, y as f32, 28.0, 3) * 0.25) * if grain > 0.42 { 1.0 } else { 0.0 }
        }),
        // Fusain : plus doux, grain plus gros et partiellement couvrant.
        make("Fusain", 0.1, &|u, v, x, y| disc(u, v, 0.55) * (0.25 + 0.75 * smooth_noise(x as f32, y as f32, 5.0, 4)) * (0.6 + 0.4 * noise(x, y, 5))),
        // Calligraphie : ellipse très aplatie, inclinée à 45° ; l'épaisseur du trait dépend de sa direction.
        make("Calligraphie", 0.04, &|u, v, _, _| {
            let (along, across) = ((u + v) / std::f32::consts::SQRT_2, (v - u) / std::f32::consts::SQRT_2);
            ((1.0 - (along / 0.98).hypot(across / 0.16)) * 14.0).clamp(0.0, 1.0)
        }),
        // Éponge : amas de taches rondes.
        make("Éponge", 0.3, &|u, v, x, y| {
            let blobs = smooth_noise(x as f32, y as f32, 22.0, 6) * 0.65 + smooth_noise(x as f32, y as f32, 7.0, 7) * 0.35;
            disc(u, v, 0.3) * ((blobs - 0.45) * 7.0).clamp(0.0, 1.0)
        }),
        // Éclaboussure : une tache centrale et des gouttes projetées autour.
        make("Éclaboussure", 0.6, &|u, v, _, _| {
            let mut best = ((0.34 - u.hypot(v)) * 30.0).clamp(0.0, 1.0);
            for drop in 0..46u32 {
                let (angle, reach) = (noise(drop, 1, 8) * std::f32::consts::TAU, 0.3 + noise(drop, 2, 8).powf(0.7) * 0.66);
                let radius = 0.012 + noise(drop, 3, 8).powi(2) * 0.085;
                let d = (u - angle.cos() * reach).hypot(v - angle.sin() * reach);
                best = best.max(((radius - d) * 40.0).clamp(0.0, 1.0));
            }
            best
        }),
    ]
}

/// Lit toutes les pointes d'un fichier, d'après son extension.
pub fn load_tips(bytes: &[u8], origin: &Path) -> Result<Vec<BrushTip>, EngineError> {
    let stem = origin.file_stem().and_then(|s| s.to_str()).unwrap_or("Pinceau").to_owned();
    let extension = origin.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let tips = match extension.as_str() {
        "abr" => parse_abr(bytes, &stem),
        "gbr" => parse_gbr(bytes, &stem).into_iter().collect(),
        _ => vec![tip_from_image(bytes, &stem)?],
    };
    if tips.is_empty() {
        return Err(EngineError::InvalidOperation(format!(
            "aucune pointe exploitable dans « {} » (seuls les pinceaux dessinés d'un fichier ABR sont repris, pas les pinceaux ronds paramétriques)",
            origin.display()
        )));
    }
    Ok(tips)
}

/// Image ordinaire : sa transparence sert de pointe ; si elle est opaque, c'est sa noirceur (noir = encre, blanc = rien).
fn tip_from_image(bytes: &[u8], name: &str) -> Result<BrushTip, EngineError> {
    let image = image::load_from_memory(bytes).map_err(|e| EngineError::Corrupt(format!("image de pinceau illisible : {e}")))?.into_rgba8();
    let (width, height) = image.dimensions();
    let opaque = image.pixels().all(|px| px.0[3] == 255);
    let mask = image
        .pixels()
        .map(|px| {
            let [r, g, b, a] = px.0;
            if opaque { 255 - ((u32::from(r) * 30 + u32::from(g) * 59 + u32::from(b) * 11) / 100) as u8 } else { a }
        })
        .collect();
    BrushTip::new(name.to_owned(), width, height, mask, 0.25).ok_or_else(|| EngineError::InvalidOperation("cette image ne contient aucune forme de pinceau".into()))
}

/// Lecteur grand-boutiste qui ne panique jamais : toute lecture hors limites rend `None`.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let slice = self.bytes.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(slice)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.take(n).map(|_| ())
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// Pixels d'une pointe ABR : bruts, ou compressés ligne par ligne en PackBits (comme dans un PSD).
fn abr_pixels(reader: &mut Reader, width: u32, height: u32, depth: u16, compressed: bool) -> Option<Vec<u8>> {
    let bytes_per_sample = usize::from(depth / 8).max(1);
    let (w, h) = (width as usize, height as usize);
    let row_bytes = w * bytes_per_sample;
    let mut raw = Vec::with_capacity(row_bytes * h);
    if compressed {
        let lengths: Vec<usize> = (0..h).map(|_| reader.u16().map(usize::from)).collect::<Option<_>>()?;
        for length in lengths {
            let mut line = Reader { bytes: reader.take(length)?, at: 0 };
            let start = raw.len();
            while raw.len() - start < row_bytes {
                let control = line.u8()? as i8;
                if control >= 0 {
                    raw.extend_from_slice(line.take(control as usize + 1)?);
                } else if control != -128 {
                    let value = line.u8()?;
                    raw.extend(std::iter::repeat_n(value, 1 - control as isize as usize));
                }
            }
            raw.truncate(start + row_bytes);
        }
    } else {
        raw.extend_from_slice(reader.take(row_bytes * h)?);
    }
    // 16 bits : l'octet de poids fort suffit.
    Some(raw.chunks_exact(bytes_per_sample).map(|sample| sample[0]).collect())
}

/// Fichier de pinceaux Photoshop. Versions 1 et 2 : liste de pinceaux ; versions 6 et suivantes :
/// blocs `8BIM`, les pointes étant dans le bloc `samp`. Une pointe illisible est ignorée, pas le fichier.
fn parse_abr(bytes: &[u8], stem: &str) -> Vec<BrushTip> {
    let mut reader = Reader { bytes, at: 0 };
    let Some(version) = reader.u16() else { return Vec::new() };
    let mut tips = Vec::new();
    let push = |tips: &mut Vec<BrushTip>, name: Option<String>, width, height, mask, spacing| {
        let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("{stem} {}", tips.len() + 1));
        tips.extend(BrushTip::new(name, width, height, mask, spacing));
    };
    match version {
        1 | 2 => {
            let count = reader.u16().unwrap_or(0);
            for _ in 0..count {
                let (Some(kind), Some(size)) = (reader.u16(), reader.u32()) else { break };
                let next = reader.at + size as usize;
                if kind == 2 {
                    let mut read = || -> Option<()> {
                        reader.skip(4)?; // divers
                        let spacing = f32::from(reader.u16()?) / 100.0;
                        let name = if version == 2 {
                            let length = reader.u32()? as usize;
                            let units: Vec<u16> = (0..length).map(|_| reader.u16()).collect::<Option<_>>()?;
                            Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned())
                        } else {
                            None
                        };
                        reader.skip(1 + 8)?; // antialiasing, bornes courtes
                        let (top, left, bottom, right) = (reader.u32()?, reader.u32()?, reader.u32()?, reader.u32()?);
                        let depth = reader.u16()?;
                        let compressed = reader.u8()? != 0;
                        let (width, height) = (right.checked_sub(left)?, bottom.checked_sub(top)?);
                        if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
                            return None;
                        }
                        let mask = abr_pixels(&mut reader, width, height, depth, compressed)?;
                        push(&mut tips, name, width, height, mask, spacing);
                        Some(())
                    };
                    let _ = read();
                }
                reader.at = next.min(bytes.len());
            }
        }
        6..=10 => {
            let subversion = reader.u16().unwrap_or(0);
            while let (Some(signature), Some(key), Some(length)) = (reader.take(4), reader.take(4), reader.u32()) {
                let end = (reader.at + length as usize).min(bytes.len());
                if signature == b"8BIM" && key == b"samp" {
                    while reader.at + 4 <= end {
                        let Some(size) = reader.u32() else { break };
                        let next = reader.at + ((size as usize + 3) & !3); // chaque pinceau est aligné sur 4 octets
                        let mut read = || -> Option<()> {
                            // Identifiant du pinceau (37 octets), puis un bloc de taille fixe selon la sous-version.
                            reader.skip(if subversion == 1 { 47 } else { 301 })?;
                            let (top, left, bottom, right) = (reader.u32()?, reader.u32()?, reader.u32()?, reader.u32()?);
                            let depth = reader.u16()?;
                            let compressed = reader.u8()? != 0;
                            let (width, height) = (right.checked_sub(left)?, bottom.checked_sub(top)?);
                            if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
                                return None;
                            }
                            let mask = abr_pixels(&mut reader, width, height, depth, compressed)?;
                            push(&mut tips, None, width, height, mask, 0.25);
                            Some(())
                        };
                        let _ = read();
                        reader.at = next.min(end);
                    }
                }
                reader.at = end;
            }
        }
        _ => {}
    }
    tips
}

/// Pinceau GIMP (.gbr, version 2) : niveaux de gris (0 = rien… 255 = encre) ou RGBA (l'alpha sert de pointe).
fn parse_gbr(bytes: &[u8], stem: &str) -> Option<BrushTip> {
    let mut reader = Reader { bytes, at: 0 };
    let header = reader.u32()? as usize;
    let version = reader.u32()?;
    let (width, height, channels) = (reader.u32()?, reader.u32()?, reader.u32()?);
    if version < 2 || reader.take(4)? != b"GIMP" || width == 0 || height == 0 || width > 16_384 || height > 16_384 {
        return None;
    }
    let spacing = reader.u32()? as f32 / 100.0;
    let name = String::from_utf8_lossy(bytes.get(reader.at..header)?).trim_end_matches('\0').trim().to_owned();
    reader.at = header;
    let data = reader.take((width * height * channels) as usize)?;
    let mask = match channels {
        1 => data.to_vec(),
        4 => data.chunks_exact(4).map(|px| px[3]).collect(),
        _ => return None,
    };
    BrushTip::new(if name.is_empty() { stem.to_owned() } else { name }, width, height, mask, spacing)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pointe 4 x 3 : un dégradé, facile à reconnaître.
    fn pixels() -> Vec<u8> {
        (0..12).map(|i| (i * 20 + 10) as u8).collect()
    }

    fn packbits_row(row: &[u8]) -> Vec<u8> {
        let mut out = vec![(row.len() - 1) as u8]; // un seul paquet littéral
        out.extend_from_slice(row);
        out
    }

    #[test]
    fn reads_sampled_brushes_from_abr_version_6_raw_and_compressed() {
        for compressed in [false, true] {
            let mut brush = vec![0u8; 47]; // identifiant + bloc fixe (sous-version 1)
            for v in [0u32, 0, 3, 4] {
                brush.extend_from_slice(&v.to_be_bytes()); // haut, gauche, bas, droite
            }
            brush.extend_from_slice(&8u16.to_be_bytes());
            brush.push(u8::from(compressed));
            if compressed {
                let rows: Vec<Vec<u8>> = pixels().chunks(4).map(packbits_row).collect();
                rows.iter().for_each(|r| brush.extend_from_slice(&(r.len() as u16).to_be_bytes()));
                rows.iter().for_each(|r| brush.extend_from_slice(r));
            } else {
                brush.extend_from_slice(&pixels());
            }
            let mut samp = (brush.len() as u32).to_be_bytes().to_vec();
            samp.extend_from_slice(&brush);
            samp.resize(samp.len().div_ceil(4) * 4, 0);

            let mut file = vec![0, 6, 0, 1];
            file.extend_from_slice(b"8BIMdesc");
            file.extend_from_slice(&4u32.to_be_bytes());
            file.extend_from_slice(&[1, 2, 3, 4]); // un bloc inconnu, à sauter
            file.extend_from_slice(b"8BIMsamp");
            file.extend_from_slice(&(samp.len() as u32).to_be_bytes());
            file.extend_from_slice(&samp);

            let tips = load_tips(&file, Path::new("Craie.abr")).expect("pinceaux");
            assert_eq!(tips.len(), 1);
            assert_eq!((tips[0].width, tips[0].height, tips[0].name.as_str()), (4, 3, "Craie 1"));
            assert_eq!(tips[0].mask, pixels(), "compressé : {compressed}");
        }
    }

    #[test]
    fn reads_abr_version_2_with_its_name_and_spacing() {
        let mut body = vec![0u8; 4];
        body.extend_from_slice(&40u16.to_be_bytes()); // espacement 40 %
        let name: Vec<u16> = "Fusain\0".encode_utf16().collect();
        body.extend_from_slice(&(name.len() as u32).to_be_bytes());
        name.iter().for_each(|u| body.extend_from_slice(&u.to_be_bytes()));
        body.extend_from_slice(&[0; 9]);
        for v in [0u32, 0, 3, 4] {
            body.extend_from_slice(&v.to_be_bytes());
        }
        body.extend_from_slice(&8u16.to_be_bytes());
        body.push(0);
        body.extend_from_slice(&pixels());
        let mut file = vec![0, 2, 0, 2];
        // Un pinceau « calculé » (type 1), ignoré, puis le pinceau dessiné.
        file.extend_from_slice(&1u16.to_be_bytes());
        file.extend_from_slice(&6u32.to_be_bytes());
        file.extend_from_slice(&[0; 6]);
        file.extend_from_slice(&2u16.to_be_bytes());
        file.extend_from_slice(&(body.len() as u32).to_be_bytes());
        file.extend_from_slice(&body);

        let tips = load_tips(&file, Path::new("x.abr")).expect("pinceaux");
        assert_eq!(tips.len(), 1);
        assert_eq!((tips[0].name.as_str(), tips[0].spacing, tips[0].mask.clone()), ("Fusain", 0.4, pixels()));
    }

    #[test]
    fn reads_gimp_brushes_and_plain_images_and_never_panics_on_garbage() {
        let name = b"Eponge\0";
        let header = 28 + name.len() as u32;
        let mut gbr = Vec::new();
        for v in [header, 2, 4, 3, 1] {
            gbr.extend_from_slice(&v.to_be_bytes());
        }
        gbr.extend_from_slice(b"GIMP");
        gbr.extend_from_slice(&15u32.to_be_bytes());
        gbr.extend_from_slice(name);
        gbr.extend_from_slice(&pixels());
        let tip = &load_tips(&gbr, Path::new("e.gbr")).expect("gbr")[0];
        assert_eq!((tip.name.as_str(), tip.width, tip.height, tip.spacing), ("Eponge", 4, 3, 0.15));

        // Image opaque : le noir devient de l'encre, le blanc rien.
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_fn(2, 1, |x, _| image::Rgba(if x == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] })).write_to(&mut png, image::ImageFormat::Png).expect("png");
        assert_eq!(load_tips(png.get_ref(), Path::new("tampon.png")).expect("image")[0].mask, vec![255, 0]);

        for garbage in [&b""[..], &[0, 6][..], &[0, 6, 0, 1, b'8', b'B', b'I', b'M', b's', b'a', b'm', b'p', 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 9][..], &[0, 2, 0xFF, 0xFF, 0, 2, 0xFF][..]] {
            assert!(load_tips(garbage, Path::new("casse.abr")).is_err());
        }
    }

    #[test]
    fn builtin_tips_are_valid_textured_and_stable() {
        let tips = builtin_tips();
        assert_eq!(tips.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Craie", "Fusain", "Calligraphie", "Éponge", "Éclaboussure"]);
        for tip in &tips {
            assert_eq!(tip.mask.len(), (tip.width * tip.height) as usize);
            let inked = tip.mask.iter().filter(|v| **v > 128).count() as f32 / tip.mask.len() as f32;
            assert!((0.02..0.8).contains(&inked), "{} : {inked} de la surface encrée", tip.name);
            assert_eq!(tip.mask[0], 0, "{} : les coins sont vides", tip.name);
        }
        assert_eq!(builtin_tips(), tips, "calcul déterministe : les mêmes pointes à chaque lancement");
        // La calligraphie est bien une barre oblique : encrée le long de la diagonale, vide sur l'autre.
        let pen = &tips[2];
        assert!(pen.sample(0.75, 0.75) > 0.9 && pen.sample(0.75, 0.25) < 0.05);
    }

    #[test]
    fn sampling_is_interpolated_and_empty_outside() {
        let tip = BrushTip::new("t".into(), 2, 1, vec![0, 255], 0.25).expect("pointe");
        assert_eq!((tip.sample(-0.1, 0.5), tip.sample(1.0, 0.5)), (0.0, 0.0));
        assert!((tip.sample(0.5, 0.5) - 0.5).abs() < 0.01 && tip.sample(0.9, 0.5) > 0.95);
        let huge = BrushTip::new("g".into(), 4096, 2, vec![200; 8192], 0.25).expect("pointe");
        assert!(huge.width <= MAX_TIP_SIDE && huge.mask.iter().all(|v| *v == 200), "réduite sans changer sa valeur");
    }
}
