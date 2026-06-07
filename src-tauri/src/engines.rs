use crate::{
    capabilities::{allowed_audio_codec_for_format, allowed_video_codec_for_format},
    documents,
    models::{
        AdvancedOptions, ConversionJob, FileCategory, JobStatus, OverwritePolicy, QualityMode,
    },
};
use anyhow::{Context, Result, anyhow};
use image::ImageEncoder;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

pub fn resolve_output_path(
    input_path: &str,
    output_dir: Option<&str>,
    target_format: &str,
    overwrite: &OverwritePolicy,
) -> Result<String> {
    let input = Path::new(input_path);
    let parent = output_dir
        .map(PathBuf::from)
        .or_else(|| input.parent().map(Path::to_path_buf))
        .ok_or_else(|| anyhow!("Cannot resolve output directory"))?;
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("converted");
    let mut candidate = parent.join(format!("{stem}.{target_format}"));

    if matches!(overwrite, OverwritePolicy::Rename) {
        let mut index = 1;
        while candidate.exists() {
            candidate = parent.join(format!("{stem} ({index}).{target_format}"));
            index += 1;
        }
    }

    Ok(candidate.to_string_lossy().to_string())
}

pub async fn convert_job(
    app: AppHandle,
    mut job: ConversionJob,
    ffmpeg_path: Option<String>,
    cancel: Arc<AtomicBool>,
) -> ConversionJob {
    let started_at = Instant::now();
    job.status = JobStatus::Running;
    job.progress = 0.0;
    job.processing_seconds = Some(0);
    job.eta_seconds = None;
    emit_job(&app, &job);

    let result = match job.category {
        FileCategory::Video | FileCategory::Audio => {
            run_ffmpeg(&app, &mut job, ffmpeg_path, cancel.clone()).await
        }
        FileCategory::Image => run_image(&mut job, cancel.clone()).await,
        FileCategory::Document => run_document(&mut job, cancel.clone()).await,
        FileCategory::Unsupported => {
            Err(anyhow!("Unsupported source format: {}", job.source_format))
        }
    };

    if cancel.load(Ordering::Relaxed) {
        job.status = JobStatus::Canceled;
        job.error = None;
        job.error_details = None;
    } else if let Err(error) = result {
        remove_failed_output(&job);
        job.status = JobStatus::Failed;
        let error = error.to_string();
        job.error = Some(human_error(&error));
        job.error_details = Some(technical_error(&error, &job));
    } else {
        job.status = JobStatus::Done;
        job.progress = 1.0;
        job.error = None;
        job.error_details = None;
    }

    job.processing_seconds = Some(started_at.elapsed().as_secs());
    job.eta_seconds = None;
    emit_job(&app, &job);
    job
}

async fn run_document(job: &mut ConversionJob, cancel: Arc<AtomicBool>) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    let job = job.clone();
    tokio::task::spawn_blocking(move || documents::run_document_job(&job)).await??;
    Ok(())
}

pub fn build_ffmpeg_args(job: &ConversionJob) -> Vec<String> {
    build_ffmpeg_args_with_duration(job, None)
}

fn build_ffmpeg_args_with_duration(
    job: &ConversionJob,
    duration_seconds: Option<f64>,
) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".into(),
        "-y".into(),
        "-i".into(),
        job.input_path.clone(),
    ];

    match job.category {
        FileCategory::Video => args.extend(video_args(job).into_iter().map(String::from)),
        FileCategory::Audio => args.extend(audio_args(job).into_iter().map(String::from)),
        _ => {}
    }

    apply_target_size(job, duration_seconds, &mut args);

    args.extend([
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
        job.output_path.clone(),
    ]);
    args
}

