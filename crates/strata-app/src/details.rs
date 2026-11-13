// SPDX-License-Identifier: MIT

//! The text a view shows of a row's [`Facts`]: the details line under an Icons tile, and the
//! List view's size column when it comes. Shared code: both views call it, and a component
//! gets the finished text as data. Text comes from the catalogs only: sizes through
//! `strata_core::i18n::file_size`, the plural of "items" through `count_u64`.
//!
//! A port of GTK's `src/ui/browser_modes/metadata.rs` (`format_duration`, `folder_item_info`,
//! `entry_icons_item_info`) at `008d7e124` on `strata-next`, the same file as `b97813450` on
//! `main`, as `docs/rune/parity/packages/A1.md`, "Ported from GTK", lists it. GTK's
//! `format_file_size` is `strata_core::i18n::file_size` already.

use strata_core::i18n::{count_u64, file_size, tr};

use crate::listing::Facts;

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

#[cfg(test)]
mod tests;
