use crate::{
    capabilities,
    engines::{build_ffmpeg_args, resolve_output_path},
    models::{
        AUDIO_FORMATS, ConversionCapabilities, ConversionJob, CreateJobGroup, DOCUMENT_FORMATS,
        DocumentJobOptions, EncodingPreset, FileCategory, IMAGE_FORMATS, JobStatus,
        OutputSizeEstimate, OutputSizeEstimateRequest, PreviewRequest, PreviewResult, ProbeResult,
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
pub fn export_custom_encoding_preset(
    app: AppHandle,
    id: String,
    path: String,
) -> Result<(), String> {
    presets::export_custom_preset(&app, &id, &path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn import_custom_encoding_presets(
    app: AppHandle,
    path: String,
) -> Result<Vec<EncodingPreset>, String> {
    presets::import_custom_presets(&app, &path).map_err(|error| error.to_string())
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
pub fn estimate_output_size(
    request: OutputSizeEstimateRequest,
) -> Result<OutputSizeEstimate, String> {
    if !matches!(request.category, FileCategory::Video | FileCategory::Audio) {
        return Ok(OutputSizeEstimate {
            source_size_bytes: None,
            estimated_min_bytes: None,
            estimated_max_bytes: None,
            estimated_output_size_bytes: None,
            estimated_delta_bytes: None,
            confidence: "Unavailable".into(),
            basis: "Size estimates are available for video and audio conversions.".into(),
        });
    }

    let source_size = sum_source_sizes(&request.paths);
    let Some(source_size) = source_size else {
        return Ok(OutputSizeEstimate {
            source_size_bytes: None,
            estimated_min_bytes: None,
            estimated_max_bytes: None,
            estimated_output_size_bytes: None,
            estimated_delta_bytes: None,
            confidence: "Unavailable".into(),
            basis: "Could not read source file sizes.".into(),
        });
    };

    let factor = rough_size_factor(
        &request.category,
        &request.target_format,
        &request.preset.quality_mode,
        request.advanced_options.as_ref(),
    );
    let estimated = ((source_size as f64) * factor).max(1.0).round() as u64;
    let spread = rough_size_spread(
        &request.preset.quality_mode,
        request.advanced_options.as_ref(),
    );
    let min = ((estimated as f64) * (1.0 - spread)).max(1.0).round() as u64;
    let max = ((estimated as f64) * (1.0 + spread))
        .max(min as f64)
        .round() as u64;

    Ok(OutputSizeEstimate {
        source_size_bytes: Some(source_size),
        estimated_min_bytes: Some(min),
        estimated_max_bytes: Some(max),
        estimated_output_size_bytes: Some(estimated),
        estimated_delta_bytes: Some(estimated as i64 - source_size as i64),
        confidence: rough_size_confidence(spread).into(),
        basis: "Rough estimate based on source size, selected format, codec, and preset.".into(),
    })
}

#[tauri::command]
pub async fn create_conversion_preview(request: PreviewRequest) -> Result<PreviewResult, String> {
    if !matches!(request.category, FileCategory::Video | FileCategory::Audio) {
        return Err("Preview is available for video and audio files.".into());
    }

    let ffmpeg = find_ffmpeg(request.ffmpeg_path.as_deref());
    let ffprobe = find_ffprobe(request.ffmpeg_path.as_deref());
    let duration = probe_duration_for_validation(&ffprobe, &request.path);
    let preview_seconds = preview_duration(duration);
    let temp_dir = std::env::temp_dir().join("shiftr-previews");
    std::fs::create_dir_all(&temp_dir).map_err(|error| error.to_string())?;
    let exact_preview_path = temp_dir.join(format!(
        "encoded-{}.{}",
        Uuid::new_v4(),
        request.target_format
    ));
    let playback_path = temp_dir.join(format!(
        "playback-{}.{}",
        Uuid::new_v4(),
        playback_preview_extension(&request.category)
    ));

    let job = ConversionJob {
        id: Uuid::new_v4().to_string(),
        input_path: request.path.clone(),
        input_paths: vec![request.path.clone()],
        output_path: exact_preview_path.to_string_lossy().to_string(),
        source_format: request.source_format.clone(),
        target_format: request.target_format.clone(),
        category: request.category.clone(),
        preset: request.preset,
        advanced_options: request.advanced_options,
        document_operation: None,
        status: JobStatus::Queued,
        progress: 0.0,
        speed: None,
        processing_seconds: None,
        eta_seconds: None,
        error: None,
        error_details: None,
    };

    let mut args = build_ffmpeg_args(&job);
    strip_progress_args(&mut args);
    let output_index = args.len().saturating_sub(1);
    args.splice(
        output_index..output_index,
        ["-t".into(), format!("{preview_seconds:.3}")],
    );

    let output = Command::new(&ffmpeg)
        .args(&args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    render_playback_preview(
        &ffmpeg,
        &request.path,
        &request.category,
        preview_seconds,
        &playback_path.to_string_lossy(),
    )?;

    let source_size = std::fs::metadata(&request.path)
        .ok()
        .map(|metadata| metadata.len());
    let preview_size = std::fs::metadata(&exact_preview_path)
        .ok()
        .map(|metadata| metadata.len());
    let estimated_output_size = match (duration, preview_size) {
        (Some(duration), Some(size)) if preview_seconds > 0.0 => {
            Some(((size as f64) * (duration / preview_seconds)).round() as u64)
        }
        _ => None,
    };
    let estimated_delta = match (estimated_output_size, source_size) {
        (Some(output), Some(source)) => Some(output as i64 - source as i64),
        _ => None,
    };

    Ok(PreviewResult {
        input_path: request.path,
        preview_path: playback_path.to_string_lossy().to_string(),
        waveform_path: None,
        duration_seconds: duration,
        preview_seconds,
        source_size_bytes: source_size,
        preview_size_bytes: preview_size,
        estimated_output_size_bytes: estimated_output_size,
        estimated_delta_bytes: estimated_delta,
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

fn find_ffmpeg(configured: Option<&str>) -> String {
    if let Some(path) = configured.filter(|path| Path::new(path).exists()) {
        return path.into();
    }

    let exe = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let bundled = Path::new("resources").join("bin").join(exe);
    if bundled.exists() {
        return bundled.to_string_lossy().to_string();
    }

    exe.into()
}

fn preview_duration(duration: Option<f64>) -> f64 {
    match duration {
        Some(value) if value <= 1.5 => value.max(0.5),
        Some(value) if value < 5.0 => (value / 2.0).clamp(0.75, value),
        Some(value) if value < 10.0 => (value / 3.0).clamp(1.0, 5.0),
        _ => 5.0,
    }
}

fn playback_preview_extension(category: &FileCategory) -> &'static str {
    match category {
        FileCategory::Video => "webm",
        FileCategory::Audio => "wav",
        _ => "mp4",
    }
}

fn strip_progress_args(args: &mut Vec<String>) {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "-progress" && index + 1 < args.len() {
            args.drain(index..=index + 1);
        } else if args[index] == "-nostats" {
            args.remove(index);
        } else {
            index += 1;
        }
    }
}

fn render_playback_preview(
    ffmpeg: &str,
    input_path: &str,
    category: &FileCategory,
    preview_seconds: f64,
    output_path: &str,
) -> Result<(), String> {
    let mut args = vec![
        "-hide_banner".to_string(),
        "-y".to_string(),
        "-t".to_string(),
        format!("{preview_seconds:.3}"),
        "-i".to_string(),
        input_path.to_string(),
    ];

    match category {
        FileCategory::Video => args.extend([
            "-map".into(),
            "0:v:0?".into(),
            "-map".into(),
            "0:a:0?".into(),
            "-c:v".into(),
            "libvpx-vp9".into(),
            "-deadline".into(),
            "realtime".into(),
            "-cpu-used".into(),
            "8".into(),
            "-b:v".into(),
            "0".into(),
            "-crf".into(),
            "36".into(),
            "-c:a".into(),
            "libopus".into(),
            "-b:a".into(),
            "96k".into(),
        ]),
        FileCategory::Audio => args.extend([
            "-vn".into(),
            "-c:a".into(),
            "pcm_s16le".into(),
            "-ar".into(),
            "44100".into(),
            "-ac".into(),
            "2".into(),
        ]),
        _ => return Err("Playback preview is available for video and audio files.".into()),
    }

    args.push(output_path.into());
    let output = Command::new(ffmpeg)
        .args(&args)
        .output()
        .map_err(|error| error.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
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

fn sum_source_sizes(paths: &[String]) -> Option<u64> {
    let mut total = 0_u64;
    let mut found = false;
    for path in paths {
        if let Ok(metadata) = std::fs::metadata(path) {
            total = total.saturating_add(metadata.len());
            found = true;
        }
    }
    found.then_some(total)
}

fn rough_size_factor(
    category: &FileCategory,
    target_format: &str,
    quality_mode: &crate::models::QualityMode,
    options: Option<&crate::models::AdvancedOptions>,
) -> f64 {
    if options.is_some_and(|options| options.copy_streams) {
        return 1.0;
    }

    match category {
        FileCategory::Audio => rough_audio_factor(target_format, quality_mode, options),
        FileCategory::Video => rough_video_factor(target_format, quality_mode, options),
        _ => 1.0,
    }
}

fn rough_audio_factor(
    target_format: &str,
    quality_mode: &crate::models::QualityMode,
    options: Option<&crate::models::AdvancedOptions>,
) -> f64 {
    let codec = options
        .and_then(|options| options.audio_codec.as_deref())
        .unwrap_or(match target_format {
            "flac" => "flac",
            "wav" => "pcm_s16le",
            "ogg" | "opus" => "libopus",
            "mp3" => "libmp3lame",
            _ => "aac",
        });

    let codec_factor = match codec {
        "pcm_s16le" => 1.6,
        "flac" => 0.9,
        "libopus" => 0.38,
        "aac" | "libmp3lame" => 0.52,
        _ => 0.6,
    };

    codec_factor * quality_factor(quality_mode)
}

fn rough_video_factor(
    target_format: &str,
    quality_mode: &crate::models::QualityMode,
    options: Option<&crate::models::AdvancedOptions>,
) -> f64 {
    let codec = options
        .and_then(|options| options.video_codec.as_deref())
        .unwrap_or(match target_format {
            "webm" => "libvpx-vp9",
            "avi" => "mpeg4",
            _ => "libx264",
        });

    let codec_factor = match codec {
        "libaom-av1" => 0.34,
        "libvpx-vp9" | "libx265" | "hevc_nvenc" | "hevc_qsv" | "hevc_amf" => 0.45,
        "libx264" | "h264_nvenc" | "h264_qsv" | "h264_amf" => 0.58,
        "mpeg4" => 0.9,
        "mjpeg" => 1.25,
        _ => 0.7,
    };

    let scale_factor = options
        .and_then(|options| options.max_width)
        .map(|width| match width {
            0..=720 => 0.55,
            721..=1280 => 0.72,
            1281..=1920 => 0.9,
            _ => 1.0,
        })
        .unwrap_or(1.0);

    codec_factor * quality_factor(quality_mode) * scale_factor
}

fn quality_factor(quality_mode: &crate::models::QualityMode) -> f64 {
    match quality_mode {
        crate::models::QualityMode::FastRemux => 1.0,
        crate::models::QualityMode::FastEncode => 0.72,
        crate::models::QualityMode::SmallSize => 0.5,
        crate::models::QualityMode::Balanced => 1.0,
        crate::models::QualityMode::HighQuality => 1.25,
        crate::models::QualityMode::KeepSource => 1.05,
    }
}

fn rough_size_spread(
    quality_mode: &crate::models::QualityMode,
    options: Option<&crate::models::AdvancedOptions>,
) -> f64 {
    if options.is_some_and(|options| options.copy_streams) {
        return 0.08;
    }
    match quality_mode {
        crate::models::QualityMode::FastRemux => 0.08,
        crate::models::QualityMode::SmallSize => 0.45,
        crate::models::QualityMode::FastEncode => 0.42,
        crate::models::QualityMode::Balanced => 0.55,
        crate::models::QualityMode::HighQuality => 0.65,
        crate::models::QualityMode::KeepSource => 0.7,
    }
}

fn rough_size_confidence(spread: f64) -> &'static str {
    if spread <= 0.12 {
        "High"
    } else if spread <= 0.45 {
        "Medium"
    } else {
        "Low"
    }
}

fn size_target_estimate(
    path: &str,
    category: FileCategory,
    target_size_mb: u32,
    audio_bitrate: Option<&str>,
    duration: Option<f64>,
) -> SizeTargetFileEstimate {
    let source_size = std::fs::metadata(path).ok().map(|metadata| metadata.len());
    let estimated_output = Some(u64::from(target_size_mb) * 1024 * 1024);
    let estimated_delta = match (estimated_output, source_size) {
        (Some(output), Some(source)) => Some(output as i64 - source as i64),
        _ => None,
    };

    let Some(duration) = duration else {
        return SizeTargetFileEstimate {
            path: path.into(),
            source_size_bytes: source_size,
            estimated_output_size_bytes: estimated_output,
            estimated_delta_bytes: estimated_delta,
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
                source_size_bytes: source_size,
                estimated_output_size_bytes: estimated_output,
                estimated_delta_bytes: estimated_delta,
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
                source_size_bytes: source_size,
                estimated_output_size_bytes: estimated_output,
                estimated_delta_bytes: estimated_delta,
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
            source_size_bytes: source_size,
            estimated_output_size_bytes: estimated_output,
            estimated_delta_bytes: estimated_delta,
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
pub async fn apply_encoding_recipe_to_queued(
    preset: EncodingPreset,
    state: State<'_, Arc<QueueState>>,
) -> Result<Vec<ConversionJob>, String> {
    state
        .apply_recipe_to_queued(&preset)
        .await
        .map_err(|error| error.to_string())
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
