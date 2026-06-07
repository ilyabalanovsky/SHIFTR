mod capabilities;
mod commands;
mod documents;
mod engines;
mod models;
mod presets;
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
            commands::get_encoding_presets,
            commands::save_custom_encoding_preset,
            commands::delete_custom_encoding_preset,
            commands::get_conversion_capabilities,
            commands::validate_size_target,
            commands::probe_file,
            commands::create_jobs,
            commands::create_jobs_batch,
            commands::create_document_job,
            commands::update_queued_jobs,
            commands::start_queue,
            commands::pause_queue,
            commands::cancel_job,
            commands::retry_job,
            commands::remove_job,
            commands::clear_finished_jobs,
            commands::reset_queue,
            commands::rename_job_output,
            commands::open_output_folder
        ])
        .run(tauri::generate_context!())
        .expect("failed to run SHIFTR");
}
