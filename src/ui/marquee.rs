// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;

/// Distance from a viewport edge at which a marquee drag starts scrolling.
const AUTO_SCROLL_MARGIN: f64 = 28.0;
/// Largest scroll step, in pixels, applied per auto-scroll frame.
const AUTO_SCROLL_MAX_STEP: f64 = 24.0;
const AUTO_SCROLL_INTERVAL: Duration = Duration::from_millis(16);

/// Visits every bound item of a collection view as `(position, widget)`, dropping
/// entries whose widgets have been recycled.
pub(super) type ItemVisitor = Rc<dyn Fn(&mut dyn FnMut(u32, &gtk::Widget))>;
pub(super) type ItemPredicate = Rc<dyn Fn(&gtk::Widget) -> bool>;

/// One collection view a drag can select in. A grouped view contributes one target
/// per group, since each group renders through its own view and selection model.
pub(super) struct MarqueeTarget {
    pub selection: gtk::MultiSelection,
    pub visit_items: ItemVisitor,
}

/// Targets shared with the caller, so a view that rebuilds its groups can replace
/// them without reinstalling the drag.
pub(super) type MarqueeTargets = Rc<RefCell<Vec<MarqueeTarget>>>;

pub(super) struct MarqueeSetup {
    pub view: gtk::Widget,
    pub scroll: gtk::ScrolledWindow,
    pub overlay: gtk::Overlay,
    pub targets: MarqueeTargets,
    pub is_item: ItemPredicate,
}

#[derive(Clone)]
pub(super) struct Marquee {
    state: Rc<MarqueeState>,
}

struct MarqueeState {
    view: gtk::Widget,
    scroll: gtk::ScrolledWindow,
    overlay: gtk::Overlay,
    band: gtk::Box,
    targets: MarqueeTargets,
    is_item: ItemPredicate,
    active: Cell<bool>,
    /// Anchor in the view's own coordinates, so it stays glued to the content
    /// while the list scrolls underneath the pointer.
    anchor: Cell<(f64, f64)>,
    /// Last pointer position in the scrolled window's coordinates, which do not
    /// move while the content scrolls.
    pointer: Cell<(f64, f64)>,
    /// Selection of every target as the drag began, one entry per target.
    initial: RefCell<Vec<gtk::Bitset>>,
    modifiers: Cell<(bool, bool)>,
    auto_scroll: RefCell<Option<glib::SourceId>>,
}

