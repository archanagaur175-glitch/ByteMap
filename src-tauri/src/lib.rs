//! bytemap library: modules are here so integration tests can reach them.
//! The binary in main.rs only calls `run()`.

pub mod commands;
pub mod state;
pub mod view;

use state::SharedSession;

pub fn run() {
    tauri::Builder::default()
        .manage(SharedSession::default())
        .invoke_handler(tauri::generate_handler![
            commands::start_scan,
            commands::cancel_scan,
            commands::node_view,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ByteMap");
}
