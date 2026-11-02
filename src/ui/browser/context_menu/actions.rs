// SPDX-License-Identifier: MIT

#[cfg(test)]
use std::collections::HashMap;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::{Rc, Weak},
};

use gtk::{gio, prelude::*};

#[cfg(test)]
mod tests;

mod presentation;
mod send_to;
#[cfg(test)]
use presentation::collect_presentations;
pub(super) use presentation::refresh_presentation;
use send_to::{SendToActions, SendToChoices};

use crate::{
    assets::icons,
    model::{FileEntry, Location, MenuPlacement},
    services::{InvocationSource, MatchedAction},
    ui::actions::{action_icon, folder_input, inputs_for_entries, native_paths, run_action},
};

use super::super::ViewState;

pub(super) struct ActionMenuSection {
    popover: gtk::PopoverMenu,
    model: gio::Menu,
    provider_model: gio::Menu,
    provider_group: gio::SimpleActionGroup,
    provider_epoch: Rc<std::cell::Cell<u64>>,
    pub(super) transfer_sections: Option<[gio::Menu; 2]>,
    send_to_model: gio::Menu,
    send_to_provider_model: gio::Menu,
    send_to_offset: Rc<Cell<i32>>,
    send_to_transfer_index: Cell<Option<usize>>,
    send_to_row: Cell<Option<(usize, i32)>>,
    root_model: gio::Menu,
    header: Option<gtk::Widget>,
    owner: gtk::glib::WeakRef<gtk::Widget>,
    actions: gio::SimpleActionGroup,
    dispatch: super::commands::MenuDispatch,
    _commands: super::commands::CommandMenus,
    navigation: Rc<super::keyboard::NativeMenuNavigation>,
    removable_watch: RefCell<Option<crate::ui::RemovableWatch>>,
    refresh_pending: Cell<bool>,
    selection: RefCell<Option<SelectionMenuContext>>,
    pressed_inside: Rc<Cell<bool>>,
}

#[derive(Clone)]
struct SelectionMenuContext {
    state: Weak<ViewState>,
    entries: Vec<FileEntry>,
    parent: Option<PathBuf>,
}

impl Drop for ActionMenuSection {
    fn drop(&mut self) {
        release_popover(&self.popover);
    }
}

// GTK before 4.22 can keep a window's pointer focus on a hidden popover and crash
// on its missing surface after unparenting. A sensitivity change repicks the target.
fn release_popover(popover: &gtk::PopoverMenu) {
    if popover.parent().is_none() {
        return;
    }
    popover.set_sensitive(false);
    popover.set_sensitive(true);
    popover.unparent();
}

// GTK before 4.22 leaves each replaced model's position handler on its freed
// section box, so the popover must start without a model.
#[expect(
    unsafe_code,
    reason = "gtk4-rs requires a model to construct a nested PopoverMenu"
)]
fn nested_popover_menu() -> gtk::PopoverMenu {
    use gtk::glib::translate::{IntoGlib, from_glib_none};

    // SAFETY: GTK accepts a null model and returns a new floating widget.
    let widget = unsafe {
        gtk::ffi::gtk_popover_menu_new_from_model_full(
            std::ptr::null_mut(),
            gtk::PopoverMenuFlags::NESTED.into_glib(),
        )
    };
    // SAFETY: `widget` is the non-null widget GTK just returned; this sinks its floating reference.
    let widget: gtk::Widget = unsafe { from_glib_none(widget) };
    widget.downcast().expect("GTK constructs a PopoverMenu")
}

