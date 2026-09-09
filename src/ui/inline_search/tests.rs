// SPDX-License-Identifier: MIT

use super::*;
use crate::model::Location;
use std::{
    fs,
    time::{Instant, SystemTime},
};

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
            let widget = wrap(&original, &entry, Some(root.clone()), &browser).widget;
            let stack = widget
                .clone()
                .downcast::<gtk::Stack>()
                .expect("local search stack");
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
            fs::remove_dir_all(fixture).expect("remove fixture");
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
            let _theme = super::super::theme::ThemeManager::shared();
            super::super::thumbnail::hold_thumbnail_workers();
            window.present();
            entry.set_text("same");
            entry.grab_focus();
            wait_until(|| state.items.borrow().len() == 3);
            let mut items = state.items.borrow().clone();
            items.sort_by(|a, b| a.path.cmp(&b.path));
            wait_until(|| {
                items
                    .iter()
                    .all(|item| super::super::thumbnail::pending_thumbnail_id(&item.path).is_some())
            });
            // Stop the real worker before injecting deterministic progressive publications.
            state.handle.borrow_mut().take();
            update_rows(state, vec![items[1].clone()], fixture.path(), true);
            wait_until(|| super::super::thumbnail::pending_thumbnail_id(&items[1].path).is_some());
            let pending = super::super::thumbnail::pending_thumbnail_id(&items[1].path);
            let selected = state.list.row_at_index(0).expect("selected row");
            state.list.select_row(Some(&selected));
            let focus = gtk::prelude::GtkWindowExt::focus(&window);
            assert!(
                focus
                    .as_ref()
                    .is_some_and(|focused| focused.is_ancestor(&entry))
            );
            let icon = selected
                .child()
                .and_then(|line| line.first_child())
                .and_downcast::<super::super::thumbnail::ThumbnailSlot>()
                .expect("thumbnail slot");
            let bytes = glib::Bytes::from_static(&[255, 0, 0, 255]);
            let texture =
                gtk::gdk::MemoryTexture::new(1, 1, gtk::gdk::MemoryFormat::R8g8b8a8, &bytes, 4);
            icon.set_texture(texture.upcast_ref());
            let resizes = icon.resize_calls();
            let removed = Rc::new(Cell::new(0));
            let removed_for_notify = removed.clone();
            selected.connect_parent_notify(move |row| {
                if row.parent().is_none() {
                    removed_for_notify.set(removed_for_notify.get() + 1);
                }
            });
            for update in [
                vec![items[1].clone(), items[2].clone()],
                items.clone(),
                items.clone(),
            ] {
                update_rows(state, update, fixture.path(), true);
                assert_eq!(state.list.selected_row(), Some(selected.clone()));
                assert_eq!(
                    search.selected_entry().expect("selected entry").location,
                    Location::local(&items[1].path)
                );
                assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), focus);
                assert_eq!(icon.texture(), Some(texture.clone().upcast()));
                assert_eq!(icon.resize_calls(), resizes);
                assert_eq!(
                    super::super::thumbnail::pending_thumbnail_id(&items[1].path),
                    pending
                );
                assert_eq!(entry.text(), "same");
            }
            assert_eq!(
                removed.get(),
                0,
                "appending and inserting must retain existing rows"
            );
            update_rows(
                state,
                vec![items[2].clone(), items[1].clone(), items[0].clone()],
                fixture.path(),
                false,
            );
            assert_eq!(state.list.selected_row(), Some(selected.clone()));
            assert_eq!(
                search
                    .selected_entry()
                    .expect("reordered selection")
                    .location,
                Location::local(&items[1].path)
            );
            assert_eq!(
                removed.get(),
                0,
                "reordering must keep retained rows rooted"
            );
            assert_eq!(icon.texture(), Some(texture.upcast()));
            assert_eq!(
                super::super::thumbnail::pending_thumbnail_id(&items[1].path),
                pending
            );
            assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), focus);
            assert!(search.is_item_target(icon.upcast_ref()));
            assert!(!search.is_item_target(state.list.upcast_ref()));
            assert!(!search.is_item_target(state.status.upcast_ref()));
            update_rows(
                state,
                vec![items[2].clone(), items[0].clone()],
                fixture.path(),
                true,
            );
            assert_eq!(
                search.selected_entry().expect("next slot").location,
                Location::local(&items[0].path)
            );
            update_rows(state, vec![items[2].clone()], fixture.path(), true);
            assert_eq!(
                search.selected_entry().expect("last slot").location,
                Location::local(&items[2].path)
            );
            update_rows(state, vec![], fixture.path(), true);
            assert!(search.selected_entry().is_none());
            assert!(state.list.first_child().is_none());
            assert!(super::super::thumbnail::pending_thumbnail_id(&items[1].path).is_none());
            super::super::thumbnail::clear_thumbnail_runtime();
            window.close();
        },
    );
}
