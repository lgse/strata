// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::model::Location;

use super::{
    ViewState,
    context_menu::{
        ContextResolver, bind_column_context_owner, context_menu_danger_option,
        context_menu_option, context_menu_popover, focus_context_column, focus_context_entry,
        move_to_trash_is_visible, permanently_delete_is_visible, preview_context_entry,
        rename_context_entry, show_context_popover,
    },
    paths::is_trash_location,
};

#[derive(Clone, Copy)]
enum Action {
    Rename,
    Preview,
    Properties,
    Trash,
    PermanentDelete,
    NewFolder,
}

fn menu(
    options: &[(Action, &str, &str, &str, bool, bool)],
    run: impl Fn(Action) + 'static,
) -> (gtk::Popover, gtk::ScrolledWindow) {
    let content = super::super::accessibility::menu_box();
    content.add_css_class("folder-context-menu");
    content.add_css_class("chooser-context-menu");
    let (popover, scroll) = context_menu_popover(&content);
    popover.add_css_class("folder-context-popover");
    let pending = Rc::new(Cell::new(None));
    for &(action, icon, label, shortcut, enabled, danger) in options {
        let button = if danger {
            context_menu_danger_option(icon, label, shortcut)
        } else {
            context_menu_option(icon, label, shortcut)
        };
        button.set_sensitive(enabled);
        let pending = pending.clone();
        let weak = popover.downgrade();
        button.connect_clicked(move |_| {
            pending.set(Some(action));
            if let Some(popover) = weak.upgrade() {
                popover.popdown();
            }
        });
        content.append(&button);
    }
    let run = Rc::new(run);
    popover.connect_closed(move |popover| {
        popover.unparent();
        if let Some(action) = pending.take() {
            let run = run.clone();
            glib::idle_add_local_once(move || run(action));
        }
    });
    (popover, scroll)
}

pub(super) fn install_folder(
    state: &Rc<ViewState>,
    parent: &gtk::Widget,
    is_item_target: Rc<dyn Fn(&gtk::Widget) -> bool>,
    depth: usize,
    location: Location,
) -> Rc<dyn Fn(f64, f64)> {
    let weak = Rc::downgrade(state);
    let anchor_for_trigger = parent.downgrade();
    let location_for_trigger = location.clone();
    let open_at: Rc<dyn Fn(f64, f64)> = {
        let weak = weak.clone();
        Rc::new(move |x: f64, y: f64| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let Some(anchor) = anchor_for_trigger.upgrade() else {
                return;
            };
            let weak = Rc::downgrade(&state);
            let location = location_for_trigger.clone();
            let (popover, scroll) = menu(
                &[(
                    Action::NewFolder,
                    crate::assets::icons::FOLDER_PLUS,
                    "New Folder",
                    "Ctrl+Shift+N",
                    true,
                    false,
                )],
                move |_| {
                    if let Some(state) = weak.upgrade() {
                        state.begin_new_entry(depth, location.clone(), true);
                    }
                },
            );
            bind_column_context_owner(&state, &popover, depth);
            focus_context_column(&state, depth);
            show_context_popover(&popover, &scroll, &anchor, x, y);
        })
    };

    let click = gtk::GestureClick::new();
    click.set_button(3);
    let open_for_click = open_at.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(anchor) = gesture.widget() else {
            return;
        };
        if anchor
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .is_some_and(|picked| is_item_target(&picked))
        {
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
        open_for_click(x, y);
    });
    parent.add_controller(click);
    open_at
}

pub(super) fn install_item(
    state: &Rc<ViewState>,
    widget: &gtk::Widget,
    resolve: ContextResolver,
    depth: usize,
) -> Rc<dyn Fn(f64, f64)> {
    let weak = Rc::downgrade(state);
    let widget_for_trigger = widget.downgrade();
    let open_at_resolved: Rc<dyn Fn(f64, f64) -> bool> = Rc::new(move |x: f64, y: f64| {
        let Some(widget) = widget_for_trigger.upgrade() else {
            return false;
        };
        let Some(picked) = widget.pick(x, y, gtk::PickFlags::DEFAULT) else {
            return false;
        };
        let Some(state) = weak.upgrade() else {
            return false;
        };
        let Some((source, entry)) = resolve(&picked) else {
            return false;
        };
        focus_context_entry(&state, depth, source, &entry);
        let single = source.is_none() || state.browser.selected_entries().len() == 1;
        let in_trash = state
            .browser
            .location_at(depth)
            .as_ref()
            .is_some_and(is_trash_location);
        let can_trash = state.browser.can_trash_at(depth);
        let can_delete = state.browser.can_delete_at(depth);
        let trash_visible = move_to_trash_is_visible(in_trash, can_trash);
        let permanent_visible = permanently_delete_is_visible(in_trash, can_delete);
        let delete_label = if in_trash {
            "Permanently delete"
        } else {
            "Move to Trash"
        };
        let mut options = vec![(
            Action::Rename,
            crate::assets::icons::PENCIL,
            "Rename",
            "F2 / Ctrl+R",
            single,
            false,
        )];
        if single && super::super::preview::entry_supports_quick_preview(&entry) {
            options.push((
                Action::Preview,
                crate::assets::icons::EYE,
                "Quick preview",
                "Space",
                true,
                false,
            ));
        }
        options.push((
            Action::Properties,
            crate::assets::icons::INFO,
            "Properties",
            "Alt+Enter",
            true,
            false,
        ));
        if trash_visible {
            options.push((
                if in_trash {
                    Action::PermanentDelete
                } else {
                    Action::Trash
                },
                crate::assets::icons::TRASH,
                delete_label,
                "Del",
                true,
                in_trash,
            ));
        }
        if permanent_visible {
            options.push((
                Action::PermanentDelete,
                crate::assets::icons::TRASH,
                "Permanently delete",
                "Shift+Del",
                true,
                true,
            ));
        }
        let weak = weak.clone();
        let target = Rc::new(RefCell::new(Some((source, entry.clone()))));
        let (popover, scroll) = menu(&options, move |action| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            match action {
                Action::Rename => {
                    rename_context_entry(&state, depth, source, entry.clone());
                }
                Action::Preview => preview_context_entry(&state, depth, source, entry.clone()),
                Action::Properties => state.show_entry_properties_at(entry.clone(), depth),
                Action::Trash | Action::PermanentDelete => {
                    let entries = super::context_menu::context_entries(&state, &target);
                    let permanent = matches!(action, Action::PermanentDelete);
                    state.request_delete(entries, permanent);
                }
                Action::NewFolder => unreachable!(),
            }
        });
        bind_column_context_owner(&state, &popover, depth);
        show_context_popover(&popover, &scroll, &widget, x, y);
        true
    });

    let open_for_trigger = open_at_resolved.clone();
    let open_at: Rc<dyn Fn(f64, f64)> = Rc::new(move |x, y| {
        open_for_trigger(x, y);
    });

    let click = gtk::GestureClick::new();
    click.set_button(3);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    click.connect_pressed(move |gesture, _, x, y| {
        if open_at_resolved(x, y) {
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    widget.add_controller(click);
    open_at
}
