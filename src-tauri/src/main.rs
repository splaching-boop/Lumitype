// Pas de console sous Windows en version finale.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod gpu_info;
mod myref;
mod net;
mod rt;
mod settings;
mod state;
mod viewer;

fn main() {
    // Générateur de miniatures de l'explorateur de fichiers (Linux, src-tauri/linux/lumitype.thumbnailer) :
    // `lumitype --miniature -s <taille> <RAW> <PNG>`, sans fenêtre, puis on s'arrête.
    if std::env::args().nth(1).as_deref() == Some("--miniature") {
        std::process::exit(thumbnail(std::env::args().skip(2).collect()));
    }
    // Chromium : ce même exécutable sert aussi de processus de rendu et de carte graphique ; ceux-là s'arrêtent ici.
    #[cfg(feature = "cef")]
    if rt::start_cef() {
        return;
    }
    // Visionneuse (viewer.rs) : `lumitype --visionneuse [image]`, ou l'exécutable `lumitype-visionneuse` (lien vers
    // celui-ci). Une seule fenêtre, sans l'éditeur.
    let viewer = viewer_mode();
    let mut context = tauri::generate_context!();
    if viewer {
        context.config_mut().app.windows = vec![viewer_window()];
    }
    let mut builder = tauri::Builder::<rt::Runtime>::new();
    if !viewer && session_bus_usable() {
        // À enregistrer en premier : un second lancement (double-clic sur un autre projet) ne démarre pas une
        // deuxième application ; ses fichiers sont transmis à la fenêtre déjà ouverte, qui passe au premier plan. La
        // visionneuse n'y participe pas : chaque image ouverte depuis l'explorateur a sa fenêtre.
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            use tauri::{Emitter, Manager};
            let files = commands::files_among(args.into_iter().skip(1));
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            if !files.is_empty() {
                let _ = app.emit_to("main", "open-files", files);
            }
        }));
    }
    let result = builder
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::default())
        .setup(move |app| {
            if viewer {
                // Ni disque de travail (un éditeur ouvert à côté s'en sert : rien n'y est effacé), ni bannière.
                return Ok(());
            }
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
            commands::raw_open,
            commands::raw_open_dropped,
            commands::raw_preview,
            commands::raw_region,
            commands::raw_pick_white,
            commands::raw_auto_tone,
            commands::raw_finish,
            commands::raw_close,
            commands::raw_close_all,
            commands::raw_open_smart,
            commands::raw_redevelop,
            commands::raw_open_layer,
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
            commands::open_project_link,
            commands::bug_report_context,
            commands::open_logs_dir,
            commands::adjust_plan,
            commands::adjustment_lut,
            commands::adjustment_gpu,
            commands::pattern_catalog,
            commands::pattern_swatch,
            commands::bitmap_preview,
            commands::cmyk_gamut,
            commands::cmyk_proof_table,
            commands::cmyk_profiles,
            commands::cmyk_import_profile,
            commands::cmyk_profile_info,
            commands::autosave,
            commands::recovery_list,
            commands::recovery_open,
            commands::recovery_delete,
            commands::read_file_bytes,
            commands::write_json_file,
            commands::clipboard_fingerprint,
            commands::clipboard_image,
            commands::clipboard_set_text,
            commands::layer_pixels,
            commands::history_step,
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
            commands::import_brushes_dropped,
            commands::list_brushes,
            commands::brush_thumbnail,
            commands::brush_library,
            commands::brush_preview,
            commands::remove_brush_pack,
            commands::export_preview,
            commands::ink_report,
            commands::cmyk_of,
            commands::preflight,
            commands::separations,
            commands::export_image,
            commands::import_url,
            commands::clipboard_png,
            commands::layer_export_overview,
            commands::export_layers,
            commands::copy_layers_png,
            commands::layer_file_stem,
            commands::project_text_styles,
            commands::document_preview,
            commands::advanced_selection_open,
            commands::advanced_selection_image,
            commands::advanced_selection_preview,
            commands::advanced_selection_close,
            commands::stroke_begin_selection,
            commands::filter_preview,
            commands::text_caret,
            commands::text_hit,
            commands::sample_color,
            commands::move_plan,
            commands::move_layout,
            commands::move_slice,
            commands::get_tile_part,
            commands::content_bounds,
            commands::edit,
            commands::stroke_begin,
            commands::stroke_points,
            commands::stroke_stamps,
            commands::brush_tip_image,
            commands::brush_pattern_image,
            commands::stroke_end,
            // Bibliothèque d'images de référence « My ref » (accueil).
            myref::myref_overview,
            myref::myref_collection,
            myref::myref_search,
            myref::myref_create_collection,
            myref::myref_rename_collection,
            myref::myref_delete_collection,
            myref::myref_reorder_collections,
            myref::myref_import_paths,
            myref::myref_import_bytes,
            myref::myref_import_url,
            myref::myref_update_item,
            myref::myref_rename_item,
            myref::myref_move_item,
            myref::myref_memberships,
            myref::myref_copy_item,
            myref::myref_remove_from,
            myref::myref_reorder_items,
            myref::myref_set_cover,
            myref::myref_sort_items,
            myref::myref_open_collection,
            myref::myref_delete_item,
            myref::myref_thumbnail,
            myref::myref_image,
            settings::load_settings,
            settings::save_settings,
            // Visionneuse.
            viewer::viewer_start,
            viewer::viewer_folder,
            viewer::viewer_image,
            viewer::viewer_thumb,
            viewer::viewer_screen,
            viewer::viewer_info,
            viewer::viewer_rotate,
            viewer::viewer_trash,
            viewer::viewer_copy,
            viewer::viewer_edit,
        ])
        .run(context);

    if let Err(error) = result {
        eprintln!("Démarrage impossible : {error}");
        std::process::exit(1);
    }
}


