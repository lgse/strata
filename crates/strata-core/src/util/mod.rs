// SPDX-License-Identifier: MIT

pub mod ui_thread;

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

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

const CLOCK_CACHE_TTL: Duration = Duration::from_millis(250);
const CALENDAR_DAY_LOOKBACK_SECS: i64 = 31 * 86_400;

struct LocalClock {
    refreshed_at: Instant,
    tz: glib::TimeZone,
    now_unix: i64,
    now_year: i32,
    today_midnight_unix: Option<i64>,
}

impl LocalClock {
    fn sample() -> Option<Self> {
        let now = glib::DateTime::now_local().ok()?;
        let tz = now.timezone();
        let (year, month, day) = now.ymd();
        let today_midnight_unix = glib::DateTime::new(&tz, year, month, day, 0, 0, 0.0)
            .ok()
            .map(|dt| dt.to_unix());
        Some(Self {
            refreshed_at: Instant::now(),
            tz,
            now_unix: now.to_unix(),
            now_year: year,
            today_midnight_unix,
        })
    }

    fn is_fresh(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.refreshed_at) < CLOCK_CACHE_TTL
    }
}

thread_local! {
    // Avoid manager lookups on every row render.
    static MODIFIED_DATE_FORMAT: Cell<DateFormat> = const { Cell::new(DateFormat::Relative) };
    static LOCAL_CLOCK: RefCell<Option<LocalClock>> = const { RefCell::new(None) };
}

pub fn set_date_format(format: DateFormat) {
    MODIFIED_DATE_FORMAT.with(|current| current.set(format));
}

pub fn date_format() -> DateFormat {
    MODIFIED_DATE_FORMAT.with(Cell::get)
}

fn with_local_clock<R>(f: impl FnOnce(&LocalClock) -> R) -> Option<R> {
    let instant = Instant::now();
    LOCAL_CLOCK.with(|slot| {
        let mut guard = slot.borrow_mut();
        let stale = guard.as_ref().is_none_or(|clock| !clock.is_fresh(instant));
        if stale {
            *guard = LocalClock::sample();
        }
        guard.as_ref().map(f)
    })
}

fn format_iso8601(date: &glib::DateTime) -> String {
    let (year, month, day) = date.ymd();
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        date.hour(),
        date.minute()
    )
}

fn format_absolute(date: &glib::DateTime, format: DateFormat) -> String {
    match format {
        DateFormat::Long => localized_date(date, &format.absolute_pattern()),
        DateFormat::Relative | DateFormat::Iso8601 => format_iso8601(date),
    }
}

fn local_date_from_unix(seconds: i64, tz: &glib::TimeZone) -> Option<glib::DateTime> {
    glib::DateTime::from_unix_utc(seconds)
        .ok()?
        .to_timezone(tz)
        .ok()
}

fn recent_relative(diff_seconds: i64) -> Option<String> {
    if !(-60..86_400).contains(&diff_seconds) {
        return None;
    }
    if diff_seconds < 60 {
        return Some(crate::i18n::tr("Just now"));
    }
    let minutes = diff_seconds / 60;
    if minutes < 60 {
        return Some(rust_i18n::t!("%{count}m ago", count = minutes).into_owned());
    }
    let hours = diff_seconds / 3_600;
    Some(rust_i18n::t!("%{count}h ago", count = hours).into_owned())
}

