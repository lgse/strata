// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};

use super::{
    blur::BlurBin,
    browser::{dismiss_modal_layer, modal_layer, show_error_dialog},
    controls::modal_layout,
};

pub(super) fn categorized_apps(
    content_type: &str,
    requires_uris: bool,
) -> (Vec<gio::AppInfo>, Vec<gio::AppInfo>) {
    let default = gio::AppInfo::default_for_type(content_type, requires_uris);
    let recommended = filter_apps(
        gio::AppInfo::all_for_type(content_type),
        default,
        requires_uris,
    );
    let other = filter_other_apps(gio::AppInfo::all(), &recommended, requires_uris);
    (recommended, other)
}

// Non-native GVfs files can still provide FUSE paths for %f/%F handlers.
pub(super) fn requires_uri_handlers(files: &[gio::File]) -> bool {
    files
        .iter()
        .any(|file| path_requires_uri_handlers(file.path().as_deref()))
}

fn path_requires_uri_handlers(path: Option<&std::path::Path>) -> bool {
    path.is_none()
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

pub(super) fn filter_other_apps(
    apps: Vec<gio::AppInfo>,
    recommended: &[gio::AppInfo],
    requires_uris: bool,
) -> Vec<gio::AppInfo> {
    let mut apps = apps
        .into_iter()
        .filter(|app| {
            app.should_show()
                && (!requires_uris || app.supports_uris())
                && !recommended.iter().any(|rec| rec.equal(app))
        })
        .collect::<Vec<_>>();
    apps.sort_by_cached_key(|app| app.display_name().to_lowercase());
    let mut unique: Vec<gio::AppInfo> = Vec::with_capacity(apps.len());
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
    // GIO drops files without a local path when expanding %f/%F.
    if !app.supports_uris() && requires_uri_handlers(files) {
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
    search_entry: &gtk::SearchEntry,
    list: &gtk::ListBox,
    close: &gtk::Button,
    cancel: &gtk::Button,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let search = search_entry.downgrade();
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
        let (Some(search), Some(list), Some(close), Some(cancel)) = (
            search.upgrade(),
            list.upgrade(),
            close.upgrade(),
            cancel.upgrade(),
        ) else {
            return glib::Propagation::Proceed;
        };
        let Some(focus) = list.root().and_then(|root| root.focus()) else {
            return glib::Propagation::Proceed;
        };
        let backward = key == gtk::gdk::Key::ISO_Left_Tab
            || modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        let focus_selected_row = || {
            if list.is_visible() {
                list.selected_row().is_some_and(|row| row.grab_focus())
            } else {
                false
            }
        };
        let moved = if focus == search || focus.is_ancestor(&search) {
            if backward {
                close.grab_focus()
            } else if focus_selected_row() {
                true
            } else {
                cancel.grab_focus()
            }
        } else if focus == list || focus.is_ancestor(&list) {
            if backward {
                if search.is_visible() {
                    search.grab_focus()
                } else {
                    close.grab_focus()
                }
            } else {
                cancel.grab_focus()
            }
        } else if backward && focus == cancel {
            if focus_selected_row() {
                true
            } else if search.is_visible() {
                search.grab_focus()
            } else {
                close.grab_focus()
            }
        } else if !backward && focus == close {
            if search.is_visible() {
                search.grab_focus()
            } else if focus_selected_row() {
                true
            } else {
                cancel.grab_focus()
            }
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

struct AppEntry {
    app: gio::AppInfo,
    row: gtk::ListBoxRow,
    is_recommended: bool,
    haystack: String,
}

fn create_section_header(title: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("open-with-heading-row");
    row.set_activatable(false);
    row.set_selectable(false);
    row.set_focusable(false);
    row.set_can_focus(false);

    let label = gtk::Label::new(Some(title));
    label.add_css_class("open-with-heading");
    label.set_xalign(0.0);
    row.set_child(Some(&label));
    row
}

fn create_app_row(app: &gio::AppInfo, display: &gtk::gdk::Display) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("open-with-row");
    row.update_property(&[gtk::accessible::Property::Label(&app.display_name())]);
    if let Some(description) = app.description() {
        row.update_property(&[gtk::accessible::Property::Description(&description)]);
    }
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = application_icon(app, display);
    icon.set_pixel_size(22);
    icon.add_css_class("open-with-icon");
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 0);
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
    row
}

pub(super) fn show(
    parent: &impl IsA<gtk::Widget>,
    files: Vec<gio::File>,
    recommended_apps: Vec<gio::AppInfo>,
    other_apps: Vec<gio::AppInfo>,
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
    layout.content.set_size_request(480, 460);
    layout.body.set_vexpand(true);

    let search_entry = gtk::SearchEntry::new();
    search_entry.add_css_class("open-with-search");
    search_entry.set_placeholder_text(Some("Search applications…"));
    layout.body.append(&search_entry);

    let list = gtk::ListBox::new();
    list.add_css_class("open-with-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(false);
    list.update_property(&[gtk::accessible::Property::Label("Applications")]);
    install_list_tab_navigation(
        &layout.content,
        &search_entry,
        &list,
        &layout.close,
        &layout.cancel,
    );

    let mut entries = Vec::new();

    let mut recommended_heading_row = None;
    if !recommended_apps.is_empty() {
        let heading = create_section_header("Recommended Applications");
        list.append(&heading);
        recommended_heading_row = Some(heading);

        for app in &recommended_apps {
            let row = create_app_row(app, &list.display());
            let haystack = format!(
                "{} {} {} {} {}",
                app.display_name(),
                app.name(),
                app.description().unwrap_or_default(),
                app.executable().to_string_lossy(),
                app.id().unwrap_or_default(),
            )
            .to_lowercase();
            list.append(&row);
            entries.push(AppEntry {
                app: app.clone(),
                row,
                is_recommended: true,
                haystack,
            });
        }
    }

    let mut other_heading_row = None;
    if !other_apps.is_empty() {
        let heading = create_section_header("Other Applications");
        list.append(&heading);
        other_heading_row = Some(heading);

        for app in &other_apps {
            let row = create_app_row(app, &list.display());
            let haystack = format!(
                "{} {} {} {} {}",
                app.display_name(),
                app.name(),
                app.description().unwrap_or_default(),
                app.executable().to_string_lossy(),
                app.id().unwrap_or_default(),
            )
            .to_lowercase();
            list.append(&row);
            entries.push(AppEntry {
                app: app.clone(),
                row,
                is_recommended: false,
                haystack,
            });
        }
    }

    if let Some(first_entry) = entries.first() {
        list.select_row(Some(&first_entry.row));
    }

    let list_scroll = gtk::ScrolledWindow::new();
    list_scroll.add_css_class("open-with-scroll");
    list_scroll.set_child(Some(&list));
    list_scroll.set_propagate_natural_height(false);
    list_scroll.set_vexpand(true);
    list_scroll.set_hexpand(true);
    list_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    layout.body.append(&list_scroll);

    let empty_search = gtk::Label::new(Some("No matching applications were found."));
    empty_search.add_css_class("open-with-empty");
    empty_search.set_wrap(true);
    empty_search.set_xalign(0.5);
    empty_search.set_halign(gtk::Align::Center);
    empty_search.set_valign(gtk::Align::Center);
    empty_search.set_vexpand(true);
    empty_search.set_visible(false);
    layout.body.append(&empty_search);

    let has_apps = !entries.is_empty();
    if !has_apps {
        search_entry.set_visible(false);
        list_scroll.set_visible(false);
        let empty = gtk::Label::new(Some("No compatible applications were found."));
        empty.add_css_class("open-with-empty");
        empty.set_wrap(true);
        empty.set_xalign(0.5);
        empty.set_halign(gtk::Align::Center);
        empty.set_valign(gtk::Align::Center);
        empty.set_vexpand(true);
        layout.body.append(&empty);
        layout.confirm.set_sensitive(false);
    }

    let search_keys = gtk::EventControllerKey::new();
    let list_for_search = list.downgrade();
    search_keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Down
            && let Some(list) = list_for_search.upgrade()
            && let Some(row) = list.selected_row()
        {
            row.grab_focus();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    search_entry.add_controller(search_keys);

    let list_keys = gtk::EventControllerKey::new();
    let search_for_list = search_entry.downgrade();
    let list_weak = list.downgrade();
    list_keys.connect_key_pressed(move |_, key, _, modifiers| {
        let (Some(list), Some(search)) = (list_weak.upgrade(), search_for_list.upgrade()) else {
            return glib::Propagation::Proceed;
        };

        if key == gtk::gdk::Key::Up
            && let Some(selected) = list.selected_row()
        {
            let mut prev = selected.prev_sibling();
            let mut has_prev_selectable = false;
            while let Some(w) = prev {
                if w.is_visible() && w.can_focus() {
                    has_prev_selectable = true;
                    break;
                }
                prev = w.prev_sibling();
            }
            if !has_prev_selectable && search.is_visible() {
                search.grab_focus();
                return glib::Propagation::Stop;
            }
        }

        let has_command_modifiers = modifiers.intersects(
            gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK,
        );
        if !has_command_modifiers && search.is_visible() {
            if key == gtk::gdk::Key::BackSpace {
                search.grab_focus();
                let mut text = search.text().to_string();
                text.pop();
                search.set_text(&text);
                search.set_position(-1);
                return glib::Propagation::Stop;
            }
            if let Some(ch) = key.to_unicode().filter(|c| !c.is_control()) {
                search.grab_focus();
                let mut text = search.text().to_string();
                text.push(ch);
                search.set_text(&text);
                search.set_position(-1);
                return glib::Propagation::Stop;
            }
        }

        glib::Propagation::Proceed
    });
    list.add_controller(list_keys);

    let confirm_for_search = layout.confirm.downgrade();
    search_entry.connect_activate(move |_| {
        if let Some(confirm) = confirm_for_search.upgrade()
            && confirm.is_sensitive()
        {
            confirm.emit_clicked();
        }
    });

    let entries_rc = Rc::new(entries);
    let entries_for_filter = entries_rc.clone();
    let list_for_filter = list.downgrade();
    let list_scroll_for_filter = list_scroll.downgrade();
    let empty_search_for_filter = empty_search.downgrade();
    let confirm_for_filter = layout.confirm.downgrade();
    let rec_heading = recommended_heading_row;
    let oth_heading = other_heading_row;

    search_entry.connect_changed(move |search| {
        let (Some(list), Some(list_scroll), Some(empty_label), Some(confirm)) = (
            list_for_filter.upgrade(),
            list_scroll_for_filter.upgrade(),
            empty_search_for_filter.upgrade(),
            confirm_for_filter.upgrade(),
        ) else {
            return;
        };
        let query = search.text().trim().to_lowercase();
        let mut rec_count = 0;
        let mut oth_count = 0;
        let mut first_visible_row: Option<gtk::ListBoxRow> = None;
        let current_selected = list.selected_row();
        let mut selected_is_visible = false;

        for entry in entries_for_filter.iter() {
            let matches = query.is_empty() || entry.haystack.contains(&query);
            entry.row.set_visible(matches);
            if matches {
                if entry.is_recommended {
                    rec_count += 1;
                } else {
                    oth_count += 1;
                }
                if first_visible_row.is_none() {
                    first_visible_row = Some(entry.row.clone());
                }
                if let Some(selected) = &current_selected
                    && &entry.row == selected
                {
                    selected_is_visible = true;
                }
            }
        }

        if let Some(heading) = &rec_heading {
            heading.set_visible(rec_count > 0);
        }
        if let Some(heading) = &oth_heading {
            heading.set_visible(oth_count > 0);
        }

        let total_matches = rec_count + oth_count;
        if total_matches == 0 {
            empty_label.set_visible(true);
            list_scroll.set_visible(false);
            confirm.set_sensitive(false);
            list.select_row(None::<&gtk::ListBoxRow>);
        } else {
            empty_label.set_visible(false);
            list_scroll.set_visible(true);
            confirm.set_sensitive(true);
            if !selected_is_visible && let Some(row) = &first_visible_row {
                list.select_row(Some(row));
            }
        }
    });

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
    let entries_for_open = entries_rc;
    let open_parent = parent.as_ref().downgrade();
    let selected_list = list.downgrade();
    layout.confirm.connect_clicked(move |_| {
        let Some(list) = selected_list.upgrade() else {
            return;
        };
        let Some(row) = list.selected_row() else {
            return;
        };
        let Some(entry) = entries_for_open.iter().find(|e| e.row == row) else {
            return;
        };
        let context = list.display().app_launch_context();
        if let Err(error) = launch(&entry.app, &open_files, Some(&context)) {
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

    let escape_dismiss = dismiss;
    let escape_search = search_entry.downgrade();
    let escape = gtk::EventControllerKey::new();
    escape.set_propagation_phase(gtk::PropagationPhase::Capture);
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(search) = escape_search.upgrade()
                && !search.text().is_empty()
            {
                search.set_text("");
                search.grab_focus();
                return glib::Propagation::Stop;
            }
            escape_dismiss();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);
    if !has_apps {
        layout.cancel.grab_focus();
    } else {
        search_entry.grab_focus();
    }
}

#[cfg(test)]
mod tests;
