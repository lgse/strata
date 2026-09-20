// SPDX-License-Identifier: MIT

use super::*;
use crate::model::Location;
use std::{
    fs,
    time::{Instant, SystemTime},
};

#[test]
fn search_presence_preserves_dangling_symlinks_but_not_removed_entries() {
    let fixture = tempfile::tempdir().expect("fixture");
    let target = fixture.path().join("target");
    let link = fixture.path().join("link");
    fs::write(&target, b"body").expect("target");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    assert!(search_path_present(&target));
    assert!(search_path_present(&link));
    fs::remove_file(&target).expect("remove target");
    assert!(!search_path_present(&target));
    assert!(search_path_present(&link));
    fs::remove_file(&link).expect("remove link");
    assert!(!search_path_present(&link));
}

fn labels(widget: &gtk::Widget) -> Vec<String> {
    let mut result = Vec::new();
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        result.push(label.text().to_string());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        result.extend(labels(&widget));
    }
    result
}

fn search_options(
    browser: &Rc<Browser>,
    presentation: SearchPresentation,
) -> SearchCollectionOptions {
    let weak = Rc::downgrade(browser);
    let activate = Rc::new(move |entry: FileEntry| {
        if let Some(browser) = weak.upgrade() {
            if entry.is_directory() {
                browser.navigate(entry.location);
            } else {
                browser.open_location(entry.location);
            }
        }
    });
    SearchCollectionOptions {
        presentation,
        multiple_selection: Rc::new(Cell::new(true)),
        activate: activate.clone(),
        single_click: activate,
        selection_changed: Rc::new(|_| {}),
        focus_items: Rc::new(|| {}),
    }
}

fn wait_until(condition: impl Fn() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "search did not update"
        );
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn result_paths_are_relative_to_the_search_root() {
    let root = Path::new("/tmp/strata-search-demo");

    assert_eq!(
        relative_result_path(root, Path::new("/tmp/strata-search-demo/Videos/clip.mp4")),
        "Videos/clip.mp4"
    );
    assert_eq!(
        relative_result_path(root, Path::new("/tmp/strata-search-demo/notes.txt")),
        "notes.txt"
    );
}

#[test]
fn pointer_selection_extends_and_toggles_the_plain_click_selection() {
    crate::test_support::gtk_test(
        "ui::inline_search::tests::pointer_selection_extends_and_toggles_the_plain_click_selection",
        || {
            let first = pointer_selection(None, 0, false, false, None, true);
            assert!(first.contains(0));

            let both = pointer_selection(Some(&first), 1, true, false, Some(0), true);
            assert!(both.contains(0));
            assert!(both.contains(1));

            let second = pointer_selection(Some(&both), 0, true, false, Some(1), true);
            assert!(!second.contains(0));
            assert!(second.contains(1));

            let empty = pointer_selection(Some(&first), 0, true, false, Some(0), true);
            assert!(empty.is_empty());
        },
    );
}

#[test]
fn paths_outside_the_search_root_remain_unchanged() {
    assert_eq!(
        relative_result_path(
            Path::new("/tmp/search-root"),
            Path::new("/tmp/other/file.txt")
        ),
        "/tmp/other/file.txt"
    );
}

