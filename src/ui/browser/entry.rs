// SPDX-License-Identifier: MIT

use crate::adapters::directory_summary::{DirectorySummary, summarize_directory};
use crate::adapters::gio_file_for_location;
use crate::assets::icons;
use crate::model::{EntryKind, FileEntry, MetadataValue};
use crate::services::{
    PathMatcher, PathQuery, PreviewContent, content_family, filter_name_matches, fold_for_search,
    has_plain_text_extension, is_extensionless_dotfile,
};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

thread_local! {
    static FILTER_TERMS: RefCell<Option<(String, PathMatcher)>> = const { RefCell::new(None) };
}

pub(in crate::ui) fn format_file_size(bytes: u64) -> String {
    crate::i18n::file_size(bytes)
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
        && (query.trim().is_empty() || {
            let name = fold_for_search(model_display_name(value));
            if crate::ui::tenxer_mode::chrome_suppressed() {
                with_filter_terms(query, |terms| terms.score(&name, 0).is_some())
            } else {
                filter_name_matches(&name, query)
            }
        })
}

pub(super) fn with_filter_terms<R>(query: &str, apply: impl FnOnce(&mut PathMatcher) -> R) -> R {
    FILTER_TERMS.with_borrow_mut(|cached| {
        if cached.as_ref().is_none_or(|(cached, _)| cached != query) {
            *cached = Some((query.to_owned(), PathMatcher::new(&PathQuery::parse(query))));
        }
        let (_, matcher) = cached.as_mut().expect("filter terms cached above");
        apply(matcher)
    })
}

static FILENAME_ICONS: &[(&str, &str)] = &[
    ("package-lock.json", icons::COG),
    ("npm-shrinkwrap.json", icons::COG),
    ("pnpm-lock.yaml", icons::COG),
    ("bun.lockb", icons::COG),
    ("cargo.lock", icons::COG),
    ("gemfile", icons::COG),
    ("go.mod", icons::COG),
    ("pom.xml", icons::COG),
    ("build.gradle", icons::COG),
    ("cmakelists.txt", icons::COG),
    ("dockerfile", icons::COG),
    ("makefile", icons::COG),
    (".gitignore", icons::COG),
    (".gitconfig", icons::COG),
    (".editorconfig", icons::COG),
    (".inputrc", icons::COG),
    (".npmrc", icons::COG),
    (".yarnrc", icons::COG),
    (".pypirc", icons::COG),
    (".xcompose", icons::COG),
    (".vimrc", icons::COG),
    (".gvimrc", icons::COG),
    (".viminfo", icons::COG),
    (".bashrc", icons::FILE_TERMINAL),
    (".bash_profile", icons::FILE_TERMINAL),
    (".bash_login", icons::FILE_TERMINAL),
    (".bash_logout", icons::FILE_TERMINAL),
    (".zshrc", icons::FILE_TERMINAL),
    (".zprofile", icons::FILE_TERMINAL),
    (".zlogin", icons::FILE_TERMINAL),
    (".zlogout", icons::FILE_TERMINAL),
    (".profile", icons::FILE_TERMINAL),
    (".login", icons::FILE_TERMINAL),
    (".logout", icons::FILE_TERMINAL),
    (".kshrc", icons::FILE_TERMINAL),
    (".cshrc", icons::FILE_TERMINAL),
    (".tcshrc", icons::FILE_TERMINAL),
    ("id_rsa", icons::KEY_ROUND),
    ("id_ed25519", icons::KEY_ROUND),
    ("authorized_keys", icons::KEY_ROUND),
    ("known_hosts", icons::KEY_ROUND),
    ("readme", icons::DOCUMENTS),
    ("license", icons::DOCUMENTS),
];

static FILENAME_AFFIX_PATTERNS: &[(&str, &str, &str)] = &[
    ("Dockerfile.", "", icons::COG),
    ("tsconfig.", ".json", icons::COG),
    ("", "_history", icons::FILE_TERMINAL),
    ("", ".lock", icons::COG),
];

fn exact_filename_icon(lowered: &str) -> Option<&'static str> {
    FILENAME_ICONS
        .iter()
        .find(|(name, _)| *name == lowered)
        .map(|(_, icon)| *icon)
}

