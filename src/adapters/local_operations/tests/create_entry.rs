// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{CreateDirectoryRequest, CreateFileRequest};

fn create(
    parent: &Path,
    name: &str,
    unique_name: bool,
    count: usize,
    directory: bool,
) -> Vec<OperationEvent> {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock();
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("exclusive main context");
    let events = Rc::new(RefCell::new(Vec::new()));
    let _loads: Vec<_> = (0..count)
        .map(|id| {
            let received = events.clone();
            let emit: Rc<dyn Fn(OperationEvent)> =
                Rc::new(move |event| received.borrow_mut().push(event));
            if directory {
                LocalOperationProvider.create_directory(
                    CreateDirectoryRequest {
                        id: OperationRequestId(id as u64),
                        parent: Location::local(parent),
                        name: name.to_owned(),
                        unique_name,
                    },
                    emit,
                )
            } else {
                LocalOperationProvider.create_file(
                    CreateFileRequest {
                        id: OperationRequestId(id as u64),
                        parent: Location::local(parent),
                        name: name.to_owned(),
                        unique_name,
                    },
                    emit,
                )
            }
        })
        .collect();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while events.borrow().len() < count {
        assert!(
            std::time::Instant::now() < deadline,
            "creation did not finish"
        );
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
    events.take()
}

#[test]
fn unique_creation_retries_atomic_collisions_without_overwriting() {
    for directory in [false, true] {
        let fixture = tempfile::tempdir().expect("fixture");
        let name = if directory { "new folder" } else { "new file" };
        fs::write(fixture.path().join(name), b"keep").expect("existing file");
        std::os::unix::fs::symlink("missing", fixture.path().join(format!("{name} (1)")))
            .expect("dangling link");
        fs::create_dir(fixture.path().join(format!("{name} (2)"))).expect("existing folder");
        let events = create(fixture.path(), name, true, 2, directory);
        let created: HashSet<_> = events
            .into_iter()
            .map(|event| match event {
                OperationEvent::EntryCreated { location, .. } => location,
                other => panic!("unexpected creation event: {other:?}"),
            })
            .collect();
        assert_eq!(
            created,
            HashSet::from([
                Location::local(fixture.path().join(format!("{name} (3)"))),
                Location::local(fixture.path().join(format!("{name} (4)"))),
            ])
        );
        assert_eq!(
            fs::read(fixture.path().join(name)).expect("existing contents"),
            b"keep"
        );
        assert!(fixture.path().join(format!("{name} (1)")).is_symlink());
        assert!(fixture.path().join(format!("{name} (2)")).is_dir());
        for suffix in [3, 4] {
            let path = fixture.path().join(format!("{name} ({suffix})"));
            if directory {
                assert!(path.is_dir());
            } else {
                assert_eq!(fs::read(path).expect("empty new file"), b"");
            }
        }
    }
}

#[test]
fn unique_creation_uses_the_first_available_name() {
    for directory in [false, true] {
        let fixture = tempfile::tempdir().expect("fixture");
        let name = if directory { "new folder" } else { "new file" };
        fs::create_dir(fixture.path().join(format!("{name} (1)"))).expect("numbered folder");
        assert!(
            matches!(create(fixture.path(), name, true, 1, directory).as_slice(), [OperationEvent::EntryCreated { location, .. }] if location == &Location::local(fixture.path().join(name)))
        );
        fs::remove_dir(fixture.path().join(format!("{name} (1)"))).expect("release first suffix");
        fs::create_dir(fixture.path().join(format!("{name} (2)"))).expect("later suffix");
        assert!(
            matches!(create(fixture.path(), name, true, 1, directory).as_slice(), [OperationEvent::EntryCreated { location, .. }] if location == &Location::local(fixture.path().join(format!("{name} (1)"))))
        );
    }
}

#[test]
fn exact_creation_reports_conflicts_and_unique_creation_rejects_invalid_names() {
    for directory in [false, true] {
        let fixture = tempfile::tempdir().expect("fixture");
        fs::write(fixture.path().join("existing"), b"keep").expect("existing item");
        assert!(matches!(
            create(fixture.path(), "existing", false, 1, directory).as_slice(),
            [OperationEvent::Failed { .. }]
        ));
        for name in ["", "   ", ".", "..", "nested/name", "nul\0name"] {
            assert!(matches!(
                create(fixture.path(), name, true, 1, directory).as_slice(),
                [OperationEvent::Failed { .. }]
            ));
        }
        assert_eq!(fs::read_dir(fixture.path()).expect("listing").count(), 1);
        assert_eq!(
            fs::read(fixture.path().join("existing")).expect("existing contents"),
            b"keep"
        );
        assert!(matches!(
            create(fixture.path(), "exact", false, 1, directory).as_slice(),
            [OperationEvent::Created { .. }]
        ));
        assert_eq!(fixture.path().join("exact").is_dir(), directory);
    }
}
