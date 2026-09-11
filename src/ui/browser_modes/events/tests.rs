// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::glib;

use super::*;
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError},
    test_support::gtk_test,
};

struct StaticSource;

impl FileSource for StaticSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: entries(),
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

fn entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        thumbnail_path: None,
        kind: EntryKind::File,
        size: MetadataValue::Known(10),
        modified_unix_seconds: MetadataValue::Known(1),
        mode: MetadataValue::Known(0o100644),
        is_hidden: name.starts_with('.'),
    }
}

fn entries() -> Vec<FileEntry> {
    ["a.txt", "b.png", "c.rs"].into_iter().map(entry).collect()
}

fn presentations() -> [(BrowserMode, bool); 3] {
    [
        (BrowserMode::Icons, false),
        (BrowserMode::List, false),
        (BrowserMode::List, true),
    ]
}

struct Fixture {
    views: ModeViews,
    browser: Rc<Browser>,
    window: gtk::Window,
    outside: gtk::Entry,
}

impl Fixture {
    fn new(mode: BrowserMode, grouped: bool) -> Self {
        let browser = Browser::new(Rc::new(StaticSource));
        browser.navigate(Location::local("/fixture"));
        let mut views = ModeViews::new(
            &gtk::ScrolledWindow::new(),
            browser.clone(),
            Rc::new(Cell::new(true)),
        );
        views.set_group_by_type(grouped);
        views.prepare_mode(mode);
        views.show_mode(mode);
        let outside = gtk::Entry::new();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&outside);
        root.append(&views.widget());
        let window = gtk::Window::builder()
            .default_width(800)
            .default_height(600)
            .child(&root)
            .build();
        Self {
            views,
            browser,
            window,
            outside,
        }
    }

    fn pane(&self) -> Pane {
        self.views.single_pane().expect("visible pane").clone()
    }

    fn show(&self) {
        self.window.present();
        pump_until(|| self.pane().section.view.is_mapped());
    }

    fn names(&self) -> Vec<String> {
        let pane = self.pane();
        (0..pane.model.n_items())
            .map(|position| pane.model.string(position).expect("row").to_string())
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.views.clear_icons();
        self.views.clear_list();
        self.browser.clear_observer();
        self.window.close();
    }
}

fn pump_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "GTK state did not settle");
        context.iteration(false);
    }
}

fn visible_page(pane: &Pane) -> String {
    pane.stack.visible_child_name().expect("page").to_string()
}

fn assert_attached(pane: &Pane, attached: bool) {
    assert_eq!(pane.detached.get(), !attached);
    for section in pane.all_sections() {
        assert_eq!(section.selection.model().is_some(), attached);
        assert_eq!(section.syncing.get(), !attached);
        let model = if let Some(view) = section.view.downcast_ref::<gtk::GridView>() {
            view.model()
        } else {
            section
                .view
                .downcast_ref::<gtk::ListView>()
                .expect("collection")
                .model()
        };
        assert!(
            model.is_some(),
            "reload must retain the collection's selection model"
        );
    }
    if let Some(filtered) = &pane.filter_model {
        assert_eq!(filtered.model().is_some(), attached);
    }
}

#[test]
fn reload_retains_views_until_terminal_reconnects_models() {
    gtk_test(
        "ui::browser_modes::events::tests::reload_retains_views_until_terminal_reconnects_models",
        || {
            for (mode, grouped) in presentations() {
                let mut fixture = Fixture::new(mode, grouped);
                let pane = fixture.pane();
                fixture
                    .views
                    .handle(&BrowserEvent::ColumnReloaded { depth: 0 });
                assert_attached(&pane, false);
                assert_eq!(pane.model.n_items(), 0);
                assert!(pane.spinner.is_spinning());
                assert!(!pane.truncated_hint.get_visible());
                fixture
                    .views
                    .handle(&BrowserEvent::EntriesReplaced { depth: 0, count: 1 });
                assert_attached(&pane, false);
                assert_eq!(pane.model.n_items(), 1);
                assert!(!pane.spinner.is_spinning());
                fixture.views.handle(&BrowserEvent::EntriesPublished {
                    depth: 0,
                    position: 1,
                    count: 2,
                });
                fixture.views.handle(&BrowserEvent::LoadFinished {
                    depth: 0,
                    truncated: true,
                });
                assert_attached(&pane, true);
                assert_eq!(pane.model.n_items(), 3);
                assert!(pane.truncated_hint.get_visible());
                assert!(!pane.spinner.get_visible());
                assert_eq!(visible_page(&pane), "content");
            }
        },
    );
}

