use std::path::PathBuf;

/// Erreurs du moteur. Aucune fonction publique ne panique sur une entrée invalide.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("dimensions invalides : {width} x {height} px (maximum {max} px par côté)")]
    InvalidDimensions { width: u32, height: u32, max: u32 },

    #[error("accès au fichier impossible : {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("image illisible ou format non pris en charge : {path}")]
    Image {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },

    #[error("archive de projet invalide")]
    Archive(#[from] zip::result::ZipError),

    #[error("structure de projet invalide")]
    Structure(#[from] serde_json::Error),

    #[error("ce fichier n'est pas un projet de l'application")]
    NotAProject,

    #[error("projet enregistré avec un format plus récent (version {found}, maximum géré {supported})")]
    UnsupportedVersion { found: u32, supported: u32 },

    #[error("projet incohérent : {0}")]
    Corrupt(String),

    #[error("calque {0} introuvable")]
    UnknownLayer(u64),

    #[error("{0}")]
    InvalidOperation(String),

    /// Opération refusée, avec son code propre (voir [`EngineError::code`]). Les `InvalidOperation` y
    /// passent peu à peu, site par site : leur message reste le même, seul le code s'affine.
    #[error("{message}")]
    Coded { code: &'static str, message: String },
}

impl EngineError {
    /// Opération refusée avec un code précis, par exemple `EngineError::coded("E-DOC-012", "…")`.
    pub fn coded(code: &'static str, message: impl Into<String>) -> Self {
        Self::Coded { code, message: message.into() }
    }

    /// Code stable de l'erreur, affiché à l'utilisateur et repris dans les rapports de bug. Il ne change
    /// pas quand le message est reformulé ou traduit : c'est lui qui identifie le problème.
    ///
    /// Familles : `E-DIM` dimensions, `E-IO` fichiers, `E-IMG` images, `E-PRJ` projets `.graf`,
    /// `E-LAY` calques, `E-OP` opérations refusées (code générique tant que le site n'a pas le sien).
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidDimensions { .. } => "E-DIM-001",
            Self::Io { source, .. } => match source.kind() {
                std::io::ErrorKind::NotFound => "E-IO-001",
                std::io::ErrorKind::PermissionDenied => "E-IO-002",
                std::io::ErrorKind::StorageFull => "E-IO-003",
                std::io::ErrorKind::AlreadyExists => "E-IO-004",
                _ => "E-IO-000",
            },
            Self::Image { .. } => "E-IMG-001",
            Self::Archive(_) => "E-PRJ-001",
            Self::Structure(_) => "E-PRJ-002",
            Self::NotAProject => "E-PRJ-003",
            Self::UnsupportedVersion { .. } => "E-PRJ-004",
            Self::Corrupt(_) => "E-PRJ-005",
            Self::UnknownLayer(_) => "E-LAY-001",
            Self::InvalidOperation(_) => "E-OP-000",
            Self::Coded { code, .. } => code,
        }
    }

    /// Transforme une erreur d'entrée-sortie en erreur du moteur, en gardant le chemin concerné.
    pub fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { path, source }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_a_stable_code() {
        let missing = EngineError::io("/nulle/part")(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert_eq!(missing.code(), "E-IO-001");
        assert_eq!(EngineError::NotAProject.code(), "E-PRJ-003");
        assert_eq!(EngineError::InvalidOperation("refusé".into()).code(), "E-OP-000");
        let coded = EngineError::coded("E-DOC-012", "la position de ce calque est verrouillée");
        assert_eq!((coded.code(), coded.to_string().as_str()), ("E-DOC-012", "la position de ce calque est verrouillée"));
        // Forme imposée : E-FAMILLE-NNN, pour que l'interface et les rapports puissent s'y fier.
        for code in [missing.code(), EngineError::UnknownLayer(3).code(), coded.code()] {
            let parts: Vec<_> = code.split('-').collect();
            assert!(parts.len() == 3 && parts[0] == "E" && parts[2].len() == 3 && parts[2].chars().all(|c| c.is_ascii_digit()), "{code}");
        }
    }
}
