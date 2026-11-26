//! Serica browser backend (Rust + Tauri v2 + WebView2).

mod app;
mod backgrounds;
mod bookmarks;
mod browser_import;
mod default_browser;
mod commands;
mod downloads;
mod extensions;
mod files;
mod focus;
mod focus_stats;
mod history;
mod importer;
mod local_ai;
mod mcp;
mod mcp_http;
mod model;
mod perf;
mod previews;
mod settings;
mod shields;
mod store;
mod tabhost;
mod ui_patch;
mod updater;
mod vpn;
mod widgets;
mod window_state;
#[cfg(windows)]
mod clip_win;
#[cfg(windows)]
mod edge_watch;
#[cfg(windows)]
mod webview_win;
#[cfg(windows)]
mod dragdrop_win;

use tauri::Manager;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Serica was called Neutrino up to v0.4.1 (identifier `com.neutrino.browser`).
/// Moves an existing Neutrino profile (tabs, settings, logins, and the UI's
/// localStorage) to the Serica folders once, before any webview is created.
#[cfg(windows)]
fn migrate_legacy_data() {
    for var in ["APPDATA", "LOCALAPPDATA"] {
        let Some(base) = std::env::var_os(var).map(std::path::PathBuf::from) else { continue };
        let old = base.join("com.neutrino.browser");
        let new = base.join("com.serica.browser");
        if !old.is_dir() || new.exists() {
            continue;
        }
        match std::fs::rename(&old, &new) {
            Ok(()) => tracing::info!("migrated {} to {}", old.display(), new.display()),
            Err(e) => tracing::warn!("could not migrate {}: {e}", old.display()),
        }
    }
}

