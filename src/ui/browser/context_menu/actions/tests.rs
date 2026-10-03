// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser::context_menu::tests::wait_until;
use crate::ui::browser::{BrowserView, PeekBehavior, SendToMenuTestOverride};

fn has_visible_label(widget: &gtk::Widget, text: &str) -> bool {
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.is_visible()
        && label.text() == text
    {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if has_visible_label(&widget, text) {
            return true;
        }
    }
    false
}

#[test]
fn send_to_updates_an_open_menu_and_reopens_without_navigation() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::actions::tests::send_to_updates_an_open_menu_and_reopens_without_navigation",
        || {
            let directory = tempfile::tempdir().expect("source fixture");
            std::fs::write(directory.path().join("note.txt"), b"source").expect("source file");
            let destination = tempfile::tempdir().expect("destination fixture");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(1000)
                .default_height(700)
                .build();
            window.present();
            view.navigate_location(Location::local(directory.path()));
            wait_until(|| view.browser().entry_at(0, 0).is_some());
            let entry = view.browser().entry_at(0, 0).expect("source entry");
            let before = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let after = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let single = gtk::Button::with_label("Copy to…");
            let multiple = gtk::Button::with_label("Copy selected to…");
            after.append(&single);
            after.append(&multiple);
            let menu = ActionMenuSection::new(
                &before,
                &after,
                None,
                view.state.overlay.upcast_ref(),
                Some([single, multiple]),
            );
            let handlers = super::super::super::SendToMenuHandlers {
                activate_root: Rc::new(|_, _| {}),
                activate_recent: Rc::new(|_, _, _| {}),
                choose_folder: Rc::new(|_, _| {}),
            };
            let update = |destinations| {
                view.state
                    .send_to_menu_test_override
                    .replace(Some(SendToMenuTestOverride {
                        destinations,
                        recent_destinations: HashMap::new(),
                        handlers: handlers.clone(),
                    }));
            };
            update(Vec::new());
            menu.rebuild_for_selection(
                &view.state,
                std::slice::from_ref(&entry),
                Some(directory.path().to_owned()),
            );
            menu.show(&view.widget(), 400.0, 300.0);
            let popover = menu.popover();
            wait_until(|| popover.is_mapped());
            assert!(!has_visible_label(popover.upcast_ref(), "Send to…"));
            let targets = vec![crate::ui::RemovableDestination {
                id: "volume:fixture-usb".into(),
                name: "Fixture USB".into(),
                root: destination.path().to_owned(),
            }];
            update(targets.clone());
            menu.schedule_removable_refresh();
            wait_until(|| has_visible_label(popover.upcast_ref(), "Send to…"));
            assert!(
                has_visible_label(popover.upcast_ref(), "Send to…"),
                "hot-plug updates the already open menu"
            );
            update(Vec::new());
            menu.schedule_removable_refresh();
            wait_until(|| !has_visible_label(popover.upcast_ref(), "Send to…"));
            assert!(
                !has_visible_label(popover.upcast_ref(), "Send to…"),
                "unplug removes the action without closing the menu"
            );
            popover.popdown();
            wait_until(|| !popover.is_mapped());
            update(targets);
            menu.rebuild_for_selection(
                &view.state,
                std::slice::from_ref(&entry),
                Some(directory.path().to_owned()),
            );
            menu.show(&view.widget(), 400.0, 300.0);
            wait_until(|| popover.is_mapped());
            assert!(
                has_visible_label(popover.upcast_ref(), "Send to…"),
                "the same cached menu sees the new drive on reopening"
            );
            assert_eq!(
                view.browser().active_location(),
                Some(Location::local(directory.path()))
            );
            let weak = Rc::downgrade(&menu);
            drop(menu);
            assert!(
                weak.upgrade().is_none(),
                "monitor subscriptions do not retain closed menus"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}
