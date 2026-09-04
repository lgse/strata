// SPDX-License-Identifier: GPL-3.0-or-later

use std::rc::Rc;

use crate::services::{
    BuildKind, Channel, InstallSource, ManagedInstall, ReleaseMetadata, UpdateCheck, UpdateMethod,
    Version,
};

use super::{
    CHANNEL_ORDER, COMPACT_NAVIGATION_BREAKPOINT, DIALOG_HEIGHT, DIALOG_MARGIN, DIALOG_WIDTH,
    RELEASE_CHANNEL_DESCRIPTION, RELEASE_CHANNEL_TITLE, channel_index, effective_update_channel,
    install_guard, installed_version_status, is_stale_check, managed_channel_description,
    managed_install_summary, offer_still_eligible, omarchy_update_command, responsive_dialog_size,
    shows_available_release_notes, theme_background_is_light, theme_name_matches,
    update_check_message, update_dialog_status, update_status_markup, uses_compact_navigation,
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

fn packaged() -> InstallSource {
    let managed: ManagedInstall = toml::from_str(
        r#"
        manager = "pacman"
        package = "strata-bin"
        channel = "stable"
        update_command = "yay -S strata-bin"
        alternate_package = "strata-rc-bin"
        "#,
    )
    .expect("the marker to parse");
    InstallSource::Managed(managed)
}

fn available_release() -> UpdateCheck {
    UpdateCheck::Available {
        release: ReleaseMetadata {
            version: "0.8.0".to_owned(),
            url: "https://github.com/lgse/strata/releases/tag/v0.8.0".to_owned(),
            notes: String::new(),
            note_blocks: Vec::new(),
            kind: BuildKind::Stable,
            tag: "v0.8.0".to_owned(),
            published_at: None,
            commit: None,
        },
        download_url: "https://example.invalid/strata.tar.gz".to_owned(),
    }
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
fn a_packaged_install_is_told_how_to_update_through_its_package_manager() {
    let result = available_release();
    let message = update_check_message(&result, UpdateMethod::Aur);
    let markup = update_status_markup(message, &result, &packaged());

    assert!(markup.ends_with("\nUpdate Strata with: yay -S strata-bin"));
}

#[test]
fn a_user_owned_install_gets_no_packaging_guidance() {
    let result = available_release();
    let message = update_check_message(&result, UpdateMethod::InPlace);

    assert_eq!(
        update_status_markup(message.clone(), &result, &InstallSource::SelfManaged),
        message
    );
}

#[test]
fn packaging_guidance_is_withheld_when_no_update_is_available() {
    let message = update_check_message(&UpdateCheck::UpToDate, UpdateMethod::Aur);

    assert_eq!(
        update_status_markup(message.clone(), &UpdateCheck::UpToDate, &packaged()),
        message
    );
}

#[test]
fn the_managed_row_names_the_package_channel_and_commands() {
    let source = packaged();
    let managed = source.managed().expect("a managed install");

    assert_eq!(
        managed_install_summary(managed),
        "Installed by pacman as strata-bin.\n\
         Tracking the stable release channel.\n\
         Update Strata with: yay -S strata-bin\n\
         Other release channels are published as strata-rc-bin."
    );
}

#[test]
fn the_channel_selector_explains_a_packaged_channel_and_how_to_change_it() {
    let source = packaged();
    let managed = source.managed().expect("a managed install");

    assert_eq!(
        managed_channel_description(managed),
        "This install tracks the stable release channel. \
         Other release channels are published as strata-rc-bin."
    );
}

#[test]
fn the_packaged_channel_selection_follows_the_installed_package() {
    let source = packaged();
    let managed = source.managed().expect("a managed install");

    assert_eq!(managed.tracked_channel(), Some(Channel::Stable));
}

#[test]
fn the_update_dialog_defers_to_the_package_manager() {
    let source = packaged();
    let managed = source.managed().expect("a managed install");

    assert_eq!(
        update_dialog_status(managed),
        "Installed by pacman as strata-bin. Update Strata with: yay -S strata-bin"
    );
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
        installed_version_status(&version, BuildKind::Stable, UpdateMethod::InPlace),
        "Version 0.6.0"
    );
}

#[test]
fn installed_version_status_names_the_build_kind_for_a_prerelease() {
    let version = Version::parse("0.6.0-rc.1").expect("valid version");
    assert_eq!(
        installed_version_status(&version, BuildKind::Rc, UpdateMethod::InPlace),
        "Version 0.6.0-rc.1 · Release candidate"
    );
}

#[test]
fn package_managed_updates_always_follow_stable() {
    for selected in [Channel::Stable, Channel::Preview, Channel::Nightly] {
        assert_eq!(
            effective_update_channel(selected, UpdateMethod::Omarchy),
            Channel::Stable
        );
        assert_eq!(
            effective_update_channel(selected, UpdateMethod::Pacman),
            Channel::Stable
        );
    }
}

#[test]
fn manual_and_marked_package_updates_keep_the_selected_channel() {
    for selected in [Channel::Stable, Channel::Preview, Channel::Nightly] {
        assert_eq!(
            effective_update_channel(selected, UpdateMethod::InPlace),
            selected
        );
        assert_eq!(
            effective_update_channel(selected, UpdateMethod::Aur),
            selected
        );
    }
}

#[test]
fn package_managed_status_identifies_omarchy() {
    let version = Version::parse("0.8.0").expect("valid version");
    assert_eq!(
        installed_version_status(&version, BuildKind::Stable, UpdateMethod::Omarchy),
        "Version 0.8.0 · Managed by Omarchy"
    );
}

#[test]
fn omarchy_updates_open_in_the_configured_terminal() {
    let command = omarchy_update_command();

    assert_eq!(command.get_program(), "xdg-terminal-exec");
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        ["--", "omarchy", "update"]
    );
}

#[test]
fn package_managed_update_directs_users_to_omarchy_update() {
    let message = update_check_message(
        &UpdateCheck::Available {
            release: ReleaseMetadata {
                version: "0.9.0".to_owned(),
                url: "https://example.test/release".to_owned(),
                notes: String::new(),
                note_blocks: Vec::new(),
                kind: BuildKind::Stable,
                tag: "v0.9.0".to_owned(),
                published_at: None,
                commit: None,
            },
            download_url: "https://example.test/download".to_owned(),
        },
        UpdateMethod::Omarchy,
    );

    assert!(message.contains("Run “omarchy update” to install"));
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

#[test]
fn the_selector_highlights_the_button_for_the_persisted_channel() {
    for (index, channel) in CHANNEL_ORDER.into_iter().enumerate() {
        assert_eq!(channel_index(channel), index);
    }
}
