// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    model::Location,
    ui::{browser::BrowserView, preferences::PreferenceManager},
};

use super::{
    RecentAvailability, SIDEBAR_WIDTH, SidebarState, SidebarView, TrashContents,
    event_changes_trash_contents, install_sidebar_file_drop, load_pinned_places,
    resolve_place_order, select_sidebar_row,
};

pub(in crate::ui) fn build_sidebar(
    view: BrowserView,
    preferences: Rc<PreferenceManager>,
    local_only: bool,
) -> SidebarView {
    let shell = SidebarShell::new();
    let state = SidebarState::new(
        shell.places,
        view,
        preferences,
        local_only,
        shell.update_label.clone(),
    );
    state.bind_order();
    state.observe_navigation_and_trash();
    let (handlers, mount_handler) = connect_device_changes(&state);
    let recent_setting_handler = connect_recent_setting_changes(&state);
    // Device discovery remains deferred to the window's first-paint callback.
    state.append_static_places();
    state.sync_active_place();
    SidebarView {
        widget: shell.widget.upcast(),
        state,
        update_notice: shell.update_notice,
        update_area: shell.update_area,
        update_label: shell.update_label,
        handlers: RefCell::new(handlers),
        mount_handler: RefCell::new(Some(mount_handler)),
        recent_setting_handler: RefCell::new(recent_setting_handler),
    }
}

fn connect_recent_setting_changes(
    state: &Rc<SidebarState>,
) -> Option<(gtk::Settings, glib::SignalHandlerId)> {
    let settings = gtk::Settings::default()?;
    let weak = Rc::downgrade(state);
    let handler = settings.connect_gtk_recent_files_enabled_notify(move |_| {
        if let Some(state) = weak.upgrade() {
            state
                .recent_availability
                .set(RecentAvailability::from_runtime());
            state.queue_rebuild();
        }
    });
    Some((settings, handler))
}

struct SidebarShell {
    widget: gtk::Box,
    places: gtk::Box,
    update_notice: gtk::Button,
    update_area: gtk::Box,
    update_label: gtk::Label,
}

impl SidebarShell {
    fn new() -> Self {
        let places = gtk::Box::new(gtk::Orientation::Vertical, 2);
        places.add_css_class("sidebar");
        let scroller = gtk::ScrolledWindow::builder()
            .child(&places)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .overlay_scrolling(false)
            .width_request(SIDEBAR_WIDTH)
            .vexpand(true)
            .build();
        scroller.add_css_class("sidebar-scroll");
        scroller.add_css_class("fixed-scrollbar");
        let (update_area, update_notice, update_label) = update_notice();
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("sidebar-shell");
        widget.append(&scroller);
        widget.append(&update_area);
        Self {
            widget,
            places,
            update_notice,
            update_area,
            update_label,
        }
    }
}

fn update_notice() -> (gtk::Box, gtk::Button, gtk::Label) {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let dot = gtk::Label::new(Some("●"));
    dot.add_css_class("sidebar-update-dot");
    let label = gtk::Label::new(None);
    label.add_css_class("sidebar-update-label");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&dot);
    content.append(&label);
    content.append(&crate::assets::primary_icon(
        crate::assets::icons::DOWNLOADS,
        17,
    ));
    let notice = gtk::Button::builder().child(&content).build();
    notice.add_css_class("sidebar-update");
    notice.set_cursor_from_name(Some("pointer"));
    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    separator.add_css_class("sidebar-separator");
    separator.add_css_class("sidebar-update-separator");
    let area = gtk::Box::new(gtk::Orientation::Vertical, 0);
    area.set_visible(false);
    area.append(&separator);
    area.append(&notice);
    (area, notice, label)
}

impl SidebarState {
    fn new(
        widget: gtk::Box,
        view: BrowserView,
        preference_manager: Rc<PreferenceManager>,
        local_only: bool,
        update_label: gtk::Label,
    ) -> Rc<Self> {
        let volume_monitor = gio::VolumeMonitor::get();
        let place_order = resolve_place_order(&preference_manager.sidebar_order());
        let places_visibility = preference_manager.sidebar_places_visibility();
        Rc::new(Self {
            widget,
            browser: view.browser(),
            view,
            volume_monitor,
            mount_monitor: gio_unix::MountMonitor::get(),
            preference_manager,
            place_order: RefCell::new(place_order),
            places_visibility: RefCell::new(places_visibility),
            pinned_places: Rc::new(RefCell::new(load_pinned_places().unwrap_or_default())),
            place_rows: RefCell::new(Vec::new()),
            trash_contents: Cell::new(TrashContents::Unknown),
            trash_menu_rows: RefCell::new(None),
            trash_monitor: RefCell::new(None),
            trash_probe_running: Cell::new(false),
            trash_probe_pending: Cell::new(false),
            local_only,
            recent_availability: Cell::new(RecentAvailability::from_runtime()),
            pending_scroll: Cell::new(None),
            rebuild_queued: Cell::new(false),
            scroll_restore_queued: Cell::new(false),
            rail: Cell::new(false),
            saved_width: Cell::new(None),
            update_label,
        })
    }

