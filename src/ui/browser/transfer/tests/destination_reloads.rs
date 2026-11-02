// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{CancelledOperation, DirectoryChange, OperationEvent};

fn local_listing_loaded(view: &crate::ui::browser::BrowserView, count: usize) -> bool {
    view.browser()
        .column_snapshot(0)
        .is_some_and(|snapshot| !snapshot.loading && snapshot.count == count)
        && view.browser().column_has_monitor(0)
}

fn selected_locations(view: &crate::ui::browser::BrowserView) -> Vec<Location> {
    view.browser()
        .selected_entries()
        .into_iter()
        .map(|entry| entry.location)
        .collect()
}

#[test]
fn cancelling_an_operation_keeps_the_live_local_listing_visible() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::destination_reloads::cancelling_an_operation_keeps_the_live_local_listing_visible",
        || {
            let fixture = tempfile::tempdir().expect("cancelled transfer fixture");
            std::fs::write(fixture.path().join("copied.txt"), b"copied").expect("copied fixture");
            let root = Location::local(fixture.path());
            let view = local_view();
            view.navigate_location(root.clone());
            wait_until(|| local_listing_loaded(&view, 1), "the local listing");
            let request = view.browser().column_request_id(0).expect("loaded request");

            view.state
                .handle(&crate::app::BrowserEvent::OperationCancelled {
                    completed: 1,
                    failed: 0,
                    not_attempted: 1,
                    affected_locations: std::collections::HashSet::from([root]),
                    report: Default::default(),
                });

            assert_eq!(view.browser().column_request_id(0), Some(request));
            let snapshot = view.browser().column_snapshot(0).expect("visible listing");
            assert!(!snapshot.loading);
            assert_eq!(snapshot.count, 1);
        },
    );
}

type Notify = Rc<dyn Fn(crate::services::DirectoryChange)>;

/// A local source that counts its loads and watches every folder but one.
struct Folders {
    unwatched: Location,
    loads: RefCell<Vec<Location>>,
    monitors: RefCell<Vec<(Location, Notify)>>,
}

impl Folders {
    fn new(unwatched: Location) -> Rc<Self> {
        Rc::new(Self {
            unwatched,
            loads: RefCell::default(),
            monitors: RefCell::default(),
        })
    }

    fn loads(&self, location: &Location) -> usize {
        self.loads
            .borrow()
            .iter()
            .filter(|loaded| *loaded == location)
            .count()
    }

    fn monitor_of(&self, location: &Location) -> Notify {
        self.monitors
            .borrow()
            .iter()
            .find(|(watched, _)| watched == location)
            .map(|(_, notify)| notify.clone())
            .expect("the watched folder's monitor")
    }
}

impl crate::services::FileSource for Folders {
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
        self.loads.borrow_mut().push(request.location.clone());
        crate::adapters::LocalFileSource.enumerate(request, emit)
    }

    fn watch(
        &self,
        location: Location,
        _: bool,
        notify: Notify,
    ) -> Option<crate::services::LoadHandle> {
        (location != self.unwatched).then(|| {
            self.monitors.borrow_mut().push((location, notify));
            crate::services::LoadHandle::new(|| {})
        })
    }
}

struct HeldView {
    source: Rc<Folders>,
    view: crate::ui::browser::BrowserView,
    operations: Rc<crate::test_support::operations::HeldOperations>,
}

fn held_view(unwatched: Location) -> HeldView {
    let source = Folders::new(unwatched);
    let view = crate::ui::browser::BrowserView::new(
        source.clone(),
        crate::ui::browser::PeekBehavior::default(),
    );
    let operations = Rc::new(crate::test_support::operations::HeldOperations::default());
    view.browser().set_operation_provider(operations.clone());
    HeldView {
        source,
        view,
        operations,
    }
}

