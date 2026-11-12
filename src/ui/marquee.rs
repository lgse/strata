// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;

mod geometry;
mod hit_test;
mod origin;
mod state;
mod surface_click;

use geometry::translate;
pub(super) use hit_test::{item_bounds_predicate, item_content_predicate};
#[cfg(test)]
use origin::is_inert_chrome;
pub(super) use origin::{OriginClick, install_shared_origin_surface};
use state::DragMotion;

thread_local! {
    static UPDATING_SELECTION: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn is_updating_selection() -> bool {
    UPDATING_SELECTION.get()
}

fn with_selection_update(update: impl FnOnce()) {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            UPDATING_SELECTION.set(self.0);
        }
    }
    // GTK selection signals and their browser observers run synchronously on
    // this thread. Keep their origin through nested grouped-selection updates.
    let _restore = Restore(UPDATING_SELECTION.replace(true));
    update();
}

const AUTO_SCROLL_INTERVAL: Duration = Duration::from_millis(16);

/// Visits every bound item of a collection view as `(position, widget)`, dropping
/// entries whose widgets have been recycled.
pub(super) type ItemVisitor = Rc<dyn Fn(&mut dyn FnMut(u32, &gtk::Widget))>;
pub(super) type ItemPredicate = Rc<dyn Fn(&gtk::Widget, f64, f64) -> bool>;

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
    /// Includes the viewport's unused area and, in Columns, its empty-state surface.
    pub surface: gtk::Widget,
    pub scroll: gtk::ScrolledWindow,
    pub overlay: gtk::Overlay,
    pub targets: MarqueeTargets,
    pub is_item: ItemPredicate,
    pub clear_selection: Rc<dyn Fn()>,
    pub allow_drag: Rc<Cell<bool>>,
}

#[derive(Clone)]
pub(super) struct Marquee {
    state: Rc<MarqueeState>,
    gestures: Rc<RefCell<Vec<gtk::GestureDrag>>>,
}

struct MarqueeState {
    // Weak: the drag gesture lives on the surface, and capturing these widgets
    // would pin the collection (and its model) after a mode switch.
    view: glib::WeakRef<gtk::Widget>,
    scroll: glib::WeakRef<gtk::ScrolledWindow>,
    overlay: glib::WeakRef<gtk::Overlay>,
    band: gtk::Box,
    targets: MarqueeTargets,
    is_item: ItemPredicate,
    active: Cell<bool>,
    dragging: Cell<bool>,
    clear_on_click: Cell<bool>,
    clear_selection: Rc<dyn Fn()>,
    allow_drag: Rc<Cell<bool>>,
    /// Anchor in scroll-content coordinates. Native GtkScrollable views move their
    /// rows internally, whereas GtkViewport moves its child; neither may move the anchor.
    anchor: Cell<(f64, f64)>,
    /// Last pointer position in the scrolled window's coordinates, which do not
    /// move while the content scrolls.
    pointer: Cell<(f64, f64)>,
    // Surface coordinates distinguish pointer motion from a column moving underneath it.
    press_position: Cell<Option<(f64, f64)>>,
    /// Selection of every target as the drag began, one entry per target.
    initial: RefCell<Vec<gtk::Bitset>>,
    /// Keep geometry after virtualization unbinds a row so extending or retracting
    /// the band still resolves against items scrolled out of the viewport.
    item_bounds: RefCell<Vec<BTreeMap<u32, graphene::Rect>>>,
    modifiers: Cell<(bool, bool)>,
    auto_scroll: RefCell<Option<glib::SourceId>>,
    frame_handler: RefCell<Option<(glib::WeakRef<gtk::gdk::FrameClock>, glib::SignalHandlerId)>>,
    refresh_pending: Cell<bool>,
    finishing: Cell<bool>,
}

impl MarqueeState {
    fn new(setup: MarqueeSetup, band: gtk::Box) -> Rc<Self> {
        Rc::new(Self {
            view: setup.view.downgrade(),
            scroll: setup.scroll.downgrade(),
            overlay: setup.overlay.downgrade(),
            band,
            targets: setup.targets,
            is_item: setup.is_item,
            active: Cell::new(false),
            dragging: Cell::new(false),
            clear_on_click: Cell::new(false),
            clear_selection: setup.clear_selection,
            allow_drag: setup.allow_drag,
            anchor: Cell::new((0.0, 0.0)),
            pointer: Cell::new((0.0, 0.0)),
            press_position: Cell::new(None),
            initial: RefCell::new(Vec::new()),
            item_bounds: RefCell::new(Vec::new()),
            modifiers: Cell::new((false, false)),
            auto_scroll: RefCell::new(None),
            frame_handler: RefCell::new(None),
            refresh_pending: Cell::new(false),
            finishing: Cell::new(false),
        })
    }
}

