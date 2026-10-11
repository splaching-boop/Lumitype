# Copie locale pour Lumitype

Source : https://github.com/SableClient/tauri-runtime-cef, commit `6568170ae5fb27b38dae1e367f0bb3a09a040152`
(1er octobre 2026). Copiée ici plutôt que tirée de GitHub : Lumitype utilise Tauri 2.12 (tauri-runtime 2.12), le
dépôt d'origine vise tauri-runtime 2.11. Exclue de l'espace de travail Cargo : compilée seulement avec
`--features cef` (voir `src-tauri/Cargo.toml`).

Changements (tous marqués « Lumitype » dans le code) :
- `winit` figé à `=0.31.0-beta.2` : les bêtas suivantes changent les événements de glisser-déposer.
- `CefRuntimeHandle` : `primary_monitor`, `monitor_from_point`, `available_monitors` rendent un `Result`
  (signature de tauri-runtime 2.12).
- `CefWindowDispatcher::set_fullscreen_on_monitor` (nouveau dans 2.12) : plein écran sur l'écran courant.
- `WindowBuilderWrapper::no_redirection_bitmap` (nouveau dans 2.12, Windows seulement) : sans effet.
- Fonction `devtools` activée par Lumitype (`open_devtools`, `close_devtools`, `is_devtools_open`).

Quand Tauri publiera officiellement son runtime CEF, cette copie disparaît au profit de la version publiée.

## Windows : abandonné (10/10/2026)

Le Chromium embarqué n'est utilisé que sous Linux. Sous Windows, Lumitype garde WebView2 (export/windows.ps1).
Un essai (05-06/10) s'est soldé par une fenêtre blanche malgré le manifeste `supportedOS` de cefsimple ; le code
Windows de ce runtime, « porté à l'aveugle » par son auteur, n'a jamais été mis au point. Ce qui en reste :

- dépendance `windows` 0.61 → 0.62 (celle de `tauri-runtime` 2.12 ; compilation croisée Windows correcte) ;
- `runtime.rs` : journal Chromium dans `<cache>/chromium.log` (Linux : `~/.cache/dev.imageeditor.app/chromium.log`),
  avertissements et erreurs — utile aussi sous Linux.
