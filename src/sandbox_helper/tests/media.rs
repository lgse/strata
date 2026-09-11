// SPDX-License-Identifier: MIT

use std::{fs, path::Path, process::Command, time::Duration};

use crate::{
    sandbox::{MAX_OUTPUT_BYTES, MediaPreviewBackend},
    sandbox_helper::{
        MediaBackend, bounded_output_with_timeout, input_has_video, media_backends, media_command,
        media_preview_size, render_media_preview, run_media_backends,
    },
    services::MediaPreviewSize,
};

#[test]
fn failed_or_unavailable_software_h264_falls_back_to_vp8() {
    let backends = media_backends(&[], MediaPreviewBackend::Software);
    for h264_result in [Err(()), Ok(None)] {
        let mut attempts = Vec::new();
        let result = run_media_backends(&backends, |backend| {
            attempts.push(backend.clone());
            match backend {
                MediaBackend::SoftwareH264 => h264_result,
                MediaBackend::SoftwareVp8 => Ok(Some("webm")),
                _ => panic!("software previews must not attempt hardware"),
            }
        });
        assert_eq!(result, Ok("webm"));
        assert_eq!(attempts, backends);
    }
}

#[test]
fn media_size_protocol_is_bounded_and_rejects_malformed_dimensions() {
    assert_eq!(
        media_preview_size("520x800").expect("pane dimensions"),
        MediaPreviewSize::new(520, 800)
    );
    assert_eq!(
        media_preview_size("0").expect("legacy full-size request"),
        MediaPreviewSize::new(1280, 1280)
    );
    assert_eq!(
        media_preview_size("99999x-1").expect("clamped dimensions"),
        MediaPreviewSize::new(1280, 16)
    );
    for value in ["", "520", "520x", "x800", "520x800x2", "2147483648x800"] {
        assert!(media_preview_size(value).is_err(), "{value}");
    }
}

fn successful_output(command: &mut Command) -> Vec<u8> {
    let output = bounded_output_with_timeout(command, MAX_OUTPUT_BYTES, Duration::from_secs(25))
        .expect("FFmpeg and ffprobe must be installed for media regression tests")
        .expect("media command must finish within its deadline");
    assert!(output.status.success(), "media command failed: {command:?}");
    output.stdout
}

fn fixture(path: &Path, size: &str, fps: i32, duration: i32, audio: bool) {
    let mut command = Command::new("ffmpeg");
    command
        .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "testsrc2=size={size}:rate={fps}:duration={duration}"
        ));
    if audio {
        command.args(["-f", "lavfi", "-i"]).arg(format!(
            "sine=frequency=440:sample_rate=48000:duration={duration}"
        ));
    }
    command
        .args(["-c:v", "ffv1", "-threads", "1", "-c:a", "pcm_s16le"])
        .arg(path);
    successful_output(&mut command);
}

fn probe(path: &Path) -> serde_json::Value {
    let output = successful_output(
        Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_streams",
                "-show_format",
                "-of",
                "json",
            ])
            .arg(path),
    );
    serde_json::from_slice(&output).expect("ffprobe JSON")
}

fn preview(
    input: &Path,
    output: &Path,
    backend: &MediaBackend,
    size: MediaPreviewSize,
) -> serde_json::Value {
    let data = successful_output(&mut media_command(backend, input, size, true));
    assert!(!data.is_empty());
    assert!(data.len() as u64 <= MAX_OUTPUT_BYTES);
    fs::write(output, data).expect("normalized media");
    probe(output)
}

