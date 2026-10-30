//! Conversion RVB → CMJN : épreuve écran, alerte de gamme, export (PDF, TIFF et JPEG en CMJN) et mode Couleurs CMJN.
//!
//! Un seul CMJN pour tout Lumitype (Édition › Paramètres de couleur, comme Photoshop) : un **espace CMJN**
//! ([`CmykSpace`]) réunit le profil de presse, l'intention de rendu et la compensation du point noir. L'épreuve
//! (Ctrl+Y), le mode Couleurs CMJN et l'export s'en servent tous ; un document converti en CMJN garde le sien.
//!
//! Les tables sont calculées à la demande par LittleCMS (la référence) puis gardées en mémoire, pour n'importe quel
//! profil : les cinq livrés (`assets/cmyk/<id>.icc`, tous libres, voir assets/cmyk/README.md), ceux installés
//! sur l'ordinateur (par Photoshop notamment, voir [`system_profile_dirs`]) et ceux que l'on charge soi-même.
//! - table d'export : grille 33 × 33 × 33 de couleurs sRGB (rouge le plus rapide, puis vert, puis bleu ; le nœud `i`
//!   vaut `i × 255 / 32`), chaque nœud donnant son CMJN 8 bits (0 = pas d'encre, 255 = 100 %) ;
//! - table d'épreuve : même grille, chaque nœud donnant la couleur sRGB que l'impression pourra rendre (aller-retour
//!   sRGB → CMJN → sRGB, retour en colorimétrie relative avec compensation du point noir). L'interface la charge
//!   telle quelle en texture 3D ; l'aperçu de l'export et l'alerte de gamme la lisent ici.
//!
//! Entre les nœuds, interpolation tétraédrique (celle des moteurs de gestion des couleurs : plus fidèle que la
//! trilinéaire sur les dégradés neutres, et moins coûteuse).
//!
//! Un profil qui n'est pas livré (« personnalisé ») est désigné par l'empreinte de son contenu ([`CmykProfile::Custom`]) :
//! le même fichier a le même identifiant sur tous les ordinateurs. Un projet en mode CMJN avec un tel profil le
//! garde dans son archive (io.rs) : il se rouvre à l'identique ailleurs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use rayon::prelude::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::EngineError;

/// Points par axe des tables.
pub const N: usize = 33;

/// Profil d'impression CMJN : un des cinq livrés, ou un profil personnalisé désigné par l'empreinte de son contenu.
/// Écrit en JSON comme l'interface le nomme : `"fogra39"`, …, `"icc-0123456789abcdef"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CmykProfile {
    #[default]
    Fogra39,
    Fogra51,
    Fogra52,
    Gracol2006,
    Swop2006c3,
    Custom(u64),
}

/// Intention de rendu vers le CMJN (le retour à l'écran de l'épreuve est toujours en colorimétrie relative).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RenderIntent {
    /// Colorimétrie relative : les couleurs imprimables restent exactes, les autres sont ramenées au bord de la gamme.
    #[default]
    Relative,
    /// Perceptive : toute l'image est resserrée pour garder les écarts entre couleurs (si le profil la décrit).
    Perceptual,
}

fn yes() -> bool {
    true
}

/// Espace CMJN : profil, intention et compensation du point noir. Le réglage par défaut est celui de Photoshop en
/// Europe (Coated FOGRA39, colorimétrie relative, compensation du point noir).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CmykSpace {
    pub profile: CmykProfile,
    #[serde(default)]
    pub intent: RenderIntent,
    #[serde(default = "yes")]
    pub bpc: bool,
}

impl Default for CmykSpace {
    fn default() -> Self {
        Self { profile: CmykProfile::default(), intent: RenderIntent::default(), bpc: true }
    }
}

impl From<CmykProfile> for CmykSpace {
    fn from(profile: CmykProfile) -> Self {
        Self { profile, ..Self::default() }
    }
}

impl CmykProfile {
    /// Les profils livrés, dans l'ordre des listes.
    pub const BUILTIN: [Self; 5] = [Self::Fogra39, Self::Fogra51, Self::Fogra52, Self::Gracol2006, Self::Swop2006c3];

