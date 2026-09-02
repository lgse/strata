// SPDX-License-Identifier: GPL-3.0-or-later

use crate::services::{InstallSource, ManagedInstall, ReleaseMetadata, UpdateCheck};

use super::{
    COMPACT_NAVIGATION_BREAKPOINT, DIALOG_HEIGHT, DIALOG_MARGIN, DIALOG_WIDTH,
    managed_install_summary, responsive_dialog_size, shows_available_release_notes,
    theme_background_is_light, theme_name_matches, update_dialog_status, update_status_markup,
    uses_compact_navigation,
};

fn packaged() -> InstallSource {
    let managed: ManagedInstall = toml::from_str(
        r#"
        manager = "pacman"
        package = "strata-bin"
        channel = "stable"
        update_command = "sudo pacman -Syu strata-bin"
        alternate_package = "strata-preview-bin"
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
        },
        download_url: Some("https://example.invalid/strata.tar.gz".to_owned()),
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
        },
        download_url: None,
    }));
}

#[test]
fn a_packaged_install_is_told_how_to_update_through_its_package_manager() {
    let markup = update_status_markup(&available_release(), &packaged());

    assert!(
        markup.ends_with("\nUpdate Strata with: sudo pacman -Syu strata-bin"),
        "expected packaging guidance, got: {markup}"
    );
}

#[test]
fn a_user_owned_install_gets_no_packaging_guidance() {
    assert_eq!(
        update_status_markup(&available_release(), &InstallSource::SelfManaged),
        super::update_check_message(&available_release())
    );
}

#[test]
fn packaging_guidance_is_withheld_when_no_update_is_available() {
    assert_eq!(
        update_status_markup(&UpdateCheck::UpToDate, &packaged()),
        super::update_check_message(&UpdateCheck::UpToDate)
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
         Update Strata with: sudo pacman -Syu strata-bin\n\
         Other release channels are published as strata-preview-bin."
    );
}

#[test]
fn the_update_dialog_defers_to_the_package_manager() {
    let source = packaged();
    let managed = source.managed().expect("a managed install");

    assert_eq!(
        update_dialog_status(managed),
        "Installed by pacman as strata-bin. Update Strata with: sudo pacman -Syu strata-bin"
    );
}
