// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    APPLICATION_ICON, DESKTOP_ENTRY, desktop_entry_with_exec, find_binary, first_hash_token,
    refresh_desktop_metadata,
};

const PACKAGED_ENTRY: &str =
    "[Desktop Entry]\nType=Application\nName=Strata\nExec=strata %U\nIcon=io.github.lgse.Strata\n";

fn scratch_dir(label: &str, line: u32) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "strata-update-install-test-{label}-{}-{line}",
        std::process::id()
    ));
    let _removed = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn packaged_metadata(package_dir: &Path) {
    fs::create_dir_all(package_dir).expect("create package dir");
    fs::write(package_dir.join(DESKTOP_ENTRY), PACKAGED_ENTRY).expect("write packaged entry");
    fs::write(package_dir.join(APPLICATION_ICON), b"<svg/>").expect("write packaged icon");
}

fn installed_icon(data_home: &Path) -> PathBuf {
    data_home
        .join("icons/hicolor/scalable/apps")
        .join(APPLICATION_ICON)
}

#[test]
fn first_hash_token_lowercases_and_ignores_trailing_filename() {
    assert_eq!(
        first_hash_token("ABCDEF  strata-0.2.0-x86_64-unknown-linux-gnu.tar.gz\n"),
        Some("abcdef".to_owned())
    );
}

#[test]
fn first_hash_token_rejects_empty_input() {
    assert_eq!(first_hash_token("   \n"), None);
}

#[test]
fn find_binary_locates_the_nested_executable() {
    let dir = std::env::temp_dir().join(format!(
        "strata-update-install-test-{}-{}",
        std::process::id(),
        line!()
    ));
    let package_dir = dir.join("strata-0.2.0-x86_64-unknown-linux-gnu");
    fs::create_dir_all(&package_dir).expect("create package dir");
    fs::write(package_dir.join("strata"), b"binary").expect("write binary");

    let found = find_binary(&dir).expect("binary should be found");
    assert_eq!(found, package_dir.join("strata"));

    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn find_binary_errors_when_missing() {
    let dir = std::env::temp_dir().join(format!(
        "strata-update-install-test-empty-{}-{}",
        std::process::id(),
        line!()
    ));
    fs::create_dir_all(&dir).expect("create empty dir");

    assert!(find_binary(&dir).is_err());

    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn desktop_entry_exec_points_at_the_install_path_and_keeps_field_codes() {
    let entry = desktop_entry_with_exec(PACKAGED_ENTRY, Path::new("/home/user/.local/bin/strata"));

    assert!(entry.contains("Exec=/home/user/.local/bin/strata %U\n"));
    assert!(entry.starts_with("[Desktop Entry]\n"));
    assert!(entry.contains("Icon=io.github.lgse.Strata\n"));
}

#[test]
fn desktop_entry_exec_quotes_paths_containing_spaces() {
    let entry = desktop_entry_with_exec(PACKAGED_ENTRY, Path::new("/opt/my apps/strata"));

    assert!(entry.contains("Exec=\"/opt/my apps/strata\" %U\n"));
}

#[test]
fn desktop_entry_without_field_codes_keeps_a_bare_exec() {
    let entry = desktop_entry_with_exec(
        "[Desktop Entry]\nExec=strata\n",
        Path::new("/usr/bin/strata"),
    );

    assert_eq!(entry, "[Desktop Entry]\nExec=/usr/bin/strata\n");
}

#[test]
fn refresh_rewrites_an_installed_entry_and_icon() {
    let dir = scratch_dir("refresh", line!());
    let package_dir = dir.join("strata-0.7.0-x86_64-unknown-linux-gnu");
    packaged_metadata(&package_dir);
    let data_home = dir.join("share");
    let applications = data_home.join("applications");
    fs::create_dir_all(&applications).expect("create applications dir");
    fs::write(
        applications.join(DESKTOP_ENTRY),
        "[Desktop Entry]\nExec=strata %U\nIcon=system-file-manager\n",
    )
    .expect("write stale entry");
    let executable = dir.join("bin/strata");

    refresh_desktop_metadata(&package_dir, &executable, &data_home);

    let entry = fs::read_to_string(applications.join(DESKTOP_ENTRY)).expect("read entry");
    assert!(entry.contains(&format!("Exec={} %U\n", executable.display())));
    assert!(entry.contains("Icon=io.github.lgse.Strata\n"));
    assert_eq!(
        fs::read(installed_icon(&data_home)).expect("read icon"),
        b"<svg/>"
    );

    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn refresh_does_not_create_metadata_the_user_never_installed() {
    let dir = scratch_dir("no-entry", line!());
    let package_dir = dir.join("strata-0.7.0-x86_64-unknown-linux-gnu");
    packaged_metadata(&package_dir);
    let data_home = dir.join("share");

    refresh_desktop_metadata(&package_dir, &dir.join("bin/strata"), &data_home);

    assert!(!data_home.join("applications").join(DESKTOP_ENTRY).exists());
    assert!(!installed_icon(&data_home).exists());

    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn refresh_keeps_an_installed_entry_when_the_archive_omits_metadata() {
    let dir = scratch_dir("legacy-archive", line!());
    let package_dir = dir.join("strata-0.7.0-x86_64-unknown-linux-gnu");
    fs::create_dir_all(&package_dir).expect("create package dir");
    let data_home = dir.join("share");
    let applications = data_home.join("applications");
    fs::create_dir_all(&applications).expect("create applications dir");
    let existing = "[Desktop Entry]\nExec=strata %U\n";
    fs::write(applications.join(DESKTOP_ENTRY), existing).expect("write entry");

    refresh_desktop_metadata(&package_dir, &dir.join("bin/strata"), &data_home);

    assert_eq!(
        fs::read_to_string(applications.join(DESKTOP_ENTRY)).expect("read entry"),
        existing
    );

    fs::remove_dir_all(&dir).expect("cleanup");
}