impl HeldView {
    fn listed(&self, depth: usize) -> Vec<String> {
        self.view
            .browser()
            .with_column_entries(depth, |entries| {
                entries
                    .iter()
                    .map(|entry| entry.display_name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn sorted_names(&self, depth: usize) -> Vec<String> {
        let mut names = self.listed(depth);
        names.sort();
        names
    }

    fn column_loaded(&self, depth: usize) -> bool {
        self.view
            .browser()
            .column_snapshot(depth)
            .is_some_and(|snapshot| !snapshot.loading)
    }

    fn column_listed(&self, depth: usize) -> bool {
        self.view
            .browser()
            .column_snapshot(depth)
            .is_some_and(|snapshot| !snapshot.loading && snapshot.count > 0)
    }

    fn last_started(&self) -> crate::services::OperationRequestId {
        self.view
            .browser()
            .last_started_operation()
            .expect("transfer started")
    }
}

struct Extraction {
    held: HeldView,
    archive: PathBuf,
    other: Location,
    extracted: PathBuf,
    folder: Location,
    _fixture: tempfile::TempDir,
}

fn extraction(watched: bool) -> Extraction {
    let fixture = tempfile::tempdir().expect("extraction fixture");
    let archive = fixture.path().join("bundle.tar");
    std::fs::write(&archive, b"archive").expect("archive");
    let other = fixture.path().join("other");
    std::fs::create_dir(&other).expect("other folder");
    let extracted = fixture.path().join("bundle");
    let folder = Location::local(fixture.path());
    let unwatched = if watched {
        Location::local(&extracted)
    } else {
        folder.clone()
    };
    Extraction {
        held: held_view(unwatched),
        archive,
        other: Location::local(&other),
        extracted,
        folder,
        _fixture: fixture,
    }
}

impl Extraction {
    fn open_the_other_folder(&self) {
        let browser = self.held.view.browser();
        browser.descend(0, self.other.clone());
        wait_until(
            || browser.location_at(1) == Some(self.other.clone()),
            "the other folder",
        );
        browser.set_active_column(1);
    }

    fn loads(&self) -> (usize, usize) {
        (
            self.held.source.loads(&self.folder),
            self.held.source.loads(&self.other),
        )
    }

    fn extract_and_announce(&self, watched: bool) {
        self.held
            .view
            .state
            .extract_entry(transfer_entry(&self.archive));
        let id = self.held.last_started();
        std::fs::create_dir(&self.extracted).expect("extracted folder");
        if watched {
            let notify = self.held.source.monitor_of(&self.folder);
            notify(DirectoryChange::Upsert(FileEntry {
                kind: crate::model::EntryKind::Directory,
                ..transfer_entry(&self.extracted)
            }));
        }
        self.held.operations.emit(
            id,
            OperationEvent::Extracted {
                request_id: id,
                first_name: Some("bundle".to_owned()),
                created: vec![Location::local(&self.extracted)],
            },
        );
    }

    fn extracted_listed(&self, active_elsewhere: bool) -> bool {
        self.held.listed(0).contains(&"bundle".to_owned())
            && (active_elsewhere
                || selected_locations(&self.held.view) == [Location::local(&self.extracted)])
    }
}

#[test]
fn an_extraction_reloads_its_folder_only_without_a_monitor() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::destination_reloads::an_extraction_reloads_its_folder_only_without_a_monitor",
        || {
            for (row, watched, active_elsewhere) in [
                ("watched", true, false),
                ("unwatched", false, false),
                ("unwatched, another folder active", false, true),
            ] {
                let extraction = extraction(watched);
                let held = &extraction.held;
                held.view.navigate_location(extraction.folder.clone());
                wait_until(|| held.listed(0).len() == 2, "the archive's folder");
                if active_elsewhere {
                    extraction.open_the_other_folder();
                }
                let loads = extraction.loads();

                extraction.extract_and_announce(watched);

                wait_until(
                    || extraction.extracted_listed(active_elsewhere),
                    &format!("{row}: the extracted folder to be listed"),
                );
                let reloaded = extraction.loads();
                assert_eq!(
                    (reloaded.0 - loads.0, reloaded.1 - loads.1),
                    (usize::from(!watched), 0),
                    "{row}: only the extraction's folder without a monitor reloads"
                );
                held.view.browser().clear_observer();
            }
        },
    );
}

struct WatchedTransfer {
    held: HeldView,
    item: PathBuf,
    target: PathBuf,
    root: Location,
    destination: Location,
    _fixture: tempfile::TempDir,
}

fn watched_transfer(
    name: &str,
    item_contents: &[u8],
    existing_target: Option<&[u8]>,
) -> WatchedTransfer {
    let fixture = tempfile::tempdir().expect(name);
    let item = fixture.path().join("item.txt");
    std::fs::write(&item, item_contents).expect("transferred item");
    let destination_path = fixture.path().join("destination");
    std::fs::create_dir(&destination_path).expect("destination folder");
    let target = destination_path.join("item.txt");
    if let Some(contents) = existing_target {
        std::fs::write(&target, contents).expect("replaced item");
    }
    let root = Location::local(fixture.path());
    let destination = Location::local(&destination_path);
    WatchedTransfer {
        held: held_view(destination.clone()),
        item,
        target,
        root,
        destination,
        _fixture: fixture,
    }
}

impl WatchedTransfer {
    fn open_destination(&self, listed: impl Fn(usize) -> bool) {
        let browser = self.held.view.browser();
        self.held.view.navigate_location(self.root.clone());
        wait_until(|| listed(0), "the watched folder's listing");
        browser.descend(0, self.destination.clone());
        wait_until(|| listed(1), "the destination's listing");
    }

    fn loads(&self) -> (usize, usize) {
        (
            self.held.source.loads(&self.root),
            self.held.source.loads(&self.destination),
        )
    }

    /// Starts the transfer and places the item as the operation would.
    fn transfer(
        &self,
        conflict: TransferConflict,
        moving: bool,
        reveal: bool,
    ) -> crate::services::OperationRequestId {
        let browser = self.held.view.browser();
        browser.transfer(
            self.destination.clone(),
            vec![PasteItem {
                source: Location::local(&self.item),
                conflict,
                target_name: None,
            }],
            moving,
            reveal,
        );
        let id = self.held.last_started();
        let row = if moving { "move" } else { "copy" };
        assert_eq!(browser.has_foreground_operation(), moving, "{row}");
        if moving {
            std::fs::rename(&self.item, &self.target).expect("moved item");
        } else {
            std::fs::copy(&self.item, &self.target).expect("copied item");
        }
        id
    }

    fn cancel(&self, id: crate::services::OperationRequestId) {
        self.held.view.browser().cancel_operation(id);
        self.held.operations.emit(
            id,
            OperationEvent::Cancelled {
                request_id: id,
                result: CancelledOperation {
                    completed: vec![Location::local(&self.item)],
                    failed: Vec::new(),
                    not_attempted: Vec::new(),
                    affected_locations: HashSet::from([
                        self.root.clone(),
                        self.destination.clone(),
                        Location::local(&self.item),
                        Location::local(&self.target),
                    ]),
                },
            },
        );
    }

    fn complete(&self, id: crate::services::OperationRequestId, moving: bool) {
        self.held.operations.emit(
            id,
            OperationEvent::TransferProgress {
                request_id: id,
                completed_items: 1,
                completed_files: 1,
                total_files: Some(1),
                current_file: None,
                transferred_bytes: 8,
                total_bytes: Some(8),
                created_location: (!moving).then(|| Location::local(&self.target)),
            },
        );
        self.held.operations.emit(
            id,
            OperationEvent::Pasted {
                request_id: id,
                locations: vec![Location::local(&self.item)],
            },
        );
    }
}

#[test]
fn a_cancelled_transfer_reloads_its_unwatched_destination_while_a_watched_folder_rescans() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::destination_reloads::a_cancelled_transfer_reloads_its_unwatched_destination_while_a_watched_folder_rescans",
        || {
            // The view docks a copy in the background and keeps a move in the foreground.
            for moving in [false, true] {
                let row = if moving { "move" } else { "copy" };
                let transfer = watched_transfer("cancelled transfer fixture", b"item", None);
                let held = &transfer.held;
                transfer.open_destination(|depth| match depth {
                    0 => held.sorted_names(0).len() == 2,
                    depth => held.column_loaded(depth),
                });
                let loaded = transfer.loads();

                let id = transfer.transfer(TransferConflict::FailIfExists, moving, false);
                let rescan = held.source.monitor_of(&transfer.root);
                rescan(DirectoryChange::Rescan);
                transfer.cancel(id);

                wait_until(
                    || held.sorted_names(1) == ["item.txt"],
                    &format!("{row}: the destination to list what the transfer placed"),
                );
                let listed = if moving {
                    vec!["destination"]
                } else {
                    vec!["destination", "item.txt"]
                };
                wait_until(
                    || held.sorted_names(0) == listed,
                    &format!("{row}: the watched folder's rescan"),
                );
                assert_eq!(
                    transfer.loads(),
                    (loaded.0 + 1, loaded.1 + 1),
                    "{row}: each folder reloads once"
                );
                held.view.browser().clear_observer();
            }
        },
    );
}

