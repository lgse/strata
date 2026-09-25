// SPDX-License-Identifier: MIT

use super::{DateFormat, modified_date_at};

fn date_in_timezone(
    timezone: &glib::TimeZone,
    year: i32,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
) -> glib::DateTime {
    glib::DateTime::new(timezone, year, month, day, hour, minute, 0.0).expect("valid test date")
}

fn utc_date(year: i32, month: i32, day: i32, hour: i32, minute: i32) -> glib::DateTime {
    date_in_timezone(&glib::TimeZone::utc(), year, month, day, hour, minute)
}

#[test]
fn date_format_parsing_tolerates_hand_edited_values() {
    for (value, expected) in [
        ("relative", DateFormat::Relative),
        ("iso", DateFormat::Iso8601),
        ("iso8601", DateFormat::Iso8601),
        ("ISO-8601", DateFormat::Iso8601),
        (" Iso ", DateFormat::Iso8601),
        ("long", DateFormat::Long),
        ("", DateFormat::Relative),
        ("garbage", DateFormat::Relative),
    ] {
        assert_eq!(DateFormat::parse(value), expected, "{value:?}");
    }
}

#[test]
fn relative_time_boundaries_survive_midnight_and_clock_skew() {
    let now = utc_date(2026, 9, 8, 0, 0);
    for (seconds_ago, expected) in [
        (-61, "2026-09-08 00:01"),
        (-60, "Just now"),
        (0, "Just now"),
        (59, "Just now"),
        (60, "1m ago"),
        (3599, "59m ago"),
        (3600, "1h ago"),
        (86399, "23h ago"),
        (86400, "Monday"),
    ] {
        let modified = now.add_seconds(-f64::from(seconds_ago)).expect("offset");
        assert_eq!(
            modified_date_at(&modified, &now, DateFormat::Relative),
            expected,
            "{seconds_ago} seconds ago"
        );
    }
}

#[test]
fn older_relative_dates_follow_calendar_boundaries() {
    let now = utc_date(2026, 9, 8, 0, 0);
    for (modified, expected) in [
        (utc_date(2026, 9, 2, 23, 59), "Wednesday"),
        (utc_date(2026, 9, 1, 23, 59), "1w ago"),
        (utc_date(2026, 8, 26, 23, 59), "1w ago"),
        (utc_date(2026, 8, 25, 23, 59), "2w ago"),
        (utc_date(2026, 8, 9, 23, 59), "4w ago"),
        (utc_date(2026, 8, 8, 23, 59), "Aug 8, 23:59"),
        (utc_date(2025, 9, 8, 23, 59), "Sep 8, 2025"),
    ] {
        assert_eq!(
            modified_date_at(&modified, &now, DateFormat::Relative),
            expected
        );
    }
    assert_eq!(
        modified_date_at(
            &utc_date(2026, 12, 31, 12, 0),
            &utc_date(2027, 1, 1, 12, 0),
            DateFormat::Relative
        ),
        "Thursday"
    );
}

#[test]
fn saved_formats_always_render_absolute() {
    let now = utc_date(2026, 9, 8, 0, 0);
    let recent = utc_date(2026, 9, 7, 23, 59);
    let future = utc_date(2026, 9, 8, 0, 30);
    for (format, expected_recent, expected_future) in [
        (DateFormat::Iso8601, "2026-09-07 23:59", "2026-09-08 00:30"),
        (
            DateFormat::Long,
            "September 7, 2026, 23:59",
            "September 8, 2026, 00:30",
        ),
    ] {
        assert_eq!(modified_date_at(&recent, &now, format), expected_recent);
        assert_eq!(modified_date_at(&future, &now, format), expected_future);
    }
}

#[test]
fn daylight_saving_uses_elapsed_hours_then_calendar_days() {
    for (zone, modified, now, expected) in [
        (
            "America/New_York",
            (2026, 3, 8, 1, 50),
            (2026, 3, 8, 3, 10),
            "20m ago",
        ),
        (
            "America/New_York",
            (2026, 3, 7, 12, 0),
            (2026, 3, 8, 12, 0),
            "23h ago",
        ),
        (
            "America/New_York",
            (2026, 11, 1, 0, 1),
            (2026, 11, 1, 23, 59),
            "24h ago",
        ),
        (
            "America/New_York",
            (2026, 11, 1, 0, 1),
            (2026, 11, 2, 0, 1),
            "Sunday",
        ),
        (
            "America/New_York",
            (2026, 3, 2, 12, 0),
            (2026, 3, 9, 12, 0),
            "1w ago",
        ),
        (
            "Australia/Lord_Howe",
            (2026, 4, 5, 1, 20),
            (2026, 4, 5, 2, 10),
            "1h ago",
        ),
    ] {
        let timezone = glib::TimeZone::from_identifier(Some(zone)).expect("timezone");
        let date = |(year, month, day, hour, minute)| {
            date_in_timezone(&timezone, year, month, day, hour, minute)
        };
        assert_eq!(
            modified_date_at(&date(modified), &date(now), DateFormat::Relative),
            expected,
            "{zone}: {modified:?} -> {now:?}"
        );
    }
}

#[test]
fn calendar_rendering_converts_entries_into_now_timezone() {
    let new_york =
        glib::TimeZone::from_identifier(Some("America/New_York")).expect("America/New_York");
    let modified = utc_date(2026, 9, 7, 3, 30);
    let now = date_in_timezone(&new_york, 2026, 9, 8, 3, 30);
    for (format, expected) in [
        (DateFormat::Relative, "Sunday"),
        (DateFormat::Iso8601, "2026-09-06 23:30"),
    ] {
        assert_eq!(modified_date_at(&modified, &now, format), expected);
    }
}
