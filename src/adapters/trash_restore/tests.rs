// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

fn context_for(home_trash: &Path, uid: u32, mount_point: &Path) -> RestoreContext {
    RestoreContext {
        home_trash_root: home_trash.to_path_buf(),
        uid,
        mounts: MountTable::parse(&format!(
            "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n22 1 8:2 / {} rw - ext4 /dev/sdb1 rw\n",
            mount_point.display()
        )),
    }
}

fn shared_volume_trash(root: &Path, uid: u32) -> PathBuf {
    let trash = root.join(".Trash").join(uid.to_string());
    fs::create_dir_all(trash.join("files")).expect("files");
    fs::create_dir_all(trash.join("info")).expect("info");
    trash
}

fn volume_trash(root: &Path, uid: u32) -> PathBuf {
    let trash = root.join(format!(".Trash-{uid}"));
    fs::create_dir_all(trash.join("files")).expect("files");
    fs::create_dir_all(trash.join("info")).expect("info");
    trash
}

#[test]
fn decode_trashinfo_path_percent_decodes_and_rejects_empty() {
    assert_eq!(
        decode_trashinfo_path("/home/user/My%20File.txt").as_deref(),
        Some(Path::new("/home/user/My File.txt"))
    );
    assert_eq!(
        decode_trashinfo_path("Documents/%2e%2e/%2e%2e/etc/passwd").as_deref(),
        Some(Path::new("Documents/../../etc/passwd"))
    );
    assert_eq!(decode_trashinfo_path(""), None);
    assert_eq!(decode_trashinfo_path("   "), None);
    assert_eq!(decode_trashinfo_path("%00"), None);
}

#[test]
fn relative_orig_path_is_joined_to_the_trash_parent() {
    let fixture = tempfile::tempdir().expect("fixture");
    let uid = 1000;
    let trash = volume_trash(fixture.path(), uid);
    let source = trash.join("files/report.txt");
    fs::write(&source, b"ok").expect("source");
    let context = context_for(&fixture.path().join("home-trash"), uid, fixture.path());
    let plan = plan_restore_from_known_paths(
        &source,
        Path::new("Documents/report.txt"),
        &trash,
        Some(trash.join("info/report.txt.trashinfo")),
        &context,
    )
    .expect("in-scope relative path");
    let mount = fixture.path().canonicalize().expect("canonical mount");
    assert_eq!(plan.destination, mount.join("Documents/report.txt"));
    assert_eq!(plan.allowed_root, mount);
}

#[test]
fn relative_orig_path_in_home_trash_is_joined_to_xdg_data_home() {
    let fixture = tempfile::tempdir().expect("fixture");
    let uid = 1000;
    let xdg_data = fixture.path().join("xdg-data");
    let trash = xdg_data.join("Trash");
    fs::create_dir_all(trash.join("files")).expect("files");
    fs::create_dir_all(trash.join("info")).expect("info");
    let source = trash.join("files/report.txt");
    fs::write(&source, b"ok").expect("source");
    let context = context_for(&trash, uid, fixture.path());
    let plan = plan_restore_from_known_paths(
        &source,
        Path::new("Documents/report.txt"),
        &trash,
        Some(trash.join("info/report.txt.trashinfo")),
        &context,
    )
    .expect("home-trash relative path");
    let data_home = xdg_data.canonicalize().expect("canonical xdg data");
    assert_eq!(plan.destination, data_home.join("Documents/report.txt"));
    assert_ne!(
        plan.destination,
        fixture
            .path()
            .canonicalize()
            .expect("canonical mount")
            .join("Documents/report.txt")
    );
}

#[test]
fn absolute_orig_path_on_the_same_volume_is_accepted() {
    let fixture = tempfile::tempdir().expect("fixture");
    let uid = 1000;
    let trash = volume_trash(fixture.path(), uid);
    let source = trash.join("files/cat.png");
    fs::write(&source, b"ok").expect("source");
    fs::create_dir_all(fixture.path().join("Photos")).expect("photos");
    let orig = fixture.path().join("Photos/cat.png");
    let context = context_for(&fixture.path().join("home-trash"), uid, fixture.path());
    let plan = plan_restore_from_known_paths(&source, &orig, &trash, None, &context).expect("plan");
    assert_eq!(
        plan.destination,
        fixture
            .path()
            .canonicalize()
            .expect("canonical topdir")
            .join("Photos/cat.png")
    );
}

