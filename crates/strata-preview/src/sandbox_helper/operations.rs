// SPDX-License-Identifier: MIT
//! The helper's file-producing operations, looked up by their argument name.

use std::{fs, os::fd::RawFd, path::Path};

use super::{
    appimage, archive_cover, document_media, image_conversion, media, model, pdf,
    raster::{render_camera_raw, render_camera_raw_preview, render_raw},
    raw_metadata, render_media, route, run_archive_list, write_media_metadata,
};
use crate::sandbox::{CoverFormat, DocumentFormat, MAX_OUTPUT_BYTES, MediaPreviewBackend};

/// One parsed `[operation, input, output, value, media_backend]` invocation.
pub(super) struct Request<'a> {
    pub(super) operation: &'a str,
    pub(super) input: &'a Path,
    pub(super) output: &'a Path,
    pub(super) value: &'a str,
    pub(super) media_backend: MediaPreviewBackend,
    pub(super) start_tick: u32,
    pub(super) secret_fd: Option<RawFd>,
}

impl Request<'_> {
    fn thumbnail_edge(&self) -> Result<i32, String> {
        let value = self
            .value
            .parse::<i32>()
            .map_err(|_| "Invalid preview helper size or page".to_owned())?;
        Ok(value.clamp(16, 256))
    }

    fn cover_format(&self) -> Result<CoverFormat, String> {
        Ok(CoverFormat::from_argument(self.value).ok_or("Invalid cover format")?)
    }

    fn sibling(&self, name: &str) -> std::path::PathBuf {
        self.output.with_file_name(name)
    }

    /// Leaves the message beside the output so the parent can show it verbatim.
    fn report<T>(&self, result: Result<T, String>) -> Result<T, String> {
        result.inspect_err(|message| {
            let _ = fs::write(self.sibling("result.error"), message.as_bytes());
        })
    }
}

