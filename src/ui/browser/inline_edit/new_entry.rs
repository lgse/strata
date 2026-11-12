// SPDX-License-Identifier: MIT

use crate::app::BrowserColumnSnapshot;
use crate::model::{FileEntry, Location};
use crate::ui::browser::ViewState;
use crate::ui::browser::paths::is_trash_location;
use crate::ui::browser_modes::BrowserMode;
use crate::ui::collection_edit::Naming;
use gtk::glib::ControlFlow;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(in crate::ui::browser) struct PendingEntryRename {
    pub(super) depth: usize,
    parent: Location,
    reveal_generation: u64,
    move_sources: RefCell<Vec<Location>>,
    pub(super) created: RefCell<Option<Location>>,
    /// A group folder takes a single name while its selection moves in.
    pub(super) naming: Naming,
}

/// The created item a tick callback waits on until its row can take the editor.
struct CreatedEntryWatch {
    pending: Rc<PendingEntryRename>,
    location: Location,
    deadline: std::time::Instant,
    selected: Cell<bool>,
    /// Opening rename before the source rows disappear lets their removal
    /// rebind the edited row and tear down the field.
    group_sources: RefCell<Vec<Location>>,
}

/// What a new entry is created as and what moves into it.
struct NewEntry {
    location: Location,
    is_directory: bool,
    move_sources: Vec<Location>,
}

impl ViewState {
    pub(in crate::ui::browser) fn rename_created_entry(self: &Rc<Self>, location: &Location) {
        let Some(pending) = self
            .pending_new_entry
            .borrow()
            .clone()
            .filter(|pending| Some(pending.parent.clone()) == location.parent())
        else {
            return;
        };
        pending.created.replace(Some(location.clone()));
        let watch = CreatedEntryWatch {
            pending,
            location: location.clone(),
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(5),
            selected: Cell::new(false),
            group_sources: RefCell::new(Vec::new()),
        };
        let weak = Rc::downgrade(self);
        // Wait for the refreshed listing and the virtualized row to be allocated.
        self.overlay.add_tick_callback(move |_, _| {
            let Some(state) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            state.created_entry_tick(&watch)
        });
    }

    fn created_entry_is_current(&self, pending: &Rc<PendingEntryRename>) -> bool {
        self.pending_new_entry
            .borrow()
            .as_ref()
            .is_some_and(|current| Rc::ptr_eq(current, pending))
    }

    fn created_entry_expired(&self, watch: &CreatedEntryWatch) -> bool {
        let pending = &watch.pending;
        std::time::Instant::now() >= watch.deadline
            || self.rename_reveal_generation.get() != pending.reveal_generation
            || self.browser.location_at(pending.depth).as_ref() != Some(&pending.parent)
    }

    fn created_entry_tick(self: &Rc<Self>, watch: &CreatedEntryWatch) -> ControlFlow {
        let pending = &watch.pending;
        if !self.created_entry_is_current(pending) {
            return ControlFlow::Break;
        }
        if self.created_entry_expired(watch) {
            self.cancel_new_entry();
            return ControlFlow::Break;
        }
        let Some(snapshot) = self
            .browser
            .column_snapshot(pending.depth)
            .filter(|snapshot| !snapshot.loading)
        else {
            return ControlFlow::Continue;
        };
        let position = self
            .browser
            .with_entries(pending.depth, 0..snapshot.count, |entries| {
                entries
                    .iter()
                    .position(|entry| entry.location == watch.location)
            })
            .flatten();
        let Some(position) = position else {
            return ControlFlow::Continue;
        };
        self.move_selection_into_created(watch, position);
        if !watch.selected.replace(true) {
            self.select_created_entry(pending, position);
        } else if self.begin_created_rename(watch, &snapshot, position) {
            self.pending_new_entry.take();
            return ControlFlow::Break;
        }
        self.scroll_created_into_view(pending.depth, position);
        ControlFlow::Continue
    }

