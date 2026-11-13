// SPDX-License-Identifier: MIT

//! The text a view shows of a row's [`Facts`]: the details line under an Icons tile, and the
//! List view's size column when it comes. Shared code: both views call it, and a component
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the List view's columns read it, in the pull request after this one"
    )
)]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the List view's columns read it, in the pull request after this one"
    )
)]
pub(crate) fn mode_cell(facts: &Facts) -> String {
    facts.mode.map(permissions).unwrap_or_default()
}

/// The Size cell: a folder's item count, else a file's size, or nothing (`entry_size`).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the List view's columns read it, in the pull request after this one"
    )
)]
pub(crate) fn size_cell(folder: bool, facts: &Facts) -> String {
    if folder {
        return items(facts).unwrap_or_default();
    }
    facts.size.map(file_size).unwrap_or_default()
}

/// The Modified cell: the date in the saved format, or a dash
/// (`strata_core::util::modified_date`).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the List view's columns read it, in the pull request after this one"
    )
)]
pub(crate) fn modified_cell(facts: &Facts) -> String {
    match facts.modified {
        Some(seconds) => modified_date_for_seconds(seconds, DateDisplay::Preferred),
        None => "—".to_owned(),
    }
}

/// The Type cell: the MIME type's description, or the kind's in the catalogs (`entry_type`).
/// Read for the rows drawn only: it is a MIME lookup.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the List view's columns read it, in the pull request after this one"
    )
)]
pub(crate) fn type_cell(folder: &Location, kind: EntryKind, name: &str) -> String {
    match entry_type_of(folder, kind, name) {
        EntryType::Known(description) => description,
        other => tr(other.description()),
    }
}

#[cfg(test)]
mod tests;