fn video_args(job: &ConversionJob) -> Vec<&'static str> {
    if let Some(options) = &job.advanced_options {
        return advanced_video_args(job, options);
    }

    if matches!(job.preset.quality_mode, QualityMode::KeepSource) {
        if same_source_target_format(job) {
            return vec!["-map", "0", "-c", "copy", "-movflags", "+faststart"];
        }
        return high_quality_video_args(&job.target_format);
    }

    match (&job.preset.quality_mode, job.target_format.as_str()) {
        (QualityMode::FastRemux, _) => {
            vec!["-map", "0", "-c", "copy", "-movflags", "+faststart"]
        }
        (QualityMode::FastEncode, "webm") => vec![
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-c:v",
            "libvpx-vp9",
            "-deadline",
            "realtime",
            "-cpu-used",
            "8",
            "-b:v",
            "0",
            "-crf",
            "34",
            "-c:a",
            "libopus",
            "-b:a",
            "128k",
        ],
        (QualityMode::FastEncode, _) => vec![
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-c:v",
            "mpeg4",
            "-q:v",
            "7",
            "-c:a",
            "aac",
            "-b:a",
            "160k",
            "-movflags",
            "+faststart",
        ],
        (QualityMode::SmallSize, "webm") => vec![
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-c:v",
            "libvpx-vp9",
            "-deadline",
            "good",
            "-cpu-used",
            "4",
            "-b:v",
            "0",
            "-crf",
            "38",
            "-vf",
            "scale='min(1280,iw)':-2",
            "-c:a",
            "libopus",
            "-b:a",
            "96k",
        ],
        (QualityMode::SmallSize, _) => vec![
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-c:v",
            "mpeg4",
            "-q:v",
            "8",
            "-vf",
            "scale='min(1280,iw)':-2",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-movflags",
            "+faststart",
        ],
        (QualityMode::HighQuality, _) => high_quality_video_args(&job.target_format),
        (QualityMode::Balanced, "webm") => vec![
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-c:v",
            "libvpx-vp9",
            "-deadline",
            "good",
            "-cpu-used",
            "5",
            "-b:v",
            "0",
            "-crf",
            "30",
            "-c:a",
            "libopus",
            "-b:a",
            "160k",
        ],
        (QualityMode::Balanced, _) => vec![
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
        ],
        (QualityMode::KeepSource, _) => unreachable!("KeepSource is handled before the match"),
    }
}

fn audio_args(job: &ConversionJob) -> Vec<&'static str> {
    if let Some(options) = &job.advanced_options {
        return advanced_audio_args(job, options);
    }

    if matches!(job.preset.quality_mode, QualityMode::KeepSource) {
        if same_source_target_format(job) {
            return vec!["-vn", "-c:a", "copy"];
        }
        return high_quality_audio_args(job);
    }

    match job.preset.quality_mode {
        QualityMode::FastRemux => vec!["-vn", "-c:a", "copy"],
        QualityMode::FastEncode => vec!["-vn", "-c:a", audio_codec(job), "-b:a", "160k"],
        QualityMode::SmallSize => vec!["-vn", "-c:a", audio_codec(job), "-b:a", "96k"],
        QualityMode::Balanced => vec!["-vn", "-c:a", audio_codec(job), "-b:a", "192k"],
        QualityMode::HighQuality => high_quality_audio_args(job),
        QualityMode::KeepSource => unreachable!("KeepSource is handled before the match"),
    }
}

fn same_source_target_format(job: &ConversionJob) -> bool {
    job.source_format.eq_ignore_ascii_case(&job.target_format)
}

fn high_quality_video_args(target_format: &str) -> Vec<&'static str> {
    match target_format {
        "webm" => vec![
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-c:v",
            "libvpx-vp9",
            "-deadline",
            "good",
            "-cpu-used",
            "2",
            "-b:v",
            "0",
            "-crf",
            "24",
            "-c:a",
            "libopus",
            "-b:a",
            "192k",
        ],
        _ => vec![
            "-map",
            "0:v:0?",
            "-map",
            "0:a:0?",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "-c:a",
            "aac",
            "-b:a",
            "256k",
            "-movflags",
            "+faststart",
        ],
    }
}

