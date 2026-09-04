// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

use super::{InstallSource, ManagedInstall, ensure_self_managed, marker_path_for_executable};
use crate::services::Channel;

const PACKAGED_MARKER: &str = r#"
manager = "pacman"
package = "strata-bin"
channel = "stable"
aur_helpers = ["yay", "paru", "pikaur", "trizen"]
alternate_package = "strata-rc-bin"
"#;

fn marker(contents: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("install-source.toml");
    std::fs::write(&path, contents).expect("the marker to be written");
    (directory, path)
}

fn load(contents: &str) -> InstallSource {
    let (_directory, path) = marker(contents);
    InstallSource::load(&path)
}

#[test]
fn a_missing_marker_means_a_user_owned_install() {
    assert_eq!(
        InstallSource::from_marker_path(None),
        InstallSource::SelfManaged
    );
    assert!(
        !Path::new("/nonexistent/bin/strata").is_file(),
        "the prefix used below must not exist"
    );
    assert_eq!(
        InstallSource::from_marker_path(
            marker_path_for_executable(Path::new("/nonexistent/bin/strata"))
                .filter(|path| path.is_file())
        ),
        InstallSource::SelfManaged
    );
}

#[test]
fn a_populated_marker_describes_the_owning_package() {
    let source = load(PACKAGED_MARKER);

    let managed = source.managed().expect("a managed install");
    assert_eq!(managed.manager(), "pacman");
    assert_eq!(managed.package(), Some("strata-bin"));
    assert_eq!(managed.channel(), Some("stable"));
    assert_eq!(managed.alternate_package(), Some("strata-rc-bin"));
    assert_eq!(
        managed.ownership_summary(),
        "Installed by pacman as strata-bin."
    );
    assert_eq!(
        managed.alternate_instruction().as_deref(),
        Some("Other release channels are published as strata-rc-bin.")
    );
}

#[test]
fn the_update_command_names_an_installed_aur_helper() {
    let managed = load(PACKAGED_MARKER)
        .managed()
        .cloned()
        .expect("a managed install");

    assert_eq!(
        managed.update_instruction_with(|helper| helper == "paru"),
        "Update Strata with: paru -S strata-bin",
        "the first listed helper that is present wins, not the first listed"
    );
}

#[test]
fn the_update_command_falls_back_when_no_helper_is_installed() {
    let managed = load(PACKAGED_MARKER)
        .managed()
        .cloned()
        .expect("a managed install");

    assert_eq!(
        managed.update_instruction_with(|_| false),
        "Update Strata with an AUR helper, for example: yay -S strata-bin"
    );
}

#[test]
fn an_explicit_update_command_wins_over_helper_detection() {
    let managed = load(
        r#"
        manager = "apt"
        package = "strata"
        update_command = "sudo apt install --only-upgrade strata"
        aur_helpers = ["yay"]
        "#,
    )
    .managed()
    .cloned()
    .expect("a managed install");

    assert_eq!(
        managed.update_instruction_with(|_| true),
        "Update Strata with: sudo apt install --only-upgrade strata"
    );
}

#[test]
fn the_packaging_channel_maps_onto_the_app_channel() {
    let tracked = |channel: &str| {
        load(&format!("channel = \"{channel}\""))
            .managed()
            .expect("a managed install")
            .tracked_channel()
    };

    assert_eq!(tracked("rc"), Some(Channel::Preview));
    assert_eq!(tracked("stable"), Some(Channel::Stable));
    assert_eq!(tracked("preview"), Some(Channel::Preview));
    assert_eq!(tracked("nightly"), Some(Channel::Nightly));
    assert_eq!(tracked("something-else"), None);
    assert_eq!(
        load("")
            .managed()
            .expect("a managed install")
            .tracked_channel(),
        None
    );
}

#[test]
fn an_unreadable_marker_still_counts_as_packaged() {
    let (_directory, path) = marker("");
    std::fs::write(&path, [0xff, 0xfe, 0x00]).expect("the marker to be written");

    assert_eq!(
        InstallSource::load(&path),
        InstallSource::Managed(ManagedInstall::default())
    );
}

#[test]
fn an_unparseable_marker_still_counts_as_packaged() {
    let source = load("this is not toml");

    assert_eq!(
        source,
        InstallSource::Managed(ManagedInstall::default()),
        "a corrupt marker must not re-enable installing over package-owned files"
    );
}

#[test]
fn unknown_keys_do_not_break_an_older_binary() {
    let source = load(
        r#"
        manager = "pacman"
        packaged_by_a_newer_strata = "some value"
        "#,
    );

    assert_eq!(
        source.managed().map(ManagedInstall::manager),
        Some("pacman")
    );
}

#[test]
fn an_empty_marker_falls_back_to_generic_guidance() {
    let source = load("");
    let managed = source.managed().expect("a managed install");

    assert_eq!(managed.manager(), "your package manager");
    assert_eq!(
        managed.ownership_summary(),
        "Installed by your package manager."
    );
    assert_eq!(
        managed.update_instruction(),
        "Update Strata through your package manager."
    );
    assert_eq!(managed.alternate_instruction(), None);
}

#[test]
fn blank_values_are_treated_as_absent() {
    let source = load(
        r#"
        manager = "  "
        package = ""
        alternate_package = ""
        aur_helpers = ["", "  "]
        "#,
    );
    let managed = source.managed().expect("a managed install");

    assert_eq!(managed.manager(), "your package manager");
    assert_eq!(managed.package(), None);
    assert_eq!(managed.alternate_instruction(), None);
    assert_eq!(
        managed.update_instruction_with(|_| true),
        "Update Strata through your package manager.",
        "a blank helper must not render as ` -S `"
    );
}

#[test]
fn the_marker_is_resolved_relative_to_the_install_prefix() {
    assert_eq!(
        marker_path_for_executable(Path::new("/usr/bin/strata")),
        Some(PathBuf::from("/usr/share/strata/install-source.toml"))
    );
    assert_eq!(
        marker_path_for_executable(Path::new("/opt/strata/bin/strata")),
        Some(PathBuf::from(
            "/opt/strata/share/strata/install-source.toml"
        ))
    );
    assert_eq!(marker_path_for_executable(Path::new("strata")), None);
}

#[test]
fn a_user_owned_install_may_replace_its_own_binary() {
    assert_eq!(ensure_self_managed(&InstallSource::SelfManaged), Ok(()));
}

#[test]
fn a_packaged_install_refuses_to_replace_its_own_binary() {
    let source = load(
        r#"
        manager = "pacman"
        package = "strata-bin"
        update_command = "yay -S strata-bin"
        "#,
    );

    assert_eq!(
        ensure_self_managed(&source),
        Err("Installed by pacman as strata-bin. Update Strata with: yay -S strata-bin".to_owned())
    );
}
