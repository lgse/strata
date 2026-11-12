// SPDX-License-Identifier: MIT

mod sort_menu;
pub(in crate::ui) use sort_menu::{
    column_sort_menu, list_sort_menu, sync_list_sort_menu, sync_open_sort_menus,
};

use crate::app::Browser;
use crate::model::{SortDirection, SortKey};
use crate::ui::browser::ViewState;
use crate::ui::controls::menu_action_with_detail;
use crate::ui::focus_navigation::vim_focus_direction;
use crate::ui::keys::{
    self,
    bindings::{BACKSPACE, CONTEXT_MENU},
};
use gtk::gdk::ModifierType as Modifiers;
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;

pub(in crate::ui) fn pane_new_folder_button(
    state: std::rc::Weak<ViewState>,
    depth: usize,
) -> gtk::Button {
    let button = gtk::Button::new();
    crate::ui::shortcut_reference::follow_tooltip(
        &button,
        "New Folder (%{keys})",
        crate::ui::keys::Keys::One(crate::ui::keys::bindings::NEW_FOLDER.shown()),
    );
    button.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::FOLDER_PLUS,
    )));
    crate::ui::controls::pane_header_action(&button);
    button.add_css_class("chooser-new-folder");
    if let Some(state) = state.upgrade() {
        button.set_sensitive(state.interactive || state.chooser_allows_create.get());
        if state
            .browser
            .location_at(depth)
            .is_some_and(|location| location.is_recent_location() || location.is_computer_root())
        {
            button.set_visible(false);
        }
    }
    crate::ui::accessibility::set_label(&button, &crate::i18n::tr("New Folder"));
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
    let button = gtk::Button::new();
    let keys = crate::ui::keys::Keys::One(crate::ui::keys::bindings::REFRESH.shown());
    crate::ui::shortcut_reference::follow_tooltip(&button, "Refresh (%{keys})", keys);
    crate::ui::shortcut_reference::follow_label(&button, "Refresh (%{keys})", keys);
    button.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::REFRESH,
    )));
    crate::ui::controls::pane_header_action(&button);
    let weak_browser = Rc::downgrade(browser);
    button.connect_clicked(move |_| {
        if crate::ui::tenxer_mode::chrome_suppressed() {
            return;
        }
        if let Some(browser) = weak_browser.upgrade() {
            crate::adapters::trash_support::forget_all();
            browser.retry_column(depth);
        }
    });
    crate::ui::tenxer_mode::hide_while_enabled(&button);
    button
}