/// Lancé comme visionneuse : option `--visionneuse`, ou exécutable nommé `…visionneuse` (lien créé par l'installateur).
fn viewer_mode() -> bool {
    let named = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase())).is_some_and(|s| s.ends_with("visionneuse"));
    named || std::env::args().skip(1).any(|a| a == "--visionneuse")
}

/// Fenêtre de la visionneuse (sans barre de titre du système : elle dessine la sienne, comme l'éditeur).
fn viewer_window() -> tauri::utils::config::WindowConfig {
    tauri::utils::config::WindowConfig {
        label: "viewer".into(),
        title: "Lumitype Visionneuse".into(),
        url: tauri::WebviewUrl::App("viewer.html".into()),
        width: 1400.0,
        height: 900.0,
        min_width: Some(640.0),
        min_height: Some(420.0),
        center: true,
        decorations: false,
        shadow: true,
        background_color: Some(tauri::utils::config::Color(18, 17, 17, 255)),
        // Glisser une image sur la fenêtre l'ouvre (chemins fournis par Tauri).
        drag_drop_enabled: true,
        ..Default::default()
    }
}

/// `-s <taille> <entrée> <sortie>` (ordre de la spécification freedesktop) : écrit la miniature PNG. Code de sortie
/// non nul en cas d'échec : l'explorateur garde alors son icône.
fn thumbnail(args: Vec<String>) -> i32 {
    let mut size = 256u32;
    let mut files = Vec::new();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        if arg == "-s" {
            size = it.next().and_then(|v| v.parse().ok()).unwrap_or(size);
        } else {
            files.push(std::path::PathBuf::from(arg));
        }
    }
    let [input, output] = files.as_slice() else {
        eprintln!("usage : lumitype --miniature -s <taille> <RAW> <PNG>");
        return 2;
    };
    match engine::io::raw_thumbnail_png(input, size).and_then(|png| std::fs::write(output, png).map_err(engine::EngineError::io(output))) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("miniature impossible : {error}");
            1
        }
    }
}

/// Instance unique sous Linux : passe par le bus de session D-Bus, et l'extension panique (unwrap) si son adresse est
/// illisible, au lieu de s'en passer. C'est le cas dans les bacs à sable sans D-Bus (« unsupported transport
/// 'disabled' » : firejail du test d'AppImageHub, certains conteneurs) : Lumitype se fermait aussitôt. Sans bus,
/// pas d'instance unique : chaque lancement ouvre sa fenêtre, comme la visionneuse.
fn session_bus_usable() -> bool {
    #[cfg(target_os = "linux")]
    {
        zbus::Address::session().is_ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}
