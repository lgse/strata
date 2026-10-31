// SPDX-License-Identifier: MIT

use std::path::Path;

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

fn menu_labels(menu: &ActionMenuSection) -> Vec<String> {
    let mut items = Vec::new();
    if let Some(model) = menu.popover.menu_model() {
        collect_presentations(&model, &mut items);
    }
    items.into_iter().map(|item| item.label).collect()
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
            let devices = gio::Menu::new();
            devices.append(Some("Pixel"), Some("provider.action-0-pixel"));
            menu.send_to_provider_model
                .insert_section(0, Some("KDE Connect"), &devices);
            wait_until(|| has_visible_label(popover.upcast_ref(), "Send to…"));
            assert!(
                has_visible_label(popover.upcast_ref(), "Send to…"),
                "a provider destination shows Send to… without any drive"
            );
            let send_to = menu.send_to_model.clone();
            update(targets.clone());
            menu.schedule_removable_refresh();
            let drives_only = || {
                send_to.n_items() == 1 && send_to.item_attribute_value(0, "label", None).is_none()
            };
            wait_until(drives_only);
            assert_eq!(
                send_to.n_items(),
                1,
                "a rebuild withdraws stale provider destinations until they are re-sent"
            );
            assert!(has_visible_label(popover.upcast_ref(), "Send to…"));
            menu.send_to_provider_model
                .insert_section(0, Some("KDE Connect"), &devices);
            assert_eq!(
                send_to.n_items(),
                2,
                "drives first, then the provider section"
            );
            assert!(send_to.item_link(0, "section").is_some());
            assert_eq!(
                send_to
                    .item_attribute_value(1, "label", None)
                    .and_then(|value| value.get::<String>())
                    .as_deref(),
                Some("KDE Connect")
            );
            menu.send_to_provider_model.remove_all();
            assert_eq!(send_to.n_items(), 1, "withdrawing devices keeps the drives");
            assert!(has_visible_label(popover.upcast_ref(), "Send to…"));
            update(Vec::new());
            menu.schedule_removable_refresh();
            wait_until(|| !has_visible_label(popover.upcast_ref(), "Send to…"));
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

#[test]
fn send_to_offers_a_device_once_its_root_is_checked_off_the_ui_thread() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::actions::tests::send_to_offers_a_device_once_its_root_is_checked_off_the_ui_thread",
        || {
            let directory = tempfile::tempdir().expect("source fixture");
            std::fs::write(directory.path().join("note.txt"), b"source").expect("source file");
            let device = tempfile::tempdir().expect("device fixture");
            for remembered in ["Photos", "Damaged"] {
                std::fs::create_dir(device.path().join(remembered)).expect("remembered folder");
            }
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            for remembered in ["Gone", "Photos", "Damaged"] {
                preferences.remember_send_to_destination(
                    "volume:fixture-usb",
                    Path::new(remembered),
                    None,
                );
            }
            let parked = crate::test_support::parked::Parked::default();
            let recent = crate::test_support::parked::Parked::default();
            let _device = crate::adapters::FsQuirks::install(
                device.path(),
                crate::adapters::Quirks {
                    before: vec![
                        (crate::adapters::FsOp::Access, "*", 0, parked.racer()),
                        (
                            crate::adapters::FsOp::FolderStamp,
                            "Photos",
                            0,
                            recent.racer(),
                        ),
                    ],
                    fail: vec![
                        (
                            crate::adapters::FsOp::FolderStamp,
                            "Damaged",
                            0,
                            rustix::io::Errno::IO
                        );
                        20
                    ],
                    ..crate::adapters::Quirks::default()
                },
            );
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
            crate::ui::offer_removable_destinations_for_test(vec![
                crate::ui::RemovableDestination {
                    id: "volume:fixture-usb".into(),
                    name: "Fixture USB".into(),
                    root: device.path().to_owned(),
                },
            ]);
            menu.rebuild_for_selection(
                &view.state,
                std::slice::from_ref(&entry),
                Some(directory.path().to_owned()),
            );
            menu.show(&view.widget(), 400.0, 300.0);
            let popover = menu.popover();
            wait_until(|| popover.is_mapped() && parked.reached());
            assert!(
                !has_visible_label(popover.upcast_ref(), "Send to…"),
                "the menu opens while the device is being checked"
            );
            assert_eq!(parked.reached_on_ui_thread(), Some(false));
            parked.release();
            wait_until(|| has_visible_label(popover.upcast_ref(), "Send to…") && recent.reached());
            assert_eq!(
                recent.reached_on_ui_thread(),
                Some(false),
                "remembered folders are checked on the device's lane"
            );
            assert!(!menu_labels(&menu).contains(&"Photos (recent)".to_owned()));
            recent.release();
            wait_until(|| menu_labels(&menu).contains(&"Photos (recent)".to_owned()));
            for (unoffered, why) in [
                (
                    "Gone (recent)",
                    "a remembered folder that no longer exists is not offered",
                ),
                (
                    "Damaged (recent)",
                    "a remembered folder that keeps failing is not offered",
                ),
            ] {
                assert!(!menu_labels(&menu).contains(&unoffered.to_owned()), "{why}");
            }
            popover.popdown();
            view.browser().clear_observer();
            window.destroy();
        },
    );
}
