// SPDX-License-Identifier: MIT
use super::*;
use std::os::unix::fs::symlink;

#[test]
fn restart_and_integration_paths_follow_activation_but_old_helper_paths_remain_usable() {
    let root = tempfile::tempdir().expect("fixture");
    let bin = root.path().join("custom bin");
    let storage = bin.join(".strata-bundles");
    let old = storage.join("versions").join("a".repeat(64));
    let new = storage.join("versions").join("b".repeat(64));
    for (path, bytes) in [(&old, b"old"), (&new, b"new")] {
        fs::create_dir_all(path).expect("bundle");
        fs::write(path.join("strata"), bytes).expect("application");
        fs::write(path.join("strata-media-helper"), bytes).expect("matching helper");
    }
    symlink(
        old.strip_prefix(&storage).expect("old relative path"),
        storage.join("current"),
    )
    .expect("activation");
    symlink(".strata-bundles/current/strata", bin.join("strata")).expect("stable launcher");
    let running = old.join("strata");
    assert_eq!(bin_dir(&running), Some(bin.as_path()));
    assert_eq!(
        fs::read(launch_path(&running).expect("restart")).expect("active"),
        b"old"
    );
    symlink(
        new.strip_prefix(&storage).expect("new relative path"),
        storage.join("next"),
    )
    .expect("staging");
    fs::rename(storage.join("next"), storage.join("current")).expect("atomic activation");
    assert_eq!(
        fs::read(launch_path(&running).expect("restart after update")).expect("active"),
        b"new"
    );
    assert_eq!(
        fs::read(running.with_file_name("strata-media-helper")).expect("old instance helper"),
        b"old"
    );
    fs::remove_file(bin.join("strata")).expect("damaged launcher");
    assert!(launch_path(&running).is_err());
}

#[test]
fn legacy_deleted_executable_restarts_at_the_replacement_without_changing_plain_build_paths() {
    let root = tempfile::tempdir().expect("fixture");
    let executable = root.path().join("strata");
    fs::write(&executable, b"replacement").expect("replacement");
    assert_eq!(
        launch_path(&root.path().join("strata (deleted)")).expect("legacy restart"),
        executable
    );
    assert_eq!(
        launch_path(&executable).expect("development build"),
        executable
    );
}