fn affix_pattern_icon(name: &str) -> Option<&'static str> {
    FILENAME_AFFIX_PATTERNS
        .iter()
        .find(|(prefix, suffix, _)| name.starts_with(prefix) && name.ends_with(suffix))
        .map(|(_, _, icon)| *icon)
}

pub(in crate::ui) fn icon_for_name(name: &str) -> &'static str {
    let lowered = name.to_ascii_lowercase();
    if let Some(icon) = exact_filename_icon(&lowered) {
        return icon;
    }
    if let Some(icon) = affix_pattern_icon(name) {
        return icon;
    }
    let extension = lowered.rsplit_once('.').map(|(_, extension)| extension);
    match extension {
        Some(
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif" | "heic" | "heif"
            | "jxl" | "tif" | "tiff" | "3fr" | "arw" | "cr2" | "cr3" | "dcr" | "dng" | "erf"
            | "kdc" | "mef" | "mos" | "mrw" | "nef" | "nrw" | "orf" | "pef" | "raf" | "raw" | "rw2"
            | "rwl" | "sr2" | "srf" | "srw" | "x3f",
        ) => crate::assets::icons::PICTURES,
        Some("mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v") => crate::assets::icons::VIDEOS,
        Some("mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" | "wma" | "aiff") => {
            crate::assets::icons::FILE_AUDIO
        }
        Some("html" | "htm" | "css" | "scss" | "xml") => crate::assets::icons::GLOBE,
        Some("zip" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "rar") => {
            crate::assets::icons::FILE_ARCHIVE
        }
        Some("deb" | "rpm" | "pkg" | "appimage" | "msi" | "exe" | "apk") => {
            crate::assets::icons::BOX
        }
        Some("pem" | "crt" | "cer" | "key" | "der" | "csr" | "pub" | "p12" | "pfx" | "jks") => {
            crate::assets::icons::KEY_ROUND
        }
        Some("yaml" | "yml" | "toml" | "ini" | "conf" | "env") => crate::assets::icons::COG,
        Some("json" | "jsonc") => crate::assets::icons::FILE_BRACES,
        Some("db" | "sqlite" | "sqlite3" | "sql" | "psql" | "pgsql" | "mdb" | "accdb") => {
            crate::assets::icons::DATABASE
        }
        Some("iso" | "img" | "dmg" | "vhd" | "vhdx" | "vdi" | "qcow") => crate::assets::icons::DISC,
        Some("csv" | "tsv" | "xls" | "xlsx" | "ods") => icons::FILE_SPREADSHEET,
        Some(
            "rs" | "c" | "h" | "cpp" | "hpp" | "go" | "java" | "kt" | "swift" | "dart" | "scala"
            | "hs" | "lua" | "rb" | "php" | "py" | "js" | "ts" | "jsx" | "tsx" | "m" | "v" | "cs",
        ) => crate::assets::icons::FILE_CODE,
        Some("sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" | "ps1" | "bat" | "cmd") => {
            crate::assets::icons::FILE_TERMINAL
        }
        Some("ppt" | "pptx" | "pps" | "ppsx" | "odp") => icons::PRESENTATION,
        Some("ttf" | "otf" | "woff" | "woff2" | "eot" | "ttc" | "otc") => icons::FILE_TYPE,
        _ => icons::DOCUMENTS,
    }
}

pub(super) fn item_count_label(count: usize) -> String {
    crate::i18n::count("items", count)
}

pub(super) fn entry_kind_summary(entries: &[FileEntry]) -> String {
    let directories = entries.iter().filter(|entry| entry.is_directory()).count();
    let files = entries.len().saturating_sub(directories);
    match (files, directories) {
        (files, 0) => crate::i18n::count("files", files),
        (0, directories) => crate::i18n::count("folders", directories),
        _ => item_count_label(entries.len()),
    }
}

pub(super) async fn aggregate_directory_summary(entries: &[FileEntry]) -> DirectorySummary {
    let mut total = DirectorySummary::default();
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 && index % 256 == 0 {
            glib::timeout_future(std::time::Duration::ZERO).await;
        }
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

#[cfg(test)]
mod tests;
