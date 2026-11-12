// SPDX-License-Identifier: MIT

pub mod ui_thread;

use std::cell::Cell;

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

    fn absolute_pattern(&self) -> String {
        match self {
            Self::Long => crate::i18n::tr("dates.long"),
            Self::Relative | Self::Iso8601 => "%Y-%m-%d %H:%M".to_owned(),
        }
    }
}

#[derive(Clone, Copy)]
pub enum DateDisplay {
    Preferred,
    Full,
}

thread_local! {
    // Avoid manager lookups on every row render.
    static MODIFIED_DATE_FORMAT: Cell<DateFormat> = const { Cell::new(DateFormat::Relative) };
}

pub fn set_date_format(format: DateFormat) {
    MODIFIED_DATE_FORMAT.with(|current| current.set(format));
}

pub fn modified_date(entry: &FileEntry) -> String {
    let MetadataValue::Known(seconds) = entry.modified_unix_seconds else {
        return "—".to_owned();
    };
    modified_date_for_seconds(seconds, DateDisplay::Preferred)
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

pub fn modified_date_for_seconds(seconds: i64, display: DateDisplay) -> String {
    let format = MODIFIED_DATE_FORMAT.with(Cell::get);
    let Some(modified) = glib::DateTime::from_unix_local(seconds).ok() else {
        return "—".to_owned();
    };
    if matches!(display, DateDisplay::Full) {
        let pattern = match format {
            DateFormat::Relative => crate::i18n::tr("dates.full"),
            _ => format.absolute_pattern(),
        };
        return localized_date(&modified, &pattern);
    }
    let Some(now) = glib::DateTime::now_local().ok() else {
        return localized_date(&modified, &format.absolute_pattern());
    };

    modified_date_at(&modified, &now, format)
}

pub fn localized_date(date: &glib::DateTime, pattern: &str) -> String {
    // GLib's named months follow the process locale, not the saved app language.
    let pattern = pattern
        .replace(
            "%B",
            &crate::i18n::tr(&format!("dates.month.{}", date.month())),
        )
        .replace(
            "%b",
            &crate::i18n::tr(&format!("dates.short_month.{}", date.month())),
        )
        .replace("%p", if date.hour() < 12 { "AM" } else { "PM" });
    date.format(&pattern)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "—".to_owned())
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
    let absolute = |format: DateFormat| localized_date(modified, &format.absolute_pattern());
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
        return crate::i18n::tr("Just now");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return rust_i18n::t!("%{count}m ago", count = minutes).into_owned();
    }
    let hours = span / 3_600_000_000;
    if hours < 24 {
        return rust_i18n::t!("%{count}h ago", count = hours).into_owned();
    }
    let day_diff = calendar_day_difference(modified, now).unwrap_or(span / 86_400_000_000);
    if day_diff == 0 {
        // A fall-back day can exceed 24 elapsed hours before local midnight.
        return rust_i18n::t!("%{count}h ago", count = hours).into_owned();
    }
    if day_diff <= 6 {
        crate::i18n::tr(&format!("dates.weekday.{}", modified.day_of_week()))
    } else if day_diff <= 30 {
        rust_i18n::t!("%{count}w ago", count = day_diff / 7).into_owned()
    } else if now.year() == modified.year() {
        localized_date(modified, &crate::i18n::tr("dates.recent"))
    } else {
        localized_date(modified, &crate::i18n::tr("dates.older"))
    }
}

#[cfg(test)]
mod tests;
