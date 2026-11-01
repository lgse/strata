// SPDX-License-Identifier: MIT

use super::*;

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
        fixture(
            &input,
            Clip {
                size: source,
                rate: 60,
                seconds: 1,
                audio: true,
            },
        );
        let (header, frames, end) = decoded(&input, size, 0);
        assert_eq!(
            (header.width, header.height, header.audio, frames.len(), end),
            (expected.0, expected.1, true, 30, 1_000_000)
        );
        assert_ne!(frames[0].pixels, frames[20].pixels);
        assert!(has_audible_samples(&frames));
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
        fixture(
            &input,
            Clip {
                size: "64x48",
                rate: 1,
                seconds,
                audio,
            },
        );
        for start in starts {
            assert_full_decode_from(&input, seconds, audio, start);
        }
        assert!(stream_fails(&input, seconds * 30));
    }
}

fn assert_full_decode_from(input: &Path, seconds: u32, audio: bool, start: u32) {
    let (header, frames, end) = decoded(input, "520x800", start);
    assert_eq!(
        (
            header.duration_us,
            end,
            frames.len(),
            frames[0].tick,
            header.audio
        ),
        (
            u64::from(seconds) * 1_000_000,
            header.duration_us,
            (seconds * 30 - start) as usize,
            start,
            audio
        )
    );
    assert!(frames.iter().all(|frame| frame.samples.is_empty() != audio));
    if audio {
        assert!(has_audible_samples(&frames));
    }
}

#[test]
fn first_frames_arrive_before_completion_and_closed_consumers_cancel_backpressure() {
    let directory = tempfile::tempdir().expect("fixture");
    let input = directory.path().join("clip.mkv");
    fixture(
        &input,
        Clip {
            size: "160x90",
            rate: 30,
            seconds: 35,
            audio: true,
        },
    );
    let (read, mut write) = UnixStream::pair().expect("private frame pipe");
    let worker = thread::spawn(move || {
        stream(
            &input,
            "520x800",
            MediaPreviewBackend::Software,
            0,
            false,
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

fn assert_raw_probe_seeks(path: &Path) {
    let size = MediaPreviewSize::new(520, 800);
    let at_zero = probe(path, size, 0).expect("raw probe");
    assert_eq!(
        (at_zero.raw_video, at_zero.header.duration_us),
        (true, media::MAX_DURATION_US)
    );
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
}

fn assert_raw_frames_seek(path: &Path) {
    let (header, frames, end) = decoded(path, "520x800", 0);
    assert_eq!(
        (header.duration_us, frames.len(), end),
        (media::MAX_DURATION_US, 60, 2_000_000)
    );
    assert_ne!(frames[0].pixels, frames[30].pixels);
    for start in [30_u32, 20, 58] {
        let (sought_header, sought, sought_end) = decoded(path, "520x800", start);
        assert_eq!(
            (sought_header.duration_us, sought_end, sought.len()),
            (header.duration_us, end, frames.len() - start as usize)
        );
        assert_ne!(sought[0].pixels, frames[0].pixels);
        for (offset, frame) in sought.iter().enumerate() {
            assert_eq!(frame.pixels, frames[start as usize + offset].pixels);
        }
    }
}

fn assert_past_end_raw_seek_fails(path: &Path) {
    let mut past = Vec::new();
    let started = Instant::now();
    assert!(
        stream(
            path,
            "520x800",
            MediaPreviewBackend::Software,
            90,
            false,
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

fn assert_elementary_playback(path: &Path) {
    assert_raw_probe_seeks(path);
    assert_raw_frames_seek(path);
    assert_past_end_raw_seek_fails(path);
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
    assert_eq!(
        (mp4_info.raw_video, mp4_info.header.duration_us),
        (false, 2_000_000)
    );
    assert_input_seek(
        &command_args(&mp4, &mp4_info, &Backend::Software, Track::Video),
        "1.000000",
    );

    let hevc = directory.path().join("clip.hevc");
    encoded_video(&hevc, "hevc", "libx265");
    assert_elementary_playback(&hevc);
}

fn variable_rate_clip(path: &Path) {
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
            .arg(path),
    );
}

fn assert_audio_runs_ahead_of_its_frames(frames: &[Frame]) {
    assert!((0..8).all(|tick| !audible(audio_block(frames, tick))));
    assert!(audible(audio_block(frames, 10)));
    assert!(
        audible(audio_block(frames, 89)) && !audible(&frames[89].samples),
        "audio keeps its timeline while running a lead ahead of the frames"
    );
}

#[test]
fn variable_frame_rate_and_offset_audio_keep_their_original_timeline() {
    let directory = tempfile::tempdir().expect("fixtures");
    let input = directory.path().join("variable.mkv");
    variable_rate_clip(&input);
    let (_, frames, end) = decoded(&input, "160x90", 0);
    assert_eq!((end, frames.len()), (3_000_000, 90));
    assert_audio_runs_ahead_of_its_frames(&frames);
    assert!(
        frames[34].pixels == frames[35].pixels,
        "VFR frames must hold until their next presentation time"
    );
    let (_, sought, _) = decoded(&input, "160x90", 45);
    assert_eq!(sought[0].tick, 45);
    assert!(audible(audio_block(&sought, 0)));
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
    animated_gif(&input, 1);
    let (header, frames, end) = decoded(&input, "160x90", 0);
    assert_eq!(
        (header.duration_us, frames.len(), end),
        (30_000_000, 900, 30_000_000)
    );
    assert!(frames[0].pixels == frames[30].pixels, "GIF loop phase");
    assert!(frames[0].pixels != frames[15].pixels, "moving GIF frames");
    let (_, sought, _) = decoded(&input, "160x90", 765);
    assert!(
        frames[15].pixels == sought[0].pixels,
        "seek keeps the GIF phase"
    );
    let (_, loop_boundary, end) = decoded(&input, "160x90", 750);
    assert_eq!((loop_boundary.len(), end), (150, 30_000_000));
    for (actual, expected) in loop_boundary.iter().zip(&frames[750..]) {
        assert_eq!(actual.pixels, expected.pixels);
    }
    assert_long_gif_plays_once(&directory.path().join("long.gif"));
}

fn assert_long_gif_plays_once(long: &Path) {
    animated_gif(long, 35);
    let (header, frames, end) = decoded(long, "160x90", 0);
    assert_eq!(
        (header.duration_us, frames.len(), end),
        (35_000_000, 1050, 35_000_000)
    );
    let (_, sought, _) = decoded(long, "160x90", 960);
    assert_eq!(frames[960].pixels, sought[0].pixels);
}
