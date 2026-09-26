// SPDX-License-Identifier: MIT

use crate::model::{FileEntry, Location};
use crate::services::{OperationRequestId, RequestId, validate_basename};
use crate::ui::browser::ViewState;
use crate::ui::browser::paths::is_trash_location;
use crate::ui::browser_modes::BrowserMode;
use gtk::prelude::*;
use std::rc::Rc;

struct ColumnsRenameTarget {
    row: gtk::Box,
    edit: crate::ui::collection_edit::EditWidgets,
    spacer: gtk::Box,
    size: gtk::Label,
    scroll: gtk::ScrolledWindow,
}

enum PendingRenameState {
    Queued,
    Dispatching,
    Running(OperationRequestId),
    AwaitingRefresh { requests: Vec<(usize, RequestId)> },
}

pub(super) struct PendingRename {
    old_location: Location,
    new_location: Option<Location>,
    old_name: String,
    new_name: String,
    generation: u64,
    monitor_has_new_location: bool,
    reveal_generation: u64,
    scroll_value: Option<f64>,
    source_position: Option<usize>,
    state: PendingRenameState,
}

impl PendingRename {
    fn begin_dispatch(&mut self) -> bool {
        if !matches!(self.state, PendingRenameState::Queued) {
            return false;
        }
        self.state = PendingRenameState::Dispatching;
        true
    }

    fn finish_dispatch(&mut self, operation_id: OperationRequestId) -> bool {
        if !matches!(self.state, PendingRenameState::Dispatching) {
            return false;
        }
        self.state = PendingRenameState::Running(operation_id);
        true
    }

    fn owns_operation(
        &self,
        operation_id: OperationRequestId,
        last_started: Option<OperationRequestId>,
    ) -> bool {
        matches!(&self.state, PendingRenameState::Running(id) if *id == operation_id)
            || (matches!(&self.state, PendingRenameState::Dispatching)
                && last_started == Some(operation_id))
    }

    fn complete(
        &mut self,
        operation_id: OperationRequestId,
        last_started: Option<OperationRequestId>,
    ) -> bool {
        if !self.owns_operation(operation_id, last_started) {
            return false;
        }
        self.state = PendingRenameState::AwaitingRefresh {
            requests: Vec::new(),
        };
        true
    }
}

fn constrain_rename_to_viewport(field: &gtk::Entry, viewport: &gtk::ScrolledWindow) {
    let Some(editor) = field.parent() else { return };
    let Some(bounds) = editor.compute_bounds(viewport) else {
        return;
    };
    if bounds.width() <= 0.0 || viewport.width() <= 0 {
        return;
    }
    // Columns can be wider than the viewport. GtkText must scroll within the
    // visible slice, not an allocation clipped by the outer horizontal scroller.
    let start = (-bounds.x()).ceil().max(0.0) as i32;
    let end = (bounds.x() + bounds.width() - viewport.width() as f32)
        .ceil()
        .max(0.0) as i32;
    field.set_margin_start(start.min(editor.width().saturating_sub(1)));
    field.set_margin_end(end.min(editor.width().saturating_sub(start + 1).max(0)));
}

pub(in crate::ui) fn reveal_rename_row(
    row: &impl IsA<gtk::Widget>,
    scroll: &gtk::ScrolledWindow,
    footer: Option<&gtk::Widget>,
) -> Option<(f32, f32, f32, f64)> {
    if row.height() <= 0 || row.width() <= 0 || scroll.height() <= 0 {
        return None;
    }
    let bounds = row.compute_bounds(scroll)?;
    let bottom = footer.map_or(Some(scroll.height() as f32), |footer| {
        Some((scroll.height() as f32).min(footer.compute_bounds(scroll)?.y()))
    })?;
    if bottom <= 0.0 {
        return None;
    }
    let delta = if bounds.y() < 0.0 {
        bounds.y()
    } else {
        (bounds.y() + bounds.height() - bottom).max(0.0)
    };
    let adjustment = scroll.vadjustment();
    if delta != 0.0 {
        adjustment.set_value(adjustment.value() + f64::from(delta));
        return None;
    }
    Some((bounds.y(), bounds.height(), bottom, adjustment.value()))
}

pub(super) struct PendingEntryRename {
    depth: usize,
    parent: Location,
    reveal_generation: u64,
}

pub(in crate::ui) fn set_rename_label(label: &gtk::Widget, name: &str) {
    if let Some(label) = label.downcast_ref::<gtk::Inscription>() {
        label.set_text(Some(name));
    } else if let Some(label) = label.downcast_ref::<gtk::Label>() {
        label.set_label(name);
    }
}

