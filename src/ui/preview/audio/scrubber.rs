// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, glib, graphene, prelude::*, subclass::prelude::*};

use super::palette::{follow_theme, palette, with_alpha};
use crate::media::peaks::{BUCKETS, rms};

const HEIGHT: i32 = 40;
const BAR_WIDTH: f32 = 2.0;
const BAR_GAP: f32 = 1.0;
const LINE_HEIGHT: f32 = 3.0;
const MIN_LOUDEST: f32 = 0.04;
const GROW_RATE: f32 = 9.0;
const SETTLED: f32 = 0.002;
const KEY_SEEK_US: i64 = 5_000_000;

type PreviewCallback = Rc<dyn Fn(Option<i64>)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Scrubber {
        pub(super) media: RefCell<Option<gtk::MediaStream>>,
        pub(super) handlers: RefCell<Vec<glib::SignalHandlerId>>,
        pub(super) targets: RefCell<Vec<f32>>,
        pub(super) shown: RefCell<Vec<f32>>,
        pub(super) loudest: Cell<f32>,
        pub(super) stale: Cell<bool>,
        pub(super) played_pixels: Cell<i32>,
        pub(super) spoken_second: Cell<i64>,
        pub(super) hover: Cell<Option<f64>>,
        pub(super) drag: Cell<Option<f64>>,
        pub(super) drag_origin: Cell<f64>,
        pub(super) tick: RefCell<Option<gtk::TickCallbackId>>,
        pub(super) last_frame: Cell<Option<i64>>,
        pub(super) on_preview: RefCell<Option<PreviewCallback>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Scrubber {
        const NAME: &'static str = "StrataAudioScrubber";
        type Type = super::Scrubber;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Slider);
        }
    }

    impl ObjectImpl for Scrubber {
        fn dispose(&self) {
            self.obj().set_media(None);
        }
    }

    impl WidgetImpl for Scrubber {
        fn measure(&self, orientation: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Vertical => (HEIGHT, HEIGHT, -1, -1),
                _ => (0, 0, -1, -1),
            }
        }

        fn unmap(&self) {
            if let Some(id) = self.tick.borrow_mut().take() {
                id.remove();
            }
            self.last_frame.set(None);
            self.parent_unmap();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct Scrubber(ObjectSubclass<imp::Scrubber>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Scrubber {
    pub(in crate::ui::preview) fn new() -> Self {
        let scrubber: Self = glib::Object::new();
        scrubber.add_css_class("preview-audio-scrubber");
        scrubber.set_focusable(true);
        scrubber.set_cursor_from_name(Some("pointer"));
        scrubber.update_property(&[gtk::accessible::Property::Label("Playback position")]);
        scrubber.imp().targets.replace(vec![0.0; BUCKETS as usize]);
        scrubber.imp().shown.replace(vec![0.0; BUCKETS as usize]);
        follow_theme(&scrubber);
        scrubber.install_input();
        scrubber
    }

    pub(in crate::ui::preview) fn connect_preview(&self, callback: impl Fn(Option<i64>) + 'static) {
        self.imp().on_preview.replace(Some(Rc::new(callback)));
    }

    pub(in crate::ui::preview) fn set_media(&self, media: Option<&gtk::MediaStream>) {
        let imp = self.imp();
        if let Some(previous) = imp.media.borrow_mut().take() {
            for handler in imp.handlers.borrow_mut().drain(..) {
                previous.disconnect(handler);
            }
        }
        imp.drag.set(None);
        imp.spoken_second.set(-1);
        let Some(media) = media else {
            return;
        };
        let weak = self.downgrade();
        let handlers = ["timestamp", "duration", "seekable"].map(|property| {
            let weak = weak.clone();
            media.connect_notify_local(Some(property), move |_, _| {
                if let Some(scrubber) = weak.upgrade() {
                    scrubber.sync_position();
                }
            })
        });
        imp.handlers.replace(handlers.into());
        imp.media.replace(Some(media.clone()));
        imp.played_pixels.set(-1);
        self.sync_position();
    }

    pub(in crate::ui::preview) fn clear_levels(&self) {
        self.imp().targets.borrow_mut().fill(0.0);
        // The outgoing waveform keeps its scale while it lowers.
        self.imp().stale.set(true);
        self.animate();
    }

    pub(super) fn add_levels(&self, start: u32, levels: &[u8]) {
        let imp = self.imp();
        let mut targets = imp.targets.borrow_mut();
        let mut loudest = if imp.stale.replace(false) {
            0.0
        } else {
            imp.loudest.get()
        };
        for (target, level) in targets.iter_mut().skip(start as usize).zip(levels) {
            *target = rms(*level);
            loudest = loudest.max(*target);
        }
        imp.loudest.set(loudest);
        drop(targets);
        self.animate();
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
                    super::clock(timestamp),
                    super::clock(duration)
                )),
            ]);
        }
    }

    fn preview(&self) {
        let imp = self.imp();
        let duration = self.duration();
        let fraction = imp.drag.get().or(imp.hover.get());
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
            #[weak(rename_to = scrubber)]
            self,
            move |gesture, x, _| {
                if scrubber.duration() <= 0 {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                scrubber.grab_focus();
                scrubber.imp().drag_origin.set(x);
                scrubber.imp().drag.set(Some(scrubber.fraction_at(x)));
                scrubber.preview();
            }
        ));
        drag.connect_drag_update(glib::clone!(
            #[weak(rename_to = scrubber)]
            self,
            move |_, dx, _| {
                let imp = scrubber.imp();
                if imp.drag.get().is_some() {
                    imp.drag
                        .set(Some(scrubber.fraction_at(imp.drag_origin.get() + dx)));
                    scrubber.preview();
                }
            }
        ));
        drag.connect_drag_end(glib::clone!(
            #[weak(rename_to = scrubber)]
            self,
            move |_, _, _| {
                if let Some(fraction) = scrubber.imp().drag.take() {
                    scrubber.seek_to((fraction * scrubber.duration() as f64) as i64);
                    scrubber.imp().played_pixels.set(-1);
                    scrubber.preview();
                }
            }
        ));
        self.add_controller(drag);

        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = scrubber)]
            self,
            move |_, x, _| {
                scrubber.imp().hover.set(Some(scrubber.fraction_at(x)));
                scrubber.preview();
            }
        ));
        motion.connect_leave(glib::clone!(
            #[weak(rename_to = scrubber)]
            self,
            move |_| {
                scrubber.imp().hover.set(None);
                scrubber.preview();
            }
        ));
        self.add_controller(motion);

        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = scrubber)]
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
                let timestamp = scrubber
                    .imp()
                    .media
                    .borrow()
                    .as_ref()
                    .map_or(0, |media| media.timestamp());
                scrubber.seek_to(timestamp + delta);
                glib::Propagation::Stop
            }
        ));
        self.add_controller(keys);
    }

    fn animate(&self) {
        let imp = self.imp();
        if !crate::ui::motion::animations_enabled() || !self.is_mapped() {
            imp.shown.borrow_mut().clone_from(&imp.targets.borrow());
            self.queue_draw();
            return;
        }
        if imp.tick.borrow().is_some() {
            return;
        }
        let id = self.add_tick_callback(|scrubber, clock| {
            let imp = scrubber.imp();
            let now = clock.frame_time();
            let elapsed = imp
                .last_frame
                .replace(Some(now))
                .map_or(1.0 / 60.0, |last| (now - last) as f32 / 1_000_000.0);
            let step = 1.0 - (-GROW_RATE * elapsed.min(0.1)).exp();
            let mut moving = false;
            for (shown, target) in imp
                .shown
                .borrow_mut()
                .iter_mut()
                .zip(imp.targets.borrow().iter())
            {
                *shown += (target - *shown) * step;
                if (target - *shown).abs() > SETTLED {
                    moving = true;
                } else {
                    *shown = *target;
                }
            }
            scrubber.queue_draw();
            if moving {
                glib::ControlFlow::Continue
            } else {
                imp.tick.borrow_mut().take();
                imp.last_frame.set(None);
                glib::ControlFlow::Break
            }
        });
        imp.tick.replace(Some(id));
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let (width, height) = (self.width() as f32, self.height() as f32);
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let colors = palette();
        let played = self.played_fraction() as f32 * width;
        let hover = imp
            .drag
            .get()
            .or(imp.hover.get())
            .map(|fraction| fraction as f32 * width);
        let center = (height / 2.0).round();
        let shown = imp.shown.borrow();
        let loudest = imp.loudest.get().max(MIN_LOUDEST);
        let pitch = BAR_WIDTH + BAR_GAP;
        let count = ((width + BAR_GAP) / pitch).floor().max(1.0) as usize;
        let buckets = shown.len();
        let unplayed = with_alpha(colors.dim, 0.45);
        let hovered = with_alpha(colors.accent, 0.55);

        for index in 0..count {
            let x = index as f32 * pitch;
            let first = index * buckets / count;
            let last = ((index + 1) * buckets / count).max(first + 1).min(buckets);
            let level = shown[first..last].iter().copied().fold(0.0, f32::max);
            let bar = ((level / loudest).min(1.0) * (height - 2.0)).max(LINE_HEIGHT);
            let middle = x + BAR_WIDTH / 2.0;
            let color = if middle <= played {
                colors.accent
            } else if hover.is_some_and(|hover| middle <= hover) {
                hovered
            } else {
                unplayed
            };
            snapshot.append_color(
                &color,
                &graphene::Rect::new(x, center - bar / 2.0, BAR_WIDTH, bar),
            );
        }

        let head = played.clamp(1.0, width - 1.0);
        snapshot.append_color(
            &colors.peak,
            &graphene::Rect::new(head - 1.0, 0.0, 2.0, height),
        );
        if let Some(hover) = hover.filter(|hover| (hover - head).abs() > 2.0) {
            snapshot.append_color(
                &with_alpha(colors.text, 0.5),
                &graphene::Rect::new(hover.clamp(0.0, width - 1.0), 0.0, 1.0, height),
            );
        }
    }
}
