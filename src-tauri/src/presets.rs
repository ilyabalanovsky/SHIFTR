use crate::models::{
    AdvancedOptions, ConversionPreset, EncodingPreset, EncodingPresetStore, FileCategory,
    OverwritePolicy, QualityMode,
};
use anyhow::{Context, Result};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

const SCHEMA_VERSION: u32 = 1;

pub fn all_presets(app: &AppHandle) -> Result<Vec<EncodingPreset>> {
    let mut presets = built_in_presets();
    presets.extend(load_custom_presets(app)?);
    Ok(presets)
}

pub fn save_custom_preset(
    app: &AppHandle,
    mut preset: EncodingPreset,
) -> Result<Vec<EncodingPreset>> {
    preset.built_in = false;
    if preset.id.trim().is_empty() || preset.built_in {
        preset.id = custom_id();
    }
    if !preset.id.starts_with("custom_") {
        preset.id = format!("custom_{}", preset.id);
    }

    let mut custom = load_custom_presets(app)?;
    custom.retain(|item| item.id != preset.id);
    custom.push(preset);
    write_custom_presets(app, custom)?;
    all_presets(app)
}

pub fn delete_custom_preset(app: &AppHandle, id: &str) -> Result<Vec<EncodingPreset>> {
    let mut custom = load_custom_presets(app)?;
    custom.retain(|preset| preset.id != id || preset.built_in);
    write_custom_presets(app, custom)?;
    all_presets(app)
}

pub fn export_custom_preset(app: &AppHandle, id: &str, path: &str) -> Result<()> {
    let preset = load_custom_presets(app)?
        .into_iter()
        .find(|preset| preset.id == id)
        .context("Custom recipe was not found")?;
    let store = EncodingPresetStore {
        schema_version: SCHEMA_VERSION,
        presets: vec![preset],
    };
    let text = serde_json::to_string_pretty(&store).context("Could not serialize custom recipe")?;
    fs::write(path, text).context("Could not export custom recipe")
}

pub fn import_custom_presets(app: &AppHandle, path: &str) -> Result<Vec<EncodingPreset>> {
    let text = fs::read_to_string(path).context("Could not read recipe import file")?;
    let mut imported = serde_json::from_str::<EncodingPresetStore>(&text)
        .map(|store| store.presets)
        .or_else(|_| serde_json::from_str::<Vec<EncodingPreset>>(&text))
        .context("Could not parse recipe import file")?;

    let mut custom = load_custom_presets(app)?;
    for mut preset in imported.drain(..) {
        preset.built_in = false;
        preset.id = custom_id();
        if preset.name.trim().is_empty() {
            preset.name = "Imported recipe".into();
        }
        if preset.description.trim().is_empty() {
            preset.description = "Imported custom encoding recipe.".into();
        }
        preset.platform = preset.platform.or_else(|| Some("Imported".into()));
        custom.push(preset);
    }

    write_custom_presets(app, custom)?;
    all_presets(app)
}

fn custom_id() -> String {
    format!("custom_{}", Uuid::new_v4())
}

fn presets_file(app: &AppHandle) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_config_dir()
        .context("Could not resolve app config directory")?;
    fs::create_dir_all(&dir).context("Could not create app config directory")?;
    Ok(dir.join("presets.json"))
}