pub fn run() {
    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .try_init();
    #[cfg(windows)]
    migrate_legacy_data();

    // Serve a downloaded UI fix (see ui_patch.rs) instead of the built-in UI.
    let mut context = tauri::generate_context!();
    ui_patch::install(&mut context);

    let app = tauri::Builder::default()
        // Must be first: a second launch (e.g. double-clicking a PDF while
        // Serica is open) hands its arguments to this instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(shared) = app.try_state::<app::Shared>() {
                files::open_args(&shared, argv.into_iter().skip(1));
            }
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        // No click script: it is injected into every webview, tabs included,
        // and swallows left-clicks on target=_blank links (the page may not
        // call the opener), so they did nothing. Tabs handle those links in
        // `on_new_window`.
        .plugin(tauri_plugin_opener::Builder::new().open_js_links_on_click(false).build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Pictures chosen for the new-tab background.
        .register_uri_scheme_protocol("sericabg", |ctx, request| backgrounds::serve(ctx.app_handle(), &request))
        // Page captures: only the app's own UI may read them, never a web page.
        .register_uri_scheme_protocol("sericacap", |ctx, request| match ctx.app_handle().try_state::<app::Shared>() {
            Some(app) if ctx.webview_label() == "main" => app.previews.serve(&request),
            _ => tauri::http::Response::builder().status(404).body(Vec::new()).unwrap_or_default(),
        })
        .setup(|app| {
            perf::start(app.handle().clone());
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("com.serica.browser"));
            if let Some(window) = app.get_webview_window("main") {
                window_state::attach(&window, data_dir.join("window.json"));
            }
            let shared = app::App::new(app.handle().clone(), data_dir);
            #[cfg(windows)]
            edge_watch::start(shared.clone());
            let scale = shared.settings().text_scale;
            if (scale - 1.0).abs() > 0.001 {
                commands::apply_ui_zoom(&shared, scale);
            }
            files::open_args(&shared, std::env::args().skip(1));
            // Files, links and text dropped on Serica's own interface.
            #[cfg(windows)]
            if let Some(main) = app.get_webview("main") {
                dragdrop_win::install(&shared, &main);
            }
            app.manage(shared);
            updater::start(app.handle().clone());
            ui_patch::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_browser_state,
            commands::get_ui_patch_status,
            commands::confirm_ui_patch,
            commands::apply_ui_patch,
            commands::page_menu_select,
            commands::widget_github,
            commands::widget_geocode,
            commands::widget_weather,
            commands::get_pdf_dark_tab,
            commands::set_pdf_dark_tab,
            commands::create_tab,
            commands::create_private_tab,
            commands::close_tab,
            commands::select_tab,
            commands::reopen_closed_tab,
            commands::cycle_tab,
            commands::update_tab,
            commands::toggle_pin_tab,
            commands::pin_tab,
            commands::unpin_tab,
            commands::add_pinned_sites,
            commands::toggle_sleep_tab,
            commands::reorder_tabs,
            commands::duplicate_tab,
            commands::move_tab,
            commands::add_favorite,
            commands::remove_favorite,
            commands::create_folder,
            commands::update_folder,
            commands::delete_folder,
            commands::move_tab_to_folder,
            commands::navigate,
            commands::site_action,
            commands::open_external_url,
            commands::answer_page_dialog,
            commands::answer_permission,
            commands::open_dropped_paths,
            commands::set_viewport,
            commands::viewport_capture,
            commands::set_viewport_hole,
            commands::ui_cursor,
            commands::viewport_freeze,
            commands::viewport_thaw,
            commands::find_in_page,
            commands::set_zoom_factor,
            commands::capture_tab_preview,
            commands::get_tab_preview,
            commands::open_devtools,
            commands::get_media_state,
            commands::media_control,
            commands::select_space,
            commands::create_space,
            commands::delete_space,
            commands::update_space,
            commands::reorder_space,
            commands::list_profiles,
            commands::create_profile,
            commands::rename_profile,
            commands::delete_profile,
            commands::list_archive,
            commands::restore_archived_tab,
            commands::get_settings,
            commands::update_settings,
            commands::set_theme_mode,
            commands::get_performance_status,
            commands::toggle_shields,
            commands::toggle_site_whitelist,
            commands::get_shield_stats,
            commands::reset_shield_stats,
            commands::get_shields_info,
            commands::update_filter_lists,
            commands::zen_optimize,
            commands::window_control,
            commands::get_history,
            commands::open_file,
            commands::read_clipboard_text,
            commands::cookie_sites,
            commands::delete_cookies,
            commands::history_suggest,
            commands::delete_history_entry,
            commands::clear_history,
            commands::get_vpn_status,
            commands::connect_vpn,
            commands::disconnect_vpn,
            commands::update_vpn_config,
            commands::check_vpn_ip,
            commands::mcp_call,
            commands::get_mcp_logs,
            commands::mcp_server_status,
            commands::mcp_regenerate_token,
            commands::assistant_open,
            commands::assistant_bounds,
            commands::assistant_close,
            commands::assistant_insert_text,
            commands::passwords_status,
            commands::passwords_list,
            commands::list_downloads,
            commands::download_action,
            commands::start_download_drag,
            commands::clear_downloads,
            commands::open_downloads_folder,
            commands::list_extensions,
            commands::install_extension,
            commands::set_extension_enabled,
            commands::remove_extension,
            commands::open_extension_page,
            commands::get_update_status,
            commands::check_for_update,
            commands::install_update,
            focus::focus_get,
            focus::focus_save_block,
            focus::focus_delete_block,
            focus::focus_start,
            focus::focus_stop,
            focus::focus_challenge,
            focus::focus_check,
            focus_stats::focus_stats,
            local_ai::local_ai_scan,
            local_ai::local_ai_chat,
            local_ai::local_ai_stop,
            focus_stats::focus_stats_tracking,
            focus_stats::focus_stats_clear,
            focus_stats::focus_stats_export,
            backgrounds::pick_background_image,
            default_browser::default_browser_status,
            default_browser::make_default_browser,
            browser_import::import_sources,
            browser_import::import_from_browser,
            browser_import::import_passwords_file,
        ])
        .build(context)
        .expect("error while building serica browser");

    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            if let Some(shared) = handle.try_state::<app::Shared>() {
                shared.notify(app::Notice::Quit);
                shared.save_now();
                shared.focus.save(true);
                shared.focus_stats.save(true);
                tabhost::TabHost::close_all(&shared);
            }
        }
    });
}
