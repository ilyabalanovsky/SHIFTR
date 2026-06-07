use crate::{
    engines::{self, resolve_output_path},
    models::{ConversionJob, JobStatus, QueueOptions},
};
use anyhow::{Result, anyhow};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::AppHandle;
use tokio::sync::{Mutex, Semaphore};

#[derive(Default)]
pub struct QueueState {
    jobs: Mutex<HashMap<String, ConversionJob>>,
    cancel_flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
    paused: AtomicBool,
}

impl QueueState {
    pub async fn set_jobs(&self, jobs: Vec<ConversionJob>) {
        let mut map = self.jobs.lock().await;
        map.clear();
        for job in jobs {
            map.insert(job.id.clone(), job);
        }
    }

    pub async fn add_jobs(&self, jobs: Vec<ConversionJob>) {
        let mut map = self.jobs.lock().await;
        for job in jobs {
            map.insert(job.id.clone(), job);
        }
    }

    pub async fn jobs(&self) -> Vec<ConversionJob> {
        self.jobs.lock().await.values().cloned().collect()
    }

    pub async fn update_queued_options(&self, options: &QueueOptions) -> Result<()> {
        let mut jobs = self.jobs.lock().await;
        let mut reserved_paths = HashSet::new();
        for job in jobs
            .values_mut()
            .filter(|job| job.status == JobStatus::Queued)
        {
            job.target_format = options.target_format.clone();
            job.preset = options.preset.clone();
            job.advanced_options = options.advanced_options.clone();
            let output_path = resolve_output_path(
                &job.input_path,
                options.output_dir.as_deref(),
                &options.target_format,
                &options.preset.overwrite_policy,
            )?;
            job.output_path =
                reserve_output_path(&output_path, &job.input_path, &mut reserved_paths)?;
        }
        Ok(())
    }

    pub async fn rename_output_file(&self, id: &str, output_name: &str) -> Result<ConversionJob> {
        let mut jobs = self.jobs.lock().await;
        let existing_paths = jobs
            .values()
            .filter(|item| item.id != id)
            .map(|item| normalized_path_key(&item.output_path))
            .collect::<HashSet<_>>();
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| anyhow!("Job was not found"))?;
        if job.status != JobStatus::Queued {
            return Err(anyhow!("Only queued jobs can be renamed"));
        }

        let output = Path::new(&job.output_path);
        let parent = output
            .parent()
            .ok_or_else(|| anyhow!("Cannot resolve output directory"))?;
        let file_name = normalized_output_name(output_name, &job.target_format)?;
        let output_path = unique_output_path(parent, &file_name, &job.input_path, &existing_paths)?;
        job.output_path = output_path.to_string_lossy().to_string();
        Ok(job.clone())
    }

    pub async fn retry_job(&self, id: &str) -> Result<ConversionJob> {
        let mut jobs = self.jobs.lock().await;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| anyhow!("Job was not found"))?;
        if !matches!(job.status, JobStatus::Failed | JobStatus::Canceled) {
            return Err(anyhow!("Only failed or canceled jobs can be retried"));
        }

        job.status = JobStatus::Queued;
        job.progress = 0.0;
        job.speed = None;
        job.processing_seconds = None;
        job.eta_seconds = None;
        job.error = None;
        job.error_details = None;
        Ok(job.clone())
    }

    pub async fn remove_job(&self, id: &str) -> Vec<ConversionJob> {
        if let Some(flag) = self.cancel_flags.lock().await.remove(id) {
            flag.store(true, Ordering::Relaxed);
        }

        self.jobs.lock().await.remove(id);
        self.jobs().await
    }

    pub async fn clear_finished(&self) -> Vec<ConversionJob> {
        let mut jobs = self.jobs.lock().await;
        jobs.retain(|_, job| matches!(job.status, JobStatus::Queued | JobStatus::Running));
        jobs.values().cloned().collect()
    }

    pub async fn clear_all(&self) {
        let mut flags = self.cancel_flags.lock().await;
        for flag in flags.values() {
            flag.store(true, Ordering::Relaxed);
        }
        flags.clear();
        self.jobs.lock().await.clear();
    }

    pub async fn cancel_job(&self, id: &str) -> Option<ConversionJob> {
        if let Some(flag) = self.cancel_flags.lock().await.get(id) {
            flag.store(true, Ordering::Relaxed);
        }

        let mut jobs = self.jobs.lock().await;
        let job = jobs.get_mut(id)?;
        job.status = JobStatus::Canceled;
        job.eta_seconds = None;
        Some(job.clone())
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    pub async fn start(
        self: Arc<Self>,
        app: AppHandle,
        options: QueueOptions,
    ) -> Vec<ConversionJob> {
        self.paused.store(false, Ordering::Relaxed);
        let jobs = self.jobs().await;
        let parallelism = options
            .parallelism
            .unwrap_or_else(default_parallelism)
            .max(1);
        let semaphore = Arc::new(Semaphore::new(parallelism));
        let mut handles = Vec::new();

        for job in jobs
            .into_iter()
            .filter(|job| job.status == JobStatus::Queued)
        {
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore closed");
            if self.paused.load(Ordering::Relaxed) {
                break;
            }
            let state = self.clone();
            let app = app.clone();
            let ffmpeg_path = options.ffmpeg_path.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            self.cancel_flags
                .lock()
                .await
                .insert(job.id.clone(), cancel.clone());

            handles.push(tauri::async_runtime::spawn(async move {
                let _permit = permit;
                let completed = engines::convert_job(app, job, ffmpeg_path, cancel).await;
                let mut jobs = state.jobs.lock().await;
                if jobs.contains_key(&completed.id) {
                    jobs.insert(completed.id.clone(), completed.clone());
                }
                completed
            }));
        }

        let mut completed = Vec::new();
        for handle in handles {
            if let Ok(job) = handle.await {
                completed.push(job);
            }
        }
        completed
    }
}

