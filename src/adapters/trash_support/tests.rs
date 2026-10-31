// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

use super::*;
use crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT;

const HEALTHY: FuseFacts = FuseFacts {
    same_device_as_home: false,
    item_present: true,
    mount_reachable: true,
    parent_writable: true,
};

fn io(code: gio::IOErrorEnum) -> glib::Error {
    glib::Error::new(code, "fixture")
}

fn file_error(code: glib::FileError) -> glib::Error {
    glib::Error::new(code, "fixture")
}

#[test]
fn classifier_maps_errors_by_mount_and_mount_health() {
    use gio::IOErrorEnum as Io;
    use glib::FileError as File;
    let refused = TrashFailure::Unsupported(UnsupportedReason::FuseRefused);
    let reported = TrashFailure::Unsupported(UnsupportedReason::Reported);
    let cases = [
        (
            io(Io::Cancelled),
            Some("fuse.sshfs"),
            Some(HEALTHY),
            TrashFailure::Cancelled,
        ),
        (io(Io::NotSupported), Some("ext4"), None, reported),
        (io(Io::NotSupported), None, None, reported),
        (
            io(Io::NotFound),
            Some("fuse.sshfs"),
            Some(HEALTHY),
            TrashFailure::NotFound,
        ),
        (
            io(Io::ReadOnly),
            Some("fuse.sshfs"),
            Some(HEALTHY),
            TrashFailure::ReadOnly,
        ),
        (
            io(Io::NoSpace),
            Some("fuse.sshfs"),
            Some(HEALTHY),
            TrashFailure::NoSpace,
        ),
        (io(Io::Failed), Some("fuse.sshfs"), Some(HEALTHY), refused),
        (
            io(Io::PermissionDenied),
            Some("fuse.rclone"),
            Some(HEALTHY),
            refused,
        ),
        (
            io(Io::InvalidArgument),
            Some("fuseblk"),
            Some(HEALTHY),
            refused,
        ),
        (
            file_error(File::Failed),
            Some("fuse.sshfs"),
            Some(HEALTHY),
            refused,
        ),
        (
            file_error(File::Nospc),
            Some("fuse.sshfs"),
            Some(HEALTHY),
            TrashFailure::NoSpace,
        ),
        (
            file_error(File::Nametoolong),
            Some("fuse.sshfs"),
            Some(HEALTHY),
            TrashFailure::Other,
        ),
        (
            io(Io::Exists),
            Some("fuse.sshfs"),
            Some(HEALTHY),
            TrashFailure::Other,
        ),
        (
            io(Io::Failed),
            Some("ext4"),
            Some(HEALTHY),
            TrashFailure::Other,
        ),
        (
            io(Io::PermissionDenied),
            Some("tmpfs"),
            Some(HEALTHY),
            TrashFailure::Other,
        ),
        (
            io(Io::Failed),
            Some("fuse.sshfs"),
            None,
            TrashFailure::Other,
        ),
        (
            io(Io::Failed),
            Some("fuse.sshfs"),
            Some(FuseFacts {
                same_device_as_home: true,
                ..HEALTHY
            }),
            TrashFailure::Other,
        ),
        (
            io(Io::Failed),
            Some("fuse.sshfs"),
            Some(FuseFacts {
                item_present: false,
                ..HEALTHY
            }),
            TrashFailure::Other,
        ),
        (
            io(Io::Failed),
            Some("fuse.sshfs"),
            Some(FuseFacts {
                mount_reachable: false,
                ..HEALTHY
            }),
            TrashFailure::Other,
        ),
        (
            io(Io::PermissionDenied),
            Some("fuse.sshfs"),
            Some(FuseFacts {
                parent_writable: false,
                ..HEALTHY
            }),
            TrashFailure::Other,
        ),
    ];
    for (error, fs_type, facts, expected) in cases {
        assert_eq!(
            decide(&error, fs_type, facts),
            expected,
            "{error:?} on {fs_type:?} with {facts:?}"
        );
    }
}

