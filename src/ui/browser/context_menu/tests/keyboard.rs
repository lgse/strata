// SPDX-License-Identifier: MIT

use super::super::context_menu_popover;
use super::menus::{MenuSource, descendants, label, open_menu, wait_until};
use crate::model::Location;
use crate::ui::browser::{BrowserView, PeekBehavior};
use crate::ui::browser_modes::BrowserMode;
use gtk::{gdk::Key, prelude::*};
use std::{cell::RefCell, rc::Rc};

fn menu(window: &gtk::Window) -> gtk::Popover {
    let popup = descendants(window.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Popover>().ok())
        .find(|popover| popover.is_mapped())
        .expect("mapped context menu");
    wait_until(|| {
        descendants(popup.upcast_ref())
            .iter()
            .any(|widget| widget.is::<gtk::Button>() && widget.is_mapped() && widget.width() > 0)
    });
    popup
}

fn press(popover: &gtk::Popover, key: Key) {
    let controllers = popover.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .find(|keys| keys.propagation_phase() == gtk::PropagationPhase::Capture)
        .expect("menu keyboard controller");
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
    ));
}

fn position(view: &BrowserView, name: &str) -> usize {
    (0..100)
        .find(|position| {
            view.browser()
                .entry_at(0, *position)
                .is_some_and(|entry| entry.display_name == name)
        })
        .expect("source entry")
}

fn native_selection_count(view: &BrowserView) -> u64 {
    descendants(&view.widget())
        .into_iter()
        .filter(|widget| widget.is_mapped())
        .filter_map(|widget| {
            widget
                .downcast_ref::<gtk::ListView>()
                .and_then(|view| view.model())
                .or_else(|| {
                    widget
                        .downcast_ref::<gtk::GridView>()
                        .and_then(|view| view.model())
                })
        })
        .map(|selection| selection.selection().size())
        .sum()
}

#[test]
fn context_menus_preserve_filtered_grouped_and_chooser_selections() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::context_menus_preserve_filtered_grouped_and_chooser_selections",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                for chooser in [false, true] {
                    for grouped in [false, true]
                        .into_iter()
                        .filter(|grouped| !grouped || mode != BrowserMode::Columns)
                    {
                        let root = tempfile::tempdir().expect("fixture");
                        for name in [".a-hidden", "notes.txt", "picture.png"] {
                            std::fs::write(root.path().join(name), "fixture").expect("file");
                        }
                        std::fs::create_dir(root.path().join("folder")).expect("folder");
                        let source = Rc::new(crate::adapters::LocalFileSource);
                        let view = if chooser {
                            BrowserView::new_chooser(source, true)
                        } else {
                            BrowserView::new(source, PeekBehavior::default())
                        };
                        view.set_view_mode(mode);
                        view.set_group_by_type(grouped);
                        let window = gtk::Window::builder()
                            .child(&view.widget())
                            .default_width(1000)
                            .default_height(700)
                            .build();
                        window.present();
                        view.browser().navigate(Location::local(root.path()));
                        if view.browser().preferences().show_hidden {
                            view.browser().toggle_hidden();
                        }
                        wait_until(|| label(&view.widget(), "picture.png").is_some());
                        let notes = position(&view, "notes.txt");
                        let picture = position(&view, "picture.png");
                        view.browser()
                            .set_selection(0, &[notes, picture], Some(picture));
                        view.browser().focus_active();
                        wait_until(|| view.item_view_has_focus());
                        let selected_locations = || {
                            view.browser()
                                .selected_entries()
                                .into_iter()
                                .map(|entry| entry.location)
                                .collect::<Vec<_>>()
                        };
                        let before = selected_locations();
                        assert_eq!(native_selection_count(&view), 2);
                        let origin = gtk::prelude::RootExt::focus(&window);
                        assert!(
                            view.open_focused_context_menu(),
                            "{mode:?} chooser={chooser} grouped={grouped}"
                        );
                        let popup = menu(&window);
                        assert!(
                            label(
                                popup.upcast_ref(),
                                if chooser { "Properties" } else { "Copy" }
                            )
                            .is_some(),
                            "{mode:?} chooser={chooser} grouped={grouped}"
                        );
                        assert!(label(popup.upcast_ref(), "New Folder").is_none());
                        if !chooser {
                            assert!(label(popup.upcast_ref(), "2 items selected").is_some());
                        }
                        press(&popup, Key::Escape);
                        wait_until(|| popup.parent().is_none());
                        assert_eq!(selected_locations(), before);
                        assert_eq!(native_selection_count(&view), 2);
                        assert_eq!(gtk::prelude::RootExt::focus(&window), origin);

                        let popup = open_menu(&view, Some("notes.txt"));
                        press(&popup, Key::Escape);
                        wait_until(|| popup.parent().is_none());
                        assert_eq!(selected_locations(), before);
                        assert_eq!(native_selection_count(&view), 2);
                        assert_eq!(view.browser().focused_item().expect("cursor").1, notes);
                        wait_until(|| {
                            let focused = gtk::prelude::RootExt::focus(&window);
                            label(&view.widget(), "notes.txt").is_some_and(|label| {
                                focused.is_some_and(|focused| label.is_ancestor(&focused))
                            })
                        });

                        view.browser().clear_active_selection();
                        assert!(view.open_focused_context_menu());
                        let popup = menu(&window);
                        assert!(label(popup.upcast_ref(), "New Folder").is_some());
                        press(&popup, Key::Escape);
                        wait_until(|| popup.parent().is_none());
                        assert!(view.browser().selected_entries().is_empty());
                        assert_eq!(native_selection_count(&view), 0);
                        view.browser().clear_observer();
                        window.destroy();
                    }
                }
            }
        },
    );
}

