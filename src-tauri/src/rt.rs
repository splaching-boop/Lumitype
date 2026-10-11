//! Moteur de la fenêtre (runtime Tauri) : Chromium embarqué avec `--features cef` (Linux, ce que construit
//! export/linux.sh), webview du système sinon (wry : WebView2 sous Windows, toujours ; WebKitGTK sous Linux en
//! secours, `LUMITYPE_MOTEUR=webkit`).
//! Tout le code passe par ces alias : le reste de l'application ne dépend pas du moteur choisi.

#[cfg(not(any(feature = "cef", feature = "wry")))]
compile_error!("Choisir un moteur : fonction « wry » (par défaut : webview du système) ou « cef » (Chromium).");

#[cfg(feature = "cef")]
pub type Runtime = tauri_runtime_cef::CefRuntime<tauri::EventLoopMessage>;
#[cfg(not(feature = "cef"))]
pub type Runtime = tauri::Wry;

pub type AppHandle = tauri::AppHandle<Runtime>;

/// Chromium : réglages communs au processus principal et à ses processus auxiliaires (rendu, carte graphique), à
/// poser avant toute chose. Renvoie vrai si ce processus EST un processus auxiliaire : il a alors fini son travail.
#[cfg(feature = "cef")]
pub fn start_cef() -> bool {
    use tauri_runtime_cef::{CefConfig, LinuxWindowing};
    // `LUMITYPE_WAYLAND=1` : essai de Chromium en Wayland natif (Ozone) au lieu de X11 (XWayland). Ce mode du
    // runtime n'accepte qu'une fenêtre, sans déplacement ni redimensionnement : inutilisable tel quel (beta99).
    let wayland = cfg!(target_os = "linux") && std::env::var_os("LUMITYPE_WAYLAND").is_some_and(|v| v != "0");
    // Cadence (Linux, X11 sous XWayland) : Chromium cale `requestAnimationFrame` sur le vsync que XWayland lui
    // annonce, 60 Hz même sur un écran 240 Hz. Ces deux switches lèvent la borne ; c'est le compositeur (Mutter,
    // KWin) qui présente la fenêtre à la cadence réelle de l'écran, sans déchirure. Lumitype ne dessine que quand
    // quelque chose change, donc rien ne tourne à vide. `LUMITYPE_VSYNC=1` rétablit l'ancien comportement.
    let unlimited = cfg!(target_os = "linux") && !std::env::var_os("LUMITYPE_VSYNC").is_some_and(|v| v != "0");
    tauri_runtime_cef::configure(CefConfig {
        identifier: "dev.imageeditor.app".into(),
        command_line_args: [
            // Lumitype compose tout en WebGL : la carte graphique est exigée même si Chromium la juge douteuse.
            ("ignore-gpu-blocklist".into(), None),
            ("enable-gpu-rasterization".into(), None),
            ("enable-zero-copy".into(), None),
            // Pas de bac à sable : il demande un assistant installé avec des droits spéciaux (chrome-sandbox).
            ("no-sandbox".into(), None),
        ]
        .into_iter()
        .chain(unlimited.then(|| [("disable-frame-rate-limit".into(), None), ("disable-gpu-vsync".into(), None)]).into_iter().flatten())
        .chain(cef_files_switches())
        .collect(),
        linux_windowing: if wayland { LinuxWindowing::Wayland } else { LinuxWindowing::X11 },
        ..Default::default()
    });
    if std::env::args().any(|arg| arg.starts_with("--type=")) {
        tauri_runtime_cef::run_cef_helper_process();
        return true;
    }
    false
}

/// Paquets Linux installés : l'exécutable est dans `/usr/bin` (ou `~/.local/bin`, `/app/bin`), les
/// fichiers de Chromium dans `../lib/Lumitype/` avec les ressources de Lumitype. `libcef.so` y est trouvé par le chemin de recherche de
/// l'exécutable (`rpath`, voir build.rs) ; ses données (`*.pak`, `locales/`) sont désignées explicitement. Rien à
/// faire en développement : `target/release/` contient tout, à côté de l'exécutable.
#[cfg(feature = "cef")]
fn cef_files_switches() -> Vec<(String, Option<String>)> {
    let Ok(exe) = std::env::current_exe() else { return Vec::new() };
    let Some(dir) = exe.parent() else { return Vec::new() };
    if dir.join("icudtl.dat").exists() {
        return Vec::new();
    }
    let Ok(lib) = dir.join("../lib/Lumitype").canonicalize() else { return Vec::new() };
    if !lib.join("icudtl.dat").exists() {
        return Vec::new();
    }
    vec![
        ("resources-dir-path".into(), Some(lib.display().to_string())),
        ("locales-dir-path".into(), Some(lib.join("locales").display().to_string())),
    ]
}
