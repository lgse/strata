// SPDX-License-Identifier: MIT

//! GTK's side of the scripted QA mode. `strata-qa` runs the session and decides what a step,
//! a target and a predicate mean; this module answers its `Backend` from widgets and sends
//! input through X or Wayland.

mod backend;
mod check;
mod clipboard;
mod criticals;
mod dispatch;
mod input;
mod keyboard;
mod observe;
mod reveal;
mod samples;
mod state;
mod targets;

use std::{
    cell::RefCell,
    ffi::OsString,
    fs::File,
    time::{Duration, Instant},
};
use strata_qa as protocol;

use gtk::{gdk, glib, prelude::*};

use self::{
    backend::{Gtk, Host},
    protocol::{Capture, EVENT_GAP, Outcome, POLL_INTERVAL, REVEAL_SCROLL_ATTEMPTS},
};

/// The release workflow and QA job grep for its `usage: strata --qa` prefix;
/// `scripts/test_qa_mutation.py` checks the patterns against this text.
const USAGE: &str = "usage: strata --qa <directory|--portal>";
/// Mirrors the private `ui::browser::peek::PEEK_LABEL`; a peek test fails if they drift.
pub(crate) const PEEK_LABEL: &str = "Folder peek";
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const READY_POLL: Duration = Duration::from_millis(5);
const FRAME_TIMEOUT: Duration = Duration::from_millis(250);
const WHEEL_REVEAL_CLICKS: u32 = 3;
const DROP_SAMPLE_DELAYS: [Duration; 3] = [
    Duration::from_millis(60),
    Duration::from_millis(120),
    Duration::from_millis(180),
];
const DROP_SAMPLE_LIMIT: Duration = Duration::from_millis(240);
const DROP_SAMPLE_DRIFT: f64 = 1.5;
const DROP_SAMPLE_RECOVERY: f64 = 1.25;

pub(crate) struct Session {
    protocol: File,
    application_arguments: Vec<OsString>,
    portal: bool,
}

const PORTAL_ARGUMENT: &str = "--portal";

impl Session {
    /// Takes over stdout for replies before anything can log there; app output goes to stderr.
    pub(crate) fn prepare(arguments: &[OsString]) -> Result<Self, String> {
        let [program, _, directory] = arguments else {
            return Err(USAGE.to_owned());
        };
        // The directory is opened like any launch argument, so a main-thread
        // I/O audit sees no file call that a release build would not make.
        let portal = directory == PORTAL_ARGUMENT;
        let protocol = protocol::reply_stream()?;
        criticals::capture();
        Ok(Self {
            protocol,
            application_arguments: vec![program.clone(), directory.clone()],
            portal,
        })
    }

    pub(crate) fn portal(&self) -> bool {
        self.portal
    }

    pub(crate) fn start_portal(self, main_loop: &glib::MainLoop) {
        self.start(Host::Portal(main_loop.clone()));
    }

    pub(crate) fn application_arguments(&self) -> &[OsString] {
        &self.application_arguments
    }

    pub(crate) fn attach(self, application: &gtk::Application) {
        let session = RefCell::new(Some(self));
        application.connect_startup(move |application| {
            if let Some(session) = session.take() {
                session.start(Host::Application(application.clone()));
            }
        });
    }

    fn start(self, host: Host) {
        let commands = protocol::command_lines();
        let backend = Gtk::new(host);
        glib::MainContext::default().spawn_local(async move {
            let _hold = backend.hold();
            protocol::serve(&backend, self.protocol, commands).await;
        });
    }
}

#[derive(Clone)]
enum Sample {
    Pixels(Capture),
    Text { position: f64, contrast: u64 },
    Instant(Instant),
}

async fn settle(window: Option<&gtk::Window>) {
    glib::timeout_future_with_priority(glib::Priority::LOW, Duration::ZERO).await;
    if let Some(clock) = window.and_then(WidgetExt::frame_clock) {
        let painted = next_paint(&clock, FRAME_TIMEOUT);
        clock.request_phase(gdk::FrameClockPhase::AFTER_PAINT);
        painted.await;
    }
    glib::timeout_future_with_priority(glib::Priority::LOW, Duration::ZERO).await;
}

/// Listens from the call, so a frame requested afterwards is not missed.
fn next_paint(clock: &gdk::FrameClock, limit: Duration) -> impl Future<Output = ()> + use<> {
    let (sender, receiver) = futures_channel::oneshot::channel();
    let sender = RefCell::new(Some(sender));
    let handler = clock.connect_after_paint(move |_| {
        if let Some(sender) = sender.take() {
            let _ = sender.send(());
        }
    });
    let clock = clock.clone();
    async move {
        let _ = glib::future_with_timeout(limit, receiver).await;
        clock.disconnect(handler);
    }
}

#[cfg(test)]
mod tests;
