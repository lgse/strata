// SPDX-License-Identifier: MIT

//! Restores cancelled before or while they place an item.

use super::*;

#[test]
fn cancelling_restore_before_io_reports_every_item_as_unattempted() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let location = Location::local("/fixture/trashed.txt");
    let trashed = Path::new("/fixture/trashed.txt");
    for (source, expected) in [
        (
            RestoreSource::TrashEntries(vec![trashed_file(trashed, trashed)]),
            vec![location.clone()],
        ),
        (
            RestoreSource::OriginalLocations {
                locations: vec![location.clone()],
                originals: HashMap::new(),
            },
            vec![location],
        ),
        (
            RestoreSource::OriginalLocations {
                locations: Vec::new(),
                originals: HashMap::new(),
            },
            Vec::new(),
        ),
    ] {
        let (events, emit) = recording();
        let operation = LocalOperationProvider.restore(
            RestoreRequest {
                id: OperationRequestId(9),
                source,
            },
            emit,
        );

        drop(operation);
        while events.borrow().is_empty() {
            glib::MainContext::default().iteration(true);
        }

        let events = events.take();
        let result = cancelled_result(&events, "before io");
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(outcome(result), (Vec::new(), Vec::new(), expected));
    }
    Ok(())
}

/// A restore whose worker is held at a racer until the main thread cancels it.
struct CancelledRestore {
    cancel: CancelFromMain,
    operation: Option<LoadHandle>,
    events: Rc<RefCell<Vec<OperationEvent>>>,
}

impl CancelledRestore {
    fn start(cancel: CancelFromMain, items: Vec<RestoreTrashItem>) -> Self {
        let (operation, events) = start_restore(1615, items);
        Self {
            cancel,
            operation: Some(operation),
            events,
        }
    }

    /// Pumps the main context until an event ends the restore, serving the cancel
    /// once `ready` holds for the events so far, and returns every event.
    fn drive(
        mut self,
        ready: impl Fn(&[OperationEvent]) -> bool,
        ended: impl Fn(&OperationEvent) -> bool,
        row: &str,
    ) -> Vec<OperationEvent> {
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.events.borrow().iter().any(&ended) {
            assert!(
                Instant::now() < deadline,
                "{row}: {:?}",
                self.events.borrow()
            );
            if ready(&self.events.borrow()) {
                self.cancel.serve(|| drop(self.operation.take()));
            }
            context.iteration(false);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(self.cancel.served(), "{row}");
        self.events.take()
    }
}

#[test]
fn a_restore_cut_off_before_it_places_the_item_counts_it_as_not_attempted()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let fixture = tempfile::tempdir()?;
    let trash = VolumeTrash::create(fixture.path())?;
    fs::create_dir(fixture.path().join("Documents"))?;
    let source = trash.file("report");
    fs::write(&source, b"original")?;
    trash.add_info("report", "Documents/report")?;
    let destination = fixture.path().join("Documents/report");
    let cancel = CancelFromMain::default();
    let _scope = FsQuirks::install(
        fixture.path(),
        Quirks {
            before: vec![(Op::Stat, "report", 0, cancel.racer())],
            ..Quirks::default()
        },
    );
    let restore = CancelledRestore::start(cancel, vec![trashed_file(&source, &destination)]);

    let events = restore.drive(
        |_| true,
        |event| matches!(event, OperationEvent::Cancelled { .. }),
        "cut off before placing",
    );

    let result = cancelled_result(&events, "cut off before placing");
    assert_eq!(
        outcome(result),
        (Vec::new(), Vec::new(), vec![Location::local(&source)])
    );
    expect_contents(&[(source, b"original")]);
    expect_no_entry(&[&destination]);
    Ok(())
}

/// Items restored in order, where `cut` is being copied when the cancel lands.
struct CutCopy {
    names: &'static [&'static str],
    /// A stuck cut-off copy cannot be removed.
    stuck: bool,
}

impl CutCopy {
    fn row(&self) -> String {
        format!("{:?}, {}", self.names, self.stuck)
    }

    fn contents(name: &str) -> Vec<u8> {
        match name {
            "cut" => vec![0x5a; 4 * 1024 * 1024],
            _ => name.as_bytes().to_vec(),
        }
    }