/// Canonical, because the classifier resolves symlinks before looking up a mount.
fn mounted_at(root: &Path, mount_id: u64, fs_type: &str) -> MountTable {
    let root = fs::canonicalize(root).expect("fixture root");
    let mount_point = root.to_str().expect("UTF-8 fixture path");
    MountTable::parse(format!(
        "1 0 8:1 / / rw - ext4 /dev/root rw\n\
         {mount_id} 1 0:{mount_id} / {mount_point} rw - {fs_type} fixture rw\n",
    ))
}

#[test]
fn unsupported_mounts_are_remembered_per_mount_instance() {
    crate::test_support::gtk_test(
        "adapters::trash_support::tests::unsupported_mounts_are_remembered_per_mount_instance",
        || {
            let root = tempfile::tempdir().expect("tempdir");
            let inside = root.path().join("folder/file");
            let reason_while_published = |mounts: MountTable| {
                crate::adapters::mount_watch::publish_for_isolated_test(Some(mounts));
                unsupported_reason_for(&inside)
            };
            let mounts = mounted_at(root.path(), 7101, "fuse.sshfs");
            let mount = mounts.mount_for(&inside).expect("fixture mount");
            record_unsupported(mount.clone(), UnsupportedReason::FuseRefused);
            assert_eq!(
                reason_while_published(mounts),
                Some((mount, UnsupportedReason::FuseRefused))
            );
            assert_eq!(
                reason_while_published(mounted_at(root.path(), 7102, "fuse.sshfs")),
                None,
                "a remount is a different mount"
            );

            let local = mounted_at(root.path(), 7103, "ext4");
            record_unsupported(
                local.mount_for(&inside).expect("fixture mount"),
                UnsupportedReason::Reported,
            );
            assert_eq!(
                reason_while_published(local),
                None,
                "GIO's probe is accurate outside FUSE"
            );
        },
    );
}

/// Treats a tempdir as a mount whose backend refuses the trash rename.
pub(in crate::adapters) struct RefusingMount {
    pub(in crate::adapters) mounts: MountTable,
    attempts: Cell<usize>,
}

impl RefusingMount {
    pub(in crate::adapters) fn at(root: &Path, mount_id: u64, fs_type: &str) -> Self {
        Self {
            mounts: mounted_at(root, mount_id, fs_type),
            attempts: Cell::new(0),
        }
    }
}

impl TrashEnvironment for RefusingMount {
    fn mounts(&self) -> MountTable {
        self.mounts.clone()
    }

    fn home(&self) -> PathBuf {
        PathBuf::from("/proc")
    }

    /// Fails like GLib after it created the trash folders, which it leaves behind.
    fn trash(
        &self,
        file: &gio::File,
        _cancellable: &gio::Cancellable,
        _deadline: &crate::adapters::local_operations::Deadline,
    ) -> Pin<Box<dyn Future<Output = Result<Option<crate::services::ItemIdentity>, glib::Error>>>>
    {
        self.attempts.set(self.attempts.get() + 1);
        let parent = fs::canonicalize(file.path().expect("native").parent().expect("parent"))
            .expect("parent");
        let root = self.mounts.mount_point_for(&parent).expect("mount");
        let trash = root.join(format!(".Trash-{}", rustix::process::getuid().as_raw()));
        fs::create_dir_all(trash.join("info")).expect("info");
        fs::create_dir_all(trash.join("files")).expect("files");
        Box::pin(std::future::ready(Err(io(gio::IOErrorEnum::Failed))))
    }
}

pub(in crate::adapters) fn leftover_trash(root: &Path) -> PathBuf {
    root.join(format!(".Trash-{}", rustix::process::getuid().as_raw()))
}

