// SPDX-License-Identifier: MIT

use std::rc::Rc;

#[derive(Clone, Default)]
pub(super) struct MenuDispatch;

impl MenuDispatch {
    pub fn defer(&self, command: impl FnOnce() + 'static) {
        gtk::glib::idle_add_local_once(command);
    }
}
use gtk::{gio, prelude::*};

pub(super) struct CommandMenus {
    pub before: Vec<gio::Menu>,
    pub after: Vec<gio::Menu>,
    pub group: gio::SimpleActionGroup,
    _actions: Vec<gio::SimpleAction>,
    _controls: [gtk::Widget; 2],
    refresh: Vec<Rc<dyn Fn(bool)>>,
}

impl CommandMenus {
    pub fn new(
        before: &gtk::Widget,
        after: &gtk::Widget,
        popover: &gtk::PopoverMenu,
        dispatch: &MenuDispatch,
        navigation: &Rc<super::keyboard::NativeMenuNavigation>,
    ) -> Self {
        let group = gio::SimpleActionGroup::new();
        let mut actions = Vec::new();
        let mut refresh = Vec::new();
        let before_models = sections(
            before,
            &group,
            &mut actions,
            popover,
            &mut refresh,
            dispatch,
            navigation,
        );
        let after_models = sections(
            after,
            &group,
            &mut actions,
            popover,
            &mut refresh,
            dispatch,
            navigation,
        );
        Self {
            before: before_models,
            after: after_models,
            group,
            _actions: actions,
            _controls: [before.clone(), after.clone()],
            refresh,
        }
    }

    pub fn refresh(&self) {
        for refresh in &self.refresh {
            refresh(true);
        }
    }
}

fn sections(
    source: &gtk::Widget,
    group: &gio::SimpleActionGroup,
    actions: &mut Vec<gio::SimpleAction>,
    popover: &gtk::PopoverMenu,
    updates: &mut Vec<Rc<dyn Fn(bool)>>,
    dispatch: &MenuDispatch,
    navigation: &Rc<super::keyboard::NativeMenuNavigation>,
) -> Vec<gio::Menu> {
    let mut rows = Vec::new();
    collect(source, &mut rows);
    let mut sections = vec![gio::Menu::new()];
    for row in rows {
        let Some(button) = row else {
            if sections.last().is_some_and(|section| section.n_items() > 0) {
                sections.push(gio::Menu::new());
            }
            continue;
        };
        let section = sections.last().expect("initial section");
        let name = format!("item-{}", actions.len());
        let action = gio::SimpleAction::new(&name, None);
        button
            .bind_property("sensitive", &action, "enabled")
            .sync_create()
            .build();
        let activated = button.clone();
        let dispatch = dispatch.clone();
        action.connect_activate(move |_, _| {
            let activated = activated.clone();
            dispatch.defer(move || activated.emit_clicked());
        });
        let item = gio::MenuItem::new(None, Some(&format!("builtin.{name}")));
        item.set_attribute_value("hidden-when", Some(&"action-missing".to_variant()));
        update_item(&item, &button);
        let index = section.n_items();
        section.append_item(&item);

        let weak_button = button.downgrade();
        let weak_section = section.downgrade();
        let weak_popover = popover.downgrade();
        let navigation_for_refresh = navigation.clone();
        let refresh: Rc<dyn Fn(bool)> = Rc::new(move |force| {
            if !force
                && !weak_popover
                    .upgrade()
                    .is_some_and(|popover| popover.is_visible())
            {
                return;
            }
            if let (Some(button), Some(section)) = (weak_button.upgrade(), weak_section.upgrade()) {
                update_item(&item, &button);
                section.remove(index);
                section.insert_item(index, &item);
                if let Some(popover) = weak_popover.upgrade() {
                    super::actions::refresh_presentation(&popover, &navigation_for_refresh);
                    navigation_for_refresh.model_changed();
                }
            }
        });
        let tooltip_refresh = refresh.clone();
        button.connect_tooltip_text_notify(move |_| tooltip_refresh(false));
        if let Some(row) = button.child() {
            let mut child = row.first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                    let refresh = refresh.clone();
                    label.connect_label_notify(move |_| refresh(false));
                } else if let Some(image) = widget.downcast_ref::<gtk::Image>() {
                    let paint = refresh.clone();
                    image.connect_paintable_notify(move |_| paint(false));
                    let size = refresh.clone();
                    image.connect_pixel_size_notify(move |_| size(false));
                }
            }
        }
        let weak_button = button.downgrade();
        let weak_group = group.downgrade();
        let weak_action = action.downgrade();
        let weak_popover = popover.downgrade();
        let navigation = navigation.clone();
        updates.push(refresh);
        let visibility: Rc<dyn Fn(bool)> = Rc::new(move |force| {
            if !force
                && !weak_popover
                    .upgrade()
                    .is_some_and(|popover| popover.is_visible())
            {
                return;
            }
            if let (Some(button), Some(group), Some(action)) = (
                weak_button.upgrade(),
                weak_group.upgrade(),
                weak_action.upgrade(),
            ) {
                let mut node = Some(button.upcast::<gtk::Widget>());
                let mut visible = true;
                while let Some(widget) = node {
                    visible &= widget.get_visible();
                    node = widget.parent();
                }
                if visible && !group.has_action(action.name().as_str()) {
                    group.add_action(&action);
                } else if !visible && group.has_action(action.name().as_str()) {
                    group.remove_action(action.name().as_str());
                }
                if let Some(popover) = weak_popover.upgrade() {
                    super::actions::refresh_presentation(&popover, &navigation);
                    navigation.model_changed();
                }
            }
        });
        let mut ancestor = Some(button.clone().upcast::<gtk::Widget>());
        while let Some(widget) = ancestor {
            ancestor = widget.parent();
            let visibility = visibility.clone();
            widget.connect_visible_notify(move |_| visibility(false));
        }
        visibility(true);
        updates.push(visibility);
        actions.push(action);
    }
    sections.retain(|section| section.n_items() > 0);
    sections
}

