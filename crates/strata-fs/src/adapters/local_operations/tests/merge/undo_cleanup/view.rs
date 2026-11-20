// SPDX-License-Identifier: MIT

use crate::app::{Browser, BrowserEvent};

use super::*;

/// Lists local folders, watching them only when `watched`, as where no
/// folder monitor is available.
struct Listing {
    watched: bool,
}

impl crate::services::FileSource for Listing {
    fn validate_location(
        &self,
        location: &Location,
    ) -> Result<(), crate::services::LocationValidationError> {
        crate::adapters::LocalFileSource.validate_location(location)
    }

    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), crate::services::LocationValidationError>)>,
    ) -> crate::services::LoadHandle {
        crate::adapters::LocalFileSource.validate_location_async(location, emit)
    }

    fn enumerate(
        &self,
        request: crate::services::DirectoryRequest,
        emit: Rc<dyn Fn(crate::services::DirectoryEvent)>,
    ) -> crate::services::LoadHandle {
        crate::adapters::LocalFileSource.enumerate(request, emit)
    }

    fn start_watch(
        &self,
        request: crate::services::WatchRequest,
        notify: Rc<dyn Fn(crate::services::DirectoryChange)>,
        report: Rc<dyn Fn(crate::services::Watching)>,
    ) -> crate::services::LoadHandle {
        if self.watched {
            crate::adapters::LocalFileSource.start_watch(request, notify, report)
        } else {
            report(crate::services::Watching::Unavailable);
            crate::services::LoadHandle::new(|| {})
        }
    }
}

type Events = Rc<RefCell<Vec<BrowserEvent>>>;

