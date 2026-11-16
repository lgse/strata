// SPDX-License-Identifier: MIT

//! Unit tests for stored click-activation counts and per-view defaults.

use strata_components::collection::pointer::{ClickCount, ClickModes};
use strata_desktop::settings::Preferences;

use super::{columns, icons, list, one_click_files};

#[test]
fn stored_click_counts_honor_each_views_defaults_when_out_of_range() {
    let defaults = Preferences::default();
    assert_eq!(
        columns(&defaults),
        ClickModes::new(ClickCount::Two, ClickCount::One)
    );
    assert_eq!(
        icons(&defaults),
        ClickModes::new(ClickCount::Two, ClickCount::Two)
    );
    assert_eq!(
        list(&defaults),
        ClickModes::new(ClickCount::Two, ClickCount::Two)
    );
    assert!(!one_click_files(columns(&defaults)));

    let mut invalid = Preferences {
        columns_file_clicks: 9,
        columns_folder_clicks: 0,
        icons_file_clicks: 1,
        icons_folder_clicks: 7,
        list_file_clicks: 0,
        list_folder_clicks: 1,
        ..Preferences::default()
    };
    assert_eq!(
        columns(&invalid),
        ClickModes::new(ClickCount::Two, ClickCount::One),
        "Columns folder clicks fall back to Single, not Double"
    );
    assert_eq!(
        icons(&invalid),
        ClickModes::new(ClickCount::One, ClickCount::Two)
    );
    assert!(one_click_files(icons(&invalid)));
    assert_eq!(
        list(&invalid),
        ClickModes::new(ClickCount::Two, ClickCount::One)
    );

    invalid.columns_folder_clicks = 2;
    assert_eq!(
        columns(&invalid),
        ClickModes::new(ClickCount::Two, ClickCount::Two)
    );
}