fn advanced_video_args(job: &ConversionJob, options: &AdvancedOptions) -> Vec<&'static str> {
    if options.copy_streams {
        return vec!["-map", "0", "-c", "copy", "-movflags", "+faststart"];
    }

    let mut args = vec!["-map", "0:v:0?", "-map", "0:a:0?"];
    let video_codec = allowed_video_codec(job, options.video_codec.as_deref());
    args.extend(["-c:v", video_codec]);

    match video_codec {
        "libvpx-vp9" => {
            args.extend([
                "-deadline",
                "good",
                "-cpu-used",
                "4",
                "-b:v",
                "0",
                "-crf",
                quality_to_crf(options.video_quality).unwrap_or("30"),
            ]);
        }
        "mpeg4" => {
            args.extend([
                "-q:v",
                quality_to_qscale(options.video_quality).unwrap_or("5"),
            ]);
        }
        _ => {}
    }

    if let Some(width) = options.max_width.and_then(max_width_filter) {
        args.extend(["-vf", width]);
    }
    if let Some(frame_rate) = options.frame_rate.and_then(allowed_frame_rate) {
        args.extend(["-r", frame_rate]);
    }

    let audio_codec = allowed_audio_codec(job, options.audio_codec.as_deref());
    args.extend(["-c:a", audio_codec]);
    if let Some(bitrate) = allowed_bitrate(options.audio_bitrate.as_deref()) {
        args.extend(["-b:a", bitrate]);
    } else if audio_codec != "flac" && audio_codec != "pcm_s16le" {
        args.extend(["-b:a", "192k"]);
    }

    if job.target_format != "webm" {
        args.extend(["-movflags", "+faststart"]);
    }

    args
}

fn advanced_audio_args(job: &ConversionJob, options: &AdvancedOptions) -> Vec<&'static str> {
    if options.copy_streams {
        return vec!["-vn", "-c:a", "copy"];
    }

    let audio_codec = allowed_audio_codec(job, options.audio_codec.as_deref());
    let mut args = vec!["-vn", "-c:a", audio_codec];
    if audio_codec == "flac" {
        args.extend(["-compression_level", "8"]);
    } else if let Some(bitrate) = allowed_bitrate(options.audio_bitrate.as_deref()) {
        args.extend(["-b:a", bitrate]);
    } else if audio_codec != "pcm_s16le" {
        args.extend(["-b:a", "192k"]);
    }
    args
}

fn allowed_video_codec(job: &ConversionJob, requested: Option<&str>) -> &'static str {
    allowed_video_codec_for_format(&job.target_format, requested)
}

fn allowed_audio_codec(job: &ConversionJob, requested: Option<&str>) -> &'static str {
    allowed_audio_codec_for_format(&job.target_format, requested)
}

fn allowed_bitrate(requested: Option<&str>) -> Option<&'static str> {
    match requested {
        Some("96k") => Some("96k"),
        Some("128k") => Some("128k"),
        Some("160k") => Some("160k"),
        Some("192k") => Some("192k"),
        Some("256k") => Some("256k"),
        Some("320k") => Some("320k"),
        _ => None,
    }
}

fn apply_target_size(job: &ConversionJob, duration_seconds: Option<f64>, args: &mut Vec<String>) {
    let Some(options) = job.advanced_options.as_ref() else {
        return;
    };
    let Some(target_size_mb) = options.target_size_mb.and_then(allowed_target_size_mb) else {
        return;
    };
    let Some(duration) = duration_seconds.filter(|duration| *duration > 0.0) else {
        return;
    };

    let total_kbps = ((f64::from(target_size_mb) * 8192.0) / duration * 0.94).floor() as u32;
    match job.category {
        FileCategory::Video => {
            let requested_audio =
                parse_audio_bitrate_kbps(options.audio_bitrate.as_deref()).unwrap_or(160);
            let audio_kbps = requested_audio.min(total_kbps.saturating_sub(160)).max(64);
            let video_kbps = total_kbps.saturating_sub(audio_kbps).max(120);
            remove_option_pairs(args, &["-b:v", "-crf", "-q:v", "-b:a"]);
            args.extend([
                "-b:v".into(),
                format!("{video_kbps}k"),
                "-maxrate".into(),
                format!("{}k", video_kbps + video_kbps / 2),
                "-bufsize".into(),
                format!("{}k", video_kbps * 2),
                "-b:a".into(),
                format!("{audio_kbps}k"),
            ]);
        }
        FileCategory::Audio => {
            let audio_kbps = total_kbps.clamp(32, 320);
            remove_option_pairs(args, &["-b:a"]);
            args.extend(["-b:a".into(), format!("{audio_kbps}k")]);
        }
        _ => {}
    }
}

