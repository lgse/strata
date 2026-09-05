// SPDX-License-Identifier: GPL-3.0-or-later

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

use crate::{
    model::Location,
    sandbox::{Cancellation, MediaPreviewBackend, ParseOperation},
    services::{
        LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
        content_family, has_database_extension, has_plain_text_extension,
        is_non_executable_extensionless_dotfile,
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
            if matches!(content, PreviewContent::Unsupported) {
                if gio::content_type_is_a(&content_type, "text/plain")
                    || has_plain_text_extension(&entry.native_name)
                    || is_non_executable_extensionless_dotfile(&entry.native_name, unix_mode)
                {
                    content = PreviewContent::Text {
                        content: String::new(),
                        truncated: false,
                    };
                } else if has_database_extension(&entry.native_name) {
                    content = PreviewContent::Database {
                        tables: Vec::new(),
                        selected: None,
                    };
                }
            }

            let operation = match content {
                PreviewContent::Pdf { .. } => Some(ParseOperation::PreviewPdf),
                PreviewContent::Image => Some(ParseOperation::PreviewImage),
                PreviewContent::Media => Some(ParseOperation::PreviewMedia),
                PreviewContent::Database { .. } => Some(ParseOperation::PreviewDatabase),
                PreviewContent::Text { .. }
                | PreviewContent::Rasterized { .. }
                | PreviewContent::SandboxedMedia { .. }
                | PreviewContent::DatabaseTable(_)
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
                let value = if operation == ParseOperation::PreviewPdf && request.pdf_page < 0 {
                    0
                } else {
                    request.pdf_page
                };
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
                    Ok(Ok(output)) if operation == ParseOperation::PreviewDatabase => {
                        parse_database_output(&output.data, value)
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

fn parse_database_output(data: &[u8], value: i32) -> PreviewContent {
    let text = String::from_utf8_lossy(data);
    if text.is_empty() {
        return PreviewContent::Database {
            tables: Vec::new(),
            selected: None,
        };
    }

    if let Some((tables_part, data_part)) = text.split_once("\n---DATA---\n") {
        let tables = parse_tables_list(tables_part);
        let selected = parse_table_data(data_part, 0);
        PreviewContent::Database { tables, selected }
    } else if value >= 0 {
        let page = crate::services::decode_database_page(value)
            .map(|(_, page)| page)
            .unwrap_or(0);
        let data = parse_table_data(&text, page);
        if let Some(data) = data {
            PreviewContent::DatabaseTable(data)
        } else {
            PreviewContent::Database {
                tables: Vec::new(),
                selected: None,
            }
        }
    } else {
        let tables = parse_tables_list(&text);
        PreviewContent::Database {
            tables,
            selected: None,
        }
    }
}

fn parse_tables_list(raw: &str) -> Vec<crate::services::DatabaseTableItem> {
    raw.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next().unwrap_or("").to_owned();
            let kind = parts.next().unwrap_or("table");
            crate::services::DatabaseTableItem {
                name,
                is_view: kind == "view",
            }
        })
        .collect()
}

fn parse_table_data(raw: &str, page: usize) -> Option<crate::services::DatabaseTableData> {
    let (name_block, count_and_rows) = raw.split_once("\n---COUNT---\n")?;
    let (name, rest) = name_block
        .split_once("\n---SCHEMA---\n")
        .unwrap_or((name_block, ""));
    // Helpers predating ---TYPES--- emit no marker; the whole block is schema.
    let (schema, types_raw) = rest.split_once("\n---TYPES---\n").unwrap_or((rest, ""));
    let (count_str, rows_csv) = count_and_rows
        .split_once("\n---ROWS---\n")
        .unwrap_or((count_and_rows, ""));
    let total_rows = count_str.trim().parse::<usize>().ok();

    Some(crate::services::DatabaseTableData {
        name: name.to_owned(),
        is_view: false,
        schema: schema.to_owned(),
        columns: parse_columns_list(types_raw),
        total_rows,
        rows_csv: rows_csv.to_owned(),
        page,
    })
}

fn parse_columns_list(raw: &str) -> Vec<crate::services::DatabaseColumn> {
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split('\t');
            crate::services::DatabaseColumn {
                name: parts.next().unwrap_or("").to_owned(),
                decl_type: parts.next().unwrap_or("").trim().to_owned(),
            }
        })
        .filter(|column| !column.name.is_empty())
        .collect()
}

#[cfg(test)]
mod tests;