    fn fill(&self, trash: &VolumeTrash, documents: &Path) -> io::Result<()> {
        if self.names.contains(&"taken") {
            fs::write(documents.join("taken"), b"occupant")?;
        }
        for name in self.names {
            fs::write(trash.file(name), Self::contents(name))?;
            trash.add_info(name, &format!("Documents/{name}"))?;
        }
        Ok(())
    }

    fn quirks(&self, cancel: &CancelFromMain) -> Quirks {
        Quirks {
            before: vec![
                (Op::CopyRange, "cut", 0, cancel.racer()),
                (Op::Write, "cut", 0, cancel.racer()),
                (Op::Stat, "last", 0, cancel.waiter()),
            ],
            fail: if self.stuck {
                vec![(Op::Unlink, "cut", 0, Errno::IO)]
            } else {
                Vec::new()
            },
            reflink: Support::Rejected(Errno::OPNOTSUPP),
            ..Quirks::without_placement()
        }
    }

    /// The items the cancel completed, failed and did not attempt: "taken" fails as its
    /// name is in use, and a stuck copy fails every item after the first.
    fn expected_outcome(
        &self,
        trash: &VolumeTrash,
        documents: &Path,
    ) -> (Vec<Location>, Vec<Location>, Vec<Location>) {
        let in_trash = |name: &str| Location::local(trash.file(name));
        let (mut completed, mut failed) = (Vec::new(), Vec::new());
        if self.names[0] == "taken" {
            failed.push(in_trash("taken"));
        } else {
            completed.push(Location::local(documents.join("first")));
        }
        let mut not_attempted = self.names[1..].iter().map(|name| in_trash(name)).collect();
        if self.stuck {
            failed.append(&mut not_attempted);
        }
        (completed, failed, not_attempted)
    }

    fn expect_first_item_outcome(&self, trash: &VolumeTrash, documents: &Path) {
        if self.names[0] == "taken" {
            expect_contents(&[(documents.join("taken"), b"occupant")]);
        } else {
            expect_contents(&[(documents.join("first"), b"first")]);
            expect_gone(&[&trash.info("first")]);
        }
    }

    /// Every later item stays in the Trash, and only a stuck cut copy is left behind.
    fn expect_later_items_kept(&self, trash: &VolumeTrash, documents: &Path) {
        for name in &self.names[1..] {
            expect_contents(&[(trash.file(name), &Self::contents(name))]);
            assert!(trash.info(name).exists());
            let left = fs::symlink_metadata(documents.join(name)).is_ok();
            assert_eq!(left, self.stuck && *name == "cut", "{}: {name}", self.row());
        }
    }
}

#[test]
fn a_restore_cancelled_while_it_copies_an_item_ends_as_a_cancel() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let rows = [
        (&["first", "cut"][..], false),
        (&["first", "cut", "last"], false),
        (&["taken", "cut"], false),
        (&["first", "cut"], true),
    ]
    .map(|(names, stuck)| CutCopy { names, stuck });
    for cut in rows {
        let row = cut.row();
        let fixture = tempfile::tempdir()?;
        let trash = VolumeTrash::create(fixture.path())?;
        let documents = fixture.path().join("Documents");
        fs::create_dir(&documents)?;
        cut.fill(&trash, &documents)?;
        let cancel = CancelFromMain::default();
        let _scope = FsQuirks::install(fixture.path(), cut.quirks(&cancel));
        let items = cut
            .names
            .iter()
            .map(|name| trashed_file(&trash.file(name), &documents.join(name)))
            .collect();
        let restore = CancelledRestore::start(cancel, items);

        let events = restore.drive(
            |events| {
                events.iter().any(|event| {
                    matches!(event, OperationEvent::RestoreProgress { completed: 1, .. })
                })
            },
            |event| {
                matches!(
                    event,
                    OperationEvent::Cancelled { .. }
                        | OperationEvent::Restored { .. }
                        | OperationEvent::RestoreCompletedWithErrors { .. }
                )
            },
            &row,
        );

        let result = cancelled_result(&events, &row);
        assert_eq!(
            outcome(result),
            cut.expected_outcome(&trash, &documents),
            "{row}"
        );
        cut.expect_first_item_outcome(&trash, &documents);
        cut.expect_later_items_kept(&trash, &documents);
    }
    Ok(())
}