fn collect(widget: &gtk::Widget, rows: &mut Vec<Option<gtk::Button>>) {
    if let Some(button) = widget.downcast_ref::<gtk::Button>() {
        rows.push(Some(button.clone()));
        return;
    }
    if widget.is::<gtk::Separator>() {
        rows.push(None);
        return;
    }
    let mut next = widget.first_child();
    while let Some(child) = next {
        next = child.next_sibling();
        collect(&child, rows);
    }
}

fn update_item(item: &gio::MenuItem, button: &gtk::Button) {
    let mut labels = Vec::new();
    if let Some(row) = button.child() {
        let mut next = row.first_child();
        while let Some(child) = next {
            next = child.next_sibling();
            if let Some(label) = child.downcast_ref::<gtk::Label>() {
                labels.push(label.text().to_string());
            } else if let Some(image) = child.downcast_ref::<gtk::Image>()
                && let Some(texture) = image.paintable().and_downcast::<gtk::gdk::Texture>()
            {
                item.set_icon(&texture);
                item.set_attribute_value(
                    "x-strata-icon-size",
                    Some(&image.pixel_size().to_variant()),
                );
            }
        }
    }
    if let Some(label) = labels.first() {
        item.set_label(Some(&label.replace('_', "__")));
    }
    let shortcut = labels.get(1).map(String::as_str).unwrap_or("");
    let tooltip = button.tooltip_text();
    let description = tooltip.as_deref().unwrap_or(shortcut);
    item.set_attribute_value("x-strata-description", Some(&description.to_variant()));
    item.set_attribute_value(
        "x-strata-tooltip",
        tooltip.as_ref().map(|text| text.to_variant()).as_ref(),
    );
    item.set_attribute_value(
        "x-strata-danger",
        Some(&button.has_css_class("danger").to_variant()),
    );
    if !shortcut.is_empty() {
        let accelerator = shortcut
            .split(" / ")
            .next()
            .unwrap_or(shortcut)
            .replace("Ctrl+", "<Control>")
            .replace("Shift+", "<Shift>")
            .replace("Alt+", "<Alt>")
            .replace('↵', "Return");
        let accelerator = match accelerator.as_str() {
            "Del" => "Delete".to_owned(),
            "Enter" => "Return".to_owned(),
            "Space" => "space".to_owned(),
            "<Shift>Del" => "<Shift>Delete".to_owned(),
            _ => accelerator,
        };
        item.set_attribute_value("accel", Some(&accelerator.to_variant()));
    }
}
