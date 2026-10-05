// SPDX-License-Identifier: MIT

use std::{
    io::{self, Read, Write},
    os::fd::AsFd,
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, Stdio},
    time::{Duration, Instant},
};

use crate::{
    media::{self, AUDIO_BYTES, FRAME_TIMEOUT, Frame, Header, TimedReader},
    sandbox::{Cancellation, MediaPreviewBackend, gpu_devices, numbered_name},
    services::MediaPreviewSize,
};

use super::{
    MAX_OUTPUT_BYTES, bounded_output, bounded_output_with_timeout, media_preview_size,
    read_limited, stop_child,
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(4);
const HARDWARE_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Backend {
    VaApi(PathBuf),
    Vulkan(usize),
    Software,
}

#[derive(Debug)]
struct Input {
    header: Header,
    video: Option<u32>,
    audio: Option<u32>,
    cover: bool,
    gif_period_us: Option<u64>,
    raw_video: bool,
}

pub(super) fn run(
    input: &Path,
    output: &Path,
    size: &str,
    policy: MediaPreviewBackend,
    start_tick: u32,
    audio_only: bool,
) -> Result<(), String> {
    let mut writer = std::fs::File::create(output).map_err(|error| error.to_string())?;
    stream(input, size, policy, start_tick, audio_only, &mut writer)
}

fn stream(
    input: &Path,
    size: &str,
    policy: MediaPreviewBackend,
    start_tick: u32,
    audio_only: bool,
    writer: &mut impl Write,
) -> Result<(), String> {
    let size = media_preview_size(size)?;
    let mode = if audio_only {
        ProbeMode::SkipArtwork
    } else {
        ProbeMode::Playback
    };
    let input_info = probe(input, size, start_tick, mode).map_err(|error| error.to_string())?;
    let backends =
        if input_info.video.is_none() || input_info.cover || input_info.gif_period_us.is_some() {
            vec![Backend::Software]
        } else {
            backends(&gpu_devices(Path::new("/dev"), policy), policy)
        };
    let hardware_deadline = Instant::now() + HARDWARE_TIMEOUT;
    for backend in backends {
        let deadline = if backend == Backend::Software {
            Instant::now() + FRAME_TIMEOUT
        } else {
            hardware_deadline.min(Instant::now() + ATTEMPT_TIMEOUT)
        };
        if deadline <= Instant::now() {
            continue;
        }
        let Ok(mut decoder) = RawDecoder::spawn(input, &input_info, &backend) else {
            continue;
        };
        let Ok(Some(first)) = decoder.frame(start_tick, deadline) else {
            continue;
        };
        let result = (|| -> io::Result<()> {
            input_info.header.write(writer)?;
            first.write(writer)?;
            let mut next = start_tick + 1;
            while next < input_info.header.ticks() {
                let Some(frame) = decoder.frame(next, Instant::now() + FRAME_TIMEOUT)? else {
                    break;
                };
                frame.write(writer)?;
                next += 1;
            }
            if !decoder.successful()? {
                return Err(io::Error::other("The media decoder failed"));
            }
            media::write_end(
                writer,
                next,
                input_info.header.duration_us.min(media::timestamp(next)),
            )?;
            writer.flush()
        })();
        return result.map_err(|error| error.to_string());
    }
    Err("No sandboxed media decoder succeeded".into())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProbeMode {
    Playback,
    SkipArtwork,
    Audio,
}

fn probe(
    path: &Path,
    size: MediaPreviewSize,
    start_tick: u32,
    mode: ProbeMode,
) -> io::Result<Input> {
    metadata(&probe_json(path)?, size, start_tick, mode)
}

fn probe_json(path: &Path) -> io::Result<Vec<u8>> {
    let output = bounded_output_with_timeout(Command::new("ffprobe").args([
        "-v", "error", "-show_entries",
        "stream=index,codec_type,width,height,sample_aspect_ratio:stream_disposition=attached_pic:stream_tags=comment,title:stream_side_data=rotation:format=duration,format_name",
        "-of", "json",
    ]).arg(path), 64 * 1024, PROBE_TIMEOUT)?
        .filter(|output| output.status.success()).ok_or_else(|| io::Error::other("Unable to inspect media inside the sandbox"))?;
    Ok(output.stdout)
}

fn is_cover(stream: &serde_json::Value) -> bool {
    stream["codec_type"] == "video" && stream["disposition"]["attached_pic"].as_u64() == Some(1)
}

fn stream_index(stream: &serde_json::Value) -> io::Result<u32> {
    stream["index"]
        .as_u64()
        .filter(|index| *index < 1024)
        .map(|index| index as u32)
        .ok_or_else(|| media::invalid("Invalid media stream index"))
}

fn attached_picture(streams: &[serde_json::Value]) -> Option<&serde_json::Value> {
    let mut pictures = streams.iter().filter(|stream| is_cover(stream));
    pictures
        .clone()
        .find(|stream| {
            ["comment", "title"].iter().any(|key| {
                stream["tags"][*key].as_str().is_some_and(|tag| {
                    matches!(
                        tag.to_ascii_lowercase().as_str(),
                        "cover (front)" | "front cover" | "front"
                    )
                })
            })
        })
        .or_else(|| pictures.next())
}

fn metadata(
    bytes: &[u8],
    size: MediaPreviewSize,
    start_tick: u32,
    mode: ProbeMode,
) -> io::Result<Input> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let streams = value["streams"]
        .as_array()
        .ok_or_else(|| media::invalid("Missing media streams"))?;
    let video = if mode == ProbeMode::Audio {
        None
    } else {
        streams
            .iter()
            .find(|stream| stream["codec_type"] == "video" && !is_cover(stream))
            .or_else(|| {
                (mode == ProbeMode::Playback)
                    .then(|| attached_picture(streams))
                    .flatten()
            })
    };
    let audio = streams
        .iter()
        .find(|stream| stream["codec_type"] == "audio");
    if mode == ProbeMode::Audio && audio.is_none() {
        return Err(media::invalid("The file has no audio track"));
    }
    let duration = value["format"]["duration"]
        .as_str()
        .and_then(|duration| duration.parse::<f64>().ok())
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .unwrap_or(media::MAX_DURATION_US as f64 / 1_000_000.0);
    let format_name = value["format"]["format_name"].as_str().unwrap_or("");
    let gif_period_us = (video.is_some() && format_name == "gif" && duration < 30.0)
        .then_some((duration * 1_000_000.0).ceil() as u64);
    let raw_video = format_name
        .split(',')
        .any(|name| matches!(name.trim(), "h264" | "hevc"));
    let duration = if gif_period_us.is_some() {
        30.0
    } else {
        duration
    };
    let (width, height) = if let Some(video) = video {
        let width = video["width"].as_u64().unwrap_or(0);
        let height = video["height"].as_u64().unwrap_or(0);
        if width == 0
            || height == 0
            || width
                .checked_mul(height)
                .is_none_or(|pixels| pixels > 50_000_000)
        {
            return Err(media::invalid("Unsupported source dimensions"));
        }
        let sar = video["sample_aspect_ratio"]
            .as_str()
            .and_then(|sar| sar.split_once(':'))
            .and_then(|(n, d)| Some(n.parse::<f64>().ok()? / d.parse::<f64>().ok()?))
            .filter(|sar| sar.is_finite() && *sar > 0.0 && *sar < 100.0)
            .unwrap_or(1.0);
        let (mut width, mut height) = (width as f64 * sar, height as f64);
        if video["side_data_list"].as_array().is_some_and(|data| {
            data.iter().any(|data| {
                data["rotation"]
                    .as_i64()
                    .is_some_and(|rotation| rotation.rem_euclid(180) == 90)
            })
        }) {
            std::mem::swap(&mut width, &mut height);
        }
        let scale = (f64::from(size.width) / width)
            .min(f64::from(size.height) / height)
            .min(1.0);
        (
            (width * scale).floor().max(1.0) as u32,
            (height * scale).floor().max(1.0) as u32,
        )
    } else {
        (0, 0)
    };
    let header = Header {
        width,
        height,
        audio: audio.is_some(),
        duration_us: (duration * 1_000_000.0).ceil() as u64,
        start_tick,
    }
    .validate(size, start_tick)?;
    Ok(Input {
        header,
        video: video.map(stream_index).transpose()?,
        audio: audio.map(stream_index).transpose()?,
        cover: video.is_some_and(is_cover),
        gif_period_us,
        raw_video,
    })
}

fn ffmpeg_command(backend: &Backend, background: bool) -> Command {
    let mut command = if background {
        let mut command = Command::new("nice");
        command.args(["-n", "10", "prlimit"]);
        command
    } else {
        Command::new("prlimit")
    };
    command.args(["--core=0", "--fsize=536870912"]);
    if *backend == Backend::Software {
        command.arg("--as=2147483648");
    }
    command.args([
        "--",
        "ffmpeg",
        "-nostdin",
        "-v",
        "quiet",
        "-max_alloc",
        "536870912",
        "-max_pixels",
        "50000000",
        "-threads",
        "2",
        "-thread_queue_size",
        "2",
        "-filter_threads",
        "1",
        "-filter_complex_threads",
        "1",
    ]);
    command.env("MALLOC_ARENA_MAX", "1");
    command
}

pub(super) fn run_peaks(input: &Path, output: &Path) -> Result<(), String> {
    let mut writer = std::fs::File::create(output).map_err(|error| error.to_string())?;
    let info = probe(input, MediaPreviewSize::new(16, 16), 0, ProbeMode::Audio)
        .map_err(|error| error.to_string())?;
    let audio = info.audio.ok_or("The file has no audio track")?;
    let duration_us = info.header.duration_us;
    if duration_us >= media::MAX_DURATION_US {
        return Err("The audio duration is unknown".into());
    }
    let mut child = ffmpeg_command(&Backend::Software, true)
        .arg("-i")
        .arg(input)
        .arg("-map")
        .arg(format!("0:{audio}"))
        .args(["-vn", "-sn", "-dn", "-ac", "1", "-ar"])
        .arg(media::peaks::SAMPLE_RATE.to_string())
        .args(["-c:a", "pcm_s16le", "-f", "s16le", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let result = (|| -> io::Result<()> {
        let pcm = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Missing decoded audio pipe"))?;
        media::peaks::write_header(&mut writer, duration_us)?;
        let mut accumulator = media::peaks::Accumulator::new(duration_us);
        let mut buffer = vec![0; 16 * 1024];
        loop {
            let read = read_chunk(&pcm, &mut buffer, Instant::now() + FRAME_TIMEOUT)?;
            if read == 0 {
                break;
            }
            accumulator.push(&buffer[..read - read % 2], &mut writer)?;
        }
        if !child.wait()?.success() {
            return Err(io::Error::other("The audio decoder failed"));
        }
        accumulator.finish(&mut writer)
    })();
    if result.is_err() {
        stop_child(&mut child);
    }
    result.map_err(|error| error.to_string())
}

/// One software keyframe decode per cell, each an input seek, so the cost
/// scales with the cell count rather than the file length.
pub(super) fn run_storyboard(input: &Path, output: &Path, cell_edge: u32) -> Result<(), String> {
    use media::storyboard::{self, Sheet};

    let mut writer = std::fs::File::create(output).map_err(|error| error.to_string())?;
    let edge = cell_edge.clamp(storyboard::MIN_CELL_EDGE, storyboard::MAX_CELL_EDGE) as i32;
    let info = probe(
        input,
        MediaPreviewSize::new(edge, edge),
        0,
        ProbeMode::Playback,
    )
    .map_err(|error| error.to_string())?;
    let Some(video) = info.video.filter(|_| !info.cover && !info.raw_video) else {
        return Err("The file has no seekable video stream".into());
    };
    if info.gif_period_us.is_some() {
        return Err("Animations have no storyboard".into());
    }
    let duration_us = info.header.duration_us;
    if duration_us >= media::MAX_DURATION_US {
        return Err("The video duration is unknown".into());
    }
    if duration_us < storyboard::MIN_DURATION_US {
        return Err("The video is too short for a storyboard".into());
    }
    let sheet = Sheet {
        width: info.header.width,
        height: info.header.height,
        count: storyboard::cell_count(duration_us),
        duration_us,
    }
    .validate()
    .map_err(|error| error.to_string())?;
    sheet
        .write(&mut writer)
        .map_err(|error| error.to_string())?;
    // Preroll timestamps can be negative; reset them so rawvideo sync keeps the frame.
    let filter = format!(
        "setpts=PTS-STARTPTS,scale={}:{}:flags=fast_bilinear,setsar=1,format=rgba",
        sheet.width, sheet.height
    );
    for index in storyboard::subdivision_order(sheet.count) {
        let seconds = sheet.cell_time_us(index) as f64 / 1_000_000.0;
        let mut child = ffmpeg_command(&Backend::Software, true)
            .args(["-noaccurate_seek", "-skip_frame", "nokey", "-ss"])
            .arg(format!("{seconds:.6}"))
            .arg("-i")
            .arg(input)
            .arg("-map")
            .arg(format!("0:{video}"))
            .args(["-an", "-sn", "-dn", "-vf"])
            .arg(&filter)
            .args(["-frames:v", "1", "-f", "rawvideo", "pipe:1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        let result = (|| -> io::Result<()> {
            let pipe = child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("Missing storyboard pipe"))?;
            let mut pixels = vec![0; sheet.cell_bytes()];
            let read = read_chunk(&pipe, &mut pixels, Instant::now() + FRAME_TIMEOUT)?;
            drop(pipe);
            let exited = child.wait()?.success();
            if exited && read == pixels.len() {
                storyboard::write_cell(&mut writer, index, &pixels)?;
                writer.flush()?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            stop_child(&mut child);
            return Err(error.to_string());
        }
    }
    storyboard::write_end(&mut writer, sheet).map_err(|error| error.to_string())
}

pub(super) fn audio_tags(input: &Path) -> Result<Vec<u8>, String> {
    let keys = crate::sandbox::metadata::TAG_KEYS;
    bounded_output_with_timeout(
        Command::new("ffprobe")
            .args(["-v", "error", "-threads", "1", "-select_streams", "a:0"])
            .arg("-show_entries")
            .arg(format!("format_tags={keys}:stream_tags={keys}"))
            .args(["-of", "json"])
            .arg(input),
        crate::sandbox::metadata::MAX_METADATA_BYTES,
        PROBE_TIMEOUT,
    )
    .map_err(|error| error.to_string())?
    .filter(|output| output.status.success())
    .map(|output| output.stdout)
    .ok_or_else(|| "Unable to read audio tags".into())
}

pub(super) fn cover(input: &Path, size: u32) -> Result<Vec<u8>, String> {
    let bytes = probe_json(input).map_err(|error| error.to_string())?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let streams = value["streams"].as_array().ok_or("Missing media streams")?;
    let Some(picture) = attached_picture(streams) else {
        return Ok(Vec::new());
    };
    let index = stream_index(picture).map_err(|error| error.to_string())?;
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let path = directory.path().join("cover.png");
    let output = bounded_output(
        ffmpeg_command(&Backend::Software, false)
            .arg("-i")
            .arg(input)
            .arg("-map")
            .arg(format!("0:{index}"))
            .args(["-frames:v", "1", "-an", "-sn", "-dn", "-vf"])
            .arg(format!(
                "scale=w='min(iw,{size})':h='min(ih,{size})':force_original_aspect_ratio=decrease"
            ))
            .args(["-c:v", "png", "-threads", "1", "-y"])
            .arg(&path),
        MAX_OUTPUT_BYTES,
    )
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("Unable to decode embedded artwork".into());
    }
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    read_limited(file, MAX_OUTPUT_BYTES).map_err(|error| error.to_string())
}

fn backends(devices: &[PathBuf], policy: MediaPreviewBackend) -> Vec<Backend> {
    let mut nodes: Vec<_> = devices
        .iter()
        .filter(|device| {
            device
                .file_name()
                .is_some_and(|name| numbered_name(name, "renderD"))
        })
        .cloned()
        .collect();
    nodes.sort();
    let vulkan_count = nodes.len().max(
        devices
            .iter()
            .filter(|device| {
                device
                    .file_name()
                    .is_some_and(|name| numbered_name(name, "nvidia"))
            })
            .count(),
    );
    let mut result = Vec::new();
    if matches!(
        policy,
        MediaPreviewBackend::Automatic | MediaPreviewBackend::VaApi
    ) {
        result.extend(nodes.into_iter().map(Backend::VaApi));
    }
    if matches!(
        policy,
        MediaPreviewBackend::Automatic | MediaPreviewBackend::Vulkan
    ) {
        result.extend((0..vulkan_count).map(Backend::Vulkan));
    }
    result.push(Backend::Software);
    result
}

#[derive(Clone, Copy)]
enum Track {
    Video,
    Audio,
}

fn command(path: &Path, input: &Input, backend: &Backend, track: Track) -> Command {
    let mut command = ffmpeg_command(backend, false);
    match backend {
        Backend::VaApi(device) => {
            command
                .args(["-hwaccel", "vaapi", "-hwaccel_device"])
                .arg(device);
        }
        Backend::Vulkan(index) => {
            command
                .args(["-hwaccel", "vulkan", "-hwaccel_device"])
                .arg(index.to_string());
        }
        Backend::Software => {}
    }
    let start_us = media::timestamp(input.header.start_tick);
    let offset_us = input
        .gif_period_us
        .map_or(start_us, |period| start_us % period.max(1));
    let start = offset_us as f64 / 1_000_000.0;
    if input.gif_period_us.is_some() {
        command.args(["-stream_loop", "-1"]);
    }
    let remaining =
        (input.header.duration_us - media::timestamp(input.header.start_tick)) as f64 / 1_000_000.0;
    let cover = input.cover && matches!(track, Track::Video);
    // Input seeking can discard attached pictures and timestamp-less raw video.
    let seek = !cover && offset_us != 0;
    let position = format!("{start:.6}");
    if seek && !input.raw_video {
        command.arg("-ss").arg(&position);
    }
    // Keep the frame covering the seek point; fps trims negative preroll timestamps.
    if seek && !input.raw_video && matches!(track, Track::Video) {
        command.arg("-noaccurate_seek");
    }
    command.arg("-i").arg(path);
    if seek && input.raw_video {
        command.arg("-ss").arg(position);
    }
    match track {
        Track::Video => {
            let filter = format!(
                "{}scale={}:{}:flags=fast_bilinear,setsar=1,format=rgba",
                if cover { "" } else { "fps=30:start_time=0," },
                input.header.width,
                input.header.height
            );
            command
                .arg("-map")
                .arg(format!("0:{}", input.video.expect("video track")))
                .args(["-an", "-sn", "-dn", "-vf"])
                .arg(filter)
                .args([
                    "-c:v",
                    "rawvideo",
                    "-threads",
                    "1",
                    "-thread_queue_size",
                    "2",
                    "-max_muxing_queue_size",
                    "2",
                    "-frames:v",
                ])
                .arg(
                    if cover {
                        1
                    } else {
                        input.header.ticks() - input.header.start_tick
                    }
                    .to_string(),
                )
                .arg("-t")
                .arg(format!("{remaining:.6}"))
                .args(["-f", "rawvideo", "pipe:1"]);
        }
        Track::Audio => {
            command
                .arg("-map")
                .arg(format!("0:{}", input.audio.expect("audio track")))
                .args([
                    "-vn",
                    "-sn",
                    "-dn",
                    "-af",
                    "aresample=48000:async=1:first_pts=0",
                    "-ac",
                    "2",
                    "-ar",
                    "48000",
                    "-c:a",
                    "pcm_s16le",
                    "-thread_queue_size",
                    "2",
                    "-max_muxing_queue_size",
                    "2",
                    "-t",
                ])
                .arg(format!("{remaining:.6}"))
                .args(["-f", "s16le", "pipe:1"]);
        }
    }
    command
}

struct RawDecoder {
    children: Vec<Child>,
    video: Option<ChildStdout>,
    audio: Option<ChildStdout>,
    last_pixels: Vec<u8>,
    video_ended: bool,
    audio_tick: u32,
    audio_end: Option<u32>,
    decoded_video: bool,
}

impl RawDecoder {
    fn spawn(path: &Path, input: &Input, backend: &Backend) -> io::Result<Self> {
        let mut decoder = Self {
            children: Vec::new(),
            video: None,
            audio: None,
            video_ended: input.video.is_none(),
            audio_tick: input.header.start_tick,
            audio_end: input.audio.is_none().then_some(input.header.start_tick),
            decoded_video: false,
            last_pixels: vec![0; input.header.video_bytes()],
        };
        for (present, track, backend) in [
            (input.video.is_some(), Track::Video, backend),
            (input.audio.is_some(), Track::Audio, &Backend::Software),
        ] {
            if !present {
                continue;
            }
            let mut child = command(path, input, backend, track)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?;
            match track {
                Track::Video => decoder.video = child.stdout.take(),
                Track::Audio => decoder.audio = child.stdout.take(),
            }
            decoder.children.push(child);
        }
        Ok(decoder)
    }

    fn successful(&mut self) -> io::Result<bool> {
        for child in &mut self.children {
            if child.try_wait()?.is_some_and(|status| !status.success()) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn frame(&mut self, tick: u32, deadline: Instant) -> io::Result<Option<Frame>> {
        let mut pixels = self.last_pixels.clone();
        let mut video_bytes = 0;
        if let Some(video) = &self.video
            && !self.video_ended
        {
            video_bytes = read_chunk(video, &mut pixels, deadline)?;
            if video_bytes == 0 {
                self.video_ended = true;
            } else if video_bytes != pixels.len() {
                return Err(media::invalid("Truncated decoded frame"));
            } else {
                self.decoded_video = true;
                self.last_pixels.clone_from(&pixels);
            }
        }
        if self.video.is_some() && !self.decoded_video {
            return Err(media::invalid("No decoded video frame"));
        }
        let mut samples = Vec::new();
        if let Some(audio) = &self.audio {
            // The first record carries the whole lead, later ones a single block;
            // blocks past the end of the track stay silent.
            let wanted = tick
                .saturating_add(media::AUDIO_LEAD_TICKS + 1)
                .saturating_sub(self.audio_tick);
            samples = vec![0; wanted as usize * AUDIO_BYTES];
            for block in samples.chunks_mut(AUDIO_BYTES) {
                if self.audio_end.is_none() {
                    let read = read_chunk(audio, block, deadline)?;
                    if read % 4 != 0 {
                        return Err(media::invalid("Truncated PCM sample"));
                    }
                    if read < AUDIO_BYTES {
                        self.audio_end = Some(self.audio_tick.saturating_add(u32::from(read > 0)));
                    }
                }
                self.audio_tick = self.audio_tick.saturating_add(1);
            }
        }
        if !self.successful()? {
            return Err(io::Error::other("The media decoder failed"));
        }
        if video_bytes == 0 && self.audio_end.is_some_and(|end| tick >= end) {
            return Ok(None);
        }
        Ok(Some(Frame {
            tick,
            pixels,
            samples,
        }))
    }
}

impl Drop for RawDecoder {
    fn drop(&mut self) {
        for child in &mut self.children {
            stop_child(child);
        }
    }
}

fn read_chunk(fd: &impl AsFd, bytes: &mut [u8], deadline: Instant) -> io::Result<usize> {
    let cancellation = Cancellation::default();
    let mut reader = TimedReader {
        fd,
        deadline,
        cancellation: &cancellation,
    };
    let mut read = 0;
    while read < bytes.len() {
        let count = reader.read(&mut bytes[read..])?;
        if count == 0 {
            break;
        }
        read += count;
    }
    Ok(read)
}

#[cfg(test)]
mod tests;
