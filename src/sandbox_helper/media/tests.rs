// SPDX-License-Identifier: MIT

use super::*;
use crate::media::{Decoder, Packet};
use std::{io::Cursor, os::unix::net::UnixStream, thread};

fn success(command: &mut Command) -> Vec<u8> {
    let output = bounded_output_with_timeout(command, 32 * 1024 * 1024, Duration::from_secs(30))
        .expect("FFmpeg tools are required")
        .expect("fixture deadline");
    assert!(output.status.success(), "fixture failed: {command:?}");
    output.stdout
}

fn fixture(path: &Path, size: &str, rate: u32, duration: u32, audio: bool) {
    let mut command = Command::new("ffmpeg");
    command
        .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "testsrc2=size={size}:rate={rate}:duration={duration}"
        ));
    if audio {
        command.args(["-f", "lavfi", "-i"]).arg(format!(
            "sine=frequency=660:sample_rate=48000:duration={duration}"
        ));
    }
    command
        .args(["-c:v", "ffv1", "-threads", "1", "-c:a", "pcm_s16le"])
        .arg(path);
    success(&mut command);
}

fn encoded_video(path: &Path, format: &str, encoder: &str) {
    let mut command = Command::new("ffmpeg");
    command.args([
        "-nostdin",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=64x48:rate=30:duration=2",
        "-an",
        "-c:v",
        encoder,
        "-pix_fmt",
        "yuv420p",
        "-preset",
        "ultrafast",
        "-threads",
        "1",
    ]);
    if encoder == "libx264" {
        command.args(["-x264-params", "threads=1:lookahead-threads=1"]);
    } else if encoder == "libx265" {
        command.args(["-x265-params", "pools=none:frame-threads=1"]);
    }
    command.args(["-f", format]).arg(path);
    success(&mut command);
}

fn decoded(input: &Path, size: &str, start: u32) -> (Header, Vec<Frame>, u64) {
    let mut bytes = Vec::new();
    stream(
        input,
        size,
        MediaPreviewBackend::Software,
        start,
        &mut bytes,
    )
    .expect("raw media stream");
    let mut reader = Cursor::new(bytes);
    let header = Header::read(&mut reader, media_preview_size(size).expect("size"), start)
        .expect("validated header");
    let mut decoder = Decoder::new(header);
    let mut frames = Vec::new();
    loop {
        match decoder.read(&mut reader).expect("validated packet") {
            Packet::Frame(frame) => frames.push(frame),
            Packet::End(end) => {
                assert_eq!(reader.position() as usize, reader.get_ref().len());
                return (header, frames, end);
            }
        }
    }
}

#[test]
fn raw_software_video_fits_landscape_portrait_hidpi_and_does_not_enlarge() {
    let directory = tempfile::tempdir().expect("fixtures");
    for (index, (source, size, expected)) in [
        ("320x180", "520x800", (320, 180)),
        ("1920x1080", "800x640", (800, 450)),
        ("360x640", "320x480", (270, 480)),
        ("1920x1080", "1040x1280", (1040, 585)),
    ]
    .into_iter()
    .enumerate()
    {
        let input = directory.path().join(format!("{index}.mkv"));
        fixture(&input, source, 60, 1, true);
        let (header, frames, end) = decoded(&input, size, 0);
        assert_eq!((header.width, header.height), expected);
        assert!(header.audio);
        assert_eq!(frames.len(), 30);
        assert_eq!(end, 1_000_000);
        assert_ne!(frames[0].pixels, frames[20].pixels);
        assert!(
            frames
                .iter()
                .any(|frame| frame.samples.iter().any(|sample| *sample != 0))
        );
    }
}

#[test]
fn full_sources_and_hour_long_seeks_reach_the_original_file_end() {
    let directory = tempfile::tempdir().expect("fixture");
    for (seconds, audio, starts) in [
        (35, true, [0, 900, 1049]),
        (3600, false, [107850, 107970, 107999]),
    ] {
        let input = directory.path().join(format!("{seconds}.mkv"));
        fixture(&input, "64x48", 1, seconds, audio);
        for start in starts {
            let (header, frames, end) = decoded(&input, "520x800", start);
            assert_eq!(header.duration_us, u64::from(seconds) * 1_000_000);
            assert_eq!(end, header.duration_us);
            assert_eq!(frames.len(), (seconds * 30 - start) as usize);
            assert_eq!(frames[0].tick, start);
            assert_eq!(header.audio, audio);
            assert!(frames.iter().all(|frame| frame.samples.is_empty() != audio));
            if audio {
                assert!(
                    frames
                        .iter()
                        .any(|frame| frame.samples.iter().any(|sample| *sample != 0))
                );
            }
        }
        assert!(
            stream(
                &input,
                "520x800",
                MediaPreviewBackend::Software,
                seconds * 30,
                &mut Vec::new()
            )
            .is_err()
        );
    }
}

