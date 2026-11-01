// SPDX-License-Identifier: MIT

use crate::ui::browser::entry::format_file_size;

/// `None` while the transfer still prepares its items.
pub(super) fn transfer_count(
    completed_items: usize,
    total_items: usize,
    completed_files: usize,
    total_files: Option<usize>,
) -> Option<(usize, usize, &'static str)> {
    match total_files.filter(|total| *total > 0) {
        Some(total_files) => Some((completed_files, total_files, "files")),
        None => (total_items > 0).then_some((completed_items, total_items, "items")),
    }
}

pub(super) fn transfer_progress_status(
    completed_items: usize,
    total_items: usize,
    completed_files: usize,
    total_files: Option<usize>,
    transferred_bytes: u64,
    total_bytes: Option<u64>,
    current_file: Option<&str>,
) -> (String, String, String, Option<f64>) {
    let count = transfer_count(completed_items, total_items, completed_files, total_files);
    let items = transfer_items_text(count, current_file);
    let bytes = transfer_bytes_text(transferred_bytes, total_bytes);
    let (status, fraction) =
        transfer_fraction_status(completed_items, total_items, transferred_bytes, total_bytes);
    (status, bytes, items, fraction)
}

fn transfer_items_text(
    count: Option<(usize, usize, &'static str)>,
    current_file: Option<&str>,
) -> String {
    let items = match count {
        Some((completed, total, kind)) => rust_i18n::t!(
            "%{completed} of %{items}",
            completed = crate::i18n::integer(completed as u64),
            items = crate::i18n::count(kind, total)
        )
        .into_owned(),
        None => crate::i18n::tr("Preparing items…"),
    };
    match current_file.filter(|name| !name.is_empty()) {
        Some(name) => format!("{items} · {name}"),
        None => items,
    }
}

fn transfer_bytes_text(transferred_bytes: u64, total_bytes: Option<u64>) -> String {
    match total_bytes {
        Some(total) => format!(
            "{} / {}",
            format_file_size(transferred_bytes),
            format_file_size(total)
        ),
        None => format_file_size(transferred_bytes),
    }
}

fn transfer_fraction_status(
    completed_items: usize,
    total_items: usize,
    transferred_bytes: u64,
    total_bytes: Option<u64>,
) -> (String, Option<f64>) {
    match total_bytes {
        Some(0) if total_items > 0 => {
            let fraction = (completed_items as f64 / total_items as f64).clamp(0.0, 1.0);
            (
                crate::i18n::percent((fraction * 100.0) as usize),
                Some(fraction),
            )
        }
        Some(0) => (crate::i18n::tr("Preparing…"), None),
        Some(total) => {
            let fraction = (transferred_bytes as f64 / total as f64).clamp(0.0, 1.0);
            let percentage = (fraction * 100.0) as usize;
            (
                crate::i18n::percent(if transferred_bytes > 0 {
                    percentage.max(1)
                } else {
                    percentage
                }),
                Some(fraction),
            )
        }
        None if transferred_bytes == 0 && completed_items == 0 => {
            (crate::i18n::tr("Preparing…"), None)
        }
        None => (crate::i18n::tr("Transferring…"), None),
    }
}

pub(super) fn transfer_rate_status(
    rate: Option<f64>,
    transferred: u64,
    total: Option<u64>,
) -> String {
    let Some(rate) = rate.filter(|rate| rate.is_finite() && *rate > 0.0) else {
        return crate::i18n::tr("Calculating speed…");
    };
    let speed = crate::i18n::transfer_rate(rate as u64);
    let Some(remaining) = total.and_then(|total| total.checked_sub(transferred)) else {
        return speed;
    };
    if remaining == 0 {
        return speed;
    }
    let seconds = (remaining as f64 / rate).ceil().max(1.0) as u64;
    rust_i18n::t!(
        "%{speed} · %{time} left",
        speed = speed,
        time = crate::i18n::duration(seconds)
    )
    .into_owned()
}

pub(in crate::ui::browser) const TRANSFER_SUBTITLE: &str =
    "Cancelling will not undo completed changes";

pub(in crate::ui::browser) fn transfer_heading(moving: bool) -> (&'static str, &'static str) {
    if moving {
        (crate::assets::icons::FOLDER, "Moving items")
    } else {
        (crate::assets::icons::COPY, "Copying items")
    }
}

pub(in crate::ui::browser) fn cancelled_operation_summary(
    completed: usize,
    failed: usize,
    not_attempted: usize,
) -> String {
    rust_i18n::t!(
        "%{results}.\n\nCompleted changes were not reverted.",
        results = crate::i18n::capitalize_first(crate::i18n::list([
            crate::i18n::count("items_completed", completed),
            crate::i18n::count("items_failed", failed),
            crate::i18n::count("items_not_attempted", not_attempted),
        ]))
    )
    .into_owned()
}
