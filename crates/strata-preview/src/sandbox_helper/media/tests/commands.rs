// SPDX-License-Identifier: MIT

use super::*;

const BOUNDS: [&str; 3] = [
    "-max_pixels 50000000",
    "-max_alloc 536870912",
    "-filter_threads 1 -filter_complex_threads 1",
];

const ENCODERS: [&str; 3] = ["h264", "libvpx", " copy"];

fn assert_backend_order(devices: &[PathBuf]) {
    assert_eq!(
        backends(devices, MediaPreviewBackend::Automatic),
        vec![
            Backend::VaApi(devices[2].clone()),
            Backend::VaApi(devices[1].clone()),
            Backend::Vulkan(0),
            Backend::Vulkan(1),
            Backend::Software
        ]
    );
    for policy in [MediaPreviewBackend::VaApi, MediaPreviewBackend::Vulkan] {
        let choices = backends(&devices[1..2], policy);
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[1], Backend::Software);
    }
}

fn assert_every_backend_decodes_within_bounds(devices: &[PathBuf], ordinary: &Input) {
    for backend in backends(devices, MediaPreviewBackend::Automatic) {
        let args = command_args(Path::new("/input"), ordinary, &backend, Track::Video).join(" ");
        assert_eq!(present(&args, &ENCODERS), NO_TERMS);
        assert_eq!(
            args.contains("--as=2147483648"),
            backend == Backend::Software
        );
        assert_eq!(missing(&args, &BOUNDS), NO_TERMS);
    }
}

fn assert_decoder_command_is_limited() {
    let decoder = ffmpeg_command(&Backend::Software);
    let args = decoder
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(args.starts_with("--core=0 --fsize=536870912 --as=2147483648"));
    assert_eq!(missing(&args, &[BOUNDS[0], BOUNDS[2]]), NO_TERMS);
}

fn flag_positions(args: &[String], flags: &[&str]) -> Vec<usize> {
    flags
        .iter()
        .map(|flag| {
            args.iter()
                .position(|arg| arg == flag)
                .unwrap_or_else(|| panic!("{flag} in {args:?}"))
        })
        .collect()
}

fn assert_positive_seek_prerolls_before_the_input(ordinary: &Input) {
    let positive = software_args(ordinary, Track::Video);
    let args = positive.join(" ");
    assert_input_seek(&positive, "3595.000000");
    assert!(flag_positions(&positive, &["-ss", "-noaccurate_seek", "-i"]).is_sorted());
    assert_eq!(
        missing(&args, &["-t 5.000000", "-frames:v 150", "-c:v rawvideo"]),
        NO_TERMS
    );
    assert_eq!(present(&args, &ENCODERS), NO_TERMS);
    let audio = software_args(ordinary, Track::Audio).join(" ");
    assert_eq!(
        (audio.contains("-c:a pcm_s16le"), audio.contains("-hwaccel")),
        (true, false)
    );
}

fn assert_seek_placement_follows_source_kind() {
    assert_output_seek(
        &software_args(&sample_input(107850, true), Track::Video),
        "3595.000000",
    );
    assert_omits_seek(&software_args(&sample_input(0, false), Track::Video));
    let mut covered = sample_input(30, false);
    covered.cover = true;
    assert_omits_seek(&software_args(&covered, Track::Video));
    assert_input_seek(&software_args(&covered, Track::Audio), "1.000000");
}

#[test]
fn hardware_order_and_commands_decode_only_and_bound_all_outputs() {
    let devices = vec![
        "/dev/nvidia0".into(),
        "/dev/dri/renderD129".into(),
        "/dev/dri/renderD128".into(),
    ];
    assert_backend_order(&devices);
    let ordinary = sample_input(107850, false);
    assert_every_backend_decodes_within_bounds(&devices, &ordinary);
    assert_decoder_command_is_limited();
    assert_positive_seek_prerolls_before_the_input(&ordinary);
    assert_seek_placement_follows_source_kind();
}

