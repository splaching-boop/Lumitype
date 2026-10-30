//! Téléchargement d'une image désignée par son adresse web : image glissée depuis un navigateur qui n'en
//! fournit que l'adresse, « Coller l'adresse d'une image », ou image déposée dans une collection My ref.
//! Client HTTP bloquant (ureq) : à appeler hors du thread de l'interface (voir `blocking` dans commands.rs).

use std::path::PathBuf;

/// Taille maximale acceptée : au-delà, ce n'est plus une image raisonnable (ou pas une image du tout).
pub const MAX_BYTES: u64 = 80 * 1024 * 1024;

/// Image téléchargée : son contenu et le nom tiré de l'adresse (dernier segment, décodé, sans paramètres).
pub struct Downloaded {
    pub bytes: Vec<u8>,
    pub name: PathBuf,
}

/// Télécharge `url` (http ou https seulement), dans la limite de `MAX_BYTES`. L'erreur est un message lisible.
pub fn download_image(url: &str) -> Result<Downloaded, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("seules les adresses http(s) peuvent être importées".into());
    }
    let failed = |e: &dyn std::fmt::Display| format!("téléchargement impossible : {e}");
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(20)).build();
    let response = agent.get(url).call().map_err(|e| failed(&e))?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(response.into_reader(), MAX_BYTES), &mut bytes).map_err(|e| failed(&e))?;
    // Nom : dernier segment de l'adresse, sans ses paramètres.
    let last = url.split(['?', '#']).next().unwrap_or_default().rsplit('/').next().unwrap_or_default();
    let name = PathBuf::from(if last.is_empty() { "Image".to_owned() } else { percent_decode(last) });
    Ok(Downloaded { bytes, name })
}

/// Décode les séquences `%XX` d'un nom (encodeURIComponent côté interface, adresses web). Les séquences
/// invalides restent telles quelles ; un UTF-8 invalide est remplacé plutôt que refusé.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                i += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("ville%20brume.jpg"), "ville brume.jpg");
        assert_eq!(percent_decode("foug%C3%A8re.png"), "fougère.png");
        assert_eq!(percent_decode("100%.png"), "100%.png");
    }

    #[test]
    fn refuses_other_schemes() {
        assert!(download_image("file:///etc/passwd").is_err());
    }
}
