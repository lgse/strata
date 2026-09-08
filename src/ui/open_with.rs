// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};

use super::{
    blur::BlurBin,
    browser::{dismiss_modal_layer, modal_layer, show_error_dialog},
    controls::modal_layout,
};

pub(super) fn compatible_apps(content_type: &str, requires_uris: bool) -> Vec<gio::AppInfo> {
    let default = gio::AppInfo::default_for_type(content_type, requires_uris);
    filter_apps(
        gio::AppInfo::all_for_type(content_type),
        default,
        requires_uris,
    )
}

fn filter_apps(
    apps: Vec<gio::AppInfo>,
    default: Option<gio::AppInfo>,
    requires_uris: bool,
) -> Vec<gio::AppInfo> {
    let mut apps = apps
        .into_iter()
        .filter(|app| app.should_show() && (!requires_uris || app.supports_uris()))
        .collect::<Vec<_>>();
    apps.sort_by_cached_key(|app| app.display_name().to_lowercase());
    let mut unique = Vec::with_capacity(apps.len());
    if let Some(default) = default.filter(|app| !requires_uris || app.supports_uris()) {
        unique.push(default);
    }
    for app in apps {
        if !unique.iter().any(|existing| existing.equal(&app)) {
            unique.push(app);
        }
    }
    unique
}

pub(super) fn launch(
    app: &gio::AppInfo,
    files: &[gio::File],
    context: Option<&impl IsA<gio::AppLaunchContext>>,
) -> Result<(), glib::Error> {
    // GIO can silently drop non-native files when expanding %f/%F.
    if !app.supports_uris() && files.iter().any(|file| !file.is_native()) {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            "This application cannot open files at this location",
        ));
    }
    app.launch(files, context)
}

fn application_icon(app: &gio::AppInfo, display: &gtk::gdk::Display) -> gtk::Image {
    let icon = app.icon().and_then(|icon| {
        if let Some(file_icon) = icon.downcast_ref::<gio::FileIcon>() {
            return gtk::gdk::Texture::from_file(&file_icon.file())
                .ok()
                .map(|texture| gtk::Image::from_paintable(Some(&texture)));
        }
        gtk::IconTheme::for_display(display)
            .has_gicon(&icon)
            .then(|| gtk::Image::from_gicon(&icon))
    });
    icon.unwrap_or_else(|| crate::assets::primary_icon(crate::assets::icons::FILE_CODE, 24))
}

fn install_list_tab_navigation(
    content: &gtk::Box,
    list: &gtk::ListBox,
    close: &gtk::Button,
    cancel: &gtk::Button,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let list = list.downgrade();
    let close = close.downgrade();
    let cancel = cancel.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if !matches!(key, gtk::gdk::Key::Tab | gtk::gdk::Key::ISO_Left_Tab)
            || modifiers.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK,
            )
        {
            return glib::Propagation::Proceed;
        }
        let (Some(list), Some(close), Some(cancel)) =
            (list.upgrade(), close.upgrade(), cancel.upgrade())
        else {
            return glib::Propagation::Proceed;
        };
        let Some(focus) = list.root().and_then(|root| root.focus()) else {
            return glib::Propagation::Proceed;
        };
        let backward = key == gtk::gdk::Key::ISO_Left_Tab
            || modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        let moved = if focus == list || focus.is_ancestor(&list) {
            if backward {
                close.grab_focus()
            } else {
                cancel.grab_focus()
            }
        } else if (backward && focus == cancel) || (!backward && focus == close) {
            list.selected_row().is_some_and(|row| row.grab_focus())
        } else {
            false
        };
        if moved {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    content.add_controller(keys);
}

