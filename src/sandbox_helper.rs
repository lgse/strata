// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use gdk_pixbuf::prelude::*;
use gtk::gio;

use crate::sandbox::{MAX_OUTPUT_BYTES, MediaPreviewBackend, gpu_devices, numbered_name};

const HARDWARE_ATTEMPT_TIME_LIMIT: Duration = Duration::from_secs(8);
const HARDWARE_TOTAL_TIME_LIMIT: Duration = Duration::from_secs(12);
const MEDIA_TOTAL_TIME_LIMIT: Duration = Duration::from_secs(28);
const MAX_MEDIA_ALLOCATION_BYTES: u64 = 512 * 1024 * 1024;
const MAX_MEDIA_DECODE_PIXELS: u64 = 50_000_000;
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, Eq, PartialEq)]
enum MediaBackend {
    VaApi(PathBuf),
    Vulkan(usize),
    Software,
}

pub(crate) fn run(arguments: &[String]) -> Result<(), String> {
    let [operation, input, output, value, media_backend] = arguments else {
        return Err("Invalid preview helper arguments".to_owned());
    };
    let input = Path::new(input);
    let output = Path::new(output);
    let value = value
        .parse::<i32>()
        .map_err(|_| "Invalid preview helper size or page".to_owned())?;
    let media_backend = MediaPreviewBackend::from_argument(media_backend)
        .ok_or_else(|| "Invalid media preview backend".to_owned())?;

    let (png, metadata) = match operation.as_str() {
        "thumbnail-image" => (render_pixbuf(input, value.clamp(16, 256))?, None),
        "thumbnail-raw" => (render_raw(input, value.clamp(16, 256))?, None),
        "thumbnail-pdf" => (render_pdf_thumbnail(input, value.clamp(16, 256))?, None),
        "thumbnail-video" => (render_media(input, value.clamp(16, 256))?, None),
        "preview-image" => (render_raw(input, 1400)?, None),
        "preview-pdf" => {
            let (png, page, pages) = render_pdf_page(input, value)?;
            (png, Some(format!("{page} {pages}")))
        }
        "preview-media" => (render_media_preview(input, media_backend)?, None),
        _ => return Err("Unknown preview helper operation".to_owned()),
    };
    fs::write(output, png).map_err(|error| error.to_string())?;
    if let Some(metadata) = metadata {
        fs::write(output.with_file_name("result.meta"), metadata)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn render_pixbuf(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    gdk_pixbuf::Pixbuf::from_file_at_scale(path, size, size, true)
        .map_err(|error| error.to_string())?
        .save_to_bufferv("png", &[])
        .map_err(|error| error.to_string())
}

fn render_raw(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    render_pixbuf(path, size)
        .or_else(|_| render_imagemagick(path, size))
        .or_else(|_| render_dcraw(path, size))
}

fn render_imagemagick(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    for executable in ["magick", "convert"] {
        let output = bounded_output(
            Command::new(executable)
                .arg(path)
                .args(["-auto-orient", "-thumbnail"])
                .arg(format!("{size}x{size}"))
                .arg("png:-"),
            MAX_OUTPUT_BYTES,
        );
        if let Ok(output) = output
            && output.status.success()
            && !output.stdout.is_empty()
        {
            return Ok(output.stdout);
        }
    }
    Err("No RAW image renderer succeeded".to_owned())
}

// LibRaw's dcraw_emu does not support `-e`; `-c` is a threshold, not stdout.
fn render_dcraw(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let classic = bounded_output(
        Command::new("dcraw").args(["-e", "-c"]).arg(path),
        MAX_OUTPUT_BYTES,
    );
    if let Ok(output) = classic
        && output.status.success()
        && !output.stdout.is_empty()
        && let Ok(png) = scale_embedded_thumbnail(&output.stdout, size)
    {
        return Ok(png);
    }
    render_simple_dcraw(path, size)
}

fn render_simple_dcraw(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    use std::os::unix::fs::symlink;

    // Writes `<file>.thumb.jpg` next to the input, which is a read-only bind.
    let staging = Path::new("/tmp/raw-thumb");
    let _ = fs::remove_file(staging);
    symlink(path, staging).map_err(|error| error.to_string())?;
    let status = Command::new("simple_dcraw")
        .arg("-e")
        .arg(staging)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("simple_dcraw failed".to_owned());
    }
    for thumb in ["/tmp/raw-thumb.thumb.jpg", "/tmp/raw-thumb.thumb.ppm"] {
        let Ok(file) = fs::File::open(thumb) else {
            continue;
        };
        let Ok(data) = read_limited(file, MAX_OUTPUT_BYTES) else {
            continue;
        };
        if data.is_empty() {
            continue;
        }
        if let Ok(png) = scale_embedded_thumbnail(&data, size) {
            return Ok(png);
        }
    }
    Err("simple_dcraw produced no thumbnail".to_owned())
}

fn scale_embedded_thumbnail(data: &[u8], size: i32) -> Result<Vec<u8>, String> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader
        .write(data)
        .and_then(|()| loader.close())
        .map_err(|error| error.to_string())?;
    let pixbuf = loader
        .pixbuf()
        .ok_or_else(|| "Unable to decode embedded RAW thumbnail".to_owned())?;
    let width = pixbuf.width().max(1);
    let height = pixbuf.height().max(1);
    let scale = (f64::from(size) / f64::from(width))
        .min(f64::from(size) / f64::from(height))
        .min(1.0);
    pixbuf
        .scale_simple(
            (f64::from(width) * scale).round().max(1.0) as i32,
            (f64::from(height) * scale).round().max(1.0) as i32,
            gdk_pixbuf::InterpType::Bilinear,
        )
        .ok_or_else(|| "Unable to scale embedded RAW thumbnail".to_owned())?
        .save_to_bufferv("png", &[])
        .map_err(|error| error.to_string())
}

