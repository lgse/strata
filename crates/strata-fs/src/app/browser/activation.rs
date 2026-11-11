// SPDX-License-Identifier: MIT

use std::rc::Rc;

use crate::{
    model::{FileEntry, Location},
    services::{ArchiveFormat, DirectoryChange},
};

use super::columns::Descent;
use super::{Browser, BrowserEvent, LoadSelection, load_selection};

impl Browser {
    pub(super) fn should_extract_on_activate(&self, entry: &FileEntry) -> bool {
        !self.is_chooser_mode() && ArchiveFormat::for_entry(entry).is_some()
    }

    pub fn preview(self: &Rc<Self>, depth: usize, position: usize) {
        let Some(entry) = self.entry_at(depth, position) else {
            return;
        };
        if entry.is_directory() && self.is_open_child(depth, &entry.location) {
            self.close_column(depth + 1);
            return;
        }
        self.select(depth, position);
        if entry.is_directory() {
            self.descend(depth, entry.location);
        } else {
            self.close_column(depth + 1);
            self.emit(BrowserEvent::PreviewRequested {
                entry,
                automatic: false,
            });
        }
    }

    pub fn request_preview(&self, entry: FileEntry) {
        self.emit(BrowserEvent::PreviewRequested {
            entry,
            automatic: false,
        });
    }

    pub fn request_automatic_preview(&self, entry: FileEntry) {
        self.emit(BrowserEvent::PreviewRequested {
            entry,
            automatic: true,
        });
    }

    pub fn open_location(&self, location: Location) {
        self.emit(BrowserEvent::OpenRequested { location });
    }

    pub fn request_empty_trash(&self) {
        self.emit(BrowserEvent::EmptyTrashRequested);
    }

    pub fn activate(self: &Rc<Self>, depth: usize, position: usize) {
        let open_child = self.entry_at(depth, position).is_some_and(|entry| {
            entry.is_directory()
                && !entry.is_locked_folder()
                && self.is_open_child(depth, &entry.location)
        });
        self.select(depth, position);
        if open_child {
            // Reopening lands where opening did, without picking a child entry.
            self.set_active_column(depth + 1);
            self.focus_active();
            return;
        }
        self.activate_focused_with_selection(false);
    }

    /// The successful creation has already established the entry's type and location.
    pub fn reveal_created_entry(self: &Rc<Self>, depth: usize, position: usize) {
        let _update = self.navigation_update();
        let Some(entry) = self.entry_at(depth, position) else {
            return;
        };
        self.select(depth, position);
        if entry.is_directory() {
            // Do not start another asynchronous validation that can outlive inline rename.
            self.bump_navigation_generation();
            self.close_peek();
            self.descend_validated(Descent::new(depth, entry.location, LoadSelection::Nothing));
            self.select(depth, position);
        } else {
            self.close_column(depth + 1);
        }
    }

    pub fn is_open_child(&self, parent_depth: usize, location: &Location) -> bool {
        parent_depth
            .checked_add(1)
            .and_then(|depth| self.location_at(depth))
            .as_ref()
            == Some(location)
    }

    /// Activates an item using conventional single-pane list navigation.
    pub fn activate_in_place(self: &Rc<Self>, depth: usize, position: usize) {
        self.activate_in_place_with_selection(depth, position, false);
    }

    pub(super) fn activate_in_place_with_selection(
        self: &Rc<Self>,
        depth: usize,
        position: usize,
        select_first: bool,
    ) {
        self.select(depth, position);
        let Some(entry) = self.entry_at(depth, position) else {
            return;
        };
        if self.open_locked_folder(&entry, select_first) {
            return;
        }
        if entry.is_directory() {
            self.navigate_location(entry.location, select_first);
        } else if self.should_extract_on_activate(&entry) {
            self.emit(BrowserEvent::ExtractRequested { entry });
        } else {
            self.emit(BrowserEvent::OpenRequested {
                location: entry.location,
            });
        }
    }

    pub fn activate_focused_in_place(self: &Rc<Self>) {
        let Some((depth, position, _)) = self.focused_item() else {
            self.move_selection(1);
            return;
        };
        self.activate_in_place_with_selection(depth, position, true);
    }

    pub fn enter_focused_directory(self: &Rc<Self>) {
        match self.focused_entry() {
            Some(entry) if !entry.is_directory() => self.focus_child(),
            _ => self.activate_focused(),
        }
    }

