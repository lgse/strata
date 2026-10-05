// SPDX-License-Identifier: MIT

//! Autoplayed sound starts silent and rises on a slow-in, slow-out curve. The
//! gain is a multiplier on the player's volume element, so the saved volume is
//! never touched and no audio is processed in the application.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{glib, prelude::*};

use crate::ui::media::DecodedMedia;

const STEP: Duration = Duration::from_millis(16);
/// Shorter files would lose most of themselves to the rise; they start at full
/// volume. Unknown durations ease in.
const MIN_DURATION_US: i64 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Off,
    /// Silent until sound starts flowing and the rise begins.
    Armed,
    Rising,
}

pub(super) struct EaseIn {
    ramp: Duration,
    state: Cell<State>,
    timer: RefCell<Option<glib::SourceId>>,
    media: RefCell<Option<glib::WeakRef<DecodedMedia>>>,
}

/// Slow in, slow out, on a loudness-friendly curve: a smoothstep squared, so
/// the gain leaves zero and reaches one with no slope, and the ear hears an
/// even rise rather than a late jump.
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

    /// Silences `media` until `start`; a muted stream is left alone.
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

    /// Begins the rise, when sound actually starts flowing. A short file skips
    /// it and plays at full volume from here.
    pub(super) fn start(self: &Rc<Self>) {
        if self.state.get() != State::Armed {
            return;
        }
        let duration = self
            .media
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade)
            .map_or(0, |media| media.duration());
        if (1..MIN_DURATION_US).contains(&duration) {
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

    /// Any deliberate playback input brings the sound in at once.
    pub(super) fn end(&self) {
        if self.state.get() == State::Off {
            return;
        }
        self.cancel();
        self.set_fade(1.0);
    }

    /// Forgets the stream without touching it, as it is being replaced.
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
