// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use crate::app::Browser;
use crate::model::{SortDirection, SortKey};
use crate::ui::browser::ViewState;
use crate::ui::controls::menu_option;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(in crate::ui) fn pane_new_folder_button(
    state: std::rc::Weak<ViewState>,
    depth: usize,
) -> gtk::Button {
    let button = gtk::Button::builder()
        .tooltip_text("New Folder (Ctrl+Shift+N)")
        .build();
    button.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::FOLDER_PLUS,
    )));
    crate::ui::controls::pane_header_action(&button);
    button.add_css_class("chooser-new-folder");
    if state
        .upgrade()
        .and_then(|state| state.browser.location_at(depth))
        .is_some_and(|location| location.is_recent_location())
    {
        button.set_visible(false);
    }
    button.update_property(&[gtk::accessible::Property::Label("New Folder")]);
    button.connect_clicked(move |_| {
        if let Some(state) = state.upgrade()
            && let Some(location) = state.browser.location_at(depth)
        {
            state.begin_new_entry(depth, location, true);
        }
    });
    button
}

pub(in crate::ui) fn pane_refresh_button(browser: &Rc<Browser>, depth: usize) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text("Refresh (F5)").build();
    button.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::REFRESH,
    )));
    crate::ui::controls::pane_header_action(&button);
    let weak_browser = Rc::downgrade(browser);
    button.connect_clicked(move |_| {
        if let Some(browser) = weak_browser.upgrade() {
            browser.retry_column(depth);
        }
    });
    button
}

pub(in crate::ui) fn column_sort_menu(browser: &Rc<Browser>, depth: usize) -> gtk::MenuButton {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.add_css_class("column-menu");
    let heading = gtk::Label::new(Some("SORT BY"));
    heading.set_xalign(0.0);
    heading.add_css_class("menu-heading");
    content.append(&heading);

    let preferences = browser.column_preferences(depth).unwrap_or_default();
    let selected_checks: Rc<RefCell<Vec<(SortKey, gtk::Image)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .halign(gtk::Align::End)
        .position(gtk::PositionType::Bottom)
        .build();
    popover.add_css_class("column-popover");
    crate::ui::scrolling::popover::dismiss_on_outside_scroll(&popover);
    let popover_weak = popover.downgrade();
    let location = browser.location_at(depth);
    let camera_photos = location
        .as_ref()
        .is_some_and(|location| location.is_camera_photo_root());
    let recent = location.is_some_and(|location| location.is_recent_root());
    for (label, key) in [
        ("Device order", SortKey::DeviceOrder),
        ("Recency", SortKey::Recency),
        ("Name", SortKey::Name),
        ("Size", SortKey::Size),
        ("Modified", SortKey::Modified),
        ("Type", SortKey::Type),
    ] {
        if (key == SortKey::DeviceOrder && !camera_photos) || (key == SortKey::Recency && !recent) {
            continue;
        }
        let (option, check) = menu_option(label, preferences.sort_key == key);
        if key == SortKey::DeviceOrder {
            option.set_tooltip_text(Some("Append photos as the device lists them; not necessarily chronological. Selecting this reloads the library."));
        }
        selected_checks.borrow_mut().push((key, check));
        let checks = selected_checks.clone();
        let weak_browser = Rc::downgrade(browser);
        let popover_weak = popover_weak.clone();
        option.connect_clicked(move |_| {
            for (check_key, check) in checks.borrow().iter() {
                check.set_visible(*check_key == key);
            }
            if let Some(browser) = weak_browser.upgrade() {
                browser.set_sort_key(depth, key);
            }
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });
        content.append(&option);
    }

    let folders_state = if !recent {
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let (folders_first, folders_check) =
            menu_option("Folders first", preferences.folders_first);
        let folders_enabled = Rc::new(Cell::new(preferences.folders_first));
        let weak_browser = Rc::downgrade(browser);
        let folders_enabled_for_click = folders_enabled.clone();
        let folders_check_for_click = folders_check.clone();
        let popover_weak = popover_weak.clone();
        folders_first.connect_clicked(move |_| {
            let enabled = !folders_enabled_for_click.get();
            folders_enabled_for_click.set(enabled);
            folders_check_for_click.set_visible(enabled);
            if let Some(browser) = weak_browser.upgrade() {
                browser.set_folders_first(depth, enabled);
            }
            if let Some(popover) = popover_weak.upgrade() {
                popover.popdown();
            }
        });
        content.append(&folders_first);
        Some((folders_enabled, folders_check))
    } else {
        None
    };

    popover.set_child(Some(&content));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(|controller, key, _, modifiers| {
        if modifiers
            .intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK)
        {
            return glib::Propagation::Proceed;
        }
        let Some(popover) = controller.widget().and_downcast::<gtk::Popover>() else {
            return glib::Propagation::Proceed;
        };
        if key == gtk::gdk::Key::BackSpace {
            popover.popdown();
            glib::Propagation::Stop
        } else if let Some(direction) = match key {
            gtk::gdk::Key::h => Some(gtk::DirectionType::Left),
            gtk::gdk::Key::j => Some(gtk::DirectionType::Down),
            gtk::gdk::Key::k => Some(gtk::DirectionType::Up),
            gtk::gdk::Key::l => Some(gtk::DirectionType::Right),
            _ => None,
        } {
            popover.child_focus(direction);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    popover.add_controller(keys);
    let weak_browser = Rc::downgrade(browser);
    let checks = selected_checks.clone();
    popover.connect_map(move |_| {
        let Some(preferences) = weak_browser
            .upgrade()
            .and_then(|browser| browser.column_preferences(depth))
        else {
            return;
        };
        for (key, check) in checks.borrow().iter() {
            check.set_visible(*key == preferences.sort_key);
        }
        if let Some((folders_enabled, folders_check)) = folders_state.as_ref() {
            folders_enabled.set(preferences.folders_first);
            folders_check.set_visible(preferences.folders_first);
        }
    });
    let button = gtk::MenuButton::builder()
        .tooltip_text("Choose sort field")
        .popover(&popover)
        .build();
    button.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::SETTINGS_2,
    )));
    crate::ui::controls::pane_header_action(&button);
    button
}

