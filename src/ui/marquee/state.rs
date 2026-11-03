// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::rc::Rc;

use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;

use super::geometry::{
    Band, BandBox, advance, auto_scroll_step, band_placement, content_point, translate,
};
use super::{AUTO_SCROLL_INTERVAL, MarqueeState, MarqueeTarget, with_selection_update};

/// A drag update expressed in the gesture widget's coordinates.
pub(super) struct DragMotion {
    start: (f64, f64),
    offset: (f64, f64),
    event_position: Option<(f64, f64)>,
}

impl DragMotion {
    /// `None` until the gesture has a start point.
    pub(super) fn of(gesture: &gtk::GestureDrag, offset: (f64, f64)) -> Option<Self> {
        let start = gesture.start_point()?;
        Some(Self {
            start,
            offset,
            event_position: gesture.current_event().and_then(|event| event.position()),
        })
    }

    fn current(&self) -> (f64, f64) {
        (self.start.0 + self.offset.0, self.start.1 + self.offset.1)
    }
}

/// The item nearest the pointer, preferring rows fully inside the viewport.
struct NearestItem {
    clipped: bool,
    distance: f64,
    widget: gtk::Widget,
}

impl NearestItem {
    fn beats(&self, other: Option<&Self>) -> bool {
        other.is_none_or(|old| {
            (!self.clipped && old.clipped)
                || (self.clipped == old.clipped && self.distance < old.distance)
        })
    }
}

impl MarqueeState {
    pub(super) fn view(&self) -> Option<gtk::Widget> {
        self.view.upgrade()
    }

    pub(super) fn scroll(&self) -> Option<gtk::ScrolledWindow> {
        self.scroll.upgrade()
    }

    fn overlay(&self) -> Option<gtk::Overlay> {
        self.overlay.upgrade()
    }

