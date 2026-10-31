// SPDX-License-Identifier: MIT

use super::*;
use crate::services::file_activity::{self, OperationScope};

struct Fixture {
    browser: Rc<Browser>,
    events: Rc<RefCell<Vec<BrowserEvent>>>,
    source: Rc<ScriptedSource>,
}

impl Fixture {
    fn new(initial: MetadataValue<u64>) -> Self {
        let mut source = ScriptedSource::scripted(vec!["document.txt"], Vec::new());
        source.dirs = vec!["destination", "source", "untouched"];
        source.watched = true;
        let (browser, events, source) = scripted_browser(source);
        browser.navigate(Location::local("/fixture"));
        for entry in &mut browser.state.borrow_mut().columns[0].entries {
            if entry.is_directory() {
                entry.child_count = initial.clone();
            }
        }
        events.borrow_mut().clear();
        Self {
            browser,
            events,
            source,
        }
    }

    fn position(&self, name: &str) -> usize {
        self.browser.state.borrow().columns[0]
            .entries
            .iter()
            .position(|entry| entry.display_name == name)
            .expect("folder position")
    }

    fn count(&self, name: &str) -> MetadataValue<u64> {
        self.browser
            .entry_at(0, self.position(name))
            .expect("folder")
            .child_count
    }

    fn request(&self, name: &str) {
        self.browser.request_visible_metadata_fill(
            0,
            self.position(name),
            self.source.entry_location(name),
            true,
        );
    }

    fn dispatch_metadata(&self) {
        pump_until(|| self.browser.metadata_idle.borrow().is_none());
    }

