use crate::{
    capabilities::{allowed_audio_codec_for_format, allowed_video_codec_for_format},
    documents,
    models::{AdvancedOptions, ConversionJob, FileCategory, JobStatus, OverwritePolicy, QualityMode},
};
use anyhow::{anyhow, Context, Result};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

pub fn resolve_output_path(input_path: &str, output_dir: Option<&str>, target_format: &str, overwrite: &OverwritePolicy) -> Result<String> {
    let input = Path::new(input_path);
    let parent = output_dir
        .map(PathBuf::from)
        .or_else(|| input.parent().map(Path::to_path_buf))
        .ok_or_else(|| anyhow!("Cannot resolve output directory"))?;
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("converted");
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
    job.status = JobStatus::Running;
    job.progress = 0.0;
    emit_job(&app, &job);

    let result = match job.category {
        FileCategory::Video | FileCategory::Audio => run_ffmpeg(&app, &mut job, ffmpeg_path, cancel.clone()).await,
        FileCategory::Image => run_image(&mut job, cancel.clone()).await,
        FileCategory::Document => run_document(&mut job, cancel.clone()).await,
        FileCategory::Unsupported => Err(anyhow!("Unsupported source format: {}", job.source_format)),
    };

    if cancel.load(Ordering::Relaxed) {
        job.status = JobStatus::Canceled;
        job.error = None;
    } else if let Err(error) = result {
        remove_failed_output(&job);
        job.status = JobStatus::Failed;
        job.error = Some(human_error(&error.to_string()));
    } else {
        job.status = JobStatus::Done;
        job.progress = 1.0;
        job.error = None;
    }

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

    match (&job.preset.quality_mode, job.target_format.as_str()) {
        (QualityMode::FastRemux | QualityMode::KeepSource, _) => vec![
            "-map", "0",
            "-c", "copy",
            "-movflags", "+faststart",
        ],
        (QualityMode::FastEncode, "webm") => vec![
            "-map", "0:v:0?", "-map", "0:a:0?",
            "-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-b:v", "0", "-crf", "34",
            "-c:a", "libopus", "-b:a", "128k",
        ],
        (QualityMode::FastEncode, _) => vec![
            "-map", "0:v:0?", "-map", "0:a:0?",
            "-c:v", "mpeg4", "-q:v", "7",
            "-c:a", "aac", "-b:a", "160k",
            "-movflags", "+faststart",
        ],
        (QualityMode::SmallSize, "webm") => vec![
            "-map", "0:v:0?", "-map", "0:a:0?",
            "-c:v", "libvpx-vp9", "-deadline", "good", "-cpu-used", "4", "-b:v", "0", "-crf", "38",
            "-vf", "scale='min(1280,iw)':-2",
            "-c:a", "libopus", "-b:a", "96k",
        ],
        (QualityMode::SmallSize, _) => vec![
            "-map", "0:v:0?", "-map", "0:a:0?",
            "-c:v", "mpeg4", "-q:v", "8",
            "-vf", "scale='min(1280,iw)':-2",
            "-c:a", "aac", "-b:a", "128k",
            "-movflags", "+faststart",
        ],
        (QualityMode::HighQuality, "webm") => vec![
            "-map", "0:v:0?", "-map", "0:a:0?",
            "-c:v", "libvpx-vp9", "-deadline", "good", "-cpu-used", "2", "-b:v", "0", "-crf", "24",
            "-c:a", "libopus", "-b:a", "192k",
        ],
        (QualityMode::HighQuality, _) => vec![
            "-map", "0:v:0?", "-map", "0:a:0?",
            "-c:v", "mpeg4", "-q:v", "2",
            "-c:a", "aac", "-b:a", "256k",
            "-movflags", "+faststart",
        ],
        (QualityMode::Balanced, "webm") => vec![
            "-map", "0:v:0?", "-map", "0:a:0?",
            "-c:v", "libvpx-vp9", "-deadline", "good", "-cpu-used", "5", "-b:v", "0", "-crf", "30",
            "-c:a", "libopus", "-b:a", "160k",
        ],
        (QualityMode::Balanced, _) => vec![
            "-map", "0:v:0?", "-map", "0:a:0?",
            "-c:v", "mpeg4", "-q:v", "5",
            "-c:a", "aac", "-b:a", "192k",
            "-movflags", "+faststart",
        ],
    }
}

