// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn same_filesystem_nested_mount_is_rejected_during_planning() {
    let fixture = tempfile::tempdir().expect("fixture");
    let trash = volume_trash(fixture.path(), 1000);
    let source = trash.join("files/report");
    fs::write(&source, b"original").expect("source");
    let nested = fixture.path().join("nested");
    fs::create_dir(&nested).expect("nested mount");
    let destination = nested.join("report");
    assert_eq!(
        restore_volume_relation(&Location::local(&source), &Location::local(&destination)),
        VolumeRelation::Same
    );
    for fs_type in ["ext4", "btrfs"] {
        let context = RestoreContext {
            home_trash_root: fixture.path().join("home-trash"),
            uid: 1000,
            mounts: MountTable::parse(format!(
                "1 0 8:1 / / rw - {fs_type} /dev/root rw\n2 1 8:1 / {} rw - {fs_type} /dev/root rw\n3 2 8:1 /original {} rw - {fs_type} /dev/root rw\n",
                fixture.path().display(),
                nested.display()
            )),
        };
        let error = plan_restore_from_known_paths(&source, &destination, &trash, None, &context)
            .expect_err("nested mount");
        assert!(error.message().contains("bind mount or subvolume boundary"));
        assert_eq!(fs::read(&source).expect("source"), b"original");
        assert!(!destination.exists());
    }
}

#[test]
fn escaped_path_error_does_not_blame_bind_mounts() {
    let fixture = tempfile::tempdir().expect("fixture");
    let trash = volume_trash(fixture.path(), 1000);
    let source = trash.join("files/report");
    fs::write(&source, b"original").expect("source");
    let context = context_for(&fixture.path().join("home-trash"), 1000, fixture.path());
    let error = plan_restore_from_known_paths(
        &source,
        Path::new("/outside/report"),
        &trash,
        None,
        &context,
    )
    .expect_err("outside root");
    assert!(error.message().contains("outside the trash volume"));
    assert!(!error.message().contains("Bind"));
}