    fn bind_order(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.preference_manager.bind_preference(
            &self.widget,
            PreferenceManager::sidebar_order,
            move |_, order| {
                if let Some(state) = weak.upgrade() {
                    let order = resolve_place_order(&order);
                    if *state.place_order.borrow() != order {
                        state.place_order.replace(order);
                        state.rebuild();
                    }
                }
            },
        );
        let weak = Rc::downgrade(self);
        self.preference_manager.bind_preference(
            &self.widget,
            PreferenceManager::sidebar_places_visibility,
            move |_, visibility| {
                if let Some(state) = weak.upgrade()
                    && *state.places_visibility.borrow() != visibility
                {
                    state.places_visibility.replace(visibility);
                    state.rebuild();
                }
            },
        );
    }

    fn observe_navigation_and_trash(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.browser.observe(move |event| {
            let changes_active_place = Self::event_changes_active_place(event);
            let changes_trash = event_changes_trash_contents(event);
            if !changes_active_place && !changes_trash {
                return;
            }
            let Some(state) = weak.upgrade() else {
                return;
            };
            if changes_active_place {
                state.sync_active_place();
            }
            if changes_trash {
                state.refresh_trash_contents();
            }
        });
    }

    pub(super) fn bind_place_row(
        &self,
        row: &gtk::Button,
        location: Location,
        navigation: PlaceNavigation,
    ) {
        self.place_rows
            .borrow_mut()
            .push((location.clone(), row.clone()));
        install_sidebar_file_drop(&self.view, row, location.clone());
        let browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        let keyboard_activation = Rc::new(Cell::new(false));
        let activating = keyboard_activation.clone();
        row.connect_activate(move |_| activating.set(true));
        row.connect_clicked(move |_| {
            let select_first = keyboard_activation.replace(false);
            select_sidebar_row(&sidebar, &selected_row);
            if let Some(browser) = browser.upgrade() {
                match navigation {
                    PlaceNavigation::Direct => {
                        browser.navigate_with_selection(location.clone(), select_first);
                    }
                    PlaceNavigation::Validate => {
                        browser.navigate_location(location.clone(), select_first);
                    }
                }
            }
        });
    }
}

// Trash and reorderable standard places already have a known destination;
// ordinary place rows retain Browser::navigate_location validation.
#[derive(Clone, Copy)]
pub(super) enum PlaceNavigation {
    Direct,
    Validate,
}

fn connect_device_changes(
    state: &Rc<SidebarState>,
) -> (Vec<glib::SignalHandlerId>, glib::SignalHandlerId) {
    let monitor = &state.volume_monitor;
    let handlers = vec![
        monitor.connect_mount_added(rebuild_on_change(state)),
        monitor.connect_mount_removed(rebuild_on_change(state)),
        monitor.connect_mount_changed(rebuild_on_change(state)),
        monitor.connect_volume_added(rebuild_on_change(state)),
        monitor.connect_volume_removed(rebuild_on_change(state)),
        monitor.connect_volume_changed(rebuild_on_change(state)),
        monitor.connect_drive_connected(rebuild_on_change(state)),
        monitor.connect_drive_disconnected(rebuild_on_change(state)),
        monitor.connect_drive_changed(rebuild_on_change(state)),
    ];
    let weak = Rc::downgrade(state);
    let mount_handler = state.mount_monitor.connect_mounts_changed(move |_| {
        if let Some(state) = weak.upgrade() {
            state.queue_rebuild();
        }
    });
    (handlers, mount_handler)
}

fn rebuild_on_change<T: 'static>(
    state: &Rc<SidebarState>,
) -> impl Fn(&gio::VolumeMonitor, &T) + 'static {
    let weak = Rc::downgrade(state);
    move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.queue_rebuild();
        }
    }
}

#[cfg(test)]
mod tests;
