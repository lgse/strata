// SPDX-License-Identifier: MIT
//! One preview request, carried through classification to the emitted event.

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
};

use gio::prelude::*;

use super::{
    PREVIEW_CACHE, PreviewCacheKey, pdf_render_size, remote, remote::StagedPreview,
    request_heavy_preview_permit,
};
use crate::{
    adapters::{
        gio_file_for_location,
        local_operations::{ArchiveListingStatus, decode_archive_listing},
    },
    model::{FileEntry, MetadataValue},
    sandbox::{Cancellation, DocumentFormat, MediaPreviewBackend, ParseOperation, ParseOutput},
    services::{
        ArchiveFormat, DocumentLayout, MediaPreviewSize, ModelFormat, ModelRender, Preview,
        PreviewContent, PreviewEvent, PreviewRequest, SandboxedMedia, content_family,
        document_kind, has_plain_text_extension, is_non_executable_extensionless_dotfile,
        layout_document, looks_like_text, parse_document, read_head_sample, reconcile_with_sample,
        route_preview::{RouteFormat, RoutePreview, RouteRender},
    },
};

const WORKER_STOPPED: &str = "The preview worker stopped unexpectedly.";

pub(super) trait RenderFn:
    FnOnce(
        &Path,
        ParseOperation,
        i32,
        MediaPreviewBackend,
        &Cancellation,
    ) -> Result<ParseOutput, String>
    + Send
    + 'static
{
}

impl<F> RenderFn for F where
    F: FnOnce(
            &Path,
            ParseOperation,
            i32,
            MediaPreviewBackend,
            &Cancellation,
        ) -> Result<ParseOutput, String>
        + Send
        + 'static
{
}

/// Parses and lays out a document source; the kind is fixed when the parser is made.
trait DocumentParser:
    FnOnce(&str, &Cancellation) -> Result<(DocumentLayout, Vec<String>), String> + Send + 'static
{
}

impl<F> DocumentParser for F where
    F: FnOnce(&str, &Cancellation) -> Result<(DocumentLayout, Vec<String>), String>
        + Send
        + 'static
{
}

/// Why a load ended before it produced a preview.
enum Stop {
    Failed(String),
    NeedsPassword,
    /// The load ends without an event, after a cancellation or a vanished worker.
    Silent,
}

type Step<T> = Result<T, Stop>;
type WorkerResult<T> = std::thread::Result<Result<T, String>>;
type Thumbnail = (PathBuf, i64, Vec<u8>);

/// A sandboxed render chosen for the entry, with the keys its cache entry needs.
struct RenderPlan {
    operation: ParseOperation,
    model: Option<ModelRender>,
    route: Option<RouteRender>,
}

impl RenderPlan {
    fn pdf_page(&self, page: i32) -> Option<(i32, crate::sandbox::PdfRenderSize)> {
        match self.operation {
            ParseOperation::PreviewPdf(size) => Some((page, size)),
            _ => None,
        }
    }

    fn is_heavy(&self) -> bool {
        matches!(
            self.operation,
            ParseOperation::PreviewPdf(_)
                | ParseOperation::PreviewModel(_)
                | ParseOperation::PreviewRoute(_)
                | ParseOperation::PreviewCover(_)
        )
    }
}

pub(super) struct PreviewLoad {
    request: PreviewRequest,
    entry: FileEntry,
    emit: Rc<dyn Fn(PreviewEvent)>,
    backend: MediaPreviewBackend,
    cancellation: Cancellation,
    abort_safe: Rc<Cell<bool>>,
    sample: Option<Vec<u8>>,
    content_type: String,
    content: PreviewContent,
    thumbnail: Option<Thumbnail>,
}

fn empty_text() -> PreviewContent {
    PreviewContent::Text {
        content: String::new(),
        truncated: false,
    }
}

fn plain_document(
    source: String,
    fallback_reason: Option<String>,
    truncated: bool,
) -> PreviewContent {
    PreviewContent::Document {
        source,
        document: None,
        fallback_reason,
        warnings: Vec::new(),
        truncated,
    }
}

