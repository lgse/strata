// SPDX-License-Identifier: GPL-3.0-or-later

use std::rc::Rc;

use crate::services::{BuildKind, Channel, ReleaseMetadata, UpdateCheck, Version};

use super::{
    COMPACT_NAVIGATION_BREAKPOINT, DIALOG_HEIGHT, DIALOG_MARGIN, DIALOG_WIDTH,
    RELEASE_CHANNEL_DESCRIPTION, RELEASE_CHANNEL_TITLE, install_guard, installed_version_status,
    is_stale_check, offer_still_eligible, responsive_dialog_size, shows_available_release_notes,
    theme_background_is_light, theme_name_matches, uses_compact_navigation,
    video_preview_backend_label, video_preview_control_state,
};
use crate::sandbox::MediaPreviewBackend;

#[test]
fn a_checks_result_is_current_only_for_the_generation_it_was_issued_under() {
    assert!(!is_stale_check(1, 1));
    assert!(!is_stale_check(0, 0));
}

#[test]
fn a_checks_result_is_stale_once_a_newer_check_has_started() {
    // The scenario Important 1 fixes: a check issued as generation 1 is
    // still in flight when a channel toggle starts generation 2. Generation
    // 1's eventual result must never be applied.
    assert!(is_stale_check(1, 2));
    assert!(is_stale_check(2, 1));
}

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
fn theme_search_is_case_insensitive_and_ignores_outer_whitespace() {
    assert!(theme_name_matches("Tokyo Night Storm", " night "));
    assert!(theme_name_matches("Dracula", "DRAC"));
    assert!(theme_name_matches("Nord", ""));
    assert!(!theme_name_matches("Solarized Light", "dark"));
}

#[test]
fn theme_appearance_uses_background_luminance() {
    assert!(theme_background_is_light("#ffffff"));
    assert!(theme_background_is_light("#efecf4"));
    assert!(!theme_background_is_light("#1e1d1f"));
    assert!(!theme_background_is_light("invalid"));
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
            kind: BuildKind::Stable,
            tag: "v1.0.0".to_owned(),
            published_at: None,
            commit: None,
        },
        download_url: "https://example.test/download".to_owned(),
    }));
}

#[test]
fn video_preview_backend_selector_labels_all_options() {
    for (backend, label) in [
        (MediaPreviewBackend::Automatic, "Automatic"),
        (MediaPreviewBackend::VaApi, "VA-API"),
        (MediaPreviewBackend::Vulkan, "Vulkan"),
    ] {
        assert_eq!(video_preview_backend_label(backend), label);
    }
    assert_eq!(
        video_preview_backend_label(MediaPreviewBackend::Software),
        "Automatic"
    );
}

#[test]
fn release_channel_copy_distinguishes_preview_from_nightly() {
    assert_eq!(RELEASE_CHANNEL_TITLE, "Release channel");
    assert_eq!(
        RELEASE_CHANNEL_DESCRIPTION,
        "Preview receives alpha, beta, and release-candidate builds. Nightly also receives daily development builds."
    );
}

#[test]
fn video_preview_controls_follow_enabled_state() {
    assert_eq!(video_preview_control_state(true), (true, true, true));
    assert_eq!(video_preview_control_state(false), (false, true, false));
}

#[test]
fn installed_version_status_stays_plain_for_a_stable_build() {
    let version = Version::parse("0.6.0").expect("valid version");
    assert_eq!(
        installed_version_status(&version, BuildKind::Stable),
        "Version 0.6.0"
    );
}

#[test]
fn installed_version_status_names_the_build_kind_for_a_prerelease() {
    let version = Version::parse("0.6.0-rc.1").expect("valid version");
    assert_eq!(
        installed_version_status(&version, BuildKind::Rc),
        "Version 0.6.0-rc.1 · Release candidate"
    );
}

#[test]
fn a_cached_prerelease_offer_stops_being_installable_once_the_channel_is_stable() {
    // The cross-window case: a window cached an RC offer while on Preview,
    // another window switched back to Stable, and the cached offer's install
    // button must refuse it.
    assert!(!offer_still_eligible(Channel::Stable, BuildKind::Rc));
    assert!(!offer_still_eligible(Channel::Stable, BuildKind::Nightly));
    assert!(!offer_still_eligible(Channel::Preview, BuildKind::Nightly));
}

#[test]
fn a_cached_offer_stays_installable_when_the_channel_still_allows_it() {
    assert!(offer_still_eligible(Channel::Stable, BuildKind::Stable));
    assert!(offer_still_eligible(Channel::Preview, BuildKind::Stable));
    assert!(offer_still_eligible(Channel::Preview, BuildKind::Alpha));
    assert!(offer_still_eligible(Channel::Preview, BuildKind::Beta));
    assert!(offer_still_eligible(Channel::Preview, BuildKind::Rc));
    assert!(offer_still_eligible(Channel::Nightly, BuildKind::Nightly));
    assert!(offer_still_eligible(Channel::Nightly, BuildKind::Rc));
}

#[test]
fn every_window_installs_behind_one_process_wide_guard() {
    // Two windows each ask for a guard the way `ui::window::present` does.
    // Handing out two independent cells is what let an update in one window
    // and another update in a second window replace the executable concurrently.
    let first = install_guard();
    let second = install_guard();
    assert!(Rc::ptr_eq(&first, &second));

    assert!(!first.replace(true));
    assert!(
        second.get(),
        "an install started in one window must be visible in every other"
    );
    first.set(false);
}