fn relative_from_clock(seconds: i64, clock: &LocalClock) -> String {
    let diff = clock.now_unix.saturating_sub(seconds);
    if let Some(recent) = recent_relative(diff) {
        return recent;
    }
    let Some(modified) = local_date_from_unix(seconds, &clock.tz) else {
        return "—".to_owned();
    };
    if diff < -60 {
        return format_iso8601(&modified);
    }
    let recent_enough_for_days = clock
        .today_midnight_unix
        .is_none_or(|midnight| seconds >= midnight.saturating_sub(CALENDAR_DAY_LOOKBACK_SECS));
    if recent_enough_for_days {
        let day_diff = calendar_day_difference_with_midnight(&modified, clock.today_midnight_unix)
            .unwrap_or(diff / 86_400);
        if day_diff == 0 {
            return rust_i18n::t!("%{count}h ago", count = diff / 3_600).into_owned();
        }
        if day_diff <= 6 {
            return crate::i18n::tr(&format!("dates.weekday.{}", modified.day_of_week()));
        }
        if day_diff <= 30 {
            return rust_i18n::t!("%{count}w ago", count = day_diff / 7).into_owned();
        }
    }
    if clock.now_year == modified.year() {
        localized_date(&modified, &crate::i18n::tr("dates.recent"))
    } else {
        localized_date(&modified, &crate::i18n::tr("dates.older"))
    }
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
    let format = date_format();
    if let Some(formatted) = with_local_clock(|clock| match display {
        DateDisplay::Full => {
            let Some(modified) = local_date_from_unix(seconds, &clock.tz) else {
                return "—".to_owned();
            };
            match format {
                DateFormat::Relative => localized_date(&modified, &crate::i18n::tr("dates.full")),
                _ => format_absolute(&modified, format),
            }
        }
        DateDisplay::Preferred if format == DateFormat::Relative => {
            relative_from_clock(seconds, clock)
        }
        DateDisplay::Preferred => local_date_from_unix(seconds, &clock.tz)
            .map_or_else(|| "—".to_owned(), |dt| format_absolute(&dt, format)),
    }) {
        return formatted;
    }
    let Ok(modified) = glib::DateTime::from_unix_local(seconds) else {
        return "—".to_owned();
    };
    format_absolute(&modified, format)
}

pub fn localized_date(date: &glib::DateTime, pattern: &str) -> String {
    use std::borrow::Cow;
    if pattern == "%Y-%m-%d %H:%M" {
        return format_iso8601(date);
    }
    // GLib's named months follow the process locale, not the saved app language.
    let mut current = Cow::Borrowed(pattern);
    if current.contains("%B") {
        current = Cow::Owned(current.replace(
            "%B",
            &crate::i18n::tr(&format!("dates.month.{}", date.month())),
        ));
    }
    if current.contains("%b") {
        current = Cow::Owned(current.replace(
            "%b",
            &crate::i18n::tr(&format!("dates.short_month.{}", date.month())),
        ));
    }
    if current.contains("%p") {
        current = Cow::Owned(current.replace("%p", if date.hour() < 12 { "AM" } else { "PM" }));
    }
    date.format(&current)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "—".to_owned())
}

fn calendar_day_difference_with_midnight(
    modified: &glib::DateTime,
    now_midnight_unix: Option<i64>,
) -> Option<i64> {
    let now_midnight = now_midnight_unix?;
    let (year, month, day) = modified.ymd();
    let mod_midnight = glib::DateTime::new(&modified.timezone(), year, month, day, 0, 0, 0.0)
        .ok()?
        .to_unix();
    let span_secs = now_midnight.checked_sub(mod_midnight)?;
    // Rounding maps 23- and 25-hour DST intervals to one civil day.
    Some((span_secs + 43_200) / 86_400)
}

fn calendar_day_difference(modified: &glib::DateTime, now: &glib::DateTime) -> Option<i64> {
    let (year, month, day) = now.ymd();
    let now_midnight = glib::DateTime::new(&now.timezone(), year, month, day, 0, 0, 0.0)
        .ok()?
        .to_unix();
    calendar_day_difference_with_midnight(modified, Some(now_midnight))
}

fn modified_date_at(modified: &glib::DateTime, now: &glib::DateTime, format: DateFormat) -> String {
    let converted = modified.to_timezone(&now.timezone());
    let modified = converted.as_ref().unwrap_or(modified);
    if format != DateFormat::Relative {
        return format_absolute(modified, format);
    }

    let span = now.difference(modified).0;
    // Tolerate up to one minute of clock skew.
    if span < -60_000_000 {
        return format_iso8601(modified);
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
