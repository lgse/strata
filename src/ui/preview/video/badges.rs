// SPDX-License-Identifier: MIT

use crate::sandbox::metadata::MediaMetadata;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Resolution,
    Hdr,
    BitDepth,
    FrameRate,
    Codec,
    Audio,
    Captions,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Badge {
    pub(super) kind: Kind,
    pub(super) label: String,
}

fn badge(kind: Kind, label: impl Into<String>) -> Badge {
    Badge {
        kind,
        label: label.into(),
    }
}

pub(super) fn badges(metadata: &MediaMetadata, sidecar_captions: usize) -> Vec<Badge> {
    let mut badges = Vec::new();
    if let Some((width, height)) = metadata.dimensions {
        badges.push(badge(Kind::Resolution, resolution_class(width, height)));
    }
    if let Some(hdr) = metadata.hdr_format() {
        badges.push(badge(Kind::Hdr, hdr));
    }
    if let Some(depth) = bit_depth(metadata.pixel_format.as_deref()) {
        badges.push(badge(Kind::BitDepth, depth));
    }
    if let Some(rate) = metadata.frame_rate {
        badges.push(badge(Kind::FrameRate, frame_rate_label(rate)));
    }
    if let Some(codec) = metadata.video_codec.as_deref() {
        badges.push(badge(Kind::Codec, codec_label(codec)));
    }
    if let Some(audio) = audio_label(
        metadata.audio_codec.as_deref(),
        metadata.channels,
        metadata.channel_layout.as_deref(),
    ) {
        badges.push(badge(Kind::Audio, audio));
    }
    let captions = metadata.subtitle_tracks.len() + sidecar_captions;
    if captions > 0 {
        badges.push(badge(
            Kind::Captions,
            if captions == 1 {
                "CC".to_owned()
            } else {
                format!("CC ×{captions}")
            },
        ));
    }
    badges
}

/// Classes by the long edge so portrait video reads the same as landscape.
fn resolution_class(width: u32, height: u32) -> &'static str {
    match width.max(height) {
        7600.. => "8K",
        3800.. => "4K",
        2000.. => "2K",
        1900.. => "1080p",
        1260.. => "720p",
        _ => "SD",
    }
}

fn bit_depth(pixel_format: Option<&str>) -> Option<&'static str> {
    let format = pixel_format?;
    if format.contains("p16") || format.contains("16le") || format.contains("16be") {
        Some("16-bit")
    } else if format.contains("p12") || format.contains("12le") || format.contains("12be") {
        Some("12-bit")
    } else if format.contains("p10") || format.contains("10le") || format.contains("10be") {
        Some("10-bit")
    } else {
        None
    }
}

fn frame_rate_label(rate: f64) -> String {
    rust_i18n::t!(
        "%{rate} fps",
        rate = crate::i18n::integer(rate.round() as u64)
    )
    .into_owned()
}

fn codec_label(codec: &str) -> String {
    match codec {
        "h264" => "H.264".into(),
        "hevc" => "HEVC".into(),
        "av1" => "AV1".into(),
        "vp9" => "VP9".into(),
        "vp8" => "VP8".into(),
        "mpeg4" => "MPEG-4".into(),
        "mpeg2video" => "MPEG-2".into(),
        "mpeg1video" => "MPEG-1".into(),
        "prores" => "ProRes".into(),
        "dnxhd" => "DNxHD".into(),
        "theora" => "Theora".into(),
        "mjpeg" => "MJPEG".into(),
        "wmv3" | "wmv2" | "wmv1" => "WMV".into(),
        "vc1" => "VC-1".into(),
        "ffv1" => "FFV1".into(),
        other => other.to_ascii_uppercase(),
    }
}

fn audio_codec_label(codec: &str) -> String {
    match codec {
        "aac" => "AAC".into(),
        "ac3" => "AC-3".into(),
        "eac3" => "E-AC-3".into(),
        "dts" => "DTS".into(),
        "truehd" => "TrueHD".into(),
        "opus" => "Opus".into(),
        "vorbis" => "Vorbis".into(),
        "flac" => "FLAC".into(),
        "mp3" => "MP3".into(),
        "mp2" => "MP2".into(),
        "alac" => "ALAC".into(),
        "wmav2" | "wmav1" | "wmapro" => "WMA".into(),
        other if other.starts_with("pcm_") => "PCM".into(),
        other => other.to_ascii_uppercase(),
    }
}

fn channels_label(channels: Option<u32>, layout: Option<&str>) -> Option<String> {
    if let Some(layout) = layout {
        // "5.1(side)" and "7.1(wide)" carry the speaker count before the variant.
        let compact = layout.split('(').next().unwrap_or(layout).trim();
        if matches!(compact, "mono" | "stereo") {
            return Some(crate::i18n::tr(if compact == "mono" {
                "Mono"
            } else {
                "Stereo"
            }));
        }
        if compact
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
            && !compact.is_empty()
        {
            return Some(compact.to_owned());
        }
    }
    Some(match channels? {
        1 => crate::i18n::tr("Mono"),
        2 => crate::i18n::tr("Stereo"),
        6 => "5.1".into(),
        8 => "7.1".into(),
        other => rust_i18n::t!("%{channels} ch", channels = other).into_owned(),
    })
}

fn audio_label(codec: Option<&str>, channels: Option<u32>, layout: Option<&str>) -> Option<String> {
    let channels = channels_label(channels, layout);
    match (codec.map(audio_codec_label), channels) {
        (Some(codec), Some(channels)) => Some(format!("{codec} {channels}")),
        (Some(codec), None) => Some(codec),
        (None, Some(channels)) => Some(channels),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests;
