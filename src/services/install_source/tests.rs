// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

use super::{InstallSource, ManagedInstall, ensure_self_managed, marker_path_for_executable};

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
        InstallSource::load(Path::new("/nonexistent/install-source.toml")),
        InstallSource::SelfManaged
    );
}

#[test]
fn a_populated_marker_describes_the_owning_package() {
    let source = load(
        r#"
        manager = "pacman"
        package = "strata-bin"
        channel = "stable"
        update_command = "sudo pacman -Syu strata-bin"
        alternate_package = "strata-preview-bin"
        "#,
    );

    let managed = source.managed().expect("a managed install");
    assert_eq!(managed.manager(), "pacman");
    assert_eq!(managed.package(), Some("strata-bin"));
    assert_eq!(managed.channel(), Some("stable"));
    assert_eq!(managed.alternate_package(), Some("strata-preview-bin"));
    assert_eq!(
        managed.ownership_summary(),
        "Installed by pacman as strata-bin."
    );
    assert_eq!(
        managed.update_instruction(),
        "Update Strata with: sudo pacman -Syu strata-bin"
    );
    assert_eq!(
        managed.alternate_instruction().as_deref(),
        Some("Other release channels are published as strata-preview-bin.")
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
        "#,
    );
    let managed = source.managed().expect("a managed install");

    assert_eq!(managed.manager(), "your package manager");
    assert_eq!(managed.package(), None);
    assert_eq!(managed.alternate_instruction(), None);
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
        update_command = "sudo pacman -Syu strata-bin"
        "#,
    );

    assert_eq!(
        ensure_self_managed(&source),
        Err(
            "Installed by pacman as strata-bin. Update Strata with: sudo pacman -Syu strata-bin"
                .to_owned()
        )
    );
}
