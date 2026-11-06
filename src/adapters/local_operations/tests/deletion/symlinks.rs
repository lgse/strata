// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn permanent_delete_removes_a_symlink_standing_in_for_a_directory_without_following_it()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let outside = std::env::temp_dir().join(format!("strata-delete-symlink-outside-{unique}"));
    let decoy = std::env::temp_dir().join(format!("strata-delete-symlink-decoy-{unique}"));
    fs::create_dir_all(&outside)?;
    let sentinel = outside.join("sentinel.txt");
    fs::write(&sentinel, b"do not delete me")?;
    // `directory_entry` reports `kind: Directory` even though the entry is
    // actually a symlink on disk, standing in for a `FileEntry` whose type
    // went stale because the real directory was swapped for a symlink.
    std::os::unix::fs::symlink(&outside, &decoy)?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(20),
            entries: vec![directory_entry(&decoy)],
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let context = glib::MainContext::default();
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Deleted { .. }))
    {
        context.iteration(true);
    }

    assert!(
        sentinel.exists(),
        "deleting the symlink must never touch what it points to"
    );
    assert_eq!(fs::read_dir(&outside)?.count(), 1);
    assert!(!decoy.exists() && !decoy.is_symlink());

    fs::remove_dir_all(outside)?;
    Ok(())
}

#[test]
fn permanent_delete_does_not_follow_a_symlink_nested_inside_the_tree() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-delete-nested-symlink-test-{unique}"));
    let outside = std::env::temp_dir().join(format!("strata-delete-nested-outside-{unique}"));
    let nested = root.join("nested");
    fs::create_dir_all(&nested)?;
    fs::create_dir_all(&outside)?;
    let sentinel = outside.join("sentinel.txt");
    fs::write(&sentinel, b"do not delete me")?;
    fs::write(nested.join("visible.txt"), b"contents")?;
    std::os::unix::fs::symlink(&outside, nested.join("decoy"))?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(21),
            entries: vec![directory_entry(&root)],
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let context = glib::MainContext::default();
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::Deleted { .. }))
    {
        context.iteration(true);
    }

    assert!(
        sentinel.exists(),
        "a symlink nested inside the deleted tree must never lead outside it"
    );
    assert_eq!(fs::read_dir(&outside)?.count(), 1);
    assert!(!root.exists());

    fs::remove_dir_all(outside)?;
    Ok(())
}

#[test]
fn permanent_delete_accepts_a_symlink_in_the_parent_path() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let actual_parent = root.path().join("actual");
    let linked_parent = root.path().join("linked");
    let target = actual_parent.join("target.txt");
    fs::create_dir(&actual_parent)?;
    fs::write(&target, b"keep")?;
    std::os::unix::fs::symlink(&actual_parent, &linked_parent)?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.delete(
        DeleteRequest {
            id: OperationRequestId(22),
            entries: vec![file_entry(&linked_parent.join("target.txt"))],
            permanent: true,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let context = glib::MainContext::default();
    while !events.borrow().iter().any(|event| {
        matches!(
            event,
            OperationEvent::Deleted { .. }
                | OperationEvent::CompletedWithErrors { .. }
                | OperationEvent::Failed { .. }
        )
    }) {
        context.iteration(true);
    }

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, OperationEvent::Deleted { .. })),
        "{:?}",
        events.borrow()
    );
    assert!(!target.exists());
    assert!(linked_parent.is_symlink());
    assert!(actual_parent.is_dir());
    Ok(())
}

#[test]
fn permanent_delete_keeps_the_open_parent_when_its_alias_is_retargeted()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let outside = root.path().join("outside");
    let alias = root.path().join("alias");
    fs::create_dir_all(actual.join("tree/nested"))?;
    fs::create_dir_all(outside.join("tree"))?;
    fs::write(actual.join("tree/nested/file.txt"), b"delete")?;
    fs::write(outside.join("tree/sentinel.txt"), b"keep")?;
    std::os::unix::fs::symlink(&outside, actual.join("tree/decoy"))?;
    std::os::unix::fs::symlink(&actual, &alias)?;
    let parent = super::super::open_local_parent_directory(&alias)?;

    std::os::unix::fs::symlink(&outside, root.path().join("replacement"))?;
    fs::rename(root.path().join("replacement"), &alias)?;
    glib::MainContext::default().block_on(super::super::permanently_delete_local(
        parent,
        OsString::from("tree"),
        None,
        gio::Cancellable::new(),
    ))?;

    assert!(!actual.join("tree").exists());
    assert_eq!(fs::read(outside.join("tree/sentinel.txt"))?, b"keep");
    assert_eq!(fs::read_link(alias)?, outside);
    Ok(())
}

