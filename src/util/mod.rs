// SPDX-License-Identifier: MIT

use std::{cell::Cell, cell::RefCell, time::Duration};

use gtk::prelude::*;

use crate::model::{FileEntry, MetadataValue};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DateFormat {
    #[default]
    Relative,
    Iso8601,
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

    fn absolute_pattern(&self) -> &'static str {
        match self {
            Self::Long => "%B %-d, %Y, %H:%M",
            Self::Relative | Self::Iso8601 => "%Y-%m-%d %H:%M",
        }
    }
}

#[derive(Clone, Copy)]
enum DateDisplay {
    Preferred,
    Full,
}

struct ModifiedDateBinding {
    label: glib::WeakRef<gtk::Label>,
    seconds: i64,
    display: DateDisplay,
}

thread_local! {
    static MODIFIED_DATE_BINDINGS: RefCell<Vec<ModifiedDateBinding>> = const { RefCell::new(Vec::new()) };
    // Keep subscriptions when recycled rows temporarily have no timestamp.
    static DATE_FORMAT_BOUND: RefCell<Vec<glib::WeakRef<gtk::Label>>> = const { RefCell::new(Vec::new()) };
    // Avoid manager lookups on every row render.
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
    modified_date_for_seconds(seconds, DateDisplay::Preferred)
}

pub(crate) fn set_full_modified_date(label: &gtk::Label, seconds: Option<i64>) {
    bind_modified_date(label, seconds, "—", DateDisplay::Full);
}

pub fn set_modified_date(label: &gtk::Label, entry: Option<&FileEntry>, fallback: &str) {
    let seconds = entry.and_then(|entry| match entry.modified_unix_seconds {
        MetadataValue::Known(seconds) => Some(seconds),
        MetadataValue::Unknown | MetadataValue::Unavailable => None,
    });
    bind_modified_date(label, seconds, fallback, DateDisplay::Preferred);
}

fn bind_modified_date(
    label: &gtk::Label,
    seconds: Option<i64>,
    fallback: &str,
    display: DateDisplay,
) {
    let text = seconds
        .map(|seconds| modified_date_for_seconds(seconds, display))
        .unwrap_or_else(|| fallback.to_owned());
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
                display,
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

// Resolve the current timestamp at notification time because rows are recycled.
fn bind_date_format(label: &gtk::Label) {
    crate::ui::preferences::PreferenceManager::shared().bind_preference(
        label,
        crate::ui::preferences::PreferenceManager::date_format,
        |widget, _| {
            let Some(label) = widget.downcast_ref::<gtk::Label>() else {
                return;
            };
            let value = MODIFIED_DATE_BINDINGS.with_borrow(|bindings| {
                bindings.iter().find_map(|binding| {
                    binding
                        .label
                        .upgrade()
                        .is_some_and(|bound| bound == *label)
                        .then_some((binding.seconds, binding.display))
                })
            });
            if let Some((seconds, display)) = value {
                label.set_text(&modified_date_for_seconds(seconds, display));
            }
        },
    );
}

pub fn modified_date_example(format: DateFormat) -> String {
    let Ok(now) = glib::DateTime::now_local() else {
        return "—".to_owned();
    };
    let Ok(sample) = glib::DateTime::from_unix_local(now.to_unix() - 300) else {
        return "—".to_owned();
    };
    modified_date_at(&sample, &now, format)
}

fn modified_date_for_seconds(seconds: i64, display: DateDisplay) -> String {
    let format = MODIFIED_DATE_FORMAT.with(Cell::get);
    let Some(modified) = glib::DateTime::from_unix_local(seconds).ok() else {
        return "—".to_owned();
    };
    if matches!(display, DateDisplay::Full) {
        let pattern = match format {
            DateFormat::Relative => "%b %-d, %Y, %-I:%M %p",
            _ => format.absolute_pattern(),
        };
        return modified
            .format(pattern)
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned());
    }
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
                        .map(|label| (label, binding.seconds, binding.display))
                })
                .collect::<Vec<_>>()
        });
        for (label, seconds, display) in &live_bindings {
            label.set_text(&modified_date_for_seconds(*seconds, *display));
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
    let converted = modified.to_timezone(&now.timezone());
    let modified = converted.as_ref().unwrap_or(modified);
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
    // Tolerate up to one minute of clock skew.
    if span < -60_000_000 {
        return absolute(DateFormat::Relative);
    }

    let seconds = span / 1_000_000;
    if seconds < 60 {
        return "Just now".to_owned();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = span / 3_600_000_000;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let day_diff = calendar_day_difference(modified, now).unwrap_or(span / 86_400_000_000);
    if day_diff == 0 {
        // A fall-back day can exceed 24 elapsed hours before local midnight.
        return format!("{hours}h ago");
    }
    if day_diff <= 6 {
        modified
            .format("%A")
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "—".to_owned())
    } else if day_diff <= 30 {
        format!("{}w ago", day_diff / 7)
    } else if now.year() == modified.year() {
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
