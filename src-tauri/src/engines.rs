use crate::models::{ConversionJob, FileCategory, JobStatus, OverwritePolicy, QualityMode};
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
        FileCategory::Unsupported => Err(anyhow!("Unsupported source format: {}", job.source_format)),
    };

    if cancel.load(Ordering::Relaxed) {
        job.status = JobStatus::Canceled;
        job.error = None;
    } else if let Err(error) = result {
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

pub fn build_ffmpeg_args(job: &ConversionJob) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".into(),
        "-y".into(),
        "-i".into(),
        job.input_path.clone(),
    ];

    match (&job.category, &job.preset.quality_mode, job.target_format.as_str()) {
        (FileCategory::Video, QualityMode::SmallSize, _) => {
            args.extend(["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "36", "-c:a", "libopus"].map(String::from));
        }
        (FileCategory::Video, QualityMode::HighQuality, "webm") => {
            args.extend(["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "24", "-c:a", "libopus"].map(String::from));
        }
        (FileCategory::Video, QualityMode::HighQuality, _) => {
            args.extend(["-c:v", "mpeg4", "-q:v", "2", "-c:a", "aac", "-b:a", "256k"].map(String::from));
        }
        (FileCategory::Video, QualityMode::KeepSource, _) => {
            args.extend(["-c", "copy"].map(String::from));
        }
        (FileCategory::Video, _, "webm") => {
            args.extend(["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "30", "-c:a", "libopus"].map(String::from));
        }
        (FileCategory::Video, _, _) => {
            args.extend(["-c:v", "mpeg4", "-q:v", "5", "-c:a", "aac", "-b:a", "192k"].map(String::from));
        }
        (FileCategory::Audio, QualityMode::SmallSize, _) => {
            args.extend(["-vn", "-b:a", "96k"].map(String::from));
        }
        (FileCategory::Audio, QualityMode::HighQuality, _) => {
            args.extend(["-vn", "-b:a", "320k"].map(String::from));
        }
        (FileCategory::Audio, QualityMode::KeepSource, _) => {
            args.extend(["-vn", "-c:a", "copy"].map(String::from));
        }
        (FileCategory::Audio, _, _) => {
            args.extend(["-vn", "-b:a", "192k"].map(String::from));
        }
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
    tokio::task::spawn_blocking(move || -> Result<()> {
        let image = image::open(&input).context("Failed to decode image")?;
        image.save(&output).context("Failed to encode image")?;
        Ok(())
    })
    .await??;
    job.progress = 1.0;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConversionPreset, FileCategory, OverwritePolicy, QualityMode};

    fn job(mode: QualityMode) -> ConversionJob {
        ConversionJob {
            id: "1".into(),
            input_path: "input.mp4".into(),
            output_path: "output.webm".into(),
            source_format: "mp4".into(),
            target_format: "webm".into(),
            category: FileCategory::Video,
            preset: ConversionPreset {
                name: "test".into(),
                quality_mode: mode,
                overwrite_policy: OverwritePolicy::Rename,
            },
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
    fn parses_progress() {
        let progress = parse_ffmpeg_progress_line("out_time_ms=5000000", Some(10.0)).unwrap();
        assert!((progress - 0.5).abs() < 0.01);
    }
}
