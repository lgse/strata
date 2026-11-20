// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use super::*;
use crate::services::{SearchEvent, SearchLimits, index_filter};

fn await_result_paths(events: &Receiver<SearchEvent>, expected: &[PathBuf]) -> Result<(), String> {
    let expected = expected.iter().cloned().collect::<BTreeSet<_>>();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = None;
    while Instant::now() < deadline {
        if let Ok(SearchEvent::Results {
            items, indexing, ..
        }) = events.recv_timeout(Duration::from_millis(100))
        {
            let paths = items
                .into_iter()
                .map(|item| item.path)
                .collect::<BTreeSet<_>>();
            if !indexing && paths == expected {
                return Ok(());
            }
            last = Some(paths);
        }
    }
    Err(format!("expected {expected:?}, last results {last:?}"))
}

#[derive(Debug, Clone, Copy)]
enum ExternalChange {
    Upsert,
    Remove,
    Rescan,
    /// A rescan deferred during an in-app operation reloads the column in place.
    RescanAfterOperation,
}

/// A watched folder that holds `needle.txt` only when the change removes it.
struct FilteredFolder {
    browser: Rc<Browser>,
    events: Events,
    notify: Rc<RefCell<Option<WatchCallback>>>,
    dir: PathBuf,
    needle: PathBuf,
    _fixture: tempfile::TempDir,
}

impl FilteredFolder {
    fn open(change: ExternalChange) -> Self {
        let fixture = tempfile::tempdir().expect("fixture");
        let dir = fixture.path().to_path_buf();
        let needle = dir.join("needle.txt");
        if change.removes_the_needle() {
            std::fs::write(&needle, "body").expect("needle");
        }
        let notify = Rc::new(RefCell::new(None::<WatchCallback>));
        let browser = Browser::new(Rc::new(WatchingFileSource {
            notify: notify.clone(),
        }));
        let events = recorded_events(&browser);
        browser.navigate(Location::local(&dir));
        Self {
            browser,
            events,
            notify,
            dir,
            needle,
            _fixture: fixture,
        }
    }

    fn needle_unless(&self, absent: bool) -> Vec<PathBuf> {
        match absent {
            true => Vec::new(),
            false => vec![self.needle.clone()],
        }
    }

    fn write_needle(&self) {
        std::fs::write(&self.needle, "body").expect("needle");
    }

    fn callback(&self) -> WatchCallback {
        self.notify
            .borrow()
            .clone()
            .expect("the directory watcher should be installed")
    }
}

impl ExternalChange {
    fn removes_the_needle(self) -> bool {
        matches!(self, Self::Remove)
    }

    /// Makes the change and returns the filter results it should lead to.
    fn apply(self, folder: &FilteredFolder, callback: &WatchCallback) -> Vec<PathBuf> {
        match self {
            Self::Upsert => {
                folder.write_needle();
                callback(DirectoryChange::Upsert(fixture_entry(
                    folder.needle.to_str().expect("utf-8 fixture"),
                )));
            }
            Self::Remove => {
                std::fs::remove_file(&folder.needle).expect("remove needle");
                callback(DirectoryChange::Remove(Location::local(&folder.needle)));
            }
            Self::Rescan => {
                folder.write_needle();
                callback(DirectoryChange::Rescan);
                assert!(
                    any_event(&folder.events, |event| matches!(
                        event,
                        BrowserEvent::ColumnReloaded { depth: 0 }
                    )),
                    "a Rescan reloads the column"
                );
            }
            Self::RescanAfterOperation => {
                folder.write_needle();
                folder.browser.refresh_operation_columns(&[0]);
            }
        }
        folder.needle_unless(self.removes_the_needle())
    }
}

fn filter_follows(change: ExternalChange) -> Option<String> {
    let folder = FilteredFolder::open(change);
    let (handle, results) = index_filter(folder.dir.clone(), false, false, SearchLimits::default());
    handle.query("needle");
    await_result_paths(
        &results,
        &folder.needle_unless(!change.removes_the_needle()),
    )
    .expect("the initial filter results");
    folder.events.borrow_mut().clear();
    let callback = folder.callback();

    let expected = change.apply(&folder, &callback);
    let failure = await_result_paths(&results, &expected)
        .err()
        .map(|error| format!("{change:?}: {error}"));
    drop(handle);
    failure
}

#[test]
fn watched_directory_changes_refresh_the_filter_index_for_that_directory() {
    let changes = [
        ExternalChange::Upsert,
        ExternalChange::Remove,
        ExternalChange::Rescan,
        ExternalChange::RescanAfterOperation,
    ];
    assert_rows(changes, filter_follows);
}

#[derive(Debug, Clone, Copy)]
enum RenameReport {
    ParentMonitor,
    /// The renamed folder is the open folder, whose own monitor reports only its removal.
    OwnMonitor,
}

impl RenameReport {
    fn report(self, source: &DepartingSource, dir: &Path, (old, new): (&Location, &Path)) {
        match self {
            Self::ParentMonitor => {
                let mut entry = fixture_entry(new.to_str().expect("utf-8 fixture"));
                entry.kind = EntryKind::Directory;
                source.directory_monitor(&Location::local(dir))(DirectoryChange::Move {
                    from: old.clone(),
                    entry,
                });
            }
            Self::OwnMonitor => source.directory_monitor(old)(DirectoryChange::Remove(old.clone())),
        }
    }
}

fn index_follows_the_rename(report: RenameReport) -> Option<String> {
    let fixture = tempfile::tempdir().expect("fixture");
    let dir = fixture.path().to_path_buf();
    let old = dir.join("old");
    let new = dir.join("new");
    std::fs::create_dir(&old).expect("old folder");
    std::fs::write(old.join("needle.txt"), "body").expect("needle");
    let (old_location, new_location) = (Location::local(&old), Location::local(&new));
    let source = Rc::new(DepartingSource::default());
    source.identities.identify(&old_location);
    let browser = Browser::new(source.clone());
    let open = match report {
        RenameReport::ParentMonitor => &dir,
        RenameReport::OwnMonitor => &old,
    };
    browser.navigate(Location::local(open));
    let (handle, results) = index_filter(old.clone(), false, true, SearchLimits::default());
    handle.query("needle");
    await_result_paths(&results, &[old.join("needle.txt")]).expect("the initial filter results");

    std::fs::rename(&old, &new).expect("rename folder");
    source.rename(&old_location, &new_location);
    report.report(&source, &dir, (&old_location, &new));

    let failure = await_result_paths(&results, &[new.join("needle.txt")])
        .err()
        .map(|error| format!("{report:?}: {error}"));
    drop(handle);
    failure
}

#[test]
fn external_renames_rebase_filter_indexes_rooted_at_the_renamed_folder() {
    assert_rows(
        [RenameReport::ParentMonitor, RenameReport::OwnMonitor],
        index_follows_the_rename,
    );
}