impl ActionMenuSection {
    pub(super) fn new(
        before: &impl IsA<gtk::Widget>,
        after: &impl IsA<gtk::Widget>,
        header: Option<&gtk::Widget>,
        anchor: &gtk::Widget,
        transfer_buttons: Option<[gtk::Button; 2]>,
    ) -> Rc<Self> {
        let model = gio::Menu::new();
        let root = gio::Menu::new();
        let popover = base_popover(header.is_none());
        let dispatch = super::commands::MenuDispatch;
        let navigation = super::keyboard::NativeMenuNavigation::new(&popover);
        let commands = super::commands::CommandMenus::new(
            before.as_ref(),
            after.as_ref(),
            &popover,
            &dispatch,
            &navigation,
            transfer_buttons.as_ref(),
        );
        popover.insert_action_group("builtin", Some(&commands.group));
        if let Some(header) = header {
            append_header_item(&root, header);
        }
        for section in &commands.before {
            root.append_section(None, section);
        }
        root.append_section(None, &model);
        let provider_model = gio::Menu::new();
        let provider_group = gio::SimpleActionGroup::new();
        let provider_epoch = Rc::new(std::cell::Cell::new(0));
        let provider_start = root.n_items();
        // An enclosing section would add a second separator above the titled groups.
        mirror_menu_items(&provider_model, &root, move || provider_start);
        popover.insert_action_group("provider", Some(&provider_group));
        // Send to… holds the removable-drive section first, then mirrored
        // provider sections that arrive and withdraw while the menu is open.
        let send_to_model = gio::Menu::new();
        let send_to_provider_model = gio::Menu::new();
        let send_to_offset = Rc::new(Cell::new(0));
        let offset = send_to_offset.clone();
        mirror_menu_items(&send_to_provider_model, &send_to_model, move || {
            offset.get()
        });
        advance_epoch_after_close(&popover, &provider_epoch);
        for section in &commands.after {
            root.append_section(None, section);
        }
        let actions = gio::SimpleActionGroup::new();
        popover.insert_action_group("custom", Some(&actions));
        release_after_close(&popover);
        let pressed_inside = guard_unpaired_release(&popover);
        refresh_presentation(&popover, &navigation);
        let transfer_sections = commands.transfer_sections.clone();
        let menu = Rc::new(Self {
            popover,
            model,
            provider_model,
            provider_group,
            provider_epoch,
            transfer_sections,
            send_to_model,
            send_to_provider_model,
            send_to_offset,
            send_to_transfer_index: Cell::new(None),
            send_to_row: Cell::new(None),
            root_model: root,
            header: header.cloned(),
            owner: anchor.downgrade(),
            actions,
            dispatch,
            _commands: commands,
            navigation,
            removable_watch: RefCell::new(None),
            refresh_pending: Cell::new(false),
            selection: RefCell::new(None),
            pressed_inside,
        });
        menu.watch_send_to_destinations();
        menu
    }

    fn watch_send_to_destinations(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.send_to_model.connect_items_changed(move |_, _, _, _| {
            if let Some(menu) = weak.upgrade() {
                menu.sync_send_to_row();
            }
        });
        if self.transfer_sections.is_none() {
            return;
        }
        let weak = Rc::downgrade(self);
        self.removable_watch
            .replace(Some(crate::ui::watch_removable_destinations(move || {
                if let Some(menu) = weak.upgrade() {
                    menu.schedule_removable_refresh();
                }
            })));
    }

    fn schedule_removable_refresh(self: &Rc<Self>) {
        if !self.popover.is_visible() || self.refresh_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        gtk::glib::idle_add_local_once(move || {
            if let Some(menu) = weak.upgrade() {
                menu.refresh_pending.set(false);
                menu.refresh_removable_destinations();
            }
        });
    }

    fn refresh_removable_destinations(&self) {
        let Some(selection) = self.selection.borrow().clone() else {
            return;
        };
        if !self.popover.is_visible() {
            return;
        }
        if let Some(state) = selection.state.upgrade() {
            self.rebuild_for_selection(&state, &selection.entries, selection.parent);
            self.popover.set_visible_submenu(Some("main"));
            self.navigation.model_changed();
        }
    }

    pub(super) fn popover(&self) -> gtk::Popover {
        self.popover.clone().upcast()
    }

    pub(super) fn show(&self, anchor: &gtk::Widget, x: f64, y: f64) {
        self.pressed_inside.set(false);
        if self.popover.parent().is_none()
            && let Some(owner) = self.owner.upgrade()
        {
            if let Some(overlay) = owner.downcast_ref::<gtk::Overlay>() {
                overlay.add_overlay(&self.popover);
            } else {
                self.popover.set_parent(&owner);
            }
        }
        self.navigation.begin();
        self._commands.refresh();
        if self.popover.menu_model().is_none() {
            self.popover.set_menu_model(Some(&self.root_model));
            if let Some(header) = &self.header {
                assert!(self.popover.add_child(header, "context-header"));
            }
            refresh_presentation(&self.popover, &self.navigation);
        }
        self.popover.set_visible_submenu(Some("main"));
        self.navigation.model_changed();
        super::show_model_context_popover(self.popover.upcast_ref(), anchor, x, y);
        super::keyboard::focus_first_or_last_menu_item(self.popover.upcast_ref(), true);
    }