pub(in crate::ui) fn column_sort_direction_toggle(
    browser: &Rc<Browser>,
    depth: usize,
) -> gtk::Button {
    let preferences = browser.column_preferences(depth).unwrap_or_default();
    let button = gtk::Button::new();
    let icon = crate::assets::chrome_icon(crate::assets::icons::ARROW_UP_NARROW_WIDE);
    button.set_child(Some(&icon));
    crate::ui::controls::pane_header_action(&button);
    sync_sort_direction_toggle(&button, &icon, preferences);

    let weak_browser = Rc::downgrade(browser);
    let icon_for_map = icon.clone();
    button.connect_map(move |button| {
        if let Some(preferences) = weak_browser
            .upgrade()
            .and_then(|browser| browser.column_preferences(depth))
        {
            sync_sort_direction_toggle(button, &icon_for_map, preferences);
        }
    });
    let weak_browser = Rc::downgrade(browser);
    button.connect_clicked(move |button| {
        let Some(browser) = weak_browser.upgrade() else {
            return;
        };
        let mut preferences = browser.column_preferences(depth).unwrap_or_default();
        if preferences.sort_key == SortKey::DeviceOrder {
            return;
        }
        let direction = match preferences.sort_direction {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        };
        preferences.sort_direction = direction;
        sync_sort_direction_toggle(button, &icon, preferences);
        browser.set_sort_direction(depth, direction);
    });
    button
}

pub(in crate::ui) fn sync_column_sort_direction(
    browser: &Browser,
    depth: usize,
    button: &gtk::Button,
) {
    if let Some(preferences) = browser.column_preferences(depth)
        && let Some(icon) = button.child().and_downcast::<gtk::Image>()
    {
        sync_sort_direction_toggle(button, &icon, preferences);
    }
}

fn sync_sort_direction_toggle(
    button: &gtk::Button,
    icon: &gtk::Image,
    preferences: crate::model::ViewPreferences,
) {
    let device_order = preferences.sort_key == SortKey::DeviceOrder;
    button.set_sensitive(!device_order);
    let descending = preferences.sort_direction == SortDirection::Descending;
    crate::assets::set_primary_icon(
        icon,
        if descending {
            crate::assets::icons::ARROW_DOWN_WIDE_NARROW
        } else {
            crate::assets::icons::ARROW_UP_NARROW_WIDE
        },
    );
    button.set_tooltip_text(Some(if device_order {
        "Device order follows discovery; choose a sort field to reverse its direction"
    } else if descending {
        "Descending — click to reverse"
    } else {
        "Ascending — click to reverse"
    }));
}

pub(in crate::ui) fn empty_trash_button(browser: &Rc<Browser>) -> gtk::Button {
    let button = gtk::Button::builder()
        .tooltip_text("Empty Trash")
        .visible(false)
        .build();
    button.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::TRASH,
    )));
    crate::ui::controls::pane_header_action(&button);
    let weak_browser = Rc::downgrade(browser);
    button.connect_clicked(move |_| {
        if let Some(browser) = weak_browser.upgrade() {
            browser.request_empty_trash();
        }
    });
    button
}
