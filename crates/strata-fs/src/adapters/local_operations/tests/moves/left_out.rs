// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_move_leaves_special_files_it_cannot_copy_at_the_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for conflict in MOVES {
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let folder = source_dir.join("folder");
        fs::create_dir_all(&folder)?;
        fs::write(folder.join("notes.txt"), b"keep-me")?;
        let _socket = std::os::unix::net::UnixListener::bind(folder.join("socket"))?;
        let _source = FsQuirks::install(&source_dir, Quirks::default());
        let (destination, _destination) = destination_for(root.path(), conflict)?;

        let (_operation, events) = paste_move(91, &destination, &folder, conflict);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{conflict:?}: {:?}",
            events.borrow()
        );
        assert_eq!(
            names(&destination.join("folder"))?,
            ["notes.txt"],
            "{conflict:?}"
        );
        assert_eq!(names(&folder)?, ["socket"], "{conflict:?}");
        assert_eq!(
            reported(&events),
            TransferReport {
                kept: vec![kept(
                    folder.join("socket"),
                    KeptCause::Skipped(SpecialKind::Socket)
                )],
                ..TransferReport::default()
            },
            "{conflict:?}"
        );
    }
    Ok(())
}

fn folder_with_links(folder: &Path) -> io::Result<()> {
    fs::create_dir_all(folder.join("sub"))?;
    fs::write(folder.join("notes.txt"), b"notes")?;
    fs::write(folder.join("sub/b.txt"), b"b")?;
    std::os::unix::fs::symlink("notes.txt", folder.join("link"))?;
    std::os::unix::fs::symlink("../notes.txt", folder.join("sub/inner"))
}

fn links_kept(folder: &Path) -> Vec<KeptItem> {
    ["link", "sub/inner"]
        .map(|link| kept(folder.join(link), KeptCause::Skipped(SpecialKind::Symlink)))
        .to_vec()
}

#[test]
fn a_move_keeps_the_links_it_leaves_out_at_the_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for conflict in MOVES {
        let root = tempfile::tempdir()?;
        let source_dir = root.path().join("source");
        let folder = source_dir.join("folder");
        folder_with_links(&folder)?;
        let _source = FsQuirks::install(&source_dir, Quirks::default());
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        // A merge leaves what it would replace with a link it leaves out.
        let there = match conflict {
            TransferConflict::Merge => Some("link"),
            TransferConflict::ReplaceExisting => Some("old.txt"),
            _ => None,
        };
        if let Some(name) = there {
            fs::create_dir(destination.join("folder"))?;
            fs::write(destination.join("folder").join(name), b"earlier")?;
        }
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                trash: super::super::fs_quirks::Trash::Unsupported,
                ..Quirks::vfat()
            },
        );

        let (_operation, events) = paste_move(93, &destination, &folder, conflict);
        pump_until_transfer(&events);

        assert!(
            matches!(
                terminal_transfer(&events.borrow()),
                Some(OperationEvent::Pasted { .. })
            ),
            "{conflict:?}: {:?}",
            events.borrow()
        );
        let moved = destination.join("folder");
        let mut expected = vec!["notes.txt", "sub"];
        if conflict == TransferConflict::Merge {
            expected.insert(0, "link");
            assert_eq!(fs::read(moved.join("link"))?, b"earlier");
        }
        assert_eq!(names(&moved)?, expected, "{conflict:?}");
        assert_eq!(names(&moved.join("sub"))?, ["b.txt"], "{conflict:?}");
        assert_eq!(names(&folder)?, ["link", "sub"], "{conflict:?}");
        assert_eq!(names(&folder.join("sub"))?, ["inner"], "{conflict:?}");
        let mut kept = reported(&events).kept;
        kept.sort_by(|left, right| left.location.compare(&right.location));
        assert_eq!(kept, links_kept(&folder), "{conflict:?}");
    }
    Ok(())
}