#[test]
fn audio_only_and_attached_cover_art_do_not_require_a_hardware_video_decoder() {
    let directory = tempfile::tempdir().expect("fixtures");
    let audio = directory.path().join("tone.flac");
    success(
        Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=660:duration=1",
            ])
            .arg(&audio),
    );
    for policy in [
        MediaPreviewBackend::Automatic,
        MediaPreviewBackend::VaApi,
        MediaPreviewBackend::Vulkan,
        MediaPreviewBackend::Software,
    ] {
        let mut bytes = Vec::new();
        stream(&audio, "520x800", policy, 0, &mut bytes).expect("audio under every policy");
        let h = Header::read(&mut Cursor::new(bytes), MediaPreviewSize::new(520, 800), 0)
            .expect("audio header");
        assert!(h.audio);
        assert_eq!(h.width, 0);
    }
    let aac = directory.path().join("tone.m4a");
    success(
        Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(&audio)
            .args(["-c:a", "aac"])
            .arg(&aac),
    );
    let (_, frames, _) = decoded(&aac, "520x800", 0);
    assert!(
        frames[0].samples[..1024].iter().any(|sample| *sample != 0),
        "zero-position playback must preserve the opening AAC samples"
    );
    let cover = directory.path().join("cover.jpg");
    success(
        Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:size=64x48",
                "-frames:v",
                "1",
                "-threads",
                "1",
            ])
            .arg(&cover),
    );
    let attached = directory.path().join("cover.flac");
    success(
        Command::new("ffmpeg")
            .arg("-v")
            .arg("error")
            .arg("-i")
            .arg(&audio)
            .arg("-i")
            .arg(&cover)
            .args([
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c",
                "copy",
                "-disposition:v",
                "attached_pic",
            ])
            .arg(&attached),
    );
    let info = probe(&attached, MediaPreviewSize::new(520, 800), 0).expect("cover metadata");
    assert!(info.cover);
    let mp3 = directory.path().join("cover.mp3");
    success(
        Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&audio)
            .arg("-i")
            .arg(&cover)
            .args([
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:a",
                "libmp3lame",
                "-c:v",
                "copy",
                "-disposition:v",
                "attached_pic",
            ])
            .arg(&mp3),
    );
    for attached in [&attached, &mp3] {
        let (header, frames, _) = decoded(attached, "520x800", 0);
        assert!(header.audio);
        assert_eq!((header.width, header.height), (64, 48));
        assert_eq!(frames.len(), 30);
        let pixel = &frames[0].pixels[..4];
        assert!(
            pixel[2] > pixel[0] && pixel[2] > pixel[1],
            "cover frame: {pixel:?}"
        );
        let (_, sought, end) = decoded(attached, "520x800", 15);
        assert_eq!(sought.len(), 15);
        assert_eq!(end, 1_000_000);
        assert_eq!(sought[0].pixels, frames[0].pixels);
        assert!(
            sought
                .iter()
                .any(|frame| frame.samples.iter().any(|sample| *sample != 0)),
            "cover-art seeking must retain audio: {attached:?}"
        );
    }
}

#[test]
fn first_frames_arrive_before_completion_and_closed_consumers_cancel_backpressure() {
    let directory = tempfile::tempdir().expect("fixture");
    let input = directory.path().join("clip.mkv");
    fixture(&input, "160x90", 30, 35, true);
    let (read, mut write) = UnixStream::pair().expect("private frame pipe");
    let worker = thread::spawn(move || {
        stream(
            &input,
            "520x800",
            MediaPreviewBackend::Software,
            0,
            &mut write,
        )
    });
    let cancellation = Cancellation::default();
    let mut reader = TimedReader {
        fd: &read,
        deadline: Instant::now() + Duration::from_secs(10),
        cancellation: &cancellation,
    };
    let header =
        Header::read(&mut reader, MediaPreviewSize::new(520, 800), 0).expect("streaming header");
    let mut decoder = Decoder::new(header);
    let Packet::Frame(first) = decoder.read(&mut reader).expect("first frame") else {
        panic!("no first frame");
    };
    let Packet::Frame(second) = decoder.read(&mut reader).expect("moving frame") else {
        panic!("no moving frame");
    };
    assert_ne!(first.pixels, second.pixels);
    thread::sleep(Duration::from_millis(150));
    assert!(
        !worker.is_finished(),
        "bounded pipe must prevent whole-clip buffering"
    );
    drop(read);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !worker.is_finished() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(worker.join().expect("worker").is_err());
}

