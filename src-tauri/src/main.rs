mod commands;
mod engines;
mod models;
mod queue;

use queue::QueueState;
use std::sync::Arc;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::new(QueueState::default()))
        .invoke_handler(tauri::generate_handler![
            commands::get_supported_formats,
            commands::probe_file,
            commands::create_jobs,
            commands::update_queued_jobs,
            commands::start_queue,
            commands::pause_queue,
            commands::cancel_job,
            commands::open_output_folder
        ])
        .run(tauri::generate_context!())
        .expect("failed to run SHIFTR");
}