    fn answer(&self, call: usize, rows: &[(&str, u64)], finished: bool) {
        let (id, emit) = {
            let calls = self.source.fill_calls.borrow();
            (calls[call].id, calls[call].emit.clone())
        };
        emit(DirectoryEvent::MetadataFilled {
            request_id: id,
            updates: rows
                .iter()
                .map(|(name, count)| MetadataUpdate {
                    location: self.source.entry_location(name),
                    child_count: MetadataValue::Known(*count),
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                    mode: MetadataValue::Unknown,
                    image_dimensions: MetadataValue::Unknown,
                    duration_seconds: MetadataValue::Unknown,
                    listable: MetadataValue::Unknown,
                })
                .collect(),
        });
        if finished {
            emit(DirectoryEvent::MetadataFinished {
                request_id: id,
                outcome: MetadataOutcome::Complete,
            });
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.browser.clear_observer();
        self.browser.cancel_deferred_work();
    }
}

#[test]
fn ended_operations_invalidate_changed_folder_counts_in_every_browser_without_reloading() {
    let _guard = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("async lock");
    let first = Fixture::new(MetadataValue::Known(26));
    let second = Fixture::new(MetadataValue::Unknown);
    for fixture in [&first, &second] {
        fixture.browser.select(0, fixture.position("destination"));
        fixture.events.borrow_mut().clear();
    }
    let activity = file_activity::begin(OperationScope {
        roots: vec!["/fixture/source/document.txt".into()],
        folders: vec![
            "/fixture/source".into(),
            "/fixture/destination".into(),
            "/fixture/destination".into(),
            "/elsewhere".into(),
        ],
    });
    drop(activity);

    for fixture in [&first, &second] {
        assert_eq!(fixture.count("destination"), MetadataValue::Unknown);
        assert_eq!(fixture.count("source"), MetadataValue::Unknown);
        assert_eq!(
            fixture.browser.selected_positions(0),
            [fixture.position("destination")]
        );
        assert!(!fixture.events.borrow().iter().any(|event| matches!(
            event,
            BrowserEvent::ColumnReloaded { .. }
                | BrowserEvent::EntriesReplaced { .. }
                | BrowserEvent::FocusChanged { .. }
        )));
        assert!(fixture.source.fill_calls.borrow().is_empty());
        for _ in 0..3 {
            fixture.request("destination");
            fixture.request("source");
        }
        fixture.dispatch_metadata();
        let calls = fixture.source.fill_calls.borrow();
        assert_eq!(calls.len(), 1);
        let mut entries = calls[0].entries.clone();
        entries.sort_by_key(Location::display_path);
        assert_eq!(
            entries,
            [
                Location::local("/fixture/destination"),
                Location::local("/fixture/source")
            ]
        );
        assert!(calls[0].include_icon_details);
        assert!(!calls[0].full);
        drop(calls);
        fixture.answer(0, &[("destination", 27), ("source", 25)], true);
        assert_eq!(fixture.count("destination"), MetadataValue::Known(27));
        assert_eq!(fixture.count("source"), MetadataValue::Known(25));
    }
    assert_eq!(first.count("untouched"), MetadataValue::Known(26));
    assert_eq!(second.count("untouched"), MetadataValue::Unknown);
}

#[test]
fn a_count_refresh_rejects_stale_results_without_cancelling_unrelated_metadata() {
    let _guard = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("async lock");
    for unrelated_work in [false, true] {
        let fixture = Fixture::new(MetadataValue::Known(26));
        fixture.request("destination");
        if unrelated_work {
            fixture.request("untouched");
        }
        fixture.dispatch_metadata();
        drop(file_activity::begin(OperationScope {
            roots: vec!["/fixture/destination".into()],
            folders: vec!["/fixture".into()],
        }));
        fixture.answer(0, &[("destination", 26), ("untouched", 28)], false);
        assert_eq!(fixture.count("destination"), MetadataValue::Unknown);
        assert_eq!(
            fixture.count("untouched"),
            MetadataValue::Known(if unrelated_work { 28 } else { 26 })
        );
        fixture.request("destination");
        fixture.dispatch_metadata();
        assert_eq!(
            fixture.source.fill_calls.borrow().len(),
            if unrelated_work { 1 } else { 2 },
            "only an emptied batch is cancelled"
        );
        fixture.answer(0, &[], true);
        fixture.dispatch_metadata();
        assert_eq!(fixture.source.fill_calls.borrow().len(), 2);
        assert_eq!(
            fixture.source.fill_calls.borrow()[1].entries,
            [Location::local("/fixture/destination")]
        );
        fixture.answer(1, &[("destination", 27)], true);
        assert_eq!(fixture.count("destination"), MetadataValue::Known(27));
    }
}

#[test]
fn only_recursive_changes_invalidate_counts_inside_the_changed_folder() {
    let _guard = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("async lock");
    let fixture = Fixture::new(MetadataValue::Known(26));
    let destination = Location::local("/fixture/destination");
    fixture.browser.show_child(0, destination.clone());
    for entry in &mut fixture.browser.state.borrow_mut().columns[1].entries {
        entry.location = destination.child(&entry.native_name).expect("nested entry");
        entry.child_count = MetadataValue::Known(26);
    }
    drop(file_activity::begin(OperationScope {
        roots: Vec::new(),
        folders: vec!["/fixture/destination".into()],
    }));
    let nested_counts = || {
        fixture.browser.state.borrow().columns[1]
            .entries
            .iter()
            .filter(|entry| entry.is_directory())
            .map(|entry| entry.child_count.clone())
            .collect::<Vec<_>>()
    };
    assert!(
        nested_counts()
            .iter()
            .all(|count| *count == MetadataValue::Known(26))
    );
    let merge = file_activity::begin(OperationScope::default());
    merge.include_trees([destination]);
    drop(merge);
    assert!(
        nested_counts()
            .iter()
            .all(|count| *count == MetadataValue::Unknown)
    );
}

#[test]
fn a_listing_that_finishes_after_an_operation_cannot_publish_its_old_counts() {
    let _guard = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("async lock");
    for already_batched in [false, true] {
        let (browser, _, source) = scripted_browser(ScriptedSource::manual(Vec::new(), Vec::new()));
        browser.navigate(Location::local("/fixture"));
        let (id, emit) = source.enumerate_calls.borrow()[0].clone();
        let mut entry = source.listed_entry("destination", true);
        entry.child_count = MetadataValue::Known(26);
        let batch = DirectoryEvent::Batch {
            request_id: id,
            entries: vec![entry],
        };
        if already_batched {
            emit(batch.clone());
        }
        drop(file_activity::begin(OperationScope {
            roots: Vec::new(),
            folders: vec!["/fixture/destination".into()],
        }));
        if !already_batched {
            emit(batch);
        }
        emit(DirectoryEvent::Finished {
            request_id: id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        pump_until(|| {
            browser
                .column_snapshot(0)
                .is_some_and(|column| !column.loading)
        });
        assert_eq!(
            browser.entry_at(0, 0).expect("loaded folder").child_count,
            MetadataValue::Unknown,
            "already batched: {already_batched}"
        );
    }
}