pub(super) fn show(
    parent: &impl IsA<gtk::Widget>,
    files: Vec<gio::File>,
    apps: Vec<gio::AppInfo>,
    on_close: Rc<dyn Fn()>,
) {
    let Some(window_overlay) = parent
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.child())
        .and_downcast::<gtk::Overlay>()
    else {
        return;
    };
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    if let Some(root) = blurred_root.as_ref() {
        root.set_blurred(true);
    }

    let subtitle = if files.len() == 1 {
        "Choose an application to open this item"
    } else {
        "Choose an application to open these items"
    };
    let layout = modal_layout(
        crate::assets::icons::EXTERNAL_LINK,
        "Open With",
        subtitle,
        "Open",
    );
    layout.content.add_css_class("open-with-dialog");

    let list = gtk::ListBox::new();
    list.add_css_class("open-with-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(false);
    list.set_vexpand(true);
    list.update_property(&[gtk::accessible::Property::Label("Applications")]);
    install_list_tab_navigation(&layout.content, &list, &layout.close, &layout.cancel);

    for app in &apps {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("open-with-row");
        row.update_property(&[gtk::accessible::Property::Label(&app.display_name())]);
        if let Some(description) = app.description() {
            row.update_property(&[gtk::accessible::Property::Description(&description)]);
        }
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let icon = application_icon(app, &list.display());
        icon.set_pixel_size(24);
        icon.add_css_class("open-with-icon");
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 1);
        labels.set_valign(gtk::Align::Center);
        let name = gtk::Label::new(Some(&app.display_name()));
        name.add_css_class("open-with-name");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let description_text = app.description().map(|value| value.to_string());
        let description = gtk::Label::new(description_text.as_deref());
        description.add_css_class("open-with-description");
        description.set_xalign(0.0);
        description.set_ellipsize(gtk::pango::EllipsizeMode::End);
        labels.append(&name);
        if description_text
            .as_deref()
            .is_some_and(|value| !value.is_empty() && value != app.display_name())
        {
            labels.append(&description);
        }
        content.append(&icon);
        content.append(&labels);
        row.set_child(Some(&content));
        list.append(&row);
    }
    if let Some(row) = list.row_at_index(0) {
        list.select_row(Some(&row));
    }
    let list_scroll = gtk::ScrolledWindow::new();
    list_scroll.add_css_class("open-with-scroll");
    list_scroll.set_child(Some(&list));
    list_scroll.set_min_content_height(52);
    list_scroll.set_max_content_height(6 * 52 + 8);
    list_scroll.set_propagate_natural_height(true);
    list_scroll.set_vexpand(true);
    list_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    layout.body.append(&list_scroll);

    let apps_empty = apps.is_empty();
    if apps_empty {
        list_scroll.set_visible(false);
        let empty = gtk::Label::new(Some("No compatible applications were found."));
        empty.add_css_class("open-with-empty");
        empty.set_wrap(true);
        empty.set_xalign(0.0);
        layout.body.append(&empty);
        layout.confirm.set_sensitive(false);
    }

    let layer = modal_layer(&layout.content, &window_overlay, blurred_root.clone(), None);
    layer.connect_unrealize(move |_| {
        let on_close = on_close.clone();
        glib::idle_add_local_once(move || on_close());
    });
    window_overlay.add_overlay(&layer);
    let dismissed = Rc::new(Cell::new(false));
    let dismiss_layer = layer.downgrade();
    let dismiss_overlay = window_overlay.downgrade();
    let dismiss_root = blurred_root.as_ref().map(|root| root.downgrade());
    let dismiss = Rc::new(move || {
        if dismissed.replace(true) {
            return;
        }
        let (Some(layer), Some(overlay)) = (dismiss_layer.upgrade(), dismiss_overlay.upgrade())
        else {
            return;
        };
        let root = dismiss_root.as_ref().and_then(|root| root.upgrade());
        dismiss_modal_layer(&layer, &overlay, root.as_ref());
    });

    let cancel_dismiss = dismiss.clone();
    layout.cancel.connect_clicked(move |_| cancel_dismiss());
    let close_dismiss = dismiss.clone();
    layout.close.connect_clicked(move |_| close_dismiss());

    let open_dismiss = dismiss.clone();
    let open_files = files;
    let open_apps = apps;
    let open_parent = parent.as_ref().downgrade();
    let selected_list = list.downgrade();
    layout.confirm.connect_clicked(move |_| {
        let Some(list) = selected_list.upgrade() else {
            return;
        };
        let Some(row) = list.selected_row() else {
            return;
        };
        let index = row.index();
        let Some(app) = open_apps.get(index as usize) else {
            return;
        };
        let context = list.display().app_launch_context();
        if let Err(error) = launch(app, &open_files, Some(&context)) {
            let detail = error.to_string();
            open_dismiss();
            let open_parent = open_parent.clone();
            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                if let Some(parent) = open_parent.upgrade() {
                    show_error_dialog(&parent, "Unable to open file", &detail);
                }
            });
            return;
        }
        open_dismiss();
    });
    let activate_confirm = layout.confirm.downgrade();
    list.connect_row_activated(move |_, _| {
        if let Some(confirm) = activate_confirm.upgrade() {
            confirm.emit_clicked();
        }
    });

    let escape_dismiss = dismiss.clone();
    let escape = gtk::EventControllerKey::new();
    escape.set_propagation_phase(gtk::PropagationPhase::Capture);
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            escape_dismiss();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);
    if apps_empty {
        layout.cancel.grab_focus();
    } else if let Some(row) = list.selected_row() {
        row.grab_focus();
    }
}

#[cfg(test)]
mod tests;