fn remote_unsupported_message(operation: &ParseOperation) -> &'static str {
    if matches!(operation, ParseOperation::PreviewModel(_)) {
        "Remote model previews are not supported; copy the file locally first"
    } else if matches!(operation, ParseOperation::PreviewRoute(_)) {
        "Copy this route file locally before previewing it."
    } else if matches!(operation, ParseOperation::PreviewCover(_)) {
        "Remote cover previews are not supported; copy the file locally first"
    } else {
        "Remote PDF previews are not supported; copy the file locally first"
    }
}

impl PreviewLoad {
    pub(super) fn new(
        request: PreviewRequest,
        emit: Rc<dyn Fn(PreviewEvent)>,
        backend: MediaPreviewBackend,
        cancellation: Cancellation,
        abort_safe: Rc<Cell<bool>>,
    ) -> Self {
        Self {
            entry: request.entry.clone(),
            request,
            emit,
            backend,
            cancellation,
            abort_safe,
            sample: None,
            content_type: String::new(),
            content: PreviewContent::Unsupported,
            thumbnail: None,
        }
    }

    pub(super) async fn run(mut self, render: impl RenderFn) {
        let outcome = self.load(render).await;
        let request_id = self.request.id;
        let Self {
            entry,
            emit,
            content_type,
            content,
            thumbnail,
            ..
        } = self;
        match outcome {
            Ok(()) => {
                emit(PreviewEvent::Ready(Preview {
                    request_id,
                    entry,
                    content_type,
                    content,
                }));
                if let Some((path, mtime, png)) = thumbnail {
                    let _ = gio::spawn_blocking(move || {
                        crate::adapters::thumbnail_cache::store(&path, mtime, &png);
                    })
                    .await;
                }
            }
            Err(Stop::Failed(message)) => emit(PreviewEvent::Failed {
                request_id,
                entry,
                message,
            }),
            Err(Stop::NeedsPassword) => emit(PreviewEvent::NeedsPassword { request_id, entry }),
            Err(Stop::Silent) => (),
        }
    }

    async fn load(&mut self, render: impl RenderFn) -> Step<()> {
        self.read_sample().await?;
        self.classify().await?;
        if let Some(format) = self.archive_format() {
            return self.list_archive(format, render).await;
        }
        if let Some(operation) = self.sandboxed_operation() {
            return self.parse_sandboxed(operation, render).await;
        }
        let document_parser = self.mark_document();
        if matches!(self.content, PreviewContent::Media) {
            return self.load_media().await;
        }
        if let Some(plan) = self.render_plan() {
            return self.render(plan, render).await;
        }
        if matches!(
            self.content,
            PreviewContent::Text { .. } | PreviewContent::Document { .. }
        ) {
            self.read_document(document_parser).await?;
        }
        self.check_cancelled()
    }

    fn check_cancelled(&self) -> Step<()> {
        if self.cancellation.is_cancelled() {
            Err(Stop::Silent)
        } else {
            Ok(())
        }
    }

    /// A failed remote staging stays silent once the load was cancelled.
    fn staging_failure(&self, message: String) -> Stop {
        if self.cancellation.is_cancelled() {
            Stop::Silent
        } else {
            Stop::Failed(message)
        }
    }