    pub(super) fn rebuild_for_selection(
        &self,
        state: &Rc<ViewState>,
        entries: &[FileEntry],
        parent: Option<PathBuf>,
    ) {
        self.selection.replace(Some(SelectionMenuContext {
            state: Rc::downgrade(state),
            entries: entries.to_vec(),
            parent: parent.clone(),
        }));
        #[cfg(test)]
        if let Some(test_override) = state.send_to_menu_test_override.borrow().clone() {
            self.rebuild_for_selection_with_destinations(
                state,
                (entries, parent),
                SendToChoices {
                    destinations: test_override.destinations,
                    recent_destinations: test_override.recent_destinations,
                    handlers: test_override.handlers,
                },
            );
            return;
        }
        self.rebuild_for_selection_with_destinations(
            state,
            (entries, parent),
            SendToChoices::current(state),
        );
    }

    fn rebuild_for_selection_with_destinations(
        &self,
        state: &Rc<ViewState>,
        (entries, parent): (&[FileEntry], Option<PathBuf>),
        choices: SendToChoices,
    ) {
        self.clear();
        let sources: Vec<_> = entries.iter().map(|entry| entry.location.clone()).collect();
        self.rebuild_send_to(&sources, &choices);

        let (Some(inputs), Some(paths), Some(parent)) =
            (inputs_for_entries(entries), native_paths(entries), parent)
        else {
            refresh_presentation(&self.popover, &self.navigation);
            return;
        };
        self.watch_providers(paths.clone(), false);
        let catalog = crate::ui::actions::shared().catalog();
        self.rebuild_custom_actions(
            state,
            &catalog.matches(&inputs),
            CustomInvocation {
                paths,
                parent,
                source: InvocationSource::Selection,
            },
        );
    }

    fn rebuild_send_to(&self, sources: &[Location], choices: &SendToChoices) {
        let transfer_index = match sources.len() {
            0 => None,
            1 => Some(0),
            _ => Some(1),
        };
        let Some(transfer_index) = transfer_index.filter(|_| self.transfer_sections.is_some())
        else {
            return;
        };
        let devices = gio::Menu::new();
        SendToActions {
            actions: &self.actions,
            dispatch: &self.dispatch,
        }
        .append_devices(&devices, choices, sources);
        self.send_to_transfer_index.set(Some(transfer_index));
        self.set_removable_devices((devices.n_items() > 0).then_some(devices));
        self.sync_send_to_row();
    }

    pub(super) fn rebuild_for_folder(&self, state: &Rc<ViewState>, location: &Location) {
        self.selection.take();
        self.clear();
        let (Some(input), Some(path)) = (folder_input(location), location.native_path()) else {
            refresh_presentation(&self.popover, &self.navigation);
            return;
        };
        self.watch_providers(vec![path.to_path_buf()], true);
        let catalog = crate::ui::actions::shared().catalog();
        self.rebuild_custom_actions(
            state,
            &catalog.matches(std::slice::from_ref(&input)),
            CustomInvocation {
                paths: vec![path.to_path_buf()],
                parent: path.to_path_buf(),
                source: InvocationSource::Background,
            },
        );
    }

    fn watch_providers(&self, paths: Vec<PathBuf>, background: bool) {
        let popover = self.popover.downgrade();
        let navigation = self.navigation.clone();
        let send_to = self
            .send_to_transfer_index
            .get()
            .map(|_| self.send_to_provider_model.clone());
        crate::ui::file_providers::watch_menu(
            (self.provider_model.clone(), send_to),
            self.provider_group.clone(),
            (
                self.owner
                    .upgrade()
                    .as_ref()
                    .unwrap_or(self.popover.upcast_ref()),
                &self.popover,
            ),
            paths,
            background,
            (self.provider_epoch.clone(), self.provider_epoch.get()),
            move |preserve_navigation| {
                if let Some(popover) = popover.upgrade() {
                    if preserve_navigation {
                        navigation.preserve_submenu_navigation();
                    }
                    refresh_presentation(&popover, &navigation);
                    navigation.model_changed();
                }
            },
        );
    }

