// SPDX-License-Identifier: MIT

use std::{cell::Cell, cell::RefCell, time::Duration};

use gtk::prelude::*;

use crate::model::{FileEntry, MetadataValue};

/// How modified timestamps render in listings and detail panes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DateFormat {
    /// "just now", "5m ago", "Yesterday, 14:30", then dated fallbacks.
    #[default]
    Relative,
    /// Always `2026-09-17 14:30`.
    Iso8601,
    /// Always `September 17, 2026, 14:30`.
    Long,
}

impl DateFormat {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "iso" | "iso8601" | "iso-8601" => Self::Iso8601,
            "long" => Self::Long,
            _ => Self::Relative,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Relative => "relative",
            Self::Iso8601 => "iso",
            Self::Long => "long",
        }
    }

    /// Absolute rendering for fixed formats; Relative uses it for
    /// far-future dates and last-resort fallbacks.
    fn absolute_pattern(&self) -> &'static str {
        match self {
            Self::Long => "%B %-d, %Y, %H:%M",
            Self::Relative | Self::Iso8601 => "%Y-%m-%d %H:%M",
        }
    }
}

struct ModifiedDateBinding {
    label: glib::WeakRef<gtk::Label>,
    seconds: i64,
}

thread_local! {
    static MODIFIED_DATE_BINDINGS: RefCell<Vec<ModifiedDateBinding>> = const { RefCell::new(Vec::new()) };
    /// Labels already listening for format changes; unlike the timestamp
    /// bindings this set is not cleared when a row is rebound without a date.
    static DATE_FORMAT_BOUND: RefCell<Vec<glib::WeakRef<gtk::Label>>> = const { RefCell::new(Vec::new()) };
    /// Pushed by `PreferenceManager` on load and save; reading `shared()` here
    /// would lazily run theme initialization inside list row binds.
    static MODIFIED_DATE_FORMAT: Cell<DateFormat> = const { Cell::new(DateFormat::Relative) };
    static MODIFIED_DATE_TIMER_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn set_date_format(format: DateFormat) {
    MODIFIED_DATE_FORMAT.with(|current| current.set(format));
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
        let unbound = DATE_FORMAT_BOUND.with_borrow_mut(|bound| {
            bound.retain(|weak| weak.upgrade().is_some());
            if bound
                .iter()
                .any(|weak| weak.upgrade().is_some_and(|bound| bound == *label))
            {
                false
            } else {
                bound.push(label.downgrade());
                true
            }
        });
        if unbound {
            bind_date_format(label);
        }
        ensure_modified_date_timer();
    }
}

/// Re-renders the label when the saved date format changes. Reads the latest
/// timestamp back out of the binding table so recycled rows stay correct.
fn bind_date_format(label: &gtk::Label) {
    crate::ui::preferences::PreferenceManager::shared().bind_preference(
        label,
        crate::ui::preferences::PreferenceManager::date_format,
        |widget, _| {
            let Some(label) = widget.downcast_ref::<gtk::Label>() else {
                return;
            };
            let seconds = MODIFIED_DATE_BINDINGS.with_borrow(|bindings| {
                bindings.iter().find_map(|binding| {
                    binding
                        .label
                        .upgrade()
                        .is_some_and(|bound| bound == *label)
                        .then_some(binding.seconds)
                })
            });
            if let Some(seconds) = seconds {
                label.set_text(&modified_date_for_seconds(seconds));
            }
        },
    );
}

/// Formats a recent sample timestamp for settings previews.
pub fn modified_date_example(format: DateFormat) -> String {
    let Ok(now) = glib::DateTime::now_local() else {
        return "—".to_owned();
    };
    let Ok(sample) = glib::DateTime::from_unix_local(now.to_unix() - 300) else {
        return "—".to_owned();
    };
    modified_date_at(&sample, &now, format)
}

fn modified_date_for_seconds(seconds: i64) -> String {
    let format = MODIFIED_DATE_FORMAT.with(Cell::get);
    let Some(modified) = glib::DateTime::from_unix_local(seconds).ok() else {
        return "—".to_owned();
    };
    let Some(now) = glib::DateTime::now_local().ok() else {
        return modified
            .format(format.absolute_pattern())
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned());
    };

    modified_date_at(&modified, &now, format)
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

fn calendar_day_difference(modified: &glib::DateTime, now: &glib::DateTime) -> Option<i64> {
    let midnight = |value: &glib::DateTime| {
        let (year, month, day) = value.ymd();
        glib::DateTime::new(&value.timezone(), year, month, day, 0, 0, 0.0).ok()
    };
    let span = midnight(now)?.difference(&midnight(modified)?).0;
    // Rounding maps 23- and 25-hour DST intervals to one civil day.
    Some((span + 43_200_000_000) / 86_400_000_000)
}

fn modified_date_at(modified: &glib::DateTime, now: &glib::DateTime, format: DateFormat) -> String {
    let absolute = |format: DateFormat| {
        modified
            .format(format.absolute_pattern())
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned())
    };
    if format != DateFormat::Relative {
        return absolute(format);
    }

    let span = now.difference(modified).0;
    // Timestamps slightly in the future are clock skew, not future files;
    // they fall through to "just now". Beyond a minute the date is shown.
    if span < -60_000_000 {
        return absolute(DateFormat::Relative);
    }

    // Sub-hour recency follows elapsed time: a file saved just before midnight
    // is still "2m ago" once the clock rolls over, not "Yesterday".
    let minutes = span / 60_000_000;
    if minutes < 60 {
        return if minutes >= 1 {
            format!("{minutes}m ago")
        } else {
            "just now".to_owned()
        };
    }

    let day_diff = calendar_day_difference(modified, now).unwrap_or(span / 86_400_000_000);
    let same_year = now.year() == modified.year();

    if day_diff == 0 {
        format!("{}h ago", span / 3_600_000_000)
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