    fn settle_worker<T>(&self, result: WorkerResult<T>) -> Step<T> {
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(message)) => Err(Stop::Failed(message)),
            Err(_) => {
                self.check_cancelled()?;
                Err(Stop::Failed(WORKER_STOPPED.to_owned()))
            }
        }
    }

    fn native_path(&self) -> Option<PathBuf> {
        self.entry.location.native_path().map(ToOwned::to_owned)
    }

    fn sample_is_text(&self) -> Option<bool> {
        self.sample.as_deref().map(looks_like_text)
    }

    fn is_unsupported(&self) -> bool {
        matches!(self.content, PreviewContent::Unsupported)
    }

    async fn read_sample(&mut self) -> Step<()> {
        let Some(path) = self.native_path() else {
            return Ok(());
        };
        self.sample = match gio::spawn_blocking(move || read_head_sample(&path)).await {
            Ok(Ok(sample)) => Some(sample),
            Ok(Err(error)) if crate::services::file_activity::is_refusal(&error) => {
                return Err(Stop::Failed(
                    crate::services::file_activity::HELD_FOR_OPERATION.to_owned(),
                ));
            }
            _ => None,
        };
        Ok(())
    }

    async fn classify(&mut self) -> Step<()> {
        let (content_type, uncertain) =
            crate::services::guess_content_type(Path::new(&self.entry.native_name));
        self.content_type = content_type;
        self.content = content_family(&self.content_type);
        let sandbox_only = crate::services::is_model(&self.entry)
            || RouteFormat::for_name(&self.entry.native_name).is_some();
        if sandbox_only {
            self.content = PreviewContent::Unsupported;
            return Ok(());
        }
        if self.is_unsupported() {
            self.adopt_plain_text_hint();
        }
        let name_is_vague = uncertain || self.entry.native_name.is_empty();
        if self.is_unsupported() && name_is_vague {
            self.query_content_type().await?;
        }
        if let Some(sample) = &self.sample {
            reconcile_with_sample(&mut self.content_type, &mut self.content, sample);
        }
        Ok(())
    }

    fn adopt_plain_text_hint(&mut self) {
        if gio::content_type_is_a(&self.content_type, "text/plain") {
            self.content = empty_text();
        } else if has_plain_text_extension(&self.entry.native_name) {
            self.content = empty_text();
            self.content_type = "text/plain".to_owned();
        }
    }

    fn reads_as_plain_text(&self, content_type: &str, unix_mode: Option<u32>) -> bool {
        gio::content_type_is_a(content_type, "text/plain")
            || has_plain_text_extension(&self.entry.native_name)
            || is_non_executable_extensionless_dotfile(&self.entry.native_name, unix_mode)
    }

    async fn query_content_type(&mut self) -> Step<()> {
        let file = gio_file_for_location(&self.entry.location);
        let info = file
            .query_info_future(
                &format!("{},unix::mode", crate::adapters::CONTENT_TYPE_ATTRIBUTES),
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await
            .map_err(|error| Stop::Failed(crate::services::gio_error_message(&error)))?;
        let queried_type = crate::adapters::content_type_of(&info)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_owned());
        let unix_mode = info
            .has_attribute(gio::FILE_ATTRIBUTE_UNIX_MODE)
            .then(|| info.attribute_uint32(gio::FILE_ATTRIBUTE_UNIX_MODE));
        let mut resolved = content_family(&queried_type);
        if matches!(resolved, PreviewContent::Unsupported)
            && self.reads_as_plain_text(&queried_type, unix_mode)
        {
            resolved = empty_text();
        }
        self.content = resolved;
        self.content_type = queried_type;
        Ok(())
    }

    fn archive_format(&self) -> Option<ArchiveFormat> {
        crate::services::archive_preview_format(&self.entry.native_name)
            .filter(|_| self.sample_is_text() != Some(true))
    }

    async fn list_archive(&mut self, format: ArchiveFormat, render: impl RenderFn) -> Step<()> {
        let Some(archive_path) = self.native_path() else {
            return Err(Stop::Failed(
                "Copy this archive locally before previewing it".into(),
            ));
        };
        let cancellation = self.cancellation.clone();
        let password = self.request.archive_password.clone();
        let listed = gio::spawn_blocking(move || {
            let output = render(
                &archive_path,
                ParseOperation::ArchiveList { format, password },
                0,
                MediaPreviewBackend::Software,
                &cancellation,
            )?;
            decode_archive_listing(&output.data).map(|listing| {
                let tree = crate::services::archive_preview_tree(listing.entries);
                (listing.status, tree)
            })
        })
        .await;
        self.check_cancelled()?;
        let (status, tree) = match listed {
            Ok(Ok(listed)) => listed,
            Ok(Err(message)) => return Err(Stop::Failed(message)),
            Err(_) => return Err(Stop::Silent),
        };
        match status {
            ArchiveListingStatus::Open => {
                self.content = PreviewContent::Archive { tree };
                Ok(())
            }
            ArchiveListingStatus::NeedsPassword => Err(Stop::NeedsPassword),
            ArchiveListingStatus::WrongPassword => Err(Stop::Failed(
                crate::services::INCORRECT_ARCHIVE_PASSWORD.to_owned(),
            )),
            ArchiveListingStatus::Unsupported => Err(Stop::Failed(
                crate::adapters::ARCHIVE_UNSUPPORTED_MESSAGE.to_owned(),
            )),
        }
    }

    fn sandboxed_operation(&self) -> Option<ParseOperation> {
        if crate::services::table::is_workbook(&self.content_type, &self.entry.native_name) {
            Some(ParseOperation::PreviewWorkbook)
        } else if let Some(format) =
            DocumentFormat::for_file(&self.content_type, &self.entry.native_name)
            && self.sample_is_text() != Some(true)
        {
            Some(ParseOperation::PreviewDocument(format))
        } else {
            None
        }
    }

    async fn parse_sandboxed(
        &mut self,
        operation: ParseOperation,
        render: impl RenderFn,
    ) -> Step<()> {
        let Some(path) = self.native_path() else {
            return Err(Stop::Failed(
                "Copy this file locally before previewing it".into(),
            ));
        };
        // Share the full-document parser slot with PDF rendering. Keep it
        // until the cancelled helper exits, not merely until the UI closes.
        let Some(permit) = request_heavy_preview_permit().acquire().await else {
            return Err(Stop::Silent);
        };
        self.check_cancelled()?;
        self.abort_safe.set(false);
        let cancellation = self.cancellation.clone();
        let backend = self.backend;
        let result = gio::spawn_blocking(move || {
            let is_workbook = matches!(operation, ParseOperation::PreviewWorkbook);
            let output = render(&path, operation, 0, backend, &cancellation)?;
            let parsed = if is_workbook {
                crate::services::table::TableData::from_json(&output.data)?.into_document()
            } else {
                crate::services::docx::RichTextData::from_json(&output.data)?
                    .into_document(&cancellation)?
            };
            layout_document(parsed.document, &cancellation).map(|document| {
                PreviewContent::Rendered {
                    document,
                    warnings: parsed.warnings,
                }
            })
        })
        .await;
        self.abort_safe.set(true);
        drop(permit);
        self.check_cancelled()?;
        self.content = self.settle_worker(result)?;
        Ok(())
    }

    /// Marks a document entry and returns its parser; other entries stay as they are.
    fn mark_document(&mut self) -> Option<impl DocumentParser> {
        let kind = document_kind(
            &self.content_type,
            &self.entry.native_name,
            self.entry.location.native_path().is_some(),
        )
        .filter(|_| self.sample.is_none() || matches!(self.content, PreviewContent::Text { .. }))?;
        self.content = plain_document(String::new(), None, false);
        Some(move |source: &str, cancellation: &Cancellation| {
            parse_document(kind, source, cancellation).and_then(|parsed| {
                layout_document(parsed.document, cancellation)
                    .map(|document| (document, parsed.warnings))
            })
        })
    }

    async fn stage_video(&self) -> Step<StagedPreview> {
        if !crate::services::supports_remote_video(&self.entry.native_name) {
            return Err(Stop::Failed(
                "Copy this media format locally before previewing it".into(),
            ));
        }
        remote::stage_video(&self.entry)
            .await
            .map_err(|message| self.staging_failure(message))
    }

    async fn load_media(&mut self) -> Step<()> {
        let staged = if self.entry.location.native_path().is_none() {
            Some(self.stage_video().await?)
        } else {
            None
        };
        let path = staged
            .as_ref()
            .map(|input| input.path())
            .or_else(|| self.entry.location.native_path())
            .expect("native or staged media")
            .to_path_buf();
        let mut media = SandboxedMedia {
            path,
            size: self.request.media_size,
            backend: self.backend,
            input_owner: None,
            audio_only: false,
        };
        if let Some(staged) = staged {
            media = media.retain_input(staged);
        }
        self.check_cancelled()?;
        self.content = PreviewContent::SandboxedMedia { media };
        Ok(())
    }

    fn model_render(&self) -> Option<ModelRender> {
        crate::services::model_format(&self.entry).map(|format| ModelRender {
            purpose: crate::services::ModelRenderPurpose::Preview,
            format,
            size: MediaPreviewSize::new(
                self.request.media_size.width.min(800),
                self.request.media_size.height.min(800),
            ),
            palette: self.request.model_palette,
            view: self.request.model_view,
        })
    }

    fn route_render(&self) -> Option<RouteRender> {
        RouteFormat::for_name(&self.entry.native_name).map(|format| RouteRender {
            format,
            palette: self.request.model_palette,
        })
    }

    fn content_operation(&self) -> Option<ParseOperation> {
        match self.content {
            PreviewContent::Pdf { .. } => Some(ParseOperation::PreviewPdf(pdf_render_size(
                self.request.media_size,
            ))),
            PreviewContent::Image
                if crate::sandbox::raw_metadata::is_raw(Path::new(&self.entry.native_name)) =>
            {
                Some(ParseOperation::PreviewRaw)
            }
            PreviewContent::Image => Some(ParseOperation::PreviewImage),
            PreviewContent::Media => None,
            PreviewContent::Text { .. }
            | PreviewContent::Model { .. }
            | PreviewContent::Route { .. }
            | PreviewContent::Document { .. }
            | PreviewContent::Rendered { .. }
            | PreviewContent::Rasterized { .. }
            | PreviewContent::SandboxedMedia { .. }
            | PreviewContent::Archive { .. }
            | PreviewContent::Unsupported => None,
        }
    }

    fn render_plan(&self) -> Option<RenderPlan> {
        let model = self.model_render();
        let route = self.route_render();
        let operation = if let Some(render) = route {
            Some(ParseOperation::PreviewRoute(render))
        } else if let Some(render) = model {
            Some(ParseOperation::PreviewModel(render))
        } else if let Some(format) = crate::sandbox::CoverFormat::for_name(&self.entry.native_name)
        {
            Some(ParseOperation::PreviewCover(format))
        } else {
            self.content_operation()
        };
        operation.map(|operation| RenderPlan {
            operation,
            model,
            route,
        })
    }

    async fn stage_render_input(&self, operation: &ParseOperation) -> Step<Option<StagedPreview>> {
        if self.entry.location.native_path().is_some() {
            return Ok(None);
        }
        if !matches!(
            operation,
            ParseOperation::PreviewImage | ParseOperation::PreviewRaw
        ) {
            return Err(Stop::Failed(remote_unsupported_message(operation).into()));
        }
        remote::stage(&self.entry)
            .await
            .map(Some)
            .map_err(|message| self.staging_failure(message))
    }

    /// Remote originals are not written to persistent thumbnail caches.
    fn cache_modified(&self, staged: bool) -> Option<i64> {
        match self.entry.modified_unix_seconds {
            _ if staged => None,
            MetadataValue::Known(m) => Some(m),
            MetadataValue::Unknown | MetadataValue::Unavailable => None,
        }
    }

    async fn acquire_heavy_permit(
        &self,
        plan: &RenderPlan,
    ) -> Step<Option<super::HeavyPreviewPermit>> {
        if !plan.is_heavy() {
            return Ok(None);
        }
        let Some(permit) = request_heavy_preview_permit().acquire().await else {
            return Err(Stop::Silent);
        };
        self.check_cancelled()?;
        Ok(Some(permit))
    }

    fn missing_gcode_thumbnail<E>(
        &self,
        rendered: &Result<Result<ParseOutput, String>, E>,
        plan: &RenderPlan,
    ) -> bool {
        matches!(
            (rendered, plan.model.map(|item| item.format)),
            (Ok(Err(message)), Some(ModelFormat::Gcode))
                if message == crate::services::model_preview::NO_GCODE_THUMBNAIL
                    && self.sample_is_text() == Some(true)
        )
    }

    fn decode_render(
        &mut self,
        rendered: WorkerResult<ParseOutput>,
        plan: &RenderPlan,
        path: &Path,
        modified: Option<i64>,
    ) -> Step<PreviewContent> {
        let output = self.settle_worker(rendered)?;
        if plan.route.is_some() {
            return RoutePreview::from_json(&output.data)
                .map(|preview| PreviewContent::Route { preview })
                .map_err(Stop::Failed);
        }
        if plan.model.is_some() {
            return Ok(PreviewContent::Model {
                png: output.data,
                mesh: output.model_mesh,
            });
        }
        if matches!(plan.operation, ParseOperation::PreviewPdf(_)) {
            if let Some(mtime) = modified
                && self.request.pdf_page == 0
            {
                self.thumbnail = Some((path.to_path_buf(), mtime, output.data.clone()));
            }
            return Ok(PreviewContent::Pdf {
                png: output.data,
                page: output.page,
                pages: output.pages,
                text_layer: output.text_layer.map(std::sync::Arc::new),
            });
        }
        if let Some(mtime) =
            modified.filter(|_| !matches!(plan.operation, ParseOperation::PreviewCover(_)))
        {
            self.thumbnail = Some((path.to_path_buf(), mtime, output.data.clone()));
        }
        Ok(PreviewContent::Rasterized { png: output.data })
    }

    async fn render(&mut self, plan: RenderPlan, render: impl RenderFn) -> Step<()> {
        let staged = self.stage_render_input(&plan.operation).await?;
        let path = staged
            .as_ref()
            .map(|file| file.path())
            .or_else(|| self.entry.location.native_path())
            .expect("native or staged preview input")
            .to_path_buf();
        let modified = self.cache_modified(staged.is_some());
        let cache_key = modified.map(|modified| PreviewCacheKey {
            path: path.clone(),
            modified,
            pdf_page: plan.pdf_page(self.request.pdf_page),
            model: plan.model,
            route: plan.route,
        });
        if let Some(cached) = cache_key
            .as_ref()
            .and_then(|key| PREVIEW_CACHE.with(|cache| cache.borrow_mut().get(key)))
        {
            self.content = cached;
            return Ok(());
        }
        self.check_cancelled()?;
        let heavy_permit = self.acquire_heavy_permit(&plan).await?;
        self.abort_safe.set(heavy_permit.is_none());
        let value = self.request.pdf_page;
        let cancellation = self.cancellation.clone();
        let backend = self.backend;
        let spawn_path = path.clone();
        let for_render = plan.operation.clone();
        let rendered = gio::spawn_blocking(move || {
            let _staged = staged;
            render(&spawn_path, for_render, value, backend, &cancellation)
        })
        .await;
        self.abort_safe.set(true);
        self.check_cancelled()?;
        self.content = if self.missing_gcode_thumbnail(&rendered, &plan) {
            let (content, truncated) = self.read_text().await?;
            PreviewContent::Text { content, truncated }
        } else {
            self.decode_render(rendered, &plan, &path, modified)?
        };
        drop(heavy_permit);
        if let Some(cache_key) = cache_key {
            PREVIEW_CACHE.with(|cache| {
                cache.borrow_mut().insert(cache_key, self.content.clone());
            });
        }
        Ok(())
    }

    async fn read_text(&self) -> Step<(String, bool)> {
        let file = gio_file_for_location(&self.entry.location);
        let native_path = self.native_path();
        super::read_text(&file, native_path.as_deref(), self.request.text_byte_limit)
            .await
            .map_err(|error| Stop::Failed(crate::services::gio_error_message(&error)))
    }

    async fn read_document(&mut self, parser: Option<impl DocumentParser>) -> Step<()> {
        let (source, truncated) = self.read_text().await?;
        self.content = match parser {
            Some(parser) => self.document_content(parser, source, truncated).await?,
            None => PreviewContent::Text {
                content: source,
                truncated,
            },
        };
        Ok(())
    }

    async fn document_content(
        &self,
        parser: impl DocumentParser,
        source: String,
        truncated: bool,
    ) -> Step<PreviewContent> {
        if truncated {
            let reason =
                "Rendered view is unavailable because the document exceeds the 1 MB preview limit.";
            return Ok(plain_document(source, Some(reason.to_owned()), truncated));
        }
        if !self.request.render_document {
            return Ok(plain_document(source, None, truncated));
        }
        let cancellation = self.cancellation.clone();
        let parsed = gio::spawn_blocking(move || {
            let parsed = parser(&source, &cancellation);
            (source, parsed)
        })
        .await;
        let (source, parsed) = match parsed {
            Ok(parsed) => parsed,
            Err(_) => {
                self.check_cancelled()?;
                return Err(Stop::Failed(WORKER_STOPPED.to_owned()));
            }
        };
        self.check_cancelled()?;
        Ok(match parsed {
            Ok((document, warnings)) => PreviewContent::Document {
                source,
                document: Some(document),
                fallback_reason: None,
                warnings,
                truncated,
            },
            Err(reason) => plain_document(source, Some(reason), truncated),
        })
    }
}
