// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

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
