// SPDX-License-Identifier: GPL-3.0-or-later

use crate::services::{ReleaseMetadata, UpdateCheck};

use super::{
    COMPACT_NAVIGATION_BREAKPOINT, DIALOG_HEIGHT, DIALOG_MARGIN, DIALOG_WIDTH,
    responsive_dialog_size, shows_available_release_notes, uses_compact_navigation,
};

#[test]
fn settings_dialog_keeps_its_preferred_size_when_space_allows() {
    assert_eq!(
        responsive_dialog_size(
            DIALOG_WIDTH + DIALOG_MARGIN * 2,
            DIALOG_HEIGHT + DIALOG_MARGIN * 2,
        ),
        (DIALOG_WIDTH, DIALOG_HEIGHT)
    );
}

#[test]
fn settings_dialog_shrinks_to_leave_a_margin_in_small_windows() {
    assert_eq!(responsive_dialog_size(640, 480), (592, 432));
}

#[test]
fn settings_dialog_size_stays_valid_at_tiny_allocations() {
    assert_eq!(responsive_dialog_size(20, 20), (1, 1));
}

#[test]
fn settings_navigation_compacts_below_the_breakpoint() {
    assert!(uses_compact_navigation(COMPACT_NAVIGATION_BREAKPOINT - 1));
    assert!(!uses_compact_navigation(COMPACT_NAVIGATION_BREAKPOINT));
}

#[test]
fn available_notes_are_shown_only_for_a_newer_release() {
    assert!(!shows_available_release_notes(&UpdateCheck::UpToDate));
    assert!(!shows_available_release_notes(&UpdateCheck::Failed(
        "offline".to_owned()
    )));
    assert!(shows_available_release_notes(&UpdateCheck::Available {
        release: ReleaseMetadata {
            version: "1.0.0".to_owned(),
            url: "https://example.test/release".to_owned(),
            notes: "Changes".to_owned(),
            note_blocks: vec![crate::services::ReleaseNoteBlock::Paragraph(
                "Changes".to_owned(),
            )],
        },
        download_url: None,
    }));
}
