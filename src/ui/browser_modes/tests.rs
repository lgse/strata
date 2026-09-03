// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    BrowserMode, ClickActivation, ClickCount, EXPLORER_COLUMN_MIN_WIDTHS, EXPLORER_COLUMN_WIDTHS,
    explorer_column_width, should_activate_pointer_click,
};

#[test]
fn explorer_columns_have_usable_minimum_widths() {
    for (index, minimum) in EXPLORER_COLUMN_MIN_WIDTHS.into_iter().enumerate() {
        assert_eq!(explorer_column_width(index, minimum - 1), minimum);
        assert_eq!(explorer_column_width(index, minimum + 1), minimum + 1);
    }
}

#[test]
fn explorer_default_widths_respect_column_minimums() {
    for (default, minimum) in EXPLORER_COLUMN_WIDTHS
        .into_iter()
        .zip(EXPLORER_COLUMN_MIN_WIDTHS)
    {
        assert!(default >= minimum);
    }
}

#[test]
fn stored_click_counts_reject_unsupported_values() {
    assert_eq!(ClickCount::from_stored(1), Some(ClickCount::One));
    assert_eq!(ClickCount::from_stored(2), Some(ClickCount::Two));
    assert_eq!(ClickCount::from_stored(0), None);
    assert_eq!(ClickCount::from_stored(3), None);
}

#[test]
fn click_activation_defaults_follow_view_conventions() {
    assert_eq!(
        ClickActivation::default_for(BrowserMode::Columns),
        ClickActivation {
            files: ClickCount::Two,
            folders: ClickCount::One,
        }
    );
    for mode in [BrowserMode::Grid, BrowserMode::Explorer] {
        assert_eq!(
            ClickActivation::default_for(mode),
            ClickActivation {
                files: ClickCount::Two,
                folders: ClickCount::Two,
            }
        );
    }
}

#[test]
fn single_click_activation_distinguishes_files_and_folders() {
    let activation = ClickActivation {
        files: ClickCount::Two,
        folders: ClickCount::One,
    };

    assert!(should_activate_pointer_click(1, true, activation));
    assert!(!should_activate_pointer_click(1, false, activation));
    assert!(!should_activate_pointer_click(2, true, activation));
}
