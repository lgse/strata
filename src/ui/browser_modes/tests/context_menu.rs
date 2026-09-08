// SPDX-License-Identifier: GPL-3.0-or-later

use super::super::*;
use crate::model::EntryKind;
use crate::services::{
    DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
};
use crate::test_support::gtk_test;
use std::time::{Duration, Instant};

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

fn entry(name: &str, kind: EntryKind) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        thumbnail_path: None,
        kind,
        size: MetadataValue::Known(10),
        modified_unix_seconds: MetadataValue::Known(1),
        mode: MetadataValue::Known(0o100644),
        is_hidden: false,
    }
}

fn entries() -> Vec<FileEntry> {
    vec![
        entry("folder", EntryKind::Directory),
        entry("a.txt", EntryKind::File),
        entry("b.png", EntryKind::File),
    ]
}

fn pump_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "GTK state did not settle");
        context.iteration(false);
    }
}

/// A populated, on-screen Icons pane with three bound items.
fn populated_icons_pane() -> (ModeViews, Rc<Browser>, gtk::Window) {
    let browser = Browser::new(Rc::new(StaticSource));
    browser.navigate(Location::local("/fixture"));
    let mut views = ModeViews::new(
        &gtk::ScrolledWindow::new(),
        browser.clone(),
        Rc::new(Cell::new(true)),
    );
    views.prepare_mode(BrowserMode::Icons);
    views.show_mode(BrowserMode::Icons);
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&views.widget())
        .build();
    window.present();
    pump_until(|| {
        views
            .single_pane()
            .is_some_and(|pane| pane.section.view.is_mapped())
    });
    pump_until(|| {
        views
            .single_pane()
            .is_some_and(|pane| pane.section.bound_items.borrow().len() == entries().len())
    });
    pump_until(|| {
        views
            .single_pane()
            .is_some_and(|pane| pane.stack.width() > 0)
    });
    (views, browser, window)
}

#[test]
fn context_menu_target_resolves_the_matching_sections_trigger() {
    gtk_test(
        "ui::browser_modes::tests::context_menu::context_menu_target_resolves_the_matching_sections_trigger",
        || {
            let (views, browser, window) = populated_icons_pane();
            let pane = views.single_pane().expect("pane").clone();

            // Move one bound item into a second section (a grouped view has one
            // section per type group) to prove the resolver searches every section,
            // not just the pane's primary one.
            let moved = pane
                .section
                .bound_items
                .borrow_mut()
                .pop()
                .expect("a bound item to move into the second section");
            let target_position = moved.item.upgrade().expect("bound item").position() as usize;

            let second_trigger: Rc<dyn Fn(f64, f64)> = Rc::new(|_, _| {});
            let second_bound_items = Rc::new(RefCell::new(vec![moved]));
            let second_section = PaneSection {
                view: pane.section.view.clone(),
                view_model: pane.section.view_model.clone(),
                selection: pane.section.selection.clone(),
                bound_items: second_bound_items.clone(),
                syncing: Rc::new(Cell::new(false)),
                visit: bound_item_visitor(second_bound_items),
                item_context_trigger: second_trigger.clone(),
            };
            pane.sections.borrow_mut().push(second_section);

            let (trigger, _, _) = views
                .context_menu_target(pane.depth, Some(target_position))
                .expect("a trigger for the moved item");
            assert!(
                Rc::ptr_eq(&trigger, &second_trigger),
                "expected the second section's trigger, not the primary section's or the background's"
            );

            browser.clear_observer();
            window.close();
        },
    );
}

#[test]
fn context_menu_target_falls_back_to_the_background_trigger() {
    gtk_test(
        "ui::browser_modes::tests::context_menu::context_menu_target_falls_back_to_the_background_trigger",
        || {
            let (views, browser, window) = populated_icons_pane();
            let pane = views.single_pane().expect("pane").clone();

            let (trigger, _, _) = views
                .context_menu_target(pane.depth, None)
                .expect("a background trigger");
            assert!(Rc::ptr_eq(&trigger, &pane.folder_context_trigger));

            browser.clear_observer();
            window.close();
        },
    );
}
