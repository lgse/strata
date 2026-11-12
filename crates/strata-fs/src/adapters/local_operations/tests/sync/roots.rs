// SPDX-License-Identifier: MIT

use super::*;

fn hint(root: &str, removable: bool) -> MountFlushHint {
    MountFlushHint {
        root: PathBuf::from(root),
        removable,
    }
}

#[test]
fn removable_copy_roots_are_synced_once() {
    let hints = [
        hint("/media/usb", true),
        hint("/home", false),
        hint("/", false),
    ];

    let removable = removable_sync_roots(
        &[SyncDestination::Local(Path::new("/media/usb/photos/a.jpg"))],
        &hints,
    );
    assert_eq!(removable, vec![PathBuf::from("/media/usb")]);

    let internal = removable_sync_roots(
        &[SyncDestination::Local(Path::new("/home/user/a.jpg"))],
        &hints,
    );
    assert!(internal.is_empty());

    let once = removable_sync_roots(
        &[
            SyncDestination::Local(Path::new("/media/usb/a.jpg")),
            SyncDestination::Local(Path::new("/media/usb/nested/b.jpg")),
        ],
        &hints,
    );
    assert_eq!(once, vec![PathBuf::from("/media/usb")]);

    let unknown = removable_sync_roots(
        &[SyncDestination::Local(Path::new("/var/tmp/file"))],
        &[hint("/media/usb", true)],
    );
    assert!(unknown.is_empty());

    let remote = removable_sync_roots(&[SyncDestination::NonLocal], &hints);
    assert!(remote.is_empty());
}

#[test]
fn sync_filesystem_on_a_local_directory_succeeds() {
    let directory = tempfile::tempdir().expect("temp directory");
    sync_filesystem(directory.path()).expect("syncfs on a local directory");
}
