use crate::{
    engines::{self, resolve_output_path},
    models::{ConversionJob, JobStatus, QueueOptions},
};
use anyhow::Result;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
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

    pub async fn jobs(&self) -> Vec<ConversionJob> {
        self.jobs.lock().await.values().cloned().collect()
    }

    pub async fn update_queued_options(&self, options: &QueueOptions) -> Result<()> {
        let mut jobs = self.jobs.lock().await;
        for job in jobs.values_mut().filter(|job| job.status == JobStatus::Queued) {
            job.target_format = options.target_format.clone();
            job.preset = options.preset.clone();
            job.output_path = resolve_output_path(
                &job.input_path,
                options.output_dir.as_deref(),
                &options.target_format,
                &options.preset.overwrite_policy,
            )?;
        }
        Ok(())
    }

    pub async fn cancel_job(&self, id: &str) -> Option<ConversionJob> {
        if let Some(flag) = self.cancel_flags.lock().await.get(id) {
            flag.store(true, Ordering::Relaxed);
        }

        let mut jobs = self.jobs.lock().await;
        let job = jobs.get_mut(id)?;
        job.status = JobStatus::Canceled;
        Some(job.clone())
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    pub async fn start(self: Arc<Self>, app: AppHandle, options: QueueOptions) -> Vec<ConversionJob> {
        self.paused.store(false, Ordering::Relaxed);
        let jobs = self.jobs().await;
        let parallelism = options.parallelism.unwrap_or_else(default_parallelism).max(1);
        let semaphore = Arc::new(Semaphore::new(parallelism));
        let mut handles = Vec::new();

        for job in jobs.into_iter().filter(|job| job.status == JobStatus::Queued) {
            let permit = semaphore.clone().acquire_owned().await.expect("semaphore closed");
            if self.paused.load(Ordering::Relaxed) {
                break;
            }
            let state = self.clone();
            let app = app.clone();
            let ffmpeg_path = options.ffmpeg_path.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            self.cancel_flags.lock().await.insert(job.id.clone(), cancel.clone());

            handles.push(tauri::async_runtime::spawn(async move {
                let _permit = permit;
                let completed = engines::convert_job(app, job, ffmpeg_path, cancel).await;
                state.jobs.lock().await.insert(completed.id.clone(), completed.clone());
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
