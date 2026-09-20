// SPDX-License-Identifier: MIT

use std::{path::PathBuf, rc::Rc};

use gtk::{gio, prelude::*};

use crate::{
    assets::{self, icons},
    model::{FileEntry, Location, MenuPlacement},
    services::{InvocationSource, MatchedAction},
    ui::actions::{action_icon, folder_input, inputs_for_entries, native_paths, run_action},
};

use super::super::ViewState;

pub(super) struct ActionMenuSection {
    popover: gtk::PopoverMenu,
    model: gio::Menu,
    root_model: gio::Menu,
    header: Option<gtk::Widget>,
    owner: gtk::glib::WeakRef<gtk::Widget>,
    actions: gio::SimpleActionGroup,
    dispatch: super::commands::MenuDispatch,
    _commands: super::commands::CommandMenus,
}

impl ActionMenuSection {
    pub(super) fn new(
        before: &impl IsA<gtk::Widget>,
        after: &impl IsA<gtk::Widget>,
        header: Option<&gtk::Widget>,
        anchor: &gtk::Widget,
    ) -> Self {
        let model = gio::Menu::new();
        let root = gio::Menu::new();
        let popover =
            gtk::PopoverMenu::from_model_full(&gio::Menu::new(), gtk::PopoverMenuFlags::NESTED);
        popover.set_menu_model(gio::MenuModel::NONE);
        popover.set_has_arrow(false);
        popover.add_css_class("folder-context-popover");
        popover.add_css_class("actions-context-popover");
        if header.is_none() {
            popover.add_css_class("folder-menu-body");
        }
        let dispatch = super::commands::MenuDispatch;
        let commands = super::commands::CommandMenus::new(
            before.as_ref(),
            after.as_ref(),
            &popover,
            &dispatch,
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
        for section in &commands.after {
            root.append_section(None, section);
        }
        let actions = gio::SimpleActionGroup::new();
        popover.insert_action_group("custom", Some(&actions));
        super::keyboard::install_menu_edges(&popover);
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
                        && popover.parent().is_some()
                    {
                        popover.unparent();
                    }
                });
            });
            handler.replace(Some(id));
            clock.request_phase(gtk::gdk::FrameClockPhase::AFTER_PAINT);
        });
        // The anchor owns the root menu; GTK owns every generated submenu.
        let weak = popover.downgrade();
        anchor.connect_destroy(move |_| {
            if let Some(popover) = weak.upgrade()
                && popover.parent().is_some()
            {
                popover.unparent();
            }
        });
        refresh_presentation(&popover);
        Self {
            popover,
            model,
            root_model: root,
            header: header.cloned(),
            owner: anchor.downgrade(),
            actions,
            dispatch,
            _commands: commands,
        }
    }

    pub(super) fn popover(&self) -> gtk::Popover {
        self.popover.clone().upcast()
    }

    pub(super) fn show(&self, anchor: &gtk::Widget, x: f64, y: f64) {
        if self.popover.parent().is_none()
            && let Some(owner) = self.owner.upgrade()
        {
            if let Some(overlay) = owner.downcast_ref::<gtk::Overlay>() {
                overlay.add_overlay(&self.popover);
            } else {
                self.popover.set_parent(&owner);
            }
        }
        self._commands.refresh();
        if self.popover.menu_model().is_none() {
            self.popover.set_menu_model(Some(&self.root_model));
            if let Some(header) = &self.header {
                assert!(self.popover.add_child(header, "context-header"));
            }
            refresh_presentation(&self.popover);
        }
        self.popover.set_visible_submenu(Some("main"));
        super::show_model_context_popover(self.popover.upcast_ref(), anchor, x, y);
    }

    pub(super) fn rebuild_for_selection(
        &self,
        state: &Rc<ViewState>,
        entries: &[FileEntry],
        parent: Option<PathBuf>,
    ) {
        let (Some(inputs), Some(paths), Some(parent)) =
            (inputs_for_entries(entries), native_paths(entries), parent)
        else {
            self.clear();
            return;
        };
        let catalog = crate::ui::actions::shared().catalog();
        self.rebuild(
            state,
            &catalog.matches(&inputs),
            paths,
            parent,
            InvocationSource::Selection,
        );
    }

    pub(super) fn rebuild_for_folder(&self, state: &Rc<ViewState>, location: &Location) {
        let (Some(input), Some(path)) = (folder_input(location), location.native_path()) else {
            self.clear();
            return;
        };
        let catalog = crate::ui::actions::shared().catalog();
        self.rebuild(
            state,
            &catalog.matches(std::slice::from_ref(&input)),
            vec![path.to_path_buf()],
            path.to_path_buf(),
            InvocationSource::Background,
        );
    }

    fn clear(&self) {
        self.model.remove_all();
        for name in self.actions.list_actions() {
            self.actions.remove_action(&name);
        }
    }

    fn rebuild(
        &self,
        state: &Rc<ViewState>,
        matched: &[MatchedAction],
        paths: Vec<PathBuf>,
        parent: PathBuf,
        source: InvocationSource,
    ) {
        self.clear();
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
                        run_action(&state.overlay, handle, paths, parent, source);
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
                item.set_attribute_value("x-strata-tooltip", Some(&hint.to_variant()));
            }
            let model = if matched.placement == MenuPlacement::Top {
                &self.model
            } else {
                &submenu
            };
            model.append_item(&item);
        }
        if submenu.n_items() > 0 {
            let item = gio::MenuItem::new_submenu(Some("Actions"), &submenu);
            item.set_icon(&gio::ThemedIcon::new(icons::PLAY));
            self.model.append_item(&item);
        }
        refresh_presentation(&self.popover);
    }
}