    /// Identifiant (celui de l'interface et des fichiers).
    pub fn id(self) -> String {
        match self {
            Self::Fogra39 => "fogra39".into(),
            Self::Fogra51 => "fogra51".into(),
            Self::Fogra52 => "fogra52".into(),
            Self::Gracol2006 => "gracol2006".into(),
            Self::Swop2006c3 => "swop2006c3".into(),
            Self::Custom(hash) => format!("icc-{hash:016x}"),
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "fogra39" => Self::Fogra39,
            "fogra51" => Self::Fogra51,
            "fogra52" => Self::Fogra52,
            "gracol2006" => Self::Gracol2006,
            "swop2006c3" => Self::Swop2006c3,
            _ => Self::Custom(u64::from_str_radix(id.strip_prefix("icc-")?, 16).ok()?),
        })
    }

    pub fn is_builtin(self) -> bool {
        !matches!(self, Self::Custom(_))
    }

    /// Fichier ICC livré.
    fn builtin_icc(self) -> Option<&'static [u8]> {
        Some(match self {
            Self::Fogra39 => include_bytes!("../assets/cmyk/fogra39.icc"),
            Self::Fogra51 => include_bytes!("../assets/cmyk/fogra51.icc"),
            Self::Fogra52 => include_bytes!("../assets/cmyk/fogra52.icc"),
            Self::Gracol2006 => include_bytes!("../assets/cmyk/gracol2006.icc"),
            Self::Swop2006c3 => include_bytes!("../assets/cmyk/swop2006c3.icc"),
            Self::Custom(_) => return None,
        })
    }

    /// Nom affiché (et inscrit dans l'intention de sortie d'un PDF), à la manière de Photoshop (« Coated FOGRA39 ») ;
    /// pour un profil personnalisé, sa description.
    pub fn display_name(self) -> Result<String, EngineError> {
        Ok(match self {
            Self::Fogra39 => "Coated FOGRA39 (ISO 12647-2:2004)".into(),
            Self::Fogra51 => "PSO Coated v3 (FOGRA51)".into(),
            Self::Fogra52 => "PSO Uncoated v3 (FOGRA52)".into(),
            Self::Gracol2006 => "GRACoL 2006 Coated #1".into(),
            Self::Swop2006c3 => "SWOP 2006 Coated #3".into(),
            Self::Custom(_) => custom(self)?.name.clone(),
        })
    }

    /// Nom court (onglet du document, barre d'état : « CMJN FOGRA39 »).
    pub fn short_name(self) -> Result<String, EngineError> {
        Ok(match self {
            Self::Fogra39 => "FOGRA39".into(),
            Self::Fogra51 => "FOGRA51".into(),
            Self::Fogra52 => "FOGRA52".into(),
            Self::Gracol2006 => "GRACoL 2006".into(),
            Self::Swop2006c3 => "SWOP 2006".into(),
            Self::Custom(_) => short(&custom(self)?.name),
        })
    }

    /// Condition d'impression caractérisée (registre de l'ICC), pour l'intention de sortie d'un PDF ; pour un profil
    /// personnalisé, sa description (rien de mieux n'est connu).
    pub fn condition(self) -> Result<String, EngineError> {
        Ok(match self {
            Self::Fogra39 => "FOGRA39".into(),
            Self::Fogra51 => "FOGRA51".into(),
            Self::Fogra52 => "FOGRA52".into(),
            Self::Gracol2006 => "CGATS TR 006".into(),
            Self::Swop2006c3 => "CGATS TR 003".into(),
            Self::Custom(_) => custom(self)?.name.clone(),
        })
    }

    /// Le profil ICC lui-même, à incorporer au fichier.
    pub fn icc_bytes(self) -> Result<Arc<[u8]>, EngineError> {
        match self.builtin_icc() {
            Some(bytes) => Ok(Arc::from(bytes)),
            None => Ok(custom(self)?.icc.clone()),
        }
    }
}

impl Serialize for CmykProfile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.id())
    }
}

impl<'de> Deserialize<'de> for CmykProfile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Self::from_id(&id).ok_or_else(|| serde::de::Error::custom(format!("profil CMJN inconnu : {id}")))
    }
}

/// Nom court d'une description de profil : sans les mentions entre parenthèses, 28 caractères au plus.
fn short(name: &str) -> String {
    let base = name.split(" (").next().unwrap_or(name).trim();
    let base = if base.is_empty() { name.trim() } else { base };
    if base.chars().count() <= 28 { base.to_string() } else { format!("{}…", base.chars().take(27).collect::<String>()) }
}

/// Profil sRGB (LittleCMS, ICC v4), incorporé aux exports en RVB quand on demande le profil.
pub fn srgb_icc() -> &'static [u8] {
    include_bytes!("../assets/cmyk/srgb.icc")
}

// ---------------------------------------------------------------------------------------------------------------
// Profils personnalisés

/// Profil personnalisé connu de cette session.
struct CustomProfile {
    name: String,
    icc: Arc<[u8]>,
}

static CUSTOM: LazyLock<RwLock<HashMap<u64, Arc<CustomProfile>>>> = LazyLock::new(|| RwLock::new(HashMap::new()));