fn remove_option_pairs(args: &mut Vec<String>, options: &[&str]) {
    let mut index = 0;
    while index < args.len() {
        if options.contains(&args[index].as_str()) {
            args.remove(index);
            if index < args.len() {
                args.remove(index);
            }
        } else {
            index += 1;
        }
    }
}

fn parse_audio_bitrate_kbps(value: Option<&str>) -> Option<u32> {
    value?.trim_end_matches('k').parse::<u32>().ok()
}

fn allowed_target_size_mb(value: u32) -> Option<u32> {
    (1..=10_240).contains(&value).then_some(value)
}

fn quality_to_crf(quality: Option<u8>) -> Option<&'static str> {
    match quality.unwrap_or(50) {
        0..=20 => Some("38"),
        21..=40 => Some("34"),
        41..=60 => Some("30"),
        61..=80 => Some("26"),
        _ => Some("22"),
    }
}

fn quality_to_qscale(quality: Option<u8>) -> Option<&'static str> {
    match quality.unwrap_or(50) {
        0..=20 => Some("8"),
        21..=40 => Some("7"),
        41..=60 => Some("5"),
        61..=80 => Some("3"),
        _ => Some("2"),
    }
}

fn max_width_filter(width: u32) -> Option<&'static str> {
    match width {
        720 => Some("scale='min(720,iw)':-2"),
        1280 => Some("scale='min(1280,iw)':-2"),
        1920 => Some("scale='min(1920,iw)':-2"),
        3840 => Some("scale='min(3840,iw)':-2"),
        _ => None,
    }
}

fn allowed_frame_rate(frame_rate: u32) -> Option<&'static str> {
    match frame_rate {
        24 => Some("24"),
        25 => Some("25"),
        30 => Some("30"),
        50 => Some("50"),
        60 => Some("60"),
        120 => Some("120"),
        _ => None,
    }
}

fn high_quality_audio_args(job: &ConversionJob) -> Vec<&'static str> {
    match job.target_format.as_str() {
        "flac" => vec!["-vn", "-c:a", "flac", "-compression_level", "8"],
        "wav" => vec!["-vn", "-c:a", "pcm_s16le"],
        _ => vec!["-vn", "-c:a", audio_codec(job), "-b:a", "320k"],
    }
}

fn audio_codec(job: &ConversionJob) -> &'static str {
    match job.target_format.as_str() {
        "flac" => "flac",
        "ogg" | "opus" => "libopus",
        "wav" => "pcm_s16le",
        _ => "aac",
    }
}

pub fn parse_ffmpeg_progress_line(line: &str, duration_seconds: Option<f64>) -> Option<f32> {
    let duration = duration_seconds?;
    if duration <= 0.0 || !line.starts_with("out_time_ms=") {
        return None;
    }
    let micros = line
        .trim_start_matches("out_time_ms=")
        .parse::<f64>()
        .ok()?;
    Some((micros / 1_000_000.0 / duration).clamp(0.0, 1.0) as f32)
}

fn estimate_remaining_seconds(started_at: Instant, progress: f32) -> Option<u64> {
    if !(0.0..1.0).contains(&progress) {
        return None;
    }

    let elapsed = started_at.elapsed().as_secs_f64();
    if elapsed <= 0.0 {
        return None;
    }

    let remaining = elapsed * ((1.0 - f64::from(progress)) / f64::from(progress));
    remaining.is_finite().then(|| remaining.ceil() as u64)
}