#[test]
fn a_refused_trash_on_fuse_is_recorded_and_its_leftovers_removed() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock");
    let as_root = rustix::process::geteuid().is_root();
    for (fs_type, item_present, folder_writable, expected) in [
        (
            "fuse.sshfs",
            true,
            true,
            TrashFailure::Unsupported(UnsupportedReason::FuseRefused),
        ),
        ("fuse.sshfs", false, true, TrashFailure::Other),
        ("fuse.sshfs", true, false, TrashFailure::Other),
        ("ext4", true, true, TrashFailure::Other),
    ] {
        if as_root && !folder_writable {
            continue;
        }
        let root = tempfile::tempdir().expect("tempdir");
        let folder = root.path().join("folder");
        fs::create_dir(&folder).expect("folder");
        let item = folder.join("doomed.txt");
        if item_present {
            fs::write(&item, b"doomed").expect("fixture");
        }
        if !folder_writable {
            fs::set_permissions(&folder, fs::Permissions::from_mode(0o500)).expect("read-only");
        }
        let environment = RefusingMount::at(root.path(), 7201, fs_type);
        let result = glib::MainContext::default().block_on(trash_with_cleanup_in(
            &environment,
            &gio::File::for_path(&item),
            &gio::Cancellable::new(),
        ));
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).expect("writable");
        let case = format!("{fs_type}, item present: {item_present}, writable: {folder_writable}");
        assert_eq!(environment.attempts.get(), 1, "{case}");
        assert_eq!(result.expect_err("trash fails").failure, expected, "{case}");
        let unsupported = expected != TrashFailure::Other;
        assert_eq!(
            unsupported_reason_in(&environment.mounts, &item).is_some(),
            unsupported,
            "{case}"
        );
        assert_eq!(leftover_trash(root.path()).exists(), !unsupported, "{case}");
        assert_eq!(item.exists(), item_present, "{case}");
    }
}

#[test]
fn cleanup_removes_only_new_empty_trash_folders() {
    let uid = rustix::process::getuid().as_raw().to_string();
    let trash_dir = |root: &Path| root.join(format!(".Trash-{uid}"));
    let create = |path: &Path| {
        fs::create_dir_all(path.join("info")).expect("info");
        fs::create_dir_all(path.join("files")).expect("files");
    };

    let created = tempfile::tempdir().expect("tempdir");
    let snapshot = TrashDirSnapshot::take(created.path());
    create(&trash_dir(created.path()));
    snapshot.remove_created();
    assert!(
        !trash_dir(created.path()).exists(),
        "a new empty folder is removed"
    );

    let existing = tempfile::tempdir().expect("tempdir");
    create(&trash_dir(existing.path()));
    TrashDirSnapshot::take(existing.path()).remove_created();
    assert!(
        trash_dir(existing.path()).join("files").is_dir(),
        "an existing folder stays"
    );

    let occupied = tempfile::tempdir().expect("tempdir");
    let snapshot = TrashDirSnapshot::take(occupied.path());
    create(&trash_dir(occupied.path()));
    fs::write(trash_dir(occupied.path()).join("files/kept.txt"), b"kept").expect("trashed");
    snapshot.remove_created();
    assert!(
        trash_dir(occupied.path()).join("files/kept.txt").exists(),
        "a non-empty folder stays"
    );

    let linked = tempfile::tempdir().expect("tempdir");
    let elsewhere = linked.path().join("elsewhere");
    let snapshot = TrashDirSnapshot::take(linked.path());
    create(&elsewhere);
    symlink(&elsewhere, trash_dir(linked.path())).expect("symlink");
    snapshot.remove_created();
    assert!(
        trash_dir(linked.path()).is_symlink(),
        "a symlinked candidate stays"
    );
    assert!(elsewhere.join("info").is_dir() && elsewhere.join("files").is_dir());

    let sticky = tempfile::tempdir().expect("tempdir");
    let shared = sticky.path().join(".Trash");
    fs::create_dir(&shared).expect(".Trash");
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o1777)).expect("sticky");
    let snapshot = TrashDirSnapshot::take(sticky.path());
    create(&shared.join(&uid));
    snapshot.remove_created();
    assert!(
        !shared.join(&uid).exists(),
        "a new per-user folder in .Trash is removed"
    );
    assert!(shared.is_dir(), "the shared .Trash stays");
}
