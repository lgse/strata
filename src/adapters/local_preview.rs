// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
};

mod remote;

use futures_channel::oneshot;
use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::{
        gio_file_for_location,
        local_operations::{ArchiveListingStatus, decode_archive_listing},
    },
    sandbox::{Cancellation, MediaPreviewBackend, ParseOperation, PdfRenderSize},
    services::{
        LoadHandle, MediaPreviewSize, Preview, PreviewContent, PreviewEvent, PreviewProvider,
        PreviewRequest, SandboxedMedia, content_family, document_kind, has_plain_text_extension,
        is_non_executable_extensionless_dotfile, layout_document, normalize_preview_text,
        parse_document,
    },
};

const MAX_PREVIEW_CACHE_ENTRIES: usize = 64;
const MAX_PREVIEW_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_CONCURRENT_PDF_RENDERS: usize = 1;

#[derive(Default)]
struct PdfRenderQueue {
    running: usize,
    next_id: u64,
    queued: VecDeque<(u64, oneshot::Sender<PdfRenderPermit>)>,
}

struct PdfRenderWaiter {
    id: u64,
    receive: Option<oneshot::Receiver<PdfRenderPermit>>,
}

impl PdfRenderWaiter {
    async fn acquire(mut self) -> Option<PdfRenderPermit> {
        self.receive.take()?.await.ok()
    }
}

impl Drop for PdfRenderWaiter {
    fn drop(&mut self) {
        PDF_RENDER_QUEUE.with(|queue| {
            queue.borrow_mut().queued.retain(|(id, _)| *id != self.id);
        });
    }
}

struct PdfRenderPermit;

impl Drop for PdfRenderPermit {
    fn drop(&mut self) {
        release_pdf_render_permit();
    }
}

fn request_pdf_render_permit() -> PdfRenderWaiter {
    let (send, receive) = oneshot::channel();
    let (id, start) = PDF_RENDER_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        let id = queue.next_id;
        queue.next_id = queue.next_id.saturating_add(1);
        if queue.running < MAX_CONCURRENT_PDF_RENDERS {
            queue.running += 1;
            (id, Some(send))
        } else {
            queue.queued.push_back((id, send));
            (id, None)
        }
    });
    if let Some(send) = start
        && let Err(permit) = send.send(PdfRenderPermit)
    {
        drop(permit);
    }
    PdfRenderWaiter {
        id,
        receive: Some(receive),
    }
}

fn release_pdf_render_permit() {
    let next = PDF_RENDER_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        queue.running = queue.running.saturating_sub(1);
        let next = queue.queued.pop_front().map(|(_, send)| send);
        if next.is_some() {
            queue.running += 1;
        }
        next
    });
    if let Some(send) = next
        && let Err(permit) = send.send(PdfRenderPermit)
    {
        drop(permit);
    }
}

struct PreviewCache {
    entries: HashMap<PreviewCacheKey, PreviewContent>,
    recent: VecDeque<PreviewCacheKey>,
    byte_count: usize,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct PreviewCacheKey {
    path: PathBuf,
    modified: i64,
    pdf_page: Option<(i32, PdfRenderSize)>,
}

impl PreviewCache {
    fn get(&mut self, key: &PreviewCacheKey) -> Option<PreviewContent> {
        let content = self.entries.get(key)?.clone();
        self.recent.retain(|k| k != key);
        self.recent.push_back(key.clone());
        Some(content)
    }

    fn insert(&mut self, key: PreviewCacheKey, content: PreviewContent) {
        if matches!(content, PreviewContent::SandboxedMedia { .. }) {
            return;
        }
        let bytes = preview_content_size(&content);
        self.recent.retain(|k| k != &key);
        if let Some(old) = self.entries.remove(&key) {
            self.byte_count = self.byte_count.saturating_sub(preview_content_size(&old));
        }
        self.byte_count = self.byte_count.saturating_add(bytes);
        self.recent.push_back(key.clone());
        self.entries.insert(key, content);
        while self.entries.len() > MAX_PREVIEW_CACHE_ENTRIES
            || self.byte_count > MAX_PREVIEW_CACHE_BYTES
        {
            let Some(oldest) = self.recent.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.byte_count = self
                    .byte_count
                    .saturating_sub(preview_content_size(&removed));
            }
        }
    }
}

