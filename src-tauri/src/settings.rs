//! Préférences de l'utilisateur : un fichier JSON par sujet dans le dossier de
//! configuration de l'application (`%APPDATA%` sous Windows, `~/.config` sous Linux).
//! Le contenu appartient à l'interface ; ce module ne fait que le stocker.

use std::path::PathBuf;

use tauri::Manager;

use crate::rt::AppHandle;

fn settings_file(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    // Le nom vient de l'interface : il ne doit jamais pouvoir sortir du dossier.
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
        return Err(format!("nom de préférences invalide : {name}"));
    }
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{name}.json")))
}

/// Contenu du fichier, ou `None` s'il n'existe pas encore.
#[tauri::command]
pub fn load_settings(app: AppHandle, name: String) -> Result<Option<String>, String> {
    let path = settings_file(&app, &name)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("lecture de {} impossible : {error}", path.display())),
    }
}

#[tauri::command]
pub fn save_settings(app: AppHandle, name: String, content: String) -> Result<(), String> {
    let path = settings_file(&app, &name)?;
    let write = || -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Écriture atomique : un fichier de préférences n'est jamais laissé à moitié écrit.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, content)?;
        std::fs::rename(&tmp, &path)
    };
    write().map_err(|error| format!("écriture de {} impossible : {error}", path.display()))
}
