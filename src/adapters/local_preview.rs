// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use futures_channel::oneshot;
use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::gio_file_for_location,
    sandbox::{Cancellation, MediaPreviewBackend, ParseOperation, PdfRenderSize},
    services::{
        LoadHandle, MediaPreviewSize, Preview, PreviewContent, PreviewEvent, PreviewProvider,
        PreviewRequest, SandboxedMedia, content_family, has_plain_text_extension,
        is_non_executable_extensionless_dotfile,
    },
};

const MAX_PREVIEW_CACHE_ENTRIES: usize = 64;
const MAX_PREVIEW_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_CONCURRENT_PDF_RENDERS: usize = 1;
const PROGRESSIVE_RENDER_SETTLE_DELAY: Duration = Duration::from_millis(50);

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
        PreviewContent::Rasterized { png } | PreviewContent::Pdf { png, .. } => png.len(),
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

            if matches!(content, PreviewContent::Media) {
                let Some(path) = entry.location.native_path().map(ToOwned::to_owned) else {
                    emit(PreviewEvent::Failed {
                        request_id,
                        entry,
                        message: "Only local files can be previewed safely".into(),
                    });
                    return;
                };
                emit(PreviewEvent::Ready(Preview {
                    request_id,
                    entry,
                    content_type,
                    content: PreviewContent::SandboxedMedia {
                        media: SandboxedMedia {
                            path,
                            size: request.media_size,
                            backend: media_preview_backend,
                        },
                    },
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
                | PreviewContent::Rasterized { .. }
                | PreviewContent::SandboxedMedia { .. }
                | PreviewContent::Unsupported => None,
            };
            if let Some(operation) = operation {
                let Some(path) = entry.location.native_path().map(ToOwned::to_owned) else {
                    emit(PreviewEvent::Failed {
                        request_id,
                        entry,
                        message: "Only local files can be previewed safely".to_owned(),
                    });
                    return;
                };
                let modified = match entry.modified_unix_seconds {
                    crate::model::MetadataValue::Known(m) => Some(m),
                    crate::model::MetadataValue::Unknown
                    | crate::model::MetadataValue::Unavailable => None,
                };
                let pdf_page = match operation {
                    ParseOperation::PreviewPdf(size) => Some((request.pdf_page, size)),
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

                let shared_thumbnail = if uses_shared_thumbnail(operation) {
                    if let Some(mtime) = modified {
                        let thumbnail_path = path.clone();
                        gio::spawn_blocking(move || {
                            crate::ui::thumbnail_cache::lookup(&thumbnail_path, mtime)
                        })
                        .await
                        .ok()
                        .flatten()
                    } else {
                        None
                    }
                } else {
                    None
                };
                let placeholder = shared_thumbnail.and_then(|thumb_png| match operation {
                    ParseOperation::PreviewImage => {
                        Some(PreviewContent::Rasterized { png: thumb_png })
                    }
                    _ => None,
                });
                let placeholder_emitted = placeholder.is_some();
                if let Some(placeholder) = placeholder {
                    emit(PreviewEvent::Ready(Preview {
                        request_id,
                        entry: entry.clone(),
                        content_type: content_type.clone(),
                        content: placeholder,
                    }));
                }

                if !wait_for_full_render(placeholder_emitted, &cancellation_for_task).await {
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
                let render = gio::spawn_blocking(move || {
                    let output = render(
                        &spawn_path,
                        operation,
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
                            && !placeholder_emitted
                        {
                            thumbnail_to_store = Some((path.clone(), mtime, output.data.clone()));
                        }
                        PreviewContent::Pdf {
                            png: output.data,
                            page: output.page,
                            pages: output.pages,
                        }
                    }
                    Ok(Ok(output)) => {
                        if let Some(mtime) = modified
                            && !placeholder_emitted
                        {
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
                    Err(_) => return,
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
            } else if matches!(content, PreviewContent::Text { .. }) {
                let file = gio_file_for_location(&entry.location);
                let native_path = entry.location.native_path().map(ToOwned::to_owned);
                content =
                    match read_text(&file, native_path.as_deref(), request.text_byte_limit).await {
                        Ok((content, truncated)) => PreviewContent::Text { content, truncated },
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

fn uses_shared_thumbnail(operation: ParseOperation) -> bool {
    operation == ParseOperation::PreviewImage
}

fn full_render_settle_delay(has_placeholder: bool) -> Duration {
    if has_placeholder {
        PROGRESSIVE_RENDER_SETTLE_DELAY
    } else {
        Duration::ZERO
    }
}

async fn wait_for_full_render(has_placeholder: bool, cancellation: &Cancellation) -> bool {
    let settle_delay = full_render_settle_delay(has_placeholder);
    if !settle_delay.is_zero() {
        glib::timeout_future(settle_delay).await;
    }
    !cancellation.is_cancelled()
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
            Ok((String::from_utf8_lossy(sample).into_owned(), truncated))
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
    Ok((String::from_utf8_lossy(sample).into_owned(), truncated))
}

#[cfg(test)]
mod tests;