#[test]
fn busy_insertions_and_splices_preserve_distinct_presentation_rules() {
    gtk_test(
        "ui::browser_modes::events::tests::busy_insertions_and_splices_preserve_distinct_presentation_rules",
        || {
            for (mode, grouped) in presentations() {
                let mut fixture = Fixture::new(mode, grouped);
                let pane = fixture.pane();
                fixture
                    .views
                    .handle(&BrowserEvent::EntriesReplaced { depth: 0, count: 0 });
                fixture
                    .views
                    .handle(&BrowserEvent::SortingStarted { depth: 0 });
                fixture.views.handle(&BrowserEvent::EntriesInserted {
                    depth: 0,
                    insertions: vec![EntryInsertion {
                        position: 0,
                        entries: vec![entry("x.txt")],
                    }],
                });
                assert_eq!(visible_page(&pane), "status");
                assert!(pane.spinner.is_spinning());
                assert_eq!(pane.spinner.tooltip_text().as_deref(), Some("Sorting…"));
                fixture.views.handle(&BrowserEvent::EntriesSpliced {
                    depth: 0,
                    splices: vec![EntrySplice {
                        position: 0,
                        removed: 1,
                        entries: vec![entry("y.txt"), entry("z.txt")],
                    }],
                });
                assert_eq!(visible_page(&pane), "content");
                assert_eq!(
                    fixture.names(),
                    vec![
                        entry_model_value(&entry("y.txt")),
                        entry_model_value(&entry("z.txt"))
                    ]
                );
                assert!(pane.spinner.is_spinning());
                fixture
                    .views
                    .handle(&BrowserEvent::SortingFinished { depth: 0 });
                assert!(!pane.spinner.is_spinning());
                assert!(!pane.spinner.get_visible());
                assert!(pane.spinner.tooltip_text().is_none());
            }
        },
    );
}

#[test]
fn failures_reconnect_without_losing_error_and_empty_transitions() {
    gtk_test(
        "ui::browser_modes::events::tests::failures_reconnect_without_losing_error_and_empty_transitions",
        || {
            for (mode, grouped) in presentations() {
                let mut fixture = Fixture::new(mode, grouped);
                let pane = fixture.pane();
                fixture
                    .views
                    .handle(&BrowserEvent::ColumnReloaded { depth: 0 });
                fixture.views.handle(&BrowserEvent::LoadFailed {
                    depth: 0,
                    message: "provider failure".into(),
                });
                assert_attached(&pane, true);
                assert_eq!(visible_page(&pane), "status");
                assert_eq!(
                    pane.status.label(),
                    "Unable to read this directory\nprovider failure"
                );
                assert!(pane.status.has_css_class("error"));
                assert!(!pane.spinner.is_spinning());
                assert!(pane.spinner.get_visible());
                fixture.views.handle(&BrowserEvent::LoadFinished {
                    depth: 0,
                    truncated: false,
                });
                assert_eq!(pane.status.label(), "This directory is empty");
                assert!(!pane.status.has_css_class("error"));
                assert!(!pane.spinner.get_visible());
                assert_eq!(visible_page(&pane), "status");
            }
        },
    );
}

#[test]
fn publication_releases_browser_borrows_before_gtk_notifications() {
    gtk_test(
        "ui::browser_modes::events::tests::publication_releases_browser_borrows_before_gtk_notifications",
        || {
            for (mode, grouped) in presentations() {
                let mut fixture = Fixture::new(mode, grouped);
                fixture
                    .views
                    .handle(&BrowserEvent::EntriesReplaced { depth: 0, count: 0 });
                let changed = Rc::new(Cell::new(false));
                let observed = changed.clone();
                let weak = Rc::downgrade(&fixture.browser);
                fixture
                    .pane()
                    .model
                    .connect_items_changed(move |_, _, _, _| {
                        weak.upgrade().expect("browser").set_selection(0, &[], None);
                        observed.set(true);
                    });
                fixture.views.handle(&BrowserEvent::EntriesPublished {
                    depth: 0,
                    position: 0,
                    count: 3,
                });
                assert!(changed.get());
                assert_eq!(
                    fixture.names(),
                    entries().iter().map(entry_model_value).collect::<Vec<_>>()
                );
            }
        },
    );
}