    fn clear(&self) {
        self.provider_epoch.set(self.provider_epoch.get() + 1);
        self.provider_model.remove_all();
        self.send_to_provider_model.remove_all();
        for name in self.provider_group.list_actions() {
            self.provider_group.remove_action(&name);
        }
        self.send_to_transfer_index.set(None);
        self.set_removable_devices(None);
        self.sync_send_to_row();
        self.model.remove_all();
        for name in self.actions.list_actions() {
            self.actions.remove_action(&name);
        }
    }

    fn set_removable_devices(&self, devices: Option<gio::Menu>) {
        if self.send_to_offset.replace(0) == 1 {
            self.send_to_model.remove(0);
        }
        if let Some(devices) = devices {
            self.send_to_model.insert_section(0, None, &devices);
            self.send_to_offset.set(1);
        }
    }

    /// Shows Send to… while it has any destination, removable or provider-supplied.
    fn sync_send_to_row(&self) {
        let Some(transfer_sections) = &self.transfer_sections else {
            return;
        };
        let wanted = self
            .send_to_transfer_index
            .get()
            .filter(|_| self.send_to_model.n_items() > 0);
        if self.send_to_row.get().map(|(index, _)| index) == wanted {
            return;
        }
        if let Some((index, row)) = self.send_to_row.take() {
            transfer_sections[index].remove(row);
        }
        if let Some(index) = wanted {
            let section = &transfer_sections[index];
            let item =
                gio::MenuItem::new_submenu(Some(&crate::i18n::tr("Send to…")), &self.send_to_model);
            item.set_icon(&gio::ThemedIcon::new(crate::assets::icons::SEND_HORIZONTAL));
            self.send_to_row.set(Some((index, section.n_items())));
            section.append_item(&item);
        }
    }

    fn rebuild_custom_actions(
        &self,
        state: &Rc<ViewState>,
        matched: &[MatchedAction],
        invocation: CustomInvocation,
    ) {
        let submenu = gio::Menu::new();
        for (index, matched) in matched.iter().enumerate() {
            let item = self.custom_action_item(state, (index, matched), &invocation);
            let model = if matched.placement == MenuPlacement::Top {
                &self.model
            } else {
                &submenu
            };
            model.append_item(&item);
        }
        if submenu.n_items() > 0 {
            let item = gio::MenuItem::new_submenu(Some(&crate::i18n::tr("Actions")), &submenu);
            item.set_icon(&gio::ThemedIcon::new(icons::PLAY));
            self.model.append_item(&item);
        }
        refresh_presentation(&self.popover, &self.navigation);
    }

    fn custom_action_item(
        &self,
        state: &Rc<ViewState>,
        (index, matched): (usize, &MatchedAction),
        invocation: &CustomInvocation,
    ) -> gio::MenuItem {
        let name = format!("run-{index}");
        let action = gio::SimpleAction::new(&name, None);
        action.set_enabled(matched.action.is_available());
        let handle = matched.action.clone();
        let CustomInvocation {
            paths,
            parent,
            source,
        } = invocation.clone();
        let state = Rc::downgrade(state);
        let dispatch = self.dispatch.clone();
        action.connect_activate(move |_, _| {
            let (state, handle, paths, parent) =
                (state.clone(), handle.clone(), paths.clone(), parent.clone());
            dispatch.defer(move || {
                if let Some(state) = state.upgrade() {
                    run_action(&state.overlay, handle, paths, parent, source, None);
                }
            });
        });
        self.actions.add_action(&action);
        let item = gio::MenuItem::new(
            Some(&matched.action.name().replace('_', "__")),
            Some(&format!("custom.{name}")),
        );
        item.set_icon(&gio::ThemedIcon::new(action_icon(
            matched.action.definition.icon.as_deref(),
        )));
        let hint = matched.action.unavailable_reason().or(matched
            .action
            .definition
            .description
            .as_deref());
        if let Some(hint) = hint {
            item.set_attribute_value("x-strata-description", Some(&hint.to_variant()));
        }
        item
    }
}

