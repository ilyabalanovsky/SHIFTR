use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FileCategory {
    Video,
    Audio,
    Image,
    Document,
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
    FastRemux,
    FastEncode,
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
    pub description: String,
    pub quality_mode: QualityMode,
    pub overwrite_policy: OverwritePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionJob {
    pub id: String,
    pub input_path: String,
    pub input_paths: Vec<String>,
    pub output_path: String,
    pub source_format: String,
    pub target_format: String,
    pub category: FileCategory,
    pub preset: ConversionPreset,
    pub advanced_options: Option<AdvancedOptions>,
    pub document_operation: Option<DocumentOperation>,
    pub status: JobStatus,
    pub progress: f32,
    pub speed: Option<String>,
    pub processing_seconds: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub error: Option<String>,
    pub error_details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DocumentOperation {
    ImagesToPdf,
    MergePdfs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvancedOptions {
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub video_quality: Option<u8>,
    pub video_bitrate: Option<String>,
    pub audio_bitrate: Option<String>,
    pub max_width: Option<u32>,
    pub frame_rate: Option<u32>,
    pub target_size_mb: Option<u32>,
    pub image_quality: Option<u8>,
    pub copy_streams: bool,
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
    pub document: Vec<String>,
    pub presets: Vec<ConversionPreset>,
    pub default_parallelism: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodecOption {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub hardware: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatCodecMatrix {
    pub target_format: String,
    pub video_codecs: Vec<CodecOption>,
    pub audio_codecs: Vec<CodecOption>,
    pub supports_video: bool,
    pub supports_audio: bool,
    pub supports_remux: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionCapabilities {
    pub ffmpeg_available: bool,
    pub ffmpeg_path: Option<String>,
    pub hardware_accels: Vec<String>,
    pub video_encoders: Vec<CodecOption>,
    pub audio_encoders: Vec<CodecOption>,
    pub matrix: Vec<FormatCodecMatrix>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueOptions {
    pub output_dir: Option<String>,
    pub target_format: String,
    pub preset: ConversionPreset,
    pub advanced_options: Option<AdvancedOptions>,
    pub parallelism: Option<usize>,
    pub ffmpeg_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateJobGroup {
    pub paths: Vec<String>,
    pub options: QueueOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentJobOptions {
    pub paths: Vec<String>,
    pub output_dir: Option<String>,
    pub output_name: Option<String>,
    pub operation: DocumentOperation,
    pub parallelism: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameOutputOptions {
    pub id: String,
    pub output_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeTargetValidationRequest {
    pub paths: Vec<String>,
    pub category: FileCategory,
    pub target_size_mb: u32,
    pub audio_bitrate: Option<String>,
    pub ffmpeg_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeTargetFileEstimate {
    pub path: String,
    pub duration_seconds: Option<f64>,
    pub total_kbps: Option<u32>,
    pub video_kbps: Option<u32>,
    pub audio_kbps: Option<u32>,
    pub applicable: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeTargetValidation {
    pub applicable: bool,
    pub warnings: Vec<String>,
    pub estimates: Vec<SizeTargetFileEstimate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodingPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: FileCategory,
    pub platform: Option<String>,
    pub target_format: String,
    pub preset: ConversionPreset,
    pub advanced_options: Option<AdvancedOptions>,
    pub built_in: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodingPresetStore {
    pub schema_version: u32,
    pub presets: Vec<EncodingPreset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueUpdate {
    pub job: ConversionJob,
}

pub const VIDEO_FORMATS: &[&str] = &["mp4", "mkv", "mov", "webm", "avi"];
pub const AUDIO_FORMATS: &[&str] = &["mp3", "aac", "m4a", "ogg", "opus", "wav", "flac"];
pub const IMAGE_FORMATS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "tiff"];
pub const DOCUMENT_FORMATS: &[&str] = &["pdf"];

pub fn category_for_extension(ext: &str) -> FileCategory {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    if VIDEO_FORMATS.contains(&ext.as_str()) {
        FileCategory::Video
    } else if AUDIO_FORMATS.contains(&ext.as_str()) {
        FileCategory::Audio
    } else if IMAGE_FORMATS.contains(&ext.as_str()) {
        FileCategory::Image
    } else if DOCUMENT_FORMATS.contains(&ext.as_str()) {
        FileCategory::Document
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
            name: "Fast remux".into(),
            description: "Changes the container without re-encoding when streams are compatible."
                .into(),
            quality_mode: QualityMode::FastRemux,
            overwrite_policy: OverwritePolicy::Rename,
        },
        ConversionPreset {
            name: "Fast encode".into(),
            description:
                "Prioritizes speed with moderate compression and broadly compatible codecs.".into(),
            quality_mode: QualityMode::FastEncode,
            overwrite_policy: OverwritePolicy::Rename,
        },
        ConversionPreset {
            name: "Balanced".into(),
            description: "Good default quality, size, and compatibility for everyday conversion."
                .into(),
            quality_mode: QualityMode::Balanced,
            overwrite_policy: OverwritePolicy::Rename,
        },
        ConversionPreset {
            name: "Small size".into(),
            description: "Smaller files with more compression and lower audio bitrates.".into(),
            quality_mode: QualityMode::SmallSize,
            overwrite_policy: OverwritePolicy::Rename,
        },
        ConversionPreset {
            name: "High quality".into(),
            description: "Preserves more detail with larger files and slower encoding.".into(),
            quality_mode: QualityMode::HighQuality,
            overwrite_policy: OverwritePolicy::Rename,
        },
        ConversionPreset {
            name: "Keep source quality".into(),
            description:
                "Copies streams when possible, otherwise uses conservative quality settings.".into(),
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
        assert_eq!(category_for_extension("pdf"), FileCategory::Document);
        assert_eq!(category_for_extension("zip"), FileCategory::Unsupported);
    }
}
