// SPDX-License-Identifier: MIT

//! Where keyboard focus sits inside a mode pane, and how it returns to the
//! listing's cursor after loads, filters, and renames.

use std::{
    cell::Cell,
    rc::{Rc, Weak},
};

use gtk::{glib, prelude::*};

use super::{
    BrowserMode, ModeViews,
    filter::pane_filter_focus,
    navigation::FocusOwner,
    pane::{BoundItems, Pane, PaneSection, focus_pane_surface},
    source_index::source_position_for_view,
};
use crate::{
    app::Browser,
    model::{FileEntry, Location},
    ui::{browser::FilterFocus, focus_navigation, loading_skeleton},
};

/// The browser's focused item: its depth, source position, and entry.
type FocusedItem = (usize, usize, FileEntry);

/// The view and position a pane focuses for the browser's cursor.
struct CursorTarget {
    view: gtk::Widget,
    position: Option<u32>,
}

impl ModeViews {
    pub fn header_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.single_pane()
            .is_some_and(|pane| widget_has_focus(&pane.header, focused.as_ref()))
    }

    pub fn focus_header_from_top_item(&self, actions_only: bool) -> bool {
        let Some(pane) = self.single_pane() else {
            return false;
        };
        let Some(focused) = self.stack.root().and_then(|root| root.focus()) else {
            return false;
        };
        if !self.focus_is_at_top(pane, &focused) {
            return false;
        }
        let target = if actions_only {
            pane.header.last_child()
        } else {
            Some(pane.header.clone().upcast())
        };
        if target.is_some_and(|target| target.child_focus(gtk::DirectionType::TabForward)) {
            self.focus_before_header.replace(Some(focused.downgrade()));
            return true;
        }
        false
    }

    /// Focus is on the pane surface, on an empty listing, or on its first item.
    fn focus_is_at_top(&self, pane: &Pane, focused: &gtk::Widget) -> bool {
        if *focused == *pane.stack.upcast_ref::<gtk::Widget>() {
            return true;
        }
        let empty = pane
            .item_sections()
            .iter()
            .all(|section| section.view_model.n_items() == 0);
        if empty && widget_has_focus(&pane.stack, Some(focused)) {
            return true;
        }
        pane.item_sections()
            .into_iter()
            .find(|section| section.view_model.n_items() > 0)
            .is_some_and(|section| self.focused_item_leads(&section, focused))
    }

    /// The focused item is first, or in Icons shares the first row.
    fn focused_item_leads(&self, section: &PaneSection, focused: &gtk::Widget) -> bool {
        let Some((position, bounds)) = focused_section_item(section, focused) else {
            return false;
        };
        if position == 0 {
            return true;
        }
        self.mode == BrowserMode::Icons && shares_first_line(section, &bounds, FirstLine::Row)
    }

    /// Whether the focused item is where Left leaves the items: the first one, or in
    /// Compact anywhere in the first column, which Left cannot move out of.
    fn focused_item_at_left_edge(&self, section: &PaneSection, focused: &gtk::Widget) -> bool {
        focused_section_item(section, focused).is_some_and(|(position, bounds)| {
            position == 0
                || (self.mode == BrowserMode::Compact
                    && shares_first_line(section, &bounds, FirstLine::Column))
        })
    }

    pub fn move_header_focus(&self, direction: gtk::DirectionType) -> bool {
        let direction = match direction {
            gtk::DirectionType::Left => gtk::DirectionType::TabBackward,
            gtk::DirectionType::Right => gtk::DirectionType::TabForward,
            _ => return false,
        };
        self.single_pane()
            .is_some_and(|pane| pane.header.child_focus(direction))
    }

    pub fn focus_items_from_header(&self) -> bool {
        if self
            .focus_before_header
            .borrow_mut()
            .take()
            .and_then(|weak| weak.upgrade())
            .is_some_and(|view| view.is_mapped() && view.grab_focus())
        {
            return true;
        }
        self.focus_pane_surface()
    }

    pub fn focus_pane_surface(&self) -> bool {
        self.single_pane().is_some_and(focus_pane_surface)
    }

    /// In Icons only a section's first item is the edge, so Left at any other row
    /// start wraps to the previous row. In Compact the whole first column is.
    pub fn at_left_edge(&self) -> bool {
        if !self.mode.is_grid() {
            return true;
        }
        let Some(focused) = self.stack.root().and_then(|root| root.focus()) else {
            return true;
        };
        self.visible_panes()
            .into_iter()
            .flat_map(Pane::item_sections)
            .find(|section| focused_section_item(section, &focused).is_some())
            .is_none_or(|section| self.focused_item_at_left_edge(&section, &focused))
    }

    /// Whether the listing of `depth` holds focus, rather than its filter session.
    pub(in crate::ui) fn listing_holds_focus(&self, depth: usize) -> bool {
        self.panes_at(depth)
            .iter()
            .any(|pane| pane_holds_keyboard_focus(pane))
    }

    /// Whether a background update of the listing of `depth` may move focus: nothing in
    /// the window has it, or that listing already holds it. A focused widget that left
    /// the window, like a row a reload removed, leaves nothing focused.
    pub(in crate::ui) fn listing_may_take_focus(&self, depth: usize) -> bool {
        self.stack
            .root()
            .and_then(|root| root.focus())
            .is_none_or(|focused| focused.root().is_none())
            || self.listing_holds_focus(depth)
    }

    pub fn item_view_has_focus(&self) -> bool {
        let focused = self.stack.root().and_then(|root| root.focus());
        self.all_panes().into_iter().any(|pane| {
            pane.search.has_item_focus(focused.as_ref())
                || focused.as_ref() == Some(pane.stack.upcast_ref())
                || pane
                    .all_sections()
                    .iter()
                    .any(|section| widget_has_focus(&section.view, focused.as_ref()))
        })
    }

    pub fn focused_position(&self) -> Option<(usize, usize)> {
        let focused = self.stack.root()?.focus()?;
        for pane in self.visible_panes() {
            let sections = pane.item_sections();
            let position = sections.iter().find_map(|section| {
                section.bound_items.borrow().iter().find_map(|bound| {
                    let widget = bound.widget.upgrade()?;
                    if !shares_focus(&widget, &focused) {
                        return None;
                    }
                    source_position_for_view(
                        &pane.source_index,
                        Some(&section.view_model),
                        bound.item.upgrade()?.position(),
                    )
                })
            });
            if let Some(position) = position {
                return Some((pane.depth, position));
            }
        }
        None
    }

    pub fn item_at_left_edge(&self) -> bool {
        let Some(focused) = self.stack.root().and_then(|root| root.focus()) else {
            return false;
        };
        let panes = self.visible_panes();
        let Some(pane) = panes.first() else {
            return false;
        };
        let Some(section) = pane
            .item_sections()
            .into_iter()
            .find(|section| widget_has_focus(&section.view, Some(&focused)))
        else {
            return false;
        };
        if self.mode == BrowserMode::List {
            return true;
        }
        self.focused_item_at_left_edge(&section, &focused)
    }

    /// Filter results count as outside: they belong to the filter session, which a
    /// reload restores only the viewport for.
    pub(super) fn focus_owner(&self, pane: &Pane) -> FocusOwner {
        let focused = pane.stack.root().and_then(|root| root.focus());
        match pane_filter_focus(pane, focused.as_ref()) {
            Some(FilterFocus::Entry) => FocusOwner::FilterEntry,
            Some(FilterFocus::Results) => FocusOwner::Outside,
            None if pane_holds_keyboard_focus(pane) => FocusOwner::Items,
            None => FocusOwner::Outside,
        }
    }

    pub fn focus_visible_pane(&self, depth: usize) {
        self.focus_pane(depth, false);
    }

    /// Focuses the first row shown of a pane that has no cursor. GTK's own focus row
    /// follows its item, which a grouped listing publishing in chunks moves away
    /// from the top.
    pub(super) fn focus_first_row(&self, depth: usize) {
        self.focus_pane(depth, true);
    }

    fn focus_pane(&self, depth: usize, first_row: bool) {
        if self.rename_is_active() || !focus_navigation::browser_can_take_focus(&self.stack) {
            return;
        }
        let Some(pane) = self
            .visible_panes()
            .into_iter()
            .find(|pane| pane.depth == depth)
        else {
            return;
        };
        if loading_skeleton::surface_takes_focus(&pane.stack) {
            pane.stack.grab_focus();
            return;
        }
        let focused = self
            .browser
            .focused_item()
            .filter(|(focused_depth, _, _)| *focused_depth == depth);
        let target = focus_target(pane, focused.as_ref(), first_row);
        let generation = self.cursor_focus_generation.get();
        let had_live_focus = target
            .view
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focused| focused.is_mapped());
        if !target.view.grab_focus() {
            for pane in self.panes_at(depth) {
                pane.stack.grab_focus();
            }
            return;
        }
        // A recycled, unmapped rename row needs GTK's cursor restored, not just widget focus.
        if self.suppress_focus_scroll.replace(false) && had_live_focus {
            self.restore_cursor_when_bound(pane, &target, focused);
            return;
        }
        if let Some(position) = target.position {
            focus_collection_item(&target.view, position);
        }
        DeferredCursorFocus {
            view: target.view.downgrade(),
            stack: pane.stack.downgrade(),
            position: target.position,
            generation,
            current_generation: self.cursor_focus_generation.clone(),
        }
        .schedule();
    }

    fn restore_cursor_when_bound(
        &self,
        pane: &Pane,
        target: &CursorTarget,
        focused: Option<FocusedItem>,
    ) {
        let Some(position) = target.position else {
            return;
        };
        let Some(items) = pane
            .item_sections()
            .into_iter()
            .find(|section| section.view == target.view)
            .map(|section| section.bound_items.clone())
        else {
            return;
        };
        let Some((depth, source, entry)) = focused else {
            return;
        };
        BoundCursor {
            view: target.view.downgrade(),
            items,
            position,
            browser: Rc::downgrade(&self.browser),
            expected: (depth, source, entry.location),
        }
        .focus_when_bound();
    }
}

