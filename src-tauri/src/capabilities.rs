use crate::models::{CodecOption, ConversionCapabilities, FormatCodecMatrix};
use std::{collections::HashSet, path::Path, process::Command};

pub fn get_capabilities(ffmpeg_path: Option<String>) -> ConversionCapabilities {
    let Some(ffmpeg) = find_ffmpeg(ffmpeg_path) else {
        return fallback_capabilities(false, None, vec!["FFmpeg was not found.".into()]);
    };

    let encoders_output = Command::new(&ffmpeg).arg("-encoders").output();
    let hwaccels_output = Command::new(&ffmpeg).arg("-hwaccels").output();

    match encoders_output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let encoders = parse_encoder_ids(&text);
            let hwaccels = hwaccels_output
                .ok()
                .filter(|output| output.status.success())
                .map(|output| parse_hwaccels(&String::from_utf8_lossy(&output.stdout)))
                .unwrap_or_default();
            capabilities_from_encoders(true, Some(ffmpeg), &encoders, hwaccels, Vec::new())
        }
        Ok(output) => fallback_capabilities(
            false,
            Some(ffmpeg),
            vec![format!(
                "FFmpeg encoder detection failed with status {}.",
                output.status
            )],
        ),
        Err(error) => fallback_capabilities(false, Some(ffmpeg), vec![error.to_string()]),
    }
}

pub fn allowed_video_codec_for_format(
    target_format: &str,
    requested: Option<&str>,
) -> &'static str {
    let allowed = video_codec_ids_for_format(target_format);
    requested
        .and_then(|codec| allowed.iter().copied().find(|allowed| *allowed == codec))
        .unwrap_or(allowed[0])
}

pub fn allowed_audio_codec_for_format(
    target_format: &str,
    requested: Option<&str>,
) -> &'static str {
    let allowed = audio_codec_ids_for_format(target_format);
    requested
        .and_then(|codec| allowed.iter().copied().find(|allowed| *allowed == codec))
        .unwrap_or(allowed[0])
}

pub fn video_codec_ids_for_format(target_format: &str) -> &'static [&'static str] {
    match target_format {
        "webm" => &["libvpx-vp9", "libvpx", "libaom-av1"],
        "mp4" | "mkv" | "mov" => &[
            "libx264",
            "h264_nvenc",
            "h264_qsv",
            "h264_amf",
            "mpeg4",
            "libx265",
            "hevc_nvenc",
            "hevc_qsv",
            "hevc_amf",
            "libaom-av1",
        ],
        "avi" => &["mpeg4", "mjpeg"],
        _ => &["mpeg4"],
    }
}

pub fn audio_codec_ids_for_format(target_format: &str) -> &'static [&'static str] {
    match target_format {
        "webm" => &["libopus", "libvorbis"],
        "ogg" => &["libopus", "libvorbis"],
        "opus" => &["libopus"],
        "flac" => &["flac"],
        "wav" => &["pcm_s16le"],
        "mp3" => &["libmp3lame"],
        "aac" | "m4a" | "mp4" | "mov" => &["aac"],
        "mkv" => &["aac", "libopus", "flac"],
        "avi" => &["aac", "libmp3lame"],
        _ => &["aac"],
    }
}

