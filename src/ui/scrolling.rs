// SPDX-License-Identifier: GPL-3.0-or-later

//! Fast scrolling shared by the browser's collection views: middle-click
//! autoscroll and the geometry behind page-sized keyboard navigation.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;

/// Pointer travel from the anchor that is treated as "not moving yet".
const DEAD_ZONE: f64 = 12.0;
/// Travel beyond the dead zone at which the fastest scroll is reached.
const FULL_SPEED_DISTANCE: f64 = 220.0;
/// Largest scroll step, in pixels, applied per autoscroll frame.
const MAX_STEP: f64 = 32.0;
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

thread_local! {
    /// At most one autoscroll runs at a time, so any click or `Escape` can end it
    /// without knowing which view started it.
    static ACTIVE: RefCell<Option<Rc<AutoScroll>>> = const { RefCell::new(None) };
}

struct AutoScroll {
    scroll: gtk::ScrolledWindow,
    overlay: gtk::Overlay,
    marker: gtk::Box,
    /// Anchor and pointer in the scrolled window's coordinates, which do not move
    /// while the content scrolls underneath.
    anchor: Cell<(f64, f64)>,
    pointer: Cell<(f64, f64)>,
    frames: RefCell<Option<glib::SourceId>>,
}

/// Installs middle-click autoscroll on a file view's scrolling area. Movement away
/// from the press point sets direction and speed; another click or `Escape` stops.
pub(super) fn install_autoscroll(scroll: &gtk::ScrolledWindow, overlay: &gtk::Overlay) {
    let marker = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    marker.add_css_class("autoscroll-anchor");
    marker.set_can_target(false);
    marker.set_halign(gtk::Align::Start);
    marker.set_valign(gtk::Align::Start);
    marker.set_visible(false);
    overlay.add_overlay(&marker);

    let state = Rc::new(AutoScroll {
        scroll: scroll.clone(),
        overlay: overlay.clone(),
        marker,
        anchor: Cell::new((0.0, 0.0)),
        pointer: Cell::new((0.0, 0.0)),
        frames: RefCell::new(None),
    });

    let press = gtk::GestureClick::new();
    press.set_button(gtk::gdk::BUTTON_MIDDLE);
    press.set_propagation_phase(gtk::PropagationPhase::Capture);
    let state_for_press = state.clone();
    press.connect_pressed(move |gesture, _, x, y| {
        if stop_autoscroll() {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            return;
        }
        let over_text = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
            .is_some_and(|widget| is_text_target(&widget));
        if over_text || !state_for_press.start((x, y)) {
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    scroll.add_controller(press);

    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    let state_for_motion = state.clone();
    motion.connect_motion(move |_, x, y| state_for_motion.track((x, y)));
    scroll.add_controller(motion);

    let state_for_unmap = state.clone();
    scroll.connect_unmap(move |_| {
        if state_for_unmap.is_active() {
            stop_autoscroll();
        }
    });
}

/// Lets any press anywhere under `root` end a running autoscroll, without
/// disturbing the click when none is running.
pub(super) fn install_autoscroll_stop(root: &impl IsA<gtk::Widget>) {
    let press = gtk::GestureClick::new();
    press.set_button(0);
    press.set_propagation_phase(gtk::PropagationPhase::Capture);
    press.connect_pressed(move |gesture, _, _, _| {
        if stop_autoscroll() {
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    root.as_ref().add_controller(press);
}

/// Stops a running autoscroll, reporting whether one was running.
pub(super) fn stop_autoscroll() -> bool {
    let Some(state) = ACTIVE.with_borrow_mut(std::option::Option::take) else {
        return false;
    };
    state.stop();
    true
}

impl AutoScroll {
    fn is_active(self: &Rc<Self>) -> bool {
        ACTIVE.with_borrow(|active| {
            active
                .as_ref()
                .is_some_and(|active| Rc::ptr_eq(active, self))
        })
    }

    /// Begins autoscrolling from `anchor`, reporting whether the view can scroll at
    /// all — a view that fits its viewport keeps the press for other handlers.
    fn start(self: &Rc<Self>, anchor: (f64, f64)) -> bool {
        if !scrollable(&self.scroll.hadjustment()) && !scrollable(&self.scroll.vadjustment()) {
            return false;
        }
        self.anchor.set(anchor);
        self.pointer.set(anchor);
        self.place_marker();
        self.scroll.set_cursor_from_name(Some("all-scroll"));
        let state = self.clone();
        let source = glib::timeout_add_local(FRAME_INTERVAL, move || {
            state.frame();
            glib::ControlFlow::Continue
        });
        self.frames.replace(Some(source));
        ACTIVE.with_borrow_mut(|active| *active = Some(self.clone()));
        true
    }

    fn stop(&self) {
        if let Some(source) = self.frames.borrow_mut().take() {
            source.remove();
        }
        self.marker.set_visible(false);
        self.scroll.set_cursor(None);
    }

    fn track(self: &Rc<Self>, pointer: (f64, f64)) {
        if self.is_active() {
            self.pointer.set(pointer);
        }
    }

    fn frame(&self) {
        let (anchor_x, anchor_y) = self.anchor.get();
        let (pointer_x, pointer_y) = self.pointer.get();
        advance(
            &self.scroll.hadjustment(),
            autoscroll_step(pointer_x - anchor_x),
        );
        advance(
            &self.scroll.vadjustment(),
            autoscroll_step(pointer_y - anchor_y),
        );
    }

    fn place_marker(&self) {
        let Some(bounds) = self.scroll.compute_bounds(&self.overlay) else {
            return;
        };
        let (x, y) = self.anchor.get();
        // The marker is sized by the theme, so its natural size centres it.
        let (_, size, _, _) = self.marker.measure(gtk::Orientation::Horizontal, -1);
        let offset = f64::from(size) / 2.0;
        self.marker
            .set_margin_start((f64::from(bounds.x()) + x - offset).round() as i32);
        self.marker
            .set_margin_top((f64::from(bounds.y()) + y - offset).round() as i32);
        self.marker.set_visible(true);
    }
}

fn is_text_target(widget: &gtk::Widget) -> bool {
    let mut current = Some(widget.clone());
    while let Some(widget) = current {
        if widget.is::<gtk::Editable>() || widget.is::<gtk::TextView>() {
            return true;
        }
        current = widget.parent();
    }
    false
}

fn scrollable(adjustment: &gtk::Adjustment) -> bool {
    adjustment.upper() - adjustment.lower() > adjustment.page_size()
}

fn advance(adjustment: &gtk::Adjustment, step: f64) {
    if step == 0.0 {
        return;
    }
    let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    let target = (adjustment.value() + step).clamp(adjustment.lower(), upper);
    if (target - adjustment.value()).abs() >= f64::EPSILON {
        adjustment.set_value(target);
    }
}

/// Scroll step for a pointer `delta` from the anchor. Speed grows with the square
/// of the distance so small movements stay controllable.
fn autoscroll_step(delta: f64) -> f64 {
    let travel = delta.abs() - DEAD_ZONE;
    if travel <= 0.0 {
        return 0.0;
    }
    let ratio = (travel / FULL_SPEED_DISTANCE).clamp(0.0, 1.0);
    delta.signum() * ratio * ratio * MAX_STEP
}

/// The collection view holding keyboard focus, with the scrolling area it moves in.
pub(super) fn focused_collection(
    focused: &gtk::Widget,
) -> Option<(gtk::Widget, gtk::ScrolledWindow)> {
    let mut current = Some(focused.clone());
    while let Some(widget) = current {
        if widget.is::<gtk::ListView>() || widget.is::<gtk::GridView>() {
            let scroll = widget
                .ancestor(gtk::ScrolledWindow::static_type())
                .and_downcast::<gtk::ScrolledWindow>()?;
            return Some((widget, scroll));
        }
        current = widget.parent();
    }
    None
}

/// One page move of a collection view: how many entries the focus travels and how
/// far that is on screen.
pub(super) struct Page {
    pub items: usize,
    distance: f64,
}

/// Brings the item a page move selected back into sight. A grouped view renders
/// through several collection views inside one scrolling area, none of which drives
/// the scrolling itself, so it scrolls by the distance the focus travelled instead.
pub(super) fn reveal_selection(
    view: &gtk::Widget,
    scroll: &gtk::ScrolledWindow,
    direction: i32,
    page: &Page,
) {
    if scroll.child().is_some_and(|child| &child == view)
        && let Some(position) = selected_position(view)
    {
        scroll_to_item(view, position);
        return;
    }
    advance(&scroll.vadjustment(), f64::from(direction) * page.distance);
}

/// Brings the newly selected item into sight after jumping to the first or last
/// entry, scrolling all the way to that edge of the pane.
pub(super) fn reveal_jump(view: &gtk::Widget, scroll: &gtk::ScrolledWindow, direction: i32) {
    if scroll.child().is_some_and(|child| &child == view)
        && let Some(position) = selected_position(view)
    {
        scroll_to_item(view, position);
        return;
    }
    advance(&scroll.vadjustment(), f64::from(direction) * f64::MAX);
}

fn selected_position(view: &gtk::Widget) -> Option<u32> {
    let model = match (
        view.downcast_ref::<gtk::ListView>(),
        view.downcast_ref::<gtk::GridView>(),
    ) {
        (Some(list), _) => list.model(),
        (_, Some(grid)) => grid.model(),
        _ => None,
    }?;
    let selection = model.selection();
    (!selection.is_empty()).then(|| selection.minimum())
}

fn scroll_to_item(view: &gtk::Widget, position: u32) {
    if let Some(list) = view.downcast_ref::<gtk::ListView>() {
        list.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
    } else if let Some(grid) = view.downcast_ref::<gtk::GridView>() {
        grid.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
    }
}

/// How far one `Page Up` or `Page Down` should move `view`, derived from the items
/// it currently shows.
pub(super) fn page(view: &gtk::Widget, scroll: &gtk::ScrolledWindow) -> Page {
    let page_size = scroll.vadjustment().page_size();
    let Some((item_height, columns)) = item_geometry(view) else {
        return Page {
            items: 1,
            distance: page_size,
        };
    };
    let rows = rows_per_page(page_size, item_height);
    Page {
        items: rows * columns,
        distance: rows as f64 * item_height,
    }
}

/// Height of a realized item and the number of items per row, measured from the
/// widgets the view currently has bound.
fn item_geometry(view: &gtk::Widget) -> Option<(f64, usize)> {
    let mut child = view.first_child();
    let mut first: Option<graphene::Rect> = None;
    let mut columns = 0;
    while let Some(widget) = child {
        child = widget.next_sibling();
        if !widget.is_visible() {
            continue;
        }
        let Some(bounds) = widget.compute_bounds(view) else {
            continue;
        };
        match first {
            None => {
                first = Some(bounds);
                columns = 1;
            }
            Some(first) => {
                if (bounds.y() - first.y()).abs() > f32::EPSILON {
                    break;
                }
                columns += 1;
            }
        }
    }
    let height = f64::from(first?.height());
    (height > 0.0).then_some((height, columns.max(1)))
}

/// Rows to move for one page, keeping a row of overlap so the reader retains
/// context across the jump.
fn rows_per_page(page_size: f64, item_height: f64) -> usize {
    if item_height <= 0.0 {
        return 1;
    }
    let rows = (page_size / item_height).floor().max(1.0) as usize;
    rows.saturating_sub(1).max(1)
}

#[cfg(test)]
mod tests;
