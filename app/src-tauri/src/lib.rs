mod cache;
mod catalog;
mod commands;
mod convert;
mod encode;
mod exif_write;
mod gpu_upload;
mod metadata;
mod session;

#[cfg(test)]
pub mod commands_test_support {
    use crate::session::InvertParams;
    /// A neutral InvertParams for tests (mirrors default_invert_params()).
    pub fn sample_invert_params() -> InvertParams {
        InvertParams {
            mode: "b".into(), stock: "none".into(), base_rect: None,
            exposure: 0.0, black: 0.0, gamma: 0.4545, auto_wb: true,
            temp: 5500.0, tint: 0.0,
            contrast: 0.0, highlights: 0.0, shadows: 0.0, whites: 0.0, blacks: 0.0,
            texture: 0.0, vibrance: 0.0, saturation: 0.0,
            tc_highlights: 0.0, tc_lights: 0.0, tc_darks: 0.0, tc_shadows: 0.0,
            tc_curve: crate::session::identity_curve(),
            tc_red: crate::session::identity_curve(),
            tc_green: crate::session::identity_curve(),
            tc_blue: crate::session::identity_curve(),
            cg_sh_hue: 0.0, cg_sh_sat: 0.0, cg_sh_lum: 0.0,
            cg_mid_hue: 0.0, cg_mid_sat: 0.0, cg_mid_lum: 0.0,
            cg_hi_hue: 0.0, cg_hi_sat: 0.0, cg_hi_lum: 0.0,
            cg_glob_hue: 0.0, cg_glob_sat: 0.0, cg_glob_lum: 0.0,
            cg_blending: 50.0, cg_balance: 0.0,
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(session::Session::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
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
            }
            let dir = app.path().app_data_dir().expect("app data dir");
            std::fs::create_dir_all(&dir).map_err(|e| format!("create app data dir: {e}"))?;
            let cache_dir = dir.join("cache");
            std::fs::create_dir_all(&cache_dir).map_err(|e| format!("create cache dir: {e}"))?;
            *app.state::<session::Session>().cache_dir.lock().unwrap() = cache_dir;
            let db_path = dir.join("catalog.db");
            let catalog = catalog::Catalog::open(&db_path)
                .unwrap_or_else(|e| panic!("open catalog db at {}: {e}", db_path.display()));
            app.manage(catalog);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::import_image,
            commands::develop_image,
            commands::set_quality,
            commands::delete_image,
            commands::render_view,
            commands::thumbnail,
            commands::export_image,
            commands::as_shot_wb,
            commands::load_catalog,
            commands::save_edits,
            commands::save_crop,
            commands::save_dust,
            commands::save_meta,
            commands::save_pref,
            commands::save_app_state,
            commands::working_info,
            commands::working_pixels,
            commands::resolved_inversion,
        ])
        .run(tauri::generate_context!())
        .expect("error while running OpenEnlarge");
}
