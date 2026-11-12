// SPDX-License-Identifier: MIT
//! Renders for the file browser's thumbnail and metadata worker.

use std::path::Path;

use super::{
    document_media, model,
    pdf::render_pdf_thumbnail,
    raster::{
        exceeds_decoded_frame_budget, image_dimensions, render_camera_raw,
        render_camera_raw_preview, render_dcraw, render_imagemagick, render_oversized,
        render_pixbuf,
    },
    raw_metadata, read_media_metadata, render_media, svg_source, video_stream_metadata,
};
use crate::{
    sandbox::browser::wire::{Operation, Response},
    services::ModelFormat,
};

fn encode_dimensions((width, height): (i32, i32)) -> Vec<u8> {
    video_stream_metadata(width, height).unwrap_or_default()
}

fn raster_image(input: &Path, response: &mut Response) {
    let size = image_dimensions(input);
    let oversized = size.is_some_and(|(w, h)| exceeds_decoded_frame_budget(w, h));
    if let Some(size) = size {
        response.metadata = encode_dimensions(size);
        response.png = if oversized {
            render_oversized(input, 256).unwrap_or_default()
        } else {
            render_pixbuf(input, 256.min(size.0.max(size.1)), Some(size)).unwrap_or_default()
        };
    }
    if response.png.is_empty() && !oversized {
        response.png = render_imagemagick(input, 256)
            .or_else(|_| render_dcraw(input, 256))
            .unwrap_or_default();
    }
}

fn image(input: &Path, response: &mut Response) {
    // glycin's nested sandbox cannot run here; use resvg first.
    if let Some(source) = svg_source(input)
        && let Ok(rendered) = document_media::svg(&source, 256)
    {
        response.metadata = encode_dimensions((rendered.width, rendered.height));
        response.png = rendered.png;
    }
    if response.png.is_empty() {
        raster_image(input, response);
    }
}

fn image_metadata(input: &Path) -> Vec<u8> {
    svg_source(input)
        .and_then(|source| document_media::svg_dimensions(&source))
        .map(encode_dimensions)
        .or_else(|| image_dimensions(input).map(encode_dimensions))
        .or_else(|| read_media_metadata(input).ok())
        .unwrap_or_default()
}

fn metadata(input: &Path, operation: Operation) -> Option<Vec<u8>> {
    match operation {
        Operation::ImageMetadata => Some(image_metadata(input)),
        Operation::MediaMetadata => Some(read_media_metadata(input).unwrap_or_default()),
        Operation::RawMetadata => Some(raw_metadata::read(input).unwrap_or_default()),
        _ => None,
    }
}

fn model_format(operation: Operation) -> Option<ModelFormat> {
    match operation {
        Operation::ThreeMfThumbnail => Some(ModelFormat::ThreeMf),
        Operation::GcodeThumbnail => Some(ModelFormat::Gcode),
        Operation::FreeCadThumbnail => Some(ModelFormat::FreeCad),
        _ => None,
    }
}

fn thumbnail(input: &Path, operation: Operation) -> Option<Result<Vec<u8>, String>> {
    Some(match operation {
        Operation::Raw => render_camera_raw(input, 256),
        Operation::Pdf => render_pdf_thumbnail(input, 256),
        Operation::Video => render_media(input, 256),
        _ => {
            let format = model_format(operation)?;
            model::thumbnail(input, format, (256, 256), &|_| {})
        }
    })
}

fn preview(input: &Path, operation: Operation) -> Option<Result<Vec<u8>, String>> {
    Some(match operation {
        Operation::PreviewImage => document_media::image(input, 800),
        Operation::PreviewRaw => render_camera_raw_preview(input, 800),
        Operation::DocumentMermaid => document_media::mermaid(input),
        Operation::DocumentMath => document_media::math(input, true),
        Operation::DocumentMathInline => document_media::math(input, false),
        _ => return None,
    })
}

pub fn browser_render(input: &Path, operation: Operation) -> Response {
    let mut response = Response::default();
    if operation == Operation::Image {
        image(input, &mut response);
    } else if let Some(metadata) = metadata(input, operation) {
        response.metadata = metadata;
    } else if let Some(png) = thumbnail(input, operation).or_else(|| preview(input, operation)) {
        response.png = png.unwrap_or_default();
    }
    response
}
