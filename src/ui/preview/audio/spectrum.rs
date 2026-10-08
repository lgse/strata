// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};

use gtk::{glib, graphene, gsk, prelude::*, subclass::prelude::*};

use super::{
    analysis::{Analyzer, Ballistics, FFT_SIZE, band_levels},
    palette::{follow_theme, palette, with_alpha},
};
use crate::ui::media::DecodedMedia;

const NATURAL_HEIGHT: i32 = 150;
const BAR_TARGET_WIDTH: f32 = 5.0;
const BAR_GAP: f32 = 2.0;
const MIN_BARS: usize = 12;
const MAX_BARS: usize = 96;
const REFLECTION: f32 = 0.2;
const REST_HEIGHT: f32 = 2.0;
const PEAK_HEIGHT: f32 = 2.0;

struct State {
    analyzer: Analyzer,
    ballistics: Ballistics,
    window: Vec<f32>,
    levels: Vec<f32>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            analyzer: Analyzer::new(),
            ballistics: Ballistics::default(),
            window: vec![0.0; FFT_SIZE],
            levels: Vec::new(),
        }
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Spectrum {
        pub(super) media: glib::WeakRef<DecodedMedia>,
        pub(super) state: RefCell<State>,
        pub(super) tick: RefCell<Option<gtk::TickCallbackId>>,
        pub(super) last_frame: Cell<Option<i64>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Spectrum {
        const NAME: &'static str = "StrataAudioSpectrum";
        type Type = super::Spectrum;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    impl ObjectImpl for Spectrum {}

    impl WidgetImpl for Spectrum {
        fn measure(&self, orientation: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Vertical => (0, NATURAL_HEIGHT, -1, -1),
                _ => (0, 0, -1, -1),
            }
        }

        fn map(&self) {
            self.parent_map();
            self.obj().wake();
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            let count = bar_count(width).clamp(MIN_BARS, MAX_BARS);
            self.state.borrow_mut().ballistics.resize(count);
        }

        fn unmap(&self) {
            self.obj().stop();
            self.parent_unmap();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            draw(
                snapshot,
                &self.state.borrow().ballistics,
                widget.width() as f32,
                widget.height() as f32,
            );
        }
    }
}

glib::wrapper! {
    pub struct Spectrum(ObjectSubclass<imp::Spectrum>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

fn bar_count(width: i32) -> usize {
    ((width as f32 + BAR_GAP) / (BAR_TARGET_WIDTH + BAR_GAP)).floor() as usize
}

impl Spectrum {
    pub(super) fn new() -> Self {
        let spectrum: Self = glib::Object::new();
        spectrum.add_css_class("preview-audio-spectrum");
        spectrum.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr(
            "Audio spectrum",
        ))]);
        follow_theme(&spectrum);
        spectrum
    }

    /// Switching tracks keeps the bars' motion so they fall and rise into the next one.
    pub(super) fn set_media(&self, media: Option<&DecodedMedia>) {
        if let Some(media) = media {
            media.retain_played_audio();
        }
        self.imp().media.set(media);
        self.wake();
    }

    pub(super) fn wake(&self) {
        let imp = self.imp();
        if imp.tick.borrow().is_some() || !self.is_mapped() {
            return;
        }
        let id = self.add_tick_callback(|widget, clock| {
            if widget.advance(clock.frame_time()) {
                glib::ControlFlow::Continue
            } else {
                widget.imp().tick.borrow_mut().take();
                widget.imp().last_frame.set(None);
                glib::ControlFlow::Break
            }
        });
        imp.tick.replace(Some(id));
    }

    fn stop(&self) {
        if let Some(id) = self.imp().tick.borrow_mut().take() {
            id.remove();
        }
        self.imp().last_frame.set(None);
    }

    fn advance(&self, frame_time: i64) -> bool {
        let imp = self.imp();
        let elapsed = imp
            .last_frame
            .replace(Some(frame_time))
            .map_or(0.0, |last| (frame_time - last) as f32 / 1_000_000.0);
        let media = imp.media.upgrade();
        let playing = media.as_ref().is_some_and(|media| media.is_playing());
        let mut state = imp.state.borrow_mut();
        let State {
            analyzer,
            ballistics,
            window,
            levels,
        } = &mut *state;
        levels.resize(ballistics.bars().len(), 0.0);
        let heard = media
            .as_ref()
            .is_some_and(|media| media.played_samples(window));
        if heard {
            band_levels(analyzer.power_spectrum(window), levels);
            ballistics.update(Some(levels), elapsed);
        } else {
            ballistics.update(None, elapsed);
        }
        let settled = ballistics.is_settled();
        drop(state);
        self.queue_draw();
        playing || !settled
    }
}

fn draw(snapshot: &gtk::Snapshot, ballistics: &Ballistics, width: f32, height: f32) {
    let bars = ballistics.bars();
    if bars.is_empty() || width <= 0.0 || height <= 0.0 {
        return;
    }
    let colors = palette();
    let count = bars.len() as f32;
    let bar_width = ((width - BAR_GAP * (count - 1.0)) / count).max(1.0);
    let pitch = bar_width + BAR_GAP;
    let baseline = (height * (1.0 - REFLECTION)).floor();
    let reflection_top = baseline + 2.0;
    let reflection_height = (height - reflection_top).max(0.0);
    let bar_stops = [
        gsk::ColorStop::new(0.0, colors.accent),
        gsk::ColorStop::new(1.0, colors.accent_bright),
    ];
    let reflection_stops = [
        gsk::ColorStop::new(0.0, with_alpha(colors.accent, 0.32)),
        gsk::ColorStop::new(1.0, with_alpha(colors.accent, 0.0)),
    ];
    // Every bar spans the same gradient, so taller bars reach brighter colors.
    for (index, level) in bars.iter().enumerate() {
        let x = index as f32 * pitch;
        let bar = (level * baseline).max(REST_HEIGHT);
        snapshot.append_linear_gradient(
            &graphene::Rect::new(x, baseline - bar, bar_width, bar),
            &graphene::Point::new(0.0, baseline),
            &graphene::Point::new(0.0, 0.0),
            &bar_stops,
        );
        let reflection = (bar * 0.4).min(reflection_height);
        if reflection > 1.0 {
            snapshot.append_linear_gradient(
                &graphene::Rect::new(x, reflection_top, bar_width, reflection),
                &graphene::Point::new(0.0, reflection_top),
                &graphene::Point::new(0.0, height),
                &reflection_stops,
            );
        }
    }

    for (index, peak) in ballistics.peaks().enumerate() {
        let top = (baseline - peak * baseline - PEAK_HEIGHT - 1.0).max(0.0);
        if peak > 0.015 {
            snapshot.append_color(
                &colors.peak,
                &graphene::Rect::new(index as f32 * pitch, top, bar_width, PEAK_HEIGHT),
            );
        }
    }
}
