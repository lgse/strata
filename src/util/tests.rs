// SPDX-License-Identifier: MIT

use super::modified_date_at;

fn utc_date(year: i32, month: i32, day: i32, hour: i32, minute: i32) -> glib::DateTime {
    glib::DateTime::from_utc(year, month, day, hour, minute, 0.0).expect("valid test date")
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