    pub(super) fn begin(
        self: &Rc<Self>,
        anchor: (f64, f64),
        gesture: &gtk::GestureDrag,
        clear_on_click: bool,
    ) {
        self.end();
        let modifiers = gesture.current_event_state();
        self.press_position
            .set(gesture.current_event().and_then(|event| event.position()));
        let Some(scroll) = self.scroll() else {
            return;
        };
        self.watch_frames(&scroll);
        self.active.set(true);
        self.dragging.set(false);
        self.clear_on_click.set(clear_on_click);
        self.anchor.set(content_point(&scroll, anchor));
        self.item_bounds.borrow_mut().clear();
        self.pointer.set(anchor);
        self.initial.replace(
            self.targets
                .borrow()
                .iter()
                .map(|target| target.selection.selection().copy())
                .collect(),
        );
        self.modifiers.set((
            modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK),
            modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK),
        ));
    }

    fn watch_frames(self: &Rc<Self>, scroll: &gtk::ScrolledWindow) {
        let Some(clock) = scroll.frame_clock() else {
            return;
        };
        let weak = Rc::downgrade(self);
        let handler = clock.connect_after_paint(move |_| {
            if let Some(state) = weak.upgrade() {
                state.after_paint();
            }
        });
        self.frame_handler
            .replace(Some((clock.downgrade(), handler)));
    }

    fn after_paint(&self) {
        if !self.active.get() {
            return;
        }
        if !self.refresh_pending.replace(false) {
            return;
        }
        self.refresh();
        if self.finishing.get() {
            self.end();
        }
    }

    pub(super) fn update_drag(self: &Rc<Self>, origin: &gtk::Widget, motion: &DragMotion) -> bool {
        if !self.active.get() {
            return false;
        }
        if !self.dragging.get() {
            if !self.threshold_crossed(origin, motion) {
                return false;
            }
            self.start_drag();
        }
        self.drag_to(origin, motion.current());
        true
    }

    fn threshold_crossed(&self, origin: &gtk::Widget, motion: &DragMotion) -> bool {
        if !self.allow_drag.get() {
            return false;
        }
        let (press, current) = self
            .press_position
            .get()
            .zip(motion.event_position)
            .unwrap_or(((0.0, 0.0), motion.offset));
        super::super::pointer::exceeds_drag_threshold(
            press,
            current,
            origin.settings().gtk_dnd_drag_threshold(),
        )
    }

    fn start_drag(&self) {
        if !self.active.get() || self.dragging.replace(true) {
            return;
        }
        let Some(view) = self.view() else {
            return;
        };
        if let Some(window) = view.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(false);
        }
        if view_has_focus(&view) {
            return;
        }
        // Focusing an old off-screen cursor can scroll away from the marquee anchor.
        if self.focus_nearest_item() {
            return;
        }
        if !view.grab_focus() {
            view.child_focus(gtk::DirectionType::TabForward);
        }
    }

    fn focus_nearest_item(&self) -> bool {
        let Some(item) = self.nearest_visible_item() else {
            return false;
        };
        item.parent().unwrap_or(item).grab_focus()
    }

    fn nearest_visible_item(&self) -> Option<gtk::Widget> {
        let scroll = self.scroll()?;
        let viewport = (f64::from(scroll.width()), f64::from(scroll.height()));
        let pointer = self.pointer.get();
        let mut nearest: Option<NearestItem> = None;
        for target in self.targets.borrow().iter() {
            (target.visit_items)(&mut |position, widget| {
                if position >= target.selection.n_items() || !widget.is_mapped() {
                    return;
                }
                let Some(candidate) = visible_item(widget, &scroll, viewport, pointer) else {
                    return;
                };
                if candidate.beats(nearest.as_ref()) {
                    nearest = Some(candidate);
                }
            });
        }
        nearest.map(|item| item.widget)
    }

    /// A plain press on clearable background deselects immediately; Ctrl/Shift presses
    /// keep the selection until release so marquee can still union or range from it.
    pub(super) fn clear_at_press(&self) {
        if self.modifiers.get() == (false, false) && self.clear_on_click.replace(false) {
            (self.clear_selection)();
        }
    }

    pub(super) fn finish(&self) {
        let settle_after_paint =
            self.active.get() && self.dragging.get() && self.refresh_pending.get();
        if settle_after_paint {
            self.stop_auto_scroll();
            self.finishing.set(true);
            self.queue_refresh();
            return;
        }
        let clear = self.active.get()
            && !self.dragging.get()
            && self.clear_on_click.get()
            && self.modifiers.get() == (false, false);
        self.end();
        if clear {
            (self.clear_selection)();
        }
    }

    pub(super) fn end(&self) {
        self.active.set(false);
        self.finishing.set(false);
        self.refresh_pending.set(false);
        self.stop_auto_scroll();
        let handler = self.frame_handler.borrow_mut().take();
        if let Some((clock, handler)) = handler
            && let Some(clock) = clock.upgrade()
        {
            clock.disconnect(handler);
        }
        self.band.set_visible(false);
        self.item_bounds.borrow_mut().clear();
        self.initial.borrow_mut().clear();
    }

    pub(super) fn queue_refresh(&self) {
        if !self.active.get() || !self.dragging.get() {
            return;
        }
        self.refresh_pending.set(true);
        let clock = self
            .frame_handler
            .borrow()
            .as_ref()
            .and_then(|(clock, _)| clock.upgrade());
        if let Some(clock) = clock {
            // Adjustment changes precede allocation of recycled rows. Hit-test only
            // after GTK has laid out and painted this frame, even with a stationary mouse.
            if let Some(scroll) = self.scroll() {
                scroll.queue_draw();
            }
            clock.request_phase(gtk::gdk::FrameClockPhase::AFTER_PAINT);
        } else {
            self.refresh_pending.set(false);
            self.refresh();
            if self.finishing.get() {
                self.end();
            }
        }
    }

    fn refresh(&self) {
        let Some(scroll) = self.scroll() else {
            return;
        };
        let current = content_point(&scroll, self.pointer.get());
        let band = Band::between(self.anchor.get(), current);
        self.place_band(&scroll, band);
        self.apply_selection(&scroll, band);
    }

    fn place_band(&self, scroll: &gtk::ScrolledWindow, band: Band) {
        let Some(overlay) = self.overlay() else {
            return;
        };
        let Some(viewport) = scroll.compute_bounds(&overlay) else {
            return;
        };
        let x = scroll.hadjustment().value();
        let y = scroll.vadjustment().value();
        let left = (band.left - x).max(0.0);
        let top = (band.top - y).max(0.0);
        let right = (band.right - x).min(f64::from(scroll.width()));
        let bottom = (band.bottom - y).min(f64::from(scroll.height()));
        let placement = band_placement(
            BandBox {
                x: f64::from(viewport.x()) + left,
                y: f64::from(viewport.y()) + top,
                width: right - left,
                height: bottom - top,
            },
            (f64::from(overlay.width()), f64::from(overlay.height())),
        );
        let Some((x, y, width, height)) = placement else {
            self.band.set_visible(false);
            return;
        };
        self.band.set_visible(true);
        self.band.set_margin_start(x);
        self.band.set_margin_top(y);
        self.band.set_size_request(width, height);
    }

    fn apply_selection(&self, scroll: &gtk::ScrolledWindow, band: Band) {
        let initials = self.initial.borrow();
        let modifiers = self.modifiers.get();
        let empty = gtk::Bitset::new_empty();
        let targets = self.targets.borrow();
        let mut all_bounds = self.item_bounds.borrow_mut();
        all_bounds.resize_with(targets.len(), BTreeMap::new);
        let mut changes = Vec::with_capacity(targets.len());
        for (index, target) in targets.iter().enumerate() {
            let bounds = &mut all_bounds[index];
            record_item_bounds(target, scroll, bounds);
            let initial = initials.get(index).unwrap_or(&empty);
            let selected = band_selection(target, bounds, band, initial, modifiers);
            let mask = gtk::Bitset::new_range(0, target.selection.n_items());
            changes.push((target.selection.clone(), selected, mask));
        }
        drop(all_bounds);
        drop(targets);
        drop(initials);
        with_selection_update(|| {
            for (selection, selected, mask) in changes {
                selection.set_selection(&selected, &mask);
            }
        });
    }

    fn stop_auto_scroll(&self) {
        if let Some(source) = self.auto_scroll.borrow_mut().take() {
            source.remove();
        }
    }

    /// Applies one frame of edge scrolling, reporting whether the timer should keep
    /// running.
    fn auto_scroll_frame(&self) -> bool {
        if !self.active.get() {
            return false;
        }
        let Some(scroll) = self.scroll() else {
            return false;
        };
        let (step_x, step_y) = self.auto_scroll_steps_for(&scroll);
        if step_x == 0.0 && step_y == 0.0 {
            return false;
        }
        if advance(&scroll.hadjustment(), step_x) | advance(&scroll.vadjustment(), step_y) {
            self.queue_refresh();
        }
        true
    }

    fn auto_scroll_steps(&self) -> (f64, f64) {
        self.scroll()
            .map(|scroll| self.auto_scroll_steps_for(&scroll))
            .unwrap_or((0.0, 0.0))
    }

    fn auto_scroll_steps_for(&self, scroll: &gtk::ScrolledWindow) -> (f64, f64) {
        let (x, y) = self.pointer.get();
        (
            self.scrollable_step(&scroll.hadjustment(), x, f64::from(scroll.width())),
            self.scrollable_step(&scroll.vadjustment(), y, f64::from(scroll.height())),
        )
    }

    /// An axis the view cannot scroll never contributes a step, so a drag that hangs
    /// far off a fixed edge does not keep the auto-scroll timer alive.
    fn scrollable_step(&self, adjustment: &gtk::Adjustment, position: f64, size: f64) -> f64 {
        if adjustment.upper() - adjustment.lower() <= adjustment.page_size() {
            return 0.0;
        }
        auto_scroll_step(position, size)
    }

    /// Records a pointer position given in `origin`'s coordinates and refreshes the
    /// band, the selection, and edge auto-scrolling.
    fn drag_to(self: &Rc<Self>, origin: &gtk::Widget, point: (f64, f64)) {
        if !self.active.get() {
            return;
        }
        let Some(scroll) = self.scroll() else {
            return;
        };
        let Some(pointer) = translate(origin, &scroll, point) else {
            return;
        };
        self.pointer.set(pointer);
        self.queue_refresh();
        if self.auto_scroll_steps() == (0.0, 0.0) {
            self.stop_auto_scroll();
            return;
        }
        if self.auto_scroll.borrow().is_some() {
            return;
        }
        let state = self.clone();
        let source = glib::timeout_add_local(AUTO_SCROLL_INTERVAL, move || {
            if state.auto_scroll_frame() {
                return glib::ControlFlow::Continue;
            }
            state.auto_scroll.borrow_mut().take();
            glib::ControlFlow::Break
        });
        self.auto_scroll.replace(Some(source));
    }
}