#[test]
fn hardware_order_and_commands_decode_only_and_bound_all_outputs() {
    let devices = vec![
        "/dev/nvidia0".into(),
        "/dev/dri/renderD129".into(),
        "/dev/dri/renderD128".into(),
    ];
    assert_eq!(
        backends(&devices, MediaPreviewBackend::Automatic),
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
    let ordinary = sample_input(107850, false);
    for backend in backends(&devices, MediaPreviewBackend::Automatic) {
        let args = command_args(Path::new("/input"), &ordinary, &backend, Track::Video).join(" ");
        assert!(!args.contains("h264"));
        assert!(!args.contains("libvpx"));
        assert!(!args.contains(" copy"));
    }
    let positive = command_args(
        Path::new("/input"),
        &ordinary,
        &Backend::Software,
        Track::Video,
    );
    let args = positive.join(" ");
    assert_input_seek(&positive, "3595.000000");
    let seek = positive.iter().position(|arg| arg == "-ss").expect("seek");
    let accurate = positive
        .iter()
        .position(|arg| arg == "-noaccurate_seek")
        .expect("preroll seek");
    let input = positive.iter().position(|arg| arg == "-i").expect("input");
    assert!(seek < accurate && accurate < input);
    assert!(args.contains("-t 5.000000"));
    assert!(args.contains("-frames:v 150"));
    assert!(args.contains("-c:v rawvideo"));
    assert!(!args.contains("h264"));
    assert!(!args.contains("libvpx"));
    assert!(!args.contains(" copy"));
    let audio = command_args(
        Path::new("/input"),
        &ordinary,
        &Backend::Software,
        Track::Audio,
    )
    .join(" ");
    assert!(audio.contains("-c:a pcm_s16le"));
    assert!(!audio.contains("-hwaccel"));
    assert_output_seek(
        &command_args(
            Path::new("/input"),
            &sample_input(107850, true),
            &Backend::Software,
            Track::Video,
        ),
        "3595.000000",
    );
    assert_omits_seek(&command_args(
        Path::new("/input"),
        &sample_input(0, false),
        &Backend::Software,
        Track::Video,
    ));
    let mut covered = sample_input(30, false);
    covered.cover = true;
    assert_omits_seek(&command_args(
        Path::new("/input"),
        &covered,
        &Backend::Software,
        Track::Video,
    ));
    assert_input_seek(
        &command_args(
            Path::new("/input"),
            &covered,
            &Backend::Software,
            Track::Audio,
        ),
        "1.000000",
    );
}

fn assert_elementary_playback(path: &Path) {
    let size = MediaPreviewSize::new(520, 800);
    let at_zero = probe(path, size, 0).expect("raw probe");
    assert!(at_zero.raw_video);
    assert_eq!(at_zero.header.duration_us, media::MAX_DURATION_US);
    assert_omits_seek(&command_args(
        path,
        &at_zero,
        &Backend::Software,
        Track::Video,
    ));
    let at_second = probe(path, size, 30).expect("raw probe at one second");
    assert!(at_second.raw_video);
    assert_output_seek(
        &command_args(path, &at_second, &Backend::Software, Track::Video),
        "1.000000",
    );

    let (header, frames, end) = decoded(path, "520x800", 0);
    assert_eq!(header.duration_us, media::MAX_DURATION_US);
    assert_eq!(frames.len(), 60);
    assert_eq!(end, 2_000_000);
    assert_ne!(frames[0].pixels, frames[30].pixels);
    for start in [30_u32, 20, 58] {
        let (sought_header, sought, sought_end) = decoded(path, "520x800", start);
        assert_eq!(sought_header.duration_us, header.duration_us);
        assert_eq!(sought_end, end);
        assert_eq!(sought.len(), frames.len() - start as usize);
        assert_ne!(sought[0].pixels, frames[0].pixels);
        for (offset, frame) in sought.iter().enumerate() {
            assert_eq!(frame.pixels, frames[start as usize + offset].pixels);
        }
    }
    let mut past = Vec::new();
    let started = Instant::now();
    assert!(
        stream(
            path,
            "520x800",
            MediaPreviewBackend::Software,
            90,
            &mut past
        )
        .is_err(),
        "a raw seek past the real end must fail"
    );
    assert!(
        past.is_empty(),
        "past-the-end seek must not fabricate frames"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "past-the-end raw seek hung"
    );
}

#[test]
fn raw_h264_and_hevc_preview_seek_and_report_the_real_end() {
    let directory = tempfile::tempdir().expect("raw video fixtures");
    let h264 = directory.path().join("clip.h264");
    encoded_video(&h264, "h264", "libx264");
    assert_elementary_playback(&h264);

    let renamed = directory.path().join("renamed-stream");
    std::fs::copy(&h264, &renamed).expect("rename raw stream");
    let renamed_info = probe(&renamed, MediaPreviewSize::new(520, 800), 20).expect("renamed probe");
    assert!(renamed_info.raw_video);
    assert_output_seek(
        &command_args(&renamed, &renamed_info, &Backend::Software, Track::Video),
        "0.666666",
    );

    let mp4 = directory.path().join("clip.mp4");
    encoded_video(&mp4, "mp4", "libx264");
    let mp4_info = probe(&mp4, MediaPreviewSize::new(520, 800), 30).expect("mp4 probe");
    assert!(!mp4_info.raw_video);
    assert_eq!(mp4_info.header.duration_us, 2_000_000);
    assert_input_seek(
        &command_args(&mp4, &mp4_info, &Backend::Software, Track::Video),
        "1.000000",
    );

    let hevc = directory.path().join("clip.hevc");
    encoded_video(&hevc, "hevc", "libx265");
    assert_elementary_playback(&hevc);
}

fn command_args(path: &Path, input: &Input, backend: &Backend, track: Track) -> Vec<String> {
    command(path, input, backend, track)
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn assert_input_seek(args: &[String], timestamp: &str) {
    let ss = args
        .iter()
        .position(|arg| arg == "-ss")
        .expect("input seek");
    let input = args
        .iter()
        .position(|arg| arg == "-i")
        .expect("decoder input");
    assert!(ss + 1 < input, "seek belongs before -i: {args:?}");
    assert_eq!(args[ss + 1], timestamp);
    assert_eq!(args.iter().filter(|arg| *arg == "-ss").count(), 1);
}

fn assert_output_seek(args: &[String], timestamp: &str) {
    let input = args
        .iter()
        .position(|arg| arg == "-i")
        .expect("decoder input");
    let ss = args
        .iter()
        .position(|arg| arg == "-ss")
        .expect("output seek");
    assert!(input < ss, "raw video seeks after -i: {args:?}");
    assert_eq!(args[ss + 1], timestamp);
    assert!(!args.iter().any(|arg| arg == "-noaccurate_seek"));
    assert_eq!(args.iter().filter(|arg| *arg == "-ss").count(), 1);
}

fn assert_omits_seek(args: &[String]) {
    assert!(
        !args
            .iter()
            .any(|arg| arg == "-ss" || arg == "-noaccurate_seek"),
        "{args:?}"
    );
}

fn sample_input(start_tick: u32, raw_video: bool) -> Input {
    Input {
        header: Header {
            width: 320,
            height: 180,
            audio: true,
            duration_us: 3_600_000_000,
            start_tick,
        },
        video: Some(0),
        audio: Some(1),
        cover: false,
        gif_period_us: None,
        raw_video,
    }
}

#[test]
fn variable_frame_rate_and_offset_audio_keep_their_original_timeline() {
    let directory = tempfile::tempdir().expect("fixtures");
    let input = directory.path().join("variable.mkv");
    success(
        Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x90:rate=60:duration=3",
                "-itsoffset",
                "0.3",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=660:sample_rate=48000:duration=2.7",
                "-vf",
                "select=if(lt(t\\,1)\\,1\\,not(mod(n\\,12)))",
                "-fps_mode",
                "vfr",
                "-c:v",
                "ffv1",
                "-threads",
                "1",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&input),
    );
    let (_, frames, end) = decoded(&input, "160x90", 0);
    assert_eq!(end, 3_000_000);
    assert_eq!(frames.len(), 90);
    assert!(
        frames[..8]
            .iter()
            .all(|frame| frame.samples.iter().all(|sample| *sample == 0))
    );
    assert!(frames[10].samples.iter().any(|sample| *sample != 0));
    assert!(
        frames[34].pixels == frames[35].pixels,
        "VFR frames must hold until their next presentation time"
    );
    let (_, sought, _) = decoded(&input, "160x90", 45);
    assert_eq!(sought[0].tick, 45);
    assert!(sought[0].samples.iter().any(|sample| *sample != 0));
    assert!(
        frames[44..=46]
            .iter()
            .any(|frame| frame.pixels == sought[0].pixels)
    );
}

#[test]
fn short_gifs_loop_inside_the_bounded_decode_generation_and_seek_by_phase() {
    let directory = tempfile::tempdir().expect("GIF fixture");
    let input = directory.path().join("loop.gif");
    success(
        Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x48:rate=10:duration=1",
                "-threads",
                "1",
            ])
            .arg(&input),
    );
    let (header, frames, end) = decoded(&input, "160x90", 0);
    assert_eq!(header.duration_us, 30_000_000);
    assert_eq!(frames.len(), 900);
    assert_eq!(end, 30_000_000);
    assert!(frames[0].pixels == frames[30].pixels, "GIF loop phase");
    assert!(frames[0].pixels != frames[15].pixels, "moving GIF frames");
    let (_, sought, _) = decoded(&input, "160x90", 765);
    assert!(
        frames[15].pixels == sought[0].pixels,
        "seek keeps the GIF phase"
    );
    let (_, loop_boundary, end) = decoded(&input, "160x90", 750);
    assert_eq!(loop_boundary.len(), 150);
    assert_eq!(end, 30_000_000);
    for (actual, expected) in loop_boundary.iter().zip(&frames[750..]) {
        assert_eq!(actual.pixels, expected.pixels);
    }
    let long = directory.path().join("long.gif");
    success(
        Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x48:rate=10:duration=35",
                "-threads",
                "1",
            ])
            .arg(&long),
    );
    let (header, frames, end) = decoded(&long, "160x90", 0);
    assert_eq!(header.duration_us, 35_000_000);
    assert_eq!(frames.len(), 1050);
    assert_eq!(end, 35_000_000);
    let (_, sought, _) = decoded(&long, "160x90", 960);
    assert_eq!(frames[960].pixels, sought[0].pixels);
}

