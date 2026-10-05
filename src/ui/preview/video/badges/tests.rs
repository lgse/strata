// SPDX-License-Identifier: MIT

use super::*;
use crate::sandbox::metadata::SubtitleTrack;

fn labels(metadata: &MediaMetadata, sidecars: usize) -> Vec<String> {
    badges(metadata, sidecars)
        .into_iter()
        .map(|badge| badge.label)
        .collect()
}

#[test]
fn badges_describe_resolution_hdr_depth_rate_codec_audio_and_captions() {
    let hdr = MediaMetadata {
        dimensions: Some((3840, 2160)),
        video_codec: Some("hevc".into()),
        pixel_format: Some("yuv420p10le".into()),
        color_transfer: Some("smpte2084".into()),
        frame_rate: Some(23.976),
        audio_codec: Some("eac3".into()),
        channels: Some(6),
        channel_layout: Some("5.1(side)".into()),
        subtitle_tracks: vec![SubtitleTrack { language: None }; 2],
        ..Default::default()
    };
    assert_eq!(
        labels(&hdr, 1),
        [
            "4K",
            "HDR10",
            "10-bit",
            "24 fps",
            "HEVC",
            "E-AC-3 5.1",
            "CC ×3"
        ]
    );
    let phone = MediaMetadata {
        dimensions: Some((1080, 1920)),
        video_codec: Some("h264".into()),
        pixel_format: Some("yuv420p".into()),
        color_transfer: Some("bt709".into()),
        frame_rate: Some(29.97),
        audio_codec: Some("aac".into()),
        channels: Some(2),
        channel_layout: Some("stereo".into()),
        ..Default::default()
    };
    assert_eq!(
        labels(&phone, 0),
        ["1080p", "30 fps", "H.264", "AAC Stereo"],
        "portrait video classes by its long edge"
    );
    let bare = MediaMetadata {
        dimensions: Some((640, 480)),
        channels: Some(1),
        ..Default::default()
    };
    assert_eq!(labels(&bare, 1), ["SD", "Mono", "CC"]);
    assert!(labels(&MediaMetadata::default(), 0).is_empty());
}

#[test]
fn resolution_classes_follow_common_names() {
    for (size, class) in [
        ((7680, 4320), "8K"),
        ((4096, 2160), "4K"),
        ((2560, 1440), "2K"),
        ((2048, 1080), "2K"),
        ((1920, 1080), "1080p"),
        ((1280, 720), "720p"),
        ((854, 480), "SD"),
    ] {
        assert_eq!(resolution_class(size.0, size.1), class, "{size:?}");
    }
    let hlg = MediaMetadata {
        color_transfer: Some("arib-std-b67".into()),
        ..Default::default()
    };
    assert_eq!(labels(&hlg, 0), ["HLG"]);
    assert_eq!(bit_depth(Some("yuv422p12le")), Some("12-bit"));
    assert_eq!(bit_depth(Some("rgb48le")), None);
    assert_eq!(codec_label("av1"), "AV1");
    assert_eq!(codec_label("cinepak"), "CINEPAK");
    assert_eq!(
        audio_label(Some("pcm_s16le"), Some(8), Some("7.1(wide)")).as_deref(),
        Some("PCM 7.1")
    );
    assert_eq!(audio_label(None, Some(3), None).as_deref(), Some("3 ch"));
}