/// Iterates the main loop until `done`, for at most ten seconds.
fn pump_until(what: &str, done: &dyn Fn() -> bool) -> Result<(), String> {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        if Instant::now() > deadline {
            return Err(format!("timed out waiting until {what}"));
        }
        context.iteration(false);
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn pump_until_event(
    events: &Events,
    what: &str,
    matches: impl Fn(&BrowserEvent) -> bool,
) -> Result<(), String> {
    pump_until(what, &|| events.borrow().iter().any(&matches))
}

/// A browser over `Listing` that records every operation event it sees.
fn observed_browser(watched: bool) -> (Rc<Browser>, Events) {
    let browser = Browser::new(Rc::new(Listing { watched }));
    browser.set_operation_provider(Rc::new(LocalOperationProvider));
    let events = Events::default();
    let observed = events.clone();
    browser.observe(move |event| {
        observed
            .borrow_mut()
            .push(crate::test_support::operations::operation_event(event).clone());
    });
    (browser, events)
}

fn listed_names(browser: &Rc<Browser>) -> Vec<String> {
    let mut names = browser
        .with_column_entries(0, |entries| {
            entries
                .iter()
                .map(|entry| entry.display_name.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn transfer(browser: &Rc<Browser>, destination: &Path, source: &Path, conflict: TransferConflict) {
    browser.transfer(
        Location::local(destination),
        vec![PasteItem {
            source: Location::local(source),
            conflict,
            target_name: None,
        }],
        false,
        false,
    );
}

#[test]
fn a_merge_undo_drops_from_view_only_what_it_removed() {
    crate::test_support::isolated_test(
        "adapters::local_operations::tests::merge::undo_cleanup::view::a_merge_undo_drops_from_view_only_what_it_removed",
        || {
            check_a_merge_undo_drops_from_view_only_what_it_removed().expect("merge undo view");
        },
    );
}

/// Whether undo fails an item, the merge ran in the background, and the
/// folder is watched.
#[derive(Clone, Copy)]
struct Viewed {
    failing: bool,
    background: bool,
    watched: bool,
}

impl Viewed {
    fn row(self) -> String {
        let Self {
            failing,
            background,
            watched,
        } = self;
        format!(
            "undo fails an item: {failing}, merged in the background: {background}, watched: {watched}"
        )
    }

    /// The Trash shares the scope, so it is on the folder's mount.
    fn quirks(self) -> Quirks {
        Quirks {
            fail: if self.failing {
                vec![(Op::Unlink, "c.txt", 0, Errno::ACCESS)]
            } else {
                Vec::new()
            },
            ..Quirks::default()
        }
    }
}

fn write_viewed_merge(source: &Path, folder: &Path) -> io::Result<()> {
    write_files(
        source,
        &[
            ("folder/a.txt", "incoming a"),
            ("folder/b.txt", "incoming b"),
            ("folder/c.txt", "incoming c"),
            ("folder/d.txt", "incoming d"),
            ("folder/new.txt", "new"),
        ],
    )?;
    write_files(
        folder,
        &[
            ("a.txt", "original a"),
            ("b.txt", "original b"),
            ("c.txt", "original c"),
            ("d.txt", "original d"),
        ],
    )
}

/// Merges `source/folder` into `folder`, changes two of its items, and
/// browses it until it lists the merge.
fn merge_and_browse(
    (browser, events): (&Rc<Browser>, &Events),
    case: Viewed,
    source: &Path,
    folder: &Path,
) -> Result<(), Box<dyn Error>> {
    let row = case.row();
    let destination = folder.parent().ok_or("the destination")?;
    transfer(
        browser,
        destination,
        &source.join("folder"),
        TransferConflict::Merge,
    );
    if case.background {
        let id = browser.last_started_operation().ok_or("the merge")?;
        assert!(browser.background_file_operation(id), "{row}");
    }
    pump_until_event(events, &format!("{row}: the merge ends"), |event| {
        matches!(event, BrowserEvent::TransferCompleted)
    })?;
    fs::write(folder.join("b.txt"), "edited after the merge")?;
    fs::remove_file(folder.join("d.txt"))?;
    browser.navigate(Location::local(folder));
    pump_until(&format!("{row}: the folder lists the merge"), &|| {
        listed_names(browser) == ["a.txt", "b.txt", "c.txt", "new.txt"]
    })?;
    Ok(())
}

/// Checks one case, returning how the folder's listing fell behind, if it did.
fn check_viewed_undo(case: Viewed, sandbox: &Path) -> Result<Option<String>, Box<dyn Error>> {
    let row = case.row();
    let root = tempfile::tempdir_in(glib::home_dir())?;
    let source = root.path().join("source");
    let folder = root.path().join("destination/folder");
    write_viewed_merge(&source, &folder)?;
    let _scope = FsQuirks::install(sandbox, case.quirks());
    let (browser, events) = observed_browser(case.watched);
    merge_and_browse((&browser, &events), case, &source, &folder)?;
    let (generation, created, overwritten) =
        browser.pending_undo_merge().ok_or("the merge's undo")?;
    events.borrow_mut().clear();

    assert!(
        browser.undo_merge(generation, created, overwritten),
        "{row}"
    );
    pump_until_event(&events, &format!("{row}: the undo ends"), |event| {
        matches!(event, BrowserEvent::RestorationFinished { .. })
    })?;

    let completed_with_errors = events
        .borrow()
        .iter()
        .any(|event| matches!(event, BrowserEvent::OperationCompletedWithErrors { .. }));
    assert_eq!(completed_with_errors, case.failing, "{row}");
    let c = if case.failing {
        "incoming c"
    } else {
        "original c"
    };
    assert_files(
        &folder,
        &[
            ("a.txt", "original a"),
            ("b.txt", "edited after the merge"),
            ("c.txt", c),
            ("d.txt", "original d"),
        ],
        &row,
    )?;
    assert!(!folder.join("new.txt").exists(), "{row}");
    let expected = ["a.txt", "b.txt", "c.txt", "d.txt"];
    // A monitor may still be reporting what the undo changed, and a
    // folder without one reloads.
    let behind = pump_until(&row, &|| listed_names(&browser) == expected)
        .is_err()
        .then(|| format!("{row}: the folder lists {:?}", listed_names(&browser)));
    assert_eq!(
        browser
            .pending_undo_merge()
            .map(|(_, created, overwritten)| (created, overwritten)),
        case.failing
            .then(|| (Vec::new(), vec![Location::local(folder.join("c.txt"))])),
        "{row}: only what failed is left to undo"
    );
    Ok(behind)
}

fn check_a_merge_undo_drops_from_view_only_what_it_removed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let sandbox = glib::user_data_dir()
        .parent()
        .ok_or("the test's data folder")?
        .to_path_buf();
    let mut failures = Vec::new();
    let rows = [false, true].into_iter().flat_map(|failing| {
        [(false, false), (true, false), (false, true), (true, true)].map(|(background, watched)| {
            Viewed {
                failing,
                background,
                watched,
            }
        })
    });
    // Without a monitor, the folder learns only from the undo's outcome.
    for case in rows {
        failures.extend(check_viewed_undo(case, &sandbox)?);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn an_undo_never_brings_back_another_item_trashed_later_from_the_same_path() {
    crate::test_support::isolated_test(
        "adapters::local_operations::tests::merge::undo_cleanup::view::an_undo_never_brings_back_another_item_trashed_later_from_the_same_path",
        || {
            for undone in [Undone::Delete, Undone::Replace, Undone::Merge] {
                check_an_undo_never_brings_back_another_item_trashed_later(undone)
                    .unwrap_or_else(|error| panic!("{undone:?}: {error}"));
            }
        },
    );
}

/// Pastes an incoming `folder/report.txt` over the original with `conflict`.
fn paste_incoming(
    (browser, events): (&Rc<Browser>, &Events),
    root: &Path,
    conflict: TransferConflict,
) -> Result<(), Box<dyn Error>> {
    write_files(&root.join("incoming"), &[("folder/report.txt", "incoming")])?;
    let (source, destination) = match conflict {
        TransferConflict::Merge => (root.join("incoming/folder"), root.to_owned()),
        _ => (root.join("incoming/folder/report.txt"), root.join("folder")),
    };
    transfer(browser, &destination, &source, conflict);
    pump_until_event(events, "the paste ends", |event| {
        matches!(event, BrowserEvent::TransferCompleted)
    })?;
    Ok(())
}

fn recorded_path(info: &Path) -> Option<PathBuf> {
    fs::read_to_string(info).ok().and_then(|contents| {
        contents
            .lines()
            .find_map(|line| line.strip_prefix("Path="))
            .and_then(crate::adapters::trash_restore::decode_trashinfo_path)
    })
}

/// Dates the Trash entry of `target` long ago.
fn backdate_entry(trash: &Path, target: &Path) -> Result<(), Box<dyn Error>> {
    let ours = fs::read_dir(trash.join("info"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|info| recorded_path(info).as_deref() == Some(target))
        .ok_or("the original's Trash entry")?;
    let contents = fs::read_to_string(&ours)?;
    fs::write(
        &ours,
        contents
            .lines()
            .map(|line| match line.strip_prefix("DeletionDate=") {
                Some(_) => "DeletionDate=2000-01-01T00:00:00",
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )?;
    Ok(())
}

/// Puts another item from `target`'s path in Trash, dated later.
fn trash_another_later(trash: &Path, target: &Path) -> io::Result<PathBuf> {
    let later = trash.join("files/report.later.txt");
    fs::write(&later, "trashed later")?;
    fs::write(
        trash.join("info/report.later.txt.trashinfo"),
        format!(
            "[Trash Info]\nPath={}\nDeletionDate=2099-01-01T00:00:00\n",
            target.display()
        ),
    )?;
    Ok(later)
}

fn start_undo(browser: &Rc<Browser>, undone: Undone) -> Result<bool, Box<dyn Error>> {
    Ok(match undone {
        Undone::Delete => browser.undo_last_trash(),
        Undone::Replace | Undone::Merge => {
            let (generation, created, overwritten) =
                browser.pending_undo_merge().ok_or("the paste's undo")?;
            browser.undo_merge(generation, created, overwritten)
        }
    })
}

fn check_an_undo_never_brings_back_another_item_trashed_later(
    undone: Undone,
) -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir_in(glib::home_dir())?;
    let target = root.path().join("folder/report.txt");
    write_files(root.path(), &[("folder/report.txt", "original")])?;
    let (browser, events) = observed_browser(false);
    let observed = (&browser, &events);
    match undone {
        Undone::Delete => {
            browser.delete(vec![file_entry(&target)], false);
            pump_until_event(&events, "the delete ends", |event| {
                matches!(event, BrowserEvent::DeletionFinished { .. })
            })?;
        }
        Undone::Replace => {
            paste_incoming(observed, root.path(), TransferConflict::ReplaceExisting)?
        }
        Undone::Merge => paste_incoming(observed, root.path(), TransferConflict::Merge)?,
    }
    let trash = glib::user_data_dir().join("Trash");
    backdate_entry(&trash, &target)?;
    let later = trash_another_later(&trash, &target)?;
    events.borrow_mut().clear();

    assert!(start_undo(&browser, undone)?, "the undo starts");
    pump_until_event(&events, "the undo ends", |event| {
        matches!(event, BrowserEvent::RestorationFinished { .. })
    })?;

    assert_eq!(fs::read_to_string(&target)?, "original");
    assert_eq!(fs::read_to_string(&later)?, "trashed later");
    fs::remove_file(later)?;
    fs::remove_file(trash.join("info/report.later.txt.trashinfo"))?;
    Ok(())
}
