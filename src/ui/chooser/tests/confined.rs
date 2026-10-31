// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    adapters::{FsOp, FsQuirks, Quirks},
    services::SearchItem,
    test_support::Gate,
    ui::browser::search_result_entry,
};
use std::time::Instant;

fn pump_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_confined_chooser_checks_paths_at_once_and_where_folders_lead_on_a_worker() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::confined::a_confined_chooser_checks_paths_at_once_and_where_folders_lead_on_a_worker",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let root = fixture.path().canonicalize().expect("fixture root");
            let device = root.join("device");
            let outside = root.join("outside");
            std::fs::create_dir_all(device.join("folder")).expect("device folder");
            std::fs::create_dir_all(&outside).expect("outside folder");
            std::fs::write(device.join("file.txt"), b"file").expect("device file");
            std::os::unix::fs::symlink(&outside, device.join("escape")).expect("escaping link");
            std::os::unix::fs::symlink(device.join("folder"), device.join("inside"))
                .expect("inner link");
            let source = ChooserFileSource {
                source: Rc::new(LocalFileSource),
                filter: Rc::new(RefCell::new(None)),
                directory_only: Rc::new(Cell::new(true)),
                root_limit: Some(device.clone()),
            };

            for (path, allowed) in [
                (device.join("folder"), true),
                (device.join("not listed yet/deeper"), true),
                (device.join("escape"), true),
                (outside.clone(), false),
                (device.join("../outside"), false),
            ] {
                let entry = search_result_entry(&SearchItem::for_test(path.clone(), true));
                assert_eq!(source.allows_entry(&entry), allowed, "{}", path.display());
            }

            let validate = |path: &Path| {
                let answer = Rc::new(RefCell::new(None));
                let delivered = answer.clone();
                let load = source.validate_location_async(
                    Location::local(path),
                    Rc::new(move |result| {
                        delivered.replace(Some(result));
                    }),
                );
                (answer, load)
            };

            let gate = Gate::default();
            let scope = FsQuirks::install(
                &device,
                Quirks {
                    before: vec![(FsOp::FolderStamp, "folder", 0, gate.racer())],
                    ..Quirks::default()
                },
            );
            let (answer, _load) = validate(&device.join("folder"));
            pump_until("the check reaches a worker", || gate.reached());
            assert!(answer.borrow().is_none(), "nothing is decided while parked");
            let ticked = Rc::new(Cell::new(false));
            let tick = ticked.clone();
            glib::timeout_add_local_once(Duration::from_millis(5), move || tick.set(true));
            pump_until("the window keeps running", || ticked.get());
            gate.release();
            pump_until("the check answers", || answer.borrow().is_some());
            assert_eq!(answer.take(), Some(Ok(())));
            drop(scope);

            for (path, expected) in [
                (device.join("inside"), Ok(())),
                (
                    device.join("escape"),
                    Err(LocationValidationError::Refused(OUTSIDE_ROOT)),
                ),
                (
                    device.join("missing"),
                    Err(LocationValidationError::Missing),
                ),
                (
                    device.join("file.txt"),
                    Err(LocationValidationError::NotDirectory),
                ),
                (
                    outside.clone(),
                    Err(LocationValidationError::Refused(OUTSIDE_ROOT)),
                ),
            ] {
                let (answer, _load) = validate(&path);
                pump_until("the check answers", || answer.borrow().is_some());
                assert_eq!(answer.take(), Some(expected), "{}", path.display());
            }

            let watch = |path: &Path| {
                let changes = Rc::new(RefCell::new(Vec::new()));
                let sink = changes.clone();
                let handle = source.start_watch(
                    Location::local(path),
                    false,
                    Rc::new(move |change| sink.borrow_mut().push(change)),
                    Rc::new(|_| {}),
                );
                (changes, handle)
            };
            let (escaped, _escape_watch) = watch(&device.join("escape"));
            let (inner, _inner_watch) = watch(&device.join("inside"));
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut round = 0;
            while inner.borrow().is_empty() {
                assert!(
                    Instant::now() < deadline,
                    "the inner watch reports a new folder"
                );
                for folder in [device.join("folder"), outside.clone()] {
                    std::fs::create_dir(folder.join(format!("arrived-{round}")))
                        .expect("new folder");
                }
                round += 1;
                pump_for(Duration::from_millis(150));
            }
            pump_for(Duration::from_millis(300));
            assert!(
                escaped.borrow().is_empty(),
                "a folder leading off the device reports nothing from outside"
            );
        },
    );
}
