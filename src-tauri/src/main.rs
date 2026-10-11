// Pas de console sous Windows en version finale.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod gpu_info;
mod rt;
mod settings;
mod state;

fn main() {
    // Chromium : ce même exécutable sert aussi de processus de rendu et de carte graphique ; ceux-là s'arrêtent ici.
    #[cfg(feature = "cef")]
    if rt::start_cef() {
        return;
    }
    let result = tauri::Builder::<rt::Runtime>::new()
        // À enregistrer en premier : un second lancement (double-clic sur un autre projet) ne démarre pas une
        // deuxième application ; ses fichiers sont transmis à la fenêtre déjà ouverte, qui passe au premier plan.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            use tauri::{Emitter, Manager};
            let files = commands::files_among(args.into_iter().skip(1));
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            if !files.is_empty() {
                let _ = app.emit_to("main", "open-files", files);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::default())
        .setup(|app| {
            // Polices livrées avec Lumitype (ressources `fonts/`) : recensées avant celles du système, pour qu'un
            // projet s'affiche pareil sur toute machine. Voir engine::text.
            use tauri::Manager;
            if let Ok(dir) = app.path().resource_dir() {
                engine::text::set_bundled_fonts_dir(dir.join("fonts"));
            }
            // Disque de travail de l'historique : `<cache>/dev.imageeditor.app/scratch/` par défaut (un autre dossier
            // peut être choisi dans les préférences). Les fichiers d'échange d'une session qui s'est mal terminée sont
            // effacés ici, avant tout document — dans ce dossier et dans celui choisi dans les préférences (performance.json).
            if let Ok(cache) = app.path().app_cache_dir() {
                let scratch = cache.join("scratch");
                engine::scratch::clean_leftovers(&scratch);
                engine::scratch::set_default_dir(scratch);
            }
            let chosen = app.path().app_config_dir().ok().and_then(|dir| std::fs::read_to_string(dir.join("performance.json")).ok());
            let chosen = chosen.and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
            if let Some(dir) = chosen.as_ref().and_then(|perf| perf.get("scratchDir")).and_then(|dir| dir.as_str()).map(str::trim).filter(|dir| !dir.is_empty()) {
                engine::scratch::clean_leftovers(std::path::Path::new(dir));
            }
            // Filet de sécurité : si l'interface ne parvenait jamais à signaler qu'elle est prête, la
            // fenêtre principale s'afficherait quand même au bout de vingt secondes. On ne reste jamais
            // bloqué devant la bannière de lancement.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(20));
                commands::reveal_main_window(&handle);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::new_document,
            commands::open_path,
            commands::import_dropped,
            commands::get_tile,
            commands::get_selection_tile,
            commands::save_project,
            commands::export_png,
            commands::close_document,
            commands::thumbnail,
            commands::pick_layer,
            commands::other_bounds,
            commands::pick_text_layer,
            commands::list_fonts,
            commands::bundled_font_file,
            commands::export_overview,
            commands::export_region,
            commands::export_exact_size,
            commands::export_preview_release,
            commands::save_preview,
            commands::load_preview,
            commands::list_templates,
            commands::template_preview,
            commands::open_template,
            commands::delete_preview,
            commands::file_exists,
            commands::reveal_path,
            commands::adjust_plan,
            commands::adjustment_lut,
            commands::adjustment_gpu,
            commands::autosave,
            commands::recovery_list,
            commands::recovery_open,
            commands::recovery_delete,
            commands::read_file_bytes,
            commands::clipboard_fingerprint,
            commands::clipboard_image,
            commands::clipboard_set_text,
            commands::layer_pixels,
            commands::layer_mask,
            commands::layer_effects,
            commands::startup_files,
            commands::copy_layers,
            commands::paste_layers,
            commands::smart_content_open,
            commands::smart_content_commit,
            commands::smart_content_export,
            commands::smart_links_check,
            commands::directory_exists,
            commands::close_splash,
            commands::load_glass_texture,
            commands::system_info,
            commands::system_gpus,
            commands::set_performance,
            commands::memory_status,
            commands::import_brushes,
            commands::list_brushes,
            commands::brush_thumbnail,
            commands::export_preview,
            commands::export_image,
            commands::import_url,
            commands::clipboard_png,
            commands::stroke_begin_selection,
            commands::filter_preview,
            commands::text_caret,
            commands::text_hit,
            commands::sample_color,
            commands::move_plan,
            commands::get_tile_part,
            commands::content_bounds,
            commands::edit,
            commands::stroke_begin,
            commands::stroke_points,
            commands::stroke_end,
            settings::load_settings,
            settings::save_settings,
        ])
        .run(tauri::generate_context!());

    if let Err(error) = result {
        eprintln!("Démarrage impossible : {error}");
        std::process::exit(1);
    }
}
