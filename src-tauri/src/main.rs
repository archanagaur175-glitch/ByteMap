// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;
mod view;

fn main() {
    tauri::Builder::default()
        .manage(state::SharedSession::default())
        .invoke_handler(tauri::generate_handler![
            commands::start_scan,
            commands::cancel_scan,
            commands::node_view,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ByteMap");
}
