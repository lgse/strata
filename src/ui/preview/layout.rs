// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::{
    browser::{BrowserView, COLUMN_WIDTH, WeakBrowserView},
    browser_modes::BrowserMode,
};

const MIN_COLUMN_MULTIPLIER: i32 = 2;

#[derive(Default)]
pub(super) struct SplitSizing {
    binding: RefCell<Option<BrowserBinding>>,
    manual_width: Cell<Option<i32>>,
    resizing: Cell<bool>,
}

impl SplitSizing {
    pub(super) fn cancel_resize(&self) {
        self.resizing.set(false);
    }
}

struct BrowserBinding {
    content: glib::WeakRef<gtk::Paned>,
    browser: WeakBrowserView,
}

#[derive(Clone, Copy)]
struct Geometry {
    available: i32,
    occupied: i32,
    start_minimum: i32,
    separator: i32,
    columns: bool,
}

impl Geometry {
    fn maximum_width(self) -> i32 {
        (self.available - self.separator - self.start_minimum).max(1)
    }

    fn minimum_width(self, manual: bool) -> i32 {
        let minimum = if manual {
            COLUMN_WIDTH
        } else if self.columns {
            COLUMN_WIDTH * MIN_COLUMN_MULTIPLIER
        } else {
            MIN_WIDTH
        };
        minimum.min(self.maximum_width())
    }

    fn preview_width(self, manual: Option<i32>) -> i32 {
        let free = (self.available - self.separator - self.occupied).max(0);
        let desired = manual.unwrap_or_else(|| {
            if self.columns {
                free
            } else {
                free.saturating_mul(9).saturating_div(10).min(MAX_WIDTH)
            }
        });
        desired.clamp(self.minimum_width(manual.is_some()), self.maximum_width())
    }

    fn position(self, manual: Option<i32>) -> i32 {
        self.available - self.separator - self.preview_width(manual)
    }
}