#[test]
fn inactive_depths_and_cached_modes_do_not_receive_row_updates() {
    gtk_test(
        "ui::browser_modes::events::tests::inactive_depths_and_cached_modes_do_not_receive_row_updates",
        || {
            let mut fixture = Fixture::new(BrowserMode::Icons, false);
            let icons = fixture.pane();
            fixture.views.prepare_mode(BrowserMode::List);
            fixture.views.show_mode(BrowserMode::List);
            let list = fixture.pane();
            let insert = |depth| BrowserEvent::EntriesInserted {
                depth,
                insertions: vec![EntryInsertion {
                    position: 3,
                    entries: vec![entry("d.txt")],
                }],
            };
            fixture.views.handle(&insert(9));
            assert_eq!(list.model.n_items(), 3);
            fixture.views.handle(&insert(0));
            assert_eq!(list.model.n_items(), 4);
            assert_eq!(icons.model.n_items(), 3);
            fixture.views.prepare_mode(BrowserMode::Columns);
            fixture.views.handle(&insert(0));
            assert_eq!(list.model.n_items(), 4);
            fixture.views.handle(&BrowserEvent::Reset);
            assert!(fixture.views.icons_panes.is_empty());
            assert!(fixture.views.list_pane.is_none());
            assert!(fixture.views.icons_root.first_child().is_none());
            assert!(fixture.views.list_root.first_child().is_none());
        },
    );
}

#[test]
fn structural_events_rebuild_only_the_active_presentation() {
    gtk_test(
        "ui::browser_modes::events::tests::structural_events_rebuild_only_the_active_presentation",
        || {
            for (mode, grouped) in presentations() {
                let mut fixture = Fixture::new(mode, grouped);
                let old = fixture.pane().shell;
                fixture.views.handle(&BrowserEvent::ColumnAdded {
                    depth: 9,
                    location: Location::local("/other"),
                });
                assert_eq!(fixture.pane().shell, old);
                fixture.views.handle(&BrowserEvent::ColumnAdded {
                    depth: 0,
                    location: Location::local("/fixture"),
                });
                assert_ne!(fixture.pane().shell, old);
                let old = fixture.pane().shell;
                fixture
                    .views
                    .handle(&BrowserEvent::ColumnsTruncated { len: 1 });
                assert_ne!(fixture.pane().shell, old);
                assert_eq!(fixture.names().len(), 3);
            }
        },
    );
}

#[test]
fn selection_preserves_external_focus_and_restores_requested_pane_focus() {
    gtk_test(
        "ui::browser_modes::events::tests::selection_preserves_external_focus_and_restores_requested_pane_focus",
        || {
            for (mode, grouped) in presentations() {
                let mut fixture = Fixture::new(mode, grouped);
                fixture.show();
                fixture.outside.grab_focus();
                fixture.browser.set_selection(0, &[1], Some(1));
                let selection = |positions, take_focus| BrowserEvent::SelectionSetChanged {
                    depth: 0,
                    positions,
                    focused: 1,
                    take_focus,
                };
                fixture.views.handle(&selection(vec![1], false));
                assert!(super::super::widget_has_focus(
                    &fixture.outside,
                    gtk::prelude::RootExt::focus(&fixture.window).as_ref()
                ));
                assert_eq!(fixture.views.selected_positions(), Some((0, vec![1])));
                fixture.views.handle(&selection(vec![1], true));
                pump_until(|| fixture.views.item_view_has_focus());
                fixture.views.suppress_focus_scroll();
                fixture.views.handle(&selection(vec![1], false));
                assert!(!fixture.views.suppress_focus_scroll.get());
                fixture.views.suppress_focus_scroll();
                fixture.views.handle(&selection(vec![], false));
                assert!(fixture.views.suppress_focus_scroll.get());
                fixture.views.handle(&BrowserEvent::FocusChanged {
                    depth: 0,
                    position: Some(1),
                });
                assert!(!fixture.views.suppress_focus_scroll.get());
                assert_eq!(fixture.views.selected_positions(), Some((0, vec![1])));
            }
        },
    );
}

