// SPDX-License-Identifier: MIT

use super::*;
use crate::media::{Decoder, Packet};
use std::{io::Cursor, os::unix::net::UnixStream, thread};

mod audio;
mod commands;
mod playback;
mod runtime;

fn probe(path: &Path, size: MediaPreviewSize, start_tick: u32) -> io::Result<Input> {
    super::probe(path, size, start_tick, ProbeMode::Playback)
}

fn metadata(bytes: &[u8], size: MediaPreviewSize, start_tick: u32) -> io::Result<Input> {
    super::metadata(bytes, size, start_tick, ProbeMode::Playback)
}

fn success(command: &mut Command) -> Vec<u8> {
    let output = bounded_output_with_timeout(command, 32 * 1024 * 1024, Duration::from_secs(30))
        .expect("FFmpeg tools are required")
        .expect("fixture deadline");
    assert!(output.status.success(), "fixture failed: {command:?}");
    output.stdout
}

/// A synthetic test pattern clip, optionally with a sine tone.
struct Clip {
    size: &'static str,
    rate: u32,
    seconds: u32,
    audio: bool,
}

fn fixture(path: &Path, clip: Clip) {
    let Clip {
        size,
        rate,
        seconds,
        audio,
    } = clip;
    let mut command = Command::new("ffmpeg");
    command
        .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "testsrc2=size={size}:rate={rate}:duration={seconds}"
        ));
    if audio {
        command.args(["-f", "lavfi", "-i"]).arg(format!(
            "sine=frequency=660:sample_rate=48000:duration={seconds}"
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

/// A test pattern GIF at ten frames per second.
fn animated_gif(path: &Path, seconds: u32) {
    success(
        Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size=64x48:rate=10:duration={seconds}"))
            .args(["-threads", "1"])
            .arg(path),
    );
}

fn decoded(input: &Path, size: &str, start: u32) -> (Header, Vec<Frame>, u64) {
    let mut bytes = Vec::new();
    stream(
        input,
        size,
        MediaPreviewBackend::Software,
        start,
        false,
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

fn stream_fails(input: &Path, start: u32) -> bool {
    stream(
        input,
        "520x800",
        MediaPreviewBackend::Software,
        start,
        false,
        &mut Vec::new(),
    )
    .is_err()
}

fn has_audible_samples(frames: &[Frame]) -> bool {
    frames
        .iter()
        .any(|frame| frame.samples.iter().any(|sample| *sample != 0))
}

fn audible(block: &[u8]) -> bool {
    block.iter().any(|sample| *sample != 0)
}

/// The PCM block for the audio tick `offset` ticks after the first frame; the
/// lead places it in the first record or ahead of its own frame.
fn audio_block(frames: &[Frame], offset: usize) -> &[u8] {
    let lead = media::AUDIO_LEAD_TICKS as usize;
    if offset <= lead {
        &frames[0].samples[offset * AUDIO_BYTES..(offset + 1) * AUDIO_BYTES]
    } else {
        &frames[offset - lead].samples
    }
}

fn command_args(path: &Path, input: &Input, backend: &Backend, track: Track) -> Vec<String> {
    command(path, input, backend, track)
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn software_args(input: &Input, track: Track) -> Vec<String> {
    command_args(Path::new("/input"), input, &Backend::Software, track)
}

/// How many `-ss` and `-noaccurate_seek` flags the command carries.
fn seek_flag_counts(args: &[String]) -> (usize, usize) {
    let count = |flag: &str| args.iter().filter(|arg| *arg == flag).count();
    (count("-ss"), count("-noaccurate_seek"))
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
    assert_eq!(seek_flag_counts(args).0, 1);
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
    assert_eq!(
        (args[ss + 1].as_str(), seek_flag_counts(args)),
        (timestamp, (1, 0))
    );
}

fn assert_omits_seek(args: &[String]) {
    assert_eq!(seek_flag_counts(args), (0, 0), "{args:?}");
}

/// The needles that `haystack` contains, for asserting a set is absent.
fn present<'a>(haystack: &str, needles: &[&'a str]) -> Vec<&'a str> {
    needles
        .iter()
        .copied()
        .filter(|needle| haystack.contains(needle))
        .collect()
}

/// The needles that `haystack` lacks, for asserting a set is present.
fn missing<'a>(haystack: &str, needles: &[&'a str]) -> Vec<&'a str> {
    needles
        .iter()
        .copied()
        .filter(|needle| !haystack.contains(needle))
        .collect()
}

const NO_TERMS: Vec<&str> = Vec::new();

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
