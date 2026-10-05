// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{glib, prelude::*};

use crate::ui::media::DecodedMedia;

const STEP: Duration = Duration::from_millis(16);
// Short clips must not lose their opening to the fade.
const MIN_DURATION_US: i64 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Off,
    Armed,
    Rising,
}

pub(super) struct EaseIn {
    ramp: Duration,
    state: Cell<State>,
    timer: RefCell<Option<glib::SourceId>>,
    media: RefCell<Option<glib::WeakRef<DecodedMedia>>>,
}

// Squaring smoothstep softens the perceived onset without a late jump.
pub(super) fn gain(progress: f64) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    let smooth = progress * progress * (3.0 - 2.0 * progress);
    smooth * smooth
}

impl EaseIn {
    pub(super) fn new(ramp: Duration) -> Rc<Self> {
        Rc::new(Self {
            ramp,
            state: Cell::new(State::Off),
            timer: RefCell::default(),
            media: RefCell::default(),
        })
    }

    pub(super) fn arm(&self, media: &gtk::MediaStream) {
        let Some(decoded) = media.downcast_ref::<DecodedMedia>() else {
            return;
        };
        self.stop();
        if media.is_muted() {
            return;
        }
        decoded.set_fade(0.0);
        self.media.replace(Some(decoded.downgrade()));
        self.state.set(State::Armed);
    }

    pub(super) fn is_armed(&self) -> bool {
        self.state.get() == State::Armed
    }

    #[cfg(test)]
    pub(super) fn is_active(&self) -> bool {
        self.state.get() != State::Off
    }

    fn duration(&self) -> i64 {
        self.media
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade)
            .map_or(0, |media| media.duration())
    }

    /// Called at prepared, before samples can pass through the muted sink.
    pub(super) fn settle(&self) {
        if self.state.get() == State::Armed && (1..MIN_DURATION_US).contains(&self.duration()) {
            self.end();
        }
    }

    pub(super) fn start(self: &Rc<Self>) {
        if self.state.get() != State::Armed {
            return;
        }
        if (1..MIN_DURATION_US).contains(&self.duration()) {
            self.end();
            return;
        }
        self.state.set(State::Rising);
        let started = Instant::now();
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local(STEP, move || {
            let Some(ease) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if ease.state.get() != State::Rising {
                return glib::ControlFlow::Break;
            }
            let progress = started.elapsed().as_secs_f64() / ease.ramp.as_secs_f64();
            if progress >= 1.0 {
                ease.timer.borrow_mut().take();
                ease.state.set(State::Off);
                ease.set_fade(1.0);
                return glib::ControlFlow::Break;
            }
            ease.set_fade(gain(progress));
            glib::ControlFlow::Continue
        });
        self.timer.replace(Some(timer));
    }

    pub(super) fn end(&self) {
        if self.state.get() == State::Off {
            return;
        }
        self.cancel();
        self.set_fade(1.0);
    }

    pub(super) fn stop(&self) {
        self.cancel();
        self.media.borrow_mut().take();
    }

    fn cancel(&self) {
        self.state.set(State::Off);
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
    }

    fn set_fade(&self, fade: f64) {
        if let Some(media) = self
            .media
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade)
        {
            media.set_fade(fade);
        }
    }
}

#[cfg(test)]
mod tests;
