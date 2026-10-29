// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{
    DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
    MetadataRequest,
    file_activity::{self, OperationScope},
};

#[derive(Default)]
struct RecordingSource {
    fills: RefCell<Vec<Vec<Location>>>,
}

impl FileSource for RecordingSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        crate::adapters::LocalFileSource.enumerate(request, emit)
    }

    fn supports_metadata_fill(&self, _: &Location) -> bool {
        true
    }

    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        self.fills.borrow_mut().push(request.entries.clone());
        crate::adapters::LocalFileSource.fill_metadata(request, emit)
    }
}

#[test]
fn completed_counts_refresh_only_for_live_visible_targets_after_the_last_writer() {
    crate::test_support::gtk_test(
        "ui::thumbnail::viewport::tests::folder_counts::completed_counts_refresh_only_for_live_visible_targets_after_the_last_writer",
        || {
            let directory = tempfile::tempdir().expect("folder fixture");
            let names = ["visible", "hidden", "unbound"];
            let paths = names.map(|name| directory.path().join(name));
            for path in &paths {
                std::fs::create_dir(path).expect("folder");
            }
            let source = Rc::new(RecordingSource::default());
            let browser = Browser::new(source.clone());
            browser.observe(|event| {
                if matches!(event, crate::app::BrowserEvent::FolderCountsInvalidated) {
                    schedule_refresh();
                }
            });
            browser.navigate(Location::local(directory.path()));
            let wait = |message: &str, ready: &dyn Fn() -> bool| {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !ready() {
                    assert!(Instant::now() < deadline, "{message}");
                    glib::MainContext::default().iteration(false);
                }
            };
            wait("folders did not load", &|| {
                browser
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading && column.count == 3)
            });
            let positions = paths.each_ref().map(|path| {
                (0..3)
                    .find(|position| {
                        browser
                            .entry_at(0, *position)
                            .is_some_and(|entry| entry.location == Location::local(path))
                    })
                    .expect("folder position")
            });
            let count = |index: usize| {
                browser
                    .entry_at(0, positions[index])
                    .expect("folder")
                    .child_count
            };
            for (index, path) in paths.iter().enumerate() {
                browser.request_metadata_fill(0, positions[index], Location::local(path), true);
            }
            wait("initial counts did not finish", &|| {
                (0..3).all(|index| count(index) == MetadataValue::Known(0))
            });
            source.fills.borrow_mut().clear();
            let slots = names.map(|_| ThumbnailSlot::new(18));
            let stack = gtk::Stack::new();
            for (index, name) in names[..2].iter().enumerate() {
                let viewport = gtk::ScrolledWindow::new();
                viewport.set_child(Some(&slots[index]));
                stack.add_named(&viewport, Some(name));
            }
            stack.set_visible_child_name("visible");
            let window = gtk::Window::new();
            window.set_child(Some(&stack));
            window.present();
            wait("visible target did not map", &|| slots[0].is_mapped());
            for (index, slot) in slots.iter().enumerate() {
                request_metadata(
                    slot,
                    slot,
                    &browser,
                    0,
                    positions[index],
                    Location::local(&paths[index]),
                    true,
                );
            }
            cancel_metadata(slots[2].as_ptr() as usize);
            wait("cached targets were not admitted", &|| {
                !REFRESH_PENDING.with(Cell::get)
            });
            assert!(source.fills.borrow().is_empty(), "known counts are reused");

            let first = file_activity::begin(OperationScope {
                roots: Vec::new(),
                folders: paths.to_vec(),
            });
            let second = file_activity::begin(OperationScope {
                roots: Vec::new(),
                folders: paths.to_vec(),
            });
            for path in &paths {
                std::fs::write(path.join("document.txt"), b"document").expect("new document");
            }
            drop(first);
            wait("first invalidation did not settle", &|| {
                !REFRESH_PENDING.with(Cell::get)
            });
            assert!(
                source.fills.borrow().is_empty(),
                "another writer still owns the folders"
            );
            drop(second);
            wait("visible count did not update", &|| {
                count(0) == MetadataValue::Known(1)
            });
            assert_eq!(
                source.fills.borrow().as_slice(),
                [vec![Location::local(&paths[0])]]
            );
            assert_eq!(count(1), MetadataValue::Unknown);
            assert_eq!(count(2), MetadataValue::Unknown);

            stack.set_visible_child_name("hidden");
            wait("newly visible count did not update", &|| {
                count(1) == MetadataValue::Known(1)
            });
            assert_eq!(
                source.fills.borrow().as_slice(),
                [
                    vec![Location::local(&paths[0])],
                    vec![Location::local(&paths[1])]
                ]
            );
            assert_eq!(
                count(2),
                MetadataValue::Unknown,
                "an unbound target never retries"
            );
            for slot in &slots {
                cancel_metadata(slot.as_ptr() as usize);
            }
            window.close();
        },
    );
}
