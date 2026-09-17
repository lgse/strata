// SPDX-License-Identifier: MIT

use std::{cell::RefCell, time::Duration};

use super::*;
use crate::{
    adapters::{LocalFileSource, LocalPreviewProvider},
    app::BrowserEvent,
    services::{SearchEvent, index_trees},
    test_support::gtk_test,
};

struct SearchFixture {
    browser: Rc<Browser>,
    preview: PreviewDrawer,
    events: Rc<RefCell<Vec<(&'static str, Location)>>>,
}

impl SearchFixture {
    fn new(preferences: &Rc<ThemeManager>) -> Self {
        let browser = Browser::new(Rc::new(LocalFileSource));
        let preferences = preferences.clone();
        let preview = PreviewDrawer::new(
            Rc::new(LocalPreviewProvider::new(Rc::new(move || {
                preferences.media_preview_backend()
            }))),
            false,
        );
        preview.observe_browser(&browser);
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        browser.observe(move |event| {
            let event = match event {
                BrowserEvent::ColumnAdded { location, .. } => ("navigate", location.clone()),
                BrowserEvent::OpenRequested { location } => ("open", location.clone()),
                _ => return,
            };
            observed.borrow_mut().push(event);
        });
        Self {
            browser,
            preview,
            events,
        }
    }
}

#[test]
fn visit_recorder_commits_only_completed_new_navigation() {
    let path = Location::local("/work/project");
    let mut recorder = VisitRecorder::default();
    assert!(
        recorder
            .handle(&BrowserEvent::ColumnAdded {
                depth: 0,
                location: path.clone(),
            })
            .is_none()
    );
    assert_eq!(
        recorder.handle(&BrowserEvent::LoadFinished {
            depth: 0,
            truncated: false,
        }),
        Some(std::path::PathBuf::from("/work/project"))
    );
    assert!(
        recorder
            .handle(&BrowserEvent::LoadFinished {
                depth: 0,
                truncated: false,
            })
            .is_none()
    );
}

#[test]
fn visit_recorder_discards_superseded_navigation_and_records_successful_retries() {
    let mut recorder = VisitRecorder::default();
    for (depth, path) in [(0, "/work"), (1, "/work/project")] {
        recorder.handle(&BrowserEvent::ColumnAdded {
            depth,
            location: Location::local(path),
        });
    }
    recorder.handle(&BrowserEvent::ColumnsTruncated { len: 1 });
    assert!(
        recorder
            .handle(&BrowserEvent::LoadFinished {
                depth: 1,
                truncated: false,
            })
            .is_none()
    );
    assert!(
        recorder
            .handle(&BrowserEvent::LoadFailed {
                depth: 0,
                message: "unavailable".into(),
            })
            .is_none()
    );
    assert_eq!(
        recorder.handle(&BrowserEvent::LoadFinished {
            depth: 0,
            truncated: false,
        }),
        Some(std::path::PathBuf::from("/work"))
    );
}

fn indexed_items(root: &std::path::Path) -> Vec<SearchItem> {
    let (handle, receiver) = index_trees(vec![root.to_path_buf()], false);
    handle.query("fixture");
    loop {
        let SearchEvent::Results {
            query,
            items,
            indexing,
            ..
        } = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("fixture search completes");
        if query == "fixture" && !indexing {
            return items;
        }
    }
}

#[test]
fn same_folder_search_preview_follows_focus_after_deletion() {
    gtk_test(
        "ui::window::composition::search::tests::same_folder_search_preview_follows_focus_after_deletion",
        || {
            let root = tempfile::tempdir().expect("fixture directory");
            let path = root.path().join("fixture.txt");
            std::fs::write(&path, "preview fixture").expect("fixture file");
            std::fs::write(root.path().join("remaining.txt"), "remaining").expect("remaining file");
            let file = indexed_items(root.path()).remove(0);
            let preferences = ThemeManager::shared();
            preferences.set_search_open_files_directly(false);
            let fixture = SearchFixture::new(&preferences);
            fixture.browser.navigate(Location::local(root.path()));
            crate::ui::media::tests::wait(|| {
                fixture
                    .browser
                    .column_snapshot(0)
                    .is_some_and(|s| !s.loading)
            });
            fixture.events.borrow_mut().clear();
            activate_result(&fixture.browser, &fixture.preview, &preferences, file);
            assert!(fixture.events.borrow().is_empty());
            assert!(fixture.preview.is_open());
            std::fs::remove_file(&path).expect("remove previewed file");
            crate::ui::media::tests::wait(|| {
                fixture
                    .browser
                    .column_snapshot(0)
                    .is_some_and(|s| s.count == 1 && s.selected_positions == [0])
            });
            assert!(fixture.preview.is_open());
            assert!(fixture.events.borrow().is_empty());
            fixture.browser.clear_observer();
        },
    );
}

#[test]
fn result_activation_reads_live_preferences_and_preserves_navigation_order() {
    gtk_test(
        "ui::window::composition::search::tests::result_activation_reads_live_preferences_and_preserves_navigation_order",
        || {
            let root = tempfile::tempdir().expect("fixture directory");
            std::fs::write(root.path().join("fixture.txt"), "preview fixture")
                .expect("fixture file");
            std::fs::create_dir(root.path().join("fixture-folder")).expect("fixture folder");
            let items = indexed_items(root.path());
            let file = items
                .iter()
                .find(|item| !item.is_directory)
                .expect("file result");
            let directory = items
                .iter()
                .find(|item| item.is_directory)
                .expect("directory result");
            let preferences = ThemeManager::shared();
            let fixtures = [
                SearchFixture::new(&preferences),
                SearchFixture::new(&preferences),
            ];
            for direct in [false, true, false] {
                preferences.set_search_open_files_directly(direct);
                for fixture in &fixtures {
                    fixture.events.borrow_mut().clear();
                    activate_result(
                        &fixture.browser,
                        &fixture.preview,
                        &preferences,
                        file.clone(),
                    );
                    assert_eq!(
                        fixture.browser.active_location(),
                        Some(Location::local(root.path()))
                    );
                    assert_eq!(fixture.preview.is_open(), !direct);
                    let mut expected = vec![("navigate", Location::local(root.path()))];
                    if direct {
                        expected.push(("open", Location::local(&file.path)));
                    }
                    assert_eq!(*fixture.events.borrow(), expected);
                    fixture.events.borrow_mut().clear();
                    activate_result(
                        &fixture.browser,
                        &fixture.preview,
                        &preferences,
                        directory.clone(),
                    );
                    assert!(!fixture.preview.is_open());
                    assert_eq!(
                        fixture.browser.active_location(),
                        Some(Location::local(&directory.path))
                    );
                    assert_eq!(
                        *fixture.events.borrow(),
                        [("navigate", Location::local(&directory.path))]
                    );
                }
            }
            for fixture in fixtures {
                fixture.browser.clear_observer();
            }
        },
    );
}