pub fn default_parallelism() -> usize {
    let cpu_half = (num_cpus::get() / 2).max(1);
    cpu_half.min(2)
}

fn normalized_output_name(output_name: &str, target_format: &str) -> Result<String> {
    let trimmed = output_name.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Output file name cannot be empty"));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(anyhow!("Output file name cannot contain folders"));
    }
    if trimmed
        .chars()
        .any(|ch| matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err(anyhow!("Output file name contains invalid characters"));
    }

    let path = Path::new(trimmed);
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| anyhow!("Output file name is invalid"))?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());

    if extension.as_deref() == Some(target_format) {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("{stem}.{target_format}"))
    }
}

fn unique_output_path(
    parent: &Path,
    file_name: &str,
    input_path: &str,
    existing_paths: &HashSet<String>,
) -> Result<PathBuf> {
    let candidate = parent.join(file_name);
    if is_available_output_path(&candidate, input_path, existing_paths) {
        return Ok(candidate);
    }

    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("Output file name is invalid"))?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or_else(|| anyhow!("Output file extension is invalid"))?;

    for index in 1..1000 {
        let candidate = parent.join(format!("{stem} ({index}).{extension}"));
        if is_available_output_path(&candidate, input_path, existing_paths) {
            return Ok(candidate);
        }
    }

    Err(anyhow!("Could not find a free output file name"))
}

fn reserve_output_path(
    output_path: &str,
    input_path: &str,
    reserved_paths: &mut HashSet<String>,
) -> Result<String> {
    let path = Path::new(output_path);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Cannot resolve output directory"))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("Output file name is invalid"))?;
    let candidate = unique_output_path(parent, file_name, input_path, reserved_paths)?;
    let output = candidate.to_string_lossy().to_string();
    reserved_paths.insert(normalized_path_key(&output));
    Ok(output)
}

fn is_available_output_path(
    candidate: &Path,
    input_path: &str,
    existing_paths: &HashSet<String>,
) -> bool {
    !candidate.exists()
        && normalized_path_key(&candidate.to_string_lossy()) != normalized_path_key(input_path)
        && !existing_paths.contains(&normalized_path_key(&candidate.to_string_lossy()))
}

fn normalized_path_key(path: &str) -> String {
    if cfg!(windows) {
        path.to_ascii_lowercase()
    } else {
        path.to_string()
    }
}