fn pending_rename_matches(pending: &PendingRename, location: &Location) -> bool {
    pending.old_location == *location
        || (matches!(&pending.state, PendingRenameState::AwaitingRefresh { .. })
            && pending
                .new_location
                .as_ref()
                .is_some_and(|new_location| new_location == location))
}

impl super::BrowserView {
    pub(in crate::ui) fn install_inline_edit_dismissal(&self, root: &impl IsA<gtk::Widget>) {
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&self.state);
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(state) = weak.upgrade() else { return };
            state.pending_new_entry.take();
            let target = gesture
                .widget()
                .and_then(|root| root.pick(x, y, gtk::PickFlags::DEFAULT));
            let field = state
                .active_rename
                .borrow()
                .as_ref()
                .map(|active| active.field.clone())
                .or_else(|| state.mode_views.borrow().active_rename_field());
            if let Some(field) = field
                && !target
                    .as_ref()
                    .is_some_and(|target| target == &field || target.is_ancestor(&field))
            {
                state.submit_rename(&field);
            }
            state.yield_rename_reveal();
        });
        root.add_controller(click);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&self.state);
        scroll.connect_scroll(move |_, _, _| {
            if let Some(state) = weak.upgrade() {
                state.yield_rename_reveal();
            }
            gtk::glib::Propagation::Proceed
        });
        root.add_controller(scroll);
    }
}

pub(in crate::ui) fn queue_rename(
    browser: &Rc<crate::app::Browser>,
    entry: FileEntry,
    name: String,
) {
    if name == entry.display_name || validate_basename(&name).is_err() {
        return;
    }
    // A rename can synchronously refresh models; dispatch after GTK's focus walk.
    let browser = Rc::downgrade(browser);
    gtk::glib::idle_add_local_once(move || {
        if let Some(browser) = browser.upgrade() {
            browser.rename(entry, name);
        }
    });
}

struct RenameRevealContext {
    old: Location,
    target: Location,
    depth: usize,
    mode: BrowserMode,
    generation: u64,
    reveal_generation: u64,
    deadline: std::time::Instant,
}

enum RenameRevealTarget {
    Wait,
    Stop,
    Ready {
        source_position: usize,
        collection: gtk::Widget,
        position: u32,
        row: Option<gtk::Widget>,
        footer: Option<gtk::Widget>,
        loading: bool,
    },
}

fn prepare_rename_reveal(
    source_position: Option<usize>,
    resolved_source_position: usize,
    loading: bool,
    scroll_value: &std::cell::Cell<Option<f64>>,
) -> bool {
    if source_position != Some(resolved_source_position) {
        scroll_value.set(None);
    }
    loading
}

impl ViewState {
    fn rename_collection_view(&self, depth: usize, mode: BrowserMode) -> Option<gtk::Widget> {
        match mode {
            BrowserMode::Columns => self
                .columns
                .borrow()
                .get(depth)
                .map(|column| column.list.clone().upcast()),
            BrowserMode::List => self
                .mode_views
                .borrow()
                .list_rename_view(depth)
                .map(|view| view.upcast()),
            BrowserMode::Icons => self.mode_views.borrow().icons_rename_view(depth),
        }
    }

    fn rename_reveal_is_valid(&self, list: &gtk::Widget, context: &RenameRevealContext) -> bool {
        if std::time::Instant::now() >= context.deadline
            || self.rename_generation.get() != context.generation
            || self.rename_reveal_generation.get() != context.reveal_generation
            || self.browser.active_depth() != Some(context.depth)
        {
            return false;
        }
        let selected = self.browser.selected_entries();
        selected.len() == 1
            && (selected[0].location == context.old || selected[0].location == context.target)
            && self.mode_views.borrow().mode() == context.mode
            && self
                .rename_collection_view(context.depth, context.mode)
                .as_ref()
                == Some(list)
    }