fn load_custom_presets(app: &AppHandle) -> Result<Vec<EncodingPreset>> {
    let path = presets_file(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let text = fs::read_to_string(path).context("Could not read custom presets")?;
    let store = serde_json::from_str::<EncodingPresetStore>(&text)
        .context("Could not parse custom presets")?;
    Ok(store
        .presets
        .into_iter()
        .map(|mut preset| {
            preset.built_in = false;
            preset
        })
        .collect())
}

fn write_custom_presets(app: &AppHandle, presets: Vec<EncodingPreset>) -> Result<()> {
    let path = presets_file(app)?;
    let store = EncodingPresetStore {
        schema_version: SCHEMA_VERSION,
        presets,
    };
    let text =
        serde_json::to_string_pretty(&store).context("Could not serialize custom presets")?;
    fs::write(path, text).context("Could not write custom presets")
}

fn built_in_presets() -> Vec<EncodingPreset> {
    vec![
        video(
            "youtube_1080p",
            "YouTube 1080p",
            "YouTube",
            "MP4 H.264/AAC tuned for standard 1080p uploads.",
            "mp4",
            QualityMode::HighQuality,
            Some(AdvancedOptions {
                video_codec: Some("libx264".into()),
                audio_codec: Some("aac".into()),
                video_quality: Some(78),
                video_bitrate: None,
                audio_bitrate: Some("192k".into()),
                max_width: Some(1920),
                frame_rate: None,
                target_size_mb: None,
                image_quality: None,
                copy_streams: false,
            }),
        ),
        video(
            "youtube_4k",
            "YouTube 4K",
            "YouTube",
            "High quality MP4 for 4K uploads while keeping broad playback compatibility.",
            "mp4",
            QualityMode::HighQuality,
            Some(AdvancedOptions {
                video_codec: Some("libx264".into()),
                audio_codec: Some("aac".into()),
                video_quality: Some(86),
                video_bitrate: None,
                audio_bitrate: Some("256k".into()),
                max_width: Some(3840),
                frame_rate: None,
                target_size_mb: None,
                image_quality: None,
                copy_streams: false,
            }),
        ),
        video(
            "instagram_reels",
            "Instagram Reels",
            "Instagram",
            "Compact MP4 for vertical short-form posts and Reels.",
            "mp4",
            QualityMode::Balanced,
            Some(AdvancedOptions {
                video_codec: Some("libx264".into()),
                audio_codec: Some("aac".into()),
                video_quality: Some(68),
                video_bitrate: None,
                audio_bitrate: Some("160k".into()),
                max_width: Some(1080),
                frame_rate: Some(30),
                target_size_mb: None,
                image_quality: None,
                copy_streams: false,
            }),
        ),
        video(
            "tiktok_vertical",
            "TikTok Vertical",
            "TikTok",
            "Fast-compatible MP4 for TikTok-style vertical video.",
            "mp4",
            QualityMode::Balanced,
            Some(AdvancedOptions {
                video_codec: Some("libx264".into()),
                audio_codec: Some("aac".into()),
                video_quality: Some(70),
                video_bitrate: None,
                audio_bitrate: Some("160k".into()),
                max_width: Some(1080),
                frame_rate: Some(30),
                target_size_mb: None,
                image_quality: None,
                copy_streams: false,
            }),
        ),
        video(
            "telegram_video",
            "Telegram Video",
            "Telegram",
            "Balanced MP4 that favors compatibility and manageable file size.",
            "mp4",
            QualityMode::Balanced,
            Some(AdvancedOptions {
                video_codec: Some("libx264".into()),
                audio_codec: Some("aac".into()),
                video_quality: Some(62),
                video_bitrate: None,
                audio_bitrate: Some("128k".into()),
                max_width: Some(1280),
                frame_rate: Some(30),
                target_size_mb: Some(100),
                image_quality: None,
                copy_streams: false,
            }),
        ),
        video(
            "discord_small",
            "Discord Small Upload",
            "Discord",
            "Small MP4 for sharing clips where upload size matters.",
            "mp4",
            QualityMode::SmallSize,
            Some(AdvancedOptions {
                video_codec: Some("libx264".into()),
                audio_codec: Some("aac".into()),
                video_quality: Some(42),
                video_bitrate: None,
                audio_bitrate: Some("96k".into()),
                max_width: Some(1280),
                frame_rate: Some(30),
                target_size_mb: Some(25),
                image_quality: None,
                copy_streams: false,
            }),
        ),
        audio(
            "podcast_mp3",
            "Podcast MP3",
            "Podcast",
            "MP3 voice export for broad publishing compatibility.",
            "mp3",
            Some("192k"),
        ),
        audio(
            "audio_archive_flac",
            "Audio Archive FLAC",
            "Archive",
            "Lossless FLAC for keeping source quality where possible.",
            "flac",
            None,
        ),
        image(
            "website_webp",
            "Website WebP",
            "Website",
            "WebP images for modern websites with a strong size-quality balance.",
            "webp",
            82,
        ),
        image(
            "website_hero_jpg",
            "Website Hero JPG",
            "Website",
            "High quality JPG for wide hero images and broad CMS compatibility.",
            "jpg",
            88,
        ),
        image(
            "social_square_jpg",
            "Social Square JPG",
            "Social",
            "High quality JPG for feed posts and social sharing.",
            "jpg",
            90,
        ),
    ]
}

fn video(
    id: &str,
    name: &str,
    platform: &str,
    description: &str,
    target_format: &str,
    quality_mode: QualityMode,
    advanced_options: Option<AdvancedOptions>,
) -> EncodingPreset {
    EncodingPreset {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        category: FileCategory::Video,
        platform: Some(platform.into()),
        target_format: target_format.into(),
        preset: recipe_preset(name, description, quality_mode),
        advanced_options,
        built_in: true,
    }
}

fn audio(
    id: &str,
    name: &str,
    platform: &str,
    description: &str,
    target_format: &str,
    audio_bitrate: Option<&str>,
) -> EncodingPreset {
    EncodingPreset {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        category: FileCategory::Audio,
        platform: Some(platform.into()),
        target_format: target_format.into(),
        preset: recipe_preset(name, description, QualityMode::Balanced),
        advanced_options: Some(AdvancedOptions {
            video_codec: None,
            audio_codec: None,
            video_quality: None,
            video_bitrate: None,
            audio_bitrate: audio_bitrate.map(String::from),
            max_width: None,
            frame_rate: None,
            target_size_mb: None,
            image_quality: None,
            copy_streams: false,
        }),
        built_in: true,
    }
}

fn image(
    id: &str,
    name: &str,
    platform: &str,
    description: &str,
    target_format: &str,
    quality: u8,
) -> EncodingPreset {
    EncodingPreset {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        category: FileCategory::Image,
        platform: Some(platform.into()),
        target_format: target_format.into(),
        preset: recipe_preset(name, description, QualityMode::Balanced),
        advanced_options: Some(AdvancedOptions {
            video_codec: None,
            audio_codec: None,
            video_quality: None,
            video_bitrate: None,
            audio_bitrate: None,
            max_width: None,
            frame_rate: None,
            target_size_mb: None,
            image_quality: Some(quality),
            copy_streams: false,
        }),
        built_in: true,
    }
}

fn recipe_preset(name: &str, description: &str, quality_mode: QualityMode) -> ConversionPreset {
    ConversionPreset {
        name: name.into(),
        description: description.into(),
        quality_mode,
        overwrite_policy: OverwritePolicy::Rename,
    }
}
