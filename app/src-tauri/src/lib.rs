mod ai_enhance;
mod autodust;
mod cache;
mod color_match;
mod catalog;
mod commands;
mod debug_log;
mod convert;
mod encode;
mod exif_write;
mod gpu_upload;
mod hdr;
mod hdr_surface;
mod metadata;
mod session;
mod telemetry;
mod tether;
mod upscale;

#[cfg(test)]
pub mod commands_test_support {
    /// A neutral InvertParams for tests (delegates to commands::default_invert_params).
    pub fn sample_invert_params() -> crate::session::InvertParams {
        crate::commands::default_invert_params()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // The Aptabase plugin's flush loop spawns onto a Tokio runtime from Tauri's
    // (synchronous) setup hook, which runs on the main thread with no runtime
    // context — so without this it panics ("no reactor running"). Own a runtime
    // and enter its context for the whole app lifetime; the guard + runtime are
    // held in locals that live until run() returns.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");
    let _runtime_guard = runtime.enter();

    // Public Aptabase client key. It ships in the binary regardless (so it is not
    // a secret), but `option_env!` lets CI inject a per-environment key — set the
    // APTABASE_KEY build env to override (e.g. a separate dev project).
    let aptabase_key = option_env!("APTABASE_KEY").unwrap_or("A-US-7946890855");

    tauri::Builder::default()
        .manage(session::Session::default())
        .manage(tether::TetherState::default())
        .manage(telemetry::TelemetryState::default())
        .manage(hdr_surface::HdrSurfaceState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_aptabase::Builder::new(aptabase_key).build())
        .setup(|app| {
            use tauri::Manager;
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(Some(monitor)) = win.primary_monitor() {
                    let size = monitor.size();
                    let scale = monitor.scale_factor();
                    let w = (size.width as f64 * 0.9) / scale;
                    let h = (size.height as f64 * 0.9) / scale;
                    let _ = win.set_size(tauri::LogicalSize::new(w, h));
                    let _ = win.center();
                }
                let _ = win.show();
                // macOS: sizing the window while it was still hidden leaves the
                // WKWebView's event region stuck at the original (config) size, so
                // part of the UI paints but silently ignores clicks — they fall
                // through and defocus the app — until the window is resized. Once the
                // window is realized, a programmatic shrink-then-restore forces the
                // webview to resync its event region to the full window.
                //
                // Two details are load-bearing (verified empirically): the delta must
                // be real — a 1px nudge gets coalesced and does nothing — and it must
                // run after the window is realized, hence the deferral.
                #[cfg(target_os = "macos")]
                {
                    let win = win.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(400));
                        if let Ok(sz) = win.inner_size() {
                            let _ = win.set_size(tauri::PhysicalSize::new(
                                sz.width.saturating_sub(80),
                                sz.height,
                            ));
                            std::thread::sleep(std::time::Duration::from_millis(150));
                            let _ = win.set_size(tauri::PhysicalSize::new(sz.width, sz.height));
                        }
                    });
                }
            }
            let dir = app.path().app_data_dir().expect("app data dir");
            std::fs::create_dir_all(&dir).map_err(|e| format!("create app data dir: {e}"))?;
            app.manage(debug_log::DebugLog::new(dir.join("debug.log")));
            let cache_dir = dir.join("cache");
            std::fs::create_dir_all(&cache_dir).map_err(|e| format!("create cache dir: {e}"))?;
            *app.state::<session::Session>().cache_dir.lock().unwrap() = cache_dir;
            let db_path = dir.join("catalog.db");
            let catalog = catalog::Catalog::open(&db_path)
                .unwrap_or_else(|e| panic!("open catalog db at {}: {e}", db_path.display()));
            // Seed analytics consent from the persisted choice so the gate is
            // already correct before the frontend hydrates — no window in which
            // an event could fire before we know whether the user opted in.
            if let Ok(prefs) = catalog.load_prefs() {
                if prefs.get("telemetry").map(|v| v == "on").unwrap_or(false) {
                    app.state::<telemetry::TelemetryState>().set(true);
                }
                if prefs.get("debug_mode").map(|v| v == "on").unwrap_or(false) {
                    let log = app.state::<debug_log::DebugLog>().inner().clone();
                    log.enable();
                    debug_log::install_panic_hook(log.clone());
                    debug_log::dlog_startup(&log);
                    let app2 = app.handle().clone();
                    debug_log::start_mem_sampler(log, move || {
                        cache::oecache_bytes(
                            &app2.state::<session::Session>().cache_dir.lock()
                                .map(|d| d.clone()).unwrap_or_default(),
                        )
                    });
                }
            }
            app.manage(catalog);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::import_image,
            commands::list_dir_files,
            commands::develop_image,
            commands::ensure_developed,
            commands::delete_image,
            commands::cache_size,
            commands::clear_image_cache,
            commands::reset_all_data,
            commands::render_view,
            commands::encode_hdr,
            commands::encode_hdr_raw,
            commands::hdr_surface_render_show,
            commands::hdr_surface_set_source,
            commands::hdr_surface_set_uniforms,
            commands::thumbnail,
            commands::save_thumbnail,
            commands::invalidate_thumbnails,
            commands::export_image,
            commands::export_image_hdr,
            commands::export_begin,
            commands::export_pixels,
            commands::export_finish,
            commands::paths_exist,
            commands::unique_path,
            commands::as_shot_wb,
            commands::per_zone_wb,
            commands::auto_brightness,
            commands::gray_point_wb,
            commands::load_catalog,
            commands::save_edits,
            commands::save_crop,
            commands::save_dust,
            commands::save_meta,
            commands::save_pref,
            commands::save_app_state,
            commands::working_info,
            commands::working_pixels,
            commands::working_baked_info,
            commands::working_baked_pixels,
            commands::resolved_inversion,
            commands::sample_base_at,
            commands::auto_base_info,
            commands::roll_base,
            commands::analyze,
            commands::analyze_white_point,
            commands::ai_enhance_image,
            commands::upscaler_status,
            commands::download_upscaler,
            commands::upscale_image,
            commands::upscale_enhanced,
            commands::save_upscaled,
            commands::save_enhanced,
            commands::color_match_params,
            commands::reference_thumb,
            commands::autodust_status,
            commands::download_autodust,
            commands::debug_set,
            commands::debug_log_append,
            commands::debug_clear,
            commands::save_log,
            hdr_surface::hdr_surface_show,
            hdr_surface::hdr_surface_set_rect,
            hdr_surface::hdr_surface_hide,
            telemetry::set_telemetry,
            telemetry::telemetry_event,
            tether::tether_start,
            tether::tether_stop,
        ])
        .build(tauri::generate_context!())
        .expect("error while running OpenEnlarge")
        .run(|app_handle, event| {
            // Flush queued analytics on quit so the session's last events aren't
            // lost. flush is harmless when nothing is queued / consent is off.
            if let tauri::RunEvent::Exit = event {
                use tauri::Manager;
                use tauri_plugin_aptabase::EventTracker;
                if app_handle.state::<telemetry::TelemetryState>().enabled() {
                    let _ = app_handle.track_event("app_exited", None);
                }
                app_handle.flush_events_blocking();
            }
        });
}
