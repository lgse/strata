// SPDX-License-Identifier: MIT

use super::*;

fn merging(id: u64, destination: &Path, source: PathBuf, move_sources: bool) -> PasteRequest {
    PasteRequest {
        id: OperationRequestId(id),
        destination: Location::local(destination),
        items: vec![PasteItem {
            source: Location::local(source),
            conflict: TransferConflict::Merge,
            target_name: None,
        }],
        move_sources,
    }
}

/// Pastes `request`, dropping the operation from the main loop once
/// `cancel` is asked to, until the paste ends.
fn paste_cancelled_from_main(
    request: PasteRequest,
    cancel: &CancelFromMain,
) -> Vec<OperationEvent> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let mut operation = Some(LocalOperationProvider.paste(
        request,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    ));
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !events.borrow().iter().any(transfer_settled) {
        assert!(Instant::now() < deadline, "the merge never ended");
        cancel.serve(|| drop(operation.take()));
        context.iteration(false);
        thread::sleep(Duration::from_millis(1));
    }
    events.take()
}

#[test]
fn a_merge_a_cancel_cuts_off_counts_as_failed_as_it_keeps_what_it_placed()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    fs::create_dir_all(sources.join("run"))?;
    fs::create_dir_all(destination.join("run"))?;
    fs::write(sources.join("run/big.bin"), vec![0x5a; 4 * 1024 * 1024])?;
    fs::write(destination.join("run/old.txt"), b"old")?;
    let cancel = CancelFromMain::default();
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            before: vec![
                (Op::CopyRange, "*", 0, cancel.racer()),
                (Op::Write, "*", 0, cancel.racer()),
            ],
            reflink: Support::Rejected(Errno::OPNOTSUPP),
            ..Quirks::default()
        },
    );

    let request = merging(1615, &destination, sources.join("run"), false);
    let events = paste_cancelled_from_main(request, &cancel);

    assert!(cancel.served());
    assert_eq!(
        cancel_outcome(&events, ""),
        [
            Vec::new(),
            vec![Location::local(sources.join("run"))],
            Vec::new()
        ]
    );
    assert_eq!(fs::read(destination.join("run/old.txt"))?, b"old");
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Copied {
    AsPlaced,
    Rewritten,
    RenamedOver,
}

impl Copied {
    /// Changes the copy at `placed` as this says, then cancels.
    fn racer(self, placed: PathBuf, theirs: PathBuf, mut cancels: Racer) -> Racer {
        Box::new(move || {
            match self {
                Copied::AsPlaced => {}
                Copied::Rewritten => fs::write(&placed, b"theirs, longer").expect("rewrite"),
                Copied::RenamedOver => {
                    fs::write(&theirs, b"theirs, longer").expect("write theirs");
                    fs::rename(&theirs, &placed).expect("rename theirs over the copy");
                }
            }
            cancels();
        })
    }
}

/// The original the cancel put back stands alone beside `b.txt`.
fn assert_put_back(
    events: &[OperationEvent],
    sources: &Path,
    run: &Path,
    row: &str,
) -> io::Result<()> {
    let [completed, failed, _] = cancel_outcome(events, row);
    assert!(completed.is_empty(), "{row}");
    assert_eq!(failed, [Location::local(sources.join("run"))], "{row}");
    assert_eq!(fs::read(run.join("a.txt"))?, b"old a", "{row}");
    assert_eq!(names(run)?, ["a.txt", "b.txt"], "{row}");
    Ok(())
}

/// The merge stopped and named the folder it left its original in, beside
/// the changed copy.
fn assert_original_kept_aside(events: &[OperationEvent], run: &Path, row: &str) -> io::Result<()> {
    let Some(OperationEvent::TransferFailed { message, .. }) = events.last() else {
        panic!("{row}: expected the merge to stop, got {events:?}");
    };
    let left = names(run)?;
    let [folder, a, b] = &left[..] else {
        panic!("{row}: expected the merge folder beside a.txt and b.txt, found {left:?}");
    };
    assert!(folder.starts_with(".strata-merge-"), "{row}: {left:?}");
    assert_eq!([a.as_str(), b.as_str()], ["a.txt", "b.txt"], "{row}");
    assert!(message.contains(folder.as_str()), "{row}: {message}");
    let kept = [
        ("a.txt", "theirs, longer"),
        (&format!("{folder}/a.txt"), "old a"),
    ];
    assert_files(run, &kept, row)
}

fn check_cancel_after_placing(
    move_sources: bool,
    copy: Copied,
    preset: Preset,
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let sources = root.path().join("sources");
    let destination = root.path().join("destination");
    let run = destination.join("run");
    write_files(
        &sources.join("run"),
        &[("a.txt", "new a"), ("b.txt", "new b"), ("c.txt", "new c")],
    )?;
    write_files(&run, &[("a.txt", "old a"), ("b.txt", "old b")])?;
    let cancel = CancelFromMain::default();
    let racer = copy.racer(
        run.join("a.txt"),
        root.path().join("theirs"),
        cancel.racer(),
    );
    let _sources = FsQuirks::install(&sources, Quirks::default());
    let _destination = FsQuirks::install(
        &destination,
        without_trash(Quirks {
            after: vec![(Op::Rename, "a.txt", 1, racer)],
            ..preset()
        }),
    );

    let request = merging(1729, &destination, sources.join("run"), move_sources);
    let events = paste_cancelled_from_main(request, &cancel);

    let row = format!(
        "moving: {move_sources}, {copy:?}, {:?}",
        preset().mount_type
    );
    assert!(cancel.served(), "{row}");
    assert_eq!(fs::read(run.join("b.txt"))?, b"old b", "{row}");
    if let Copied::AsPlaced = copy {
        assert_put_back(&events, &sources, &run, &row)?;
    } else {
        assert_original_kept_aside(&events, &run, &row)?;
    }
    assert_files(
        &sources.join("run"),
        &[("a.txt", "new a"), ("b.txt", "new b"), ("c.txt", "new c")],
        &row,
    )?;
    Ok(())
}

#[test]
fn a_merge_without_trash_cancelled_after_it_placed_a_copy_puts_the_original_back()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let mut rows = [false, true]
        .into_iter()
        .flat_map(|moving| {
            [Copied::AsPlaced, Copied::Rewritten, Copied::RenamedOver]
                .map(|copy| (moving, copy, Quirks::default as Preset))
        })
        .collect::<Vec<_>>();
    rows.push((false, Copied::AsPlaced, Quirks::exfat));
    for (move_sources, copy, preset) in rows {
        check_cancel_after_placing(move_sources, copy, preset)?;
    }
    Ok(())
}