fn cursor_target(pane: &Pane, focused: Option<&FocusedItem>) -> CursorTarget {
    focused
        .and_then(|(_, source, _)| pane.cursor_view(*source))
        .map_or_else(
            || CursorTarget {
                view: pane.focus_view(),
                position: None,
            },
            |(view, position)| CursorTarget {
                view,
                position: Some(position),
            },
        )
}

fn focus_target(pane: &Pane, focused: Option<&FocusedItem>, first_row: bool) -> CursorTarget {
    if first_row {
        first_row_target(pane)
    } else {
        cursor_target(pane, focused)
    }
}

fn first_row_target(pane: &Pane) -> CursorTarget {
    let view = pane.focus_view();
    let position = pane
        .item_sections()
        .into_iter()
        .any(|section| section.view == view && section.view_model.n_items() > 0)
        .then_some(0);
    CursorTarget { view, position }
}

/// Re-asserts the cursor focus once pending input and layout have settled.
struct DeferredCursorFocus {
    view: glib::WeakRef<gtk::Widget>,
    stack: glib::WeakRef<gtk::Stack>,
    position: Option<u32>,
    generation: u64,
    current_generation: Rc<Cell<u64>>,
}

impl DeferredCursorFocus {
    fn schedule(self) {
        glib::idle_add_local_once(move || self.run());
    }

