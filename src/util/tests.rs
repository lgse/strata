// SPDX-License-Identifier: MIT

use super::{calendar_day_difference, modified_date_at};

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

    assert_eq!(modified_date_at(&modified, &now), "2026-09-03 13:00");
}

#[test]
fn recent_past_modified_dates_remain_relative() {
    let now = utc_date(2026, 9, 3, 12, 0);
    let modified = utc_date(2026, 9, 3, 11, 45);

    assert_eq!(modified_date_at(&modified, &now), "15m ago");
}

#[test]
fn relative_days_follow_the_calendar_rather_than_24_hour_windows() {
    let now = utc_date(2026, 9, 8, 23, 0);

    assert_eq!(
        modified_date_at(&utc_date(2026, 9, 7, 0, 30), &now),
        "Yesterday, 00:30"
    );
    assert_eq!(
        modified_date_at(&utc_date(2026, 9, 6, 23, 30), &now),
        "Sunday 23:30"
    );
    assert_eq!(
        modified_date_at(&utc_date(2026, 9, 1, 23, 30), &now),
        "Sep 1, 23:30"
    );
}

#[test]
fn previous_local_date_is_yesterday_even_with_less_than_one_day_elapsed() {
    let timezone = glib::TimeZone::from_identifier(Some("America/New_York"))
        .expect("America/New_York timezone");
    let modified = date_in_timezone(&timezone, 2026, 9, 7, 23, 30);
    let now = date_in_timezone(&timezone, 2026, 9, 8, 0, 30);

    assert_eq!(modified_date_at(&modified, &now), "Yesterday, 23:30");
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
