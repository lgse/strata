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

fn still_image(path: &Path, codec: &str) {
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
                "-c:v",
            ])
            .arg(codec)
            .arg(path),
    );
}

fn attach_cover(audio: &Path, cover: &Path, output: &Path, encode: &[&str]) {
    let mut command = Command::new("ffmpeg");
    command
        .args(["-v", "error", "-i"])
        .arg(audio)
        .arg("-i")
        .arg(cover)
        .args(["-map", "0:a", "-map", "1:v"])
        .args(encode)
        .args(["-disposition:v", "attached_pic"])
        .arg(output);
    success(&mut command);
}

fn probe_stream_indexes(path: &Path, specifier: &str) -> String {
    String::from_utf8(success(
        Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                specifier,
                "-show_entries",
                "stream=index",
                "-of",
                "csv=p=0",
            ])
            .arg(path),
    ))
    .expect("ffprobe stream indexes")
    .trim()
    .to_owned()
}

fn assert_probe_lists_cover_only(path: &Path) {
    assert!(
        !probe_stream_indexes(path, "v").is_empty(),
        "{path:?} should list an attached picture under v"
    );
    assert!(
        probe_stream_indexes(path, "V").is_empty(),
        "{path:?} should list no motion video under V"
    );
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
    let png = directory.path().join("cover.png");
    still_image(&png, "png");
    let jpeg = directory.path().join("cover.jpg");
    still_image(&jpeg, "mjpeg");
    let flac = directory.path().join("cover.flac");
    attach_cover(&audio, &png, &flac, &["-c", "copy"]);
    let jpeg_flac = directory.path().join("cover-jpeg.flac");
    attach_cover(&audio, &jpeg, &jpeg_flac, &["-c", "copy"]);
    let m4a = directory.path().join("cover.m4a");
    attach_cover(&audio, &png, &m4a, &["-c:a", "aac", "-c:v", "png"]);
    for attached in [&flac, &jpeg_flac, &m4a] {
        assert_probe_lists_cover_only(attached);
        let info = probe(attached, MediaPreviewSize::new(520, 800), 0).expect("cover metadata");
        assert!(info.video.is_none());
        assert!(!info.cover);
        assert_eq!(info.audio, Some(0));
        for policy in [
            MediaPreviewBackend::Automatic,
            MediaPreviewBackend::VaApi,
            MediaPreviewBackend::Vulkan,
            MediaPreviewBackend::Software,
        ] {
            let mut bytes = Vec::new();
            stream(attached, "520x800", policy, 0, &mut bytes).expect("cover-art audio");
            let header = Header::read(&mut Cursor::new(bytes), MediaPreviewSize::new(520, 800), 0)
                .expect("cover-art header");
            assert!(header.audio);
            assert_eq!((header.width, header.height), (0, 0));
        }
        let audio_command = command_for_audio(&info);
        assert!(audio_command.contains("-map 0:0"));
        assert!(audio_command.contains("-vn"));
        assert!(!audio_command.contains("-hwaccel"));
        assert!(!audio_command.contains("0:v"));
        assert!(!audio_command.contains("h264_vaapi"));
    }
}

#[test]
fn movie_with_attached_cover_keeps_the_motion_video_stream() {
    let directory = tempfile::tempdir().expect("fixtures");
    let clip = directory.path().join("clip.mkv");
    fixture(&clip, "64x48", 1, 1, true);
    let png = directory.path().join("cover.png");
    still_image(&png, "png");
    let movie = directory.path().join("movie.mp4");
    success(
        Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&clip)
            .arg("-i")
            .arg(&png)
            .args([
                "-map",
                "0:v",
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:v:0",
                "libx264",
                "-threads",
                "1",
                "-c:a",
                "aac",
                "-c:v:1",
                "png",
                "-disposition:v:1",
                "attached_pic",
            ])
            .arg(&movie),
    );
    let all_video = probe_stream_indexes(&movie, "v");
    let motion_video = probe_stream_indexes(&movie, "V");
    assert!(
        all_video.contains('0') && all_video.contains('2'),
        "{all_video}"
    );
    assert_eq!(motion_video, "0");
    let info = probe(&movie, MediaPreviewSize::new(520, 800), 0).expect("movie metadata");
    assert_eq!(info.video, Some(0));
    assert_eq!(info.audio, Some(1));
    assert!(!info.cover);
    let video = command(Path::new("/input"), &info, &Backend::Software, Track::Video);
    let args = video
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(args.contains("-map 0:0"));
    assert!(!args.contains("-map 0:2"));
    assert!(!args.contains("0:v:0"));
    let (header, frames, _) = decoded(&movie, "520x800", 0);
    assert!(header.audio);
    assert_eq!((header.width, header.height), (64, 48));
    assert!(!frames.is_empty());
    assert!(
        frames
            .iter()
            .any(|frame| !frame.pixels.iter().all(|byte| *byte == 0))
    );
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
    let input = Input {
        header: Header {
            width: 320,
            height: 180,
            audio: true,
            duration_us: 3_600_000_000,
            start_tick: 107850,
        },
        video: Some(0),
        audio: Some(1),
        cover: false,
        gif_period_us: None,
    };
    for backend in backends(&devices, MediaPreviewBackend::Automatic) {
        let command = command(Path::new("/input"), &input, &backend, Track::Video);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(args.contains("-ss 3595.000000"));
        assert!(args.contains("-t 5.000000"));
        assert!(args.contains("-frames:v 150"));
        assert!(args.contains("-c:v rawvideo"));
        let audio = command_for_audio(&input);
        assert!(audio.contains("-c:a pcm_s16le"));
        assert!(!audio.contains("-hwaccel"));
        assert!(!args.contains("h264"));
        assert!(!args.contains("libvpx"));
        assert!(!args.contains(" copy"));
    }
}

fn command_for_audio(input: &Input) -> String {
    command(Path::new("/input"), input, &Backend::Software, Track::Audio)
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
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
    assert!(probe(Path::new("/nonexistent-strata-media"), size, 0).is_err());
    let cover_only = metadata(
        br#"{"streams":[{"index":0,"codec_type":"audio"},{"index":1,"codec_type":"video","width":64,"height":48,"disposition":{"attached_pic":1}}],"format":{"duration":"1.0"}}"#,
        size,
        0,
    )
    .expect("attached pictures are not preview video");
    assert!(cover_only.video.is_none());
    assert!(!cover_only.cover);
    assert_eq!(cover_only.audio, Some(0));
    assert_eq!((cover_only.header.width, cover_only.header.height), (0, 0));
    let movie_and_cover = metadata(
        br#"{"streams":[{"index":0,"codec_type":"video","width":64,"height":48},{"index":1,"codec_type":"audio"},{"index":2,"codec_type":"video","width":32,"height":32,"disposition":{"attached_pic":1}}],"format":{"duration":"1.0"}}"#,
        size,
        0,
    )
    .expect("motion video keeps the video pipeline");
    assert_eq!(movie_and_cover.video, Some(0));
    assert_eq!(movie_and_cover.audio, Some(1));
    assert!(!movie_and_cover.cover);
    assert_eq!(
        (movie_and_cover.header.width, movie_and_cover.header.height),
        (64, 48)
    );
}
