// SPDX-License-Identifier: MIT

#[cfg(feature = "rar")]
use std::io;
use std::{fs, io::Read, os::fd::RawFd, path::Path, process::Command, time::Duration};

use crate::{
    adapters::{encode_archive_result, list_archive_entries_direct},
    sandbox::{MAX_OUTPUT_BYTES, MediaPreviewBackend},
    services::{ArchiveFormat, MediaPreviewSize},
};

mod appimage;
pub(crate) mod archive_cover;
#[cfg(feature = "rar")]
mod archive_rar;
mod browser;
mod document_media;
mod image_conversion;
mod isobmff;
mod media;
mod model;
mod operations;
mod pdf;
mod process;
mod raster;
mod raw_metadata;
mod route;

pub(crate) use browser::browser_render;
#[cfg(test)]
use pdf::{bounded_surface_dimensions, pdf_render_request};
pub(crate) use process::run_command_with_timeout;
use process::{bounded_output, bounded_output_with_timeout, read_limited, stop_child};
use raster::render_raw;
#[cfg(test)]
use raster::{
    exceeds_decoded_frame_budget, image_dimensions, read_exif_thumbnail, read_png_dimensions,
    render_camera_raw, render_camera_raw_preview, render_pixbuf, render_simple_dcraw,
    scale_embedded_thumbnail,
};

fn parse_descriptor(descriptor: &str) -> Result<RawFd, String> {
    descriptor
        .parse::<RawFd>()
        .ok()
        .filter(|fd| *fd >= 0)
        .ok_or_else(|| "Invalid preview helper secret descriptor".to_owned())
}

fn read_password(descriptor: RawFd) -> Result<String, String> {
    let secret = read_secret_fd(descriptor)?;
    if secret.len() > crate::adapters::MAX_ARCHIVE_PASSWORD_BYTES {
        return Err("Archive password is too long.".to_owned());
    }
    String::from_utf8(secret).map_err(|_| "Archive password is not valid text.".to_owned())
}

#[cfg(feature = "rar")]
fn extract_rar(input: &str, rest: &[String]) -> Result<(), String> {
    let password = match rest {
        [] => None,
        [descriptor] => Some(read_password(parse_descriptor(descriptor)?)?),
        _ => return Err("Invalid preview helper arguments".to_owned()),
    };
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    archive_rar::run(Path::new(input), password.as_deref(), &mut stdout)
}

/// Splits the trailing seek position off a media preview's arguments.
fn split_start_tick(arguments: &[String]) -> Result<(&[String], u32), String> {
    match arguments {
        [operation, ..]
            if matches!(operation.as_str(), "preview-media" | "preview-audio")
                && arguments.len() == 6 =>
        {
            let start_tick = arguments[5]
                .parse::<u32>()
                .map_err(|_| "Invalid media seek position".to_owned())?;
            Ok((&arguments[..5], start_tick))
        }
        _ => Ok((arguments, 0)),
    }
}

fn secret_descriptor(arguments: &[String]) -> Result<Option<RawFd>, String> {
    match arguments {
        [operation, ..] if operation == "archive-list" && arguments.len() == 6 => {
            parse_descriptor(&arguments[5]).map(Some)
        }
        _ if arguments.len() == 5 => Ok(None),
        _ => Err("Invalid preview helper arguments".to_owned()),
    }
}

pub(crate) fn run(arguments: &[String]) -> Result<(), String> {
    #[cfg(not(feature = "rar"))]
    if arguments
        .first()
        .is_some_and(|operation| operation == "extract-rar")
    {
        return Err("RAR support is disabled in this build.".to_owned());
    }
    // Streams to its own stdout pipe instead of a bound `/output` file, so it
    // does not fit the fixed [operation, input, output, value, media_backend]
    // shape every other operation below shares.
    #[cfg(feature = "rar")]
    if let [operation, input, rest @ ..] = arguments
        && operation == "extract-rar"
    {
        return extract_rar(input, rest);
    }
    let (arguments, start_tick) = split_start_tick(arguments)?;
    let secret_fd = secret_descriptor(arguments)?;
    let [operation, input, output, value, media_backend] = &arguments[..5] else {
        return Err("Invalid preview helper arguments".to_owned());
    };
    let request = operations::Request {
        operation,
        input: Path::new(input),
        output: Path::new(output),
        value,
        media_backend: MediaPreviewBackend::from_argument(media_backend)
            .ok_or_else(|| "Invalid media preview backend".to_owned())?,
        start_tick,
        secret_fd,
    };
    // Decoders inherit the helper's lowered priority.
    media::lower_background_priority(operation);
    if let Some(result) = operations::write(&request) {
        return result;
    }
    operations::render(&request)?.write(request.output)
}

fn run_archive_list(
    input: &Path,
    output: &Path,
    format: &str,
    secret_fd: Option<RawFd>,
) -> Result<(), String> {
    let format = match format {
        "zip" => ArchiveFormat::Zip,
        "7z" => ArchiveFormat::SevenZ,
        "tar" => ArchiveFormat::Tar,
        "tar.gz" => ArchiveFormat::TarGz,
        _ => return Err("Unknown archive format for preview.".to_owned()),
    };
    let password = secret_fd.map(read_password).transpose()?;
    // The parent cancels the helper by terminating its process group.
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let result = list_archive_entries_direct(input, format, password.as_deref(), &cancelled);
    fs::write(output, encode_archive_result(&result)).map_err(|error| error.to_string())?;
    Ok(())
}