async fn run_ffmpeg(
    app: &AppHandle,
    job: &mut ConversionJob,
    ffmpeg_path: Option<String>,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let started_at = Instant::now();
    let ffmpeg = find_ffmpeg(ffmpeg_path).ok_or_else(|| {
        anyhow!("FFmpeg binary was not found. Add bundled resources/bin/ffmpeg or configure a system path.")
    })?;
    let duration = probe_duration(&ffmpeg, &job.input_path).await.ok();
    let args = build_ffmpeg_args_with_duration(job, duration);
    let command_line = format!("{} {}", ffmpeg, shell_words(&args));
    let mut child = Command::new(ffmpeg)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to start FFmpeg")?;

    let stdout = child
        .stdout
        .take()
        .context("Failed to read FFmpeg progress")?;
    let mut reader = BufReader::new(stdout).lines();
    let stderr = child
        .stderr
        .take()
        .context("Failed to read FFmpeg diagnostics")?;
    let stderr_handle = tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut tail = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            tail.push(line);
            if tail.len() > 80 {
                tail.remove(0);
            }
        }
        tail.join("\n")
    });

    while let Some(line) = reader.next_line().await? {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            return Ok(());
        }
        if let Some(progress) = parse_ffmpeg_progress_line(&line, duration) {
            job.progress = progress;
            job.processing_seconds = Some(started_at.elapsed().as_secs());
            job.eta_seconds = estimate_remaining_seconds(started_at, progress);
            emit_job(app, job);
        }
        if let Some(speed) = line.strip_prefix("speed=") {
            job.speed = Some(speed.to_string());
            job.processing_seconds = Some(started_at.elapsed().as_secs());
            job.eta_seconds = estimate_remaining_seconds(started_at, job.progress);
            emit_job(app, job);
        }
    }

    let status = child.wait().await?;
    let stderr_tail = stderr_handle.await.unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "FFmpeg exited with status {status}\n\nCommand:\n{command_line}\n\nOutput path:\n{}\n\nFFmpeg stderr:\n{}",
            job.output_path,
            if stderr_tail.trim().is_empty() {
                "No stderr output captured."
            } else {
                stderr_tail.trim()
            }
        ))
    }
}

async fn run_image(job: &mut ConversionJob, cancel: Arc<AtomicBool>) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    let input = job.input_path.clone();
    let output = job.output_path.clone();
    let quality_mode = job.preset.quality_mode.clone();
    let image_quality = job
        .advanced_options
        .as_ref()
        .and_then(|options| options.image_quality);
    tokio::task::spawn_blocking(move || -> Result<()> {
        let image = image::open(&input).context("Failed to decode image")?;
        save_image_with_preset(image, &output, quality_mode, image_quality)?;
        Ok(())
    })
    .await??;
    job.progress = 1.0;
    Ok(())
}

fn save_image_with_preset(
    image: image::DynamicImage,
    output: &str,
    mode: QualityMode,
    override_quality: Option<u8>,
) -> Result<()> {
    let output_path = Path::new(output);
    match output_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => {
            let quality = override_quality.unwrap_or_else(|| match mode {
                QualityMode::SmallSize => 70,
                QualityMode::FastEncode => 82,
                QualityMode::HighQuality | QualityMode::KeepSource => 95,
                QualityMode::FastRemux | QualityMode::Balanced => 86,
            });
            let file =
                std::fs::File::create(output_path).context("Failed to create image output")?;
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality);
            encoder
                .encode_image(&image)
                .context("Failed to encode JPEG")?;
        }
        "ico" => {
            let icon = image.resize(256, 256, image::imageops::FilterType::Lanczos3);
            let rgba = icon.to_rgba8();
            let file = std::fs::File::create(output_path).context("Failed to create ICO output")?;
            image::codecs::ico::IcoEncoder::new(file)
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    image::ExtendedColorType::Rgba8,
                )
                .context("Failed to encode ICO")?;
        }
        _ => image.save(output_path).context("Failed to encode image")?,
    }
    Ok(())
}

fn find_ffmpeg(configured: Option<String>) -> Option<String> {
    if let Some(path) = configured.filter(|path| Path::new(path).exists()) {
        return Some(path);
    }

    let exe = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let bundled = Path::new("resources").join("bin").join(exe);
    if bundled.exists() {
        return Some(bundled.to_string_lossy().to_string());
    }

    Some(exe.to_string())
}

async fn probe_duration(ffmpeg_path: &str, input_path: &str) -> Result<f64> {
    let ffprobe = ffmpeg_path.replace("ffmpeg", "ffprobe");
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            input_path,
        ])
        .output()
        .await
        .context("Failed to run ffprobe")?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim()
        .parse::<f64>()
        .context("Failed to parse duration")
}

