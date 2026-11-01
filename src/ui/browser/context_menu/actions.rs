// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
    rc::{Rc, Weak},
};

use gtk::{gio, prelude::*};

#[cfg(test)]
mod tests;

use crate::{
    assets::{self, icons},
    model::{FileEntry, Location, MenuPlacement},
    services::{InvocationSource, MatchedAction},
    ui::actions::{action_icon, folder_input, inputs_for_entries, native_paths, run_action},
};

use super::super::{SendToMenuHandlers, ViewState};

const SEND_TO_DEVICE_NAME_MAX_CHARS: i32 = 15;

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
        let popover = nested_popover_menu();
        popover.set_has_arrow(false);
        popover.add_css_class("folder-context-popover");
        popover.add_css_class("actions-context-popover");
        if header.is_none() {
            popover.add_css_class("folder-menu-body");
        }
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
            let item = gio::MenuItem::new(None, None);
            item.set_attribute_value("custom", Some(&"context-header".to_variant()));
            root.append_item(&item);
            header.set_margin_start(8);
            header.set_margin_end(8);
            header.set_margin_top(8);
        }
        for section in &commands.before {
            root.append_section(None, section);
        }
        root.append_section(None, &model);
        let provider_model = gio::Menu::new();
        let provider_group = gio::SimpleActionGroup::new();
        let provider_epoch = Rc::new(std::cell::Cell::new(0));
        let provider_start = root.n_items();
        let provider_root = root.clone();
        // An enclosing section would add a second separator above the titled groups.
        provider_model.connect_items_changed(move |model, position, removed, added| {
            for _ in 0..removed {
                provider_root.remove(provider_start + position);
            }
            for index in position..position + added {
                provider_root.insert_item(
                    provider_start + index,
                    &gio::MenuItem::from_model(model, index),
                );
            }
        });
        popover.insert_action_group("provider", Some(&provider_group));
        // Send to… holds the removable-drive section first, then mirrored
        // provider sections that arrive and withdraw while the menu is open.
        let send_to_model = gio::Menu::new();
        let send_to_provider_model = gio::Menu::new();
        let send_to_offset = Rc::new(Cell::new(0));
        let (send_to_root, offset) = (send_to_model.clone(), send_to_offset.clone());
        send_to_provider_model.connect_items_changed(move |model, position, removed, added| {
            for _ in 0..removed {
                send_to_root.remove(offset.get() + position);
            }
            for index in position..position + added {
                send_to_root.insert_item(
                    offset.get() + index,
                    &gio::MenuItem::from_model(model, index),
                );
            }
        });
        let closed_epoch = provider_epoch.clone();
        popover.connect_closed(move |popover| {
            // GTK closes the popover before dispatching the chosen leaf action.
            let epoch = closed_epoch.get();
            let closed_epoch = closed_epoch.clone();
            let popover = popover.downgrade();
            gtk::glib::idle_add_local_once(move || {
                if closed_epoch.get() == epoch && popover.upgrade().is_none_or(|p| !p.is_visible())
                {
                    closed_epoch.set(epoch + 1);
                }
            });
        });
        for section in &commands.after {
            root.append_section(None, section);
        }
        let actions = gio::SimpleActionGroup::new();
        popover.insert_action_group("custom", Some(&actions));
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
        // GtkModelButton activates on an unpaired release, and some compositors
        // (niri) deliver the release of the click that opened the menu to the
        // popup. A release before any press inside the menu can only be that one.
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
        let weak = Rc::downgrade(&menu);
        menu.send_to_model.connect_items_changed(move |_, _, _, _| {
            if let Some(menu) = weak.upgrade() {
                menu.sync_send_to_row();
            }
        });
        if menu.transfer_sections.is_some() {
            let weak = Rc::downgrade(&menu);
            menu.removable_watch
                .replace(Some(crate::ui::watch_removable_destinations(move || {
                    if let Some(menu) = weak.upgrade() {
                        menu.schedule_removable_refresh();
                    }
                })));
        }
        menu
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
                entries,
                parent,
                test_override.destinations,
                test_override.recent_destinations,
                test_override.handlers,
            );
            return;
        }

        let weak_state = Rc::downgrade(state);
        let activate_root: Rc<dyn Fn(String, Vec<Location>)> = Rc::new(move |id, sources| {
            if let Some(state) = weak_state.upgrade() {
                state.send_to_removable_device(id, sources);
            }
        });
        let weak_state = Rc::downgrade(state);
        let choose_folder: Rc<dyn Fn(String, Vec<Location>)> = Rc::new(move |id, sources| {
            if let Some(state) = weak_state.upgrade() {
                state.show_send_to_folder_dialog(id, sources);
            }
        });
        let weak_state = Rc::downgrade(state);
        let activate_recent: Rc<dyn Fn(String, PathBuf, Vec<Location>)> =
            Rc::new(move |id, relative, sources| {
                if let Some(state) = weak_state.upgrade() {
                    state.send_to_recent_destination(id, relative, sources);
                }
            });
        let handlers = SendToMenuHandlers {
            activate_root,
            activate_recent,
            choose_folder,
        };
        let destinations = crate::ui::removable_destinations();
        let recent_destinations = destinations
            .iter()
            .map(|destination| {
                (
                    destination.id.clone(),
                    crate::ui::send_to_recent_folders(destination),
                )
            })
            .collect();
        self.rebuild_for_selection_with_destinations(
            state,
            entries,
            parent,
            destinations,
            recent_destinations,
            handlers,
        );
    }

    pub(super) fn rebuild_for_selection_with_destinations(
        &self,
        state: &Rc<ViewState>,
        entries: &[FileEntry],
        parent: Option<PathBuf>,
        destinations: Vec<crate::ui::RemovableDestination>,
        recent_destinations: HashMap<String, Vec<PathBuf>>,
        handlers: SendToMenuHandlers,
    ) {
        self.clear();
        let sources: Vec<_> = entries.iter().map(|entry| entry.location.clone()).collect();
        if let Some(transfer_index) = match sources.len() {
            0 => None,
            1 => Some(0),
            _ => Some(1),
        } && self.transfer_sections.is_some()
        {
            let devices = gio::Menu::new();
            append_send_to_devices(
                &devices,
                &self.actions,
                &self.dispatch,
                &destinations,
                &recent_destinations,
                &sources,
                handlers,
            );
            self.send_to_transfer_index.set(Some(transfer_index));
            self.set_removable_devices((devices.n_items() > 0).then_some(devices));
            self.sync_send_to_row();
        }

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
            paths,
            parent,
            InvocationSource::Selection,
        );
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
            vec![path.to_path_buf()],
            path.to_path_buf(),
            InvocationSource::Background,
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
        paths: Vec<PathBuf>,
        parent: PathBuf,
        source: InvocationSource,
    ) {
        let submenu = gio::Menu::new();
        for (index, matched) in matched.iter().enumerate() {
            let name = format!("run-{index}");
            let action = gio::SimpleAction::new(&name, None);
            action.set_enabled(matched.action.is_available());
            let handle = matched.action.clone();
            let paths = paths.clone();
            let parent = parent.clone();
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
            if let Some(hint) = matched.action.unavailable_reason().or(matched
                .action
                .definition
                .description
                .as_deref())
            {
                item.set_attribute_value("x-strata-description", Some(&hint.to_variant()));
            }
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
}

fn append_send_to_devices(
    devices: &gio::Menu,
    actions: &gio::SimpleActionGroup,
    dispatch: &super::commands::MenuDispatch,
    destinations: &[crate::ui::RemovableDestination],
    recent_destinations: &HashMap<String, Vec<PathBuf>>,
    sources: &[Location],
    handlers: SendToMenuHandlers,
) {
    if sources.is_empty() {
        return;
    }
    for (index, destination) in destinations.iter().enumerate() {
        let name = format!("send-to-{index}");
        let id = destination.id.clone();
        let selected = sources.to_vec();
        let activate_root = handlers.activate_root.clone();
        let root_dispatch = dispatch.clone();
        let action = gio::SimpleAction::new(&name, None);
        action.connect_activate(move |_, _| {
            let id = id.clone();
            let selected = selected.clone();
            let activate_root = activate_root.clone();
            root_dispatch.defer(move || activate_root(id, selected));
        });
        actions.add_action(&action);

        let root = gio::Menu::new();
        root.append(Some("Drive root"), Some(&format!("custom.{name}")));
        let recent_paths = recent_destinations
            .get(&destination.id)
            .into_iter()
            .flatten();
        for (recent_index, relative) in recent_paths.enumerate() {
            let recent_name = format!("send-to-recent-{index}-{recent_index}");
            let id = destination.id.clone();
            let selected = sources.to_vec();
            let relative_path = relative.clone();
            let activate_recent = handlers.activate_recent.clone();
            let recent_dispatch = dispatch.clone();
            let recent_action = gio::SimpleAction::new(&recent_name, None);
            recent_action.connect_activate(move |_, _| {
                let id = id.clone();
                let relative = relative_path.clone();
                let selected = selected.clone();
                let activate_recent = activate_recent.clone();
                recent_dispatch.defer(move || activate_recent(id, relative, selected));
            });
            actions.add_action(&recent_action);
            root.append(
                Some(&format!(
                    "{} (recent)",
                    relative.to_string_lossy().replace('_', "__")
                )),
                Some(&format!("custom.{recent_name}")),
            );
        }
        let folder_name = format!("choose-folder-{index}");
        let id = destination.id.clone();
        let selected = sources.to_vec();
        let choose_folder = handlers.choose_folder.clone();
        let folder_dispatch = dispatch.clone();
        let folder_action = gio::SimpleAction::new(&folder_name, None);
        folder_action.connect_activate(move |_, _| {
            let id = id.clone();
            let selected = selected.clone();
            let choose_folder = choose_folder.clone();
            folder_dispatch.defer(move || choose_folder(id, selected));
        });
        actions.add_action(&folder_action);
        let choose = gio::Menu::new();
        choose.append(
            Some(&crate::i18n::tr("Choose folder…")),
            Some(&format!("custom.{folder_name}")),
        );
        let device = gio::Menu::new();
        device.append_section(None, &root);
        device.append_section(None, &choose);
        let item = gio::MenuItem::new_submenu(Some(&destination.name.replace('_', "__")), &device);
        item.set_icon(&gio::ThemedIcon::new(icons::HARD_DRIVE));
        item.set_attribute_value("x-strata-send-to-device", Some(&true.to_variant()));
        item.set_attribute_value("x-strata-description", Some(&destination.name.to_variant()));
        devices.append_item(&item);
    }
}

pub(super) fn refresh_presentation(
    root: &gtk::PopoverMenu,
    navigation: &Rc<super::keyboard::NativeMenuNavigation>,
) {
    let mut items = Vec::new();
    if let Some(model) = root.menu_model() {
        collect_presentations(&model, &mut items);
    }
    present_native_items(root.upcast_ref(), root, &items, navigation);
}

#[derive(Clone)]
struct ItemPresentation {
    label: String,
    description: String,
    shortcut: String,
    submenu: Option<gio::MenuModel>,
    icon_size: i32,
    send_to_device: bool,
    danger: bool,
    custom: bool,
}

fn collect_presentations(model: &gio::MenuModel, items: &mut Vec<ItemPresentation>) {
    for index in 0..model.n_items() {
        let string = |name| {
            model
                .item_attribute_value(index, name, None)
                .and_then(|value| value.get::<String>())
        };
        if let Some(label) = string("label")
            && model.item_link(index, "section").is_none()
        {
            items.push(ItemPresentation {
                label: label.replace("__", "_"),
                description: string("x-strata-description").unwrap_or_default(),
                shortcut: string("x-strata-shortcut").unwrap_or_default(),
                submenu: model.item_link(index, "submenu"),
                icon_size: model
                    .item_attribute_value(index, "x-strata-icon-size", None)
                    .and_then(|value| value.get::<i32>())
                    .unwrap_or(15),
                send_to_device: model
                    .item_attribute_value(index, "x-strata-send-to-device", None)
                    .and_then(|value| value.get::<bool>())
                    .unwrap_or(false),
                danger: model
                    .item_attribute_value(index, "x-strata-danger", None)
                    .and_then(|value| value.get::<bool>())
                    .unwrap_or(false),
                custom: string("action").is_some_and(|name| name.starts_with("custom.")),
            });
        }
        for link in ["section", "submenu"] {
            if let Some(child) = model.item_link(index, link) {
                collect_presentations(&child, items);
            }
        }
    }
}

// Preserve generated rows: GTK uses them to own submenu selection and grabs.
fn submenu_owner(root: &gtk::PopoverMenu, submenu: &gtk::PopoverMenu) -> Option<gtk::Widget> {
    fn find(
        widget: &gtk::Widget,
        root: &gtk::Widget,
        submenu: &gtk::Popover,
    ) -> Option<gtk::Widget> {
        if widget != root && widget.is::<gtk::Popover>() {
            return None;
        }
        if widget.find_property("popover").is_some()
            && widget.property::<Option<gtk::Popover>>("popover").as_ref() == Some(submenu)
        {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(owner) = find(&widget, root, submenu) {
                return Some(owner);
            }
            child = widget.next_sibling();
        }
        None
    }

    let root_widget = root.upcast_ref::<gtk::Widget>();
    let submenu = submenu.clone().upcast::<gtk::Popover>();
    find(root_widget, root_widget, &submenu)
}

fn present_native_items(
    widget: &gtk::Widget,
    root: &gtk::PopoverMenu,
    items: &[ItemPresentation],
    navigation: &Rc<super::keyboard::NativeMenuNavigation>,
) {
    if widget.is::<gtk::Button>() {
        return;
    }
    if let Some(menu) = widget.downcast_ref::<gtk::PopoverMenu>() {
        menu.add_css_class("folder-context-popover");
        menu.add_css_class("actions-context-popover");
        if menu != root && !menu.has_css_class("actions-submenu") {
            menu.add_css_class("actions-submenu");
            if let Some(owner) = submenu_owner(root, menu) {
                super::keyboard::install_submenu_return(menu, &owner, navigation);
            }
            if gtk::minor_version() < 22 {
                // Older GTK emits focus leave before updating contains-focus.
                let focus = gtk::EventControllerFocus::new();
                focus.set_propagation_limit(gtk::PropagationLimit::None);
                let weak = menu.downgrade();
                focus.connect_contains_focus_notify(move |focus| {
                    if !focus.contains_focus() {
                        let weak = weak.clone();
                        gtk::glib::idle_add_local_once(move || {
                            if let Some(menu) = weak.upgrade()
                                && let Some(widget) = menu.root().and_then(|root| root.focus())
                                && !widget.is_ancestor(&menu)
                                && !menu.is_ancestor(&widget)
                            {
                                menu.set_visible(false);
                            }
                        });
                    }
                });
                menu.add_controller(focus);
            }
        }
    }
    if let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>() {
        if !scroll.has_css_class("context-menu-scroll") {
            scroll.add_css_class("context-menu-scroll");
            let is_root = scroll.ancestor(gtk::PopoverMenu::static_type()).as_ref()
                == Some(root.upcast_ref());
            if is_root {
                // A floor rather than a fixed width: translated rows may need more room,
                // and a natural width would follow long header paths instead of ellipsizing.
                scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
                scroll.set_propagate_natural_width(false);
                crate::ui::preferences::PreferenceManager::shared().bind_interface_scale(
                    scroll,
                    |scroll, scale| {
                        scroll.set_size_request((310.0 * scale).round() as i32, -1);
                    },
                );
            }
        }
        if scroll
            .ancestor(gtk::PopoverMenu::static_type())
            .and_downcast::<gtk::PopoverMenu>()
            .and_then(|menu| menu.menu_model())
            .is_some_and(|model| model.n_items() == 1)
        {
            scroll.set_vscrollbar_policy(gtk::PolicyType::Never);
        }
    }
    let mut children = Vec::new();
    let mut next = widget.first_child();
    while let Some(child) = next {
        next = child.next_sibling();
        children.push(child);
    }
    if widget.accessible_role() == gtk::AccessibleRole::MenuItem
        && let Some(label) = children
            .iter()
            .find_map(|child| child.downcast_ref::<gtk::Label>())
        && let Some(item) = native_item_presentation(widget, label.text().as_str(), items)
    {
        let initialized = widget.has_css_class("strata-native-menu-item");
        widget.add_css_class("strata-native-menu-item");
        label.set_hexpand(true);
        if item.send_to_device {
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_max_width_chars(SEND_TO_DEVICE_NAME_MAX_CHARS);
        } else if item.custom {
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            label.set_max_width_chars(24);
        }
        if item.danger {
            widget.add_css_class("danger");
        }
        label_menu_item(widget, &item);
        if !initialized {
            let mapped_item = item.clone();
            widget.connect_map(move |widget| label_menu_item(widget, &mapped_item));
        }
        for image in children
            .iter()
            .filter_map(|child| child.downcast_ref::<gtk::Image>())
        {
            if let Some(icon) = image.gicon().and_downcast::<gio::ThemedIcon>()
                && let Some(name) = icon.names().first()
            {
                bind_menu_icon(image, name);
            } else {
                image.set_pixel_size(item.icon_size);
            }
            image.set_halign(gtk::Align::Center);
            image.set_valign(gtk::Align::Center);
            image.set_margin_end(8);
            // GTK's text-menu presentation hides icons by default.
            image.set_visible(true);
        }
    }
    for child in children {
        present_native_items(&child, root, items, navigation);
    }
}

fn label_menu_item(widget: &gtk::Widget, item: &ItemPresentation) {
    // GTK 4.14's unnamed labelled-by relation otherwise masks the accessible name.
    crate::ui::accessibility::clear_labelled_by(widget);
    crate::ui::accessibility::set_label(widget, &item.label);
    crate::ui::accessibility::set_description(widget, Some(&item.description));
    // GTK renders the `accel` attribute itself; the row shows Strata's label.
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if current.css_name() == "accelerator"
            && let Some(accelerator) = current.downcast_ref::<gtk::Label>()
        {
            accelerator.set_label(&item.shortcut);
        }
    }
}