/// The paths a custom action runs on and where it was invoked from.
#[derive(Clone)]
struct CustomInvocation {
    paths: Vec<PathBuf>,
    parent: PathBuf,
    source: InvocationSource,
}

fn base_popover(body_only: bool) -> gtk::PopoverMenu {
    let popover = nested_popover_menu();
    popover.set_has_arrow(false);
    popover.add_css_class("folder-context-popover");
    popover.add_css_class("actions-context-popover");
    if body_only {
        popover.add_css_class("folder-menu-body");
    }
    popover
}

fn append_header_item(root: &gio::Menu, header: &gtk::Widget) {
    let item = gio::MenuItem::new(None, None);
    item.set_attribute_value("custom", Some(&"context-header".to_variant()));
    root.append_item(&item);
    header.set_margin_start(8);
    header.set_margin_end(8);
    header.set_margin_top(8);
}

/// Mirrors `source`'s items into `target`, starting at the position `offset` reports.
fn mirror_menu_items(source: &gio::Menu, target: &gio::Menu, offset: impl Fn() -> i32 + 'static) {
    let target = target.clone();
    source.connect_items_changed(move |model, position, removed, added| {
        for _ in 0..removed {
            target.remove(offset() + position);
        }
        for index in position..position + added {
            target.insert_item(offset() + index, &gio::MenuItem::from_model(model, index));
        }
    });
}

fn advance_epoch_after_close(popover: &gtk::PopoverMenu, epoch: &Rc<Cell<u64>>) {
    let closed_epoch = epoch.clone();
    popover.connect_closed(move |popover| {
        // GTK closes the popover before dispatching the chosen leaf action.
        let epoch = closed_epoch.get();
        let closed_epoch = closed_epoch.clone();
        let popover = popover.downgrade();
        gtk::glib::idle_add_local_once(move || {
            if closed_epoch.get() == epoch && popover.upgrade().is_none_or(|p| !p.is_visible()) {
                closed_epoch.set(epoch + 1);
            }
        });
    });
}

fn release_after_close(popover: &gtk::PopoverMenu) {
    popover.connect_closed(|popover| {
        let Some(clock) = popover.frame_clock() else {
            return;
        };
        let weak = popover.downgrade();
        let handler = Rc::new(std::cell::RefCell::new(None));
        let handler_for_signal = handler.clone();
        let id = clock.connect_after_paint(move |clock| {
            if let Some(id) = handler_for_signal.borrow_mut().take() {
                clock.disconnect(id);
            }
            let weak = weak.clone();
            gtk::glib::idle_add_local_once(move || {
                if let Some(popover) = weak.upgrade()
                    && !popover.is_visible()
                {
                    release_popover(&popover);
                }
            });
        });
        handler.replace(Some(id));
        clock.request_phase(gtk::gdk::FrameClockPhase::AFTER_PAINT);
    });
}

// GtkModelButton activates on an unpaired release, and some compositors
// (niri) deliver the release of the click that opened the menu to the
// popup. A release before any press inside the menu can only be that one.
fn guard_unpaired_release(popover: &gtk::PopoverMenu) -> Rc<Cell<bool>> {
    let pressed_inside = Rc::new(Cell::new(false));
    let seen_press = pressed_inside.clone();
    let release_guard = gtk::EventControllerLegacy::new();
    release_guard.set_propagation_phase(gtk::PropagationPhase::Capture);
    // Also cover the nested submenu popovers, which are separate natives.
    release_guard.set_propagation_limit(gtk::PropagationLimit::None);
    release_guard.connect_event(move |_, event| match event.event_type() {
        gtk::gdk::EventType::ButtonPress => {
            seen_press.set(true);
            gtk::glib::Propagation::Proceed
        }
        gtk::gdk::EventType::ButtonRelease if !seen_press.get() => gtk::glib::Propagation::Stop,
        _ => gtk::glib::Propagation::Proceed,
    });
    popover.add_controller(release_guard);
    pressed_inside
}