fn separator(split: &gtk::Paned) -> Option<gtk::Widget> {
    let mut child = split.first_child();
    while let Some(widget) = child {
        if widget.css_name() == "separator" {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

fn separator_width(split: &gtk::Paned) -> i32 {
    separator(split).map_or(0, |handle| {
        handle.measure(gtk::Orientation::Horizontal, -1).0
    })
}

impl PreviewDrawer {
    pub(in crate::ui) fn attach_split(
        &self,
        split: &gtk::Paned,
        content: &gtk::Paned,
        browser: &BrowserView,
    ) {
        self.state.split.replace(Some(split.clone()));
        self.state.sizing.binding.replace(Some(BrowserBinding {
            content: content.downgrade(),
            browser: browser.downgrade(),
        }));
        browser.bind_preview_scrolling(&self.state.revealer);
        if !self.state.opened.get() {
            split.set_end_child(None::<&gtk::Widget>);
        }
        let weak = Rc::downgrade(&self.state);
        split.add_tick_callback(move |split, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.opened.get() && !state.animating.get() && !state.sizing.resizing.get() {
                state.sync_split(split);
            }
            glib::ControlFlow::Continue
        });
        install_resize(split, &self.state);
    }
}

impl PreviewState {
    fn geometry(&self, split: &gtk::Paned) -> Geometry {
        let available = split.width();
        let mut geometry = Geometry {
            available,
            occupied: available.saturating_sub(DEFAULT_WIDTH),
            start_minimum: split
                .start_child()
                .map_or(0, |child| child.measure(gtk::Orientation::Horizontal, -1).0),
            separator: separator_width(split),
            columns: false,
        };
        if let Some(binding) = self.sizing.binding.borrow().as_ref()
            && let Some(content) = binding.content.upgrade()
            && let Some(browser) = binding.browser.upgrade()
        {
            let sidebar = if content
                .start_child()
                .is_some_and(|child| child.is_visible())
            {
                content.position() + separator_width(&content)
            } else {
                0
            };
            geometry.columns = browser.view_mode() == BrowserMode::Columns;
            geometry.occupied =
                sidebar + browser.preview_occupied_width((available - sidebar).max(0));
        }
        geometry
    }

    fn sync_split(&self, split: &gtk::Paned) {
        if split.width() <= 0 {
            return;
        }
        let geometry = self.geometry(split);
        let manual = self.sizing.manual_width.get();
        let minimum = geometry.minimum_width(manual.is_some());
        let position = geometry.position(manual);
        if self.pane.width_request() != minimum || split.position() != position {
            self.pane.set_width_request(minimum);
            split.set_position(position);
        }
    }

    pub(super) fn opening_width(&self, available: i32) -> i32 {
        self.split
            .borrow()
            .as_ref()
            .map_or(DEFAULT_WIDTH.min(available), |split| {
                self.geometry(split)
                    .preview_width(self.sizing.manual_width.get())
            })
    }

    pub(super) fn animate_open(self: &Rc<Self>, split: &gtk::Paned) {
        let geometry = self.geometry(split);
        let target = geometry.position(self.sizing.manual_width.get());
        let start = split.width();
        split.set_position(start);
        let animation_id = self.animation_generation.get().saturating_add(1);
        self.animation_generation.set(animation_id);
        self.animating.set(true);

        if !super::super::motion::animations_enabled() || start <= 0 {
            self.animating.set(false);
            self.sync_split(split);
            return;
        }

        let started = Instant::now();
        let weak = Rc::downgrade(self);
        split.add_tick_callback(move |split, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.animation_generation.get() != animation_id {
                return glib::ControlFlow::Break;
            }
            let progress =
                (started.elapsed().as_secs_f64() / TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
            let eased = super::super::motion::emphasized_deceleration(progress);
            let position = f64::from(start) + f64::from(target - start) * eased;
            split.set_position(position.round() as i32);
            if progress >= 1.0 {
                state.animating.set(false);
                state.sync_split(split);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    pub(super) fn resize_preview(&self, split: &gtk::Paned, position: i32) {
        let geometry = self.geometry(split);
        let width = (geometry.available - geometry.separator - position)
            .clamp(geometry.minimum_width(true), geometry.maximum_width());
        self.sizing.manual_width.set(Some(width));
        self.sync_split(split);
    }
}

fn install_resize(split: &gtk::Paned, state: &Rc<PreviewState>) {
    // Observe input without competing with GtkPaned's own drag gesture.
    let pointer = gtk::EventControllerLegacy::new();
    pointer.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    pointer.connect_event(move |controller, event| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let Some(split) = controller.widget().and_downcast::<gtk::Paned>() else {
            return glib::Propagation::Proceed;
        };
        match event.event_type() {
            gtk::gdk::EventType::ButtonPress | gtk::gdk::EventType::TouchBegin
                if (event.event_type() == gtk::gdk::EventType::TouchBegin
                    || event
                        .downcast_ref::<gtk::gdk::ButtonEvent>()
                        .is_some_and(|e| e.button() == 1))
                    && state.opened.get()
                    && on_separator(&split, event) =>
            {
                state
                    .animation_generation
                    .set(state.animation_generation.get().saturating_add(1));
                state.animating.set(false);
                state.sizing.resizing.set(true);
                state
                    .pane
                    .set_width_request(state.geometry(&split).minimum_width(true));
            }
            gtk::gdk::EventType::ButtonRelease
            | gtk::gdk::EventType::TouchEnd
            | gtk::gdk::EventType::TouchCancel
            | gtk::gdk::EventType::GrabBroken => {
                state.sizing.resizing.set(false);
            }
            _ => {}
        }
        glib::Propagation::Proceed
    });
    split.add_controller(pointer);
    let weak = Rc::downgrade(state);
    split.connect_position_notify(move |split| {
        if let Some(state) = weak.upgrade()
            && state.opened.get()
            && state.sizing.resizing.replace(false)
        {
            state.resize_preview(split, split.position());
            state.sizing.resizing.set(true);
        }
    });

    // Unhandled browser keys also reach GtkPaned; only handle-focused actions are resizes.
    let weak = Rc::downgrade(state);
    split.connect_move_handle(move |split, _| {
        if split.has_focus() {
            if let Some(state) = weak.upgrade()
                && state.opened.get()
            {
                state
                    .pane
                    .set_width_request(state.geometry(split).minimum_width(true));
            }
            remember_keyboard_width(weak.clone());
        }
        false
    });
    let weak = Rc::downgrade(state);
    split.connect_cancel_position(move |split| {
        if split.has_focus() {
            remember_keyboard_width(weak.clone());
        }
        false
    });
}

fn on_separator(split: &gtk::Paned, event: &gtk::gdk::Event) -> bool {
    let Some((x, y)) = event.position() else {
        return false;
    };
    let Some(native) = split.native() else {
        return false;
    };
    let (dx, dy) = native.surface_transform();
    let native: gtk::Widget = native.upcast();
    let Some(point) = native.compute_point(
        split,
        &gtk::graphene::Point::new((x + dx) as f32, (y + dy) as f32),
    ) else {
        return false;
    };
    split.pick(
        f64::from(point.x()),
        f64::from(point.y()),
        gtk::PickFlags::DEFAULT,
    ) == separator(split)
}

fn remember_keyboard_width(weak: std::rc::Weak<PreviewState>) {
    let Some(state) = weak.upgrade().filter(|state| state.opened.get()) else {
        return;
    };
    let Some(split) = state.split.borrow().clone() else {
        return;
    };
    let before = split.position();
    state.sizing.resizing.set(true);
    // Wait for GTK's default action handler before resuming automatic layout.
    glib::idle_add_local_once(move || {
        if let Some(state) = weak.upgrade() {
            state.sizing.resizing.set(false);
            if state.opened.get() && split.position() != before {
                state.resize_preview(&split, split.position());
            }
        }
    });
}

#[cfg(test)]
mod tests;
