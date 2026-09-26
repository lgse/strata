// SPDX-License-Identifier: MIT

use crate::adapters::directory_summary::{DirectorySummary, summarize_directory};
use crate::adapters::gio_file_for_location;
use crate::model::{EntryKind, FileEntry, MetadataValue};
use crate::services::{
    PreviewContent, content_family, filter_name_matches, fold_for_search, has_plain_text_extension,
    is_extensionless_dotfile,
};
use gtk::gio;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

pub(in crate::ui) fn format_file_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let (value, unit) = rounded_size_and_unit(bytes, &UNITS);
    let formatted = format!("{value:.1}");
    format!("{} {}", formatted.trim_end_matches(".0"), UNITS[unit])
}

/// Divide `bytes` into the largest unit whose threshold it meets after
/// rounding to one decimal, returning the rounded value and unit index.
/// Callers that format with zero decimals for values >= 10 still receive
/// the one-decimal rounded value so they can decide their own precision.
pub(in crate::ui) fn rounded_size_and_unit(bytes: u64, units: &[&str]) -> (f64, usize) {
    if bytes < 1_000 {
        return (bytes as f64, 0);
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1_000.0 && unit < units.len() - 1 {
        value /= 1_000.0;
        unit += 1;
    }
    let rounded = (value * 10.0).round() / 10.0;
    if rounded >= 1_000.0 && unit < units.len() - 1 {
        (rounded / 1_000.0, unit + 1)
    } else {
        (rounded, unit)
    }
}

pub(in crate::ui) fn metadata_needs_fill(entry: &FileEntry) -> bool {
    entry.modified_unix_seconds == crate::model::MetadataValue::Unknown
        || (!entry.is_directory() && entry.size == crate::model::MetadataValue::Unknown)
}

pub(super) fn entry_responds_to_preview_click(entry: &FileEntry, previews_enabled: bool) -> bool {
    previews_enabled
        && !entry.is_directory()
        && crate::ui::preview::entry_supports_quick_preview(entry)
}

pub(super) fn entry_supports_printing(entry: &FileEntry) -> bool {
    if !matches!(entry.kind, EntryKind::File | EntryKind::FileSymbolicLink) {
        return false;
    }

    let (content_type, uncertain) =
        gio::content_type_guess(Some(Path::new(&entry.native_name)), None::<&[u8]>);
    // An uncertain name guess defers to the loader, which resolves the file's content type.
    matches!(content_family(&content_type), PreviewContent::Text { .. })
        || (uncertain && matches!(content_family(&content_type), PreviewContent::Unsupported))
        || (entry.location.native_path().is_some()
            && matches!(
                content_family(&content_type),
                PreviewContent::Image | PreviewContent::Pdf { .. }
            ))
        || gio::content_type_is_a(&content_type, "text/plain")
        || has_plain_text_extension(&entry.native_name)
        || is_extensionless_dotfile(&entry.native_name)
}

pub(in crate::ui) fn entry_model_value(entry: &FileEntry) -> String {
    let kind = if entry.is_broken_symbolic_link() {
        'x'
    } else if entry.is_directory() {
        'd'
    } else if entry.is_symbolic_link() {
        's'
    } else if entry.kind == EntryKind::Other {
        'o'
    } else {
        'f'
    };
    let hidden = if entry.is_hidden { 'h' } else { 'v' };
    let name = entry.display_name.as_str();
    let mut value = String::with_capacity(name.len() + 3);
    value.push(kind);
    value.push(hidden);
    value.push('\t');
    value.push_str(name);
    value
}

pub(super) fn model_display_name(value: &str) -> &str {
    value.split_once('\t').map_or(value, |(_, name)| name)
}

pub(super) fn model_is_directory(value: &str) -> bool {
    value.starts_with("d")
}

pub(in crate::ui) fn model_is_hidden(value: &str) -> bool {
    value.as_bytes().get(1) == Some(&b'h')
}

fn model_is_broken_link(value: &str) -> bool {
    value.starts_with("x")
}

pub(in crate::ui) const FOLDER_TYPE_GROUP: &str = crate::services::FOLDER_TYPE_NAME;
pub(in crate::ui) const OTHER_TYPE_GROUP: &str = crate::services::OTHER_TYPE_NAME;

pub(in crate::ui) fn model_type_group(value: &str) -> String {
    if model_is_directory(value) {
        return FOLDER_TYPE_GROUP.to_owned();
    }
    if model_is_broken_link(value) {
        return crate::services::BROKEN_LINK_TYPE_NAME.to_owned();
    }
    if value.starts_with('o') {
        return OTHER_TYPE_GROUP.to_owned();
    }
    let name = model_display_name(value);
    crate::services::mime_description_for_name(name)
}

pub(in crate::ui) fn entry_filter(
    show_hidden: Rc<Cell<bool>>,
    filter_query: Rc<RefCell<String>>,
) -> gtk::CustomFilter {
    gtk::CustomFilter::new(move |item| {
        let Some(item) = item.downcast_ref::<gtk::StringObject>() else {
            return false;
        };
        let value = item.string();
        entry_matches(&value, show_hidden.get(), &filter_query.borrow())
    })
}

pub(in crate::ui) fn entry_icon(entry: &FileEntry) -> &'static str {
    if entry.is_broken_symbolic_link() {
        return crate::assets::icons::X;
    }
    if entry.is_directory() {
        return crate::assets::icons::FOLDER;
    }
    icon_for_name(&entry.display_name)
}

