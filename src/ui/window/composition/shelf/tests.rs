// SPDX-License-Identifier: MIT

use super::{Shelf, ShelfView};
use crate::model::Location;
use gtk::{gdk, gio, glib, prelude::*};
use std::{cell::RefCell, rc::Rc};

#[test]
fn collecting_files_preserves_sources_and_ignores_duplicate_drops()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let first_path = dir.path().join("first.txt");
    let second_path = dir.path().join("second.txt");
    std::fs::write(&first_path, "first")?;
    std::fs::write(&second_path, "second")?;
    let first = Location::local(&first_path);
    let second = Location::local(&second_path);
    let equivalent_first = Location::uri(gio::File::for_path(&first_path).uri().to_string());
    let shelf = Shelf::default();

    assert!(shelf.add([first.clone(), second.clone(), first.clone()]));
    assert!(shelf.add([equivalent_first.clone(), second.clone()]));
    assert_eq!(shelf.locations(), [first.clone(), second.clone()]);
    assert_eq!(std::fs::read_to_string(&first_path)?, "first");
    assert_eq!(std::fs::read_to_string(&second_path)?, "second");
    let link_path = dir.path().join("first-link.txt");
    std::os::unix::fs::symlink(&first_path, &link_path)?;
    let link = Location::local(&link_path);
    assert!(shelf.add([link.clone()]));
    assert_eq!(shelf.locations(), [first, second.clone(), link.clone()]);

    shelf.remove(&equivalent_first);
    assert_eq!(shelf.locations(), [second, link]);
    shelf.clear();
    assert!(shelf.locations().is_empty());
    assert!(first_path.exists());
    assert!(second_path.exists());
    assert_eq!(std::fs::read_to_string(&link_path)?, "first");
    Ok(())
}

fn drop_target(widget: &gtk::Widget) -> Option<gtk::DropTarget> {
    let controllers = widget.observe_controllers();
    (0..controllers.n_items())
        .find_map(|index| controllers.item(index).and_downcast::<gtk::DropTarget>())
}

fn find_widget(
    widget: &gtk::Widget,
    matches: &dyn Fn(&gtk::Widget) -> bool,
) -> Option<gtk::Widget> {
    if matches(widget) {
        return Some(widget.clone());
    }
    if let Some(menu) = widget.downcast_ref::<gtk::MenuButton>()
        && let Some(popover) = menu.popover()
        && let Some(found) = find_widget(popover.upcast_ref(), matches)
    {
        return Some(found);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = find_widget(&widget, matches) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn file_drop_controller_stages_native_and_uri_lists_without_changing_sources() {
    crate::test_support::gtk_test(
        "ui::window::composition::shelf::tests::file_drop_controller_stages_native_and_uri_lists_without_changing_sources",
        || {
            let dir = tempfile::tempdir().expect("shelf drop fixture");
            let first_path = dir.path().join("first.txt");
            let second_path = dir.path().join("second with spaces.txt");
            std::fs::write(&first_path, b"first").expect("first source file");
            std::fs::write(&second_path, b"second").expect("second source file");
            let shelf = Shelf::shared();
            let view = ShelfView::new(Rc::new(RefCell::new(None)));
            let target = drop_target(view.widget.upcast_ref()).expect("shelf file drop controller");
            let viewer = find_widget(view.widget.upcast_ref(), &|widget| {
                widget.is::<gtk::Popover>() && drop_target(widget).is_some()
            })
            .expect("shelf item viewer");
            let viewer_target = drop_target(&viewer).expect("item viewer file drop controller");
            let native = gdk::FileList::from_array(&[gio::File::for_path(&first_path)]);
            let native = glib::BoxedValue(native.to_value());
            assert!(target.emit_by_name::<bool>("drop", &[&native, &0.0f64, &0.0f64]));
            assert_eq!(shelf.locations(), [Location::local(&first_path)]);
            let mixed = gdk::FileList::from_array(&[
                gio::File::for_uri("recent:///item"),
                gio::File::for_path(&first_path),
            ]);
            let mixed = glib::BoxedValue(mixed.to_value());
            assert!(target.emit_by_name::<bool>("drop", &[&mixed, &0.0f64, &0.0f64]));
            assert_eq!(shelf.locations(), [Location::local(&first_path)]);

            let uri_list = format!(
                "# URI list\r\n{}\r\n{}\r\n",
                gio::File::for_path(&first_path).uri(),
                gio::File::for_path(&second_path).uri(),
            );
            let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(uri_list));
            let converted = glib::MainContext::default()
                .block_on(gdk::content_deserialize_future(
                    &stream,
                    "text/uri-list",
                    gdk::FileList::static_type(),
                    glib::Priority::DEFAULT,
                ))
                .expect("deserialize URI list");
            let converted = glib::BoxedValue(converted);
            assert!(viewer_target.emit_by_name::<bool>("drop", &[&converted, &0.0f64, &0.0f64]));
            let expected = [Location::local(&first_path), Location::local(&second_path)];
            assert_eq!(shelf.locations(), expected);
            for invalid in [
                "not a file list".to_value(),
                Option::<gdk::FileList>::None.to_value(),
                gdk::FileList::from_array(&[gio::File::for_uri("recent:///item")]).to_value(),
            ] {
                let invalid = glib::BoxedValue(invalid);
                assert!(!target.emit_by_name::<bool>("drop", &[&invalid, &0.0f64, &0.0f64]));
                assert_eq!(shelf.locations(), expected);
            }
            let clear = find_widget(view.widget.upcast_ref(), &|widget| {
                widget
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|label| label.text() == "Clear shelf")
            })
            .expect("clear shelf action");
            clear
                .ancestor(gtk::Button::static_type())
                .expect("clear shelf button")
                .emit_by_name::<()>("clicked", &[]);
            assert!(shelf.locations().is_empty());
            assert_eq!(
                std::fs::read(&first_path).expect("first source bytes"),
                b"first"
            );
            assert_eq!(
                std::fs::read(&second_path).expect("second source bytes"),
                b"second"
            );
        },
    );
}

#[test]
fn collecting_does_not_accept_virtual_recent_entries() {
    let shelf = Shelf::default();
    assert!(!shelf.add([Location::uri("recent:///item")]));
    assert!(shelf.locations().is_empty());
}
