// SPDX-License-Identifier: MIT
//! PDF page rendering and its glyph text layer.

use std::{fs, path::Path};

use gio::prelude::*;

use crate::sandbox::PdfRenderSize;

pub(super) fn render_pdf_thumbnail(path: &Path, size: i32) -> Result<Vec<u8>, String> {
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
    document: &poppler::Document,
    requested_page: i32,
    size: PdfRenderSize,
) -> Result<PdfPageRender, String> {
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

pub(super) fn bounded_surface_dimensions(
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

pub(super) fn pdf_render_request(value: &str) -> Result<(i32, PdfRenderSize), String> {
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

fn open_preview_document(input: &Path, output: &Path) -> Result<poppler::Document, String> {
    let uri = gio::File::for_path(input).uri();
    poppler::Document::from_file(&uri, None).map_err(|error| {
        if error.matches(poppler::Error::Encrypted)
            && let Ok(bytes) =
                serde_json::to_vec(&crate::sandbox::PdfPreviewError::PasswordProtected)
        {
            let _ = fs::write(output.with_file_name("result.error"), bytes);
        }
        error.to_string()
    })
}

/// The page PNG, its `page pages` metadata and its optional text layer.
type PdfPreview = (Vec<u8>, String, Option<Vec<u8>>);

/// Renders the requested page of `page:WxH`, with its `page pages` metadata and text layer.
pub(super) fn preview(input: &Path, output: &Path, value: &str) -> Result<PdfPreview, String> {
    let (page, size) = pdf_render_request(value)?;
    let document = open_preview_document(input, output)?;
    let (png, page, pages, text_layer) = render_pdf_page(&document, page, size)?;
    Ok((png, format!("{page} {pages}"), text_layer))
}
