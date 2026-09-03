// SPDX-License-Identifier: GPL-3.0-or-later

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

use crate::{
    model::Location,
    sandbox::{Cancellation, ParseOperation},
    services::{
        LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
        content_family, document_kind, has_plain_text_extension, layout_document,
        normalize_preview_text, parse_document,
    },
};

#[derive(Default)]
pub struct LocalPreviewProvider;

impl PreviewProvider for LocalPreviewProvider {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        let request_id = request.id;
        let entry = request.entry.clone();
        let cancellation = Cancellation::default();
        let cancellation_for_task = cancellation.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            let file = file_for_location(&entry.location);
            let info = match file
                .query_info_future(
                    "standard::content-type",
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
            let document_kind = document_kind(
                &content_type,
                &entry.native_name,
                entry.location.native_path().is_some(),
            );
            let mut content = if document_kind.is_some() {
                PreviewContent::Document {
                    source: String::new(),
                    document: None,
                    fallback_reason: None,
                    warnings: Vec::new(),
                    truncated: false,
                }
            } else {
                content_family(&content_type)
            };
            if matches!(content, PreviewContent::Unsupported)
                && (gio::content_type_is_a(&content_type, "text/plain")
                    || has_plain_text_extension(&entry.native_name))
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
                | PreviewContent::Document { .. }
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
                    crate::sandbox::parse(&path, operation, value, &cancellation)
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
            } else if matches!(
                content,
                PreviewContent::Text { .. } | PreviewContent::Document { .. }
            ) {
                content = match read_text(&file, request.text_byte_limit).await {
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
    Ok((decode_text_sample(sample), truncated))
}

fn decode_text_sample(sample: &[u8]) -> String {
    normalize_preview_text(&String::from_utf8_lossy(sample)).into_owned()
}

#[cfg(test)]
mod tests;