/// Backspace closes the menu and vim keys move between its items.
fn install_menu_keys(popover: &gtk::Popover) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys::on_press(&keys, |controller, press| {
        let Some(popover) = controller.widget().and_downcast::<gtk::Popover>() else {
            return glib::Propagation::Proceed;
        };
        if press.is(BACKSPACE.ignoring(Modifiers::SHIFT_MASK | Modifiers::SUPER_MASK)) {
            popover.popdown();
            glib::Propagation::Stop
        } else if let Some(direction) = vim_focus_direction(press) {
            popover.child_focus(direction);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    popover.add_controller(keys);
}

/// Roughly eight two-line entries; longer histories scroll.
const HISTORY_MENU_MAX_HEIGHT: i32 = 400;
/// Entries listed in the history menu; older ones stay reachable with Back.
const HISTORY_MENU_MAX_ENTRIES: usize = 50;

/// Marks the popover [`attach_back_history`] parents to the Back button.
const BACK_HISTORY_POPOVER: &str = "back-history-popover";

/// Whether `focused` is a Back button that lists its history for the
/// context-menu key. Window-level key handlers run first, so they must leave
/// that key to the button rather than open the pane's context menu.
pub(in crate::ui) fn lists_back_history(focused: &gtk::Widget) -> bool {
    let mut child = focused.first_child();
    while let Some(widget) = child {
        if widget.is::<gtk::Popover>() && widget.has_css_class(BACK_HISTORY_POPOVER) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

/// Right-click, long-press, or the context-menu key on Back lists the back
/// history, most recent first, and jumps to a chosen entry in one step.
pub(in crate::ui) fn attach_back_history(browser: &Rc<Browser>, back: &gtk::Button) {
    super::super::shortcut_reference::follow_description(
        back,
        "Right-click, hold, or press %{keys} to list the back history",
        keys::Keys::One(CONTEXT_MENU.shown()),
    );
    if !browser.can_go_back() {
        return;
    }
    let content = crate::ui::accessibility::menu_box();
    content.add_css_class("column-menu");
    content.add_css_class("history-menu");
    let heading = gtk::Label::new(Some(&crate::i18n::tr("BACK TO")));
    heading.set_xalign(0.0);
    heading.add_css_class("menu-heading");
    content.append(&heading);
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .halign(gtk::Align::Start)
        .position(gtk::PositionType::Bottom)
        .build();
    popover.add_css_class("column-popover");
    popover.add_css_class(BACK_HISTORY_POPOVER);
    crate::ui::scrolling::popover::dismiss_on_outside_scroll(&popover);
    let viewport = gtk::Viewport::builder()
        .child(&content)
        .scroll_to_focus(true)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .child(&viewport)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(HISTORY_MENU_MAX_HEIGHT)
        .build();
    scroll.add_css_class("context-menu-scroll");
    popover.set_child(Some(&scroll));
    install_menu_keys(&popover);
    popover.set_parent(back);
    // A plain button does not unparent an attached popover when it is disposed.
    let attached = popover.clone();
    back.connect_destroy(move |_| attached.unparent());

    // The rows are built when the menu opens: the pane is rebuilt on every
    // navigation, and a long history is listed only when asked for.
    let popup = {
        let popover = popover.downgrade();
        let content = content.downgrade();
        let browser = Rc::downgrade(browser);
        move || {
            let (Some(popover), Some(content), Some(browser)) =
                (popover.upgrade(), content.upgrade(), browser.upgrade())
            else {
                return;
            };
            fill_back_history(&content, &browser, &popover);
            popover.popup();
        }
    };
    let secondary = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_SECONDARY)
        .build();
    let popup_on_secondary = popup.clone();
    secondary.connect_pressed(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        popup_on_secondary();
    });
    back.add_controller(secondary);
    // Claiming the sequence denies the button's own click, so releasing does not go back.
    let hold = gtk::GestureLongPress::builder().touch_only(false).build();
    let popup_on_hold = popup.clone();
    hold.connect_pressed(move |gesture, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        popup_on_hold();
    });
    back.add_controller(hold);
    let key = gtk::EventControllerKey::new();
    keys::on_press(&key, move |_, press| {
        if press.is(CONTEXT_MENU) {
            popup();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    back.add_controller(key);
}

fn fill_back_history(content: &gtk::Box, browser: &Rc<Browser>, popover: &gtk::Popover) {
    while let Some(row) = content.last_child().filter(|row| !row.is::<gtk::Label>()) {
        content.remove(&row);
    }
    let home = glib::home_dir();
    for (index, location) in browser
        .back_locations()
        .iter()
        .take(HISTORY_MENU_MAX_ENTRIES)
        .enumerate()
    {
        let path = location
            .native_path()
            .map(crate::ui::settings::abbreviate_home)
            .unwrap_or_else(|| location.display_path());
        let name = if location.native_path() == Some(home.as_path()) {
            "~".to_owned()
        } else {
            location.display_name()
        };
        let option = menu_action_with_detail(&name, &path);
        crate::ui::accessibility::describe_menu_item(&option, &name, &path);
        let weak_browser = Rc::downgrade(browser);
        let popover = popover.downgrade();
        option.connect_clicked(move |_| {
            if let Some(popover) = popover.upgrade() {
                popover.popdown();
            }
            if let Some(browser) = weak_browser.upgrade() {
                browser.back_to(index + 1);
            }
        });
        content.append(&option);
    }
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
    crate::ui::tenxer_mode::hide_sort_direction_while_enabled(&button);

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
        if crate::ui::tenxer_mode::chrome_suppressed() {
            return;
        }
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
    let device_order = matches!(
        preferences.sort_key,
        SortKey::DeviceOrder | SortKey::Unsorted
    );
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
    let hint = crate::i18n::tr(if preferences.sort_key == SortKey::Unsorted {
        "Unsorted"
    } else if device_order {
        "Device order follows discovery; choose a sort field to reverse its direction"
    } else if descending {
        "Descending — click to reverse"
    } else {
        "Ascending — click to reverse"
    });
    button.set_tooltip_text(Some(&hint));
    crate::ui::accessibility::set_own_label(button, &hint);
}

pub(in crate::ui) fn empty_trash_button(browser: &Rc<Browser>) -> gtk::Button {
    let button = gtk::Button::builder()
        .tooltip_text(crate::i18n::tr("Empty Trash"))
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

pub(in crate::ui) fn leftovers_button(browser: &Rc<Browser>, depth: usize) -> gtk::Button {
    let label = crate::i18n::tr("Items left by an interrupted operation");
    let button = gtk::Button::builder()
        .tooltip_text(&label)
        .visible(!browser.column_leftovers(depth).is_empty())
        .build();
    button.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::TRIANGLE_ALERT,
    )));
    crate::ui::controls::pane_header_action(&button);
    button.add_css_class("leftovers-button");
    crate::ui::accessibility::set_label(&button, &label);
    let weak_browser = Rc::downgrade(browser);
    button.connect_clicked(move |_| {
        if let Some(browser) = weak_browser.upgrade() {
            browser.request_leftover_review(depth);
        }
    });
    button
}
