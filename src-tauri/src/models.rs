use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FileCategory {
    Video,
    Audio,
    Image,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QualityMode {
    SmallSize,
    Balanced,
    HighQuality,
    KeepSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverwritePolicy {
    Rename,
    Overwrite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionPreset {
    pub name: String,
    pub quality_mode: QualityMode,
    pub overwrite_policy: OverwritePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionJob {
    pub id: String,
    pub input_path: String,
    pub output_path: String,
    pub source_format: String,
    pub target_format: String,
    pub category: FileCategory,
    pub preset: ConversionPreset,
    pub status: JobStatus,
    pub progress: f32,
    pub speed: Option<String>,
    pub eta_seconds: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub path: String,
    pub file_name: String,
    pub source_format: String,
    pub category: FileCategory,
    pub duration_seconds: Option<f64>,
    pub streams: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedFormats {
    pub video: Vec<String>,
    pub audio: Vec<String>,
    pub image: Vec<String>,
    pub presets: Vec<ConversionPreset>,
    pub default_parallelism: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueOptions {
    pub output_dir: Option<String>,
    pub target_format: String,
    pub preset: ConversionPreset,
    pub parallelism: Option<usize>,
    pub ffmpeg_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueUpdate {
    pub job: ConversionJob,
}

pub const VIDEO_FORMATS: &[&str] = &["mp4", "mkv", "mov", "webm", "avi"];
pub const AUDIO_FORMATS: &[&str] = &["mp3", "aac", "m4a", "ogg", "opus", "wav", "flac"];
pub const IMAGE_FORMATS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "tiff"];

pub fn category_for_extension(ext: &str) -> FileCategory {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    if VIDEO_FORMATS.contains(&ext.as_str()) {
        FileCategory::Video
    } else if AUDIO_FORMATS.contains(&ext.as_str()) {
        FileCategory::Audio
    } else if IMAGE_FORMATS.contains(&ext.as_str()) {
        FileCategory::Image
    } else {
        FileCategory::Unsupported
    }
}

pub fn extension_for_path(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

pub fn default_presets() -> Vec<ConversionPreset> {
    vec![
        ConversionPreset {
            name: "Balanced".into(),
            quality_mode: QualityMode::Balanced,
            overwrite_policy: OverwritePolicy::Rename,
        },
        ConversionPreset {
            name: "Small size".into(),
            quality_mode: QualityMode::SmallSize,
            overwrite_policy: OverwritePolicy::Rename,
        },
        ConversionPreset {
            name: "High quality".into(),
            quality_mode: QualityMode::HighQuality,
            overwrite_policy: OverwritePolicy::Rename,
        },
        ConversionPreset {
            name: "Keep source quality".into(),
            quality_mode: QualityMode::KeepSource,
            overwrite_policy: OverwritePolicy::Rename,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_categories() {
        assert_eq!(category_for_extension("mp4"), FileCategory::Video);
        assert_eq!(category_for_extension(".flac"), FileCategory::Audio);
        assert_eq!(category_for_extension("webp"), FileCategory::Image);
        assert_eq!(category_for_extension("pdf"), FileCategory::Unsupported);
    }
}