fn assert_invalid_sizes_and_streams_are_rejected(size: MediaPreviewSize) {
    for value in ["", "520", "520x", "x800", "520x800x2", "2147483648x800"] {
        assert!(media_preview_size(value).is_err());
    }
    for value in [
        br#"{}"#.as_slice(),
        br#"{"streams":[]}"#,
        br#"{"streams":[{"index":0,"codec_type":"video","width":4294967295,"height":4294967295}]}"#,
    ] {
        assert!(metadata(value, size, 0).is_err());
    }
    for duration in ["143165577", "18446744073709551615"] {
        let value = serde_json::json!({
            "streams": [{"index": 0, "codec_type": "audio"}],
            "format": {"duration": duration},
        });
        assert!(metadata(&serde_json::to_vec(&value).expect("metadata"), size, 0).is_err());
    }
}

fn audio_with_video_stream(width: u32, height: u32, attached: bool) -> Vec<u8> {
    let value = serde_json::json!({
        "streams": [
            {"index": 0, "codec_type": "audio"},
            {"index": 1, "codec_type": "video", "width": width, "height": height,
             "disposition": {"attached_pic": u8::from(attached)}}
        ],
        "format": {"duration": "2"}
    });
    serde_json::to_vec(&value).expect("metadata")
}

fn assert_bad_video_dimensions_only_block_playback(
    size: MediaPreviewSize,
    bytes: &[u8],
    attached: bool,
) {
    assert!(metadata(bytes, size, 0).is_err());
    let audio = super::super::metadata(bytes, size, 0, ProbeMode::Audio)
        .expect("peaks ignore every video stream");
    assert_eq!(
        (
            audio.audio,
            audio.video,
            (audio.header.width, audio.header.height)
        ),
        (Some(0), None, (0, 0))
    );
    let playback = super::super::metadata(bytes, size, 0, ProbeMode::SkipArtwork);
    if attached {
        let playback = playback.expect("bad artwork cannot block audio");
        assert_eq!((playback.video, playback.header.audio), (None, true));
    } else {
        assert!(playback.is_err(), "real video dimensions remain validated");
    }
}

fn assert_unknown_durations_resume_and_stay_contained(size: MediaPreviewSize) {
    let unknown = metadata(
        br#"{"streams":[{"index":0,"codec_type":"audio"}]}"#,
        size,
        960,
    )
    .expect("unknown duration can resume beyond thirty seconds");
    assert_eq!(
        (
            unknown.header.start_tick,
            unknown.header.duration_us,
            unknown.raw_video
        ),
        (960, media::MAX_DURATION_US, false)
    );
    let unknown_container = metadata(
        br#"{"streams":[{"index":0,"codec_type":"video","width":64,"height":48}],"format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2"}}"#,
        size,
        30,
    )
    .expect("missing duration does not make a container raw");
    assert_eq!(
        (
            unknown_container.raw_video,
            unknown_container.header.duration_us
        ),
        (false, media::MAX_DURATION_US)
    );
    assert_input_seek(&software_args(&unknown_container, Track::Video), "1.000000");
}

fn assert_raw_streams_seek_on_the_output(size: MediaPreviewSize) {
    let raw = metadata(
        br#"{"streams":[{"index":0,"codec_type":"video","width":64,"height":48}],"format":{"duration":"2.0","format_name":"h264"}}"#,
        size,
        30,
    )
    .expect("raw h264 metadata");
    assert_eq!((raw.raw_video, raw.header.duration_us), (true, 2_000_000));
    assert_output_seek(&software_args(&raw, Track::Video), "1.000000");
    let hevc = metadata(
        br#"{"streams":[{"index":0,"codec_type":"video","width":64,"height":48}],"format":{"format_name":"hevc"}}"#,
        size,
        0,
    )
    .expect("raw hevc metadata");
    assert_eq!(
        (hevc.raw_video, hevc.header.duration_us),
        (true, media::MAX_DURATION_US)
    );
    assert_omits_seek(&software_args(&hevc, Track::Video));
}

#[test]
fn metadata_and_size_parsing_fail_closed_on_bad_sources_and_protocol_values() {
    let size = MediaPreviewSize::new(520, 800);
    assert_invalid_sizes_and_streams_are_rejected(size);
    for (width, height) in [(0, 0), (10_000, 6_000)] {
        for attached in [true, false] {
            let bytes = audio_with_video_stream(width, height, attached);
            assert_bad_video_dimensions_only_block_playback(size, &bytes, attached);
        }
    }
    assert_unknown_durations_resume_and_stay_contained(size);
    assert_raw_streams_seek_on_the_output(size);
    assert!(probe(Path::new("/nonexistent-strata-media"), size, 0).is_err());
}