fn render_pdf_thumbnail(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let uri = gio::File::for_path(path).uri();
    let document = poppler::Document::from_file(&uri, None).map_err(|error| error.to_string())?;
    let page = document
        .page(0)
        .ok_or_else(|| "This PDF has no pages".to_owned())?;
    render_pdf_surface(
        &page,
        f64::from(size),
        f64::from(size),
        f64::from(size * size),
    )
}

fn render_pdf_page(path: &Path, requested_page: i32) -> Result<(Vec<u8>, i32, i32), String> {
    let uri = gio::File::for_path(path).uri();
    let document = poppler::Document::from_file(&uri, None).map_err(|error| error.to_string())?;
    let pages = document.n_pages();
    if pages <= 0 {
        return Err("This PDF has no pages".to_owned());
    }
    let page_index = requested_page.clamp(0, pages - 1);
    let page = document
        .page(page_index)
        .ok_or_else(|| "Unable to load that PDF page".to_owned())?;
    let png = render_pdf_surface(&page, 1400.0, 1800.0, 2_500_000.0)?;
    Ok((png, page_index, pages))
}

fn render_pdf_surface(
    page: &poppler::Page,
    max_width: f64,
    max_height: f64,
    max_pixels: f64,
) -> Result<Vec<u8>, String> {
    let (page_width, page_height) = page.size();
    if page_width <= 0.0 || page_height <= 0.0 {
        return Err("The PDF page has invalid dimensions".to_owned());
    }
    let (width, height, scale) =
        bounded_surface_dimensions(page_width, page_height, max_width, max_height, max_pixels);
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
        .map_err(|error| error.to_string())?;
    let context = cairo::Context::new(&surface).map_err(|error| error.to_string())?;
    context.set_source_rgb(1.0, 1.0, 1.0);
    context.paint().map_err(|error| error.to_string())?;
    context.scale(scale, scale);
    page.render(&context);
    surface.flush();
    let mut png = Vec::new();
    surface
        .write_to_png(&mut png)
        .map_err(|error| error.to_string())?;
    Ok(png)
}

fn bounded_surface_dimensions(
    source_width: f64,
    source_height: f64,
    max_width: f64,
    max_height: f64,
    max_pixels: f64,
) -> (i32, i32, f64) {
    let requested_scale = (max_width / source_width)
        .min(max_height / source_height)
        .min((max_pixels / (source_width * source_height)).sqrt());
    // Rounding both dimensions up can push the result beyond max_pixels, causing the parent to
    // reject an otherwise valid render. Round down and derive the final scale from the integer
    // surface so the page still fits without clipping.
    let width = (source_width * requested_scale).floor().max(1.0) as i32;
    let height = (source_height * requested_scale).floor().max(1.0) as i32;
    let scale = (f64::from(width) / source_width).min(f64::from(height) / source_height);
    (width, height, scale)
}