fn custom(profile: CmykProfile) -> Result<Arc<CustomProfile>, EngineError> {
    let CmykProfile::Custom(hash) = profile else {
        return Err(EngineError::InvalidOperation("profil livré".into()));
    };
    CUSTOM.read().expect("registre des profils").get(&hash).cloned().ok_or_else(|| {
        EngineError::InvalidOperation("profil CMJN introuvable sur cet ordinateur : rechargez-le (Édition › Paramètres de couleur)".into())
    })
}

/// Empreinte stable d'un fichier (FNV-1a 64 bits) : le même profil a le même identifiant partout, d'une version à
/// l'autre (contrairement au hachage de la bibliothèque standard).
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// En-tête ICC d'un profil CMJN (sans le lire en entier) : signature `acsp`, espace de données `CMYK`.
fn looks_cmyk(header: &[u8]) -> bool {
    header.len() >= 40 && &header[36..40] == b"acsp" && &header[16..20] == b"CMYK"
}

/// Fait connaître un profil ICC CMJN (fichier choisi, trouvé sur l'ordinateur ou gardé dans un projet) et rend son
/// identifiant. Un profil livré, chargé depuis son fichier, reste le profil livré.
pub fn register_icc(bytes: &[u8]) -> Result<CmykProfile, EngineError> {
    let refuse = || EngineError::InvalidOperation("ce fichier n'est pas un profil ICC CMJN".into());
    if !looks_cmyk(bytes) {
        return Err(refuse());
    }
    if let Some(builtin) = CmykProfile::BUILTIN.into_iter().find(|p| p.builtin_icc() == Some(bytes)) {
        return Ok(builtin);
    }
    let hash = fingerprint(bytes);
    if CUSTOM.read().expect("registre des profils").contains_key(&hash) {
        return Ok(CmykProfile::Custom(hash));
    }
    let profile = lcms2::Profile::new_icc(bytes).map_err(|_| refuse())?;
    if profile.color_space() != lcms2::ColorSpaceSignature::CmykData {
        return Err(refuse());
    }
    let name = profile
        .info(lcms2::InfoType::Description, lcms2::Locale::new("fr_FR"))
        .or_else(|| profile.info(lcms2::InfoType::Description, lcms2::Locale::none()))
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("Profil CMJN {:08x}", hash >> 32));
    // Une table d'essai : un profil illisible pour LittleCMS est refusé dès maintenant, pas à l'épreuve.
    build_tables(bytes, RenderIntent::Relative, true, 3)?;
    CUSTOM.write().expect("registre des profils").insert(hash, Arc::new(CustomProfile { name, icc: Arc::from(bytes) }));
    Ok(CmykProfile::Custom(hash))
}

/// Description d'un profil ICC quelconque (celui incorporé à une image, pour la visionneuse), s'il est lisible.
pub fn icc_description(icc: &[u8]) -> Option<String> {
    let profile = lcms2::Profile::new_icc(icc).ok()?;
    profile.info(lcms2::InfoType::Description, lcms2::Locale::none()).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

/// Profil personnalisé à garder dans un projet (`None` pour un profil livré ou inconnu).
pub fn custom_icc(profile: CmykProfile) -> Option<Arc<[u8]>> {
    match profile {
        CmykProfile::Custom(_) => custom(profile).ok().map(|c| c.icc.clone()),
        _ => None,
    }
}

/// Tous les profils personnalisés connus de cette session (copie de secours : elle les garde tous).
pub fn registered_custom() -> Vec<(CmykProfile, Arc<[u8]>)> {
    CUSTOM.read().expect("registre des profils").iter().map(|(hash, c)| (CmykProfile::Custom(*hash), c.icc.clone())).collect()
}

/// Dossiers où le système et les logiciels (Photoshop, Illustrator…) installent leurs profils ICC.
pub fn system_profile_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    if cfg!(target_os = "windows") {
        if let Some(windir) = env("WINDIR").or_else(|| env("SystemRoot")) {
            dirs.push(windir.join("System32").join("spool").join("drivers").join("color"));
        }
        for common in [env("CommonProgramFiles"), env("CommonProgramFiles(x86)"), env("CommonProgramW6432")].into_iter().flatten() {
            dirs.push(common.join("Adobe").join("Color").join("Profiles"));
        }
    } else if cfg!(target_os = "macos") {
        dirs.push("/Library/ColorSync/Profiles".into());
        dirs.push("/Library/Application Support/Adobe/Color/Profiles".into());
        dirs.push("/System/Library/ColorSync/Profiles".into());
        if let Some(home) = env("HOME") {
            dirs.push(home.join("Library/ColorSync/Profiles"));
        }
    } else {
        dirs.push("/usr/share/color/icc".into());
        dirs.push("/usr/local/share/color/icc".into());
        if let Some(data) = env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share"))) {
            dirs.push(data.join("color/icc"));
            dirs.push(data.join("icc"));
        }
        if let Some(home) = env("HOME") {
            dirs.push(home.join(".color/icc"));
        }
    }
    dirs.dedup();
    dirs
}