/// `query` must already be folded through `fold_for_search` by the caller.
pub(super) fn entry_matches(value: &str, show_hidden: bool, query: &str) -> bool {
    (show_hidden || !model_is_hidden(value))
        && (query.is_empty()
            || filter_name_matches(&fold_for_search(model_display_name(value)), query))
}

pub(in crate::ui) fn icon_for_name(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("sh" | "bash" | "zsh" | "fish") => crate::assets::icons::TERMINAL,
        Some(
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif" | "heic" | "heif"
            | "jxl" | "tif" | "tiff" | "3fr" | "arw" | "cr2" | "cr3" | "dcr" | "dng" | "erf"
            | "kdc" | "mef" | "mos" | "mrw" | "nef" | "nrw" | "orf" | "pef" | "raf" | "raw" | "rw2"
            | "rwl" | "sr2" | "srf" | "srw" | "x3f",
        ) => crate::assets::icons::PICTURES,
        Some("mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v") => crate::assets::icons::VIDEOS,
        Some("zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst") => {
            crate::assets::icons::FILE_ARCHIVE
        }
        Some(
            "rs" | "c" | "h" | "cpp" | "go" | "py" | "rb" | "java" | "js" | "jsx" | "ts" | "tsx"
            | "lua" | "php" | "html" | "css" | "scss" | "json",
        ) => crate::assets::icons::FILE_CODE,
        _ => crate::assets::icons::DOCUMENTS,
    }
}

pub(super) fn item_count_label(count: usize) -> String {
    if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{count} items")
    }
}

pub(super) fn entry_kind_summary(entries: &[FileEntry]) -> String {
    let directories = entries.iter().filter(|entry| entry.is_directory()).count();
    let files = entries.len().saturating_sub(directories);
    match (files, directories) {
        (1, 0) => "1 file".to_owned(),
        (files, 0) => format!("{files} files"),
        (0, 1) => "1 folder".to_owned(),
        (0, directories) => format!("{directories} folders"),
        _ => item_count_label(entries.len()),
    }
}

pub(super) async fn aggregate_directory_summary(entries: &[FileEntry]) -> DirectorySummary {
    let mut total = DirectorySummary::default();
    for entry in entries {
        if entry.is_directory() {
            let directory = gio_file_for_location(&entry.location);
            match summarize_directory(&directory).await {
                Ok(summary) => {
                    total.item_count = total.item_count.saturating_add(summary.item_count);
                    total.total_size = total.total_size.saturating_add(summary.total_size);
                    total.visible_file_count = total
                        .visible_file_count
                        .saturating_add(summary.visible_file_count);
                    total.visible_folder_count = total
                        .visible_folder_count
                        .saturating_add(summary.visible_folder_count);
                    total.issues.unreadable |= summary.issues.unreadable;
                    total.issues.timed_out |= summary.issues.timed_out;
                    total.issues.depth_limited |= summary.issues.depth_limited;
                }
                Err(_) => total.issues.unreadable = true,
            }
        } else {
            total.item_count = total.item_count.saturating_add(1);
            total.visible_file_count = total.visible_file_count.saturating_add(1);
            match entry.size {
                MetadataValue::Known(size) => {
                    total.total_size = total.total_size.saturating_add(size);
                }
                MetadataValue::Unknown | MetadataValue::Unavailable => {
                    total.issues.unreadable = true;
                }
            }
        }
    }
    total
}
