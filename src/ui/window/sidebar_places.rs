// SPDX-License-Identifier: MIT

//! The standard, recent and pinned places listed in the sidebar.

use std::rc::Rc;

use gtk::prelude::*;

use crate::{app::BrowserEvent, model::Location};

use super::{
    SidebarState,
    launch::home_directory,
    network_location, pinned_drop,
    places::{
        is_standard_place_location, pin_keycap, should_show_standard_place,
        sidebar_standard_place_visible, standard_place, standard_place_chord_key,
    },
    sidebar::PlaceNavigation,
    sidebar_context_menu::PlaceMenu,
    sidebar_rows::{set_sidebar_row_active, sidebar_button, sync_keycap},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RecentAvailability {
    pub(super) platform_tracking_enabled: bool,
    pub(super) runtime_backend_supported: bool,
}

impl RecentAvailability {
    pub(super) fn from_runtime() -> Self {
        let platform_tracking_enabled =
            gtk::Settings::default().is_some_and(|settings| settings.is_gtk_recent_files_enabled());
        let runtime_backend_supported = gio::Vfs::default()
            .supported_uri_schemes()
            .iter()
            .any(|scheme| scheme.as_str().eq_ignore_ascii_case("recent"));
        Self {
            platform_tracking_enabled,
            runtime_backend_supported,
        }
    }

    pub(super) fn is_available(self) -> bool {
        self.platform_tracking_enabled && self.runtime_backend_supported
    }
}

pub(super) fn should_show_recent_place(
    show_recent: bool,
    availability: RecentAvailability,
) -> bool {
    show_recent && availability.is_available()
}

impl SidebarState {
    pub(super) fn append_static_places(self: &Rc<Self>) {
        for place in self.place_order.borrow().clone() {
            self.append_ordered_place(place);
        }
        self.append_pinned_places();
    }

    pub(super) fn append_ordered_place(self: &Rc<Self>, place: &'static str) {
        match place {
            "home" => {
                if !self.preference_manager.sidebar_show_home() {
                    return;
                }
                let location = Location::local(home_directory());
                let row = self.append_place(
                    crate::assets::icons::HOME,
                    &crate::i18n::tr("Home"),
                    location.clone(),
                );
                self.add_place_keycap(&row, crate::ui::keys::bindings::GO_HOME);
                self.attach_place_context_menu(&row, location, |state| {
                    state.preference_manager.set_sidebar_show_home(false);
                });
                self.make_place_reorderable(&row, place);
            }
            "trash" => {
                if !self.local_only && self.preference_manager.sidebar_show_trash() {
                    self.append_trash_place();
                }
            }
            "network" => {
                if self.local_only || !self.preference_manager.sidebar_show_network() {
                    return;
                }
                let location = Location::uri("network:///");
                let row = self.append_place(
                    crate::assets::icons::NETWORK,
                    &crate::i18n::tr("Network"),
                    location.clone(),
                );
                self.add_place_keycap(&row, crate::ui::keys::bindings::GO_NETWORK);
                let place_menu = PlaceMenu {
                    location,
                    extra: Some(network_location::menu_extra()),
                };
                self.attach_place_context_menu_with(&row, place_menu, |state| {
                    state.preference_manager.set_sidebar_show_network(false);
                });
                self.make_place_reorderable(&row, place);
            }
            "recent" => {
                if should_show_recent_place(
                    self.preference_manager.sidebar_show_recent(),
                    self.recent_availability.get(),
                ) {
                    self.append_recent_place();
                }
            }
            _ => self.append_standard_place(place),
        }
    }

    pub(super) fn standard_place_visible(&self, id: &str) -> bool {
        sidebar_standard_place_visible(&self.preference_manager, id)
    }

    pub(super) fn append_standard_place(self: &Rc<Self>, place: &'static str) {
        if !self.standard_place_visible(place) {
            return;
        }
        if let Some((icon, name, directory)) = standard_place(place)
            && let Some(path) = glib::user_special_dir(directory)
                .filter(|path| should_show_standard_place(place, path, &home_directory()))
        {
            let row = self.append_reorderable_place(
                place,
                icon,
                &crate::i18n::tr(name),
                Location::local(path),
            );
            if let Some(step) = standard_place_chord_key(place) {
                self.add_place_keycap(&row, step);
            }
        }
    }

    pub(super) fn append_pinned_places(self: &Rc<Self>) {
        let pinned = self.listed_pins();
        let empty = pinned.is_empty();
        let before_section = self.widget.last_child();
        if self.widget.first_child().is_some() {
            self.append_separator();
        }
        let section = self.append_pinned_section(pinned.len());
        let drag_cache = Rc::new(pinned_drop::FolderDragCache::new(self.local_only));
        let mut previous = None;
        for (ordinal, pin) in pinned.into_iter().enumerate() {
            pinned_drop::append_gap(self, previous, Some(pin.1.clone()), &drag_cache);
            previous = Some(pin.1.clone());
            self.append_numbered_pin(ordinal, pin);
        }
        pinned_drop::append_gap(self, previous, None, &drag_cache);
        section.collect_items(&self.widget);
        self.pinned_section.replace(Some(section));
        self.sync_pinned_section_partition();
        let section = if empty {
            self.widgets_after(before_section)
        } else {
            Vec::new()
        };
        self.empty_pinned_section.replace(section);
        self.sync_empty_pinned_section();
    }

    /// Pins the sidebar lists, with their index in the saved pins.
    fn listed_pins(&self) -> Vec<(usize, Location, String)> {
        self.pinned_places
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, (location, _))| !is_standard_place_location(location))
            .filter(|(_, (location, _))| !self.local_only || location.native_path().is_some())
            .map(|(index, (location, name))| (index, location.clone(), name.clone()))
            .collect()
    }

    fn append_numbered_pin(
        self: &Rc<Self>,
        ordinal: usize,
        (index, location, name): (usize, Location, String),
    ) {
        self.visible_pins.borrow_mut().push(location.clone());
        let row = self.append_pinned_place(index, &name, location);
        if let Some(keycap) = pin_keycap(ordinal).and_then(|keycap| self.add_keycap(&row, &keycap))
        {
            sync_keycap(&keycap, self.keycaps_shown.get() && !self.rail.get());
        }
    }

    /// The empty Pinned section's widgets, shown only while files are dragged.
    fn widgets_after(
        &self,
        before_section: Option<gtk::Widget>,
    ) -> Vec<glib::WeakRef<gtk::Widget>> {
        let first = match before_section {
            Some(widget) => widget.next_sibling(),
            None => self.widget.first_child(),
        };
        std::iter::successors(first, gtk::Widget::next_sibling)
            .map(|widget| widget.downgrade())
            .collect()
    }

    /// With nothing pinned, the Pinned section is only a drop target, so it
    /// shows just while files are dragged over the sidebar.
    pub(in crate::ui) fn set_file_drag_over(&self, over: bool) {
        self.file_drag_over.set(over);
        self.sync_empty_pinned_section();
    }

    /// Applies the drag state to the empty section's widgets.
    pub(super) fn sync_empty_pinned_section(&self) {
        let over = self.file_drag_over.get();
        for widget in self.empty_pinned_section.borrow().iter() {
            if let Some(widget) = widget.upgrade() {
                widget.set_visible(over);
            }
        }
    }

    pub(super) fn append_recent_place(self: &Rc<Self>) {
        let location = Location::uri("recent:///");
        let row = sidebar_button(crate::assets::icons::CLOCK, &crate::i18n::tr("Recent"));
        crate::ui::accessibility::set_description(&row, Some("recent:///"));
        self.add_place_keycap(&row, crate::ui::keys::bindings::GO_RECENT);
        self.bind_place_row(&row, location.clone(), PlaceNavigation::Direct);
        self.attach_place_context_menu(&row, location, |state| {
            state.preference_manager.set_sidebar_show_recent(false);
        });
        self.append_sidebar_row(&row);
        self.make_place_reorderable(&row, "recent");
    }

    pub(super) fn append_pinned_place(
        self: &Rc<Self>,
        index: usize,
        name: &str,
        location: Location,
    ) -> gtk::Button {
        let row = self.append_place(crate::assets::icons::FOLDER, name, location.clone());
        row.add_css_class("sidebar-pinned-row");
        self.make_pinned_row_reorderable(&row, index);
        let unpinned_location = location.clone();
        self.attach_place_context_menu(&row, location, move |state| {
            state.unpin_location(&unpinned_location);
        });
        row
    }

    pub(super) fn event_changes_active_place(event: &BrowserEvent) -> bool {
        matches!(
            event,
            BrowserEvent::Reset
                | BrowserEvent::ColumnAdded { .. }
                | BrowserEvent::ColumnsTruncated { .. }
                | BrowserEvent::ColumnsRelocated { .. }
                | BrowserEvent::FocusChanged { .. }
                // A clicked place is highlighted before validation; a rejection
                // leaves the previous location open, including while a mount runs.
                | BrowserEvent::LocationNavigationRejected { .. }
        )
    }

    pub(super) fn sync_active_place(&self) {
        let active = self.browser().active_location();
        let rows = self.place_rows.borrow();
        let selected = rows
            .iter()
            .position(|(location, row)| {
                active.as_ref() == Some(location) && row.has_css_class("active")
            })
            .or_else(|| {
                rows.iter()
                    .position(|(location, _)| active.as_ref() == Some(location))
            });
        for (index, (_, row)) in rows.iter().enumerate() {
            set_sidebar_row_active(row, selected == Some(index));
        }
        drop(rows);
        self.sync_pinned_section_partition();
    }

    #[cfg(test)]
    pub(in crate::ui) fn places_for_test(&self) -> gtk::Box {
        self.widget.clone()
    }

    pub(in crate::ui) fn focus_active_place(&self) -> bool {
        let rows = self.place_rows.borrow();
        rows.iter()
            .find(|(_, row)| row.has_css_class("active"))
            .or_else(|| rows.first())
            .is_some_and(|(_, row)| row.grab_focus())
    }

    /// Pinned places in sidebar display order, excluding rows the sidebar hides.
    pub(in crate::ui) fn visible_pins(&self) -> Vec<Location> {
        self.visible_pins.borrow().clone()
    }
}