/// Profils CMJN trouvés dans ces dossiers (trois niveaux de sous-dossiers au plus), chacun rendu disponible. Les
/// fichiers illisibles ou d'un autre espace sont ignorés ; un même profil présent deux fois n'est rendu qu'une fois.
pub fn scan_profiles(dirs: &[PathBuf]) -> Vec<(CmykProfile, PathBuf)> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() && depth < 3 {
                walk(&path, depth + 1, out);
            } else if kind.is_file() && path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("icc") || e.eq_ignore_ascii_case("icm")) {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    for dir in dirs {
        walk(dir, 0, &mut files);
    }
    let mut found: Vec<(CmykProfile, PathBuf)> = Vec::new();
    for path in files {
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.len() > 32 << 20 || meta.len() < 128 {
            continue;
        }
        let mut header = [0u8; 40];
        let Ok(mut file) = std::fs::File::open(&path) else { continue };
        if std::io::Read::read_exact(&mut file, &mut header).is_err() || !looks_cmyk(&header) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        if let Ok(profile) = register_icc(&bytes) {
            if !found.iter().any(|(p, _)| *p == profile) {
                found.push((profile, path));
            }
        }
    }
    found
}

// ---------------------------------------------------------------------------------------------------------------
// Tables

/// Tables d'un espace CMJN (voir l'en-tête du module).
pub(crate) struct Tables {
    pub(crate) export: Vec<u8>,
    pub(crate) proof: Vec<u8>,
}

static TABLES: LazyLock<Mutex<HashMap<CmykSpace, Arc<Tables>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Tables de l'espace, calculées une seule fois. Le verrou est tenu pendant le calcul : deux fils demandant le
/// même espace en même temps (épreuve et export) ne le construisent pas deux fois, le second attend le premier.
/// Un verrou empoisonné (panique dans un autre fil) reste utilisable : la table y est entière ou absente.
pub(crate) fn tables(space: CmykSpace) -> Result<Arc<Tables>, EngineError> {
    let mut cache = TABLES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(found) = cache.get(&space) {
        return Ok(found.clone());
    }
    let icc = space.profile.icc_bytes()?;
    let built = Arc::new(build_tables(&icc, space.intent, space.bpc, N)?);
    cache.insert(space, built.clone());
    Ok(built)
}

/// Calcule les tables `n³` d'un profil avec LittleCMS.
fn build_tables(icc: &[u8], intent: RenderIntent, bpc: bool, n: usize) -> Result<Tables, EngineError> {
    use lcms2::{Flags, Intent, PixelFormat, Profile, Transform};
    let failed = |what: &str| EngineError::InvalidOperation(format!("profil CMJN inutilisable ({what})"));
    let srgb = Profile::new_srgb();
    let press = Profile::new_icc(icc).map_err(|_| failed("lecture"))?;
    let intent = match intent {
        RenderIntent::Relative => Intent::RelativeColorimetric,
        RenderIntent::Perceptual => Intent::Perceptual,
    };
    let flags = if bpc { Flags::BLACKPOINT_COMPENSATION | Flags::HIGHRES_PRECALC } else { Flags::HIGHRES_PRECALC };
    let to_press: Transform<[u16; 3], [u16; 4]> = Transform::new_flags(&srgb, PixelFormat::RGB_16, &press, PixelFormat::CMYK_16, intent, flags).map_err(|_| failed("vers le CMJN"))?;
    // Retour à l'écran : colorimétrie relative avec compensation du point noir (le papier devient le blanc de l'écran).
    let back: Transform<[u16; 4], [u8; 3]> = Transform::new_flags(&press, PixelFormat::CMYK_16, &srgb, PixelFormat::RGB_8, Intent::RelativeColorimetric, Flags::BLACKPOINT_COMPENSATION | Flags::HIGHRES_PRECALC).map_err(|_| failed("vers l'écran"))?;
    // Nœuds exacts : le nœud i vaut i × 255 / (n − 1), en 16 bits (× 257).
    let level = |i: usize| ((i as u64 * 65_535 * 2 + (n as u64 - 1)) / (2 * (n as u64 - 1))) as u16;
    let mut grid = Vec::with_capacity(n * n * n);
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                grid.push([level(r), level(g), level(b)]);
            }
        }
    }
    let mut cmyk = vec![[0u16; 4]; grid.len()];
    to_press.transform_pixels(&grid, &mut cmyk);
    let mut screen = vec![[0u8; 3]; grid.len()];
    back.transform_pixels(&cmyk, &mut screen);
    let to8 = |v: u16| ((u32::from(v) * 255 + 32_767) / 65_535) as u8;
    Ok(Tables { export: cmyk.iter().flat_map(|c| c.map(to8)).collect(), proof: screen.into_iter().flatten().collect() })
}

