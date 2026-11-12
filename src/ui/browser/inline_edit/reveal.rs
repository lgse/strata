// SPDX-License-Identifier: MIT

use crate::model::Location;
use crate::ui::browser::ViewState;
use crate::ui::browser_modes::BrowserMode;
use gtk::glib::ControlFlow;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

fn has_allocation(widget: &impl IsA<gtk::Widget>) -> bool {
    widget.height() > 0 && widget.width() > 0
}

/// The visible bottom edge of `scroll`, above `footer` when one overlaps it.
fn visible_bottom(scroll: &gtk::ScrolledWindow, footer: Option<&gtk::Widget>) -> Option<f32> {
    let height = scroll.height() as f32;
    match footer {
        None => Some(height),
        Some(footer) => Some(height.min(footer.compute_bounds(scroll)?.y())),
    }
}

pub(in crate::ui) fn reveal_rename_row(
    row: &impl IsA<gtk::Widget>,
    scroll: &gtk::ScrolledWindow,
    footer: Option<&gtk::Widget>,
) -> Option<(f32, f32, f32, f64)> {
    if !has_allocation(row) || scroll.height() <= 0 {
        return None;
    }
    let bounds = row.compute_bounds(scroll)?;
    let bottom = visible_bottom(scroll, footer)?;
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

struct RenameRevealContext {
    old: Location,
    target: Location,
    depth: usize,
    mode: BrowserMode,
    generation: u64,
    reveal_generation: u64,
    deadline: std::time::Instant,
}

struct RevealRow {
    source_position: usize,
    collection: gtk::Widget,
    position: u32,
    row: Option<gtk::Widget>,
    footer: Option<gtk::Widget>,
    loading: bool,
}

enum RenameRevealTarget {
    Wait,
    Stop,
    Ready(RevealRow),
}

/// Per-reveal state the tick callback carries between frames.
struct RevealProgress {
    scroll: gtk::ScrolledWindow,
    scroll_value: Cell<Option<f64>>,
    source_position: Option<usize>,
    settled_bounds: Cell<Option<(f32, f32, f32, f64)>>,
}

impl RevealProgress {
    /// A row that moved within the listing loses the pre-refresh viewport.
    fn waits_for(&self, target: &RevealRow, list: &gtk::Widget) -> bool {
        if self.source_position != Some(target.source_position) {
            self.scroll_value.set(None);
        }
        target.loading || list.height() <= 1
    }
}

/// GTK may keep a removed native row as root focus after a splice; these are the
/// focus owners a reveal may take back without stealing from an outside control.
fn focus_belongs_to_listing(
    focus: &gtk::Widget,
    list: &gtk::Widget,
    collection: &gtk::Widget,
) -> bool {
    let inside = |widget: &gtk::Widget| focus == widget || focus.is_ancestor(widget);
    inside(list) || inside(collection) || list.is_ancestor(focus) || focus.root().is_none()
}

fn recover_listing_focus(
    list: &gtk::Widget,
    collection: &gtk::Widget,
    row: &gtk::Widget,
    scroll: &gtk::ScrolledWindow,
) {
    let Some(focus) = list.root().and_then(|root| root.focus()) else {
        return;
    };
    if !focus_belongs_to_listing(&focus, list, collection) {
        return;
    }
    let Some(cursor) = row.parent() else {
        return;
    };
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

impl ViewState {
    pub(super) fn rename_collection_view(
        &self,
        depth: usize,
        mode: BrowserMode,
    ) -> Option<gtk::Widget> {
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
            BrowserMode::Icons | BrowserMode::Compact => {
                self.mode_views.borrow().icons_rename_view(depth)
            }
        }
    }

    fn rename_reveal_is_current(&self, context: &RenameRevealContext) -> bool {
        std::time::Instant::now() < context.deadline
            && self.rename_generation.get() == context.generation
            && self.rename_reveal_generation.get() == context.reveal_generation
            && self.browser.active_depth() == Some(context.depth)
    }

    fn rename_selection_is_target(&self, context: &RenameRevealContext) -> bool {
        let selected = self.browser.selected_entries();
        let [only] = selected.as_slice() else {
            return false;
        };
        only.location == context.old || only.location == context.target
    }

    fn rename_reveal_is_valid(&self, list: &gtk::Widget, context: &RenameRevealContext) -> bool {
        if !self.rename_reveal_is_current(context) || !self.rename_selection_is_target(context) {
            return false;
        }
        self.mode_views.borrow().mode() == context.mode
            && self
                .rename_collection_view(context.depth, context.mode)
                .as_ref()
                == Some(list)
    }

    fn rename_target_position(&self, context: &RenameRevealContext) -> Option<usize> {
        self.browser
            .with_column_entries(context.depth, |entries| {
                entries
                    .iter()
                    .position(|entry| entry.location == context.target)
            })
            .flatten()
    }

    fn resolve_rename_reveal_target(&self, context: &RenameRevealContext) -> RenameRevealTarget {
        let Some(snapshot) = self.browser.column_snapshot(context.depth) else {
            return RenameRevealTarget::Stop;
        };
        let parent = context
            .target
            .parent()
            .unwrap_or_else(|| context.target.clone());
        if snapshot.location != parent {
            return RenameRevealTarget::Stop;
        }
        let Some(source_position) = self.rename_target_position(context) else {
            return RenameRevealTarget::Wait;
        };
        let found = match context.mode {
            BrowserMode::Columns => self.columns_reveal_row(context.depth, source_position),
            BrowserMode::Icons | BrowserMode::Compact => {
                self.icons_reveal_row(context.depth, source_position)
            }
            BrowserMode::List => self.list_reveal_row(context, source_position),
        };
        match found {
            Ok((collection, position, row)) => RenameRevealTarget::Ready(RevealRow {
                source_position,
                collection,
                position,
                row,
                footer: None,
                loading: snapshot.loading,
            }),
            Err(target) => target,
        }
    }

    fn columns_reveal_row(
        &self,
        depth: usize,
        source_position: usize,
    ) -> Result<(gtk::Widget, u32, Option<gtk::Widget>), RenameRevealTarget> {
        let column = self.columns.borrow()[depth].clone();
        // Filter results own the rows and follow the rename themselves.
        if column.recursive_search_active.get() {
            return Err(RenameRevealTarget::Stop);
        }
        let Some(position) = column.map.view_position(source_position) else {
            return Err(RenameRevealTarget::Stop);
        };
        let row = column
            .shown_row_at(position)
            .map(|(row, _)| row.upcast::<gtk::Widget>());
        Ok((column.list.clone().upcast(), position, row))
    }

    fn icons_reveal_row(
        &self,
        depth: usize,
        source_position: usize,
    ) -> Result<(gtk::Widget, u32, Option<gtk::Widget>), RenameRevealTarget> {
        self.mode_views
            .borrow()
            .icons_rename_row(depth, source_position)
            .ok_or(RenameRevealTarget::Wait)
    }

    fn list_reveal_row(
        &self,
        context: &RenameRevealContext,
        source_position: usize,
    ) -> Result<(gtk::Widget, u32, Option<gtk::Widget>), RenameRevealTarget> {
        let Some((position, row)) = self
            .mode_views
            .borrow()
            .list_rename_row(context.depth, source_position)
        else {
            return Err(RenameRevealTarget::Stop);
        };
        let Some(collection) = self.rename_collection_view(context.depth, context.mode) else {
            return Err(RenameRevealTarget::Stop);
        };
        Ok((collection, position, row))
    }

    pub(in crate::ui) fn rename_reveal_generation(&self) -> u64 {
        self.rename_reveal_generation.get()
    }

    pub(super) fn yield_rename_reveal(&self) {
        self.rename_reveal_generation
            .set(self.rename_reveal_generation.get().wrapping_add(1));
    }

    pub(super) fn reveal_completed_rename(self: &Rc<Self>, old: Location, target: Location) {
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
        let pending = self.pending_rename.borrow();
        let progress = RevealProgress {
            scroll,
            scroll_value: Cell::new(pending.as_ref().and_then(|pending| pending.scroll_value)),
            source_position: pending.as_ref().and_then(|pending| pending.source_position),
            settled_bounds: Cell::new(None),
        };
        drop(pending);
        view.add_tick_callback(move |list, _| {
            let Some(state) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            state.reveal_rename_tick(list, &context, &progress)
        });
    }

    fn reveal_rename_tick(
        &self,
        list: &gtk::Widget,
        context: &RenameRevealContext,
        progress: &RevealProgress,
    ) -> ControlFlow {
        if !self.rename_reveal_is_valid(list, context) {
            return ControlFlow::Break;
        }
        let target = match self.resolve_rename_reveal_target(context) {
            RenameRevealTarget::Stop => return ControlFlow::Break,
            RenameRevealTarget::Wait => return ControlFlow::Continue,
            RenameRevealTarget::Ready(target) => target,
        };
        if progress.waits_for(&target, list) {
            return ControlFlow::Continue;
        }
        let Some(row) = target.row.clone().filter(has_allocation) else {
            self.seed_reveal_cursor(context.depth, &target);
            return ControlFlow::Continue;
        };
        // Model splices may replace GTK's scroll anchor even when the rename
        // stays in place. Reveal from the pre-refresh viewport instead.
        if let Some(value) = progress.scroll_value.get() {
            progress.scroll.vadjustment().set_value(value);
        }
        recover_listing_focus(list, &target.collection, &row, &progress.scroll);
        let allocated = reveal_rename_row(&row, &progress.scroll, target.footer.as_ref());
        if progress.scroll_value.get().is_some() {
            progress
                .scroll_value
                .set(Some(progress.scroll.vadjustment().value()));
        }
        if allocated.is_some() && progress.settled_bounds.get() == allocated {
            return ControlFlow::Break;
        }
        progress.settled_bounds.set(allocated);
        // Tick callbacks precede allocation; confirm visibility after GTK's anchor update.
        ControlFlow::Continue
    }

    /// Focusing an unallocated row gives GTK a zero-origin scroll anchor, so
    /// let GTK seed the cursor until the replacement row is allocated.
    fn seed_reveal_cursor(&self, depth: usize, target: &RevealRow) {
        let flags = if self.mode_views.borrow().listing_may_take_focus(depth) {
            gtk::ListScrollFlags::FOCUS
        } else {
            gtk::ListScrollFlags::NONE
        };
        crate::ui::browser::collection::apply_collection_scroll(
            &target.collection,
            target.position,
            flags,
        );
    }
}