#[test]
fn metadata_and_size_parsing_fail_closed_on_bad_sources_and_protocol_values() {
    for value in ["", "520", "520x", "x800", "520x800x2", "2147483648x800"] {
        assert!(media_preview_size(value).is_err());
    }
    let size = MediaPreviewSize::new(520, 800);
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
    let unknown = metadata(
        br#"{"streams":[{"index":0,"codec_type":"audio"}]}"#,
        size,
        960,
    )
    .expect("unknown duration can resume beyond thirty seconds");
    assert_eq!(unknown.header.start_tick, 960);
    assert_eq!(unknown.header.duration_us, media::MAX_DURATION_US);
    assert!(!unknown.raw_video);
    let unknown_container = metadata(
        br#"{"streams":[{"index":0,"codec_type":"video","width":64,"height":48}],"format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2"}}"#,
        size,
        30,
    )
    .expect("missing duration does not make a container raw");
    assert!(!unknown_container.raw_video);
    assert_eq!(unknown_container.header.duration_us, media::MAX_DURATION_US);
    assert_input_seek(
        &command_args(
            Path::new("/input"),
            &unknown_container,
            &Backend::Software,
            Track::Video,
        ),
        "1.000000",
    );
    let raw = metadata(
        br#"{"streams":[{"index":0,"codec_type":"video","width":64,"height":48}],"format":{"duration":"2.0","format_name":"h264"}}"#,
        size,
        30,
    )
    .expect("raw h264 metadata");
    assert!(raw.raw_video);
    assert_eq!(raw.header.duration_us, 2_000_000);
    assert_output_seek(
        &command_args(Path::new("/input"), &raw, &Backend::Software, Track::Video),
        "1.000000",
    );
    let hevc = metadata(
        br#"{"streams":[{"index":0,"codec_type":"video","width":64,"height":48}],"format":{"format_name":"hevc"}}"#,
        size,
        0,
    )
    .expect("raw hevc metadata");
    assert!(hevc.raw_video);
    assert_eq!(hevc.header.duration_us, media::MAX_DURATION_US);
    assert_omits_seek(&command_args(
        Path::new("/input"),
        &hevc,
        &Backend::Software,
        Track::Video,
    ));
    assert!(probe(Path::new("/nonexistent-strata-media"), size, 0).is_err());
}