#[test]
fn a_link_moved_within_a_share_that_refuses_new_links_is_moved() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let outside = root.path().join("outside");
    let share = root.path().join("share");
    for folder in [&outside, &share.join("from"), &share.join("to")] {
        fs::create_dir_all(folder)?;
    }
    std::os::unix::fs::symlink("target", outside.join("refused"))?;
    // As the server made it.
    std::os::unix::fs::symlink("target", share.join("from/link"))?;
    let _share = FsQuirks::install(&share, Quirks::cifs());
    let (_operation, events) = paste_move(
        98,
        &share,
        &outside.join("refused"),
        TransferConflict::FailIfExists,
    );
    pump_until_transfer(&events);
    assert_eq!(names(&share)?, ["from", "to"]);

    let (_operation, events) = paste_move(
        99,
        &share.join("to"),
        &share.join("from/link"),
        TransferConflict::FailIfExists,
    );
    pump_until_transfer(&events);

    assert!(
        matches!(
            terminal_transfer(&events.borrow()),
            Some(OperationEvent::Pasted { .. })
        ),
        "{:?}",
        events.borrow()
    );
    assert_eq!(fs::read_link(share.join("to/link"))?, Path::new("target"));
    assert!(names(&share.join("from"))?.is_empty());
    Ok(())
}

#[test]
fn a_move_through_gio_keeps_the_links_it_leaves_out_at_the_source() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    folder_with_links(&folder)?;
    let stand_in = root.path().join("remote");
    fs::create_dir(&stand_in)?;
    let _stand_in = FsQuirks::install(&stand_in, Quirks::vfat());
    let target = gio::File::for_uri("strata-test-remote:///folder");
    let copied_to = stand_in.join("folder");
    let remote_copy: super::super::super::RemoteCopy =
        Rc::new(move |source, target, _, cancellable, left| {
            let copied_to = copied_to.clone();
            Box::pin(async move {
                super::super::super::copy_recursively_with_progress(
                    source,
                    gio::File::for_path(copied_to),
                    super::super::super::CopyInto::New,
                    cancellable,
                    super::super::super::CopyRecord::Nothing,
                    None,
                    super::super::super::GioNames::new(fixing(), left),
                )
                .await?;
                Ok(target)
            })
        });
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let progress = TransferProgressTracker::new(
        OperationRequestId(1),
        None,
        None,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    glib::MainContext::default().block_on(super::super::super::move_local_with_progress(
        gio::File::for_path(&folder),
        target,
        Taken::Fail,
        gio::Cancellable::new(),
        super::super::super::Copying {
            progress: Some(progress),
            names: fixing(),
            ..Default::default()
        },
        super::super::super::attempt_move(None),
        remote_copy,
    ))?;

    // GIO alone would have made the links, as the stand-in's own filesystem holds them.
    assert_eq!(names(&stand_in.join("folder"))?, ["notes.txt", "sub"]);
    assert_eq!(names(&stand_in.join("folder/sub"))?, ["b.txt"]);
    assert_eq!(names(&folder)?, ["link", "sub"]);
    assert_eq!(names(&folder.join("sub"))?, ["inner"]);
    let mut kept = reported(&events).kept;
    kept.sort_by(|left, right| left.location.compare(&right.location));
    assert_eq!(kept, links_kept(&folder));
    Ok(())
}

#[test]
fn deleting_a_source_gio_moved_keeps_the_links_it_left_out() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    folder_with_links(&folder)?;
    fs::create_dir(folder.join("other"))?;
    fs::write(folder.join("other/c.txt"), b"c")?;
    let left = ["link", "sub/inner"].map(|link| gio::File::for_path(folder.join(link)));
    let mut owed = super::super::super::owed::Owed::default();
    owed.owe(super::super::super::owed::Deletion::Source(
        gio::File::for_path(&folder),
        left.to_vec(),
    ));
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let progress = TransferProgressTracker::new(
        OperationRequestId(1),
        None,
        None,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );

    glib::MainContext::default()
        .block_on(owed.settle(Some(&progress), &gio::Cancellable::new()))?;

    assert_eq!(names(&folder)?, ["link", "sub"]);
    assert_eq!(names(&folder.join("sub"))?, ["inner"]);
    let mut kept = reported(&events).kept;
    kept.sort_by(|left, right| left.location.compare(&right.location));
    assert_eq!(kept, links_kept(&folder));
    Ok(())
}