#[test]
fn alternate_view_search_finds_descendants_and_restores_the_original_view() {
    crate::test_support::gtk_test(
        "ui::inline_search::tests::alternate_view_search_finds_descendants_and_restores_the_original_view",
        || {
            let id = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let fixture = std::env::temp_dir().join(format!("strata-alternate-search-{id}"));
            let root = fixture.join("Documents");
            fs::create_dir_all(root.join("github/strata")).expect("create nested directory");
            fs::create_dir_all(fixture.join("outside-strata")).expect("create sibling directory");
            fs::write(root.join("github/readme.txt"), "fixture").expect("create second match");
            let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
            let entry = gtk::Entry::new();
            let original = gtk::Label::new(Some("Original view"));
            let widget = wrap(
                &original,
                &entry,
                Some(root.clone()),
                &browser,
                search_options(&browser, SearchPresentation::Rows),
            )
            .widget;
            let stack = widget
                .clone()
                .downcast::<gtk::Stack>()
                .expect("local search stack");
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            content.append(&entry);
            content.append(&widget);
            let window = gtk::Window::builder()
                .child(&content)
                .default_width(640)
                .default_height(480)
                .build();
            window.present();
            entry.set_text("stra");
            wait_until(|| labels(&widget).contains(&"github/strata".to_owned()));
            assert!(
                !labels(&widget)
                    .iter()
                    .any(|text| text.contains("outside-strata"))
            );
            entry.set_text("readme");
            wait_until(|| labels(&widget).contains(&"github/readme.txt".to_owned()));
            entry.set_text("");
            wait_until(|| stack.visible_child_name().as_deref() == Some("files"));
            assert_eq!(stack.visible_child(), Some(original.upcast()));
            entry.set_text("stra");
            wait_until(|| labels(&widget).contains(&"github/strata".to_owned()));
            let controllers = entry.observe_controllers();
            let keys = (0..controllers.n_items())
                .filter_map(|index| controllers.item(index))
                .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
                .expect("recursive search key controller");
            assert_eq!(keys.propagation_phase(), gtk::PropagationPhase::Capture);
            assert!(keys.emit_by_name::<bool>(
                "key-pressed",
                &[
                    &gtk::gdk::Key::Return,
                    &0u32,
                    &gtk::gdk::ModifierType::empty()
                ],
            ));
            assert_eq!(
                browser.active_location(),
                Some(Location::local(root.join("github/strata")))
            );
            entry.set_text("");
            wait_until(|| stack.visible_child_name().as_deref() == Some("files"));
            window.destroy();
            fs::remove_dir_all(fixture).expect("remove fixture");
        },
    );
}

#[test]
fn search_collection_exposes_selection_events_and_consumer_selection_mode() {
    crate::test_support::gtk_test(
        "ui::inline_search::tests::search_collection_exposes_selection_events_and_consumer_selection_mode",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            let first = SearchItem::for_test(fixture.path().join("first.txt"), false);
            let second = SearchItem::for_test(fixture.path().join("second.txt"), false);
            let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
            let multiple = Rc::new(Cell::new(false));
            let changes = Rc::new(Cell::new(0));
            let changes_for_callback = changes.clone();
            let options = SearchCollectionOptions {
                presentation: SearchPresentation::Rows,
                multiple_selection: multiple.clone(),
                activate: Rc::new(|_| {}),
                single_click: Rc::new(|_| {}),
                selection_changed: Rc::new(move |_| {
                    changes_for_callback.set(changes_for_callback.get() + 1);
                }),
                focus_items: Rc::new(|| {}),
            };
            let search = wrap(
                &gtk::Label::new(None),
                &gtk::Entry::new(),
                Some(fixture.path().into()),
                &browser,
                options,
            );
            let state = search.state.as_ref().expect("state");
            update_results(state, vec![first, second], true);
            state.collection.selection.select_item(0, true);
            state.collection.selection.select_item(1, false);
            assert_eq!(state.collection.selection.selection().size(), 1);
            assert!(state.collection.selection.is_selected(1));
            multiple.set(true);
            state.collection.selection.select_item(0, false);
            assert_eq!(state.collection.selection.selection().size(), 2);
            assert!(changes.get() >= 3);
        },
    );
}

#[test]
fn every_search_presentation_renames_the_displayed_result_inline() {
    crate::test_support::gtk_test(
        "ui::inline_search::tests::every_search_presentation_renames_the_displayed_result_inline",
        || {
            for presentation in [
                SearchPresentation::Rows,
                SearchPresentation::Icons {
                    thumbnail_size: Rc::new(Cell::new(64)),
                    max_columns: 20,
                },
            ] {
                let fixture = tempfile::tempdir().expect("fixture");
                let path = fixture.path().join("note.txt");
                fs::write(&path, "note").expect("file");
                let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
                let entry = gtk::Entry::new();
                let search = wrap(
                    &gtk::Label::new(None),
                    &entry,
                    Some(fixture.path().into()),
                    &browser,
                    search_options(&browser, presentation),
                );
                let state = search.state.as_ref().expect("state");
                let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
                content.append(&entry);
                content.append(&search.widget);
                let window = gtk::Window::builder().child(&content).build();
                window.present();
                let item = SearchItem::for_test(path.clone(), false);
                update_results(state, vec![item.clone()], true);
                state.stack.set_visible_child_name("search");
                wait_until(|| state.collection.bound_at(0).is_some());
                let active = Rc::new(RefCell::new(None));
                assert!(search.begin_rename(
                    &super::super::browser::search_result_entry(&item),
                    active.clone(),
                    Rc::downgrade(&browser),
                    std::rc::Weak::new(),
                ));
                assert!(active.borrow().is_some());
                let (field, display) = state.collection.rename_widgets(0).expect("editor");
                assert!(gtk::prelude::WidgetExt::is_visible(&field));
                assert!(!display.is_visible());
                window.destroy();
            }
        },
    );
}