fn emit_job(app: &AppHandle, job: &ConversionJob) {
    let _ = app.emit(
        "queue://job-updated",
        crate::models::QueueUpdate { job: job.clone() },
    );
}

fn human_error(error: &str) -> String {
    if error.contains("FFmpeg binary was not found") {
        "FFmpeg is not available. Add a bundled binary or configure a path in settings.".into()
    } else if error.contains("Unsupported source format") {
        error.into()
    } else if let Some(code) = ffmpeg_exit_code(error) {
        format!("{} (Error code: {code})", ffmpeg_exit_explanation(&code))
    } else {
        error.to_string()
    }
}

fn technical_error(error: &str, job: &ConversionJob) -> String {
    format!(
        "{}\n\nJob:\ninput: {}\noutput: {}\nsource format: {}\ntarget format: {}\npreset: {}",
        error,
        job.input_path,
        job.output_path,
        job.source_format,
        job.target_format,
        job.preset.name
    )
}

fn ffmpeg_exit_code(error: &str) -> Option<String> {
    if !error.contains("FFmpeg exited") {
        return None;
    }

    error
        .lines()
        .next()
        .unwrap_or(error)
        .split_whitespace()
        .rev()
        .find(|part| {
            let value = part.trim_matches(|ch: char| matches!(ch, ')' | '(' | ',' | '.'));
            value.starts_with("0x") || value.parse::<i32>().is_ok()
        })
        .map(|part| {
            part.trim_matches(|ch: char| matches!(ch, ')' | '(' | ',' | '.'))
                .to_ascii_lowercase()
        })
}

