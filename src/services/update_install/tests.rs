// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    APPLICATION_ICON, DESKTOP_ENTRY, UpdateMethod, desktop_entry_with_exec, find_binaries,
    first_hash_token, package_repository_version_for, parse_package_version,
    refresh_desktop_metadata, repository_database_version, stage_binary_path, stage_workdir,
    update_method_for,
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

fn ownership_probe(dir: &Path, exit_code: u8) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let probe = dir.join("pacman");
    fs::write(&probe, format!("#!/bin/sh\nexit {exit_code}\n")).expect("write ownership probe");
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o755))
        .expect("make ownership probe executable");
    probe
}

#[test]
fn arch_package_versions_drop_epoch_and_package_release() {
    assert_eq!(
        parse_package_version("0.8.1-1").map(|version| version.to_string()),
        Some("0.8.1".to_owned())
    );
    assert_eq!(
        parse_package_version("2:0.9.0-rc.1-3.1").map(|version| version.to_string()),
        Some("0.9.0-rc.1".to_owned())
    );
    assert!(parse_package_version("0.8.1").is_none());
    assert!(parse_package_version("not-a-version-1").is_none());
}

#[test]
fn repository_probe_reads_pacmans_machine_readable_version() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("create tempdir");
    let probe = dir.path().join("pacman");
    fs::write(&probe, "#!/bin/sh\nprintf '%s\\n' '0.8.1-1'\n").expect("write probe");
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o755)).expect("make probe executable");

    assert_eq!(
        package_repository_version_for(&probe, "strata").map(|version| version.to_string()),
        Ok("0.8.1".to_owned())
    );
}

#[test]
fn omarchy_database_reports_the_packaged_strata_version() {
    let description = b"%NAME%\nstrata\n\n%VERSION%\n0.8.1-2\n";
    let mut archive = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut archive);
        let mut header = tar::Header::new_gnu();
        header.set_size(description.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "strata-0.8.1-2/desc", &description[..])
            .expect("append package description");
        builder.finish().expect("finish repository archive");
    }
    let database = zstd::stream::encode_all(&archive[..], 0).expect("compress repository database");

    assert_eq!(
        repository_database_version(&database, "strata").map(|version| version.to_string()),
        Ok("0.8.1".to_owned())
    );
}

#[test]
fn omarchy_package_ownership_defers_updates_to_omarchy() {
    let dir = tempfile::tempdir().expect("create scratch dir");
    let probe = ownership_probe(dir.path(), 0);
    let os_release = dir.path().join("os-release");
    fs::write(&os_release, "NAME=\"Omarchy\"\nID=omarchy\nID_LIKE=arch\n")
        .expect("write os-release");

    assert_eq!(
        update_method_for(Path::new("/usr/bin/strata"), &probe, &os_release),
        UpdateMethod::Omarchy
    );
}

#[test]
fn ownership_probe_errors_disable_in_place_updates() {
    let dir = tempfile::tempdir().expect("create scratch dir");
    let os_release = dir.path().join("os-release");
    fs::write(&os_release, "ID=omarchy\n").expect("write os-release");

    assert_eq!(
        update_method_for(Path::new("/usr/bin/strata"), dir.path(), &os_release),
        UpdateMethod::Omarchy
    );
}

#[test]
fn non_omarchy_pacman_ownership_defers_updates_to_pacman() {
    let dir = tempfile::tempdir().expect("create scratch dir");
    let probe = ownership_probe(dir.path(), 0);
    let os_release = dir.path().join("os-release");
    fs::write(&os_release, "ID=arch\n").expect("write os-release");

    assert_eq!(
        update_method_for(Path::new("/usr/bin/strata"), &probe, &os_release),
        UpdateMethod::Pacman
    );
}

#[test]
fn unowned_release_binary_keeps_in_place_updates() {
    let dir = tempfile::tempdir().expect("create scratch dir");
    let probe = ownership_probe(dir.path(), 1);

    assert_eq!(
        update_method_for(
            Path::new("/home/user/.local/bin/strata"),
            &probe,
            &dir.path().join("missing-os-release"),
        ),
        UpdateMethod::InPlace
    );
}

#[test]
fn stage_workdir_is_unique_per_call() {
    let exe_dir = tempfile::tempdir().expect("create scratch exe dir");
    let first = stage_workdir(exe_dir.path()).expect("stage first workdir");
    let second = stage_workdir(exe_dir.path()).expect("stage second workdir");

    assert_ne!(first.path(), second.path());
    assert!(first.path().is_dir());
    assert!(second.path().is_dir());
    // Both live inside `exe_dir`, matching the old process-scoped scheme's
    // placement, just no longer sharing a single path within it.
    assert_eq!(first.path().parent(), Some(exe_dir.path()));
    assert_eq!(second.path().parent(), Some(exe_dir.path()));
}

#[test]
fn stage_binary_path_is_unique_per_call() {
    let exe_dir = tempfile::tempdir().expect("create scratch exe dir");
    let first = stage_binary_path(exe_dir.path()).expect("stage first binary path");
    let second = stage_binary_path(exe_dir.path()).expect("stage second binary path");

    assert_ne!(first.path(), second.path());
    assert_eq!(first.path().parent(), Some(exe_dir.path()));
    assert_eq!(second.path().parent(), Some(exe_dir.path()));
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
fn find_binaries_locates_a_single_nested_binary() {
    let dir = std::env::temp_dir().join(format!(
        "strata-update-install-test-{}-{}",
        std::process::id(),
        line!()
    ));
    let package_dir = dir.join("strata-0.2.0-x86_64-unknown-linux-gnu");
    fs::create_dir_all(&package_dir).expect("create package dir");
    fs::write(package_dir.join("strata"), b"binary").expect("write binary");

    let found = find_binaries(&dir, &["strata"]).expect("binary should be found");
    assert_eq!(found, vec![package_dir.join("strata")]);

    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn find_binaries_errors_when_a_requested_name_is_missing() {
    let dir = std::env::temp_dir().join(format!(
        "strata-update-install-test-empty-{}-{}",
        std::process::id(),
        line!()
    ));
    fs::create_dir_all(&dir).expect("create empty dir");

    assert!(find_binaries(&dir, &["strata"]).is_err());

    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn find_binaries_returns_all_requested_names_when_several_are_present() {
    let dir = std::env::temp_dir().join(format!(
        "strata-update-install-test-multi-{}-{}",
        std::process::id(),
        line!()
    ));
    let package_dir = dir.join("strata-0.2.0-x86_64-unknown-linux-gnu");
    fs::create_dir_all(&package_dir).expect("create package dir");
    fs::write(package_dir.join("strata"), b"binary").expect("write strata binary");
    fs::write(package_dir.join("strata-helper"), b"binary").expect("write helper binary");

    let found =
        find_binaries(&dir, &["strata", "strata-helper"]).expect("both binaries should be found");
    assert_eq!(
        found,
        vec![
            package_dir.join("strata"),
            package_dir.join("strata-helper"),
        ]
    );

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