#[test]
fn orig_path_outside_the_source_mount_is_rejected() {
    let fixture = tempfile::tempdir().expect("fixture");
    let uid = 1000;
    let trash = volume_trash(fixture.path(), uid);
    let source = trash.join("files/payload");
    fs::write(&source, b"bad").expect("source");
    let context = context_for(&fixture.path().join("home-trash"), uid, fixture.path());
    let error = plan_restore_from_known_paths(
        &source,
        Path::new("/home/victim/.config/autostart/payload.desktop"),
        &trash,
        None,
        &context,
    )
    .expect_err("home path from volume trash");
    assert!(
        error.message().contains("outside the trash volume"),
        "{}",
        error.message()
    );
}

#[test]
fn parent_dir_escape_is_rejected() {
    let fixture = tempfile::tempdir().expect("fixture");
    let uid = 1000;
    let trash = volume_trash(fixture.path(), uid);
    let source = trash.join("files/payload");
    fs::write(&source, b"bad").expect("source");
    let context = context_for(&fixture.path().join("home-trash"), uid, fixture.path());
    assert!(
        plan_restore_from_known_paths(
            &source,
            Path::new("../../home/victim/.ssh/authorized_keys"),
            &trash,
            None,
            &context,
        )
        .is_err()
    );
    assert!(
        plan_restore_from_known_paths(
            &source,
            &fixture.path().join("docs/../../../etc/passwd"),
            &trash,
            None,
            &context,
        )
        .is_err()
    );
}

#[test]
fn percent_encoded_parent_escape_is_rejected() {
    let fixture = tempfile::tempdir().expect("fixture");
    let uid = 1000;
    let trash = volume_trash(fixture.path(), uid);
    let source = trash.join("files/payload");
    fs::write(&source, b"bad").expect("source");
    let context = context_for(&fixture.path().join("home-trash"), uid, fixture.path());
    let orig =
        decode_trashinfo_path("%2e%2e/%2e%2e/home/victim/.bash_profile").expect("decoded escape");
    assert!(plan_restore_from_known_paths(&source, &orig, &trash, None, &context).is_err());
}

#[test]
fn similar_prefixes_are_not_treated_as_the_same_volume_root() {
    let fixture = tempfile::tempdir().expect("fixture");
    let uid = 1000;
    let trash = volume_trash(fixture.path(), uid);
    let source = trash.join("files/payload");
    fs::write(&source, b"bad").expect("source");
    let context = context_for(&fixture.path().join("home-trash"), uid, fixture.path());
    let sibling = PathBuf::from(format!("{}-evil/payload", fixture.path().display()));
    assert!(plan_restore_from_known_paths(&source, &sibling, &trash, None, &context).is_err());
}

#[test]
fn symlink_parent_that_leaves_the_volume_is_rejected() -> std::io::Result<()> {
    let Some((home, stick)) = crate::test_support::distinct_device_dirs(
        "symlink_parent_that_leaves_the_volume_is_rejected",
    ) else {
        return Ok(());
    };
    let uid = 1000;
    let trash = volume_trash(stick.path(), uid);
    let source = trash.join("files/payload");
    fs::write(&source, b"bad")?;
    let link = stick.path().join("link");
    symlink(home.path(), &link)?;
    fs::create_dir_all(home.path().join(".config/autostart"))?;
    let context = context_for(&home.path().join("home-trash"), uid, stick.path());
    let error = plan_restore_from_known_paths(
        &source,
        Path::new("link/.config/autostart/payload.desktop"),
        &trash,
        None,
        &context,
    )
    .expect_err("symlink escape");
    assert!(error.message().contains("outside the trash volume"));
    Ok(())
}

#[test]
fn destination_inside_the_trash_directory_is_rejected() {
    let fixture = tempfile::tempdir().expect("fixture");
    let uid = 1000;
    let trash = volume_trash(fixture.path(), uid);
    let source = trash.join("files/payload");
    fs::write(&source, b"bad").expect("source");
    let context = context_for(&fixture.path().join("home-trash"), uid, fixture.path());
    assert!(
        plan_restore_from_known_paths(
            &source,
            &trash.join("files/payload"),
            &trash,
            None,
            &context
        )
        .is_err()
    );
}