/// Installs marquee selection on `setup.view` and returns a handle that can grant
/// the same drag to surrounding chrome via [`Marquee::add_origin_surface`].
pub(super) fn install(setup: MarqueeSetup) -> Marquee {
    let band = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    band.add_css_class("file-marquee");
    band.set_can_target(false);
    band.set_halign(gtk::Align::Start);
    band.set_valign(gtk::Align::Start);
    band.set_visible(false);
    setup.overlay.add_overlay(&band);

    let view = setup.view;
    let state = Rc::new(MarqueeState {
        view: view.clone(),
        scroll: setup.scroll,
        overlay: setup.overlay,
        band,
        targets: setup.targets,
        is_item: setup.is_item,
        active: Cell::new(false),
        anchor: Cell::new((0.0, 0.0)),
        pointer: Cell::new((0.0, 0.0)),
        initial: RefCell::new(Vec::new()),
        modifiers: Cell::new((false, false)),
        auto_scroll: RefCell::new(None),
    });

    let gesture = gtk::GestureDrag::new();
    gesture.set_button(1);
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    let state_for_begin = state.clone();
    gesture.connect_drag_begin(move |gesture, x, y| {
        let starts_on_item = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
            .is_some_and(|widget| (state_for_begin.is_item)(&widget));
        let force = gesture
            .current_event_state()
            .contains(gtk::gdk::ModifierType::ALT_MASK);
        if !force && starts_on_item {
            state_for_begin.active.set(false);
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
        state_for_begin.begin((x, y), gesture.current_event_state());
    });
    connect_drag_progress(&gesture, &state, &view);
    view.add_controller(gesture);

    Marquee { state }
}

impl Marquee {
    pub(super) fn band(&self) -> gtk::Box {
        self.state.band.clone()
    }

    /// Lets a marquee drag begin on `surface` — chrome beside the collection view,
    /// such as a pane header or a column-heading strip — and carry into the view.
    /// Only presses landing on the surface itself qualify, so the controls it hosts
    /// keep their own drags.
    pub(super) fn add_origin_surface(&self, surface: &impl IsA<gtk::Widget>) {
        let surface = surface.clone().upcast::<gtk::Widget>();
        let gesture = gtk::GestureDrag::new();
        gesture.set_button(1);
        let state_for_begin = self.state.clone();
        let surface_for_begin = surface.clone();
        gesture.connect_drag_begin(move |gesture, x, y| {
            state_for_begin.active.set(false);
            let accepted = surface_for_begin
                .pick(x, y, gtk::PickFlags::DEFAULT)
                .is_some_and(|picked| is_inert_chrome(&surface_for_begin, &picked));
            if !accepted || !state_for_begin.view.is_mapped() {
                return;
            }
            let Some(anchor) = translate(&surface_for_begin, &state_for_begin.view, (x, y)) else {
                return;
            };
            gesture.set_state(gtk::EventSequenceState::Claimed);
            state_for_begin.begin(anchor, gesture.current_event_state());
        });
        connect_drag_progress(&gesture, &self.state, &surface);
        surface.add_controller(gesture);
    }
}

/// Chrome such as a pane header can begin a marquee drag, but only where the press
/// lands on the container itself rather than on a button, entry, or other control.
fn is_inert_chrome(surface: &gtk::Widget, picked: &gtk::Widget) -> bool {
    let mut current = Some(picked.clone());
    while let Some(widget) = current {
        if widget.eq(surface) {
            return true;
        }
        if widget.is::<gtk::Button>()
            || widget.is::<gtk::Editable>()
            || widget.is::<gtk::Range>()
            || widget.is::<gtk::Scrollbar>()
        {
            return false;
        }
        current = widget.parent();
    }
    false
}

/// Lets one long-lived surface — the blank area beside the last open column, or the
/// sidebar — begin a marquee drag on whichever collection view `resolve` picks for
/// the press position. Unlike [`Marquee::add_origin_surface`] the surface outlives
/// the views it feeds, so the target is resolved per drag. Presses landing on a
/// control the surface hosts are filtered out before `resolve` is consulted.
pub(super) fn install_shared_origin_surface(
    surface: &impl IsA<gtk::Widget>,
    resolve: impl Fn(&gtk::Widget, &gtk::Widget, f64, f64) -> Option<Marquee> + 'static,
) {
    let surface = surface.clone().upcast::<gtk::Widget>();
    let target: Rc<RefCell<Option<Rc<MarqueeState>>>> = Rc::new(RefCell::new(None));
    let gesture = gtk::GestureDrag::new();
    gesture.set_button(1);
    let target_for_begin = target.clone();
    let surface_for_begin = surface.clone();
    gesture.connect_drag_begin(move |gesture, x, y| {
        target_for_begin.replace(None);
        let Some(picked) = surface_for_begin.pick(x, y, gtk::PickFlags::DEFAULT) else {
            return;
        };
        if !is_inert_chrome(&surface_for_begin, &picked) {
            return;
        }
        let Some(marquee) = resolve(&surface_for_begin, &picked, x, y) else {
            return;
        };
        let state = marquee.state;
        if !state.view.is_mapped() {
            return;
        }
        let Some(anchor) = translate(&surface_for_begin, &state.view, (x, y)) else {
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        state.begin(anchor, gesture.current_event_state());
        target_for_begin.replace(Some(state));
    });
    let target_for_update = target.clone();
    let surface_for_update = surface.clone();
    gesture.connect_drag_update(move |gesture, offset_x, offset_y| {
        let Some((start_x, start_y)) = gesture.start_point() else {
            return;
        };
        if let Some(state) = target_for_update.borrow().as_ref() {
            state.drag_to(
                &surface_for_update,
                (start_x + offset_x, start_y + offset_y),
            );
        }
    });
    gesture.connect_drag_end(move |_, _, _| {
        if let Some(state) = target.borrow_mut().take() {
            state.end();
        }
    });
    surface.add_controller(gesture);
}

/// Wires update/end handling for a drag whose coordinates are expressed in
/// `origin`'s space.
fn connect_drag_progress(
    gesture: &gtk::GestureDrag,
    state: &Rc<MarqueeState>,
    origin: &gtk::Widget,
) {
    let state_for_update = state.clone();
    let origin_for_update = origin.clone();
    gesture.connect_drag_update(move |gesture, offset_x, offset_y| {
        let Some((start_x, start_y)) = gesture.start_point() else {
            return;
        };
        state_for_update.drag_to(&origin_for_update, (start_x + offset_x, start_y + offset_y));
    });
    let state_for_end = state.clone();
    gesture.connect_drag_end(move |_, _, _| state_for_end.end());
}

impl MarqueeState {
    fn begin(&self, anchor: (f64, f64), modifiers: gtk::gdk::ModifierType) {
        self.active.set(true);
        self.anchor.set(anchor);
        self.pointer
            .set(translate(&self.view, &self.scroll, anchor).unwrap_or_default());
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

    fn end(&self) {
        self.active.set(false);
        self.stop_auto_scroll();
        self.band.set_visible(false);
    }

    fn refresh(&self) {
        let Some((current_x, current_y)) = translate(&self.scroll, &self.view, self.pointer.get())
        else {
            return;
        };
        let (anchor_x, anchor_y) = self.anchor.get();
        let left = anchor_x.min(current_x);
        let right = anchor_x.max(current_x);
        let top = anchor_y.min(current_y);
        let bottom = anchor_y.max(current_y);
        self.place_band(left, top, right, bottom);
        self.apply_selection(left, top, right, bottom);
    }

    fn place_band(&self, left: f64, top: f64, right: f64, bottom: f64) {
        let Some(view_bounds) = self.view.compute_bounds(&self.overlay) else {
            return;
        };
        let placement = band_placement(
            f64::from(view_bounds.x()) + left,
            f64::from(view_bounds.y()) + top,
            right - left,
            bottom - top,
            f64::from(self.overlay.width()),
            f64::from(self.overlay.height()),
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

    fn apply_selection(&self, left: f64, top: f64, right: f64, bottom: f64) {
        let initials = self.initial.borrow();
        let (control, shift) = self.modifiers.get();
        let view = self.view.clone();
        let empty = gtk::Bitset::new_empty();
        for (index, target) in self.targets.borrow().iter().enumerate() {
            let initial = initials.get(index).unwrap_or(&empty);
            let selected = if control || shift {
                initial.copy()
            } else {
                gtk::Bitset::new_empty()
            };
            (target.visit_items)(&mut |position, widget| {
                if position == gtk::INVALID_LIST_POSITION {
                    return;
                }
                let Some(bounds) = widget.compute_bounds(&view) else {
                    return;
                };
                if !intersects(&bounds, left, top, right, bottom) {
                    return;
                }
                if control && initial.contains(position) {
                    selected.remove(position);
                } else {
                    selected.add(position);
                }
            });
            let mask = gtk::Bitset::new_range(0, target.selection.n_items());
            target.selection.set_selection(&selected, &mask);
        }
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
        let (step_x, step_y) = self.auto_scroll_steps();
        if step_x == 0.0 && step_y == 0.0 {
            return false;
        }
        if advance(&self.scroll.hadjustment(), step_x) | advance(&self.scroll.vadjustment(), step_y)
        {
            self.refresh();
        }
        true
    }

    fn auto_scroll_steps(&self) -> (f64, f64) {
        let (x, y) = self.pointer.get();
        (
            self.scrollable_step(
                &self.scroll.hadjustment(),
                x,
                f64::from(self.scroll.width()),
            ),
            self.scrollable_step(
                &self.scroll.vadjustment(),
                y,
                f64::from(self.scroll.height()),
            ),
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
        let Some(pointer) = translate(origin, &self.scroll, point) else {
            return;
        };
        self.pointer.set(pointer);
        self.refresh();
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

fn advance(adjustment: &gtk::Adjustment, step: f64) -> bool {
    if step == 0.0 {
        return false;
    }
    let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    let target = (adjustment.value() + step).clamp(adjustment.lower(), upper);
    if (target - adjustment.value()).abs() < f64::EPSILON {
        return false;
    }
    adjustment.set_value(target);
    true
}

fn translate(
    from: &impl IsA<gtk::Widget>,
    to: &impl IsA<gtk::Widget>,
    point: (f64, f64),
) -> Option<(f64, f64)> {
    let point = graphene::Point::new(point.0 as f32, point.1 as f32);
    from.as_ref()
        .compute_point(to, &point)
        .map(|point| (f64::from(point.x()), f64::from(point.y())))
}

/// A band with zero area still resolves against the item under it, so a press
/// that has not moved yet behaves like a click.
fn intersects(bounds: &graphene::Rect, left: f64, top: f64, right: f64, bottom: f64) -> bool {
    f64::from(bounds.x()) < right
        && f64::from(bounds.x() + bounds.width()) > left
        && f64::from(bounds.y()) < bottom
        && f64::from(bounds.y() + bounds.height()) > top
}

/// Scroll step for a pointer coordinate inside a viewport `size` pixels long.
/// Negative values scroll towards the start, positive towards the end.
fn auto_scroll_step(position: f64, size: f64) -> f64 {
    if size <= AUTO_SCROLL_MARGIN * 2.0 {
        return 0.0;
    }
    let overshoot = if position < AUTO_SCROLL_MARGIN {
        position - AUTO_SCROLL_MARGIN
    } else if position > size - AUTO_SCROLL_MARGIN {
        position - (size - AUTO_SCROLL_MARGIN)
    } else {
        return 0.0;
    };
    (overshoot / AUTO_SCROLL_MARGIN).clamp(-1.0, 1.0) * AUTO_SCROLL_MAX_STEP
}

/// Clips a band rectangle to the overlay it is drawn in, returning `None` when
/// none of it remains visible.
fn band_placement(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    overlay_width: f64,
    overlay_height: f64,
) -> Option<(i32, i32, i32, i32)> {
    let left = x.max(0.0);
    let top = y.max(0.0);
    let right = (x + width).min(overlay_width);
    let bottom = (y + height).min(overlay_height);
    if right < left || bottom < top {
        return None;
    }
    Some((
        left.round() as i32,
        top.round() as i32,
        (right - left).round().max(1.0) as i32,
        (bottom - top).round().max(1.0) as i32,
    ))
}

#[cfg(test)]
mod tests;