    fn run(self) {
        if self.current_generation.get() != self.generation {
            return;
        }
        let Some(view) = self.view.upgrade() else {
            return;
        };
        let Some(stack) = self.stack.upgrade() else {
            return;
        };
        if loading_skeleton::surface_takes_focus(&stack) {
            return;
        }
        let focused = view.root().and_then(|root| root.focus());
        if !widget_has_focus(&view, focused.as_ref()) {
            return;
        }
        if focused.is_some_and(|focused| {
            focus_navigation::editable(&focused) || focus_navigation::in_popover(&focused)
        }) {
            return;
        }
        if let Some(position) = self.position {
            focus_collection_item(&view, position);
        } else {
            view.grab_focus();
        }
    }
}

pub(super) fn widget_has_focus(
    widget: &impl IsA<gtk::Widget>,
    focused: Option<&gtk::Widget>,
) -> bool {
    widget.has_focus()
        || focused.is_some_and(|focused| {
            focused == widget.as_ref() || focused.is_ancestor(widget.as_ref())
        })
}

/// The widget is the focused one, contains it, or sits inside it.
pub(super) fn shares_focus(widget: &gtk::Widget, focused: &gtk::Widget) -> bool {
    widget == focused || widget.is_ancestor(focused) || focused.is_ancestor(widget)
}

/// Any part of the pane holds focus, its filter session included.
pub(super) fn pane_contains_focus(pane: &Pane) -> bool {
    let focused = pane.stack.root().and_then(|root| root.focus());
    widget_has_focus(&pane.stack, focused.as_ref())
        || pane
            .item_sections()
            .iter()
            .any(|section| widget_has_focus(&section.view, focused.as_ref()))
}

/// The listing itself holds focus: the pane surface or one of its item views, not the
/// filter field or the results that replace the listing, 10xer's included. Loads and
/// live changes refocus the listing only then.
pub(super) fn pane_holds_keyboard_focus(pane: &Pane) -> bool {
    let focused = pane.stack.root().and_then(|root| root.focus());
    let filter_session = pane
        .filter_entry
        .as_ref()
        .is_some_and(|entry| widget_has_focus(entry, focused.as_ref()))
        || pane.search.has_item_focus(focused.as_ref());
    !filter_session && pane_contains_focus(pane)
}