/// An operation that writes its own output files.
type Write = fn(&Request<'_>) -> Result<(), String>;

const WRITES: &[(&[&str], Write)] = &[
    (&["inspect-image", "convert-image"], convert_image),
    (&["preview-media", "preview-audio"], preview_media),
    (&["audio-peaks"], |request| {
        media::run_peaks(request.input, request.output)
    }),
    (&["video-storyboard"], video_storyboard),
    (&["audio-tags"], |request| {
        write_output(request.output, media::audio_tags(request.input)?)
    }),
    (&["preview-workbook"], preview_workbook),
    (&["preview-document"], preview_document),
    (&["media-metadata"], |request| {
        write_media_metadata(request.input, request.output)
    }),
    (&["raw-metadata"], |request| {
        write_output(request.output, raw_metadata::read(request.input)?)
    }),
    (&["archive-list"], |request| {
        run_archive_list(
            request.input,
            request.output,
            request.value,
            request.secret_fd,
        )
    }),
];

fn find<T: Copy>(table: &[(&[&str], T)], operation: &str) -> Option<T> {
    table
        .iter()
        .find(|(names, _)| names.contains(&operation))
        .map(|(_, run)| *run)
}

fn write_output(output: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    fs::write(output, bytes).map_err(|error| error.to_string())
}

fn convert_image(request: &Request<'_>) -> Result<(), String> {
    image_conversion::run(
        request.input,
        request.output,
        request.operation == "convert-image",
    )
}

fn preview_media(request: &Request<'_>) -> Result<(), String> {
    let audio_only = request.operation == "preview-audio";
    media::run(
        request.input,
        request.output,
        request.value,
        request.media_backend,
        request.start_tick,
        audio_only,
    )
}

fn video_storyboard(request: &Request<'_>) -> Result<(), String> {
    let cell_edge = request
        .value
        .parse::<u32>()
        .map_err(|_| "Invalid storyboard cell size".to_owned())?;
    media::run_storyboard(request.input, request.output, cell_edge)
}

fn write_bounded_json(
    output: &Path,
    value: &impl serde::Serialize,
    exceeded: &str,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_OUTPUT_BYTES {
        return Err(exceeded.into());
    }
    fs::write(output, bytes).map_err(|e| e.to_string())
}

fn preview_workbook(request: &Request<'_>) -> Result<(), String> {
    let table = crate::services::table::read_workbook(request.input)?;
    write_bounded_json(request.output, &table, "Table output budget exceeded")
}

fn preview_document(request: &Request<'_>) -> Result<(), String> {
    let document =
        match DocumentFormat::from_argument(request.value).ok_or("Invalid document format")? {
            DocumentFormat::Docx => crate::services::docx::read_document(request.input)?,
            DocumentFormat::Odt => crate::services::odt::read_document(request.input)?,
        };
    write_bounded_json(request.output, &document, "Document output budget exceeded")
}

/// Runs an operation that writes its own files, when `request` names one.
pub(super) fn write(request: &Request<'_>) -> Option<Result<(), String>> {
    find(WRITES, request.operation).map(|write| write(request))
}

/// The PNG an operation produced, with the optional `result.meta` and `result.text` files.
#[derive(Default)]
pub(super) struct Rendered {
    png: Vec<u8>,
    metadata: Option<String>,
    text_layer: Option<Vec<u8>>,
}

impl From<Vec<u8>> for Rendered {
    fn from(png: Vec<u8>) -> Self {
        Self {
            png,
            ..Self::default()
        }
    }
}

impl Rendered {
    pub(super) fn write(self, output: &Path) -> Result<(), String> {
        write_output(output, self.png)?;
        if let Some(metadata) = self.metadata {
            write_output(&output.with_file_name("result.meta"), metadata)?;
        }
        if let Some(text_layer) = self.text_layer {
            write_output(&output.with_file_name("result.text"), text_layer)?;
        }
        Ok(())
    }
}

type Render = fn(&Request<'_>) -> Result<Rendered, String>;

const RENDERS: &[(&[&str], Render)] = &[
    (&["thumbnail-image"], |request| {
        Ok(render_raw(request.input, request.thumbnail_edge()?)?.into())
    }),
    (&["thumbnail-raw"], |request| {
        Ok(render_camera_raw(request.input, request.thumbnail_edge()?)?.into())
    }),
    (&["preview-raw"], |request| {
        Ok(render_camera_raw_preview(request.input, 800)?.into())
    }),
    (&["thumbnail-pdf"], |request| {
        Ok(pdf::render_pdf_thumbnail(request.input, request.thumbnail_edge()?)?.into())
    }),
    (&["thumbnail-video"], |request| {
        Ok(render_media(request.input, request.thumbnail_edge()?)?.into())
    }),
    (&["audio-cover"], audio_cover),
    (&["thumbnail-appimage"], |request| {
        Ok(appimage::render(request.input, request.thumbnail_edge()?)?.into())
    }),
    (&["thumbnail-model"], model_thumbnail),
    (&["thumbnail-cover"], |request| {
        Ok(archive_cover::thumbnail(request.input, request.cover_format()?)?.into())
    }),
    (&["preview-cover"], |request| {
        Ok(archive_cover::render(request.input, request.cover_format()?, 800)?.into())
    }),
    (&["preview-image", "document-image"], |request| {
        Ok(document_media::image(request.input, 800)?.into())
    }),
    (&["document-mermaid"], |request| {
        Ok(document_media::mermaid(request.input)?.into())
    }),
    (&["document-math"], |request| {
        Ok(document_media::math(request.input, true)?.into())
    }),
    (&["document-inline-math"], |request| {
        Ok(document_media::math(request.input, false)?.into())
    }),
    (&["preview-pdf"], preview_pdf),
    (&["preview-route"], |request| {
        Ok(request
            .report(route::render(request.input, request.value))?
            .into())
    }),
    (&["preview-model"], preview_model),
];

fn audio_cover(request: &Request<'_>) -> Result<Rendered, String> {
    let cover = media::cover(request.input, 800)?;
    Ok(if cover.is_empty() {
        b"null".to_vec()
    } else {
        cover
    }
    .into())
}

fn model_thumbnail(request: &Request<'_>) -> Result<Rendered, String> {
    let format =
        crate::services::ModelFormat::from_argument(request.value).ok_or("Invalid model format")?;
    Ok(model::thumbnail(request.input, format, (256, 256), &|_| {})?.into())
}

fn preview_pdf(request: &Request<'_>) -> Result<Rendered, String> {
    let (png, metadata, text_layer) = pdf::preview(request.input, request.output, request.value)?;
    Ok(Rendered {
        png,
        metadata: Some(metadata),
        text_layer,
    })
}

fn write_model_progress(output: &Path, stage: crate::services::ModelPreviewStage) {
    let Ok(bytes) = serde_json::to_vec(&stage) else {
        return;
    };
    let pending = output.with_file_name("result.progress.tmp");
    if fs::write(&pending, bytes).is_ok() {
        let _ = fs::rename(pending, output.with_file_name("result.progress"));
    }
}

fn preview_model(request: &Request<'_>) -> Result<Rendered, String> {
    let progress = |stage| write_model_progress(request.output, stage);
    let rendered = request.report(model::render_reporting(
        request.input,
        request.value,
        &progress,
    ))?;
    fs::write(
        request.sibling("result.model"),
        serde_json::to_vec(&rendered.mesh).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(rendered.png.into())
}

/// Renders the PNG-producing operation `request` names.
pub(super) fn render(request: &Request<'_>) -> Result<Rendered, String> {
    let render = find(RENDERS, request.operation)
        .ok_or_else(|| "Unknown preview helper operation".to_owned())?;
    render(request)
}