// Reopen to read from offset zero without changing the inherited description's offset.
fn read_secret_fd(descriptor: RawFd) -> Result<Vec<u8>, String> {
    let mut secret = Vec::new();
    fs::File::open(format!("/proc/self/fd/{descriptor}"))
        .and_then(|file| {
            file.take(crate::adapters::MAX_ARCHIVE_PASSWORD_BYTES as u64 + 1)
                .read_to_end(&mut secret)
        })
        .map_err(|error| format!("Unable to read the preview secret: {error}"))?;
    Ok(secret)
}

fn write_media_metadata(input: &Path, output: &Path) -> Result<(), String> {
    fs::write(output, read_media_metadata(input)?).map_err(|error| error.to_string())
}

fn svg_source(input: &Path) -> Option<String> {
    let mut file = fs::File::open(input).ok()?;
    let mut bytes = Vec::new();
    (&mut file).take(8192).read_to_end(&mut bytes).ok()?;
    let limit = crate::services::document_media::IMAGE_INPUT_LIMIT;
    if bytes.starts_with(b"\x1f\x8b") {
        let mut decompressed = Vec::new();
        flate2::read::GzDecoder::new(fs::File::open(input).ok()?)
            .take(limit + 1)
            .read_to_end(&mut decompressed)
            .ok()?;
        bytes = decompressed;
    } else if is_svg_head(&bytes) {
        (&mut file)
            .take(limit.saturating_sub(bytes.len() as u64) + 1)
            .read_to_end(&mut bytes)
            .ok()?;
    }
    if bytes.len() as u64 > limit || !is_svg_head(&bytes) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn is_svg_head(head: &[u8]) -> bool {
    let head = head.strip_prefix(b"\xef\xbb\xbf").unwrap_or(head);
    let Some(first) = head.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return false;
    };
    if head[first] != b'<' {
        return false;
    }
    head.windows(4).enumerate().any(|(index, window)| {
        window == b"<svg"
            && matches!(
                head.get(index + 4),
                None | Some(b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/')
            )
    })
}

fn video_stream_metadata(width: i32, height: i32) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({
        "streams": [{"codec_type": "video", "width": width, "height": height}]
    }))
}

fn read_media_metadata(input: &Path) -> Result<Vec<u8>, String> {
    if let Some(source) = svg_source(input)
        && let Some((width, height)) = document_media::svg_dimensions(&source)
    {
        return video_stream_metadata(width, height).map_err(|error| error.to_string());
    }
    let probe = bounded_output_with_timeout(
        Command::new("ffprobe")
            .args([
                "-v", "error", "-threads", "1", "-show_entries",
                "stream=codec_type,codec_name,width,height,duration,avg_frame_rate,r_frame_rate,sample_rate,channels,channel_layout,pix_fmt,color_transfer:stream_disposition=attached_pic:stream_side_data=rotation:stream_tags=language:format=duration,bit_rate:chapter=start_time,end_time:chapter_tags=title",
                "-of", "json",
            ])
            .arg(input),
        crate::sandbox::metadata::MAX_METADATA_BYTES,
        Duration::from_secs(4),
    );
    let bytes = match probe {
        Ok(Some(result)) if result.status.success() => result.stdout,
        _ => {
            let (_, width, height) = gdk_pixbuf::Pixbuf::file_info(input)
                .filter(|(_, width, height)| *width > 0 && *height > 0)
                .ok_or("Unable to inspect media")?;
            video_stream_metadata(width, height).map_err(|error| error.to_string())?
        }
    };
    Ok(bytes)
}

fn media_preview_size(value: &str) -> Result<MediaPreviewSize, String> {
    if value == "0" {
        return Ok(MediaPreviewSize::new(1280, 1280));
    }
    let (width, height) = value
        .split_once('x')
        .ok_or_else(|| "Invalid media preview dimensions".to_owned())?;
    let parse = |dimension: &str| {
        dimension
            .parse::<i32>()
            .map_err(|_| "Invalid media preview dimensions".to_owned())
    };
    Ok(MediaPreviewSize::new(parse(width)?, parse(height)?))
}

fn render_media(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let output_path = directory.path().join("thumbnail.png");
    let output = bounded_output(
        Command::new("ffmpegthumbnailer")
            .arg("-i")
            .arg(path)
            .arg("-o")
            .arg(&output_path)
            .arg("-s")
            .arg(size.to_string())
            .args(["-q", "8"]),
        MAX_OUTPUT_BYTES,
    );
    if !output.is_ok_and(|output| output.status.success()) {
        let fallback = bounded_output(
            Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-threads", "1", "-i"])
                .arg(path)
                .args([
                    "-an",
                    "-frames:v",
                    "1",
                    "-threads",
                    "1",
                    "-filter_threads",
                    "1",
                    "-vf",
                ])
                .arg(format!(
                    "thumbnail=10,scale={size}:{size}:force_original_aspect_ratio=decrease"
                ))
                .arg(&output_path),
            MAX_OUTPUT_BYTES,
        )
        .map_err(|error| error.to_string())?;
        if !fallback.status.success() {
            return Err("Unable to render media thumbnail".into());
        }
    }
    let file = fs::File::open(output_path).map_err(|error| error.to_string())?;
    let png = read_limited(file, MAX_OUTPUT_BYTES).map_err(|error| error.to_string())?;
    if png.is_empty() {
        return Err("Empty media thumbnail".into());
    }
    Ok(png)
}

#[cfg(test)]
mod tests;
