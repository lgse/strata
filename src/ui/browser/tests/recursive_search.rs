// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::{
    browser::collection::deactivate_recursive_search, entry_list_model::EntryListModel,
};

#[test]
fn clearing_recursive_search_deactivates_before_rows_rebind() {
    const CHILD: &str = "STRATA_RECURSIVE_SEARCH_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "ui::browser::tests::recursive_search::clearing_recursive_search_deactivates_before_rows_rebind",
            ])
            .env(CHILD, "1")
            .status()
            .expect("isolated GTK test should start");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }

    let directory_model = EntryListModel::new(Rc::new(|position| {
        (position < 3).then(|| format!("entry-{position}"))
    }));
    directory_model.replace(3);
    let search_model = gtk::StringList::new(&["hit-a", "hit-b"]);
    let filtered_model =
        gtk::FilterListModel::new(Some(search_model.clone()), None::<gtk::CustomFilter>);
    let search_active = Rc::new(Cell::new(true));
    let search_results = Rc::new(RefCell::new(Vec::new()));

    let active_during_rebind: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(Vec::new()));
    let observed = active_during_rebind.clone();
    let flag = search_active.clone();
    filtered_model.connect_items_changed(move |_, _, _, added| {
        if added > 0 {
            observed.borrow_mut().push(flag.get());
        }
    });

    deactivate_recursive_search(
        &search_active,
        &search_results,
        &search_model,
        &filtered_model,
        &directory_model,
    );

    assert!(!search_active.get());
    assert_eq!(filtered_model.n_items(), 3);
    assert!(
        !active_during_rebind.borrow().is_empty(),
        "swapping back to the directory model should have re-added rows"
    );
    assert!(
        active_during_rebind.borrow().iter().all(|active| !active),
        "rows rebound while recursive search still looked active"
    );
}

struct TrashFiles(Vec<FileEntry>);

impl crate::services::FileSource for TrashFiles {
    fn validate_location(
        &self,
        _: &Location,
    ) -> Result<(), crate::services::LocationValidationError> {
        Ok(())
    }

    fn enumerate(
        &self,
        request: crate::services::DirectoryRequest,
        emit: Rc<dyn Fn(crate::services::DirectoryEvent)>,
    ) -> crate::services::LoadHandle {
        emit(crate::services::DirectoryEvent::Batch {
            request_id: request.id,
            entries: self.0.clone(),
        });
        emit(crate::services::DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: Some(false),
            can_delete: Some(true),
        });
        crate::services::LoadHandle::new(|| {})
    }
}

fn trash_entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::uri(format!("trash:///{name}")),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Known(42),
        modified_unix_seconds: crate::model::MetadataValue::Known(1),
        mode: crate::model::MetadataValue::Unavailable,
        is_hidden: false,
    }
}

#[test]
fn non_native_location_filters_entries_in_columns_mode() {
    crate::test_support::gtk_test(
        "ui::browser::tests::recursive_search::non_native_location_filters_entries_in_columns_mode",
        || {
            use std::time::{Duration, Instant};

            let entries = vec![
                trash_entry("Services"),
                trash_entry("Music"),
                trash_entry("Notes"),
            ];
            let view = BrowserView::new(Rc::new(TrashFiles(entries)), PeekBehavior::default());
            let browser = view.browser();
            view.set_view_mode(BrowserMode::Columns);
            browser.navigate(Location::uri("trash:///"));

            let deadline = Instant::now() + Duration::from_secs(5);
            while browser.column_snapshot(0).is_none_or(|s| s.loading) {
                assert!(Instant::now() < deadline, "column did not finish loading");
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(2));
            }

            assert_eq!(view.state.columns.borrow()[0].filtered_model.n_items(), 3);

            let filter_entry = view.state.columns.borrow()[0].filter_entry.clone();
            filter_entry.set_text("Serv");

            let deadline = Instant::now() + Duration::from_secs(5);
            while view.state.columns.borrow()[0].filtered_model.n_items() != 1 {
                assert!(Instant::now() < deadline, "filter did not settle");
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(2));
            }

            assert_eq!(view.state.columns.borrow()[0].filtered_model.n_items(), 1);
            assert!(
                view.state.columns.borrow()[0]
                    .search_handle
                    .borrow()
                    .is_none()
            );

            let opened = Rc::new(Cell::new(false));
            let opened_for_observe = opened.clone();
            browser.observe(move |event| {
                if matches!(event, crate::app::BrowserEvent::OpenRequested { .. }) {
                    opened_for_observe.set(true);
                }
            });

            filter_entry.set_text("Notes");
            let deadline = Instant::now() + Duration::from_secs(5);
            while view.state.columns.borrow()[0]
                .map
                .source_position(0)
                .and_then(|position| browser.entry_at(0, position))
                .is_none_or(|entry| entry.location != Location::uri("trash:///Notes"))
            {
                assert!(Instant::now() < deadline, "filter did not settle");
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(2));
            }
            view.state.columns.borrow()[0]
                .selection
                .select_item(0, true);
            assert_eq!(browser.selected_positions(0), vec![1]);
            let selected = view.selected_search_results().expect("filtered selection");
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].location, Location::uri("trash:///Notes"));
            assert!(!opened.get(), "trash file should not open on selection");

            filter_entry.set_text("");
            let deadline = Instant::now() + Duration::from_secs(5);
            while view.state.columns.borrow()[0].filtered_model.n_items() != 3 {
                assert!(Instant::now() < deadline, "filter clear did not settle");
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(2));
            }

            assert_eq!(view.state.columns.borrow()[0].filtered_model.n_items(), 3);
            browser.clear_observer();
        },
    );
}