#[test]
fn different_device_orig_path_is_rejected_by_volume_identity() -> std::io::Result<()> {
    let Some((home, stick)) = crate::test_support::distinct_device_dirs(
        "different_device_orig_path_is_rejected_by_volume_identity",
    ) else {
        return Ok(());
    };
    let uid = rustix::process::getuid().as_raw();
    let trash = volume_trash(stick.path(), uid);
    let source = trash.join("files/payload");
    fs::write(&source, b"bad")?;
    let dest = home.path().join(".config/autostart/payload.desktop");
    let context = RestoreContext {
        home_trash_root: home.path().join("Trash"),
        uid,
        mounts: MountTable::current(),
    };
    let error = plan_restore_from_known_paths(&source, &dest, &trash, None, &context)
        .expect_err("cross-device orig-path");
    assert!(
        error.message().contains("outside the trash volume"),
        "{}",
        error.message()
    );
    Ok(())
}

#[test]
fn lexical_normalize_collapses_dot_and_rejects_parent_dir() {
    assert_eq!(
        lexically_normalize(Path::new("/media/usb/./docs/a.txt")).as_deref(),
        Some(Path::new("/media/usb/docs/a.txt"))
    );
    assert_eq!(
        lexically_normalize(Path::new("/media/usb/../../etc/passwd")),
        None
    );
}

#[test]
fn missing_mount_table_refuses_restore() {
    let fixture = tempfile::tempdir().expect("fixture");
    let uid = 1000;
    let trash = volume_trash(fixture.path(), uid);
    let source = trash.join("files/report.txt");
    fs::write(&source, b"ok").expect("source");
    let context = RestoreContext {
        home_trash_root: fixture.path().join("home-trash"),
        uid,
        mounts: MountTable::parse(""),
    };
    let error = plan_restore_from_known_paths(
        &source,
        Path::new("Documents/report.txt"),
        &trash,
        None,
        &context,
    )
    .expect_err("empty mount table");
    assert!(
        error.message().contains("mount table is unavailable"),
        "{}",
        error.message()
    );
}

#[test]
fn relative_trashinfo_path_matches_the_topdir_resolved_orig_path() {
    let fixture = tempfile::tempdir().expect("fixture");
    let trash = volume_trash(fixture.path(), 1000);
    fs::write(trash.join("files/report.txt"), b"ok").expect("source");
    fs::write(
        trash.join("info/report.txt.trashinfo"),
        "[Trash Info]\nPath=Documents/report.txt\nDeletionDate=2026-01-01T00:00:00\n",
    )
    .expect("info");

    let found = find_trash_item_by_orig_path(&trash, &fixture.path().join("Documents/report.txt"))
        .expect("relative Path= resolves against the topdir");

    assert_eq!(found.source_path, trash.join("files/report.txt"));
    assert_eq!(found.trash_root, trash);
    assert!(
        find_trash_item_by_orig_path(&trash, Path::new("/elsewhere/Documents/report.txt"))
            .is_none()
    );
}

#[test]
fn orig_path_match_is_preferred_over_a_basename_collision() {
    let fixture = tempfile::tempdir().expect("fixture");
    let uid = 1000;
    let home_trash = fixture.path().join("home-trash");
    fs::create_dir_all(home_trash.join("files")).expect("home files");
    fs::create_dir_all(home_trash.join("info")).expect("home info");
    fs::write(home_trash.join("files/report.txt"), b"home").expect("home source");
    fs::write(
        home_trash.join("info/report.txt.trashinfo"),
        "[Trash Info]\nPath=/elsewhere/report.txt\nDeletionDate=2026-01-01T00:00:00\n",
    )
    .expect("home info");
    let trash = volume_trash(fixture.path(), uid);
    fs::write(trash.join("files/report.txt"), b"volume").expect("volume source");
    fs::write(
        trash.join("info/report.txt.trashinfo"),
        "[Trash Info]\nPath=Documents/report.txt\nDeletionDate=2026-01-01T00:00:00\n",
    )
    .expect("volume info");
    let context = context_for(&home_trash, uid, fixture.path());
    let orig = fixture.path().join("Documents/report.txt");

    let discovered = discover_trash_item(
        &Location::uri("trash:///report.txt"),
        None,
        Some(&orig),
        &context,
    )
    .expect("orig-path hit");

    assert_eq!(discovered.source_path, trash.join("files/report.txt"));
    assert_eq!(
        discovered.trash_info.as_deref(),
        Some(trash.join("info/report.txt.trashinfo").as_path())
    );
}