fn band_widget(overlay: &gtk::Overlay) -> gtk::Box {
    let band = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    band.add_css_class("file-marquee");
    band.set_can_target(false);
    band.set_halign(gtk::Align::Start);
    band.set_valign(gtk::Align::Start);
    band.set_visible(false);
    overlay.add_overlay(&band);
    band
}

/// Installs marquee selection on `setup.surface` and returns a handle that can grant
/// the same drag to surrounding chrome via [`Marquee::add_origin_surface`].
pub(super) fn install(setup: MarqueeSetup) -> Marquee {
    let band = band_widget(&setup.overlay);
    let surface = setup.surface.clone();
    let scroll = setup.scroll.clone();
    let state = MarqueeState::new(setup, band);
    follow_scroll(&scroll, &state);

    let gesture = gtk::GestureDrag::new();
    gesture.set_button(1);
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    let state_for_begin = state.clone();
    gesture.connect_drag_begin(move |gesture, x, y| {
        begin_surface_drag(&state_for_begin, gesture, x, y);
    });
    connect_drag_progress(&gesture, &state);
    surface.add_controller(gesture.clone());
    surface_click::install(&surface, &gesture, &state);

    Marquee {
        state,
        gestures: Rc::new(RefCell::new(vec![gesture])),
    }
}

/// Scrolling under a drag re-resolves the band; unmapping the view ends it.
fn follow_scroll(scroll: &gtk::ScrolledWindow, state: &Rc<MarqueeState>) {
    for adjustment in [scroll.hadjustment(), scroll.vadjustment()] {
        let weak = Rc::downgrade(state);
        adjustment.connect_value_changed(move |_| {
            if let Some(state) = weak.upgrade() {
                state.queue_refresh();
            }
        });
    }
    let weak = Rc::downgrade(state);
    scroll.connect_unmap(move |_| {
        if let Some(state) = weak.upgrade() {
            state.end();
        }
    });
}

struct SurfaceOrigin {
    anchor: (f64, f64),
    clearable: bool,
    item: Option<hit_test::ItemHit>,
}

fn surface_origin(
    state: &MarqueeState,
    gesture: &gtk::Gesture,
    (x, y): (f64, f64),
) -> Option<SurfaceOrigin> {
    let origin = gesture.widget()?;
    // Overlay scrollbars occupy otherwise blank row space.
    if super::pointer::hits_scrollbar(&origin, x, y) {
        return None;
    }
    let starts_on_item = (state.is_item)(&origin, x, y);
    let force = gesture
        .current_event_state()
        .contains(gtk::gdk::ModifierType::ALT_MASK);
    if starts_on_item && !force {
        return None;
    }
    let anchor = translate(&origin, &state.scroll()?, (x, y))?;
    Some(SurfaceOrigin {
        anchor,
        clearable: !starts_on_item && super::pointer::is_background(&origin, x, y),
        item: hit_test::item_at(&state.targets, &origin, (x, y)),
    })
}

fn begin_surface_drag(state: &Rc<MarqueeState>, gesture: &gtk::GestureDrag, x: f64, y: f64) {
    let Some(origin) = surface_origin(state, gesture.upcast_ref(), (x, y)) else {
        state.end();
        return;
    };
    state.begin(origin.anchor, gesture, origin.clearable);
    state.clear_at_press();
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
        gesture.connect_drag_begin(move |gesture, x, y| {
            origin::begin_origin_drag(&state_for_begin, gesture, x, y);
        });
        connect_drag_progress(&gesture, &self.state);
        surface.add_controller(gesture.clone());
        self.gestures.borrow_mut().push(gesture);
    }

    pub(super) fn group_background_click(&self, click: &gtk::GestureClick) {
        for drag in self.gestures.borrow().iter() {
            if drag.widget() == click.widget() {
                // Claiming a marquee press must not deny a click before it can be released.
                drag.group_with(click);
            }
        }
    }
}

/// Wires update/end handling for a drag whose coordinates are expressed in
/// the gesture widget's space.
fn connect_drag_progress(gesture: &gtk::GestureDrag, state: &Rc<MarqueeState>) {
    let state_for_update = state.clone();
    gesture.connect_drag_update(move |gesture, offset_x, offset_y| {
        let Some(motion) = DragMotion::of(gesture, (offset_x, offset_y)) else {
            return;
        };
        let Some(origin) = gesture.widget() else {
            return;
        };
        if state_for_update.update_drag(&origin, &motion) {
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    let state_for_end = state.clone();
    gesture.connect_drag_end(move |_, _, _| state_for_end.finish());
    let state_for_cancel = state.clone();
    gesture.connect_cancel(move |_, _| state_for_cancel.end());
}

#[cfg(test)]
mod tests;