#[test]
fn permanent_delete_revalidates_identity_after_a_parent_alias_changes() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let outside = root.path().join("outside");
    let alias = root.path().join("alias");
    fs::create_dir(&actual)?;
    fs::create_dir(&outside)?;
    fs::write(actual.join("file.txt"), b"original")?;
    fs::write(outside.join("file.txt"), b"keep")?;
    std::os::unix::fs::symlink(&actual, &alias)?;
    let file = gio::File::for_path(alias.join("file.txt"));
    let context = glib::MainContext::default();
    let expected = context.block_on(super::super::local_file_identity(&file))?;

    std::os::unix::fs::symlink(&outside, root.path().join("replacement"))?;
    fs::rename(root.path().join("replacement"), &alias)?;
    let error = context
        .block_on(super::super::permanently_delete_local_path_if_unchanged(
            alias.join("file.txt"),
            expected,
            gio::Cancellable::new(),
        ))
        .expect_err("retargeting the alias must not delete a different entry");

    assert!(error.to_string().contains("changed"), "{error}");
    assert_eq!(fs::read(actual.join("file.txt"))?, b"original");
    assert_eq!(fs::read(outside.join("file.txt"))?, b"keep");
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Since {
    Evicted,
    Replaced,
    ReplacedAlike,
    ReplacedOnceNumbersWent,
}

/// Changes what stands at `path` since its identity was recorded.
fn change_since(since: Since, path: &Path, theirs: &Path, row: &str) -> Result<(), Box<dyn Error>> {
    match since {
        Since::Evicted => assert!(super::super::fs_quirks::evict(path), "{row}"),
        Since::Replaced => {
            let probe = path.with_file_name("probe");
            wait_for_a_later_birth_time(&probe, &fs::metadata(path)?);
            fs::remove_file(path)?;
            fs::write(path, b"theirs")?;
            super::super::fs_quirks::recreated(path);
        }
        Since::ReplacedAlike => fs::rename(theirs, path)?,
        Since::ReplacedOnceNumbersWent => {
            assert!(
                super::super::fs_quirks::autodisable_serverino(path),
                "{row}"
            );
            fs::rename(theirs, path)?;
        }
    }
    Ok(())
}

/// Removes `item` beneath `root` only if it is still `expected`, through
/// the permanent delete or the placement's removal.
fn remove_recorded(
    root: &Path,
    expected: super::super::super::native_fs::Recorded,
    permanently: bool,
    row: &str,
) -> Result<bool, Box<dyn Error>> {
    let path = root.join("item");
    if permanently {
        let result = glib::MainContext::default().block_on(
            super::super::permanently_delete_local_path_if_unchanged(
                path,
                Some(expected),
                gio::Cancellable::new(),
            ),
        );
        if let Err(error) = &result {
            assert!(error.to_string().contains("changed"), "{row}: {error}");
        }
        return Ok(result.is_ok());
    }
    let parent = super::super::open_local_parent_directory(root)?;
    let item = std::ffi::OsStr::new("item");
    let removed = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                super::super::super::placement::remove_if_unchanged(
                    super::super::super::fs_ops::fs(),
                    super::super::super::fs_ops::Entry::new(&parent, item),
                    expected.identity,
                    expected.attributes,
                    rustix::fs::AtFlags::empty(),
                )
            })
            .join()
    })
    .map_err(|_| "the removal panicked")?;
    Ok(removed)
}

fn check_recorded_removal(
    (preset, since, removed): (Preset, Since, bool),
    permanently: bool,
) -> Result<(), Box<dyn Error>> {
    let row = format!("{:?}, {since:?}, {permanently}", preset().mount_type);
    let root = tempfile::tempdir()?;
    let path = root.path().join("item");
    let theirs = root.path().join("theirs");
    fs::write(&path, b"recorded")?;
    fs::write(&theirs, b"theirs")?;
    let hour_before = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)?
        .as_secs()
        - 3600;
    for file in [&path, &theirs] {
        set_times(file, hour_before, 0)?;
    }
    let _scope = FsQuirks::install(root.path(), preset());
    let expected = glib::MainContext::default()
        .block_on(super::super::local_file_identity(&gio::File::for_path(
            &path,
        )))?
        .ok_or("a local item")?;
    change_since(since, &path, &theirs, &row)?;

    let gone = remove_recorded(root.path(), expected, permanently, &row)?;

    assert_eq!(gone, removed, "{row}");
    assert_eq!(path.exists(), !removed, "{row}");
    Ok(())
}

#[test]
fn a_removal_by_a_recorded_identity_follows_the_mounts_rule() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let samba = || Quirks {
        births: Births::MadeUp,
        ..Quirks::cifs()
    };
    let rows: [(Preset, Since, bool); 5] = [
        (Quirks::vfat, Since::Evicted, true),
        (Quirks::vfat, Since::Replaced, false),
        (Quirks::ext4, Since::Evicted, false),
        (samba, Since::ReplacedAlike, false),
        (samba, Since::ReplacedOnceNumbersWent, false),
    ];
    for row in rows {
        for permanently in [true, false] {
            check_recorded_removal(row, permanently)?;
        }
    }
    Ok(())
}

#[test]
fn permanent_delete_of_a_symlink_through_an_alias_keeps_its_referent() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let alias = root.path().join("alias");
    fs::create_dir(&actual)?;
    fs::write(actual.join("sentinel.txt"), b"keep")?;
    std::os::unix::fs::symlink("actual", &alias)?;
    std::os::unix::fs::symlink(".", actual.join("link"))?;

    glib::MainContext::default().block_on(
        super::super::permanently_delete_local_path_if_unchanged(
            alias.join("link"),
            None,
            gio::Cancellable::new(),
        ),
    )?;

    assert!(!actual.join("link").is_symlink());
    assert_eq!(fs::read(actual.join("sentinel.txt"))?, b"keep");
    assert!(alias.is_symlink());
    Ok(())
}