fn preview_content_size(content: &PreviewContent) -> usize {
    match content {
        PreviewContent::Rasterized { png } => png.len(),
        PreviewContent::Pdf {
            png, text_layer, ..
        } => {
            png.len()
                + text_layer.as_ref().map_or(0, |layer| {
                    layer.text.len() + layer.glyphs.len() * size_of::<[f32; 4]>()
                })
        }
        PreviewContent::SandboxedMedia { .. } => 0,
        PreviewContent::Text { content, .. } => content.len(),
        _ => 0,
    }
}

thread_local! {
    static PDF_RENDER_QUEUE: RefCell<PdfRenderQueue> = RefCell::new(PdfRenderQueue::default());
    static PREVIEW_CACHE: RefCell<PreviewCache> = RefCell::new(PreviewCache {
        entries: HashMap::new(),
        recent: VecDeque::new(),
        byte_count: 0,
    });
}

pub struct LocalPreviewProvider {
    media_preview_backend: Rc<dyn Fn() -> MediaPreviewBackend>,
}

impl LocalPreviewProvider {
    pub(crate) fn new(media_preview_backend: Rc<dyn Fn() -> MediaPreviewBackend>) -> Self {
        Self {
            media_preview_backend,
        }
    }
}

impl PreviewProvider for LocalPreviewProvider {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        self.load_with_renderer(request, emit, crate::sandbox::parse)
    }
}