fn shell_words(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.contains(' ') || arg.contains('"') {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn ffmpeg_exit_explanation(code: &str) -> &'static str {
    match code {
        "0xffffffea" | "-22" => {
            "The selected format, codec, or stream settings are incompatible with this file."
        }
        "0xfffffffb" | "-5" => "FFmpeg could not read the input file or write the output file.",
        "0xfffffffe" | "-2" => {
            "FFmpeg could not find a required file, stream, codec, or dependency."
        }
        "0xffffffff" | "-1" => "FFmpeg stopped with a generic processing error.",
        "0xc0000135" => "A required FFmpeg runtime dependency is missing.",
        "1" => "FFmpeg could not complete this conversion with the selected settings.",
        _ => "FFmpeg could not complete this conversion.",
    }
}

fn remove_failed_output(job: &ConversionJob) {
    if job.output_path == job.input_path {
        return;
    }

    let output = Path::new(&job.output_path);
    if output.is_file() {
        let _ = std::fs::remove_file(output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConversionPreset, FileCategory, OverwritePolicy, QualityMode};

    fn job(mode: QualityMode) -> ConversionJob {
        ConversionJob {
            id: "1".into(),
            input_path: "input.mp4".into(),
            input_paths: vec!["input.mp4".into()],
            output_path: "output.webm".into(),
            source_format: "mp4".into(),
            target_format: "webm".into(),
            category: FileCategory::Video,
            preset: ConversionPreset {
                name: "test".into(),
                description: "test".into(),
                quality_mode: mode,
                overwrite_policy: OverwritePolicy::Rename,
            },
            advanced_options: None,
            document_operation: None,
            status: JobStatus::Queued,
            progress: 0.0,
            speed: None,
            processing_seconds: None,
            eta_seconds: None,
            error: None,
            error_details: None,
        }
    }

    #[test]
    fn builds_args_for_balanced_video() {
        let args = build_ffmpeg_args(&job(QualityMode::Balanced));
        assert!(args.contains(&"-progress".to_string()));
        assert!(args.contains(&"libvpx-vp9".to_string()));
    }

    #[test]
    fn builds_args_for_fast_remux() {
        let args = build_ffmpeg_args(&job(QualityMode::FastRemux));
        assert!(args.contains(&"-c".to_string()));
        assert!(args.contains(&"copy".to_string()));
    }

    #[test]
    fn keep_source_reencodes_when_container_changes() {
        let args = build_ffmpeg_args(&job(QualityMode::KeepSource));
        assert!(args.contains(&"libvpx-vp9".to_string()));
        assert!(!args.contains(&"copy".to_string()));
    }

    #[test]
    fn keep_source_copies_when_container_matches() {
        let mut job = job(QualityMode::KeepSource);
        job.target_format = "mp4".into();
        job.output_path = "output.mp4".into();

        let args = build_ffmpeg_args(&job);
        assert!(args.contains(&"-c".to_string()));
        assert!(args.contains(&"copy".to_string()));
    }

    #[test]
    fn advanced_video_rejects_codec_that_conflicts_with_webm() {
        let mut job = job(QualityMode::Balanced);
        job.advanced_options = Some(AdvancedOptions {
            video_codec: Some("mpeg4".into()),
            audio_codec: Some("aac".into()),
            video_quality: Some(70),
            video_bitrate: None,
            audio_bitrate: Some("192k".into()),
            max_width: Some(1280),
            frame_rate: None,
            target_size_mb: None,
            image_quality: None,
            copy_streams: false,
        });
        let args = build_ffmpeg_args(&job);
        assert!(args.contains(&"libvpx-vp9".to_string()));
        assert!(!args.contains(&"mpeg4".to_string()));
    }

    #[test]
    fn advanced_video_adds_allowed_frame_rate() {
        let mut job = job(QualityMode::Balanced);
        job.advanced_options = Some(AdvancedOptions {
            video_codec: Some("libvpx-vp9".into()),
            audio_codec: Some("libopus".into()),
            video_quality: Some(60),
            video_bitrate: None,
            audio_bitrate: Some("192k".into()),
            max_width: None,
            frame_rate: Some(30),
            target_size_mb: None,
            image_quality: None,
            copy_streams: false,
        });

        let args = build_ffmpeg_args(&job);
        assert!(args.windows(2).any(|pair| pair == ["-r", "30"]));
    }

    #[test]
    fn advanced_video_target_size_overrides_quality_bitrate() {
        let mut job = job(QualityMode::Balanced);
        job.advanced_options = Some(AdvancedOptions {
            video_codec: Some("libvpx-vp9".into()),
            audio_codec: Some("libopus".into()),
            video_quality: Some(60),
            video_bitrate: None,
            audio_bitrate: Some("128k".into()),
            max_width: None,
            frame_rate: None,
            target_size_mb: Some(25),
            image_quality: None,
            copy_streams: false,
        });

        let args = build_ffmpeg_args_with_duration(&job, Some(60.0));
        assert!(args.contains(&"-b:v".to_string()));
        assert!(args.contains(&"-maxrate".to_string()));
        assert!(!args.contains(&"-crf".to_string()));
    }

    #[test]
    fn parses_progress() {
        let progress = parse_ffmpeg_progress_line("out_time_ms=5000000", Some(10.0)).unwrap();
        assert!((progress - 0.5).abs() < 0.01);
    }

    #[test]
    fn explains_common_ffmpeg_exit_code() {
        let message = human_error("FFmpeg exited with status exit code: 0xffffffea");
        assert_eq!(
            message,
            "The selected format, codec, or stream settings are incompatible with this file. (Error code: 0xffffffea)"
        );
    }

    #[test]
    fn explains_unknown_ffmpeg_exit_code_without_raw_status_text() {
        let message = human_error("FFmpeg exited with status exit code: 123");
        assert_eq!(
            message,
            "FFmpeg could not complete this conversion. (Error code: 123)"
        );
    }

    #[test]
    fn encodes_large_png_as_ico() {
        let source_image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            512,
            512,
            image::Rgba([83, 255, 171, 255]),
        ));
        let output = std::env::temp_dir().join(format!(
            "shiftr-test-{}.ico",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        save_image_with_preset(
            source_image,
            output.to_str().unwrap(),
            QualityMode::KeepSource,
            None,
        )
        .unwrap();

        assert!(output.is_file());
        let decoded = image::open(&output).unwrap();
        assert!(decoded.width() <= 256);
        assert!(decoded.height() <= 256);
        let _ = std::fs::remove_file(output);
    }
}
