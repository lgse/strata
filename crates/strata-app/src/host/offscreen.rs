// SPDX-License-Identifier: MIT

//! The host without a window: one thread owns a Rune `Runtime` and the default GLib context,
//! and steps both in real time. The in-process tests run on it, and the `--qa` mode does.

use std::time::{Duration, Instant};

use rune::{Env, Runtime, text::TextSystem};
use strata_kit::fonts::{FAMILIES, FONTS};
use strata_platform::csd::{Backend, Csd, Policy};

use crate::{App, settings::Saved, shell, slots::Entry, startup::UiThread};

/// The QA runner's screen (`tests/qa/`: Xvfb at 1440x900): what the tests draw on.
#[cfg(test)]
const SIZE: (f32, f32) = (1440.0, 900.0);

pub(crate) struct Offscreen {
    runtime: Runtime<App>,
    context: glib::MainContext,
    stepped: Instant,
    _ui: UiThread,
}

impl Offscreen {
    #[cfg(test)]
    pub(crate) fn new(ui: UiThread, settings: Saved) -> Self {
        Self::sized(ui, settings, SIZE)
    }

    /// A host whose window is made of a test's table `parts`.
    #[cfg(test)]
    pub(crate) fn with_parts(parts: &'static [Entry<App>], ui: UiThread, settings: Saved) -> Self {
        Self::over(parts, ui, settings, SIZE)
    }

    /// A host that draws at `size`, in points.
    pub(crate) fn sized(ui: UiThread, settings: Saved, size: (f32, f32)) -> Self {
        Self::over(crate::PARTS, ui, settings, size)
    }

    fn over(parts: &'static [Entry<App>], ui: UiThread, settings: Saved, size: (f32, f32)) -> Self {
        let csd = Csd::new(Policy::Auto.frame(), Backend::Headless);
        let app = App::over(parts, &ui, csd, settings);
        let mut runtime = Runtime::with_text(
            app,
            shell::view,
            size,
            Env::new(true, 1.0),
            TextSystem::new(FONTS, FAMILIES),
        );
        runtime.poll = Some(shell::poll);
        runtime.on_key = Some(shell::on_key);
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

    /// Turns the look's motion off, as the GTK QA runs do through `gtk-enable-animations`:
    /// a column that closes then leaves the tree at once.
    #[cfg(feature = "qa")]
    pub(crate) fn without_animations(mut self) -> Self {
        self.runtime.update(|app, _| app.look.animations = false);
        self
    }

    /// Rune's side of the host: where input goes in, and the tree comes out.
    pub(crate) fn runtime(&mut self) -> &mut Runtime<App> {
        &mut self.runtime
    }

    /// The accessibility tree, as an assistive client and the QA harness read it.
    pub(crate) fn tree(&self) -> serde_json::Value {
        self.runtime.accessibility_json()
    }

    /// The elements as drawn, the ones the accessibility tree leaves out among them.
    #[cfg(feature = "qa")]
    pub(crate) fn elements(&self) -> Vec<rune::ElementInfo> {
        self.runtime.elements()
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
        shell::publish(&mut self.runtime.state);
        let dispatched = self.dispatch_ready();
        let queued = self.runtime.state.has_queued_changes();
        let now = Instant::now();
        let animating = self.runtime.frame((now - self.stepped).as_secs_f64());
        self.stepped = now;
        // What a handler queued during the frame is work for the next turn.
        let left = self.runtime.state.has_queued_changes();
        dispatched || queued || animating || left
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
    #[cfg(test)]
    pub(crate) fn run_until(&mut self, timeout: Duration, done: impl Fn(&Self) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let worked = self.turn();
            if done(self) {
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
