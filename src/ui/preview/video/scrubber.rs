// SPDX-License-Identifier: MIT

//! The video timeline: a flat track with the played range, a knob, chapter
//! ticks, and hover or drag positions reported for the storyboard bubble.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};

use crate::ui::preview::audio::{
    clock,
    palette::{follow_theme, palette, with_alpha},
};

const HEIGHT: i32 = 22;
const TRACK: f32 = 4.0;
const KNOB: f32 = 10.0;
const TICK_HEIGHT: f32 = 8.0;
const KEY_SEEK_US: i64 = 5_000_000;

type PreviewCallback = Rc<dyn Fn(Option<i64>)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Timeline {
        pub(super) media: RefCell<Option<gtk::MediaStream>>,
        pub(super) handlers: RefCell<Vec<glib::SignalHandlerId>>,
        pub(super) chapters: RefCell<Vec<f64>>,
        pub(super) played_pixels: Cell<i32>,
        pub(super) spoken_second: Cell<i64>,
        pub(super) hover: Cell<Option<f64>>,
        pub(super) drag: Cell<Option<f64>>,
        pub(super) drag_origin: Cell<f64>,
        pub(super) on_preview: RefCell<Option<PreviewCallback>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Timeline {
        const NAME: &'static str = "StrataVideoTimeline";
        type Type = super::Timeline;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Slider);
        }
    }

    impl ObjectImpl for Timeline {
        fn dispose(&self) {
            self.obj().set_media(None);
        }
    }

    impl WidgetImpl for Timeline {
        fn measure(&self, orientation: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Vertical => (HEIGHT, HEIGHT, -1, -1),
                _ => (0, 0, -1, -1),
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct Timeline(ObjectSubclass<imp::Timeline>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Timeline {
    pub(super) fn new() -> Self {
        let timeline: Self = glib::Object::new();
        timeline.add_css_class("preview-video-timeline");
        timeline.set_focusable(true);
        timeline.set_cursor_from_name(Some("pointer"));
        timeline.update_property(&[gtk::accessible::Property::Label("Playback position")]);
        follow_theme(&timeline);
        timeline.install_input();
        timeline
    }

    /// Called with the hovered or dragged time, or `None` when the pointer leaves.
    pub(super) fn connect_preview(&self, callback: impl Fn(Option<i64>) + 'static) {
        self.imp().on_preview.replace(Some(Rc::new(callback)));
    }

    pub(super) fn set_media(&self, media: Option<&gtk::MediaStream>) {
        let imp = self.imp();
        if let Some(previous) = imp.media.borrow_mut().take() {
            for handler in imp.handlers.borrow_mut().drain(..) {
                previous.disconnect(handler);
            }
        }
        imp.drag.set(None);
        imp.spoken_second.set(-1);
        let Some(media) = media else {
            self.queue_draw();
            return;
        };
        let weak = self.downgrade();
        let handlers = ["timestamp", "duration", "seekable"].map(|property| {
            let weak = weak.clone();
            media.connect_notify_local(Some(property), move |_, _| {
                if let Some(timeline) = weak.upgrade() {
                    timeline.sync_position();
                }
            })
        });
        imp.handlers.replace(handlers.into());
        imp.media.replace(Some(media.clone()));
        imp.played_pixels.set(-1);
        self.sync_position();
    }

    /// Chapter starts as fractions of the duration.
    pub(super) fn set_chapters(&self, chapters: Vec<f64>) {
        self.imp().chapters.replace(chapters);
        self.queue_draw();
    }

    #[cfg(test)]
    pub(super) fn chapters(&self) -> Vec<f64> {
        self.imp().chapters.borrow().clone()
    }

    /// The pointer's position along the track, as a fraction of the duration.
    pub(super) fn pointer_fraction(&self) -> Option<f64> {
        let imp = self.imp();
        imp.drag.get().or(imp.hover.get())
    }

    fn duration(&self) -> i64 {
        self.imp()
            .media
            .borrow()
            .as_ref()
            .map_or(0, |media| media.duration())
    }

    fn fraction_at(&self, x: f64) -> f64 {
        (x / f64::from(self.width().max(1))).clamp(0.0, 1.0)
    }

    fn played_fraction(&self) -> f64 {
        let imp = self.imp();
        if let Some(fraction) = imp.drag.get() {
            return fraction;
        }
        let duration = self.duration();
        imp.media
            .borrow()
            .as_ref()
            .filter(|_| duration > 0)
            .map_or(0.0, |media| {
                (media.timestamp() as f64 / duration as f64).clamp(0.0, 1.0)
            })
    }

    /// Timestamp notifications arrive up to every 8 ms; redraw only when a pixel changes.
    fn sync_position(&self) {
        let imp = self.imp();
        let pixels = (self.played_fraction() * f64::from(self.width())).round() as i32;
        if imp.played_pixels.replace(pixels) != pixels {
            self.queue_draw();
        }
        let Some((timestamp, duration)) = imp
            .media
            .borrow()
            .as_ref()
            .map(|media| (media.timestamp(), media.duration()))
        else {
            return;
        };
        let second = timestamp / 1_000_000;
        if imp.spoken_second.replace(second) != second {
            self.update_property(&[
                gtk::accessible::Property::ValueMin(0.0),
                gtk::accessible::Property::ValueMax((duration.max(0) / 1_000_000) as f64),
                gtk::accessible::Property::ValueNow(second as f64),
                gtk::accessible::Property::ValueText(&format!(
                    "{} of {}",
                    clock(timestamp),
                    clock(duration)
                )),
            ]);
        }
    }

    fn preview(&self) {
        let imp = self.imp();
        let duration = self.duration();
        let fraction = self.pointer_fraction();
        let callback = imp.on_preview.borrow().clone();
        if let Some(callback) = callback {
            callback(
                fraction
                    .filter(|_| duration > 0)
                    .map(|fraction| (fraction * duration as f64) as i64),
            );
        }
        self.queue_draw();
    }

    fn seek_to(&self, target: i64) {
        let media = self.imp().media.borrow().clone();
        if let Some(media) = media.filter(|media| media.is_seekable()) {
            media.seek(target.clamp(0, media.duration().max(0)));
        }
    }

    fn install_input(&self) {
        let drag = gtk::GestureDrag::new();
        drag.connect_drag_begin(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
            move |gesture, x, _| {
                if timeline.duration() <= 0 {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                timeline.grab_focus();
                timeline.imp().drag_origin.set(x);
                timeline.imp().drag.set(Some(timeline.fraction_at(x)));
                timeline.preview();
            }
        ));
        drag.connect_drag_update(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
            move |_, dx, _| {
                let imp = timeline.imp();
                if imp.drag.get().is_some() {
                    imp.drag
                        .set(Some(timeline.fraction_at(imp.drag_origin.get() + dx)));
                    timeline.preview();
                }
            }
        ));
        drag.connect_drag_end(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
            move |_, _, _| {
                if let Some(fraction) = timeline.imp().drag.take() {
                    timeline.seek_to((fraction * timeline.duration() as f64) as i64);
                    timeline.imp().played_pixels.set(-1);
                    timeline.preview();
                }
            }
        ));
        self.add_controller(drag);

        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
            move |_, x, _| {
                let fraction = timeline.fraction_at(x);
                // Pointer motion arrives far faster than a pixel moves.
                let moved = timeline
                    .imp()
                    .hover
                    .replace(Some(fraction))
                    .is_none_or(|previous| {
                        ((previous - fraction) * f64::from(timeline.width())).abs() >= 1.0
                    });
                if moved {
                    timeline.preview();
                }
            }
        ));
        motion.connect_leave(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
            move |_| {
                timeline.imp().hover.set(None);
                timeline.preview();
            }
        ));
        self.add_controller(motion);

        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = timeline)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                let delta = match key {
                    gdk::Key::Left | gdk::Key::KP_Left => -KEY_SEEK_US,
                    gdk::Key::Right | gdk::Key::KP_Right => KEY_SEEK_US,
                    _ => return glib::Propagation::Proceed,
                };
                if !modifiers.is_empty() {
                    return glib::Propagation::Proceed;
                }
                let timestamp = timeline
                    .imp()
                    .media
                    .borrow()
                    .as_ref()
                    .map_or(0, |media| media.timestamp());
                timeline.seek_to(timestamp + delta);
                glib::Propagation::Stop
            }
        ));
        self.add_controller(keys);
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let (width, height) = (self.width() as f32, self.height() as f32);
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let colors = palette();
        let played = self.played_fraction() as f32 * width;
        let hover = self
            .pointer_fraction()
            .map(|fraction| fraction as f32 * width);
        let center = (height / 2.0).round();
        let track = graphene::Rect::new(0.0, center - TRACK / 2.0, width, TRACK);
        snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(track, TRACK / 2.0));
        snapshot.append_color(&with_alpha(colors.dim, 0.35), &track);
        if let Some(hover) = hover.filter(|hover| *hover > played) {
            snapshot.append_color(
                &with_alpha(colors.accent, 0.4),
                &graphene::Rect::new(played, track.y(), hover - played, TRACK),
            );
        }
        snapshot.append_color(
            &colors.accent,
            &graphene::Rect::new(0.0, track.y(), played, TRACK),
        );
        snapshot.pop();
        for chapter in imp.chapters.borrow().iter() {
            let x = (*chapter as f32 * width).clamp(0.5, width - 0.5);
            snapshot.append_color(
                &with_alpha(colors.text, 0.55),
                &graphene::Rect::new(x - 0.5, center - TICK_HEIGHT / 2.0, 1.0, TICK_HEIGHT),
            );
        }
        if let Some(hover) = hover {
            snapshot.append_color(
                &with_alpha(colors.text, 0.5),
                &graphene::Rect::new(hover.clamp(0.0, width - 1.0), 2.0, 1.0, height - 4.0),
            );
        }
        let knob = graphene::Rect::new(
            (played - KNOB / 2.0).clamp(0.0, width - KNOB),
            center - KNOB / 2.0,
            KNOB,
            KNOB,
        );
        snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(knob, KNOB / 2.0));
        snapshot.append_color(&colors.peak, &knob);
        snapshot.pop();
    }
}