fn render_media_preview(path: &Path, policy: MediaPreviewBackend) -> Result<Vec<u8>, String> {
    let backends = media_backends(&gpu_devices(Path::new("/dev"), policy), policy);
    let started = Instant::now();
    let hardware_started = Instant::now();
    run_media_backends(&backends, |backend| {
        let total_remaining = MEDIA_TOTAL_TIME_LIMIT.saturating_sub(started.elapsed());
        let timeout = if *backend == MediaBackend::Software {
            total_remaining
        } else {
            let hardware_remaining =
                HARDWARE_TOTAL_TIME_LIMIT.saturating_sub(hardware_started.elapsed());
            HARDWARE_ATTEMPT_TIME_LIMIT
                .min(hardware_remaining)
                .min(total_remaining)
        };
        let mut command = media_command(backend, path);
        bounded_output_with_timeout(&mut command, MAX_OUTPUT_BYTES, timeout).map(|result| {
            result.and_then(|output| {
                (output.status.success() && !output.stdout.is_empty()).then_some(output.stdout)
            })
        })
    })
}

fn media_backends(devices: &[PathBuf], policy: MediaPreviewBackend) -> Vec<MediaBackend> {
    let mut render_nodes: Vec<_> = devices
        .iter()
        .filter(|device| {
            device
                .file_name()
                .is_some_and(|name| numbered_name(name, "renderD"))
        })
        .cloned()
        .collect();
    render_nodes.sort();
    let nvidia_devices = devices
        .iter()
        .filter(|device| {
            device
                .file_name()
                .is_some_and(|name| numbered_name(name, "nvidia"))
        })
        .count();
    let vulkan_devices = render_nodes.len().max(nvidia_devices);
    let mut backends = Vec::new();
    if matches!(
        policy,
        MediaPreviewBackend::Automatic | MediaPreviewBackend::VaApi
    ) {
        backends.extend(render_nodes.into_iter().map(MediaBackend::VaApi));
    }
    if matches!(
        policy,
        MediaPreviewBackend::Automatic | MediaPreviewBackend::Vulkan
    ) {
        backends.extend((0..vulkan_devices).map(MediaBackend::Vulkan));
    }
    backends.push(MediaBackend::Software);
    backends
}

fn media_command(backend: &MediaBackend, path: &Path) -> Command {
    let mut command = Command::new("ffmpeg");
    command
        .args(["-nostdin", "-v", "error", "-max_alloc"])
        .arg(MAX_MEDIA_ALLOCATION_BYTES.to_string())
        .arg("-max_pixels")
        .arg(MAX_MEDIA_DECODE_PIXELS.to_string());
    match backend {
        MediaBackend::VaApi(device) => {
            command
                .env("MALLOC_ARENA_MAX", "1")
                .args([
                    "-threads",
                    "1",
                    "-filter_threads",
                    "1",
                    "-hwaccel",
                    "vaapi",
                    "-hwaccel_device",
                ])
                .arg(device)
                .args(["-hwaccel_output_format", "vaapi"]);
        }
        MediaBackend::Vulkan(index) => {
            command
                .env("MALLOC_ARENA_MAX", "1")
                .args(["-threads", "1", "-filter_threads", "1", "-init_hw_device"])
                .arg(format!("vulkan=vk:{index}"))
                .args([
                    "-filter_hw_device",
                    "vk",
                    "-hwaccel",
                    "vulkan",
                    "-hwaccel_device",
                    "vk",
                    "-hwaccel_output_format",
                    "vulkan",
                ]);
        }
        MediaBackend::Software => {
            command.args(["-threads", "2"]);
        }
    }
    command
        .arg("-i")
        .arg(path)
        .args(["-map", "0:v:0", "-map", "0:a:0?", "-sn", "-dn", "-t", "30"]);
    match backend {
        MediaBackend::VaApi(_) => {
            command.args([
                "-vf",
                "scale_vaapi=w=1280:h=1280:force_original_aspect_ratio=decrease:force_divisible_by=16:format=nv12",
                "-c:v",
                "h264_vaapi",
            ]);
        }
        MediaBackend::Vulkan(_) => {
            command.args([
                "-vf",
                "scale_vulkan=w='max(16,trunc(min(iw,iw*1280/max(iw,ih))/16)*16)':h='max(16,trunc(min(ih,ih*1280/max(iw,ih))/16)*16)':format=nv12",
                "-c:v",
                "h264_vulkan",
                "-usage",
                "transcode",
                "-tune",
                "ull",
            ]);
        }
        MediaBackend::Software => {
            command.args([
                "-vf",
                "scale=w=1280:h=1280:force_original_aspect_ratio=decrease,format=yuv420p",
                "-c:v",
                "libvpx",
                "-auto-alt-ref",
                "0",
                "-threads",
                "2",
                "-deadline",
                "realtime",
                "-cpu-used",
                "8",
            ]);
        }
    }
    command.args(["-fpsmax", "30"]);
    command.args(["-b:v", "2M", "-maxrate", "3M", "-bufsize", "4M"]);
    match backend {
        MediaBackend::Software => command.args(["-c:a", "libopus", "-b:a", "96k", "-f", "webm"]),
        MediaBackend::VaApi(_) | MediaBackend::Vulkan(_) => command.args([
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-movflags",
            "+frag_keyframe+empty_moov",
            "-f",
            "mp4",
        ]),
    };
    command.arg("pipe:1");
    command
}

