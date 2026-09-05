// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    model::Location,
    sandbox::{Cancellation, MediaPreviewBackend, ParseOperation},
    services::{
        LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
        content_family, has_plain_text_extension, is_extensionless_dotfile,
        is_non_executable_extensionless_dotfile,
    },
};

const MAX_PREVIEW_CACHE_ENTRIES: usize = 64;
const MAX_PREVIEW_CACHE_BYTES: usize = 128 * 1024 * 1024;

struct PreviewCache {
    entries: HashMap<PreviewCacheKey, PreviewContent>,
    recent: VecDeque<PreviewCacheKey>,
    byte_count: usize,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct PreviewCacheKey {
    path: PathBuf,
    modified: Option<i64>,
    pdf_page: Option<i32>,
}

impl PreviewCache {
    fn get(&mut self, key: &PreviewCacheKey) -> Option<PreviewContent> {
        let content = self.entries.get(key)?.clone();
        self.recent.retain(|k| k != key);
        self.recent.push_back(key.clone());
        Some(content)
    }

    fn insert(&mut self, key: PreviewCacheKey, content: PreviewContent) {
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
        PreviewContent::SandboxedMedia { data } => data.len(),
        PreviewContent::Text { content, .. } => content.len(),
        _ => 0,
    }
}

thread_local! {
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
        let media_preview_backend = (self.media_preview_backend)();
        let request_id = request.id;
        let entry = request.entry.clone();
        let cancellation = Cancellation::default();
        let cancellation_for_task = cancellation.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            let (guessed_type, uncertain) =
                gio::content_type_guess(Some(Path::new(&entry.native_name)), None::<&[u8]>);
            let mut content_type = guessed_type.to_string();
            let mut content = content_family(&content_type);

            if matches!(content, PreviewContent::Unsupported)
                && (has_plain_text_extension(&entry.native_name)
                    || is_extensionless_dotfile(&entry.native_name))
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
                let file = file_for_location(&entry.location);
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

            let operation = match content {
                PreviewContent::Pdf { .. } => Some(ParseOperation::PreviewPdf),
                PreviewContent::Image => Some(ParseOperation::PreviewImage),
                PreviewContent::Media => Some(ParseOperation::PreviewMedia),
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
                    ParseOperation::PreviewPdf => Some(request.pdf_page),
                    _ => None,
                };
                let cache_key = PreviewCacheKey {
                    path: path.clone(),
                    modified,
                    pdf_page,
                };
                if let Some(cached) = PREVIEW_CACHE.with(|cache| cache.borrow_mut().get(&cache_key))
                {
                    emit(PreviewEvent::Ready(Preview {
                        request_id,
                        entry,
                        content_type,
                        content: cached,
                    }));
                    return;
                }

                if let Some(mtime) = modified
                    && let Some(thumb_png) = crate::ui::thumbnail_cache::lookup(&path, mtime)
                {
                    let placeholder = match operation {
                        ParseOperation::PreviewPdf if request.pdf_page == 0 => {
                            Some(PreviewContent::Pdf {
                                png: thumb_png.clone(),
                                page: 0,
                                pages: 1,
                            })
                        }
                        ParseOperation::PreviewImage => {
                            Some(PreviewContent::Rasterized { png: thumb_png })
                        }
                        _ => None,
                    };
                    if let Some(placeholder) = placeholder {
                        emit(PreviewEvent::Ready(Preview {
                            request_id,
                            entry: entry.clone(),
                            content_type: content_type.clone(),
                            content: placeholder,
                        }));
                    }
                }

                let value = request.pdf_page;
                let cancellation = cancellation_for_task.clone();
                let spawn_path = path.clone();
                content = match gio::spawn_blocking(move || {
                    crate::sandbox::parse(
                        &spawn_path,
                        operation,
                        value,
                        media_preview_backend,
                        &cancellation,
                    )
                })
                .await
                {
                    Ok(Ok(output)) if operation == ParseOperation::PreviewPdf => {
                        if let Some(mtime) = modified
                            && request.pdf_page == 0
                        {
                            crate::ui::thumbnail_cache::store(&path, mtime, &output.data);
                        }
                        PreviewContent::Pdf {
                            png: output.data,
                            page: output.page,
                            pages: output.pages,
                        }
                    }
                    Ok(Ok(output)) if operation == ParseOperation::PreviewMedia => {
                        PreviewContent::SandboxedMedia { data: output.data }
                    }
                    Ok(Ok(output)) => {
                        if let Some(mtime) = modified {
                            crate::ui::thumbnail_cache::store(&path, mtime, &output.data);
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
                PREVIEW_CACHE.with(|cache| {
                    cache.borrow_mut().insert(cache_key, content.clone());
                });
            } else if matches!(content, PreviewContent::Text { .. }) {
                let file = file_for_location(&entry.location);
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
            task.abort();
        })
    }
}

fn file_for_location(location: &Location) -> gio::File {
    location
        .native_path()
        .map(gio::File::for_path)
        .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
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