#[test]
fn audio_only_inputs_normalize_to_opus_for_every_backend_policy() {
    let directory = tempfile::tempdir().expect("audio fixtures");
    let input = directory.path().join("tone.ogg");
    let output = directory.path().join("preview.webm");
    successful_output(
        Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
            .arg("sine=frequency=660:duration=1")
            .arg(&input),
    );
    assert!(!input_has_video(&input));
    for policy in [
        MediaPreviewBackend::Automatic,
        MediaPreviewBackend::Software,
        MediaPreviewBackend::VaApi,
        MediaPreviewBackend::Vulkan,
    ] {
        let data = render_media_preview(&input, policy, MediaPreviewSize::new(520, 800))
            .expect("audio-only normalization");
        assert!(data.starts_with(b"\x1a\x45\xdf\xa3"));
        fs::write(&output, data).expect("normalized audio");
        let metadata = probe(&output);
        let streams = metadata["streams"].as_array().expect("audio streams");
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0]["codec_type"], "audio");
        assert_eq!(streams[0]["codec_name"], "opus");
    }
}

#[test]
fn video_and_unreadable_inputs_keep_the_video_pipeline() {
    let directory = tempfile::tempdir().expect("video fixtures");
    let input = directory.path().join("clip.mkv");
    fixture(&input, "64x48", 1, 1, true);
    assert!(input_has_video(&input));
    assert!(input_has_video(&directory.path().join("missing.mkv")));
    let output = directory.path().join("preview.mp4");
    fs::write(
        &output,
        render_media_preview(
            &input,
            MediaPreviewBackend::Software,
            MediaPreviewSize::new(520, 800),
        )
        .expect("video normalization"),
    )
    .expect("normalized video");
    let metadata = probe(&output);
    let streams = metadata["streams"].as_array().expect("video streams");
    assert!(streams.iter().any(|stream| stream["codec_type"] == "video"));
    assert!(streams.iter().any(|stream| stream["codec_type"] == "audio"));
}

#[test]
fn software_previews_fit_the_pane_without_enlarging_small_sources() {
    let directory = tempfile::tempdir().expect("media fixtures");
    let input = directory.path().join("input.mkv");
    let output = directory.path().join("preview.media");
    for (source, size, expected) in [
        ("320x180", MediaPreviewSize::new(520, 800), (320, 180)),
        ("1920x1080", MediaPreviewSize::new(800, 640), (800, 450)),
        ("360x640", MediaPreviewSize::new(320, 480), (270, 480)),
    ] {
        if input.exists() {
            fs::remove_file(&input).expect("previous fixture");
        }
        fixture(&input, source, 60, 1, true);
        for (backend, codec, audio_codec) in [
            (MediaBackend::SoftwareH264, "h264", "aac"),
            (MediaBackend::SoftwareVp8, "vp8", "opus"),
        ] {
            let metadata = preview(&input, &output, &backend, size);
            let streams = metadata["streams"].as_array().expect("preview streams");
            let video = streams
                .iter()
                .find(|stream| stream["codec_type"] == "video")
                .expect("video stream");
            let audio = streams
                .iter()
                .find(|stream| stream["codec_type"] == "audio")
                .expect("audio stream");
            assert_eq!(video["codec_name"], codec);
            assert_eq!(audio["codec_name"], audio_codec);
            assert_eq!(video["width"], expected.0);
            assert_eq!(video["height"], expected.1);
            assert_eq!(video["r_frame_rate"], "30/1");
        }
    }
}

#[test]
fn hour_long_sources_still_produce_only_thirty_second_previews() {
    let directory = tempfile::tempdir().expect("long media fixture");
    let input = directory.path().join("hour.mkv");
    let output = directory.path().join("preview.media");
    fixture(&input, "64x48", 1, 3600, false);
    let source = probe(&input);
    let duration = |metadata: &serde_json::Value| {
        metadata["format"]["duration"]
            .as_str()
            .expect("duration")
            .parse::<f64>()
            .expect("numeric duration")
    };
    assert!(duration(&source) >= 3600.0);
    for backend in [MediaBackend::SoftwareH264, MediaBackend::SoftwareVp8] {
        let metadata = preview(&input, &output, &backend, MediaPreviewSize::new(520, 800));
        assert!((duration(&metadata) - 30.0).abs() < 0.1);
        assert_eq!(metadata["streams"][0]["width"], 64);
        assert_eq!(metadata["streams"][0]["height"], 48);
    }
}