    pub fn activate_focused(self: &Rc<Self>) {
        self.activate_focused_with_selection(true);
    }

    /// `admin://` for a native folder the user can't read, when the source offers it.
    pub fn administrator_alternative(&self, location: &Location) -> Option<Location> {
        if !self.source.offers_administrator_access() {
            return None;
        }
        location.as_administrator()
    }

    /// Opening a folder the user can't read goes through `admin://`, as in GNOME Files.
    pub(super) fn open_locked_folder(
        self: &Rc<Self>,
        entry: &FileEntry,
        select_first: bool,
    ) -> bool {
        let Some(administrator) = entry
            .is_locked_folder()
            .then(|| self.administrator_alternative(&entry.location))
            .flatten()
        else {
            return false;
        };
        self.navigate_location(administrator, select_first);
        true
    }

    pub(super) fn activate_focused_with_selection(self: &Rc<Self>, select_first: bool) {
        let focused = self.state.borrow().focused_entry();
        let Some((depth, _, entry)) = focused else {
            self.move_selection(1);
            return;
        };

        if self.open_locked_folder(&entry, select_first) {
            return;
        }
        if entry.is_directory() {
            if self.is_open_child(depth, &entry.location) {
                self.focus_child();
            } else {
                let selection = load_selection(select_first);
                self.descend_with_selection(Descent::new(depth, entry.location, selection));
            }
        } else if self.should_extract_on_activate(&entry) {
            self.emit(BrowserEvent::ExtractRequested { entry });
        } else {
            self.emit(BrowserEvent::OpenRequested {
                location: entry.location,
            });
        }
    }

    pub(super) fn publish_relocation(&self, from: &Location, to: &Location) {
        self.emit(BrowserEvent::ItemRelocated {
            from: from.clone(),
            to: to.clone(),
            merged: false,
        });
    }

    pub(super) fn publish_rename(self: &Rc<Self>, old: &Location, entry: FileEntry) {
        if let (Some(from), Some(to)) = (old.native_path(), entry.location.native_path()) {
            crate::services::refresh_search_indexes_for_rename(from, to);
        }
        self.retire_recent_target(old);
        if !(0..)
            .map_while(|depth| self.location_at(depth))
            .any(|location| location.is_within(old))
        {
            return;
        }
        if let Some(parent) = old.parent() {
            let depths = (0..)
                .map_while(|depth| self.location_at(depth).map(|location| (depth, location)))
                .filter_map(|(depth, location)| (location == parent).then_some(depth))
                .collect::<Vec<_>>();
            for depth in depths {
                self.handle_directory_change(
                    depth,
                    &parent,
                    DirectoryChange::Move {
                        from: old.clone(),
                        entry: entry.clone(),
                    },
                );
            }
        }
        // The source parent need not still be open when the operation completes.
        self.relocate_open_columns(old, &entry.location);
    }

    pub(super) fn follow_renamed_location(self: &Rc<Self>, from: &Location, to: &Location) {
        if let (Some(old), Some(new)) = (from.native_path(), to.native_path()) {
            crate::services::refresh_search_indexes_for_rename(old, new);
        }
        self.relocate_open_columns(from, to);
    }

    pub(super) fn relocate_open_columns(self: &Rc<Self>, from: &Location, to: &Location) {
        let locations = (0..)
            .map_while(|depth| self.location_at(depth))
            .collect::<Vec<_>>();
        let Some(from_depth) = locations.iter().position(|location| {
            location
                .rebase(from, to)
                .is_some_and(|relocated| relocated != *location)
        }) else {
            return;
        };
        let loads = locations
            .into_iter()
            .enumerate()
            .skip(from_depth)
            .map(|(depth, location)| {
                let relocated = location.rebase(from, to).unwrap_or(location);
                (depth, relocated, self.new_request_id())
            })
            .collect::<Vec<_>>();
        self.close_peek();
        self.loads.borrow_mut().truncate(from_depth);
        self.monitors.borrow_mut().truncate(from_depth);
        self.truncate_deferred_from(from_depth);
        for (depth, location, request_id) in &loads {
            self.state
                .borrow_mut()
                .relocate_column(*depth, location.clone(), *request_id);
        }
        self.emit(BrowserEvent::ColumnsRelocated { from_depth });
        for (depth, location, request_id) in loads {
            self.start_load(depth, location, request_id);
        }
    }
}
