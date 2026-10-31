// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{PlaceEntryRequest, TypedPath};

fn place(source: &Path, directory: bool, typed: &str, new_item: bool) -> OperationEvent {
    let _guard = ASYNC_MAIN_CONTEXT_DEFAULT.lock();
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("exclusive main context");
    let events = Rc::new(RefCell::new(Vec::new()));
    let received = events.clone();
    let entry = if directory {
        directory_entry(source)
    } else {
        file_entry(source)
    };
    let _load = LocalOperationProvider.place_entry(
        PlaceEntryRequest {
            id: OperationRequestId(1),
            entry,
            path: TypedPath::parse(typed, directory, true).expect("valid path"),
            new_item,
        },
        Rc::new(move |event| received.borrow_mut().push(event)),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while events.borrow().is_empty() {
        assert!(Instant::now() < deadline, "placement did not finish");
        context.iteration(false);
        thread::sleep(Duration::from_millis(1));
    }
    let mut events = events.take();
    assert_eq!(events.len(), 1, "{events:?}");
    events.remove(0)
}

fn fixture(
    directory: bool,
    contents: &[u8],
) -> Result<(tempfile::TempDir, PathBuf), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("kept"))?;
    fs::write(root.path().join("kept/other"), "keep")?;
    fs::write(root.path().join("file"), "keep")?;
    std::os::unix::fs::symlink(root.path().join("kept"), root.path().join("link"))?;
    let source = root
        .path()
        .join(if directory { "new folder" } else { "new file" });
    if directory {
        fs::create_dir(&source)?;
    } else {
        fs::write(&source, contents)?;
    }
    Ok((root, source))
}

struct Row {
    directory: bool,
    contents: &'static [u8],
    new_item: bool,
    typed: &'static str,
    placed: &'static [&'static str],
}

#[test]
fn an_item_is_placed_at_its_typed_path() -> Result<(), Box<dyn Error>> {
    let new_file = |typed, placed| Row {
        directory: false,
        contents: b"",
        new_item: true,
        typed,
        placed,
    };
    let rows = [
        new_file("docs/", &["docs"]),
        new_file("notes/today.txt", &["notes", "notes/today.txt"]),
        new_file("kept/today.txt", &["kept/today.txt"]),
        new_file("kept/fresh/", &["kept/fresh"]),
        Row {
            directory: true,
            ..new_file("a/b/", &["a", "a/b"])
        },
        Row {
            contents: b"draft",
            new_item: false,
            ..new_file("notes/today.txt", &["notes", "notes/today.txt"])
        },
        Row {
            directory: true,
            new_item: false,
            ..new_file("kept/moved", &["kept/moved"])
        },
    ];
    for Row {
        directory,
        contents,
        new_item,
        typed,
        placed: created,
    } in rows
    {
        let (root, source) = fixture(directory, contents)?;

        let event = place(&source, directory, typed, new_item);

        let expected: Vec<_> = created
            .iter()
            .map(|path| Location::local(root.path().join(path)))
            .collect();
        assert!(
            matches!(&event, OperationEvent::EntryPlaced { placed, .. } if *placed == expected),
            "{typed}: {event:?}"
        );
        assert!(!source.exists(), "{typed}");
        let placed = root.path().join(created[created.len() - 1]);
        assert_eq!(
            placed.is_dir(),
            directory || typed.ends_with('/'),
            "{typed}"
        );
        if !placed.is_dir() {
            assert_eq!(fs::read(&placed)?, contents, "{typed}");
        }
        assert_eq!(fs::read(root.path().join("kept/other"))?, b"keep");
    }
    Ok(())
}

#[test]
fn a_refused_placement_keeps_the_new_item_and_removes_the_folders_it_made()
-> Result<(), Box<dyn Error>> {
    let not_a_folder =
        |name: &str| rust_i18n::t!("“%{name}” is not a folder", name = name).into_owned();
    let exists = |name: &str| rust_i18n::t!("“%{name}” already exists", name = name).into_owned();
    let rows = [
        ("file/today.txt", &b""[..], not_a_folder("file")),
        ("link/today.txt", b"", not_a_folder("link")),
        ("kept/other", b"", exists("other")),
        ("kept/other/", b"", exists("other")),
        (
            "made/deeper/docs/",
            b"typed meanwhile",
            rust_i18n::t!(
                "%{name} changed while the operation was in progress",
                name = "new file"
            )
            .into_owned(),
        ),
    ];
    for (typed, contents, message) in rows {
        let (root, source) = fixture(false, contents)?;
        let before = names(root.path())?;

        let event = place(&source, false, typed, true);

        assert!(
            matches!(&event, OperationEvent::Failed { message: failed, .. } if *failed == message),
            "{typed}: {event:?}"
        );
        assert_eq!(names(root.path())?, before, "{typed}");
        assert_eq!(fs::read(&source)?, contents, "{typed}");
        assert_eq!(names(&root.path().join("kept"))?, ["other"], "{typed}");
        assert_eq!(fs::read(root.path().join("file"))?, b"keep", "{typed}");
    }
    Ok(())
}

#[test]
fn an_existing_empty_file_is_never_replaced_by_a_folder() -> Result<(), Box<dyn Error>> {
    let (root, source) = fixture(false, b"")?;
    let before = names(root.path())?;

    let event = place(&source, false, "made/docs/", false);

    let refused = crate::i18n::tr("An existing file can’t become a folder");
    assert!(
        matches!(&event, OperationEvent::Failed { message, .. } if *message == refused),
        "{event:?}"
    );
    assert_eq!(names(root.path())?, before);
    assert!(source.is_file());
    Ok(())
}
