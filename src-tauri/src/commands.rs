use crate::{
    capabilities,
    engines::resolve_output_path,
    models::{
        category_for_extension, default_presets, extension_for_path, ConversionJob, CreateJobGroup, FileCategory, JobStatus,
        ConversionCapabilities, DocumentJobOptions, ProbeResult, QueueOptions, SupportedFormats, AUDIO_FORMATS, DOCUMENT_FORMATS, IMAGE_FORMATS, VIDEO_FORMATS,
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
        document: DOCUMENT_FORMATS.iter().map(|item| item.to_string()).collect(),
        presets: default_presets(),
        default_parallelism: default_parallelism(),
    }
}

#[tauri::command]
pub fn get_conversion_capabilities(ffmpeg_path: Option<String>) -> ConversionCapabilities {
    capabilities::get_capabilities(ffmpeg_path)
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
            input_path: path.clone(),
            input_paths: vec![path],
            output_path,
            source_format,
            target_format: options.target_format.clone(),
            category,
            preset: options.preset.clone(),
            advanced_options: options.advanced_options.clone(),
            document_operation: None,
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
pub async fn create_jobs_batch(
    groups: Vec<CreateJobGroup>,
    state: State<'_, Arc<QueueState>>,
) -> Result<Vec<ConversionJob>, String> {
    let mut jobs = Vec::new();

    for group in groups {
        for path in group.paths {
            let source_format = extension_for_path(&path);
            let category = category_for_extension(&source_format);
            let output_path = resolve_output_path(
                &path,
                group.options.output_dir.as_deref(),
                &group.options.target_format,
                &group.options.preset.overwrite_policy,
            )
            .map_err(|error| error.to_string())?;

            jobs.push(ConversionJob {
                id: Uuid::new_v4().to_string(),
                input_path: path.clone(),
                input_paths: vec![path],
                output_path,
                source_format,
                target_format: group.options.target_format.clone(),
                category,
                preset: group.options.preset.clone(),
                advanced_options: group.options.advanced_options.clone(),
                document_operation: None,
                status: JobStatus::Queued,
                progress: 0.0,
                speed: None,
                eta_seconds: None,
                error: None,
            });
        }
    }

    state.add_jobs(jobs.clone()).await;
    Ok(state.jobs().await)
}

#[tauri::command]
pub async fn create_document_job(
    options: DocumentJobOptions,
    state: State<'_, Arc<QueueState>>,
) -> Result<Vec<ConversionJob>, String> {
    let first_path = options.paths.first().ok_or_else(|| "No document inputs selected".to_string())?.clone();
    validate_document_inputs(&options)?;
    let output_dir = options
        .output_dir
        .clone()
        .or_else(|| Path::new(&first_path).parent().map(|path| path.to_string_lossy().to_string()))
        .ok_or_else(|| "Cannot resolve output directory".to_string())?;
    let output_name = options.output_name.unwrap_or_else(|| match options.operation {
        crate::models::DocumentOperation::ImagesToPdf => "images.pdf".into(),
        crate::models::DocumentOperation::MergePdfs => "merged.pdf".into(),
    });
    let output_path = resolve_document_output_path(&output_dir, &output_name)?;

    let job = ConversionJob {
        id: Uuid::new_v4().to_string(),
        input_path: first_path,
        input_paths: options.paths,
        output_path: output_path.to_string_lossy().to_string(),
        source_format: "documents".into(),
        target_format: "pdf".into(),
        category: FileCategory::Document,
        preset: default_presets().into_iter().next().ok_or_else(|| "Missing default preset".to_string())?,
        advanced_options: None,
        document_operation: Some(options.operation),
        status: JobStatus::Queued,
        progress: 0.0,
        speed: None,
        eta_seconds: None,
        error: None,
    };

    state.add_jobs(vec![job]).await;
    Ok(state.jobs().await)
}

fn ensure_pdf_extension(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".pdf") {
        name.into()
    } else {
        format!("{name}.pdf")
    }
}

fn validate_document_inputs(options: &DocumentJobOptions) -> Result<(), String> {
    for path in &options.paths {
        let category = category_for_extension(&extension_for_path(path));
        match options.operation {
            crate::models::DocumentOperation::ImagesToPdf if category != FileCategory::Image => {
                return Err(format!("{} is not an image input", path));
            }
            crate::models::DocumentOperation::MergePdfs if category != FileCategory::Document => {
                return Err(format!("{} is not a PDF input", path));
            }
            _ => {}
        }
    }
    Ok(())
}

fn resolve_document_output_path(output_dir: &str, output_name: &str) -> Result<std::path::PathBuf, String> {
    let base_path = Path::new(output_dir).join(ensure_pdf_extension(output_name));
    if !base_path.exists() {
        return Ok(base_path);
    }

    let stem = base_path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Invalid output file name".to_string())?;
    let parent = base_path.parent().ok_or_else(|| "Invalid output directory".to_string())?;

    for index in 1..1000 {
        let candidate = parent.join(format!("{stem} ({index}).pdf"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err("Could not find a free output file name".into())
}

#[tauri::command]
pub async fn start_queue(
    app: AppHandle,
    options: QueueOptions,
    state: State<'_, Arc<QueueState>>,
) -> Result<Vec<ConversionJob>, String> {
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
