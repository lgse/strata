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
    pub transfer_sections: Option<[gio::Menu; 2]>,
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
        transfer_buttons: Option<&[gtk::Button; 2]>,
    ) -> Self {
        let group = gio::SimpleActionGroup::new();
        let mut actions = Vec::new();
        let mut refresh = Vec::new();
        let (before_models, _) = sections(
            before,
            SectionContext {
                group: &group,
                actions: &mut actions,
                popover,
                updates: &mut refresh,
                dispatch,
                navigation,
                transfer_buttons: None,
            },
        );
        let (after_models, transfer_sections) = sections(
            after,
            SectionContext {
                group: &group,
                actions: &mut actions,
                popover,
                updates: &mut refresh,
                dispatch,
                navigation,
                transfer_buttons,
            },
        );
        let transfer_sections = match transfer_sections {
            [Some(single), Some(multiple)] => Some([single, multiple]),
            _ => None,
        };
        Self {
            before: before_models,
            after: after_models,
            group,
            transfer_sections,
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

struct SectionContext<'a> {
    group: &'a gio::SimpleActionGroup,
    actions: &'a mut Vec<gio::SimpleAction>,
    popover: &'a gtk::PopoverMenu,
    updates: &'a mut Vec<Rc<dyn Fn(bool)>>,
    dispatch: &'a MenuDispatch,
    navigation: &'a Rc<super::keyboard::NativeMenuNavigation>,
    transfer_buttons: Option<&'a [gtk::Button; 2]>,
}

struct RowContext<'a> {
    group: &'a gio::SimpleActionGroup,
    actions: &'a mut Vec<gio::SimpleAction>,
    popover: &'a gtk::PopoverMenu,
    updates: &'a mut Vec<Rc<dyn Fn(bool)>>,
    dispatch: &'a MenuDispatch,
    navigation: &'a Rc<super::keyboard::NativeMenuNavigation>,
    transfer_buttons: Option<&'a [gtk::Button; 2]>,
    transfer_sections: [Option<gio::Menu>; 2],
}

enum Row {
    Separator,
    Button(gtk::Button),
    Submenu {
        header: gtk::Button,
        entries: Vec<gtk::Button>,
    },
}

fn sections(
    source: &gtk::Widget,
    context: SectionContext<'_>,
) -> (Vec<gio::Menu>, [Option<gio::Menu>; 2]) {
    let SectionContext {
        group,
        actions,
        popover,
        updates,
        dispatch,
        navigation,
        transfer_buttons,
    } = context;
    let mut rows = Vec::new();
    collect(source, &mut rows);
    let mut sections = vec![gio::Menu::new()];
    let mut row_context = RowContext {
        group,
        actions,
        popover,
        updates,
        dispatch,
        navigation,
        transfer_buttons,
        transfer_sections: [None, None],
    };
    for row in rows {
        match row {
            Row::Separator => {
                if sections.last().is_some_and(|section| section.n_items() > 0) {
                    sections.push(gio::Menu::new());
                }
            }
            Row::Button(button) => {
                let section = sections.last().expect("initial section");
                append_row_model(section, &button, None, &mut row_context);
            }
            Row::Submenu { header, entries } => {
                let entries_model = gio::Menu::new();
                for entry in &entries {
                    append_row_model(&entries_model, entry, None, &mut row_context);
                }
                let section = sections.last().expect("initial section");
                append_row_model(section, &header, Some(&entries_model), &mut row_context);
            }
        }
    }
    sections.retain(|section| section.n_items() > 0);
    (sections, row_context.transfer_sections)
}

fn append_row_model(
    model: &gio::Menu,
    button: &gtk::Button,
    submenu: Option<&gio::Menu>,
    context: &mut RowContext<'_>,
) {
    let name = format!("item-{}", context.actions.len());
    let action = gio::SimpleAction::new(&name, None);
    button
        .bind_property("sensitive", &action, "enabled")
        .sync_create()
        .build();
    let activated = button.clone();
    let dispatch = context.dispatch.clone();
    action.connect_activate(move |_, _| {
        let activated = activated.clone();
        dispatch.defer(move || activated.emit_clicked());
    });
    let item = gio::MenuItem::new(None, Some(&format!("builtin.{name}")));
    item.set_attribute_value("hidden-when", Some(&"action-missing".to_variant()));
    update_item(&item, button);
    if let Some(submenu) = submenu {
        item.set_submenu(Some(submenu));
    }
    let index = model.n_items();
    model.append_item(&item);
    if let Some(transfer_index) = context
        .transfer_buttons
        .and_then(|buttons| buttons.iter().position(|candidate| candidate == button))
    {
        context.transfer_sections[transfer_index] = Some(model.clone());
    }

    let weak_button = button.downgrade();
    let weak_section = model.downgrade();
    let weak_popover = context.popover.downgrade();
    let navigation_for_refresh = context.navigation.clone();
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
    if let Some(option) = button.downcast_ref::<super::presentation::MenuOption>() {
        let description_refresh = refresh.clone();
        option.connect_menu_description_notify(move |_| description_refresh(false));
    }
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
    let weak_group = context.group.downgrade();
    let weak_action = action.downgrade();
    let weak_popover = context.popover.downgrade();
    let navigation = context.navigation.clone();
    context.updates.push(refresh);
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
    context.updates.push(visibility);
    context.actions.push(action);
}

fn collect(widget: &gtk::Widget, rows: &mut Vec<Row>) {
    if let Some(button) = widget.downcast_ref::<gtk::Button>() {
        rows.push(Row::Button(button.clone()));
        return;
    }
    if widget.is::<gtk::Separator>() {
        rows.push(Row::Separator);
        return;
    }
    if let Some(row) = submenu_row(widget) {
        rows.push(row);
        return;
    }
    let mut next = widget.first_child();
    while let Some(child) = next {
        next = child.next_sibling();
        collect(&child, rows);
    }
}

fn submenu_row(widget: &gtk::Widget) -> Option<Row> {
    if !widget.has_css_class(super::SUBMENU_ROW_CLASS) {
        return None;
    }
    let mut header = None;
    let mut entries = Vec::new();
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if let Some(button) = current.downcast_ref::<gtk::Button>() {
            header.get_or_insert_with(|| button.clone());
        } else if current.has_css_class(super::SUBMENU_ENTRIES_CLASS) {
            let mut entry = current.first_child();
            while let Some(item) = entry {
                entry = item.next_sibling();
                if let Some(button) = item.downcast_ref::<gtk::Button>() {
                    entries.push(button.clone());
                }
            }
        }
    }
    Some(Row::Submenu {
        header: header?,
        entries,
    })
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
    let description = button
        .downcast_ref::<super::presentation::MenuOption>()
        .map(|option| option.menu_description())
        .filter(|description| !description.is_empty())
        .unwrap_or_else(|| shortcut.to_owned());
    item.set_attribute_value("x-strata-description", Some(&description.to_variant()));
    item.set_attribute_value(
        "x-strata-danger",
        Some(&button.has_css_class("danger").to_variant()),
    );
    if shortcut.is_empty() {
        item.set_attribute_value("accel", None);
        return;
    }
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
        "<Control>Enter" => "<Control>Return".to_owned(),
        "<Shift>Enter" => "<Shift>Return".to_owned(),
        _ => accelerator,
    };
    item.set_attribute_value("accel", Some(&accelerator.to_variant()));
}
