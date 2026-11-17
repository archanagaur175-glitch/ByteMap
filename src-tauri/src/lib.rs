//! bytemap library: modules are here so integration tests can reach them.
//! The binary in main.rs only calls `run()`.

pub mod commands;
pub mod pathres;
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
            commands::node_orbit,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running ByteMap");
}
