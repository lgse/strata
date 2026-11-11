//! A video's badges from its probe: Strata's `src/ui/preview/video/badges.rs`
//! (lgse/strata `upstream/main` e9a8993), rule for rule. The words are the host's
//! ([`Labels`]).

use super::labels::Labels;

/// What the probe found (Strata's `MediaMetadata`, the fields the badges read) plus the
/// sidecar subtitle files the host counted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoDetails {
    /// Width and height in pixels.
    pub dimensions: Option<(u32, u32)>,
    /// FFmpeg's `color_transfer` (`smpte2084`, `arib-std-b67`).
    pub color_transfer: Option<String>,
    /// FFmpeg's `pix_fmt` (`yuv420p10le`).
    pub pixel_format: Option<String>,
    pub frame_rate: Option<f64>,
    /// FFmpeg's codec names (`h264`, `aac`).
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub channels: Option<u32>,
    /// FFmpeg's `channel_layout` (`5.1(side)`).
    pub channel_layout: Option<String>,
    /// Embedded subtitle tracks.
    pub subtitle_tracks: usize,
    /// `srt`, `vtt`, `ass`, `ssa` and `sub` files named after the video.
    pub sidecar_captions: usize,
}

/// What a badge says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BadgeKind {
    Resolution,
    Hdr,
    BitDepth,
    FrameRate,
    Codec,
    Audio,
    Captions,
}

/// One badge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    pub kind: BadgeKind,
    pub label: String,
}

fn badge(kind: BadgeKind, label: impl Into<String>) -> Badge {
    Badge {
        kind,
        label: label.into(),
    }
}

/// The badges, in Strata's order.
pub fn badges(d: &VideoDetails, labels: &Labels) -> Vec<Badge> {
    let mut out = Vec::new();
    if let Some((w, h)) = d.dimensions {
        out.push(badge(BadgeKind::Resolution, resolution_class(w, h)));
    }
    if let Some(hdr) = hdr_format(d.color_transfer.as_deref()) {
        out.push(badge(BadgeKind::Hdr, hdr));
    }
    if let Some(depth) = bit_depth(d.pixel_format.as_deref()) {
        out.push(badge(BadgeKind::BitDepth, depth));
    }
    if let Some(rate) = d.frame_rate {
        out.push(badge(
            BadgeKind::FrameRate,
            labels
                .fps
                .replace("{rate}", &(rate.round() as u64).to_string()),
        ));
    }
    if let Some(codec) = d.video_codec.as_deref() {
        out.push(badge(BadgeKind::Codec, codec_label(codec)));
    }
    if let Some(audio) = audio_label(
        d.audio_codec.as_deref(),
        d.channels,
        d.channel_layout.as_deref(),
        labels,
    ) {
        out.push(badge(BadgeKind::Audio, audio));
    }
    let captions = d.subtitle_tracks + d.sidecar_captions;
    if captions > 0 {
        out.push(badge(
            BadgeKind::Captions,
            if captions == 1 {
                labels.captions.clone()
            } else {
                labels
                    .captions_many
                    .replace("{count}", &captions.to_string())
            },
        ));
    }
    out
}

/// By the long edge, so portrait video reads as landscape does.
fn resolution_class(w: u32, h: u32) -> &'static str {
    match w.max(h) {
        7600.. => "8K",
        3800.. => "4K",
        2000.. => "2K",
        1900.. => "1080p",
        1260.. => "720p",
        _ => "SD",
    }
}

/// `MediaMetadata::hdr_format`.
fn hdr_format(transfer: Option<&str>) -> Option<&'static str> {
    match transfer? {
        "smpte2084" => Some("HDR10"),
        "arib-std-b67" => Some("HLG"),
        _ => None,
    }
}

fn bit_depth(format: Option<&str>) -> Option<&'static str> {
    let f = format?;
    let has = |depth: &str| {
        f.contains(&format!("p{depth}"))
            || f.contains(&format!("{depth}le"))
            || f.contains(&format!("{depth}be"))
    };
    if has("16") {
        Some("16-bit")
    } else if has("12") {
        Some("12-bit")
    } else if has("10") {
        Some("10-bit")
    } else {
        None
    }
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

fn channels_label(channels: Option<u32>, layout: Option<&str>, labels: &Labels) -> Option<String> {
    if let Some(layout) = layout {
        // "5.1(side)" and "7.1(wide)" carry the speaker count before the variant.
        let compact = layout.split('(').next().unwrap_or(layout).trim();
        if matches!(compact, "mono" | "stereo") {
            return Some(if compact == "mono" {
                labels.mono.clone()
            } else {
                labels.stereo.clone()
            });
        }
        if !compact.is_empty() && compact.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return Some(compact.to_owned());
        }
    }
    Some(match channels? {
        1 => labels.mono.clone(),
        2 => labels.stereo.clone(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        other => labels.channels.replace("{channels}", &other.to_string()),
    })
}

fn audio_label(
    codec: Option<&str>,
    channels: Option<u32>,
    layout: Option<&str>,
    labels: &Labels,
) -> Option<String> {
    match (
        codec.map(audio_codec_label),
        channels_label(channels, layout, labels),
    ) {
        (Some(c), Some(ch)) => Some(format!("{c} {ch}")),
        (Some(c), None) => Some(c),
        (None, Some(ch)) => Some(ch),
        (None, None) => None,
    }
}
