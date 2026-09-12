// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn autofs_classification_skips_caller_thread_prefix_probes() {
    let mounts = MountTable::parse(
        "1 0 0:1 / / rw - ext4 /dev/root rw\n\
         2 1 0:2 / /automount rw - autofs systemd-1 rw\n\
         3 2 8:1 / /automount/local rw - ext4 /dev/disk rw\n",
    );
    for path in ["/automount", "/automount/share", "/automount/local/file"] {
        let location = Location::local(path);
        let directory = Directory::classify_with_probe(&location, &mounts, |_| {
            panic!("autofs path {path} must not probe prefixes on the caller");
        });
        assert!(!directory.resolves_synchronously(), "{path}");
        assert!(!mounts.is_remote_path(Path::new(path)), "{path}");
    }

    let sibling = Location::local("/automount-data/file");
    let probed = std::cell::Cell::new(false);
    let directory = Directory::classify_with_probe(&sibling, &mounts, |path| {
        assert_eq!(path, sibling.native_path().expect("native path"));
        probed.set(true);
        false
    });
    assert!(
        probed.get(),
        "ordinary local paths still check for symlinks"
    );
    assert!(directory.resolves_synchronously());
}

#[test]
fn pending_lookup_uses_its_original_classification_and_finishes_promptly() {
    let root = tempfile::tempdir().expect("fixture");
    let link = root.path().join("destination");
    std::os::unix::fs::symlink(root.path(), &link).expect("symlink");
    let location = Location::local(&link);
    let directory = Directory::classify(&location, &MountTable::current());
    fs::remove_file(&link).expect("remove link");
    fs::create_dir(&link).expect("replace link with directory");
    assert!(!directory.resolves_synchronously());

    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let result = Rc::new(RefCell::new(None));
            let sink = result.clone();
            let pending = PendingVolumeLookup::start(
                &[directory],
                Box::new(move |lookup| {
                    *sink.borrow_mut() = Some(lookup);
                }),
            );
            assert!(result.borrow().is_none(), "callback must not be reentrant");
            let started = Instant::now();
            while result.borrow().is_none() && started.elapsed() < REMOTE_QUERY_TIMEOUT / 2 {
                context.iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(
                result.borrow().is_some(),
                "completion must not wait for the timeout"
            );
            drop(pending);
        })
        .expect("private context");
}
