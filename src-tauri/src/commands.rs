use crate::{
    capabilities,
    engines::resolve_output_path,
    models::{
        AUDIO_FORMATS, ConversionCapabilities, ConversionJob, CreateJobGroup, DOCUMENT_FORMATS,
        DocumentJobOptions, EncodingPreset, FileCategory, IMAGE_FORMATS, JobStatus, ProbeResult,
        QueueOptions, RenameOutputOptions, SizeTargetFileEstimate, SizeTargetValidation,
        SizeTargetValidationRequest, SupportedFormats, VIDEO_FORMATS, category_for_extension,
        default_presets, extension_for_path,
    },
    presets,
    queue::{QueueState, default_parallelism},
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use tauri::{AppHandle, State};
use uuid::Uuid;

#[tauri::command]
pub fn get_supported_formats() -> SupportedFormats {
    SupportedFormats {
        video: VIDEO_FORMATS.iter().map(|item| item.to_string()).collect(),
        audio: AUDIO_FORMATS.iter().map(|item| item.to_string()).collect(),
        image: IMAGE_FORMATS.iter().map(|item| item.to_string()).collect(),
        document: DOCUMENT_FORMATS
            .iter()
            .map(|item| item.to_string())
            .collect(),
        presets: default_presets(),
        default_parallelism: default_parallelism(),
    }
}

#[tauri::command]
pub fn get_encoding_presets(app: AppHandle) -> Result<Vec<EncodingPreset>, String> {
    presets::all_presets(&app).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_custom_encoding_preset(
    app: AppHandle,
    preset: EncodingPreset,
) -> Result<Vec<EncodingPreset>, String> {
    presets::save_custom_preset(&app, preset).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_custom_encoding_preset(
    app: AppHandle,
    id: String,
) -> Result<Vec<EncodingPreset>, String> {
    presets::delete_custom_preset(&app, &id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_conversion_capabilities(ffmpeg_path: Option<String>) -> ConversionCapabilities {
    capabilities::get_capabilities(ffmpeg_path)
}

#[tauri::command]
pub fn validate_size_target(
    request: SizeTargetValidationRequest,
) -> Result<SizeTargetValidation, String> {
    if request.target_size_mb < 1 || request.target_size_mb > 10_240 {
        return Ok(SizeTargetValidation {
            applicable: false,
            warnings: vec!["Choose a size between 1 MB and 10 GB.".into()],
            estimates: Vec::new(),
        });
    }

    if !matches!(request.category, FileCategory::Video | FileCategory::Audio) {
        return Ok(SizeTargetValidation {
            applicable: false,
            warnings: vec!["Size target is available for video and audio conversions.".into()],
            estimates: Vec::new(),
        });
    }

    let ffprobe = find_ffprobe(request.ffmpeg_path.as_deref());
    let mut estimates = Vec::new();
    let mut warnings = Vec::new();

    for path in &request.paths {
        let duration = probe_duration_for_validation(&ffprobe, path);
        let estimate = size_target_estimate(
            path,
            request.category.clone(),
            request.target_size_mb,
            request.audio_bitrate.as_deref(),
            duration,
        );
        if let Some(warning) = &estimate.warning {
            warnings.push(format!("{}: {warning}", file_label(path)));
        }
        estimates.push(estimate);
    }

    Ok(SizeTargetValidation {
        applicable: estimates.iter().all(|estimate| estimate.applicable),
        warnings,
        estimates,
    })
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
        warnings.push(format!(
            "{source_format} is not supported in the media-first v1 release"
        ));
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
    let mut reserved_paths = HashSet::new();

    for path in paths {
        let source_format = extension_for_path(&path);
        let category = category_for_extension(&source_format);
        let output_path = resolve_reserved_output_path(
            &path,
            options.output_dir.as_deref(),
            &options.target_format,
            &options.preset.overwrite_policy,
            &mut reserved_paths,
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
            processing_seconds: None,
            eta_seconds: None,
            error: None,
            error_details: None,
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
    let mut reserved_paths = state
        .jobs()
        .await
        .into_iter()
        .map(|job| normalized_path_key(&job.output_path))
        .collect::<HashSet<_>>();

    for group in groups {
        for path in group.paths {
            let source_format = extension_for_path(&path);
            let category = category_for_extension(&source_format);
            let output_path = resolve_reserved_output_path(
                &path,
                group.options.output_dir.as_deref(),
                &group.options.target_format,
                &group.options.preset.overwrite_policy,
                &mut reserved_paths,
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
                processing_seconds: None,
                eta_seconds: None,
                error: None,
                error_details: None,
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
    let first_path = options
        .paths
        .first()
        .ok_or_else(|| "No document inputs selected".to_string())?
        .clone();
    validate_document_inputs(&options)?;
    let output_dir = options
        .output_dir
        .clone()
        .or_else(|| {
            Path::new(&first_path)
                .parent()
                .map(|path| path.to_string_lossy().to_string())
        })
        .ok_or_else(|| "Cannot resolve output directory".to_string())?;
    let output_name = options
        .output_name
        .unwrap_or_else(|| match options.operation {
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
        preset: default_presets()
            .into_iter()
            .next()
            .ok_or_else(|| "Missing default preset".to_string())?,
        advanced_options: None,
        document_operation: Some(options.operation),
        status: JobStatus::Queued,
        progress: 0.0,
        speed: None,
        processing_seconds: None,
        eta_seconds: None,
        error: None,
        error_details: None,
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

fn resolve_document_output_path(
    output_dir: &str,
    output_name: &str,
) -> Result<std::path::PathBuf, String> {
    let base_path = Path::new(output_dir).join(ensure_pdf_extension(output_name));
    if !base_path.exists() {
        return Ok(base_path);
    }

    let stem = base_path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Invalid output file name".to_string())?;
    let parent = base_path
        .parent()
        .ok_or_else(|| "Invalid output directory".to_string())?;

    for index in 1..1000 {
        let candidate = parent.join(format!("{stem} ({index}).pdf"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err("Could not find a free output file name".into())
}

fn resolve_reserved_output_path(
    input_path: &str,
    output_dir: Option<&str>,
    target_format: &str,
    overwrite: &crate::models::OverwritePolicy,
    reserved_paths: &mut HashSet<String>,
) -> anyhow::Result<String> {
    let output_path = resolve_output_path(input_path, output_dir, target_format, overwrite)?;
    let path = Path::new(&output_path);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot resolve output directory"))?;
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("Output file name is invalid"))?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or_else(|| anyhow::anyhow!("Output file extension is invalid"))?;

    for index in 0..1000 {
        let candidate = if index == 0 {
            PathBuf::from(path)
        } else {
            parent.join(format!("{stem} ({index}).{extension}"))
        };
        let candidate_text = candidate.to_string_lossy().to_string();
        let key = normalized_path_key(&candidate_text);
        if !reserved_paths.contains(&key)
            && !candidate.exists()
            && normalized_path_key(&candidate_text) != normalized_path_key(input_path)
        {
            reserved_paths.insert(key);
            return Ok(candidate_text);
        }
    }

    Err(anyhow::anyhow!("Could not find a free output file name"))
}

fn normalized_path_key(path: &str) -> String {
    if cfg!(windows) {
        path.to_ascii_lowercase()
    } else {
        path.to_string()
    }
}

fn find_ffprobe(ffmpeg_path: Option<&str>) -> String {
    if let Some(path) = ffmpeg_path.filter(|path| Path::new(path).exists()) {
        return path.replace("ffmpeg", "ffprobe");
    }

    let exe = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    let bundled = Path::new("resources").join("bin").join(exe);
    if bundled.exists() {
        return bundled.to_string_lossy().to_string();
    }

    exe.into()
}

fn probe_duration_for_validation(ffprobe: &str, path: &str) -> Option<f64> {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            path,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|duration| *duration > 0.0)
}

fn size_target_estimate(
    path: &str,
    category: FileCategory,
    target_size_mb: u32,
    audio_bitrate: Option<&str>,
    duration: Option<f64>,
) -> SizeTargetFileEstimate {
    let Some(duration) = duration else {
        return SizeTargetFileEstimate {
            path: path.into(),
            duration_seconds: None,
            total_kbps: None,
            video_kbps: None,
            audio_kbps: None,
            applicable: false,
            warning: Some(
                "Could not read duration, so SHIFTR cannot validate this target size.".into(),
            ),
        };
    };

    let total_kbps = ((f64::from(target_size_mb) * 8192.0) / duration * 0.94).floor() as u32;
    match category {
        FileCategory::Audio => {
            let audio_kbps = total_kbps.clamp(32, 320);
            let warning = if total_kbps < 48 {
                Some(
                    "The requested size is likely too small; audio quality may be very poor."
                        .into(),
                )
            } else if total_kbps > 320 {
                Some(
                    "The requested size is larger than useful for common lossy audio bitrates."
                        .into(),
                )
            } else {
                None
            };
            SizeTargetFileEstimate {
                path: path.into(),
                duration_seconds: Some(duration),
                total_kbps: Some(total_kbps),
                video_kbps: None,
                audio_kbps: Some(audio_kbps),
                applicable: true,
                warning,
            }
        }
        FileCategory::Video => {
            let requested_audio = audio_bitrate
                .and_then(|value| value.trim_end_matches('k').parse::<u32>().ok())
                .unwrap_or(160);
            let audio_kbps = requested_audio.min(total_kbps.saturating_sub(160)).max(64);
            let video_kbps = total_kbps.saturating_sub(audio_kbps);
            let warning = if video_kbps < 180 {
                Some("This target is probably too small for this video; quality may be extremely poor.".into())
            } else if video_kbps < 500 {
                Some(
                    "This target is tight for this video; expect visible compression artifacts."
                        .into(),
                )
            } else if video_kbps > 20_000 {
                Some(
                    "This target is very large; the output may not become meaningfully better."
                        .into(),
                )
            } else {
                None
            };
            SizeTargetFileEstimate {
                path: path.into(),
                duration_seconds: Some(duration),
                total_kbps: Some(total_kbps),
                video_kbps: Some(video_kbps),
                audio_kbps: Some(audio_kbps),
                applicable: video_kbps >= 120,
                warning,
            }
        }
        _ => SizeTargetFileEstimate {
            path: path.into(),
            duration_seconds: Some(duration),
            total_kbps: Some(total_kbps),
            video_kbps: None,
            audio_kbps: None,
            applicable: false,
            warning: Some("Size target is not available for this file type.".into()),
        },
    }
}

fn file_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_string()
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
pub async fn cancel_job(
    id: String,
    state: State<'_, Arc<QueueState>>,
) -> Result<Option<ConversionJob>, String> {
    Ok(state.cancel_job(&id).await)
}

#[tauri::command]
pub async fn retry_job(
    id: String,
    state: State<'_, Arc<QueueState>>,
) -> Result<ConversionJob, String> {
    state
        .retry_job(&id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn remove_job(
    id: String,
    state: State<'_, Arc<QueueState>>,
) -> Result<Vec<ConversionJob>, String> {
    Ok(state.remove_job(&id).await)
}

#[tauri::command]
pub async fn clear_finished_jobs(
    state: State<'_, Arc<QueueState>>,
) -> Result<Vec<ConversionJob>, String> {
    Ok(state.clear_finished().await)
}

#[tauri::command]
pub async fn reset_queue(state: State<'_, Arc<QueueState>>) -> Result<(), String> {
    state.clear_all().await;
    Ok(())
}

#[tauri::command]
pub async fn rename_job_output(
    options: RenameOutputOptions,
    state: State<'_, Arc<QueueState>>,
) -> Result<ConversionJob, String> {
    state
        .rename_output_file(&options.id, &options.output_name)
        .await
        .map_err(|error| error.to_string())
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
