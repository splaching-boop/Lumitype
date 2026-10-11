//! Arithmétique de pixels 8 bits.
//!
//! Toutes les divisions par 255 utilisent un arrondi au plus proche exact,
//! pour que les résultats soient identiques sur CPU et, plus tard, sur GPU.

/// `round(a * b / 255)` exact pour `a, b` dans `0..=255`.
#[inline]
pub fn mul_div_255(a: u8, b: u8) -> u8 {
    let t = u32::from(a) * u32::from(b) + 128;
    (((t >> 8) + t) >> 8) as u8
}

/// Convertit un pixel RGBA à alpha droit en alpha prémultiplié.
#[inline]
pub fn premultiply(px: [u8; 4]) -> [u8; 4] {
    let a = px[3];
    match a {
        255 => px,
        0 => [0; 4],
        _ => [mul_div_255(px[0], a), mul_div_255(px[1], a), mul_div_255(px[2], a), a],
    }
}

/// Opération inverse de [`premultiply`]. Un pixel transparent devient noir transparent.
#[inline]
pub fn unpremultiply(px: [u8; 4]) -> [u8; 4] {
    let a = u32::from(px[3]);
    match a {
        255 => px,
        0 => [0; 4],
        _ => {
            let un = |c: u8| ((u32::from(c) * 255 + a / 2) / a).min(255) as u8;
            [un(px[0]), un(px[1]), un(px[2]), px[3]]
        }
    }
}

/// Fusion « Normal » (source-over) en alpha prémultiplié : `dst = src + dst * (1 - src.a)`.
#[inline]
pub fn over(src: [u8; 4], dst: [u8; 4]) -> [u8; 4] {
    let inv = 255 - src[3];
    [
        src[0].saturating_add(mul_div_255(dst[0], inv)),
        src[1].saturating_add(mul_div_255(dst[1], inv)),
        src[2].saturating_add(mul_div_255(dst[2], inv)),
        src[3].saturating_add(mul_div_255(dst[3], inv)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_255_matches_rounded_division() {
        for a in 0..=255u32 {
            for b in 0..=255u32 {
                let expected = ((a * b) as f64 / 255.0).round() as u8;
                assert_eq!(mul_div_255(a as u8, b as u8), expected, "{a} * {b}");
            }
        }
    }

    /// Le format de projet stocke des PNG à alpha droit : l'aller-retour
    /// prémultiplié -> droit -> prémultiplié doit être sans perte.
    #[test]
    fn premultiplied_roundtrip_is_lossless() {
        for a in 1..=255u8 {
            for c in 0..=a {
                let px = [c, c / 2, 0, a];
                assert_eq!(premultiply(unpremultiply(px)), px, "c={c} a={a}");
            }
        }
    }

    #[test]
    fn over_opaque_source_replaces_destination() {
        assert_eq!(over([10, 20, 30, 255], [200, 200, 200, 255]), [10, 20, 30, 255]);
    }

    #[test]
    fn over_transparent_source_keeps_destination() {
        assert_eq!(over([0; 4], [1, 2, 3, 200]), [1, 2, 3, 200]);
    }

    #[test]
    fn over_half_alpha_blends() {
        // Source blanche à 50 % (prémultipliée) sur noir opaque.
        assert_eq!(over([128, 128, 128, 128], [0, 0, 0, 255]), [128, 128, 128, 255]);
    }
}