#[test]
fn a_completed_transfer_reloads_its_unwatched_destination_once() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::destination_reloads::a_completed_transfer_reloads_its_unwatched_destination_once",
        || {
            // The view docks a copy in the background and keeps a move in the foreground.
            for moving in [false, true] {
                let row = if moving { "move" } else { "copy" };
                let transfer =
                    watched_transfer("completed transfer fixture", b"new item", Some(b"old"));
                let held = &transfer.held;
                transfer.open_destination(|depth| held.column_listed(depth));
                let loads = held.source.loads(&transfer.destination);

                let id = transfer.transfer(TransferConflict::ReplaceExisting, moving, true);
                transfer.complete(id, moving);

                wait_until(
                    || {
                        held.column_listed(1)
                            && selected_locations(&held.view) == [Location::local(&transfer.target)]
                    },
                    &format!("{row}: the destination to reveal the replaced item"),
                );
                assert_eq!(
                    held.source.loads(&transfer.destination),
                    loads + 1,
                    "{row}: the destination reloads once"
                );
                held.view.browser().clear_observer();
            }
        },
    );
}

#[test]
fn transfer_reveal_waits_for_the_live_monitor_instead_of_reloading() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::destination_reloads::transfer_reveal_waits_for_the_live_monitor_instead_of_reloading",
        || {
            let fixture = tempfile::tempdir().expect("transfer reveal fixture");
            let destination = Location::local(fixture.path());
            let view = local_view();
            view.navigate_location(destination.clone());
            wait_until(
                || local_listing_loaded(&view, 0),
                "the empty local destination",
            );
            let request = view.browser().column_request_id(0).expect("loaded request");
            let first = fixture.path().join("first.txt");
            let second = fixture.path().join("second.txt");
            view.state
                .handle(&crate::app::BrowserEvent::TransferReveal {
                    destination,
                    locations: vec![Location::local(&first), Location::local(&second)],
                });

            std::fs::write(&first, b"first").expect("first copied fixture");
            wait_until(
                || selected_locations(&view) == [Location::local(&first)],
                "the first monitor update to select its target",
            );
            assert!(view.state.pending_location_selection.borrow().is_some());
            std::fs::write(&second, b"second").expect("second copied fixture");
            wait_until(
                || selected_locations(&view) == [Location::local(&first), Location::local(&second)],
                "both transferred targets to be selected",
            );
            assert!(view.state.pending_location_selection.borrow().is_none());
            assert_eq!(view.browser().column_request_id(0), Some(request));
        },
    );
}

#[test]
fn revealing_an_already_open_local_destination_keeps_its_live_listing() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::destination_reloads::revealing_an_already_open_local_destination_keeps_its_live_listing",
        || {
            let fixture = tempfile::tempdir().expect("transfer reveal fixture");
            let copied = fixture.path().join("copied.txt");
            std::fs::write(&copied, b"copied").expect("copied fixture");
            let destination = Location::local(fixture.path());
            let view = local_view();
            view.navigate_location(destination.clone());
            wait_until(
                || local_listing_loaded(&view, 1),
                "the local destination listing",
            );
            let request = view.browser().column_request_id(0).expect("loaded request");

            view.state
                .handle(&crate::app::BrowserEvent::TransferReveal {
                    destination,
                    locations: vec![Location::local(&copied)],
                });

            assert_eq!(view.browser().column_request_id(0), Some(request));
            assert_eq!(selected_locations(&view), [Location::local(copied)]);
        },
    );
}
