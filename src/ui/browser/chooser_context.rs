// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[derive(Clone, Copy)]
enum Action {
    Rename,
    Properties,
    NewFolder,
}

fn menu(
    options: &[(Action, &str, &str, &str, bool)],
    run: impl Fn(Action) + 'static,
) -> (gtk::Popover, gtk::ScrolledWindow) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
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
) {
    let click = gtk::GestureClick::new();
    click.set_button(3);
    let weak = Rc::downgrade(state);
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
        let weak = weak.clone();
        let location = location.clone();
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
        show_context_popover(&popover, &scroll, &anchor, x, y);
    });
    parent.add_controller(click);
}

pub(super) fn install_item(
    state: &Rc<ViewState>,
    widget: &gtk::Widget,
    selection: &gtk::MultiSelection,
    pick_position: ContextPickPosition,
    source_position: ContextSourcePosition,
    clear_other_selections: Rc<dyn Fn()>,
    depth: usize,
) {
    let click = gtk::GestureClick::new();
    click.set_button(3);
    let weak = Rc::downgrade(state);
    let selection = selection.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(anchor) = gesture.widget() else {
            return;
        };
        let Some(position) = anchor
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .and_then(|picked| pick_position(&picked))
        else {
            return;
        };
        let Some(source) = source_position(position) else {
            return;
        };
        let Some(state) = weak.upgrade() else {
            return;
        };
        let Some(entry) = state.browser.entry_at(depth, source) else {
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if !selection.is_selected(position) {
            clear_other_selections();
            selection.select_item(position, true);
        }
        let single = state.browser.selected_entries().len() == 1;
        let weak = weak.clone();
        let (popover, scroll) = menu(
            &[
                (
                    Action::Rename,
                    crate::assets::icons::PENCIL,
                    "Rename",
                    "F2",
                    single,
                ),
                (
                    Action::Properties,
                    crate::assets::icons::INFO,
                    "Properties",
                    "Alt+Enter",
                    true,
                ),
            ],
            move |action| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                match action {
                    Action::Rename => {
                        state.browser.select(depth, source);
                        let weak = Rc::downgrade(&state);
                        // Selection queues collection focus; enter the editor after it settles.
                        glib::idle_add_local_once(move || {
                            if let Some(state) = weak.upgrade() {
                                state.begin_rename();
                            }
                        });
                    }
                    Action::Properties => state.show_entry_properties(entry.clone()),
                    Action::NewFolder => unreachable!(),
                }
            },
        );
        show_context_popover(&popover, &scroll, &anchor, x, y);
    });
    widget.add_controller(click);
}
