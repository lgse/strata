// SPDX-License-Identifier: MIT

//! The host without a window: one thread owns a Rune `Runtime` and the default GLib context,
//! and steps both in real time. The in-process tests run on it, and the `--qa` mode will.

use std::time::{Duration, Instant};

use rune::{Env, Runtime, text::TextSystem};
use strata_desktop::settings::Preferences;
use strata_kit::fonts::{FAMILIES, FONTS};
use strata_platform::csd::{Backend, Csd, Policy};

use crate::{
    shell::{self, App},
    startup::UiThread,
};

/// The QA runner's window (`tests/qa/`: Xvfb at 1440x900).
const SIZE: (f32, f32) = (1440.0, 900.0);

pub(crate) struct Offscreen {
    runtime: Runtime<App>,
    context: glib::MainContext,
    stepped: Instant,
    _ui: UiThread,
}

impl Offscreen {
    pub(crate) fn new(ui: UiThread, settings: &Preferences) -> Self {
        let app = App::new(
            &ui,
            Csd::new(Policy::Auto.frame(), Backend::Headless),
            settings,
        );
        let mut runtime = Runtime::with_text(
            app,
            shell::view,
            SIZE,
            Env::new(true, 1.0),
            TextSystem::new(FONTS, FAMILIES),
        );
        runtime.poll = Some(shell::poll);
        Self {
            runtime,
            context: glib::MainContext::default(),
            stepped: Instant::now(),
            _ui: ui,
        }
    }

    pub(crate) fn app(&self) -> &App {
        &self.runtime.state
    }

    /// Dispatches the sources GLib has ready. Returns whether any ran.
    fn dispatch_ready(&self) -> bool {
        let started = Instant::now();
        let mut dispatched = false;
        while self.context.iteration(false) {
            dispatched = true;
            if started.elapsed() >= super::DISPATCH_BUDGET {
                break;
            }
        }
        dispatched
    }

    /// One turn of the loop: what GLib has ready, then one Rune frame stepped by the wall
    /// clock, whose `poll` applies what the dispatch queued. Returns whether either side did
    /// anything.
    pub(crate) fn turn(&mut self) -> bool {
        let dispatched = self.dispatch_ready();
        let queued = self.runtime.state.has_queued_changes();
        let now = Instant::now();
        let animating = self.runtime.frame((now - self.stepped).as_secs_f64());
        self.stepped = now;
        dispatched || queued || animating
    }

    /// Blocks until GLib has work or Rune's next timer is due, and for `limit` at most.
    pub(crate) fn wait(&mut self, limit: Duration) {
        let since_frame = self.stepped.elapsed().as_secs_f64();
        let until_timer = self.runtime.wake_deadline().map(|deadline| {
            Duration::from_secs_f64((deadline - self.runtime.clock - since_frame).max(0.0))
        });
        let wait = until_timer.map_or(limit, |until| until.min(limit));
        let timer = glib::timeout_source_new(wait, None, glib::Priority::DEFAULT, || {
            glib::ControlFlow::Break
        });
        timer.attach(Some(&self.context));
        self.context.iteration(true);
        timer.destroy();
    }

    /// Turns until `done`, waiting whenever a turn finds nothing to do. False on a timeout.
    pub(crate) fn run_until(&mut self, timeout: Duration, done: impl Fn(&App) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let worked = self.turn();
            if done(self.app()) {
                return true;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            if !worked {
                self.wait(left);
            }
        }
    }
}

#[cfg(test)]
mod tests;