impl LocalPreviewProvider {
    fn load_with_renderer(
        &self,
        request: PreviewRequest,
        emit: Rc<dyn Fn(PreviewEvent)>,
        render: impl FnOnce(
            &Path,
            ParseOperation,
            i32,
            MediaPreviewBackend,
            &Cancellation,
        ) -> Result<crate::sandbox::ParseOutput, String>
        + Send
        + 'static,
    ) -> LoadHandle {
        let media_preview_backend = (self.media_preview_backend)();
        let request_id = request.id;
        let entry = request.entry.clone();
        let cancellation = Cancellation::default();
        let cancellation_for_task = cancellation.clone();
        let abort_safe = Rc::new(Cell::new(true));
        let abort_safe_for_task = abort_safe.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            let (guessed_type, uncertain) =
                gio::content_type_guess(Some(Path::new(&entry.native_name)), None::<&[u8]>);
            let mut content_type = guessed_type.to_string();
            let mut content = content_family(&content_type);

            if matches!(content, PreviewContent::Unsupported)
                && has_plain_text_extension(&entry.native_name)
            {
                content = PreviewContent::Text {
                    content: String::new(),
                    truncated: false,
                };
                content_type = "text/plain".to_owned();
            }

            if matches!(content, PreviewContent::Unsupported)
                && (uncertain || entry.native_name.is_empty())
            {
                let file = gio_file_for_location(&entry.location);
                let info = match file
                    .query_info_future(
                        "standard::content-type,unix::mode",
                        gio::FileQueryInfoFlags::NONE,
                        glib::Priority::DEFAULT,
                    )
                    .await
                {
                    Ok(info) => info,
                    Err(error) => {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message: error.to_string(),
                        });
                        return;
                    }
                };
                let queried_type = info
                    .content_type()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "application/octet-stream".to_owned());
                let unix_mode = info
                    .has_attribute(gio::FILE_ATTRIBUTE_UNIX_MODE)
                    .then(|| info.attribute_uint32(gio::FILE_ATTRIBUTE_UNIX_MODE));
                let mut resolved = content_family(&queried_type);
                if matches!(resolved, PreviewContent::Unsupported)
                    && (gio::content_type_is_a(&queried_type, "text/plain")
                        || has_plain_text_extension(&entry.native_name)
                        || is_non_executable_extensionless_dotfile(&entry.native_name, unix_mode))
                {
                    resolved = PreviewContent::Text {
                        content: String::new(),
                        truncated: false,
                    };
                }
                content = resolved;
                content_type = queried_type;
            }

            if let Some(format) = crate::services::archive_preview_format(&entry.native_name) {
                let archive_path = match entry.location.native_path() {
                    Some(path) => path.to_path_buf(),
                    None => {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message: "Copy this archive locally before previewing it".into(),
                        });
                        return;
                    }
                };
                let cancellation = cancellation_for_task.clone();
                let password = request.archive_password.clone();
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
                if cancellation_for_task.is_cancelled() {
                    return;
                }
                match listed {
                    Ok(Ok((status, tree))) => match status {
                        ArchiveListingStatus::Open => {
                            emit(PreviewEvent::Ready(Preview {
                                request_id,
                                entry,
                                content_type,
                                content: PreviewContent::Archive { tree },
                            }));
                            return;
                        }
                        ArchiveListingStatus::NeedsPassword => {
                            emit(PreviewEvent::NeedsPassword { request_id, entry });
                            return;
                        }
                        ArchiveListingStatus::WrongPassword => {
                            emit(PreviewEvent::Failed {
                                request_id,
                                entry,
                                message: crate::services::INCORRECT_ARCHIVE_PASSWORD.to_owned(),
                            });
                            return;
                        }
                        ArchiveListingStatus::Unsupported => {
                            emit(PreviewEvent::Failed {
                                request_id,
                                entry,
                                message:
                                    crate::adapters::ARCHIVE_UNSUPPORTED_MESSAGE.to_owned(),
                            });
                            return;
                        }
                    },
                    Ok(Err(message)) => {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message,
                        });
                        return;
                    }
                    Err(_) => return,
                }
            }

            let sandboxed = if crate::services::table::is_workbook(&content_type, &entry.native_name)
            {
                Some(ParseOperation::PreviewWorkbook)
            } else if crate::services::docx::is_document(&content_type, &entry.native_name) {
                Some(ParseOperation::PreviewDocument)
            } else {
                None
            };
            if let Some(operation) = sandboxed {
                let Some(path) = entry.location.native_path().map(ToOwned::to_owned) else {
                    emit(PreviewEvent::Failed { request_id, entry, message: "Copy this file locally before previewing it".into() });
                    return;
                };
                // Share the full-document parser slot with PDF rendering. Keep it
                // until the cancelled helper exits, not merely until the UI closes.
                let Some(permit) = request_pdf_render_permit().acquire().await else {
                    return;
                };
                if cancellation_for_task.is_cancelled() {
                    return;
                }
                abort_safe_for_task.set(false);
                let cancellation = cancellation_for_task.clone();
                let result = gio::spawn_blocking(move || {
                    let is_workbook = matches!(operation, ParseOperation::PreviewWorkbook);
                    let output = render(&path, operation, 0, media_preview_backend, &cancellation)?;
                    let parsed = if is_workbook {
                        crate::services::table::TableData::from_json(&output.data)?.into_document()
                    } else {
                        crate::services::docx::RichTextData::from_json(&output.data)?.into_document(&cancellation)?
                    };
                    layout_document(parsed.document, &cancellation).map(|document| PreviewContent::Rendered { document, warnings: parsed.warnings })
                }).await;
                abort_safe_for_task.set(true);
                drop(permit);
                if cancellation_for_task.is_cancelled() { return; }
                match result {
                    Ok(Ok(content)) => emit(PreviewEvent::Ready(Preview { request_id, entry, content_type, content })),
                    Ok(Err(message)) => emit(PreviewEvent::Failed { request_id, entry, message }),
                    Err(_) => emit(PreviewEvent::Failed { request_id, entry, message: "The preview worker stopped unexpectedly.".into() }),
                }
                return;
            }

            let document_kind = document_kind(
                &content_type,
                &entry.native_name,
                entry.location.native_path().is_some(),
            );
            if document_kind.is_some() {
                content = PreviewContent::Document {
                    source: String::new(),
                    document: None,
                    fallback_reason: None,
                    warnings: Vec::new(),
                    truncated: false,
                };
            }

            if matches!(content, PreviewContent::Media) {
                let staged = if entry.location.native_path().is_none() {
                    if !crate::services::supports_remote_video(&entry.native_name) {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message: "Copy this media format locally before previewing it".into(),
                        });
                        return;
                    }
                    match remote::stage_video(&entry).await {
                        Ok(staged) => Some(staged),
                        Err(message) => {
                            if !cancellation_for_task.is_cancelled() {
                                emit(PreviewEvent::Failed {
                                    request_id,
                                    entry,
                                    message,
                                });
                            }
                            return;
                        }
                    }
                } else {
                    None
                };
                let path = staged
                    .as_ref()
                    .map(|input| input.path())
                    .or_else(|| entry.location.native_path())
                    .expect("native or staged media")
                    .to_path_buf();
                let mut media = SandboxedMedia {
                    path,
                    size: request.media_size,
                    backend: media_preview_backend,
                    input_owner: None,
                };
                if let Some(staged) = staged {
                    media = media.retain_input(staged);
                }
                if cancellation_for_task.is_cancelled() {
                    return;
                }
                emit(PreviewEvent::Ready(Preview {
                    request_id,
                    entry,
                    content_type,
                    content: PreviewContent::SandboxedMedia { media },
                }));
                return;
            }

            let operation = match content {
                PreviewContent::Pdf { .. } => Some(ParseOperation::PreviewPdf(pdf_render_size(
                    request.media_size,
                ))),
                PreviewContent::Image => Some(ParseOperation::PreviewImage),
                PreviewContent::Media => None,
                PreviewContent::Text { .. }
                | PreviewContent::Document { .. }
                | PreviewContent::Rendered { .. }
                | PreviewContent::Rasterized { .. }
                | PreviewContent::SandboxedMedia { .. }
                | PreviewContent::Archive { .. }
                | PreviewContent::Unsupported => None,
            };
            if let Some(operation) = &operation {
                let staged = if entry.location.native_path().is_none() {
                    if !matches!(operation, ParseOperation::PreviewImage) {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message:
                                "Remote PDF previews are not supported; copy the file locally first"
                                    .into(),
                        });
                        return;
                    }
                    match remote::stage(&entry).await {
                        Ok(staged) => Some(staged),
                        Err(message) => {
                            if !cancellation_for_task.is_cancelled() {
                                emit(PreviewEvent::Failed {
                                    request_id,
                                    entry,
                                    message,
                                });
                            }
                            return;
                        }
                    }
                } else {
                    None
                };
                let path = staged
                    .as_ref()
                    .map(|file| file.path())
                    .or_else(|| entry.location.native_path())
                    .expect("native or staged preview input")
                    .to_path_buf();
                // Remote originals are not written to persistent thumbnail caches.
                let modified = match entry.modified_unix_seconds {
                    _ if staged.is_some() => None,
                    crate::model::MetadataValue::Known(m) => Some(m),
                    crate::model::MetadataValue::Unknown
                    | crate::model::MetadataValue::Unavailable => None,
                };
                let pdf_page = match operation {
                    ParseOperation::PreviewPdf(size) => Some((request.pdf_page, *size)),
                    _ => None,
                };
                let cache_key = modified.map(|modified| PreviewCacheKey {
                    path: path.clone(),
                    modified,
                    pdf_page,
                });
                if let Some(cached) = cache_key
                    .as_ref()
                    .and_then(|key| PREVIEW_CACHE.with(|cache| cache.borrow_mut().get(key)))
                {
                    emit(PreviewEvent::Ready(Preview {
                        request_id,
                        entry,
                        content_type,
                        content: cached,
                    }));
                    return;
                }

                if cancellation_for_task.is_cancelled() {
                    return;
                }

                let pdf_permit = if matches!(operation, ParseOperation::PreviewPdf(_)) {
                    let Some(permit) = request_pdf_render_permit().acquire().await else {
                        return;
                    };
                    if cancellation_for_task.is_cancelled() {
                        return;
                    }
                    Some(permit)
                } else {
                    None
                };
                abort_safe_for_task.set(pdf_permit.is_none());
                let value = request.pdf_page;
                let cancellation = cancellation_for_task.clone();
                let spawn_path = path.clone();
                let mut thumbnail_to_store = None;
                let for_render = operation.clone();
                let render = gio::spawn_blocking(move || {
                    let _staged = staged;
                    let output = render(
                        &spawn_path,
                        for_render,
                        value,
                        media_preview_backend,
                        &cancellation,
                    )?;
                    Ok::<_, String>(output)
                })
                .await;
                abort_safe_for_task.set(true);
                if cancellation_for_task.is_cancelled() {
                    return;
                }
                content = match render {
                    Ok(Ok(output)) if matches!(operation, ParseOperation::PreviewPdf(_)) => {
                        if let Some(mtime) = modified
                            && request.pdf_page == 0
                        {
                            thumbnail_to_store = Some((path.clone(), mtime, output.data.clone()));
                        }
                        PreviewContent::Pdf {
                            png: output.data,
                            page: output.page,
                            pages: output.pages,
                            text_layer: output.text_layer.map(std::sync::Arc::new),
                        }
                    }
                    Ok(Ok(output)) => {
                        if let Some(mtime) = modified {
                            thumbnail_to_store = Some((path.clone(), mtime, output.data.clone()));
                        }
                        PreviewContent::Rasterized { png: output.data }
                    }
                    Ok(Err(message)) => {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message,
                        });
                        return;
                    }
                    Err(_) => {
                        if cancellation_for_task.is_cancelled() {
                            return;
                        }
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message: "The preview worker stopped unexpectedly.".to_owned(),
                        });
                        return;
                    }
                };
                drop(pdf_permit);
                if let Some(cache_key) = cache_key {
                    PREVIEW_CACHE.with(|cache| {
                        cache.borrow_mut().insert(cache_key, content.clone());
                    });
                }
                emit(PreviewEvent::Ready(Preview {
                    request_id,
                    entry,
                    content_type,
                    content,
                }));
                if let Some((path, mtime, png)) = thumbnail_to_store {
                    let _ = gio::spawn_blocking(move || {
                        crate::ui::thumbnail_cache::store(&path, mtime, &png);
                    })
                    .await;
                }
                return;
            } else if matches!(
                content,
                PreviewContent::Text { .. } | PreviewContent::Document { .. }
            ) {
                let file = gio_file_for_location(&entry.location);
                let native_path = entry.location.native_path().map(ToOwned::to_owned);
                content =
                    match read_text(&file, native_path.as_deref(), request.text_byte_limit).await {
                        Ok((source, truncated)) => {
                            if let Some(kind) = document_kind {
                                if truncated {
                                    PreviewContent::Document {
                                        source,
                                        document: None,
                                        fallback_reason: Some(
                                            "Rendered view is unavailable because the document exceeds the 1 MB preview limit."
                                                .to_owned(),
                                        ),
                                        warnings: Vec::new(),
                                        truncated,
                                    }
                                } else if request.render_document {
                                    let cancellation = cancellation_for_task.clone();
                                    let parsed = gio::spawn_blocking(move || {
                                        let parsed = parse_document(kind, &source, &cancellation)
                                            .and_then(|parsed| {
                                                layout_document(parsed.document, &cancellation)
                                                    .map(|document| (document, parsed.warnings))
                                            });
                                        (source, parsed)
                                    })
                                    .await;
                                    let (source, parsed) = match parsed {
                                        Ok(parsed) => parsed,
                                        Err(_) => {
                                            if cancellation_for_task.is_cancelled() {
                                                return;
                                            }
                                            emit(PreviewEvent::Failed {
                                                request_id,
                                                entry,
                                                message: "The preview worker stopped unexpectedly."
                                                    .to_owned(),
                                            });
                                            return;
                                        }
                                    };
                                    if cancellation_for_task.is_cancelled() {
                                        return;
                                    }
                                    match parsed {
                                        Ok((document, warnings)) => PreviewContent::Document {
                                            source,
                                            document: Some(document),
                                            fallback_reason: None,
                                            warnings,
                                            truncated,
                                        },
                                        Err(reason) => PreviewContent::Document {
                                            source,
                                            document: None,
                                            fallback_reason: Some(reason),
                                            warnings: Vec::new(),
                                            truncated,
                                        },
                                    }
                                } else {
                                    PreviewContent::Document {
                                        source,
                                        document: None,
                                        fallback_reason: None,
                                        warnings: Vec::new(),
                                        truncated,
                                    }
                                }
                            } else {
                                PreviewContent::Text {
                                    content: source,
                                    truncated,
                                }
                            }
                        }
                        Err(error) => {
                            emit(PreviewEvent::Failed {
                                request_id,
                                entry,
                                message: error.to_string(),
                            });
                            return;
                        }
                    };
            }

            if cancellation_for_task.is_cancelled() {
                return;
            }
            emit(PreviewEvent::Ready(Preview {
                request_id,
                entry,
                content_type,
                content,
            }));
        });

        LoadHandle::new(move || {
            cancellation.cancel();
            if abort_safe.get() {
                task.abort();
            }
        })
    }
}