#[test]
fn progressive_results_retain_identity_focus_and_thumbnail() {
    crate::test_support::gtk_test(
        "ui::inline_search::tests::progressive_results_retain_identity_focus_and_thumbnail",
        || {
            let fixture = tempfile::tempdir().expect("fixture");
            for directory in ["alpha", "beta", "gamma"] {
                fs::create_dir(fixture.path().join(directory)).expect("directory");
                fs::write(fixture.path().join(directory).join("same.png"), directory)
                    .expect("duplicate name");
            }
            let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
            let entry = gtk::Entry::new();
            let search = wrap(
                &gtk::Label::new(None),
                &entry,
                Some(fixture.path().into()),
                &browser,
                search_options(&browser, SearchPresentation::Rows),
            );
            let state = search.state.as_ref().expect("search state");
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            content.append(&entry);
            content.append(&search.widget);
            let window = gtk::Window::builder()
                .child(&content)
                .default_width(480)
                .default_height(360)
                .build();
            let _theme = super::super::preferences::PreferenceManager::shared();
            super::super::thumbnail::hold_thumbnail_workers();
            window.present();
            entry.set_text("same");
            entry.grab_focus();
            wait_until(|| state.items.borrow().len() == 3);
            let mut items = state.items.borrow().clone();
            items.sort_by(|a, b| a.path.cmp(&b.path));
            state.handle.borrow_mut().take();
            update_results(state, vec![items[1].clone()], true);
            wait_until(|| state.collection.bound_at(0).is_some());
            state.collection.selection.select_item(0, true);
            let (_, selected) = state.collection.bound_at(0).expect("selected result");
            let icon = selected
                .first_child()
                .and_downcast::<super::super::thumbnail::ThumbnailSlot>()
                .expect("thumbnail slot");
            let bytes = glib::Bytes::from_static(&[255, 0, 0, 255]);
            let texture =
                gtk::gdk::MemoryTexture::new(1, 1, gtk::gdk::MemoryFormat::R8g8b8a8, &bytes, 4);
            icon.set_texture(texture.upcast_ref());
            let focus = gtk::prelude::GtkWindowExt::focus(&window);

            for update in [
                vec![items[1].clone(), items[2].clone()],
                items.clone(),
                vec![items[2].clone(), items[1].clone(), items[0].clone()],
            ] {
                update_results(state, update, true);
                let position = state
                    .items
                    .borrow()
                    .iter()
                    .position(|item| item.path == items[1].path)
                    .expect("retained result") as u32;
                let (_, widget) = state
                    .collection
                    .bound_at(position)
                    .expect("retained widget");
                assert_eq!(widget, selected);
                assert_eq!(icon.texture(), Some(texture.clone().upcast()));
                assert_eq!(
                    search.selected_entries().expect("results")[0].location,
                    Location::local(&items[1].path)
                );
                assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), focus);
            }
            assert!(search.is_item_target(icon.upcast_ref()));
            assert!(!search.is_item_target(&state.collection.view));
            assert!(!search.is_item_target(state.status.upcast_ref()));

            update_results(state, vec![items[2].clone(), items[0].clone()], true);
            assert_eq!(
                search.selected_entries().expect("results")[0].location,
                Location::local(&items[0].path)
            );
            update_results(state, vec![], true);
            assert!(search.selected_entry().is_none());
            assert_eq!(state.collection.model.n_items(), 0);
            super::super::thumbnail::clear_thumbnail_runtime();
            window.close();
        },
    );
}