fn audio_args(job: &ConversionJob) -> Vec<&'static str> {
    if let Some(options) = &job.advanced_options {
        return advanced_audio_args(job, options);
    }

    match job.preset.quality_mode {
        QualityMode::FastRemux | QualityMode::KeepSource => vec!["-vn", "-c:a", "copy"],
        QualityMode::FastEncode => vec!["-vn", "-c:a", audio_codec(job), "-b:a", "160k"],
        QualityMode::SmallSize => vec!["-vn", "-c:a", audio_codec(job), "-b:a", "96k"],
        QualityMode::Balanced => vec!["-vn", "-c:a", audio_codec(job), "-b:a", "192k"],
        QualityMode::HighQuality => high_quality_audio_args(job),
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
            args.extend(["-deadline", "good", "-cpu-used", "4", "-b:v", "0", "-crf", quality_to_crf(options.video_quality).unwrap_or("30")]);
        }
        "mpeg4" => {
            args.extend(["-q:v", quality_to_qscale(options.video_quality).unwrap_or("5")]);
        }
        _ => {}
    }

    if let Some(width) = options.max_width.and_then(max_width_filter) {
        args.extend(["-vf", width]);
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
    let micros = line.trim_start_matches("out_time_ms=").parse::<f64>().ok()?;
    Some((micros / 1_000_000.0 / duration).clamp(0.0, 1.0) as f32)
}

async fn run_ffmpeg(
    app: &AppHandle,
    job: &mut ConversionJob,
    ffmpeg_path: Option<String>,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let ffmpeg = find_ffmpeg(ffmpeg_path).ok_or_else(|| {
        anyhow!("FFmpeg binary was not found. Add bundled resources/bin/ffmpeg or configure a system path.")
    })?;
    let duration = probe_duration(&ffmpeg, &job.input_path).await.ok();
    let mut child = Command::new(ffmpeg)
        .args(build_ffmpeg_args(job))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to start FFmpeg")?;

    let stdout = child.stdout.take().context("Failed to read FFmpeg progress")?;
    let mut reader = BufReader::new(stdout).lines();

    while let Some(line) = reader.next_line().await? {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            return Ok(());
        }
        if let Some(progress) = parse_ffmpeg_progress_line(&line, duration) {
            job.progress = progress;
            emit_job(app, job);
        }
        if let Some(speed) = line.strip_prefix("speed=") {
            job.speed = Some(speed.to_string());
            emit_job(app, job);
        }
    }

    let status = child.wait().await?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("FFmpeg exited with status {status}"))
    }
}

async fn run_image(job: &mut ConversionJob, cancel: Arc<AtomicBool>) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    let input = job.input_path.clone();
    let output = job.output_path.clone();
    let quality_mode = job.preset.quality_mode.clone();
    let image_quality = job.advanced_options.as_ref().and_then(|options| options.image_quality);
    tokio::task::spawn_blocking(move || -> Result<()> {
        let image = image::open(&input).context("Failed to decode image")?;
        save_image_with_preset(image, &output, quality_mode, image_quality)?;
        Ok(())
    })
    .await??;
    job.progress = 1.0;
    Ok(())
}

fn save_image_with_preset(image: image::DynamicImage, output: &str, mode: QualityMode, override_quality: Option<u8>) -> Result<()> {
    let output_path = Path::new(output);
    match output_path.extension().and_then(|ext| ext.to_str()).unwrap_or_default().to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => {
            let quality = override_quality.unwrap_or_else(|| match mode {
                QualityMode::SmallSize => 70,
                QualityMode::FastEncode => 82,
                QualityMode::HighQuality | QualityMode::KeepSource => 95,
                QualityMode::FastRemux | QualityMode::Balanced => 86,
            });
            let file = std::fs::File::create(output_path).context("Failed to create image output")?;
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality);
            encoder.encode_image(&image).context("Failed to encode JPEG")?;
        }
        _ => image.save(output_path).context("Failed to encode image")?,
    }
    Ok(())
}

fn find_ffmpeg(configured: Option<String>) -> Option<String> {
    if let Some(path) = configured.filter(|path| Path::new(path).exists()) {
        return Some(path);
    }

    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
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
    text.trim().parse::<f64>().context("Failed to parse duration")
}

fn emit_job(app: &AppHandle, job: &ConversionJob) {
    let _ = app.emit("queue://job-updated", crate::models::QueueUpdate { job: job.clone() });
}

fn human_error(error: &str) -> String {
    if error.contains("FFmpeg binary was not found") {
        "FFmpeg is not available. Add a bundled binary or configure a path in settings.".into()
    } else if error.contains("Unsupported source format") {
        error.into()
    } else {
        format!("Conversion failed: {error}")
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
            eta_seconds: None,
            error: None,
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
    fn advanced_video_rejects_codec_that_conflicts_with_webm() {
        let mut job = job(QualityMode::Balanced);
        job.advanced_options = Some(AdvancedOptions {
            video_codec: Some("mpeg4".into()),
            audio_codec: Some("aac".into()),
            video_quality: Some(70),
            video_bitrate: None,
            audio_bitrate: Some("192k".into()),
            max_width: Some(1280),
            image_quality: None,
            copy_streams: false,
        });
        let args = build_ffmpeg_args(&job);
        assert!(args.contains(&"libvpx-vp9".to_string()));
        assert!(!args.contains(&"mpeg4".to_string()));
    }

    #[test]
    fn parses_progress() {
        let progress = parse_ffmpeg_progress_line("out_time_ms=5000000", Some(10.0)).unwrap();
        assert!((progress - 0.5).abs() < 0.01);
    }
}