fn bound_row(pane: &Pane, source: usize) -> Option<gtk::Box> {
    pane.section.bound_items.borrow().iter().find_map(|bound| {
        let item = bound.item.upgrade()?;
        (pane.source_index.of_item(&item.item()?) == Some(source))
            .then(|| bound.widget.upgrade()?.downcast::<gtk::Box>().ok())
            .flatten()
    })
}

#[test]
fn list_metadata_updates_bound_rows_without_replacing_the_model() {
    gtk_test(
        "ui::browser_modes::events::tests::list_metadata_updates_bound_rows_without_replacing_the_model",
        || {
            for grouped in [false, true] {
                let mut fixture = Fixture::new(BrowserMode::List, grouped);
                fixture.show();
                let pane = fixture.pane();
                pump_until(|| bound_row(&pane, 1).is_some());
                let row = bound_row(&pane, 1).expect("bound row");
                let (_, _, _, mode, size, _, _) =
                    super::super::list_row_parts(&row).expect("list labels");
                let changed = Rc::new(Cell::new(false));
                let observed = changed.clone();
                pane.model
                    .connect_items_changed(move |_, _, _, _| observed.set(true));
                let mut update = entry("b.png");
                update.size = MetadataValue::Known(2048);
                update.mode = MetadataValue::Known(0o100600);
                fixture.views.handle(&BrowserEvent::MetadataFilled {
                    depth: 0,
                    updates: vec![(1, update.clone())],
                });
                assert_eq!(size.label(), super::super::entry_size(&update));
                assert_eq!(mode.label(), super::super::entry_mode(&update));
                assert!(!changed.get());
            }
        },
    );
}

fn native_select_item(view: &gtk::Widget, position: u32, modify: bool, extend: bool) {
    view.activate_action(
        "list.select-item",
        Some(&(position, modify, extend).to_variant()),
    )
    .expect("list.select-item");
}

fn gtk_selected(pane: &Pane) -> Vec<usize> {
    (0..pane.section.selection.n_items())
        .filter(|&position| pane.section.selection.is_selected(position))
        .map(|position| position as usize)
        .collect()
}

#[test]
fn resume_native_selection_starts_from_the_cursor_after_escape() {
    gtk_test(
        "ui::browser_modes::events::tests::resume_native_selection_starts_from_the_cursor_after_escape",
        || {
            for (mode, grouped) in presentations() {
                let fixture = Fixture::new(mode, grouped);
                fixture.show();
                fixture.browser.select(0, 0);
                fixture.browser.set_selection(0, &[0, 1], Some(1));
                assert_eq!(fixture.browser.selected_positions(0), [0, 1]);
                assert_eq!(fixture.browser.selection_anchor_position(0), Some(0));

                assert!(fixture.browser.clear_active_selection());
                assert!(fixture.browser.selected_positions(0).is_empty());
                assert_eq!(
                    fixture
                        .browser
                        .focused_item()
                        .map(|(_, position, _)| position),
                    Some(1)
                );
                assert_eq!(fixture.browser.selection_anchor_position(0), Some(0));

                if !grouped {
                    let pane = fixture.pane();
                    pane.section.syncing.set(true);
                    native_select_item(&pane.section.view, 0, false, false);
                    native_select_item(&pane.section.view, 1, false, true);
                    pane.section.syncing.set(false);
                    assert_eq!(
                        gtk_selected(&pane),
                        [0, 1],
                        "{mode:?}: plant GTK's leftover range origin on the first item"
                    );
                }

                assert!(
                    fixture.views.resume_native_selection(),
                    "{mode:?} grouped={grouped}: seed the cursor when filled selection is empty"
                );
                assert_eq!(fixture.browser.selected_positions(0), [1]);
                assert_eq!(
                    fixture.browser.selection_anchor_position(0),
                    Some(1),
                    "{mode:?} grouped={grouped}: leftover range anchor must not be reused"
                );
                if !grouped {
                    let view = fixture.pane().section.view.clone();
                    native_select_item(&view, 2, false, true);
                    assert_eq!(
                        gtk_selected(&fixture.pane()),
                        [1, 2],
                        "{mode:?}: native Shift must not re-include the old range origin"
                    );
                }
                assert!(
                    !fixture.views.resume_native_selection(),
                    "{mode:?} grouped={grouped}: a filled selection must keep native Shift movement"
                );
            }
        },
    );
}
