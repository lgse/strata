// SPDX-License-Identifier: MIT

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

pub(crate) fn modified_cell(facts: &Facts) -> String {
    match facts.modified {
        Some(seconds) => modified_date_for_seconds(seconds, DateDisplay::Preferred),
        None => "—".to_owned(),
    }
}

pub(crate) fn modified_tooltip_date(facts: &Facts) -> Option<String> {
    facts
        .modified
        .map(|seconds| modified_date_for_seconds(seconds, DateDisplay::Preferred))
}

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

pub(crate) fn created_cell(facts: &Facts) -> String {
    facts
        .created
        .map(|seconds| modified_date_for_seconds(seconds, DateDisplay::Preferred))
        .unwrap_or_default()
}

pub(crate) fn duration_cell(folder: bool, facts: &Facts) -> String {
    if folder {
        return String::new();
    }
    facts.duration_seconds.map(duration).unwrap_or_default()
}

pub(crate) fn dimensions_cell(folder: bool, facts: &Facts) -> String {
    if folder {
        return String::new();
    }
    facts.dimensions.map(dimensions).unwrap_or_default()
}

pub(crate) fn type_cell(folder: &Location, kind: EntryKind, name: &str) -> String {
    match entry_type_of(folder, kind, name) {
        EntryType::Known(description) => description,
        other => tr(other.description()),
    }
}

#[cfg(test)]
mod tests;
