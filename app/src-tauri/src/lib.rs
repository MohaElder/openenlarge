mod commands;
mod convert;
mod encode;
mod metadata;
mod session;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(session::Session::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::import_image,
            commands::develop_image,
            commands::set_quality,
            commands::render_view,
            commands::export_image,
        ])
        .run(tauri::generate_context!())
        .expect("error while running RedRoom");
}
