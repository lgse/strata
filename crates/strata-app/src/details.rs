// SPDX-License-Identifier: MIT

//! The text a view shows of a row's [`Facts`]: the details line under an Icons tile, and the
//! List view's Mode, Size, Type and Modified cells. Shared code: both views call it, and a component
//! gets the finished text as data. Text comes from the catalogs only: sizes through
//! `strata_core::i18n::file_size`, the plural of "items" through `count_u64`.
//!
//! A port of GTK's `src/ui/browser_modes/metadata.rs` (`format_duration`, `folder_item_info`,
//! `entry_icons_item_info`) at `008d7e124` on `strata-next`, the same file as `b97813450` on
//! `main`, as `docs/rune/parity/packages/A1.md`, "Ported from GTK", lists it. GTK's
//! `format_file_size` is `strata_core::i18n::file_size` already. The List view's cells
//! (`entry_mode`, `entry_size`, `entry_type` in the same file, and `format_permissions` in
//! `src/ui/browser/properties.rs`) are ported at `008d7e124` on `strata-next`, as
//! `docs/rune/parity/packages/A2.md` lists them; the Modified cell is
//! `strata_core::util::modified_date_for_seconds`, shared already.

use strata_core::{
    i18n::{count_u64, file_size, tr},
    model::{EntryKind, Location},
    util::{DateDisplay, modified_date_for_seconds},
};
use strata_fs::services::EntryType;

use crate::listing::{Facts, entry_type_of};

/// A length of time as a clock shows it: `m:ss`, or `h:mm:ss` from an hour.
pub(crate) fn duration(seconds: u64) -> String {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if hours == 0 {
        format!("{minutes}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

/// An image's size in pixels, `320×180`.
pub(crate) fn dimensions((width, height): (u32, u32)) -> String {
    format!("{width}×{height}")
}

/// How many entries a folder holds: "No items", or the count with its plural.
pub(crate) fn items(facts: &Facts) -> Option<String> {
    match facts.child_count? {
        0 => Some(tr("No items")),
        count => Some(count_u64("items", count)),
    }
}

/// The details line under an Icons tile, in GTK's order: a folder's item count; else a
/// duration, an image's dimensions or the size, the first the browser knows.
pub(crate) fn icons_line(folder: bool, facts: &Facts) -> Option<String> {
    if folder {
        return items(facts);
    }
    if let Some(seconds) = facts.duration_seconds {
        return Some(duration(seconds));
    }
    if let Some(size) = facts.dimensions {
        return Some(dimensions(size));
    }
    facts.size.map(file_size)
}

/// Permission bits as the List view's Mode column shows them: `drwxr-xr-x  755`
/// (`format_permissions` in `src/ui/browser/properties.rs`).
pub(crate) fn permissions(mode: u32) -> String {
    let kind = if mode & 0o170_000 == 0o040_000 {
        'd'
    } else {
        '-'
    };
    let mut symbolic = String::with_capacity(10);
    symbolic.push(kind);
    for (mask, character) in [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ] {
        symbolic.push(if mode & mask != 0 { character } else { '-' });
    }
    format!("{symbolic}  {:03o}", mode & 0o777)
}

/// The Mode cell: the permissions, or nothing the browser does not know (`entry_mode`).
pub(crate) fn mode_cell(facts: &Facts) -> String {
    facts.mode.map(permissions).unwrap_or_default()
}

/// The Size cell: a folder's item count, else a file's size, or nothing (`entry_size`).
pub(crate) fn size_cell(folder: bool, facts: &Facts) -> String {
    if folder {
        return items(facts).unwrap_or_default();
    }
    facts.size.map(file_size).unwrap_or_default()
}

/// The Modified cell: the date in the saved format, or a dash
/// (`strata_core::util::modified_date`).
pub(crate) fn modified_cell(facts: &Facts) -> String {
    match facts.modified {
        Some(seconds) => modified_date_for_seconds(seconds, DateDisplay::Preferred),
        None => "—".to_owned(),
    }
}

/// Formatted modified date for a hover tooltip (absent when unknown), matching GTK's
/// `set_modified_date(..., "")` in `src/ui/browser/columns/rows/bind.rs` and
/// `src/ui/browser_modes/metadata.rs`.
pub(crate) fn modified_tooltip_date(facts: &Facts) -> Option<String> {
    facts
        .modified
        .map(|seconds| modified_date_for_seconds(seconds, DateDisplay::Preferred))
}

/// Hover tooltip text for an entry row or tile, matching GTK's
/// `src/ui/entry_tooltip.rs::name_text`:
/// - `"{name}\n{date}"` when a non-empty modified date is included for the view,
/// - `"{name}"` when no date is included and the displayed filename is truncated,
/// - `None` when no date is included and the full filename fits.
pub(crate) fn entry_tooltip_text(
    name: &str,
    truncated: bool,
    date: Option<&str>,
) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    match date.filter(|date| !date.is_empty()) {
        Some(date) => Some(format!("{name}\n{date}")),
        None if truncated => Some(name.to_owned()),
        None => None,
    }
}

/// Hover tooltip for an entry with `facts`: formats the modified date when `include_date`
/// holds; otherwise only allocates `name` when `truncated()` reports elision.
pub(crate) fn entry_tooltip(
    name: &str,
    facts: Option<&Facts>,
    include_date: bool,
    truncated: impl FnOnce() -> bool,
) -> Option<String> {
    let date = include_date
        .then(|| facts.and_then(modified_tooltip_date))
        .flatten();
    let is_truncated = date.as_deref().is_none_or(str::is_empty) && truncated();
    entry_tooltip_text(name, is_truncated, date.as_deref())
}

/// The Created cell: the creation date in the saved format, or nothing when unknown
/// (`update_list_date` in `src/ui/browser_modes/metadata.rs`).
pub(crate) fn created_cell(facts: &Facts) -> String {
    facts
        .created
        .map(|seconds| modified_date_for_seconds(seconds, DateDisplay::Preferred))
        .unwrap_or_default()
}

/// The Duration cell: a file's media length, or nothing for a folder or unknown value
/// (`list_cell_text` in `src/ui/browser_modes/metadata.rs`).
pub(crate) fn duration_cell(folder: bool, facts: &Facts) -> String {
    if folder {
        return String::new();
    }
    facts.duration_seconds.map(duration).unwrap_or_default()
}

/// The Dimensions cell: a file's width and height (`320×180`), or nothing for a folder or
/// unknown value (`list_cell_text` in `src/ui/browser_modes/metadata.rs`).
pub(crate) fn dimensions_cell(folder: bool, facts: &Facts) -> String {
    if folder {
        return String::new();
    }
    facts.dimensions.map(dimensions).unwrap_or_default()
}

/// The Type cell: the MIME type's description, or the kind's in the catalogs (`entry_type`).
/// Read for the rows drawn only: it is a MIME lookup.
pub(crate) fn type_cell(folder: &Location, kind: EntryKind, name: &str) -> String {
    match entry_type_of(folder, kind, name) {
        EntryType::Known(description) => description,
        other => tr(other.description()),
    }
}

#[cfg(test)]
mod tests;