/// Where GTK leaves focus it moved off a pane a reload hid: nowhere, on a widget that
/// left the window, on the pane surface, or on a container around the pane.
pub(super) fn reload_focus_fell_back(pane: &Pane) -> bool {
    let Some(focused) = pane.stack.root().and_then(|root| root.focus()) else {
        return true;
    };
    focused.root().is_none()
        || focused == *pane.stack.upcast_ref::<gtk::Widget>()
        || pane.stack.is_ancestor(&focused)
}

#[derive(Clone, Copy)]
enum FirstLine {
    Row,
    Column,
}

/// Whether an item lines up with the section's first item on its row or column.
fn shares_first_line(section: &PaneSection, bounds: &gtk::graphene::Rect, line: FirstLine) -> bool {
    let mut shares = false;
    (section.visit)(&mut |position, widget| {
        if position == 0
            && let Some(first) = widget.compute_bounds(&section.view)
        {
            shares = match line {
                FirstLine::Row => (first.y() - bounds.y()).abs() < 1.0,
                FirstLine::Column => (first.x() - bounds.x()).abs() < 1.0,
            };
        }
    });
    shares
}

pub(super) fn focused_section_item(
    section: &PaneSection,
    focused: &gtk::Widget,
) -> Option<(u32, gtk::graphene::Rect)> {
    section.bound_items.borrow().iter().find_map(|bound| {
        let widget = bound.widget.upgrade()?;
        let owns_focus = widget == *focused
            || focused.is_ancestor(&widget)
            || widget.parent().as_ref() == Some(focused);
        if !owns_focus {
            return None;
        }
        Some((
            bound.item.upgrade()?.position(),
            widget.compute_bounds(&section.view)?,
        ))
    })
}

pub(super) fn focus_collection_item(view: &gtk::Widget, position: u32) {
    crate::ui::browser::focus_collection_item_when_allocated(view, position);
}

/// Focuses a cursor row once the view has bound its widget.
struct BoundCursor {
    view: glib::WeakRef<gtk::Widget>,
    items: BoundItems,
    position: u32,
    browser: Weak<Browser>,
    expected: (usize, usize, Location),
}

impl BoundCursor {
    /// A later input or splice supersedes this cursor even when focus stays in the same view.
    fn is_current(&self) -> bool {
        self.browser.upgrade().is_some_and(|browser| {
            browser
                .focused_item()
                .is_some_and(|(depth, source, entry)| {
                    depth == self.expected.0
                        && source == self.expected.1
                        && entry.location == self.expected.2
                })
        })
    }

    fn focus_when_bound(self) {
        glib::idle_add_local_once(move || {
            let Some(view) = self.view.upgrade() else {
                return;
            };
            if !self.is_current() || !collection_keeps_cursor(&view) {
                return;
            }
            if focus_bound_cursor(&self.items, self.position) {
                return;
            }
            let frames = Cell::new(0u8);
            view.add_tick_callback(move |view, _| {
                if self.settled(view) || frames.get() >= 8 {
                    return glib::ControlFlow::Break;
                }
                frames.set(frames.get().saturating_add(1));
                glib::ControlFlow::Continue
            });
        });
    }

    /// The cursor landed, or something else took it over.
    fn settled(&self, view: &gtk::Widget) -> bool {
        !self.is_current()
            || !collection_keeps_cursor(view)
            || focus_bound_cursor(&self.items, self.position)
    }
}

pub(super) fn collection_keeps_cursor(view: &gtk::Widget) -> bool {
    let focused = view.root().and_then(|root| root.focus());
    if focused.as_ref().is_some_and(|focused| {
        focus_navigation::editable(focused) || focus_navigation::in_popover(focused)
    }) {
        return false;
    }
    focused.as_ref().is_none_or(|focused| {
        focused == view
            || view.is_ancestor(focused)
            || focused.is_ancestor(view)
            || focus_navigation::focus_removed_from(view, focused)
    })
}

pub(super) fn focus_bound_cursor(items: &BoundItems, position: u32) -> bool {
    let Some(widget) = items.borrow().iter().find_map(|bound| {
        let item = bound.item.upgrade()?;
        (item.position() == position)
            .then(|| bound.widget.upgrade())
            .flatten()
            .filter(|widget| widget.is_mapped())
    }) else {
        return false;
    };
    widget
        .parent()
        .map(|parent| parent.grab_focus())
        .unwrap_or_else(|| widget.grab_focus())
}
