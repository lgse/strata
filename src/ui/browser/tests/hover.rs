// SPDX-License-Identifier: MIT

use gtk::{gdk, glib, prelude::*};
use std::time::{Duration, Instant};

fn settle() {
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn background(window: &gtk::Window, row: &gtk::Widget) -> [u8; 4] {
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(window.width()),
        f64::from(window.height()),
    );
    let texture = window
        .renderer()
        .expect("renderer")
        .render_texture(snapshot.to_node().expect("snapshot"), None);
    let mut downloader = gdk::TextureDownloader::new(&texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let surface = row
        .first_child()
        .filter(|child| child.has_css_class("icons-card"))
        .and_then(|card| crate::ui::icons_cell::parts(&card))
        .and_then(|(icon, _)| icon.parent())
        .unwrap_or_else(|| row.clone());
    let bounds = surface.compute_bounds(window).expect("row bounds");
    let x = (bounds.x() + bounds.width() - 12.0) as usize;
    let y = bounds.center().y() as usize;
    bytes[y * stride + x * 4..y * stride + x * 4 + 4]
        .try_into()
        .expect("RGBA pixel")
}

#[test]
fn preselected_entries_keep_hover_feedback_in_all_modes() {
    crate::test_support::gtk_test(
        "ui::browser::tests::hover::preselected_entries_keep_hover_feedback_in_all_modes",
        || {
            crate::ui::prepare_portal_ui();
            for mode in ["Columns", "List", "Icons"] {
                let selection =
                    gtk::SingleSelection::new(Some(gtk::StringList::new(&["alpha", "beta"])));
                let factory = gtk::SignalListItemFactory::new();
                factory.connect_setup(move |_, item| {
                    let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
                    if mode == "Icons" {
                        let card = crate::ui::icons_cell::new_card(64);
                        let (_, label) = crate::ui::icons_cell::parts(&card).expect("icon card");
                        label.set_text(Some("sample"));
                        item.set_child(Some(&card));
                    } else {
                        let label = gtk::Label::new(Some("sample"));
                        label.set_height_request(26);
                        item.set_child(Some(&label));
                    }
                });
                let list: gtk::Widget = if mode == "Icons" {
                    let grid = gtk::GridView::new(Some(selection.clone()), Some(factory));
                    grid.add_css_class("file-icons");
                    grid.upcast()
                } else {
                    let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
                    list.add_css_class(if mode == "Columns" {
                        "file-list"
                    } else {
                        "file-list-mode"
                    });
                    list.upcast()
                };
                let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
                column.add_css_class("directory-column");
                column.append(&list);
                let outside = gtk::Button::with_label("Outside listing");
                let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
                root.append(&outside);
                root.append(&column);
                let window = gtk::Window::builder()
                    .default_width(300)
                    .default_height(150)
                    .child(&root)
                    .build();
                window.present();
                outside.grab_focus();
                settle();
                let first = list.first_child().expect("first row");
                let second = first.next_sibling().expect("second row");
                assert!(first.state_flags().contains(gtk::StateFlags::SELECTED));
                for focused in [false, true] {
                    root.remove_css_class("keyboard-navigation");
                    first.unset_state_flags(gtk::StateFlags::PRELIGHT);
                    second.unset_state_flags(gtk::StateFlags::PRELIGHT);
                    if focused {
                        list.grab_focus();
                    } else {
                        outside.grab_focus();
                    }
                    settle();
                    assert_eq!(
                        list.state_flags().contains(gtk::StateFlags::FOCUS_WITHIN),
                        focused
                    );
                    let normal = background(&window, &first);
                    first.set_state_flags(gtk::StateFlags::PRELIGHT, false);
                    second.set_state_flags(gtk::StateFlags::PRELIGHT, false);
                    settle();
                    let hover = background(&window, &first);
                    assert_ne!(
                        hover, normal,
                        "{mode}, focused={focused}: preselected first entry responds to hover"
                    );
                    assert_ne!(
                        hover,
                        background(&window, &second),
                        "hover preserves a distinct selected appearance"
                    );
                    root.add_css_class("keyboard-navigation");
                    settle();
                    assert_eq!(
                        background(&window, &first),
                        normal,
                        "{mode}: keyboard navigation suppresses parked-pointer hover"
                    );
                    assert_eq!(selection.selected(), 0, "hover must not change selection");
                }
                window.destroy();
            }
        },
    );
}

#[test]
fn application_stylesheet_has_no_parser_errors() {
    crate::test_support::gtk_test(
        "ui::browser::tests::hover::application_stylesheet_has_no_parser_errors",
        || {
            let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let captured = errors.clone();
            let provider = gtk::CssProvider::new();
            provider.connect_parsing_error(move |_, _, error| {
                captured.borrow_mut().push(error.to_string())
            });
            provider.load_from_string(include_str!("../../../style.css"));
            assert!(
                errors.borrow().is_empty(),
                "CSS parser errors: {:?}",
                errors.borrow()
            );
        },
    );
}
