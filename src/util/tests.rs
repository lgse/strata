// SPDX-License-Identifier: MIT

use super::{DateFormat, calendar_day_difference, modified_date_at};

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
fn future_modified_dates_use_an_absolute_timestamp() {
    let now = utc_date(2026, 9, 3, 12, 0);
    let modified = utc_date(2026, 9, 3, 13, 0);

    assert_eq!(
        modified_date_at(&modified, &now, DateFormat::Relative),
        "2026-09-03 13:00"
    );
}

#[test]
fn slight_future_timestamps_are_clock_skew_not_future_files() {
    let now = utc_date(2026, 9, 3, 12, 0);
    let skew = utc_date(2026, 9, 3, 12, 0).add_seconds(30.0).expect("skew");
    let just_past = utc_date(2026, 9, 3, 12, 1)
        .add_seconds(-1.0)
        .expect("minute");
    let minute_future = utc_date(2026, 9, 3, 12, 1)
        .add_seconds(1.0)
        .expect("future");

    assert_eq!(
        modified_date_at(&skew, &now, DateFormat::Relative),
        "just now"
    );
    assert_eq!(
        modified_date_at(&just_past, &now, DateFormat::Relative),
        "just now"
    );
    assert_eq!(
        modified_date_at(&minute_future, &now, DateFormat::Relative),
        "2026-09-03 12:01"
    );
    assert_eq!(
        modified_date_at(&skew, &now, DateFormat::Iso8601),
        "2026-09-03 12:00"
    );
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
fn recent_past_modified_dates_remain_relative() {
    let now = utc_date(2026, 9, 3, 12, 0);
    let modified = utc_date(2026, 9, 3, 11, 45);

    assert_eq!(
        modified_date_at(&modified, &now, DateFormat::Relative),
        "15m ago"
    );
}

#[test]
fn recent_files_stay_relative_across_midnight() {
    let now = utc_date(2026, 9, 8, 0, 0);

    assert_eq!(
        modified_date_at(&utc_date(2026, 9, 7, 23, 59), &now, DateFormat::Relative),
        "1m ago"
    );
    assert_eq!(
        modified_date_at(&utc_date(2026, 9, 7, 23, 45), &now, DateFormat::Relative),
        "15m ago"
    );
    assert_eq!(
        modified_date_at(&utc_date(2026, 9, 7, 23, 0), &now, DateFormat::Relative),
        "Yesterday, 23:00"
    );
}

#[test]
fn saved_formats_always_render_absolute() {
    let now = utc_date(2026, 9, 8, 0, 0);
    let recent = utc_date(2026, 9, 7, 23, 59);
    let future = utc_date(2026, 9, 8, 0, 30);

    assert_eq!(
        modified_date_at(&recent, &now, DateFormat::Iso8601),
        "2026-09-07 23:59"
    );
    assert_eq!(
        modified_date_at(&future, &now, DateFormat::Iso8601),
        "2026-09-08 00:30"
    );
    assert_eq!(
        modified_date_at(&recent, &now, DateFormat::Long),
        "September 7, 2026, 23:59"
    );
    assert_eq!(
        modified_date_at(&future, &now, DateFormat::Long),
        "September 8, 2026, 00:30"
    );
}

#[test]
fn relative_days_follow_the_calendar_rather_than_24_hour_windows() {
    let now = utc_date(2026, 9, 8, 23, 0);

    assert_eq!(
        modified_date_at(&utc_date(2026, 9, 7, 0, 30), &now, DateFormat::Relative),
        "Yesterday, 00:30"
    );
    assert_eq!(
        modified_date_at(&utc_date(2026, 9, 6, 23, 30), &now, DateFormat::Relative),
        "Sunday 23:30"
    );
    assert_eq!(
        modified_date_at(&utc_date(2026, 9, 1, 23, 30), &now, DateFormat::Relative),
        "Sep 1, 23:30"
    );
}

#[test]
fn previous_local_date_is_yesterday_even_with_less_than_one_day_elapsed() {
    let timezone = glib::TimeZone::from_identifier(Some("America/New_York"))
        .expect("America/New_York timezone");
    let modified = date_in_timezone(&timezone, 2026, 9, 7, 23, 30);
    let now = date_in_timezone(&timezone, 2026, 9, 8, 0, 30);

    assert_eq!(
        modified_date_at(&modified, &now, DateFormat::Relative),
        "Yesterday, 23:30"
    );
}

#[test]
fn calendar_days_survive_daylight_saving_transitions() {
    let timezone = glib::TimeZone::from_identifier(Some("America/New_York"))
        .expect("America/New_York timezone");
    let spring_modified = date_in_timezone(&timezone, 2026, 3, 8, 23, 30);
    let spring_now = date_in_timezone(&timezone, 2026, 3, 9, 23, 0);
    let fall_modified = date_in_timezone(&timezone, 2026, 11, 1, 23, 30);
    let fall_now = date_in_timezone(&timezone, 2026, 11, 2, 12, 0);
    let two_dates_before_fall_now = date_in_timezone(&timezone, 2026, 10, 31, 23, 30);

    assert_eq!(
        calendar_day_difference(&spring_modified, &spring_now),
        Some(1)
    );
    assert_eq!(calendar_day_difference(&fall_modified, &fall_now), Some(1));
    assert_eq!(
        calendar_day_difference(&two_dates_before_fall_now, &fall_now),
        Some(2)
    );
}
