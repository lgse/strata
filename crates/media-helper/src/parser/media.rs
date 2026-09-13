// SPDX-License-Identifier: MIT

use std::{
    io::{self, Read, Write},
    os::fd::AsFd,
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, Stdio},
    time::{Duration, Instant},
};

use strata_media_protocol::{
    self as media, AUDIO_BYTES, Cancellation, FRAME_TIMEOUT, Frame, Header, MediaPreviewBackend,
    MediaPreviewSize, TimedReader,
    devices::{gpu_devices, numbered_name},
};

use super::{bounded_output_with_timeout, media_preview_size, stop_child};

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
}

pub(super) fn run(
    input: &Path,
    output: &Path,
    size: &str,
    policy: MediaPreviewBackend,
    start_tick: u32,
) -> Result<(), String> {
    let mut writer = std::fs::File::create(output).map_err(|error| error.to_string())?;
    stream(input, size, policy, start_tick, &mut writer)
}

fn stream(
    input: &Path,
    size: &str,
    policy: MediaPreviewBackend,
    start_tick: u32,
    writer: &mut impl Write,
) -> Result<(), String> {
    let size = media_preview_size(size)?;
    let input_info = probe(input, size, start_tick).map_err(|error| error.to_string())?;
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

fn probe(path: &Path, size: MediaPreviewSize, start_tick: u32) -> io::Result<Input> {
    let output = bounded_output_with_timeout(Command::new("ffprobe").args([
        "-v", "error", "-show_entries",
        "stream=index,codec_type,width,height,sample_aspect_ratio:stream_disposition=attached_pic:stream_side_data=rotation:format=duration,format_name",
        "-of", "json",
    ]).arg(path), 64 * 1024, PROBE_TIMEOUT)?
        .filter(|output| output.status.success()).ok_or_else(|| io::Error::other("Unable to inspect media inside the sandbox"))?;
    metadata(&output.stdout, size, start_tick)
}

fn metadata(bytes: &[u8], size: MediaPreviewSize, start_tick: u32) -> io::Result<Input> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let streams = value["streams"]
        .as_array()
        .ok_or_else(|| media::invalid("Missing media streams"))?;
    let is_cover =
        |stream: &serde_json::Value| stream["disposition"]["attached_pic"].as_u64() == Some(1);
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video" && !is_cover(stream))
        .or_else(|| {
            streams
                .iter()
                .find(|stream| stream["codec_type"] == "video")
        });
    let audio = streams
        .iter()
        .find(|stream| stream["codec_type"] == "audio");
    let index = |stream: &serde_json::Value| -> io::Result<u32> {
        stream["index"]
            .as_u64()
            .filter(|index| *index < 1024)
            .map(|index| index as u32)
            .ok_or_else(|| media::invalid("Invalid media stream index"))
    };
    let duration = value["format"]["duration"]
        .as_str()
        .and_then(|duration| duration.parse::<f64>().ok())
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .unwrap_or(media::MAX_DURATION_US as f64 / 1_000_000.0);
    let gif_period_us = (value["format"]["format_name"] == "gif" && duration < 30.0)
        .then_some((duration * 1_000_000.0).ceil() as u64);
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
        video: video.map(index).transpose()?,
        audio: audio.map(index).transpose()?,
        cover: video.is_some_and(is_cover),
        gif_period_us,
    })
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
    let mut command = Command::new("prlimit");
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
    let start = input
        .gif_period_us
        .map_or(start_us, |period| start_us % period.max(1)) as f64
        / 1_000_000.0;
    if input.gif_period_us.is_some() {
        command.args(["-stream_loop", "-1"]);
    }
    let remaining =
        (input.header.duration_us - media::timestamp(input.header.start_tick)) as f64 / 1_000_000.0;
    let cover = input.cover && matches!(track, Track::Video);
    command
        .arg("-ss")
        .arg(format!("{:.6}", if cover { 0.0 } else { start }));
    // Keep the frame covering the seek point; fps trims negative preroll timestamps.
    if matches!(track, Track::Video) && !cover {
        command.arg("-noaccurate_seek");
    }
    command.arg("-i").arg(path);
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
    audio_ended: bool,
    decoded_video: bool,
}

impl RawDecoder {
    fn spawn(path: &Path, input: &Input, backend: &Backend) -> io::Result<Self> {
        let mut decoder = Self {
            children: Vec::new(),
            video: None,
            audio: None,
            video_ended: input.video.is_none(),
            audio_ended: input.audio.is_none(),
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
        let mut samples = if self.audio.is_some() {
            vec![0; AUDIO_BYTES]
        } else {
            Vec::new()
        };
        let mut video_bytes = 0;
        let mut audio_bytes = 0;
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
        if let Some(audio) = &self.audio
            && !self.audio_ended
        {
            audio_bytes = read_chunk(audio, &mut samples, deadline)?;
            if audio_bytes < samples.len() {
                self.audio_ended = true;
            }
            if audio_bytes % 4 != 0 {
                return Err(media::invalid("Truncated PCM sample"));
            }
        }
        if !self.successful()? {
            return Err(io::Error::other("The media decoder failed"));
        }
        if video_bytes == 0 && audio_bytes == 0 {
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
