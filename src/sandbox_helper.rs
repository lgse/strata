// SPDX-License-Identifier: MIT

use std::{
    fs,
    io::{self, Read, Seek},
    os::fd::RawFd,
    path::Path,
    process::{Child, Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use gdk_pixbuf::prelude::*;
use gtk::gio;

use crate::{
    adapters::{encode_archive_result, list_archive_entries_direct},
    sandbox::{FILE_SIZE_LIMIT_BYTES, MAX_OUTPUT_BYTES, MediaPreviewBackend, PdfRenderSize},
    services::{ArchiveFormat, MediaPreviewSize},
};

mod appimage;
mod document_media;
mod media;
mod raw_metadata;

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(20);

pub(crate) fn run(arguments: &[String]) -> Result<(), String> {
    let (arguments, start_tick) = match arguments {
        [operation, ..] if operation == "preview-media" && arguments.len() == 6 => (
            &arguments[..5],
            arguments[5]
                .parse::<u32>()
                .map_err(|_| "Invalid media seek position".to_owned())?,
        ),
        _ => (arguments, 0),
    };
    let secret_fd = match arguments {
        [operation, ..] if operation == "archive-list" && arguments.len() == 6 => Some(
            arguments[5]
                .parse::<RawFd>()
                .ok()
                .filter(|fd| *fd >= 0)
                .ok_or_else(|| "Invalid preview helper secret descriptor".to_owned())?,
        ),
        _ if arguments.len() == 5 => None,
        _ => return Err("Invalid preview helper arguments".to_owned()),
    };
    let [operation, input, output, value, media_backend] = &arguments[..5] else {
        return Err("Invalid preview helper arguments".to_owned());
    };
    let input = Path::new(input);
    let output = Path::new(output);
    let media_backend = MediaPreviewBackend::from_argument(media_backend)
        .ok_or_else(|| "Invalid media preview backend".to_owned())?;
    if operation == "preview-media" {
        return media::run(input, output, value, media_backend, start_tick);
    }
    if operation == "preview-workbook" {
        let table = crate::services::table::read_workbook(input)?;
        let bytes = serde_json::to_vec(&table).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_OUTPUT_BYTES {
            return Err("Table output budget exceeded".into());
        }
        return fs::write(output, bytes).map_err(|e| e.to_string());
    }
    if operation == "preview-document" {
        let document = crate::services::docx::read_document(input)?;
        let bytes = serde_json::to_vec(&document).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_OUTPUT_BYTES {
            return Err("Document output budget exceeded".into());
        }
        return fs::write(output, bytes).map_err(|e| e.to_string());
    }
    if operation == "media-metadata" {
        return write_media_metadata(input, output);
    }
    if operation == "raw-metadata" {
        return fs::write(output, raw_metadata::read(input)?).map_err(|error| error.to_string());
    }
    if operation == "archive-list" {
        return run_archive_list(input, output, value, secret_fd);
    }
    let numeric_value = || {
        value
            .parse::<i32>()
            .map_err(|_| "Invalid preview helper size or page".to_owned())
    };
    let mut text_layer_bytes = None;
    let (png, metadata) = match operation.as_str() {
        "thumbnail-image" => (render_raw(input, numeric_value()?.clamp(16, 256))?, None),
        "thumbnail-raw" => (
            render_raw_thumbnail(input, numeric_value()?.clamp(16, 256))?,
            None,
        ),
        "thumbnail-pdf" => (
            render_pdf_thumbnail(input, numeric_value()?.clamp(16, 256))?,
            None,
        ),
        "thumbnail-video" => (render_media(input, numeric_value()?.clamp(16, 256))?, None),
        "thumbnail-appimage" => (
            appimage::render(input, numeric_value()?.clamp(16, 256))?,
            None,
        ),
        "preview-image" | "document-image" => (document_media::image(input, 800)?, None),
        "document-mermaid" => (document_media::mermaid(input)?, None),
        "document-math" => (document_media::math(input, true)?, None),
        "document-inline-math" => (document_media::math(input, false)?, None),
        "preview-pdf" => {
            let (page, size) = pdf_render_request(value)?;
            let (png, page, pages, text_layer) = render_pdf_page(input, page, size)?;
            text_layer_bytes = text_layer;
            (png, Some(format!("{page} {pages}")))
        }
        _ => return Err("Unknown preview helper operation".to_owned()),
    };
    fs::write(output, png).map_err(|error| error.to_string())?;
    if let Some(metadata) = metadata {
        fs::write(output.with_file_name("result.meta"), metadata)
            .map_err(|error| error.to_string())?;
    }
    if let Some(text_layer) = text_layer_bytes {
        fs::write(output.with_file_name("result.text"), text_layer)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn run_archive_list(
    input: &Path,
    output: &Path,
    format: &str,
    secret_fd: Option<RawFd>,
) -> Result<(), String> {
    use crate::adapters::MAX_ARCHIVE_PASSWORD_BYTES;

    let format = match format {
        "zip" => ArchiveFormat::Zip,
        "7z" => ArchiveFormat::SevenZ,
        "tar" => ArchiveFormat::Tar,
        "tar.gz" => ArchiveFormat::TarGz,
        _ => return Err("Unknown archive format for preview.".to_owned()),
    };
    let password = match secret_fd {
        None => None,
        Some(descriptor) => {
            let secret = read_secret_fd(descriptor)?;
            if secret.len() > MAX_ARCHIVE_PASSWORD_BYTES {
                return Err("Archive password is too long.".to_owned());
            }
            Some(
                String::from_utf8(secret)
                    .map_err(|_| "Archive password is not valid text.".to_owned())?,
            )
        }
    };
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
                "stream=codec_type,codec_name,width,height,duration,avg_frame_rate,r_frame_rate,sample_rate,channels:stream_disposition=attached_pic:stream_side_data=rotation:format=duration,bit_rate",
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

pub(crate) fn browser_render(
    input: &Path,
    operation: crate::sandbox::browser::wire::Operation,
) -> crate::sandbox::browser::wire::Response {
    use crate::sandbox::browser::wire::{Operation, Response};
    let mut response = Response::default();
    let dimensions = || image_dimensions(input);
    let encode_dimensions =
        |(width, height)| video_stream_metadata(width, height).unwrap_or_default();
    match operation {
        Operation::Image => {
            // glycin's nested sandbox cannot run here; use resvg first.
            if let Some(source) = svg_source(input)
                && let Ok(rendered) = document_media::svg(&source, 256)
            {
                response.metadata = encode_dimensions((rendered.width, rendered.height));
                response.png = rendered.png;
            }
            if response.png.is_empty() {
                let size = dimensions();
                let oversized = size.is_some_and(|(w, h)| exceeds_decoded_frame_budget(w, h));
                if let Some(size) = size {
                    response.metadata = encode_dimensions(size);
                    if !oversized {
                        response.png =
                            render_pixbuf(input, 256.min(size.0.max(size.1))).unwrap_or_default();
                    } else if let Some(png) = read_exif_thumbnail(input, 256) {
                        response.png = png;
                    }
                }
                if response.png.is_empty() && !oversized {
                    response.png = render_imagemagick(input, 256)
                        .or_else(|_| render_dcraw(input, 256))
                        .unwrap_or_default();
                }
            }
        }
        Operation::Raw => response.png = render_raw_thumbnail(input, 256).unwrap_or_default(),
        Operation::Pdf => response.png = render_pdf_thumbnail(input, 256).unwrap_or_default(),
        Operation::Video => response.png = render_media(input, 256).unwrap_or_default(),
        Operation::ImageMetadata => {
            response.metadata = svg_source(input)
                .and_then(|source| document_media::svg_dimensions(&source))
                .map(encode_dimensions)
                .or_else(|| dimensions().map(encode_dimensions))
                .or_else(|| read_media_metadata(input).ok())
                .unwrap_or_default();
        }
        Operation::MediaMetadata => {
            response.metadata = read_media_metadata(input).unwrap_or_default()
        }
        Operation::PreviewImage => {
            response.png = document_media::image(input, 800).unwrap_or_default();
        }
        Operation::DocumentMermaid => {
            response.png = document_media::mermaid(input).unwrap_or_default();
        }
        Operation::DocumentMath => {
            response.png = document_media::math(input, true).unwrap_or_default();
        }
        Operation::DocumentMathInline => {
            response.png = document_media::math(input, false).unwrap_or_default();
        }
    }
    response
}

pub(crate) fn exceeds_decoded_frame_budget(width: i32, height: i32) -> bool {
    let width = u64::try_from(width).unwrap_or(0);
    let height = u64::try_from(height).unwrap_or(0);
    const RGBA_CHANNELS: u64 = 4;
    width.saturating_mul(height).saturating_mul(RGBA_CHANNELS) > FILE_SIZE_LIMIT_BYTES
}

fn read_jpeg_dimensions<R: io::Read + io::Seek>(reader: &mut R) -> Option<(i32, i32)> {
    let mut header = [0u8; 2];
    reader.read_exact(&mut header).ok()?;
    if header != [0xFF, 0xD8] {
        return None;
    }
    let mut byte = [0u8; 1];
    loop {
        loop {
            reader.read_exact(&mut byte).ok()?;
            if byte[0] == 0xFF {
                break;
            }
        }
        loop {
            reader.read_exact(&mut byte).ok()?;
            if byte[0] != 0xFF {
                break;
            }
        }
        let marker = byte[0];
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        if (0xD0..=0xD8).contains(&marker) || marker == 0x01 {
            continue;
        }
        let mut len_buf = [0u8; 2];
        reader.read_exact(&mut len_buf).ok()?;
        let length = u16::from_be_bytes(len_buf) as usize;
        if length < 2 {
            return None;
        }
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let mut sof_buf = [0u8; 5];
            reader.read_exact(&mut sof_buf).ok()?;
            let height = i32::from(u16::from_be_bytes([sof_buf[1], sof_buf[2]]));
            let width = i32::from(u16::from_be_bytes([sof_buf[3], sof_buf[4]]));
            if width > 0 && height > 0 {
                return Some((width, height));
            }
            return None;
        }
        reader
            .seek(io::SeekFrom::Current((length - 2) as i64))
            .ok()?;
    }
}

fn read_png_dimensions<R: io::Read>(reader: &mut R) -> Option<(i32, i32)> {
    let mut buf = [0u8; 24];
    reader.read_exact(&mut buf).ok()?;
    if &buf[0..8] != b"\x89PNG\r\n\x1a\n" || &buf[12..16] != b"IHDR" {
        return None;
    }
    let width = i32::try_from(u32::from_be_bytes(buf[16..20].try_into().ok()?)).ok()?;
    let height = i32::try_from(u32::from_be_bytes(buf[20..24].try_into().ok()?)).ok()?;
    if width > 0 && height > 0 {
        Some((width, height))
    } else {
        None
    }
}

fn read_gif_dimensions<R: io::Read>(reader: &mut R) -> Option<(i32, i32)> {
    let mut buf = [0u8; 10];
    reader.read_exact(&mut buf).ok()?;
    if &buf[0..6] != b"GIF87a" && &buf[0..6] != b"GIF89a" {
        return None;
    }
    let width = i32::from(u16::from_le_bytes([buf[6], buf[7]]));
    let height = i32::from(u16::from_le_bytes([buf[8], buf[9]]));
    if width > 0 && height > 0 {
        Some((width, height))
    } else {
        None
    }
}

fn image_dimensions(path: &Path) -> Option<(i32, i32)> {
    if let Ok(file) = fs::File::open(path) {
        let mut reader = io::BufReader::new(file);
        if let Some(dimensions) = read_jpeg_dimensions(&mut reader) {
            return Some(dimensions);
        }
        let _ = reader.seek(io::SeekFrom::Start(0));
        if let Some(dimensions) = read_png_dimensions(&mut reader) {
            return Some(dimensions);
        }
        let _ = reader.seek(io::SeekFrom::Start(0));
        if let Some(dimensions) = read_gif_dimensions(&mut reader) {
            return Some(dimensions);
        }
    }
    gdk_pixbuf::Pixbuf::file_info(path)
        .filter(|(_, width, height)| *width > 0 && *height > 0)
        .map(|(_, width, height)| (width, height))
}

fn render_pixbuf(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    if let Some((width, height)) = image_dimensions(path)
        && exceeds_decoded_frame_budget(width, height)
    {
        return Err("Image dimensions exceed the decoded frame budget".to_owned());
    }
    gdk_pixbuf::Pixbuf::from_file_at_scale(path, size, size, true)
        .map_err(|error| error.to_string())?
        .save_to_bufferv("png", &[("compression", "1")])
        .map_err(|error| error.to_string())
}

fn render_raw(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    // Preserve small sources so the preview can bound upscaling by their native dimensions.
    let info = image_dimensions(path);
    if let Some((width, height)) = info
        && width > 0
        && height > 0
    {
        if exceeds_decoded_frame_budget(width, height) {
            if let Some(png) = read_exif_thumbnail(path, size) {
                return Ok(png);
            }
            return Err("Image dimensions exceed the decoded frame budget".to_owned());
        }
        return render_pixbuf(path, size.min(width.max(height)))
            .or_else(|_| render_imagemagick(path, size))
            .or_else(|_| render_dcraw(path, size));
    }
    render_imagemagick(path, size).or_else(|_| render_dcraw(path, size))
}

fn render_raw_thumbnail(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    if let Some((width, height)) = image_dimensions(path)
        && exceeds_decoded_frame_budget(width, height)
    {
        return read_exif_thumbnail(path, size)
            .or_else(|| render_dcraw(path, size).ok())
            .ok_or_else(|| "Image dimensions exceed the decoded frame budget".to_owned());
    }
    // Prefer the camera JPEG so ImageMagick does not demosaic the list thumbnail.
    render_dcraw(path, size)
        .or_else(|_| render_pixbuf(path, size))
        .or_else(|_| render_imagemagick(path, size))
}

fn read_exif_thumbnail(path: &Path, size: i32) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    let mut reader = io::BufReader::new(file);
    let exif = exif::Reader::new()
        .continue_on_error(true)
        .read_from_container(&mut reader)
        .or_else(|error| error.distill_partial_result(|_| {}))
        .ok()?;
    let offset = exif
        .get_field(exif::Tag::JPEGInterchangeFormat, exif::In::THUMBNAIL)
        .and_then(|field| field.value.get_uint(0))? as usize;
    let len = exif
        .get_field(exif::Tag::JPEGInterchangeFormatLength, exif::In::THUMBNAIL)
        .and_then(|field| field.value.get_uint(0))? as usize;
    let end = offset.checked_add(len)?;
    let data = exif.buf().get(offset..end)?;
    scale_embedded_thumbnail(data, size).ok()
}

fn render_imagemagick(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    for executable in ["magick", "convert"] {
        let output = bounded_output(
            Command::new(executable)
                .arg(path)
                .args(["-auto-orient", "-thumbnail"])
                .arg(format!("{size}x{size}>"))
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
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let staging = directory.path().join("raw-thumb");
    symlink(path, &staging).map_err(|error| error.to_string())?;
    let status = Command::new("simple_dcraw")
        .arg("-e")
        .arg(&staging)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("simple_dcraw failed".to_owned());
    }
    for thumb in ["raw-thumb.thumb.jpg", "raw-thumb.thumb.ppm"] {
        let Ok(file) = fs::File::open(directory.path().join(thumb)) else {
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
    let dimensions = read_jpeg_dimensions(&mut io::Cursor::new(data))
        .or_else(|| read_png_dimensions(&mut io::Cursor::new(data)))
        .or_else(|| read_gif_dimensions(&mut io::Cursor::new(data)));
    if dimensions.is_some_and(|(width, height)| exceeds_decoded_frame_budget(width, height)) {
        return Err("Embedded thumbnail exceeds the decoded frame budget".to_owned());
    }
    scale_embedded_thumbnail_pixbuf(data, size).or_else(|_| render_imagemagick_bytes(data, size))
}

fn scale_embedded_thumbnail_pixbuf(data: &[u8], size: i32) -> Result<Vec<u8>, String> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader
        .write(data)
        .and_then(|()| loader.close())
        .map_err(|error| error.to_string())?;
    let pixbuf = loader
        .pixbuf()
        .ok_or_else(|| "Unable to decode embedded thumbnail".to_owned())?;
    let width = pixbuf.width().max(1);
    let height = pixbuf.height().max(1);
    let scale = (f64::from(size) / f64::from(width))
        .min(f64::from(size) / f64::from(height))
        .min(1.0);
    let target_width = (f64::from(width) * scale).round().max(1.0) as i32;
    let target_height = (f64::from(height) * scale).round().max(1.0) as i32;
    let pixbuf = if target_width == width && target_height == height {
        pixbuf
    } else {
        pixbuf
            .scale_simple(
                target_width,
                target_height,
                gdk_pixbuf::InterpType::Bilinear,
            )
            .ok_or_else(|| "Unable to scale embedded thumbnail".to_owned())?
    };
    pixbuf
        .save_to_bufferv("png", &[("compression", "1")])
        .map_err(|error| error.to_string())
}

fn render_imagemagick_bytes(data: &[u8], size: i32) -> Result<Vec<u8>, String> {
    use std::io::Write;
    if data.len() as u64 > MAX_OUTPUT_BYTES {
        return Err("Embedded thumbnail exceeds the input budget".to_owned());
    }
    let mut input = tempfile::NamedTempFile::new().map_err(|error| error.to_string())?;
    input.write_all(data).map_err(|error| error.to_string())?;
    render_imagemagick(input.path(), size)
}

fn render_pdf_thumbnail(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let uri = gio::File::for_path(path).uri();
    let document = poppler::Document::from_file(&uri, None).map_err(|error| error.to_string())?;
    let page = document
        .page(0)
        .ok_or_else(|| "This PDF has no pages".to_owned())?;
    let (page_width, page_height) = page.size();
    if page_width <= 0.0 || page_height <= 0.0 {
        return Err("The PDF page has invalid dimensions".to_owned());
    }
    let (width, height, scale) = bounded_surface_dimensions(
        page_width,
        page_height,
        f64::from(size),
        f64::from(size),
        f64::from(size * size),
    );
    render_pdf_surface(&page, width, height, scale)
}

type PdfPageRender = (Vec<u8>, i32, i32, Option<Vec<u8>>);

fn render_pdf_page(
    path: &Path,
    requested_page: i32,
    size: PdfRenderSize,
) -> Result<PdfPageRender, String> {
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
    let size = PdfRenderSize::new(size.width, size.height);
    let (_, _, max_pixels) = size.image_limits();
    let (page_width, page_height) = page.size();
    if page_width <= 0.0 || page_height <= 0.0 {
        return Err("The PDF page has invalid dimensions".to_owned());
    }
    let (width, height, scale) = bounded_surface_dimensions(
        page_width,
        page_height,
        f64::from(size.width),
        f64::from(size.height),
        max_pixels as f64,
    );
    let png = render_pdf_surface(&page, width, height, scale)?;
    let text_layer = pdf_text_layer(&page, width, height, scale);
    Ok((png, page_index, pages, text_layer))
}

// poppler-rs does not bind poppler_page_get_text_layout, so the glyph boxes come
// through FFI; the returned array is g_malloc'd and freed here.
#[expect(
    unsafe_code,
    reason = "poppler-rs exposes no safe binding for poppler_page_get_text_layout"
)]
fn pdf_text_layer(page: &poppler::Page, width: i32, height: i32, scale: f64) -> Option<Vec<u8>> {
    use glib::translate::ToGlibPtr;

    let text = page.text()?;
    if text.is_empty() {
        return None;
    }
    let mut rects = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: page is a valid PopplerPage; rects/count are valid out-pointers.
    let ok = unsafe {
        poppler::ffi::poppler_page_get_text_layout(page.to_glib_none().0, &mut rects, &mut count)
    };
    if ok == glib::ffi::GFALSE || rects.is_null() {
        return None;
    }
    // SAFETY: on success poppler returned a g_malloc'd array of count rectangles.
    let layout = unsafe { std::slice::from_raw_parts(rects, count as usize) };
    let glyphs: Vec<[f32; 4]> = layout
        .iter()
        .map(|rect| {
            [
                (rect.x1 * scale) as f32,
                (rect.y1 * scale) as f32,
                (rect.x2 * scale) as f32,
                (rect.y2 * scale) as f32,
            ]
        })
        .collect();
    // SAFETY: rects came from g_malloc and is freed exactly once here.
    unsafe { glib::ffi::g_free(rects.cast()) };
    if glyphs.len() != text.chars().count() {
        return None;
    }
    let layer = crate::services::PdfTextLayer {
        width: width as f32,
        height: height as f32,
        text: text.to_string(),
        glyphs,
    };
    serde_json::to_vec(&layer)
        .ok()
        .filter(|bytes| bytes.len() as u64 <= crate::sandbox::MAX_TEXT_LAYER_BYTES)
}

fn render_pdf_surface(
    page: &poppler::Page,
    width: i32,
    height: i32,
    scale: f64,
) -> Result<Vec<u8>, String> {
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

fn pdf_render_request(value: &str) -> Result<(i32, PdfRenderSize), String> {
    let (page, dimensions) = value
        .split_once(':')
        .ok_or_else(|| "Invalid PDF preview request".to_owned())?;
    let page = page
        .parse::<i32>()
        .map_err(|_| "Invalid PDF preview page".to_owned())?;
    let (width, height) = dimensions
        .split_once('x')
        .ok_or_else(|| "Invalid PDF preview dimensions".to_owned())?;
    let parse = |dimension: &str| {
        dimension
            .parse::<i32>()
            .map_err(|_| "Invalid PDF preview dimensions".to_owned())
    };
    Ok((page, PdfRenderSize::new(parse(width)?, parse(height)?)))
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
        .stderr(Stdio::inherit())
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
