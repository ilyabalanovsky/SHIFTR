use crate::{
    engines::resolve_output_path,
    models::{
        category_for_extension, default_presets, extension_for_path, ConversionJob, FileCategory, JobStatus,
        ProbeResult, QueueOptions, SupportedFormats, AUDIO_FORMATS, IMAGE_FORMATS, VIDEO_FORMATS,
    },
    queue::{default_parallelism, QueueState},
};
use std::{path::Path, sync::Arc};
use tauri::{AppHandle, State};
use uuid::Uuid;

#[tauri::command]
pub fn get_supported_formats() -> SupportedFormats {
    SupportedFormats {
        video: VIDEO_FORMATS.iter().map(|item| item.to_string()).collect(),
        audio: AUDIO_FORMATS.iter().map(|item| item.to_string()).collect(),
        image: IMAGE_FORMATS.iter().map(|item| item.to_string()).collect(),
        presets: default_presets(),
        default_parallelism: default_parallelism(),
    }
}

#[tauri::command]
pub async fn probe_file(path: String) -> Result<ProbeResult, String> {
    let source_format = extension_for_path(&path);
    let category = category_for_extension(&source_format);
    let file_name = Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Unknown file")
        .to_string();
    let mut warnings = Vec::new();
    if category == FileCategory::Unsupported {
        warnings.push(format!("{source_format} is not supported in the media-first v1 release"));
    }

    Ok(ProbeResult {
        path,
        file_name,
        source_format,
        category,
        duration_seconds: None,
        streams: Vec::new(),
        warnings,
    })
}

#[tauri::command]
pub async fn create_jobs(
    paths: Vec<String>,
    options: QueueOptions,
    state: State<'_, Arc<QueueState>>,
) -> Result<Vec<ConversionJob>, String> {
    let mut jobs = Vec::new();

    for path in paths {
        let source_format = extension_for_path(&path);
        let category = category_for_extension(&source_format);
        let output_path = resolve_output_path(
            &path,
            options.output_dir.as_deref(),
            &options.target_format,
            &options.preset.overwrite_policy,
        )
        .map_err(|error| error.to_string())?;

        jobs.push(ConversionJob {
            id: Uuid::new_v4().to_string(),
            input_path: path,
            output_path,
            source_format,
            target_format: options.target_format.clone(),
            category,
            preset: options.preset.clone(),
            status: JobStatus::Queued,
            progress: 0.0,
            speed: None,
            eta_seconds: None,
            error: None,
        });
    }

    state.set_jobs(jobs.clone()).await;
    Ok(jobs)
}

#[tauri::command]
pub async fn start_queue(
    app: AppHandle,
    options: QueueOptions,
    state: State<'_, Arc<QueueState>>,
) -> Result<Vec<ConversionJob>, String> {
    state
        .update_queued_options(&options)
        .await
        .map_err(|error| error.to_string())?;
    Ok(state.inner().clone().start(app, options).await)
}

#[tauri::command]
pub async fn update_queued_jobs(
    options: QueueOptions,
    state: State<'_, Arc<QueueState>>,
) -> Result<Vec<ConversionJob>, String> {
    state
        .update_queued_options(&options)
        .await
        .map_err(|error| error.to_string())?;
    Ok(state.jobs().await)
}

#[tauri::command]
pub fn pause_queue(state: State<'_, Arc<QueueState>>) -> Result<(), String> {
    state.pause();
    Ok(())
}

#[tauri::command]
pub async fn cancel_job(id: String, state: State<'_, Arc<QueueState>>) -> Result<Option<ConversionJob>, String> {
    Ok(state.cancel_job(&id).await)
}

#[tauri::command]
pub fn open_output_folder(path: String) -> Result<(), String> {
    let path = Path::new(&path);
    let folder = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };
    tauri_plugin_opener::open_path(folder.to_string_lossy().to_string(), None::<&str>)
        .map_err(|error| error.to_string())
}
