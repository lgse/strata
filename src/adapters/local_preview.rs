// SPDX-License-Identifier: GPL-3.0-or-later

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

use crate::{
    model::Location,
    sandbox::{Cancellation, MediaPreviewBackend, ParseOperation},
    services::{
        LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
        content_family, has_plain_text_extension, is_non_executable_extensionless_dotfile,
    },
};

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
            let content_type = info
                .content_type()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "application/octet-stream".to_owned());
            let unix_mode = info
                .has_attribute(gio::FILE_ATTRIBUTE_UNIX_MODE)
                .then(|| info.attribute_uint32(gio::FILE_ATTRIBUTE_UNIX_MODE));
            let mut content = content_family(&content_type);
            if matches!(content, PreviewContent::Unsupported)
                && (gio::content_type_is_a(&content_type, "text/plain")
                    || has_plain_text_extension(&entry.native_name)
                    || is_non_executable_extensionless_dotfile(&entry.native_name, unix_mode))
            {
                content = PreviewContent::Text {
                    content: String::new(),
                    truncated: false,
                };
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
                let value = request.pdf_page;
                let cancellation = cancellation_for_task.clone();
                content = match gio::spawn_blocking(move || {
                    crate::sandbox::parse(
                        &path,
                        operation,
                        value,
                        media_preview_backend,
                        &cancellation,
                    )
                })
                .await
                {
                    Ok(Ok(output)) if operation == ParseOperation::PreviewPdf => {
                        PreviewContent::Pdf {
                            png: output.data,
                            page: output.page,
                            pages: output.pages,
                        }
                    }
                    Ok(Ok(output)) if operation == ParseOperation::PreviewMedia => {
                        PreviewContent::SandboxedMedia { data: output.data }
                    }
                    Ok(Ok(output)) => PreviewContent::Rasterized { png: output.data },
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
            } else if matches!(content, PreviewContent::Text { .. }) {
                content = match read_text(&file, request.text_byte_limit).await {
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

async fn read_text(file: &gio::File, byte_limit: usize) -> Result<(String, bool), glib::Error> {
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