fn capabilities_from_encoders(
    ffmpeg_available: bool,
    ffmpeg_path: Option<String>,
    encoders: &HashSet<String>,
    hardware_accels: Vec<String>,
    warnings: Vec<String>,
) -> ConversionCapabilities {
    let video_ids = [
        "libx264",
        "h264_nvenc",
        "h264_qsv",
        "h264_amf",
        "mpeg4",
        "libx265",
        "hevc_nvenc",
        "hevc_qsv",
        "hevc_amf",
        "libvpx-vp9",
        "libvpx",
        "libaom-av1",
        "mjpeg",
    ];
    let audio_ids = [
        "aac",
        "libopus",
        "libvorbis",
        "flac",
        "pcm_s16le",
        "libmp3lame",
    ];
    let target_formats = [
        "mp4", "mkv", "mov", "webm", "avi", "mp3", "aac", "m4a", "ogg", "opus", "wav", "flac",
    ];

    ConversionCapabilities {
        ffmpeg_available,
        ffmpeg_path,
        hardware_accels,
        video_encoders: video_ids
            .iter()
            .map(|id| codec_option(id, encoders))
            .collect(),
        audio_encoders: audio_ids
            .iter()
            .map(|id| codec_option(id, encoders))
            .collect(),
        matrix: target_formats
            .iter()
            .map(|format| FormatCodecMatrix {
                target_format: (*format).into(),
                video_codecs: video_codec_ids_for_format(format)
                    .iter()
                    .map(|id| codec_option(id, encoders))
                    .collect(),
                audio_codecs: audio_codec_ids_for_format(format)
                    .iter()
                    .map(|id| codec_option(id, encoders))
                    .collect(),
                supports_video: matches!(*format, "mp4" | "mkv" | "mov" | "webm" | "avi"),
                supports_audio: true,
                supports_remux: matches!(*format, "mp4" | "mkv" | "mov" | "webm" | "avi"),
            })
            .collect(),
        warnings,
    }
}

fn fallback_capabilities(
    ffmpeg_available: bool,
    ffmpeg_path: Option<String>,
    warnings: Vec<String>,
) -> ConversionCapabilities {
    capabilities_from_encoders(
        ffmpeg_available,
        ffmpeg_path,
        &fallback_encoder_ids(),
        Vec::new(),
        warnings,
    )
}

fn fallback_encoder_ids() -> HashSet<String> {
    ["mpeg4", "libvpx-vp9", "aac", "libopus", "flac", "pcm_s16le"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn codec_option(id: &str, encoders: &HashSet<String>) -> CodecOption {
    CodecOption {
        id: id.into(),
        label: codec_label(id),
        available: encoders.contains(id),
        hardware: id.contains("_nvenc")
            || id.contains("_qsv")
            || id.contains("_amf")
            || id.contains("videotoolbox"),
    }
}

fn codec_label(id: &str) -> String {
    match id {
        "libx264" => "H.264 x264",
        "h264_nvenc" => "H.264 NVIDIA NVENC",
        "h264_qsv" => "H.264 Intel QSV",
        "h264_amf" => "H.264 AMD AMF",
        "mpeg4" => "MPEG-4 Part 2",
        "libx265" => "HEVC x265",
        "hevc_nvenc" => "HEVC NVIDIA NVENC",
        "hevc_qsv" => "HEVC Intel QSV",
        "hevc_amf" => "HEVC AMD AMF",
        "libvpx-vp9" => "VP9",
        "libvpx" => "VP8",
        "libaom-av1" => "AV1 libaom",
        "mjpeg" => "Motion JPEG",
        "aac" => "AAC",
        "libopus" => "Opus",
        "libvorbis" => "Vorbis",
        "flac" => "FLAC",
        "pcm_s16le" => "WAV PCM",
        "libmp3lame" => "MP3 LAME",
        _ => id,
    }
    .into()
}

fn parse_encoder_ids(text: &str) -> HashSet<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let mut parts = trimmed.split_whitespace();
            let flags = parts.next()?;
            if flags.len() == 6 && (flags.starts_with('V') || flags.starts_with('A')) {
                parts.next().map(String::from)
            } else {
                None
            }
        })
        .collect()
}

fn parse_hwaccels(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("Hardware acceleration"))
        .map(String::from)
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_encoder_ids() {
        let encoders = parse_encoder_ids(" V..... libx264 H.264\n A..... aac AAC\n ------ nope");
        assert!(encoders.contains("libx264"));
        assert!(encoders.contains("aac"));
        assert!(!encoders.contains("nope"));
    }

    #[test]
    fn matrix_restricts_webm_codecs() {
        assert!(video_codec_ids_for_format("webm").contains(&"libvpx-vp9"));
        assert!(!video_codec_ids_for_format("webm").contains(&"libx264"));
    }
}
