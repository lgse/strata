// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::Cell, cell::RefCell, time::Duration};

use gtk::prelude::*;

use crate::model::{FileEntry, MetadataValue};

struct ModifiedDateBinding {
    label: glib::WeakRef<gtk::Label>,
    seconds: i64,
}

thread_local! {
    static MODIFIED_DATE_BINDINGS: RefCell<Vec<ModifiedDateBinding>> = const { RefCell::new(Vec::new()) };
    static MODIFIED_DATE_TIMER_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

pub fn modified_date(entry: &FileEntry) -> String {
    let MetadataValue::Known(seconds) = entry.modified_unix_seconds else {
        return "—".to_owned();
    };
    modified_date_for_seconds(seconds)
}

pub fn set_modified_date(label: &gtk::Label, entry: Option<&FileEntry>, fallback: &str) {
    let seconds = entry.and_then(|entry| match entry.modified_unix_seconds {
        MetadataValue::Known(seconds) => Some(seconds),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    });
    let text = match (entry, seconds) {
        (Some(entry), Some(_)) => modified_date(entry),
        _ => fallback.to_owned(),
    };
    label.set_text(&text);

    MODIFIED_DATE_BINDINGS.with_borrow_mut(|bindings| {
        bindings.retain(|binding| {
            binding
                .label
                .upgrade()
                .is_some_and(|bound_label| bound_label != *label)
        });
        if let Some(seconds) = seconds {
            bindings.push(ModifiedDateBinding {
                label: label.downgrade(),
                seconds,
            });
        }
    });

    if seconds.is_some() {
        ensure_modified_date_timer();
    }
}

fn modified_date_for_seconds(seconds: i64) -> String {
    let Some(modified) = glib::DateTime::from_unix_local(seconds).ok() else {
        return "—".to_owned();
    };
    let Some(now) = glib::DateTime::now_local().ok() else {
        return modified
            .format("%Y-%m-%d %H:%M")
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned());
    };

    modified_date_at(&modified, &now)
}

fn ensure_modified_date_timer() {
    let already_active = MODIFIED_DATE_TIMER_ACTIVE.with(|active| active.replace(true));
    if already_active {
        return;
    }

    glib::timeout_add_local(Duration::from_secs(30), || {
        let live_bindings = MODIFIED_DATE_BINDINGS.with_borrow_mut(|bindings| {
            bindings.retain(|binding| binding.label.upgrade().is_some());
            bindings
                .iter()
                .filter_map(|binding| {
                    binding
                        .label
                        .upgrade()
                        .map(|label| (label, binding.seconds))
                })
                .collect::<Vec<_>>()
        });
        for (label, seconds) in &live_bindings {
            label.set_text(&modified_date_for_seconds(*seconds));
        }

        if live_bindings.is_empty() {
            MODIFIED_DATE_TIMER_ACTIVE.with(|active| active.set(false));
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn modified_date_at(modified: &glib::DateTime, now: &glib::DateTime) -> String {
    let span = now.difference(modified).0;
    if span < 0 {
        return modified
            .format("%Y-%m-%d %H:%M")
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned());
    }

    let day_diff = span / 86_400_000_000;
    let same_year = now.year() == modified.year();

    if day_diff == 0 {
        let hours = span / 3_600_000_000;
        let minutes = span / 60_000_000;
        if hours >= 1 {
            format!("{}h ago", hours)
        } else if minutes >= 1 {
            format!("{}m ago", minutes)
        } else {
            "just now".to_owned()
        }
    } else if day_diff == 1 {
        modified
            .format("%H:%M")
            .map(|s| format!("Yesterday, {}", s))
            .unwrap_or_else(|_| "—".to_owned())
    } else if day_diff < 7 {
        modified
            .format("%A %H:%M")
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned())
    } else if same_year {
        modified
            .format("%b %-d, %H:%M")
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned())
    } else {
        modified
            .format("%b %-d, %Y")
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned())
    }
}

#[cfg(test)]
mod tests;
