// SPDX-License-Identifier: GPL-3.0-or-later

use crate::model::{EntryKind, FileEntry};
use crate::services::{
    PreviewContent, content_family, has_plain_text_extension, is_extensionless_dotfile,
};
use gtk::gio;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

pub(in crate::ui) fn format_file_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    if bytes < 1_000 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1_000.0 && unit < UNITS.len() - 1 {
        value /= 1_000.0;
        unit += 1;
    }
    let formatted = format!("{value:.1}");
    format!("{} {}", formatted.trim_end_matches(".0"), UNITS[unit])
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

    let (content_type, _) =
        gio::content_type_guess(Some(Path::new(&entry.native_name)), None::<&[u8]>);
    matches!(
        content_family(&content_type),
        PreviewContent::Text { .. } | PreviewContent::Image | PreviewContent::Pdf { .. }
    ) || gio::content_type_is_a(&content_type, "text/plain")
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

/// Directories lead a grouped view, and files whose type the shared MIME database
/// cannot name fall back to a plain label.
pub(in crate::ui) const FOLDER_TYPE_GROUP: &str = "Folder";

const UNTYPED_TYPE_GROUP: &str = "File";

/// The user-facing file-type label a model value belongs to when the browser groups
/// entries by type. Labels come from the shared MIME database, so they read the way
/// they do elsewhere on the desktop: "JSON document", "Python script", and so on.
pub(in crate::ui) fn model_type_group(value: &str) -> String {
    if model_is_directory(value) {
        return FOLDER_TYPE_GROUP.to_owned();
    }
    if model_is_broken_link(value) {
        return "Broken link".to_owned();
    }
    let name = model_display_name(value);
    TYPE_GROUPS.with_borrow_mut(|cache| {
        if let Some(label) = cache.get(type_group_key(name)) {
            return label.clone();
        }
        let label = guess_type_group(name);
        // A directory listing holds far more entries than distinct types, and the
        // cache is keyed by suffix, so it stays small; clear it if that ever fails.
        if cache.len() >= TYPE_GROUP_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(type_group_key(name).to_owned(), label.clone());
        label
    })
}

/// Names sharing a suffix share a type, so the cache is keyed by suffix where there
/// is one and by the whole name otherwise.
fn type_group_key(name: &str) -> &str {
    match name.rfind('.') {
        Some(position) if position > 0 => &name[position..],
        _ => name,
    }
}

fn guess_type_group(name: &str) -> String {
    let (content_type, _) = gio::content_type_guess(Some(Path::new(name)), None::<&[u8]>);
    if content_type.is_empty() || content_type == "application/octet-stream" {
        return UNTYPED_TYPE_GROUP.to_owned();
    }
    let description = gio::content_type_get_description(&content_type);
    if description.is_empty() {
        return UNTYPED_TYPE_GROUP.to_owned();
    }
    description.to_string()
}

const TYPE_GROUP_CACHE_LIMIT: usize = 2048;

thread_local! {
    static TYPE_GROUPS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
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

/// `query` must already be folded to lowercase by the caller.
pub(super) fn entry_matches(value: &str, show_hidden: bool, query: &str) -> bool {
    (show_hidden || !model_is_hidden(value))
        && (query.is_empty() || model_display_name(value).to_lowercase().contains(query))
}

pub(super) fn icon_for_name(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("sh" | "bash" | "zsh" | "fish") => crate::assets::icons::TERMINAL,
        Some(
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif" | "tif" | "tiff"
            | "3fr" | "arw" | "cr2" | "cr3" | "dcr" | "dng" | "erf" | "kdc" | "mef" | "mos" | "mrw"
            | "nef" | "nrw" | "orf" | "pef" | "raf" | "raw" | "rw2" | "rwl" | "sr2" | "srf" | "srw"
            | "x3f",
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

#[cfg(test)]
mod tests;
