// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};

use crate::model::Location;

use super::{
    ViewState,
    context_menu::{
        ContextResolver, bind_column_context_owner, context_menu_option, context_menu_popover,
        focus_context_column, preview_context_entry, rename_context_entry, show_context_popover,
    },
};

#[derive(Clone, Copy)]
enum Action {
    Rename,
    Preview,
    Properties,
    NewFolder,
}

fn menu(
    options: &[(Action, &str, &str, &str, bool)],
    run: impl Fn(Action) + 'static,
) -> (gtk::Popover, gtk::ScrolledWindow) {
    let content = super::super::accessibility::menu_box();
    content.add_css_class("folder-context-menu");
    content.add_css_class("chooser-context-menu");
    let (popover, scroll) = context_menu_popover(&content);
    popover.add_css_class("folder-context-popover");
    let pending = Rc::new(Cell::new(None));
    for &(action, icon, label, shortcut, enabled) in options {
        let button = context_menu_option(icon, label, shortcut);
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
    let anchor_for_trigger = parent.clone();
    let location_for_trigger = location.clone();
    let open_at: Rc<dyn Fn(f64, f64)> = {
        let weak = weak.clone();
        Rc::new(move |x: f64, y: f64| {
            let Some(state) = weak.upgrade() else {
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
                )],
                move |_| {
                    if let Some(state) = weak.upgrade() {
                        state.begin_new_entry(depth, location.clone(), true);
                    }
                },
            );
            bind_column_context_owner(&state, &popover, depth);
            focus_context_column(&state, depth);
            show_context_popover(&popover, &scroll, &anchor_for_trigger, x, y);
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
    let widget_for_trigger = widget.clone();
    let open_at_resolved: Rc<dyn Fn(f64, f64) -> bool> = Rc::new(move |x: f64, y: f64| {
        let Some(picked) = widget_for_trigger.pick(x, y, gtk::PickFlags::DEFAULT) else {
            return false;
        };
        let Some(state) = weak.upgrade() else {
            return false;
        };
        let Some((source, entry)) = resolve(&picked) else {
            return false;
        };
        let single = source.is_none() || state.browser.selected_entries().len() == 1;
        let mut options = vec![(
            Action::Rename,
            crate::assets::icons::PENCIL,
            "Rename",
            "F2 / Ctrl+R",
            single,
        )];
        if single && super::super::preview::entry_supports_quick_preview(&entry) {
            options.push((
                Action::Preview,
                crate::assets::icons::EYE,
                "Quick preview",
                "Space",
                true,
            ));
        }
        options.push((
            Action::Properties,
            crate::assets::icons::INFO,
            "Properties",
            "Alt+Enter",
            true,
        ));
        let weak = weak.clone();
        let (popover, scroll) = menu(&options, move |action| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            match action {
                Action::Rename => {
                    rename_context_entry(&state, depth, source, entry.clone());
                }
                Action::Preview => preview_context_entry(&state, depth, source, entry.clone()),
                Action::Properties => state.show_entry_properties(entry.clone()),
                Action::NewFolder => unreachable!(),
            }
        });
        bind_column_context_owner(&state, &popover, depth);
        focus_context_column(&state, depth);
        show_context_popover(&popover, &scroll, &widget_for_trigger, x, y);
        true
    });

    let open_for_trigger = open_at_resolved.clone();
    let open_at: Rc<dyn Fn(f64, f64)> = Rc::new(move |x, y| {
        open_for_trigger(x, y);
    });

    let click = gtk::GestureClick::new();
    click.set_button(3);
    click.connect_pressed(move |gesture, _, x, y| {
        if open_at_resolved(x, y) {
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    widget.add_controller(click);
    open_at
}