    fn resolve_rename_reveal_target(&self, context: &RenameRevealContext) -> RenameRevealTarget {
        let Some(snapshot) = self.browser.column_snapshot(context.depth) else {
            return RenameRevealTarget::Stop;
        };
        if snapshot.location
            != context
                .target
                .parent()
                .unwrap_or_else(|| context.target.clone())
        {
            return RenameRevealTarget::Stop;
        }
        let position = (0..snapshot.count).find(|position| {
            self.browser
                .entry_at(context.depth, *position)
                .is_some_and(|entry| entry.location == context.target)
        });
        let Some(source_position) = position else {
            return RenameRevealTarget::Wait;
        };
        let loading = snapshot.loading;
        let (collection, position, row, footer) = if context.mode == BrowserMode::Columns {
            let column = self.columns.borrow()[context.depth].clone();
            let Some(position) = column.map.view_position(source_position) else {
                return RenameRevealTarget::Stop;
            };
            let row = column.bound_rows.borrow().iter().find_map(|bound| {
                (bound.item.upgrade()?.position() == position)
                    .then(|| bound.row.upgrade())
                    .flatten()
                    .filter(|row| row.is_mapped() && row.is_ancestor(&column.list))
                    .map(|row| row.upcast::<gtk::Widget>())
            });
            (column.list.clone().upcast(), position, row, None)
        } else if context.mode == BrowserMode::Icons {
            let Some((collection, position, row)) = self
                .mode_views
                .borrow()
                .icons_rename_row(context.depth, source_position)
            else {
                return RenameRevealTarget::Wait;
            };
            (collection, position, row, None)
        } else {
            let Some((position, row)) = self
                .mode_views
                .borrow()
                .list_rename_row(context.depth, source_position)
            else {
                return RenameRevealTarget::Stop;
            };
            let Some(collection) = self.rename_collection_view(context.depth, context.mode) else {
                return RenameRevealTarget::Stop;
            };
            (collection, position, row, None)
        };
        RenameRevealTarget::Ready {
            source_position,
            collection,
            position,
            row,
            footer,
            loading,
        }
    }

    pub(in crate::ui) fn rename_reveal_generation(&self) -> u64 {
        self.rename_reveal_generation.get()
    }

    fn yield_rename_reveal(&self) {
        self.rename_reveal_generation
            .set(self.rename_reveal_generation.get().wrapping_add(1));
    }

    pub(super) fn rename_operation_pending(&self) -> bool {
        self.pending_rename.borrow().is_some()
    }

    pub(in crate::ui) fn pending_rename_name(&self, entry: &FileEntry) -> Option<String> {
        self.pending_rename
            .borrow()
            .as_ref()
            .filter(|pending| pending_rename_matches(pending, &entry.location))
            .map(|pending| pending.new_name.clone())
    }

    fn start_pending_rename(&self, entry: &FileEntry, new_name: String) -> u64 {
        let new_location = entry
            .location
            .parent()
            .and_then(|parent| parent.child(std::ffi::OsStr::new(&new_name)));
        let generation = self.rename_generation.get().saturating_add(1);
        self.rename_generation.set(generation);
        self.pending_rename.replace(Some(PendingRename {
            old_location: entry.location.clone(),
            new_location,
            old_name: entry.display_name.clone(),
            new_name,
            generation,
            monitor_has_new_location: false,
            reveal_generation: self.rename_reveal_generation.get(),
            source_position: self.browser.rename_item().map(|(_, position, _)| position),
            scroll_value: self.browser.active_depth().and_then(|depth| {
                let view = self.rename_collection_view(depth, self.mode_views.borrow().mode())?;
                view.ancestor(gtk::ScrolledWindow::static_type())
                    .and_downcast::<gtk::ScrolledWindow>()
                    .map(|scroll| scroll.vadjustment().value())
            }),
            state: PendingRenameState::Queued,
        }));
        generation
    }

