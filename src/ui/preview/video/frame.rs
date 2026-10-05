// SPDX-License-Identifier: MIT

//! The frame area's stand-in until a decoded frame exists: the listing's
//! thumbnail dimmed as a poster, or the picture's own surface colour at the
//! video's aspect. It has no edge of its own, so nothing changes at the
//! border when the first frame replaces it.

use std::{
    cell::{Cell, RefCell},
    time::{Duration, Instant},
};

use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};

use crate::ui::preview::audio::palette::{follow_theme, palette, with_alpha};

const FADE: Duration = Duration::from_millis(150);
const RADIUS: f32 = 8.0;
const DEFAULT_ASPECT: f64 = 16.0 / 9.0;
pub(super) const POSTER_OPACITY: f64 = 0.55;

/// The largest rectangle of `aspect` inside `width` × `height`, centred.
pub(super) fn fitted(width: f32, height: f32, aspect: f64) -> graphene::Rect {
    let aspect = aspect as f32;
    let (fit_width, fit_height) = if width / height > aspect {
        (height * aspect, height)
    } else {
        (width, width / aspect)
    };
    graphene::Rect::new(
        ((width - fit_width) / 2.0).floor(),
        ((height - fit_height) / 2.0).floor(),
        fit_width.floor(),
        fit_height.floor(),
    )
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Placeholder {
        pub(super) poster: RefCell<Option<gdk::Texture>>,
        pub(super) poster_opacity: Cell<f64>,
        pub(super) aspect: Cell<Option<f64>>,
        pub(super) fade: Cell<Option<Instant>>,
        pub(super) tick: RefCell<Option<gtk::TickCallbackId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Placeholder {
        const NAME: &'static str = "StrataVideoPlaceholder";
        type Type = super::Placeholder;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Placeholder {
        fn constructed(&self) {
            self.parent_constructed();
            let widget = self.obj();
            widget.add_css_class("preview-video-placeholder");
            widget.set_can_target(false);
            widget.set_can_focus(false);
            widget.set_accessible_role(gtk::AccessibleRole::Presentation);
            self.poster_opacity.set(POSTER_OPACITY);
            follow_theme(&*widget);
        }

        fn dispose(&self) {
            if let Some(tick) = self.tick.borrow_mut().take() {
                tick.remove();
            }
        }
    }

    impl WidgetImpl for Placeholder {
        fn measure(&self, _: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (width, height) = (widget.width() as f32, widget.height() as f32);
            if width <= 0.0 || height <= 0.0 {
                return;
            }
            let poster = self.poster.borrow().clone();
            let aspect = self
                .aspect
                .get()
                .or_else(|| {
                    poster
                        .as_ref()
                        .map(|poster| poster.intrinsic_aspect_ratio())
                })
                .filter(|aspect| aspect.is_finite() && *aspect > 0.0)
                .unwrap_or(DEFAULT_ASPECT);
            let rect = fitted(width, height, aspect);
            let rounded = gsk::RoundedRect::from_rect(rect, RADIUS);
            snapshot.push_rounded_clip(&rounded);
            match poster {
                Some(poster) => {
                    snapshot.push_opacity(self.poster_opacity.get());
                    snapshot.append_texture(&poster, &rect);
                    snapshot.pop();
                }
                // The same surface the picture draws behind its frames.
                None => snapshot.append_color(&with_alpha(palette().background, 0.55), &rect),
            }
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct Placeholder(ObjectSubclass<imp::Placeholder>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Placeholder {
    pub(super) fn new() -> Self {
        glib::Object::new()
    }

    pub(super) fn set_poster(&self, poster: Option<gdk::Texture>) {
        self.set_poster_with_opacity(poster, POSTER_OPACITY);
    }

    /// A storyboard cell stands in at nearly full strength during a seek.
    pub(super) fn set_poster_with_opacity(&self, poster: Option<gdk::Texture>, opacity: f64) {
        self.imp().poster.replace(poster);
        self.imp().poster_opacity.set(opacity);
        self.queue_draw();
    }

    #[cfg(test)]
    pub(in crate::ui::preview) fn poster(&self) -> Option<gdk::Texture> {
        self.imp().poster.borrow().clone()
    }

    /// The video's aspect once probed; `None` falls back to the poster or 16:9.
    pub(super) fn set_aspect(&self, aspect: Option<f64>) {
        if self.imp().aspect.replace(aspect) != aspect {
            self.queue_draw();
        }
    }

    pub(super) fn reveal(&self) {
        self.stop_fade();
        self.set_opacity(1.0);
        self.set_visible(true);
    }

    /// Fades out over the frame that replaced it, or hides at once without
    /// animations or while unmapped.
    pub(super) fn conceal(&self) {
        if !self.is_visible() || self.imp().fade.get().is_some() {
            return;
        }
        if !crate::ui::motion::animations_enabled() || !self.is_mapped() {
            self.set_visible(false);
            return;
        }
        self.imp().fade.set(Some(Instant::now()));
        let tick = self.add_tick_callback(|widget, _| {
            let Some(started) = widget.imp().fade.get() else {
                return glib::ControlFlow::Break;
            };
            let progress = started.elapsed().as_secs_f64() / FADE.as_secs_f64();
            if progress >= 1.0 {
                widget.imp().fade.set(None);
                widget.imp().tick.borrow_mut().take();
                widget.set_visible(false);
                widget.set_opacity(1.0);
                return glib::ControlFlow::Break;
            }
            widget.set_opacity(1.0 - crate::ui::motion::emphasized_deceleration(progress));
            glib::ControlFlow::Continue
        });
        self.imp().tick.replace(Some(tick));
    }

    fn stop_fade(&self) {
        self.imp().fade.set(None);
        if let Some(tick) = self.imp().tick.borrow_mut().take() {
            tick.remove();
        }
    }
}

#[cfg(test)]
mod tests;
