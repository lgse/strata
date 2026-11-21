// SPDX-License-Identifier: MIT

//! The scripted QA mode: `strata-rune --qa <directory>` speaks the protocol of `strata-qa`
//! on stdin and stdout, as the GTK application's `strata --qa` does.
//!
//! `strata-qa` runs the session and decides what a step, a target and a predicate mean. This
//! module answers its `Backend` from Rune's accessibility tree and sends input at Rune's
//! input layer, on the offscreen host: it needs no display server. With `STRATA_QA_WINDOW=1`
//! it serves the same protocol from a real window instead ([`session`]).
//! `docs/rune/slice/V7-rune-qa-backend.md` describes it.

mod backend;
mod compose;
mod control_words;
mod field;
mod filter;
mod host;
mod input_log;
mod invariants;
mod jobs;
mod kept;
mod keyboard;
mod keymap;
mod keys;
mod portal;
mod replaced;
mod reveal;
mod roles;
mod session;
mod sidebar;
mod stage;
mod staged;
mod submenus;
mod tenxer;
mod toplevel;
mod tree;
mod windows;

use std::{
    ffi::{OsStr, OsString},
    future::Future,
    pin::pin,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

use strata_core::logging::criticals;

use strata_platform::single_instance::{Claim, Launch};

use self::{backend::Rune, input_log::InputLog};
use crate::{
    application::{self, Opening},
    host::offscreen::Offscreen,
    launch,
    settings::Saved,
    startup,
};

/// What a wrong `--qa` command line prints. The release workflow fails a release binary that
/// holds the `usage: strata --qa` prefix, and the QA job requires it in a QA build of the GTK
/// application; the same prefix here lets both checks cover this binary once they are pointed
/// at it. It names `strata` because that is what this interface's binary is called at parity.
const USAGE: &str = "usage: strata --qa <directory|--plain|--portal>";
/// The argument that serves the portal chooser instead of a folder (`src/qa.rs`).
const PORTAL: &str = "--portal";
/// The argument of a plain launch, with no folder: the window restores the saved tab session
/// or opens the default folder, and saves its tabs, as `strata-rune` with no argument does
/// (`src/qa.rs`, and the runner's `{"restart":{"plain":true}}`).
const PLAIN: &str = "--plain";
const OPTION: &str = "--qa";
/// The GTK window's default size (`src/ui/window`), so `window_width` means the same.
const SIZE: (f32, f32) = (1200.0, 760.0);
/// The longest the idle loop sleeps without looking for a command again.
const IDLE_WAIT: Duration = Duration::from_secs(1);

/// Runs the QA mode when the arguments ask for it. `None` for any other command line.
pub(crate) fn launch(arguments: &[OsString]) -> Option<ExitCode> {
    if arguments.get(1).is_none_or(|option| option != OPTION) {
        return None;
    }
    let [_, _, directory] = arguments else {
        eprintln!("{USAGE}");
        return Some(ExitCode::from(2));
    };
    let served = if directory == PORTAL && session::wanted() {
        session::serve_portal()
    } else if directory == PORTAL {
        serve_portal()
    } else if session::wanted() {
        session::serve(directory)
    } else {
        serve(directory)
    };
    Some(match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("strata-rune: {error}");
            ExitCode::FAILURE
        }
    })
}

fn serve(directory: &OsStr) -> Result<(), String> {
    // Before anything can log there: stdout is the replies' from here on.
    let protocol = strata_qa::reply_stream()?;
    criticals::capture();
    let settings = startup::initialize_process();
    let initial = initial(directory, &settings)?;
    let primary = claim()?;
    launch::leave_launch_folder();
    let log = InputLog::from_environment()?;
    let ui = startup::UiThread::claim()?;
    application::attach(primary);
    let mut host = Offscreen::sized(ui, settings, SIZE).without_animations();
    host.app_mut().start(initial);
    let backend = Rune::new(host, log);
    let commands = strata_qa::command_lines();
    block_on(&backend, strata_qa::serve(&backend, protocol, commands));
    backend.flush_at_exit();
    Ok(())
}

/// What the window opens: the folder the argument names, or for `--plain` what a plain launch
/// opens.
pub(super) fn initial(directory: &OsStr, settings: &Saved) -> Result<launch::Initial, String> {
    if directory == PLAIN {
        return Ok(launch::initial_for(Opening::Plain, settings));
    }
    launch::location(Some(directory))
        .map(launch::Initial::Explicit)
        .ok_or_else(|| "the argument is not a location".to_owned())
}

/// This run's process as the primary on the runner's session bus, where a `launch` step's
/// `strata-rune` hands its arguments to it; a run with no bus runs alone.
fn claim() -> Result<strata_platform::single_instance::Primary, String> {
    match application::claim(Launch::default(), true) {
        Claim::Primary(primary) | Claim::Standalone(primary, _) => Ok(primary),
        Claim::Forwarded(_) => {
            Err("another strata-rune is the primary on the run's session bus".to_owned())
        }
    }
}

/// Serves the portal chooser with no window until a request opens one.
fn serve_portal() -> Result<(), String> {
    let protocol = strata_qa::reply_stream()?;
    criticals::capture();
    let _settings = startup::initialize_process();
    launch::leave_launch_folder();
    let log = InputLog::from_environment()?;
    let host = portal::PortalStaged::start(startup::UiThread::claim()?, log)?;
    let backend = Rune::on(host);
    let commands = strata_qa::command_lines();
    block_on(&backend, strata_qa::serve(&backend, protocol, commands));
    Ok(())
}

/// Wakes the interface thread out of its wait for GLib, from the thread that reads stdin.
struct Awake(AtomicBool);

impl Wake for Awake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.store(true, Ordering::Release);
        glib::MainContext::default().wakeup();
    }
}

/// Runs the session on this thread, which owns the host. A step turns the host itself while
/// it waits; between steps this loop does, so the application keeps running as a window's
/// does while the runner works on files. Ends when the session does or the backend quits.
fn block_on<H: host::Host>(backend: &Rune<H>, session: impl Future<Output = ()>) {
    let awake = Arc::new(Awake(AtomicBool::new(true)));
    let waker = Waker::from(awake.clone());
    let mut context = Context::from_waker(&waker);
    let mut session = pin!(session);
    while !backend.quitting() {
        if awake.0.swap(false, Ordering::AcqRel) {
            if let Poll::Ready(()) = session.as_mut().poll(&mut context) {
                return;
            }
        } else if !backend.idle_turn() && !awake.0.load(Ordering::Acquire) {
            // A wake-up that lands before the wait begins ends it at once: GLib keeps it.
            backend.pause(IDLE_WAIT);
        }
    }
}

#[cfg(test)]
mod tests;