#[test]
fn keyboard_trash_menu_targets_the_selection_in_every_view() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::keyboard_trash_menu_targets_the_selection_in_every_view",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                let view = BrowserView::new(Rc::new(MenuSource), PeekBehavior::default());
                view.set_view_mode(mode);
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(1000)
                    .default_height(700)
                    .build();
                window.present();
                view.browser().navigate(Location::uri("trash:///"));
                wait_until(|| label(&view.widget(), "notes.txt").is_some());
                view.browser().select(0, position(&view, "notes.txt"));
                view.browser().focus_active();
                assert!(view.open_focused_context_menu());
                let popup = menu(&window);
                assert!(label(popup.upcast_ref(), "Restore").is_some(), "{mode:?}");
                assert!(label(popup.upcast_ref(), "notes.txt").is_some());
                press(&popup, Key::Escape);
                wait_until(|| popup.parent().is_none());
                assert_eq!(
                    view.browser().selected_entries()[0].display_name,
                    "notes.txt"
                );
                view.browser().clear_observer();
                window.destroy();
            }
        },
    );
}

#[test]
fn menu_keys_skip_inactive_actions_wrap_scroll_and_activate() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::keyboard::menu_keys_skip_inactive_actions_wrap_scroll_and_activate",
        || {
            let content = crate::ui::accessibility::menu_box();
            let nested = gtk::Box::new(gtk::Orientation::Vertical, 0);
            content.append(&gtk::Label::new(Some("Header")));
            content.append(&nested);
            let activated = Rc::new(RefCell::new(Vec::new()));
            let buttons: Vec<_> = (0..30)
                .map(|index| {
                    let button = gtk::Button::with_label(&format!("Action {index}"));
                    let activated = activated.clone();
                    button.connect_clicked(move |_| activated.borrow_mut().push(index));
                    nested.append(&button);
                    button
                })
                .collect();
            buttons[1].set_sensitive(false);
            buttons[2].set_visible(false);
            nested.insert_child_after(
                &gtk::Separator::new(gtk::Orientation::Horizontal),
                Some(&buttons[0]),
            );
            let (popup, scroll) = context_menu_popover(&content);
            scroll.set_max_content_height(120);
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let window = gtk::Window::builder()
                .child(&anchor)
                .default_width(400)
                .default_height(300)
                .build();
            popup.set_parent(&anchor);
            window.present();
            popup.popup();
            wait_until(|| {
                popup.is_mapped() && scroll.vadjustment().upper() > scroll.vadjustment().page_size()
            });
            for (key, index) in [
                (Key::Home, 0),
                (Key::Down, 3),
                (Key::Up, 0),
                (Key::Up, 29),
                (Key::Down, 0),
                (Key::End, 29),
                (Key::Tab, 0),
                (Key::ISO_Left_Tab, 29),
            ] {
                window.set_focus_visible(false);
                press(&popup, key);
                assert!(
                    buttons[index]
                        .state_flags()
                        .contains(gtk::StateFlags::FOCUSED | gtk::StateFlags::FOCUS_VISIBLE),
                    "{key:?} must visibly focus Action {index} after the focus indicator expires"
                );
                wait_until(|| {
                    let bounds = buttons[index]
                        .compute_bounds(&scroll)
                        .expect("button bounds");
                    bounds.y() >= -0.5
                        && bounds.y() + bounds.height() <= scroll.height() as f32 + 0.5
                });
            }
            for (index, key) in [Key::Return, Key::KP_Enter, Key::space]
                .into_iter()
                .enumerate()
            {
                press(&popup, key);
                assert_eq!(
                    *activated.borrow(),
                    vec![29; index + 1],
                    "{key:?} must dispatch the focused action before key handling returns"
                );
            }
            press(&popup, Key::Escape);
            wait_until(|| !popup.is_visible());
            popup.unparent();
            window.destroy();
        },
    );
}