fn view_has_focus(view: &gtk::Widget) -> bool {
    view.root()
        .and_then(|root| root.focus())
        .is_some_and(|focus| focus == *view || focus.is_ancestor(view))
}

/// The item's distance from the pointer when any of it shows in the viewport.
fn visible_item(
    widget: &gtk::Widget,
    scroll: &gtk::ScrolledWindow,
    (width, height): (f64, f64),
    (x, y): (f64, f64),
) -> Option<NearestItem> {
    let bounds = widget.compute_bounds(scroll)?;
    let left = f64::from(bounds.x());
    let top = f64::from(bounds.y());
    let right = left + f64::from(bounds.width());
    let bottom = top + f64::from(bounds.height());
    let empty = right <= left || bottom <= top;
    let outside = right <= 0.0 || left >= width || bottom <= 0.0 || top >= height;
    if empty || outside {
        return None;
    }
    let dx = (left - x).max(0.0).max(x - right);
    let dy = (top - y).max(0.0).max(y - bottom);
    Some(NearestItem {
        clipped: top < 0.0 || bottom > height,
        distance: dx * dx + dy * dy,
        widget: widget.clone(),
    })
}

/// Keeps the geometry of every mapped row in scroll-content coordinates.
fn record_item_bounds(
    target: &MarqueeTarget,
    scroll: &gtk::ScrolledWindow,
    bounds: &mut BTreeMap<u32, graphene::Rect>,
) {
    (target.visit_items)(&mut |position, widget| {
        // Unmapped virtual rows can retain allocations from an earlier scroll position.
        let Some(rect) = mapped_item_rect(target, position, widget, scroll) else {
            return;
        };
        let (x, y) = content_point(scroll, (f64::from(rect.x()), f64::from(rect.y())));
        bounds.insert(
            position,
            graphene::Rect::new(x as f32, y as f32, rect.width(), rect.height()),
        );
    });
}

fn mapped_item_rect(
    target: &MarqueeTarget,
    position: u32,
    widget: &gtk::Widget,
    scroll: &gtk::ScrolledWindow,
) -> Option<graphene::Rect> {
    if position >= target.selection.n_items() || !widget.is_mapped() {
        return None;
    }
    widget
        .compute_bounds(scroll)
        .filter(|rect| rect.width() > 0.0 && rect.height() > 0.0)
}

/// The selection the band produces from the drag's initial selection: Ctrl
/// toggles the covered items, Shift adds them, a plain drag replaces everything.
fn band_selection(
    target: &MarqueeTarget,
    bounds: &BTreeMap<u32, graphene::Rect>,
    band: Band,
    initial: &gtk::Bitset,
    (control, shift): (bool, bool),
) -> gtk::Bitset {
    let selected = if control || shift {
        initial.copy()
    } else {
        gtk::Bitset::new_empty()
    };
    for (&position, rect) in bounds.iter() {
        if position >= target.selection.n_items() || !band.intersects(rect) {
            continue;
        }
        if control && initial.contains(position) {
            selected.remove(position);
        } else {
            selected.add(position);
        }
    }
    selected
}
