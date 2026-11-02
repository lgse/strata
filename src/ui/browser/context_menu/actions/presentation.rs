// SPDX-License-Identifier: MIT

use gtk::{gio, prelude::*};
use std::rc::Rc;

use crate::assets;
use crate::ui::browser::context_menu::keyboard::{self, NativeMenuNavigation};

const SEND_TO_DEVICE_NAME_MAX_CHARS: i32 = 15;

pub(in crate::ui::browser::context_menu) fn refresh_presentation(
    root: &gtk::PopoverMenu,
    navigation: &Rc<NativeMenuNavigation>,
) {
    let mut items = Vec::new();
    if let Some(model) = root.menu_model() {
        collect_presentations(&model, &mut items);
    }
    present_native_items(root.upcast_ref(), root, &items, navigation);
}

#[derive(Clone)]
pub(super) struct ItemPresentation {
    pub(super) label: String,
    description: String,
    shortcut: String,
    submenu: Option<gio::MenuModel>,
    icon_size: i32,
    send_to_device: bool,
    danger: bool,
    custom: bool,
}

fn item_attribute<T: gtk::glib::variant::FromVariant>(
    model: &gio::MenuModel,
    index: i32,
    name: &str,
) -> Option<T> {
    model
        .item_attribute_value(index, name, None)
        .and_then(|value| value.get::<T>())
}

fn item_presentation(model: &gio::MenuModel, index: i32, label: String) -> ItemPresentation {
    let string = |name| item_attribute::<String>(model, index, name);
    ItemPresentation {
        label: label.replace("__", "_"),
        description: string("x-strata-description").unwrap_or_default(),
        shortcut: string("x-strata-shortcut").unwrap_or_default(),
        submenu: model.item_link(index, "submenu"),
        icon_size: item_attribute(model, index, "x-strata-icon-size").unwrap_or(15),
        send_to_device: item_attribute(model, index, "x-strata-send-to-device").unwrap_or(false),
        danger: item_attribute(model, index, "x-strata-danger").unwrap_or(false),
        custom: string("action").is_some_and(|name| name.starts_with("custom.")),
    }
}

pub(super) fn collect_presentations(model: &gio::MenuModel, items: &mut Vec<ItemPresentation>) {
    for index in 0..model.n_items() {
        if let Some(label) = item_attribute::<String>(model, index, "label")
            && model.item_link(index, "section").is_none()
        {
            items.push(item_presentation(model, index, label));
        }
        for link in ["section", "submenu"] {
            if let Some(child) = model.item_link(index, link) {
                collect_presentations(&child, items);
            }
        }
    }
}

fn owns_popover(widget: &gtk::Widget, submenu: &gtk::Popover) -> bool {
    widget.find_property("popover").is_some()
        && widget.property::<Option<gtk::Popover>>("popover").as_ref() == Some(submenu)
}

fn find_submenu_owner(
    widget: &gtk::Widget,
    root: &gtk::Widget,
    submenu: &gtk::Popover,
) -> Option<gtk::Widget> {
    if widget != root && widget.is::<gtk::Popover>() {
        return None;
    }
    if owns_popover(widget, submenu) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(owner) = find_submenu_owner(&widget, root, submenu) {
            return Some(owner);
        }
        child = widget.next_sibling();
    }
    None
}

// Preserve generated rows: GTK uses them to own submenu selection and grabs.
fn submenu_owner(root: &gtk::PopoverMenu, submenu: &gtk::PopoverMenu) -> Option<gtk::Widget> {
    let root_widget = root.upcast_ref::<gtk::Widget>();
    let submenu = submenu.clone().upcast::<gtk::Popover>();
    find_submenu_owner(root_widget, root_widget, &submenu)
}

fn focus_left_menu(menu: &gtk::PopoverMenu) -> bool {
    let Some(widget) = menu.root().and_then(|root| root.focus()) else {
        return false;
    };
    !widget.is_ancestor(menu) && !menu.is_ancestor(&widget)
}

/// Older GTK emits focus leave before updating contains-focus.
fn close_submenu_when_focus_leaves(menu: &gtk::PopoverMenu) {
    let focus = gtk::EventControllerFocus::new();
    focus.set_propagation_limit(gtk::PropagationLimit::None);
    let weak = menu.downgrade();
    focus.connect_contains_focus_notify(move |focus| {
        if focus.contains_focus() {
            return;
        }
        let weak = weak.clone();
        gtk::glib::idle_add_local_once(move || {
            if let Some(menu) = weak.upgrade()
                && focus_left_menu(&menu)
            {
                menu.set_visible(false);
            }
        });
    });
    menu.add_controller(focus);
}

fn style_popover_menu(
    menu: &gtk::PopoverMenu,
    root: &gtk::PopoverMenu,
    navigation: &Rc<NativeMenuNavigation>,
) {
    menu.add_css_class("folder-context-popover");
    menu.add_css_class("actions-context-popover");
    if menu == root || menu.has_css_class("actions-submenu") {
        return;
    }
    menu.add_css_class("actions-submenu");
    if let Some(owner) = submenu_owner(root, menu) {
        keyboard::install_submenu_return(menu, &owner, navigation);
    }
    if gtk::minor_version() < 22 {
        close_submenu_when_focus_leaves(menu);
    }
}

fn style_menu_scroll(scroll: &gtk::ScrolledWindow, root: &gtk::PopoverMenu) {
    if !scroll.has_css_class("context-menu-scroll") {
        scroll.add_css_class("context-menu-scroll");
        let is_root =
            scroll.ancestor(gtk::PopoverMenu::static_type()).as_ref() == Some(root.upcast_ref());
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

fn widget_children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut children = Vec::new();
    let mut next = widget.first_child();
    while let Some(child) = next {
        next = child.next_sibling();
        children.push(child);
    }
    children
}

fn style_item_label(label: &gtk::Label, item: &ItemPresentation) {
    label.set_hexpand(true);
    if item.send_to_device {
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(SEND_TO_DEVICE_NAME_MAX_CHARS);
    } else if item.custom {
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        label.set_max_width_chars(24);
    }
}

fn style_item_image(image: &gtk::Image, item: &ItemPresentation) {
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

fn present_menu_item(widget: &gtk::Widget, children: &[gtk::Widget], items: &[ItemPresentation]) {
    if widget.accessible_role() != gtk::AccessibleRole::MenuItem {
        return;
    }
    let Some(label) = children
        .iter()
        .find_map(|child| child.downcast_ref::<gtk::Label>())
    else {
        return;
    };
    let Some(item) = native_item_presentation(widget, label.text().as_str(), items) else {
        return;
    };
    let initialized = widget.has_css_class("strata-native-menu-item");
    widget.add_css_class("strata-native-menu-item");
    style_item_label(label, &item);
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
        style_item_image(image, &item);
    }
}

fn present_native_items(
    widget: &gtk::Widget,
    root: &gtk::PopoverMenu,
    items: &[ItemPresentation],
    navigation: &Rc<NativeMenuNavigation>,
) {
    if widget.is::<gtk::Button>() {
        return;
    }
    if let Some(menu) = widget.downcast_ref::<gtk::PopoverMenu>() {
        style_popover_menu(menu, root, navigation);
    }
    if let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>() {
        style_menu_scroll(scroll, root);
    }
    let children = widget_children(widget);
    present_menu_item(widget, &children, items);
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
