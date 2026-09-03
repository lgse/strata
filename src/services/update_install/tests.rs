// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;

use super::{find_binaries, first_hash_token, stage_binary_path, stage_workdir};

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