#[test]
fn shared_trash_dir_requires_sticky_bit_and_rejects_symlinks() -> std::io::Result<()> {
    let fixture = tempfile::tempdir()?;
    let uid = 1000;
    let unsticky = fixture.path().join(".Trash");
    fs::create_dir(&unsticky)?;
    assert_eq!(valid_shared_trash_dir(fixture.path(), uid), None);

    let mut permissions = fs::metadata(&unsticky)?.permissions();
    permissions.set_mode(0o1777);
    fs::set_permissions(&unsticky, permissions)?;
    assert_eq!(
        valid_shared_trash_dir(fixture.path(), uid).as_deref(),
        Some(unsticky.join(uid.to_string()).as_path())
    );

    fs::remove_dir(&unsticky)?;
    symlink("/tmp", &unsticky)?;
    assert_eq!(valid_shared_trash_dir(fixture.path(), uid), None);
    Ok(())
}

#[test]
fn gvfs_named_volume_item_is_discovered_from_its_relative_trashinfo() {
    let fixture = tempfile::tempdir().expect("fixture");
    let uid = 1000;
    let trash = volume_trash(fixture.path(), uid);
    fs::write(trash.join("files/report.txt"), b"ok").expect("source");
    fs::write(
        trash.join("info/report.txt.trashinfo"),
        "[Trash Info]\nPath=Documents/report.txt\nDeletionDate=2026-01-01T00:00:00\n",
    )
    .expect("info");
    let context = context_for(&fixture.path().join("home-trash"), uid, fixture.path());
    let escaped = format!(
        "trash:///{}",
        glib::Uri::escape_string(
            &format!("{}/.Trash-{uid}/files/report.txt", fixture.path().display())
                .replace('/', "\\"),
            None,
            false,
        )
    );

    let discovered = discover_trash_item(
        &Location::uri(escaped),
        None,
        Some(&fixture.path().join("Documents/report.txt")),
        &context,
    )
    .expect("volume item found through its orig path");

    assert_eq!(discovered.source_path, trash.join("files/report.txt"));
    assert_eq!(
        discovered.trash_info.as_deref(),
        Some(trash.join("info/report.txt.trashinfo").as_path())
    );
}

#[test]
fn trash_item_from_files_path_requires_a_files_entry() {
    let item = trash_item_from_files_path(Path::new("/media/usb/.Trash-1000/files/report.txt"))
        .expect("files entry");
    assert_eq!(item.trash_root, Path::new("/media/usb/.Trash-1000"));
    assert_eq!(
        item.trash_info.as_deref(),
        Some(Path::new(
            "/media/usb/.Trash-1000/info/report.txt.trashinfo"
        ))
    );
    assert!(trash_item_from_files_path(Path::new("/media/usb/Documents/report.txt")).is_none());
    assert!(trash_item_from_files_path(Path::new("/media/usb/.Trash-1000/info/x")).is_none());
}

#[test]
fn trash_root_is_derived_from_the_files_entry() {
    let path = Path::new("/media/usb/.Trash-1000/files/report.txt");
    assert_eq!(
        trash_root_from_files_path(path).as_deref(),
        Some(Path::new("/media/usb/.Trash-1000"))
    );
    assert_eq!(
        trash_root_from_files_path(Path::new("/tmp/not-trash/report.txt")),
        None
    );
}

#[test]
fn relative_orig_path_in_shared_trash_is_joined_to_the_volume_topdir() {
    let mount = tempfile::tempdir().expect("mount");
    let mount_path = mount.path().canonicalize().expect("canonical mount");
    let uid = 1000;
    let trash = shared_volume_trash(&mount_path, uid);
    let source = trash.join("files/report.txt");
    fs::write(&source, b"report").expect("source");
    let context = context_for(&mount_path.join("unused"), uid, &mount_path);

    let plan = plan_restore_from_known_paths(
        &source,
        Path::new("Documents/report.txt"),
        &trash,
        None,
        &context,
    )
    .expect("shared trash restore");

    assert_eq!(plan.destination, mount_path.join("Documents/report.txt"));
    assert_eq!(plan.allowed_root, mount_path);
}

#[test]
fn shared_trash_item_is_discovered_from_its_relative_trashinfo() {
    let mount = tempfile::tempdir().expect("mount");
    let mount_path = mount.path().canonicalize().expect("canonical mount");
    let uid = 1000;
    let trash = shared_volume_trash(&mount_path, uid);
    fs::write(trash.join("files/report.txt"), b"report").expect("source");
    fs::write(
        trash.join("info/report.txt.trashinfo"),
        "[Trash Info]\nPath=Documents/report.txt\nDeletionDate=2026-01-01T00:00:00\n",
    )
    .expect("info");

    let found = find_trash_item_by_orig_path(&trash, &mount_path.join("Documents/report.txt"))
        .expect("discovered by orig path");

    assert_eq!(found.source_path, trash.join("files/report.txt"));
    assert_eq!(found.trash_root, trash);
}