fn run_media_backends<T, E>(
    backends: &[MediaBackend],
    mut run: impl FnMut(&MediaBackend) -> Result<Option<T>, E>,
) -> Result<T, String> {
    for backend in backends {
        if let Ok(Some(output)) = run(backend) {
            return Ok(output);
        }
    }
    Err("Unable to normalize media preview".to_owned())
}

fn bounded_output_with_timeout(
    command: &mut Command,
    max_bytes: u64,
    timeout: Duration,
) -> io::Result<Option<Output>> {
    if timeout.is_zero() {
        return Ok(None);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Unable to capture provider output"))?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut data = Vec::new();
        let result = stdout
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut data)
            .map(|_| data);
        let _sent = sender.send(result);
    });
    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut output = None;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(current) => status = current,
                Err(error) => {
                    stop_child(&mut child);
                    let _joined = reader.join();
                    return Err(error);
                }
            }
        }
        if output.is_none() {
            match receiver.try_recv() {
                Ok(Ok(data)) if data.len() as u64 > max_bytes => {
                    stop_child(&mut child);
                    let _joined = reader.join();
                    return Err(io::Error::other(
                        "Preview provider output exceeded its limit",
                    ));
                }
                Ok(Ok(data)) => output = Some(data),
                Ok(Err(error)) => {
                    stop_child(&mut child);
                    let _joined = reader.join();
                    return Err(error);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    stop_child(&mut child);
                    let _joined = reader.join();
                    return Err(io::Error::other("Unable to read provider output"));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(status) = status
            && let Some(stdout) = output.take()
        {
            let _joined = reader.join();
            return Ok(Some(Output {
                status,
                stdout,
                stderr: Vec::new(),
            }));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            stop_child(&mut child);
            let _joined = reader.join();
            return Ok(None);
        }
        thread::sleep(PROCESS_POLL_INTERVAL.min(remaining));
    }
}

fn stop_child(child: &mut Child) {
    let _killed = child.kill();
    let _waited = child.wait();
}

pub(crate) fn run_command_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> io::Result<bool> {
    if timeout.is_zero() {
        return Ok(false);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.success());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            stop_child(&mut child);
            return Ok(false);
        }
        thread::sleep(PROCESS_POLL_INTERVAL.min(remaining));
    }
}

fn render_media(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let output = bounded_output(
        Command::new("ffmpegthumbnailer")
            .arg("-i")
            .arg(path)
            .args(["-o", "/dev/stdout", "-s"])
            .arg(size.to_string())
            .args(["-q", "8"]),
        MAX_OUTPUT_BYTES,
    )
    .map_err(|error| error.to_string())?;
    if output.status.success() && !output.stdout.is_empty() {
        Ok(output.stdout)
    } else {
        Err("Unable to render media thumbnail".to_owned())
    }
}

fn read_limited(reader: impl Read, max_bytes: u64) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    reader
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut data)?;
    if data.len() as u64 > max_bytes {
        return Err(io::Error::other(
            "Preview provider output exceeded its limit",
        ));
    }
    Ok(data)
}

fn bounded_output(command: &mut Command, max_bytes: u64) -> io::Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let read = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Unable to capture provider output"))
        .and_then(|stdout| read_limited(stdout, max_bytes));
    let stdout = match read {
        Ok(stdout) => stdout,
        Err(error) => {
            let _killed = child.kill();
            let _waited = child.wait();
            return Err(error);
        }
    };
    let status = child.wait()?;
    Ok(Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

#[cfg(test)]
mod tests;