/// Table de l'épreuve écran (33³ × 3 octets, voir l'en-tête), pour la carte graphique.
pub fn proof_table(space: CmykSpace) -> Result<Vec<u8>, EngineError> {
    Ok(tables(space)?.proof.clone())
}

/// Description d'un profil pour l'interface.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfo {
    pub id: CmykProfile,
    pub name: String,
    pub short: String,
    pub builtin: bool,
}

impl ProfileInfo {
    pub fn of(profile: CmykProfile) -> Result<Self, EngineError> {
        Ok(Self { id: profile, name: profile.display_name()?, short: profile.short_name()?, builtin: profile.is_builtin() })
    }
}

/// Position d'une valeur 8 bits sur un axe de la table : (indice du nœud inférieur, fraction de 0 à 256).
/// Le nœud `i` vaut `i × 255 / 32` ; 255 tombe sur le dernier intervalle, fraction pleine.
const AXIS: [(u16, u16); 256] = {
    let mut out = [(0u16, 0u16); 256];
    let mut v = 0;
    while v < 256 {
        let pos = (v * (N - 1) * 256 + 127) / 255; // position en 1/256 de nœud
        let (mut i, mut f) = (pos / 256, pos % 256);
        if i >= N - 1 {
            i = N - 2;
            f = 256;
        }
        out[v] = (i as u16, f as u16);
        v += 1;
    }
    out
};

/// Interpolation tétraédrique dans une table N³ à `C` composantes.
#[inline]
pub(crate) fn lookup<const C: usize>(lut: &[u8], r: u8, g: u8, b: u8) -> [u8; C] {
    let (ri, rf) = AXIS[r as usize];
    let (gi, gf) = AXIS[g as usize];
    let (bi, bf) = AXIS[b as usize];
    let (rf, gf, bf) = (i32::from(rf), i32::from(gf), i32::from(bf));
    let (dr, dg, db) = (C, N * C, N * N * C);
    let c000 = ((usize::from(bi) * N + usize::from(gi)) * N + usize::from(ri)) * C;
    let c111 = c000 + dr + dg + db;
    // Les trois sommets intermédiaires du tétraèdre qui contient le point, et les poids correspondants.
    let (a, b2, wa, wb, wc) = if rf >= gf {
        if gf >= bf {
            (c000 + dr, c000 + dr + dg, rf, gf, bf)
        } else if rf >= bf {
            (c000 + dr, c000 + dr + db, rf, bf, gf)
        } else {
            (c000 + db, c000 + dr + db, bf, rf, gf)
        }
    } else if bf >= gf {
        (c000 + db, c000 + dg + db, bf, gf, rf)
    } else if bf >= rf {
        (c000 + dg, c000 + dg + db, gf, bf, rf)
    } else {
        (c000 + dg, c000 + dr + dg, gf, rf, bf)
    };
    std::array::from_fn(|k| {
        let v0 = i32::from(lut[c000 + k]);
        let va = i32::from(lut[a + k]);
        let vb = i32::from(lut[b2 + k]);
        let v1 = i32::from(lut[c111 + k]);
        let v = v0 * 256 + wa * (va - v0) + wb * (vb - va) + wc * (v1 - vb);
        ((v + 128) >> 8).clamp(0, 255) as u8
    })
}