    /// A folder made from a selection takes that selection in once it is listed.
    fn move_selection_into_created(self: &Rc<Self>, watch: &CreatedEntryWatch, position: usize) {
        let pending = &watch.pending;
        let move_sources = if self
            .browser
            .entry_at(pending.depth, position)
            .is_some_and(|entry| entry.is_directory())
        {
            pending.move_sources.take()
        } else {
            Vec::new()
        };
        if move_sources.is_empty() {
            return;
        }
        watch.group_sources.replace(move_sources.clone());
        self.browser.expect_group_folder(watch.location.clone());
        let weak = Rc::downgrade(self);
        let destination = watch.location.clone();
        gtk::glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.start_transfer_with_reveal(crate::ui::browser::transfer::TransferRequest {
                    destination,
                    sources: move_sources,
                    move_sources: true,
                    reveal: false,
                    send_to: None,
                    focus: crate::ui::browser::transfer::ConflictFocus::Replace,
                });
            }
        });
    }

    fn select_created_entry(&self, pending: &Rc<PendingEntryRename>, position: usize) {
        if self.mode_views.borrow().mode() == BrowserMode::Columns {
            self.pending_new_entry.take();
            self.browser.reveal_created_entry(pending.depth, position);
            // Revealing the child synchronously truncates columns and cancels
            // pending editors. Retain this creation's authority for the next frame.
            self.pending_new_entry.replace(Some(pending.clone()));
        } else {
            self.browser.select(pending.depth, position);
        }
    }

    fn group_sources_left(
        &self,
        watch: &CreatedEntryWatch,
        snapshot: &BrowserColumnSnapshot,
    ) -> bool {
        watch.group_sources.borrow().iter().all(|source| {
            self.browser
                .with_entries(watch.pending.depth, 0..snapshot.count, |entries| {
                    !entries.iter().any(|entry| entry.location == *source)
                })
                .unwrap_or(true)
        })
    }

    fn begin_created_rename(
        self: &Rc<Self>,
        watch: &CreatedEntryWatch,
        snapshot: &BrowserColumnSnapshot,
        position: usize,
    ) -> bool {
        let pending = &watch.pending;
        if !self.group_sources_left(watch, snapshot) {
            return false;
        }
        let Some(entry) = self.browser.entry_at(pending.depth, position) else {
            return false;
        };
        self.begin_rename_item((pending.depth, position), entry, pending.naming)
    }

    fn scroll_created_into_view(&self, depth: usize, position: usize) {
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            return;
        }
        if let Some(column) = self.columns.borrow().get(depth)
            && let Some(position) = column.map.view_position(position)
        {
            column
                .list
                .scroll_to(position, gtk::ListScrollFlags::NONE, None);
        }
    }

    pub(in crate::ui::browser) fn new_folder_with_selection(
        self: &Rc<Self>,
        depth: usize,
        entries: &[FileEntry],
    ) -> bool {
        let Some(location) = self
            .browser
            .location_at(depth)
            .filter(|location| !is_trash_location(location) && !location.is_recent_location())
        else {
            return false;
        };
        // A recursive search shows its own result model; the created row never
        // materializes there, so the move and rename would never dispatch.
        if crate::ui::browser::context_menu::context_search_active(self, depth)
            || entries.is_empty()
        {
            return false;
        }
        let move_sources = entries.iter().map(|entry| entry.location.clone()).collect();
        self.begin_new_entry_inner(
            depth,
            NewEntry {
                location,
                is_directory: true,
                move_sources,
            },
        );
        true
    }

    pub(in crate::ui::browser) fn begin_new_entry(
        self: &Rc<Self>,
        depth: usize,
        location: Location,
        is_directory: bool,
    ) {
        self.begin_new_entry_inner(
            depth,
            NewEntry {
                location,
                is_directory,
                move_sources: Vec::new(),
            },
        );
    }

    fn may_create_in(&self, location: &Location) -> bool {
        let interactive = self.interactive || self.chooser_allows_create.get();
        interactive
            && !is_trash_location(location)
            && !location.is_recent_location()
            && !location.is_computer_root()
    }

    fn begin_new_entry_inner(self: &Rc<Self>, depth: usize, entry: NewEntry) {
        let NewEntry {
            location,
            is_directory,
            move_sources,
        } = entry;
        if !self.may_create_in(&location) {
            return;
        }
        self.cancel_new_entry();
        self.cancel_rename();
        if let Some(column) = self.columns.borrow().get(depth) {
            column.filter_entry.set_text("");
        }
        self.mode_views.borrow().clear_filter(depth);
        self.pending_new_entry
            .replace(Some(Rc::new(PendingEntryRename {
                depth,
                parent: location.clone(),
                reveal_generation: self.rename_reveal_generation.get(),
                naming: if move_sources.is_empty() {
                    Naming::NewItem
                } else {
                    Naming::Single
                },
                move_sources: RefCell::new(move_sources),
                created: RefCell::new(None),
            })));
        if is_directory {
            self.browser.create_new_folder(location);
        } else {
            self.browser.create_new_file(location);
        }
    }

    pub(in crate::ui::browser) fn expect_new_entry(&self, depth: usize, parent: Location) {
        self.cancel_new_entry();
        self.cancel_rename();
        self.pending_new_entry
            .replace(Some(Rc::new(PendingEntryRename {
                depth,
                parent,
                reveal_generation: self.rename_reveal_generation.get(),
                move_sources: RefCell::new(Vec::new()),
                created: RefCell::new(None),
                // A recovered leftover may hold data, so it is not new.
                naming: Naming::Path,
            })));
    }

    pub(in crate::ui::browser) fn cancel_new_entry(&self) -> bool {
        let pending = self.pending_new_entry.take();
        if let Some(pending) = &pending
            && let Some(created) = pending.created.borrow().as_ref()
        {
            self.browser.clear_group_folder_for(created);
        }
        pending.is_some()
    }
}
