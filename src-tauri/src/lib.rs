//! bytemap library: modules are here so integration tests can reach them.
//! The binary in main.rs only calls `run()`.

pub mod backdrop;
pub mod commands;
pub mod pathres;
pub mod refresh;
pub mod snapshot_store;
pub mod state;
pub mod view;

use state::SharedSession;

pub fn run() {
    tauri::Builder::default()
        .manage(SharedSession::default())
        .setup(|app| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                // Dark is the default theme; the UI re-tints on switch.
                let material = backdrop::store(&window, true);
                println!("[backdrop] {material}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_scan,
            commands::cancel_scan,
            commands::ensure_helper,
            commands::node_view,
            commands::node_orbit,
            commands::node_strata,
            commands::bench_report,
            commands::basket_add,
            commands::basket_remove,
            commands::basket_clear,
            commands::basket_list,
            commands::dry_run_basket,
            commands::execute_recycle,
            commands::execute_permanent,
            commands::execute_archive,
            commands::undo_last,
            snapshot_store::snapshot_save,
            snapshot_store::snapshot_list,
            snapshot_store::snapshot_load,
            snapshot_store::snapshot_delete,
            snapshot_store::snapshot_diff,
            refresh::snapshot_refresh,
            backdrop::backdrop_state,
            backdrop::set_backdrop_theme,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ByteMap");
}