/// Pose un pixel RGBA (alpha droit) sur du blanc : l'impression n'a pas de transparence.
#[inline]
fn on_white(px: &[u8]) -> (u8, u8, u8) {
    let a = u32::from(px[3]);
    if a == 255 {
        return (px[0], px[1], px[2]);
    }
    let mix = |c: u8| ((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
    (mix(px[0]), mix(px[1]), mix(px[2]))
}

/// CMJN 8 bits → sRGB (colorimétrie relative, compensation du point noir) : le retour à l'écran de l'épreuve, pour
/// des couleurs d'encre données (correction de l'encrage).
pub fn cmyk_to_rgb(space: CmykSpace, inks: &[[u8; 4]]) -> Result<Vec<[u8; 3]>, EngineError> {
    use lcms2::{Flags, Intent, PixelFormat, Profile, Transform};
    let failed = |what: &str| EngineError::InvalidOperation(format!("profil CMJN inutilisable ({what})"));
    let icc = space.profile.icc_bytes()?;
    let press = Profile::new_icc(&icc).map_err(|_| failed("lecture"))?;
    let srgb = Profile::new_srgb();
    let back: Transform<[u16; 4], [u8; 3]> = Transform::new_flags(&press, PixelFormat::CMYK_16, &srgb, PixelFormat::RGB_8, Intent::RelativeColorimetric, Flags::BLACKPOINT_COMPENSATION | Flags::HIGHRES_PRECALC).map_err(|_| failed("vers l'écran"))?;
    let wide: Vec<[u16; 4]> = inks.iter().map(|c| c.map(|v| u16::from(v) * 257)).collect();
    let mut out = vec![[0u8; 3]; wide.len()];
    back.transform_pixels(&wide, &mut out);
    Ok(out)
}

/// Une couleur sRGB en CMJN 8 bits (0 = pas d'encre).
pub fn rgb_to_cmyk_pixel(space: CmykSpace, r: u8, g: u8, b: u8) -> Result<[u8; 4], EngineError> {
    Ok(lookup::<4>(&tables(space)?.export, r, g, b))
}

/// Image RGBA (alpha droit, non prémultiplié) → CMJN 4 octets par pixel, transparence posée sur du blanc.
/// Sur tous les cœurs.
pub fn rgba_to_cmyk(rgba: &[u8], space: CmykSpace) -> Result<Vec<u8>, EngineError> {
    let tables = tables(space)?;
    let lut = &tables.export;
    let mut out = vec![0u8; rgba.len()];
    out.par_chunks_mut(4 * 4096).zip(rgba.par_chunks(4 * 4096)).for_each(|(dst, src)| {
        for (o, px) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            let (r, g, b) = on_white(px);
            o.copy_from_slice(&lookup::<4>(lut, r, g, b));
        }
    });
    Ok(out)
}

/// Épreuve écran de l'export CMJN : chaque pixel (posé sur du blanc) tel que l'impression pourra le rendre,
/// ramené en sRGB pour l'affichage. RGBA opaque.
pub fn soft_proof(rgba: &[u8], space: CmykSpace) -> Result<Vec<u8>, EngineError> {
    let tables = tables(space)?;
    let lut = &tables.proof;
    let mut out = vec![0u8; rgba.len()];
    out.par_chunks_mut(4 * 4096).zip(rgba.par_chunks(4 * 4096)).for_each(|(dst, src)| {
        for (o, px) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            let (r, g, b) = on_white(px);
            let [r, g, b] = lookup::<3>(lut, r, g, b);
            o.copy_from_slice(&[r, g, b, 255]);
        }
    });
    Ok(out)
}

