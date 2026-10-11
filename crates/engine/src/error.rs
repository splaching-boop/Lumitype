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
}

impl EngineError {
    /// Transforme une erreur d'entrée-sortie en erreur du moteur, en gardant le chemin concerné.
    pub fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { path, source }
    }
}