pub(super) fn refresh_presentation(root: &gtk::PopoverMenu) {
    let mut items = Vec::new();
    if let Some(model) = root.menu_model() {
        collect_presentations(&model, &mut items);
    }
    present_native_items(root.upcast_ref(), root, &items);
}

#[derive(Clone)]
struct ItemPresentation {
    label: String,
    description: String,
    tooltip: Option<String>,
    icon_size: i32,
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
        if let Some(label) = string("label") {
            items.push(ItemPresentation {
                label: label.replace("__", "_"),
                description: string("x-strata-description").unwrap_or_default(),
                tooltip: string("x-strata-tooltip"),
                icon_size: model
                    .item_attribute_value(index, "x-strata-icon-size", None)
                    .and_then(|value| value.get::<i32>())
                    .unwrap_or(15),
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
fn present_native_items(widget: &gtk::Widget, root: &gtk::PopoverMenu, items: &[ItemPresentation]) {
    if widget.is::<gtk::Button>() {
        return;
    }
    if let Some(menu) = widget.downcast_ref::<gtk::PopoverMenu>() {
        menu.add_css_class("folder-context-popover");
        menu.add_css_class("actions-context-popover");
        if menu != root && !menu.has_css_class("actions-submenu") {
            menu.add_css_class("actions-submenu");
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
                crate::ui::preferences::PreferenceManager::shared().bind_interface_scale(
                    scroll,
                    |scroll, scale| {
                        if let Some(scroll) = scroll.downcast_ref::<gtk::ScrolledWindow>() {
                            let width = (310.0 * scale).round() as i32;
                            scroll.set_min_content_width(width);
                            scroll.set_max_content_width(width);
                        }
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
        && let Some(item) = items
            .iter()
            .find(|item| item.label == label.text())
            .cloned()
    {
        let initialized = widget.has_css_class("strata-native-menu-item");
        widget.add_css_class("strata-native-menu-item");
        label.set_hexpand(true);
        if item.custom {
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            label.set_max_width_chars(24);
        }
        if item.danger {
            widget.add_css_class("danger");
        }
        widget.set_tooltip_text(item.tooltip.as_deref());
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
        present_native_items(&child, root, items);
    }
}

fn label_menu_item(widget: &gtk::Widget, item: &ItemPresentation) {
    // GTK 4.14's unnamed labelled-by relation otherwise masks the accessible name.
    widget.reset_relation(gtk::AccessibleRelation::LabelledBy);
    widget.update_property(&[
        gtk::accessible::Property::Label(&item.label),
        gtk::accessible::Property::Description(&item.description),
    ]);
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