fn pdf_render_size(viewport: MediaPreviewSize) -> PdfRenderSize {
    PdfRenderSize::for_viewport_width(viewport.width)
}

async fn read_text(
    file: &gio::File,
    native_path: Option<&Path>,
    byte_limit: usize,
) -> Result<(String, bool), glib::Error> {
    if let Some(path) = native_path {
        let path = path.to_path_buf();
        let result = gio::spawn_blocking(move || {
            use std::io::Read;
            let file = std::fs::File::open(&path)
                .map_err(|e| glib::Error::new(gio::IOErrorEnum::Failed, &e.to_string()))?;
            let mut bytes = Vec::new();
            file.take(byte_limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| glib::Error::new(gio::IOErrorEnum::Failed, &e.to_string()))?;
            let truncated = bytes.len() > byte_limit;
            let sample = &bytes[..bytes.len().min(byte_limit)];
            Ok((decode_text_sample(sample), truncated))
        })
        .await;
        match result {
            Ok(ok) => return ok,
            Err(_) => {
                return Err(glib::Error::new(
                    gio::IOErrorEnum::Cancelled,
                    "Read cancelled",
                ));
            }
        }
    }
    let stream = file.read_future(glib::Priority::DEFAULT).await?;
    let bytes = stream
        .read_bytes_future(byte_limit.saturating_add(1), glib::Priority::DEFAULT)
        .await?;
    let bytes = bytes.as_ref();
    let truncated = bytes.len() > byte_limit;
    let sample = &bytes[..bytes.len().min(byte_limit)];
    Ok((decode_text_sample(sample), truncated))
}

fn decode_text_sample(sample: &[u8]) -> String {
    normalize_preview_text(&String::from_utf8_lossy(sample)).into_owned()
}

#[cfg(test)]
mod tests;
