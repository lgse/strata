// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::geometry::translate;
use super::state::DragMotion;
use super::{Marquee, MarqueeState};

/// The widget under a press at (`x`, `y`) on `surface` when it is blank chrome rather
/// than a control. Insensitive widgets are picked so a disabled button is rejected
/// like an enabled one instead of exposing the box behind it.
fn inert_chrome_at(surface: &gtk::Widget, x: f64, y: f64) -> Option<gtk::Widget> {
    surface
        .pick(x, y, gtk::PickFlags::INSENSITIVE)
        .filter(|picked| is_inert_chrome(surface, picked))
}

/// Chrome such as a pane header can begin a marquee drag, but only where the press
/// lands on the container itself rather than on a button, entry, or other control.
pub(super) fn is_inert_chrome(surface: &gtk::Widget, picked: &gtk::Widget) -> bool {
    let mut current = Some(picked.clone());
    while let Some(widget) = current {
        if widget.eq(surface) {
            return true;
        }
        if is_control(&widget) {
            return false;
        }
        current = widget.parent();
    }
    false
}

fn is_control(widget: &gtk::Widget) -> bool {
    is_native_control(widget) || is_custom_control(widget) || hosts_drag_source(widget)
}

/// Item containers count as controls: the preview pane hosts lists of its own whose
/// presses must not be claimed.
fn is_native_control(widget: &gtk::Widget) -> bool {
    let kind = widget.type_();
    [
        gtk::Button::static_type(),
        gtk::Editable::static_type(),
        gtk::Range::static_type(),
        gtk::Scrollbar::static_type(),
        gtk::TextView::static_type(),
        gtk::Picture::static_type(),
        gtk::Image::static_type(),
        gtk::Label::static_type(),
        gtk::ListView::static_type(),
        gtk::GridView::static_type(),
        gtk::ColumnView::static_type(),
        gtk::ListBox::static_type(),
        gtk::FlowBox::static_type(),
    ]
    .into_iter()
    .any(|control| kind.is_a(control))
}

/// Custom-drawn controls and artwork, such as the audio preview's scrubber.
fn is_custom_control(widget: &gtk::Widget) -> bool {
    matches!(
        widget.accessible_role(),
        gtk::AccessibleRole::Slider | gtk::AccessibleRole::Img
    )
}

/// Chrome or item rows with a file drag source keep their presses for that source.
pub(super) fn hosts_drag_source(widget: &gtk::Widget) -> bool {
    widget
        .observe_controllers()
        .iter::<glib::Object>()
        .flatten()
        .any(|controller| controller.is::<gtk::DragSource>())
}

/// What a plain click on a shared marquee origin does when it never becomes a drag.
#[derive(Clone)]
pub(in crate::ui) enum OriginClick {
    /// Blank chrome beside the panes acts like pane background.
    ClearsSelection,
    /// Clicks inside this widget (a preview showing something) never clear; the rest
    /// of the surface does.
    KeepsSelectionWithin(glib::WeakRef<gtk::Widget>),
}

impl OriginClick {
    pub(in crate::ui) fn keeps_selection_within(content: &impl IsA<gtk::Widget>) -> Self {
        Self::KeepsSelectionWithin(content.upcast_ref::<gtk::Widget>().downgrade())
    }

    pub(super) fn clears_at(&self, picked: &gtk::Widget) -> bool {
        match self {
            Self::ClearsSelection => true,
            Self::KeepsSelectionWithin(content) => !content
                .upgrade()
                .is_some_and(|content| *picked == content || picked.is_ancestor(&content)),
        }
    }
}

/// Starts a drag from chrome that belongs to one collection view, as granted by
/// [`Marquee::add_origin_surface`].
pub(super) fn begin_origin_drag(
    state: &Rc<MarqueeState>,
    gesture: &gtk::GestureDrag,
    x: f64,
    y: f64,
) {
    let Some(surface) = gesture.widget() else {
        return;
    };
    let accepted = inert_chrome_at(&surface, x, y).is_some();
    let Some(view) = state.view() else {
        return;
    };
    if !accepted || !view.is_mapped() {
        return;
    }
    let Some(scroll) = state.scroll() else {
        return;
    };
    let Some(anchor) = translate(&surface, &scroll, (x, y)) else {
        return;
    };
    gesture.set_state(gtk::EventSequenceState::Claimed);
    state.begin(anchor, gesture, true);
    state.clear_at_press();
}

type ResolveTarget = dyn Fn(&gtk::Widget, &gtk::Widget, f64, f64) -> Option<Marquee>;

/// A long-lived surface whose drag target is resolved per press.
struct SharedOrigin {
    surface: gtk::Widget,
    click: OriginClick,
    resolve: Box<ResolveTarget>,
    target: RefCell<Option<Rc<MarqueeState>>>,
}

impl SharedOrigin {
    fn begin(&self, gesture: &gtk::GestureDrag, x: f64, y: f64) {
        self.target.replace(None);
        let Some(picked) = inert_chrome_at(&self.surface, x, y) else {
            return;
        };
        let Some(marquee) = (self.resolve)(&self.surface, &picked, x, y) else {
            return;
        };
        let state = marquee.state;
        let Some(view) = state.view() else {
            return;
        };
        if !view.is_mapped() {
            return;
        }
        let Some(scroll) = state.scroll() else {
            return;
        };
        let Some(anchor) = translate(&self.surface, &scroll, (x, y)) else {
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        state.begin(anchor, gesture, self.click.clears_at(&picked));
        state.clear_at_press();
        self.target.replace(Some(state));
    }

    fn update(&self, gesture: &gtk::GestureDrag, offset: (f64, f64)) {
        let Some(motion) = DragMotion::of(gesture, offset) else {
            return;
        };
        let state = self.target.borrow().clone();
        if let Some(state) = state {
            state.update_drag(&self.surface, &motion);
        }
    }

    fn cancel(&self) {
        let state = self.target.borrow_mut().take();
        if let Some(state) = state {
            state.end();
        }
    }

    fn end(&self) {
        let state = self.target.borrow_mut().take();
        if let Some(state) = state {
            state.finish();
        }
    }
}

/// Lets one long-lived surface — the blank area beside the last open column, the
/// sidebar, or the preview pane — begin a marquee drag on whichever collection view
/// `resolve` picks for the press position. Unlike [`Marquee::add_origin_surface`] the
/// surface outlives the views it feeds, so the target is resolved per drag. Presses
/// landing on a control the surface hosts are filtered out before `resolve` is
/// consulted.
pub(in crate::ui) fn install_shared_origin_surface(
    surface: &impl IsA<gtk::Widget>,
    click: OriginClick,
    resolve: impl Fn(&gtk::Widget, &gtk::Widget, f64, f64) -> Option<Marquee> + 'static,
) {
    let surface = surface.clone().upcast::<gtk::Widget>();
    let origin = Rc::new(SharedOrigin {
        surface: surface.clone(),
        click,
        resolve: Box::new(resolve),
        target: RefCell::new(None),
    });
    let gesture = gtk::GestureDrag::new();
    gesture.set_button(1);
    let origin_for_begin = origin.clone();
    gesture.connect_drag_begin(move |gesture, x, y| origin_for_begin.begin(gesture, x, y));
    let origin_for_update = origin.clone();
    gesture.connect_drag_update(move |gesture, offset_x, offset_y| {
        origin_for_update.update(gesture, (offset_x, offset_y));
    });
    let origin_for_cancel = origin.clone();
    gesture.connect_cancel(move |_, _| origin_for_cancel.cancel());
    gesture.connect_drag_end(move |_, _, _| origin.end());
    surface.add_controller(gesture);
}