fn native_item_presentation(
    widget: &gtk::Widget,
    label: &str,
    items: &[ItemPresentation],
) -> Option<ItemPresentation> {
    let popover = widget
        .find_property("popover")
        .and_then(|_| widget.property::<Option<gtk::Popover>>("popover"))
        .and_then(|popover| popover.downcast::<gtk::PopoverMenu>().ok());

    if let Some(popover) = popover {
        let submenu = popover.menu_model()?;
        return items
            .iter()
            .find(|item| item.submenu.as_ref() == Some(&submenu))
            .cloned();
    }

    items.iter().find(|item| item.label == label).cloned()
}

fn bind_menu_icon(image: &gtk::Image, name: &str) {
    let source = assets::primary_icon(name, 15);
    if let Some(texture) = source.paintable().and_downcast::<gtk::gdk::Texture>() {
        image.set_from_gicon(&texture);
    }
    source
        .bind_property("pixel-size", image, "pixel-size")
        .sync_create()
        .build();
    let target = image.downgrade();
    source.connect_paintable_notify(move |source| {
        if let (Some(image), Some(texture)) = (
            target.upgrade(),
            source.paintable().and_downcast::<gtk::gdk::Texture>(),
        ) {
            image.set_from_gicon(&texture);
        }
    });
    image.connect_destroy(move |_| source.clear());
}