    fn queue_pending_rename(self: &Rc<Self>, entry: FileEntry, name: String, generation: u64) {
        let browser = self.browser.clone();
        let operation_at_queue = browser.last_started_operation();
        let weak = Rc::downgrade(self);
        gtk::glib::idle_add_local_once(move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.browser.last_started_operation() != operation_at_queue {
                let abandoned = state
                    .pending_rename
                    .borrow()
                    .as_ref()
                    .is_some_and(|pending| pending.generation == generation);
                if abandoned {
                    state.fail_pending_rename();
                }
                return;
            }
            let dispatch = state
                .pending_rename
                .borrow_mut()
                .as_mut()
                .filter(|pending| pending.generation == generation)
                .is_some_and(PendingRename::begin_dispatch);
            if dispatch {
                let operation_id = state.browser.rename(entry, name);
                if let Some(operation_id) = operation_id
                    && let Some(pending) = state
                        .pending_rename
                        .borrow_mut()
                        .as_mut()
                        .filter(|pending| pending.generation == generation)
                {
                    pending.finish_dispatch(operation_id);
                }
            }
        });
    }

    fn rename_parent_is_visible(&self, pending: &PendingRename) -> bool {
        let Some(parent) = pending.old_location.parent() else {
            return false;
        };
        (0..)
            .map_while(|depth| self.browser.column_snapshot(depth))
            .any(|snapshot| snapshot.location == parent)
    }

    pub(super) fn note_pending_rename_splices(
        &self,
        depth: usize,
        splices: &[crate::app::EntrySplice],
    ) {
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        let completed = {
            let mut pending = self.pending_rename.borrow_mut();
            let Some(pending) = pending.as_mut() else {
                return;
            };
            let Some(new_location) = pending.new_location.as_ref() else {
                return;
            };
            if pending.old_location.parent().as_ref() != Some(&snapshot.location)
                || !splices
                    .iter()
                    .flat_map(|splice| splice.entries.iter())
                    .any(|entry| &entry.location == new_location)
            {
                return;
            }
            for entry in splices.iter().flat_map(|splice| &splice.entries) {
                if &entry.location == new_location {
                    crate::ui::thumbnail::preserve_renamed_thumbnail(&pending.old_location, entry);
                }
            }
            pending.monitor_has_new_location = true;
            matches!(&pending.state, PendingRenameState::AwaitingRefresh { .. })
        };
        if completed {
            self.pending_rename.take();
        }
    }

    pub(super) fn reconcile_pending_rename(&self) {
        let abandoned = self
            .pending_rename
            .borrow()
            .as_ref()
            .is_some_and(|pending| {
                matches!(&pending.state, PendingRenameState::AwaitingRefresh { .. })
                    && !self.rename_parent_is_visible(pending)
            });
        if abandoned {
            self.pending_rename.take();
        }
    }

    pub(super) fn note_pending_rename_refresh(&self, depth: usize) {
        let Some(request_id) = self.browser.column_request_id(depth) else {
            return;
        };
        let Some(parent) = self
            .pending_rename
            .borrow()
            .as_ref()
            .and_then(|pending| pending.old_location.parent())
        else {
            return;
        };
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        if snapshot.location != parent {
            return;
        }
        let mut pending = self.pending_rename.borrow_mut();
        let Some(pending) = pending.as_mut() else {
            return;
        };
        let PendingRenameState::AwaitingRefresh { requests } = &mut pending.state else {
            return;
        };
        requests.retain(|(pending_depth, _)| *pending_depth != depth);
        requests.push((depth, request_id));
    }

    pub(super) fn reconcile_pending_rename_after_load(&self, depth: usize) {
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        if snapshot.loading {
            return;
        }
        let Some(request_id) = self.browser.column_request_id(depth) else {
            return;
        };
        let finished = {
            let mut pending = self.pending_rename.borrow_mut();
            let Some(pending) = pending.as_mut() else {
                return;
            };
            let PendingRenameState::AwaitingRefresh { requests } = &mut pending.state else {
                return;
            };
            let expected = requests
                .iter()
                .position(|(pending_depth, pending_request)| {
                    *pending_depth == depth && *pending_request == request_id
                });
            let Some(index) = expected else {
                return;
            };
            requests.remove(index);
            requests.is_empty()
        };
        if finished {
            self.pending_rename.take();
        }
    }

    pub(super) fn abandon_pending_rename(&self, operation_id: OperationRequestId) {
        let owned = self
            .pending_rename
            .borrow()
            .as_ref()
            .is_some_and(|pending| {
                pending.owns_operation(operation_id, self.browser.last_started_operation())
            });
        if owned {
            self.fail_pending_rename();
        }
    }

    pub(super) fn complete_pending_rename(self: &Rc<Self>, operation_id: OperationRequestId) {
        let completed = {
            let mut pending = self.pending_rename.borrow_mut();
            let Some(pending) = pending.as_mut() else {
                return;
            };
            if !pending.complete(operation_id, self.browser.last_started_operation()) {
                return;
            }
            (
                pending.old_location.clone(),
                pending.new_location.clone(),
                pending.new_name.clone(),
            )
        };
        let (old_location, new_location, new_name) = completed;
        self.update_rename_labels(&old_location, new_location.as_ref(), &new_name);
        if let Some(location) = new_location
            && self
                .pending_rename
                .borrow()
                .as_ref()
                .is_some_and(|pending| {
                    pending.reveal_generation == self.rename_reveal_generation.get()
                })
        {
            self.reveal_completed_rename(old_location, location);
        }
        let observed = self
            .pending_rename
            .borrow()
            .as_ref()
            .is_some_and(|pending| pending.monitor_has_new_location);
        if observed {
            self.pending_rename.take();
        } else {
            self.reconcile_pending_rename();
        }
    }

    fn reveal_completed_rename(self: &Rc<Self>, old: Location, target: Location) {
        let Some(depth) = self.browser.active_depth() else {
            return;
        };
        let mode = self.mode_views.borrow().mode();
        let view = self.rename_collection_view(depth, mode);
        let Some(view) = view else { return };
        let Some(scroll) = view
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
        else {
            return;
        };
        let weak = Rc::downgrade(self);
        let context = RenameRevealContext {
            old,
            target,
            depth,
            mode,
            generation: self.rename_generation.get(),
            reveal_generation: self.rename_reveal_generation.get(),
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(5),
        };
        let scroll_value = std::cell::Cell::new(
            self.pending_rename
                .borrow()
                .as_ref()
                .and_then(|pending| pending.scroll_value),
        );
        let source_position = self
            .pending_rename
            .borrow()
            .as_ref()
            .and_then(|pending| pending.source_position);
        let settled_bounds = std::cell::Cell::new(None);
        view.add_tick_callback(move |list, _| {
            let Some(state) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            if !state.rename_reveal_is_valid(list, &context) {
                return gtk::glib::ControlFlow::Break;
            }
            let (resolved_source_position, collection, position, row, footer, loading) =
                match state.resolve_rename_reveal_target(&context) {
                    RenameRevealTarget::Stop => return gtk::glib::ControlFlow::Break,
                    RenameRevealTarget::Wait => return gtk::glib::ControlFlow::Continue,
                    RenameRevealTarget::Ready {
                        source_position: resolved_source_position,
                        collection,
                        position,
                        row,
                        footer,
                        loading: is_loading,
                    } => (
                        resolved_source_position,
                        collection,
                        position,
                        row,
                        footer,
                        is_loading,
                    ),
                };
            if prepare_rename_reveal(
                source_position,
                resolved_source_position,
                loading,
                &scroll_value,
            ) || list.height() <= 1
            {
                return gtk::glib::ControlFlow::Continue;
            }
            let Some(row) = row else {
                super::collection::apply_collection_scroll(
                    &collection,
                    position,
                    gtk::ListScrollFlags::NONE,
                );
                return gtk::glib::ControlFlow::Continue;
            };
            // A newly rebound item can have CSS bounds but no allocation. Focusing it
            // then gives GTK a zero-origin scroll anchor and sends the column to the top.
            if row.height() <= 0 || row.width() <= 0 {
                super::collection::apply_collection_scroll(
                    &collection,
                    position,
                    gtk::ListScrollFlags::NONE,
                );
                return gtk::glib::ControlFlow::Continue;
            }
            // Model splices may replace GTK's scroll anchor even when the rename
            // stays in place. Reveal from the pre-refresh viewport instead.
            if let Some(value) = scroll_value.get() {
                scroll.vadjustment().set_value(value);
            }
            // GTK may keep a removed native row as root focus after a splice.
            // Recover it without taking focus from an attached outside control.
            if let Some(focus) = list.root().and_then(|root| root.focus())
                && (focus == *list
                    || focus.is_ancestor(list)
                    || focus == collection
                    || focus.is_ancestor(&collection)
                    || list.is_ancestor(&focus)
                    || focus.root().is_none())
                && let Some(cursor) = row.parent()
            {
                let adjustment = scroll.vadjustment();
                let value = adjustment.value();
                if focus.root().is_none() {
                    if let Some(root) = list.root() {
                        root.set_focus(Some(&cursor));
                    }
                } else {
                    cursor.grab_focus();
                }
                adjustment.set_value(value);
            }
            let allocated = reveal_rename_row(&row, &scroll, footer.as_ref());
            if scroll_value.get().is_some() {
                scroll_value.set(Some(scroll.vadjustment().value()));
            }
            if allocated.is_some() && settled_bounds.get() == allocated {
                return gtk::glib::ControlFlow::Break;
            }
            settled_bounds.set(allocated);
            // Tick callbacks precede allocation; confirm visibility after GTK's anchor update.
            gtk::glib::ControlFlow::Continue
        });
    }

    pub(super) fn fail_pending_rename(&self) {
        let Some(pending) = self.pending_rename.take() else {
            return;
        };
        self.update_rename_labels(&pending.old_location, None, &pending.old_name);
    }

    pub(super) fn fail_pending_rename_from_browser(
        &self,
        operation_id: Option<OperationRequestId>,
    ) {
        let owned =
            self.pending_rename
                .borrow()
                .as_ref()
                .is_some_and(|pending| match operation_id {
                    None => matches!(
                        pending.state,
                        PendingRenameState::Queued | PendingRenameState::Dispatching
                    ),
                    Some(actual) => {
                        pending.owns_operation(actual, self.browser.last_started_operation())
                    }
                });
        if owned {
            self.fail_pending_rename();
        }
    }

    pub(in crate::ui) fn rename_label_widgets(
        &self,
        old_location: &Location,
        new_location: Option<&Location>,
    ) -> Vec<gtk::Widget> {
        let mut labels = Vec::new();
        {
            let columns = self.columns.borrow();
            for (depth, column) in columns.iter().enumerate() {
                column.bound_rows.borrow_mut().retain(|bound| {
                    let (Some(item), Some(_row)) = (bound.item.upgrade(), bound.row.upgrade())
                    else {
                        return false;
                    };
                    let Some(position) = column.map.source_position(item.position()) else {
                        return true;
                    };
                    let Some(entry) = self.browser.entry_at(depth, position) else {
                        return true;
                    };
                    if (entry.location == *old_location
                        || new_location.is_some_and(|location| location == &entry.location))
                        && let Some(label) = bound.rename_label.upgrade()
                    {
                        labels.push(label.upcast());
                    }
                    true
                });
            }
        }
        labels.extend(
            self.mode_views
                .borrow()
                .rename_label_widgets(old_location, new_location),
        );
        labels
    }

    fn update_rename_labels(
        &self,
        old_location: &Location,
        new_location: Option<&Location>,
        name: &str,
    ) {
        for label in self.rename_label_widgets(old_location, new_location) {
            set_rename_label(&label, name);
        }
    }

    pub(super) fn rename_created_entry(self: &Rc<Self>, location: &Location) {
        let Some(pending) = self
            .pending_new_entry
            .borrow()
            .clone()
            .filter(|pending| Some(pending.parent.clone()) == location.parent())
        else {
            return;
        };
        let location = location.clone();
        let weak = Rc::downgrade(self);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let selected = std::cell::Cell::new(false);
        // Wait for the refreshed listing and the virtualized row to be allocated.
        self.overlay.add_tick_callback(move |_, _| {
            let Some(state) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            if !state
                .pending_new_entry
                .borrow()
                .as_ref()
                .is_some_and(|current| Rc::ptr_eq(current, &pending))
            {
                return gtk::glib::ControlFlow::Break;
            }
            if std::time::Instant::now() >= deadline
                || state.rename_reveal_generation.get() != pending.reveal_generation
                || state.browser.location_at(pending.depth).as_ref() != Some(&pending.parent)
            {
                state.pending_new_entry.take();
                return gtk::glib::ControlFlow::Break;
            }
            let Some(snapshot) = state
                .browser
                .column_snapshot(pending.depth)
                .filter(|snapshot| !snapshot.loading)
            else {
                return gtk::glib::ControlFlow::Continue;
            };
            let position = state
                .browser
                .with_entries(pending.depth, 0..snapshot.count, |entries| {
                    entries.iter().position(|entry| entry.location == location)
                })
                .flatten();
            if let Some(position) = position {
                if !selected.replace(true) {
                    if state.mode_views.borrow().mode() == BrowserMode::Columns {
                        state.browser.reveal_created_entry(pending.depth, position);
                        // Revealing the child synchronously truncates columns and cancels
                        // pending editors. Retain this creation's authority for the next frame.
                        state.pending_new_entry.replace(Some(pending.clone()));
                    } else {
                        state.browser.select(pending.depth, position);
                    }
                } else if let Some(entry) = state.browser.entry_at(pending.depth, position)
                    && state.begin_rename_item(pending.depth, position, entry)
                {
                    state.pending_new_entry.take();
                    return gtk::glib::ControlFlow::Break;
                }
                if state.mode_views.borrow().mode() == BrowserMode::Columns
                    && let Some(column) = state.columns.borrow().get(pending.depth)
                    && let Some(position) = column.map.view_position(position)
                {
                    column
                        .list
                        .scroll_to(position, gtk::ListScrollFlags::NONE, None);
                }
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    pub(super) fn begin_new_entry(
        self: &Rc<Self>,
        depth: usize,
        location: Location,
        is_directory: bool,
    ) {
        if is_trash_location(&location) || location.is_recent_location() {
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
            })));
        if is_directory {
            self.browser.create_new_folder(location);
        } else {
            self.browser.create_new_file(location);
        }
    }

    pub(super) fn cancel_new_entry(&self) -> bool {
        self.pending_new_entry.take().is_some()
    }

    pub(in crate::ui) fn schedule_click_rename(
        self: &Rc<Self>,
        depth: usize,
        source_position: usize,
    ) {
        self.cancel_click_rename();
        let Some(entry) = self.browser.entry_at(depth, source_position) else {
            return;
        };
        let generation = self.click_rename_generation.get() + 1;
        self.click_rename_generation.set(generation);
        let interval = self.scroller.settings().gtk_double_click_time().max(1) as u64;
        let weak = Rc::downgrade(self);
        let id = gtk::glib::timeout_add_local_once(
            std::time::Duration::from_millis(interval),
            move || {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                if state.click_rename_generation.get() != generation {
                    return;
                }
                state.pending_click_rename.take();
                if state.rename_operation_pending()
                    || state.active_rename.borrow().is_some()
                    || state
                        .scroller
                        .root()
                        .and_then(|root| root.focus())
                        .as_ref()
                        .is_some_and(crate::ui::focus_navigation::editable)
                    || state.browser.selected_entries().len() != 1
                    || !state.browser.focused_item().is_some_and(
                        |(current_depth, position, current)| {
                            current_depth == depth
                                && position == source_position
                                && current.location == entry.location
                        },
                    )
                {
                    return;
                }
                state.begin_rename();
            },
        );
        self.pending_click_rename.replace(Some(id));
    }

    pub(in crate::ui) fn cancel_click_rename(&self) {
        if let Some(id) = self.pending_click_rename.take() {
            id.remove();
        }
        self.click_rename_generation
            .set(self.click_rename_generation.get() + 1);
    }

    pub(in crate::ui) fn begin_entry_rename(
        self: &Rc<Self>,
        depth: usize,
        entry: &FileEntry,
    ) -> bool {
        if self.begin_search_result_rename(depth, entry) {
            return true;
        }
        self.sync_mode_selection();
        if !self
            .browser
            .rename_item()
            .is_some_and(|(_, _, selected)| selected.location == entry.location)
        {
            return false;
        }
        self.begin_rename()
    }

    pub(in crate::ui) fn begin_search_result_rename(
        self: &Rc<Self>,
        depth: usize,
        entry: &FileEntry,
    ) -> bool {
        self.cancel_click_rename();
        if self.rename_operation_pending() || is_trash_location(&entry.location) {
            return false;
        }
        self.cancel_new_entry();
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            return self.mode_views.borrow().begin_search_rename(depth, entry);
        }
        let Some(path) = entry.location.native_path() else {
            return false;
        };
        let position = {
            let columns = self.columns.borrow();
            let Some(column) = columns.get(depth) else {
                return false;
            };
            if !column.recursive_search_active.get() {
                return false;
            }
            column
                .search_results
                .borrow()
                .iter()
                .position(|item| item.path == path)
        };
        let Some(position) = position.and_then(|position| u32::try_from(position).ok()) else {
            return false;
        };
        self.cancel_rename();
        let Some(target) = self.resolve_columns_rename_target_at_view_position(depth, position)
        else {
            return false;
        };
        self.activate_columns_rename(target, entry.clone());
        true
    }

    pub(super) fn begin_rename(self: &Rc<Self>) -> bool {
        self.cancel_click_rename();
        if self.rename_operation_pending() {
            return false;
        }
        self.cancel_new_entry();
        self.sync_mode_selection();
        let Some((depth, source_position, entry)) = self.browser.rename_item() else {
            return false;
        };
        self.begin_rename_item(depth, source_position, entry)
    }

    fn begin_rename_item(
        self: &Rc<Self>,
        depth: usize,
        source_position: usize,
        entry: FileEntry,
    ) -> bool {
        if is_trash_location(&entry.location) {
            return false;
        }
        if self.mode_views.borrow().mode() != BrowserMode::Columns {
            return self
                .mode_views
                .borrow()
                .begin_rename(depth, source_position, &entry);
        }
        self.cancel_rename();
        let Some(target) = self.resolve_columns_rename_target(depth, source_position) else {
            return false;
        };
        self.activate_columns_rename(target, entry);
        true
    }

    fn resolve_columns_rename_target(
        &self,
        depth: usize,
        source_position: usize,
    ) -> Option<ColumnsRenameTarget> {
        let filtered_position = self
            .columns
            .borrow()
            .get(depth)?
            .map
            .view_position(source_position)?;
        self.resolve_columns_rename_target_at_view_position(depth, filtered_position)
    }

    fn resolve_columns_rename_target_at_view_position(
        &self,
        depth: usize,
        filtered_position: u32,
    ) -> Option<ColumnsRenameTarget> {
        let columns = self.columns.borrow();
        let column = columns.get(depth)?;
        // Prepare before checking allocation: it cancels deferred scrolling and lets GTK bind
        // the row needed by the editor.
        super::prepare_collection_inline_edit(column.list.upcast_ref(), filtered_position);
        let bound_rows = column.bound_rows.borrow();
        let bound = bound_rows.iter().find(|bound| {
            bound
                .item
                .upgrade()
                .is_some_and(|item| item.position() == filtered_position)
        })?;
        let row = bound.row.upgrade()?;
        if !row.is_mapped()
            || row.width() <= 0
            || !row.is_ancestor(&column.list)
            || column.presentation.stack.is_transition_running()
        {
            return None;
        }
        Some(ColumnsRenameTarget {
            row,
            edit: bound.edit.clone(),
            spacer: bound.spacer.clone(),
            size: bound.size.clone(),
            scroll: column.listing_scroll.clone(),
        })
    }

    fn activate_columns_rename(self: &Rc<Self>, target: ColumnsRenameTarget, entry: FileEntry) {
        let ColumnsRenameTarget {
            row,
            edit,
            spacer,
            size,
            scroll,
        } = target;
        spacer.set_visible(false);
        size.set_visible(false);
        constrain_rename_to_viewport(&edit.field, &self.scroller);
        let viewport = self.scroller.downgrade();
        let row = row.downgrade();
        let scroll = scroll.downgrade();
        let weak = Rc::downgrade(self);
        let reveal_generation = self.rename_reveal_generation.get();
        let mut target = crate::ui::collection_edit::EditTarget::from(edit.clone());
        target.reveal = Some(Rc::new(move |field| {
            if let Some(viewport) = viewport.upgrade() {
                constrain_rename_to_viewport(field, &viewport);
            }
            if !weak
                .upgrade()
                .is_some_and(|state| state.rename_reveal_generation.get() == reveal_generation)
            {
                return;
            }
            if let (Some(row), Some(scroll)) = (row.upgrade(), scroll.upgrade()) {
                reveal_rename_row(&row, &scroll, None);
            }
        }));
        let field = edit.field.downgrade();
        target.finish = Some(Rc::new(move || {
            if let Some(field) = field.upgrade() {
                field.set_margin_start(0);
                field.set_margin_end(0);
            }
            spacer.set_visible(true);
            size.set_visible(!size.label().is_empty());
        }));
        let state = Rc::downgrade(self);
        crate::ui::collection_edit::begin(
            &self.active_rename,
            entry,
            target,
            Rc::new(move |field| {
                if let Some(state) = state.upgrade() {
                    state.submit_rename(field);
                }
            }),
        );
    }

    pub(super) fn cancel_rename(&self) -> bool {
        self.cancel_click_rename();
        let mode_rename = self.mode_views.borrow().take_rename();
        let cancelled = mode_rename.is_some();
        drop(mode_rename);
        crate::ui::collection_edit::cancel(&self.active_rename) || cancelled
    }

    fn submit_rename_entry(self: &Rc<Self>, entry: FileEntry, name: String) {
        let valid_change = name != entry.display_name && validate_basename(&name).is_ok();
        if !valid_change {
            return;
        }
        let generation = self.start_pending_rename(&entry, name.clone());
        self.update_rename_labels(&entry.location, None, &name);
        self.queue_pending_rename(entry, name, generation);
    }

    pub(in crate::ui) fn submit_mode_rename(self: &Rc<Self>, field: &gtk::Entry) {
        let active_rename = self
            .mode_views
            .try_borrow_mut()
            .ok()
            .and_then(|mode_views| mode_views.take_active_rename(field));
        let Some((entry, name)) = active_rename else {
            return;
        };
        self.submit_rename_entry(entry, name);
    }

    pub(super) fn submit_rename(self: &Rc<Self>, field: &gtk::Entry) {
        let mode_field_active = self
            .mode_views
            .try_borrow()
            .ok()
            .and_then(|mode_views| mode_views.active_rename_field())
            .as_ref()
            == Some(field);
        if mode_field_active {
            self.submit_mode_rename(field);
            return;
        }
        let Some((entry, name)) =
            crate::ui::collection_edit::take_submission(&self.active_rename, field)
        else {
            return;
        };
        self.submit_rename_entry(entry, name);
    }
}