/// Couleur sRGB en L*a*b* (D65), pour mesurer un écart perceptible.
fn lab([r, g, b]: [u8; 3]) -> [f32; 3] {
    let linear = |v: u8| {
        let c = f32::from(v) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (linear(r), linear(g), linear(b));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.950_47;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.088_83;
    let f = |t: f32| if t > 0.008_856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// Écart (ΔE 1976, luminosité pondérée de moitié) au-delà duquel une couleur est signalée hors gamme. Les neutres et
/// les couleurs douces font moins de 2 à l'aller-retour (arrondis et interpolation de la table), le noir pur sur papier
/// non couché 6,8 (point noir du papier, un peu chaud), une couleur déjà ramenée dans la gamme jusqu'à 4 ; les
/// primaires vives de l'écran, 10 à 40.
pub const GAMUT_DELTA_E: f32 = 7.5;

/// Gamme d'impression d'une couleur sRGB : son équivalent imprimable (aller-retour sRGB → CMJN → sRGB, la table de
/// l'épreuve écran), l'écart entre les deux (ΔE 1976, luminosité pondérée de moitié), et si cet écart la met hors gamme (voir [`GAMUT_DELTA_E`]).
/// L'alerte du sélecteur de couleur en mode CMJN, et son bouton « ramener dans la gamme » (qui prend l'équivalent).
pub fn gamut(space: CmykSpace, [r, g, b]: [u8; 3]) -> Result<([u8; 3], f32, bool), EngineError> {
    let printable = lookup::<3>(&tables(space)?.proof, r, g, b);
    let (a, p) = (lab([r, g, b]), lab(printable));
    // La luminosité compte pour moitié : la compensation du point noir assombrit ou éclaircit surtout les tons très
    // sombres (le noir pur sur papier non couché, ΔL ≈ 7) sans qu'ils soient pour autant « hors gamme » au sens de
    // Photoshop ; ce sont la saturation et la teinte perdues qui font l'alerte.
    let delta = ((0.5 * (a[0] - p[0])).powi(2) + (a[1] - p[1]).powi(2) + (a[2] - p[2]).powi(2)).sqrt();
    Ok((printable, delta, delta > GAMUT_DELTA_E))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> impl Iterator<Item = CmykSpace> {
        CmykProfile::BUILTIN.into_iter().map(CmykSpace::from)
    }

    #[test]
    fn tables_have_the_expected_size_and_profiles_are_cmyk() {
        for space in all() {
            let t = tables(space).unwrap();
            assert_eq!(t.export.len(), N * N * N * 4);
            assert_eq!(t.proof.len(), N * N * N * 3);
            let icc = space.profile.icc_bytes().unwrap();
            assert!(looks_cmyk(&icc), "{}", space.profile.id());
        }
        assert_eq!(&srgb_icc()[16..20], b"RGB ");
    }

    /// Demandées en même temps par plusieurs fils, les tables d'un espace ne sont calculées qu'une fois (même `Arc`),
    /// et un verrou empoisonné n'empêche pas les appels suivants.
    #[test]
    fn tables_are_built_once_and_survive_a_poisoned_lock() {
        let space = CmykSpace::from(CmykProfile::Fogra39);
        let built: Vec<Arc<Tables>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4).map(|_| scope.spawn(move || tables(space).unwrap())).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(built.iter().all(|t| Arc::ptr_eq(t, &built[0])), "une seule construction");
        let _ = std::thread::spawn(|| {
            let _guard = TABLES.lock().unwrap();
            panic!("empoisonnement volontaire");
        })
        .join();
        assert!(Arc::ptr_eq(&tables(space).unwrap(), &built[0]));
    }

    #[test]
    fn interpolation_hits_the_nodes_exactly() {
        let space = CmykSpace::default();
        let lut = tables(space).unwrap().export.clone();
        for (r, g, b) in [(0usize, 0usize, 0usize), (32, 32, 32), (32, 0, 0), (8, 16, 24)] {
            let node = ((b * N + g) * N + r) * 4;
            let level = |i: usize| (i * 255 + 16) / 32;
            let got = rgb_to_cmyk_pixel(space, level(r) as u8, level(g) as u8, level(b) as u8).unwrap();
            for k in 0..4 {
                assert!((i32::from(got[k]) - i32::from(lut[node + k])).abs() <= 2, "{r} {g} {b} : {got:?}");
            }
        }
    }

    #[test]
    fn primary_colors_land_on_the_right_inks() {
        for space in all() {
            let white = rgb_to_cmyk_pixel(space, 255, 255, 255).unwrap();
            assert!(white.iter().all(|&v| v <= 2), "blanc {white:?}");
            let black = rgb_to_cmyk_pixel(space, 0, 0, 0).unwrap();
            assert!(black[3] > 200, "noir {black:?}");
            let [c, m, y, k] = rgb_to_cmyk_pixel(space, 255, 0, 0).unwrap();
            assert!(m > 200 && y > 200 && c < 30 && k < 30, "rouge {:?}", [c, m, y, k]);
            let [c, m, y, _] = rgb_to_cmyk_pixel(space, 0, 0, 255).unwrap();
            assert!(c > 180 && m > 150 && y < 30, "bleu");
        }
    }

    #[test]
    fn the_runtime_tables_match_the_reference_values() {
        // Valeurs mesurées avec LittleCMS 2.18 (Pillow) au moment du passage aux tables calculées : le bleu pur en
        // FOGRA51 donne C97 M75 J0 N0, affiché 13,75,153 ; en FOGRA39, C93 M85 et 53,61,144 (à 2 niveaux près).
        let near = |a: &[u8], b: &[u8]| a.iter().zip(b).all(|(x, y)| (i32::from(*x) - i32::from(*y)).abs() <= 3);
        let fogra51 = CmykSpace::from(CmykProfile::Fogra51);
        let cmyk = rgb_to_cmyk_pixel(fogra51, 0, 0, 255).unwrap().map(|v| (u32::from(v) * 100 + 127) / 255);
        assert!(near(&cmyk.map(|v| v as u8), &[97, 75, 0, 0]), "{cmyk:?}");
        assert!(near(&gamut(fogra51, [0, 0, 255]).unwrap().0, &[13, 75, 153]));
        assert!(near(&gamut(CmykSpace::default(), [0, 0, 255]).unwrap().0, &[53, 61, 144]));
    }

    #[test]
    fn intent_and_black_point_compensation_change_the_tables() {
        let base = CmykSpace::from(CmykProfile::Fogra52);
        let perceptual = CmykSpace { intent: RenderIntent::Perceptual, ..base };
        let no_bpc = CmykSpace { bpc: false, ..base };
        let a = tables(base).unwrap();
        assert_ne!(a.export, tables(perceptual).unwrap().export);
        assert_ne!(a.export, tables(no_bpc).unwrap().export);
    }

    #[test]
    fn transparency_is_flattened_on_white() {
        let space = CmykSpace::default();
        let rgba = [0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 0, 255];
        let cmyk = rgba_to_cmyk(&rgba, space).unwrap();
        assert!(cmyk[..4].iter().all(|&v| v <= 2));
        assert!(cmyk[5] > 200 && cmyk[6] > 200);
        assert!(cmyk[11] > 200);
        let proof = soft_proof(&rgba, space).unwrap();
        assert!(proof[..3].iter().all(|&v| v >= 250) && proof[3] == 255);
        assert!(proof[4] > 180 && proof[5] < 80, "rouge imprimable {:?}", &proof[4..8]);
    }

    #[test]
    fn vivid_screen_colors_are_out_of_gamut_and_neutrals_are_not() {
        for space in all() {
            for rgb in [[255, 255, 255], [0, 0, 0], [128, 128, 128], [200, 170, 150], [90, 110, 130]] {
                let (_, delta, out) = gamut(space, rgb).unwrap();
                assert!(!out, "{} {rgb:?} : ΔE {delta}", space.profile.id());
            }
            for rgb in [[0, 0, 255], [0, 255, 0], [255, 0, 255]] {
                let (printable, delta, out) = gamut(space, rgb).unwrap();
                assert!(out, "{} {rgb:?} : ΔE {delta}", space.profile.id());
                // L'équivalent imprimable est, lui, dans la gamme.
                let (_, again, still) = gamut(space, printable).unwrap();
                assert!(!still, "{} {printable:?} ramené : ΔE {again}", space.profile.id());
            }
        }
    }

    #[test]
    fn a_custom_profile_gets_a_stable_id_and_round_trips() {
        // Le profil GRACoL livré, légèrement modifié (octet de description inchangé, fin du fichier) : un profil
        // « inconnu », valide pour LittleCMS.
        let mut bytes = CmykProfile::Gracol2006.icc_bytes().unwrap().to_vec();
        assert_eq!(register_icc(&bytes).unwrap(), CmykProfile::Gracol2006, "un profil livré reste le profil livré");
        bytes[44] ^= 1; // drapeaux du profil (sans effet sur la conversion)
        let id = register_icc(&bytes).unwrap();
        assert!(matches!(id, CmykProfile::Custom(_)));
        assert_eq!(register_icc(&bytes).unwrap(), id);
        let json = serde_json::to_string(&id).unwrap();
        assert!(json.starts_with("\"icc-") && json.len() == 2 + 4 + 16, "{json}");
        assert_eq!(serde_json::from_str::<CmykProfile>(&json).unwrap(), id);
        assert_eq!(&*custom_icc(id).unwrap(), &bytes[..]);
        assert!(!ProfileInfo::of(id).unwrap().name.is_empty());
        let same = rgb_to_cmyk_pixel(CmykSpace::from(id), 0, 0, 255).unwrap();
        assert_eq!(same, rgb_to_cmyk_pixel(CmykSpace::from(CmykProfile::Gracol2006), 0, 0, 255).unwrap());
        assert!(register_icc(srgb_icc()).is_err(), "un profil RVB n'est pas un profil CMJN");
        assert!(register_icc(b"pas un profil").is_err());
    }

    #[test]
    fn scanning_a_folder_finds_cmyk_profiles_only() {
        let dir = std::env::temp_dir().join(format!("lumitype-icc-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Recommended")).unwrap();
        std::fs::write(dir.join("Recommended/presse.icc"), &*CmykProfile::Fogra51.icc_bytes().unwrap()).unwrap();
        std::fs::write(dir.join("ecran.icm"), srgb_icc()).unwrap();
        std::fs::write(dir.join("notes.txt"), b"rien").unwrap();
        let found = scan_profiles(&[dir.clone()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, CmykProfile::Fogra51);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn ids_and_spaces_match_the_interface() {
        for profile in CmykProfile::BUILTIN {
            let json = serde_json::to_string(&profile).unwrap();
            assert_eq!(json, format!("\"{}\"", profile.id()));
            assert_eq!(serde_json::from_str::<CmykProfile>(&json).unwrap(), profile);
        }
        assert!(serde_json::from_str::<CmykProfile>("\"inconnu\"").is_err());
        let space: CmykSpace = serde_json::from_str(r#"{"profile":"fogra51"}"#).unwrap();
        assert_eq!(space, CmykSpace { profile: CmykProfile::Fogra51, intent: RenderIntent::Relative, bpc: true });
        assert_eq!(serde_json::to_string(&space).unwrap(), r#"{"profile":"fogra51","intent":"relative","bpc":true}"#);
    }
}
