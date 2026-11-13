// SPDX-License-Identifier: MIT

//! The scripted QA mode: `strata-rune --qa <directory>` speaks the protocol of `strata-qa`
//! on stdin and stdout, as the GTK application's `strata --qa` does.
//!
//! `strata-qa` runs the session and decides what a step, a target and a predicate mean. This
//! module answers its `Backend` from Rune's accessibility tree and sends input at Rune's
//! input layer, on the offscreen host: it needs no display server.
//! `docs/rune/slice/V7-rune-qa-backend.md` describes it.

mod backend;
mod compose;
mod field;
mod input_log;
mod keyboard;
mod keymap;
mod keys;
mod roles;
mod sidebar;
mod stage;
mod tree;

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

use self::{backend::Rune, input_log::InputLog};
use crate::{host::offscreen::Offscreen, launch, startup};

/// What a wrong `--qa` command line prints. The release workflow fails a release binary that
/// holds the `usage: strata --qa` prefix, and the QA job requires it in a QA build of the GTK
/// application; the same prefix here lets both checks cover this binary once they are pointed
/// at it. It names `strata` because that is what this interface's binary is called at parity.
const USAGE: &str = "usage: strata --qa <directory>";
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
    Some(match serve(directory) {
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
    let location = launch::location(Some(directory)).ok_or("the argument is not a location")?;
    launch::leave_launch_folder();
    let log = InputLog::from_environment()?;
    let host = Offscreen::sized(startup::UiThread::claim()?, settings, SIZE).without_animations();
    host.app().open(location);
    let backend = Rune::new(host, log);
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
fn block_on(backend: &Rune, session: impl Future<Output = ()>) {
    let awake = Arc::new(Awake(AtomicBool::new(true)));
    let waker = Waker::from(awake.clone());
    let mut context = Context::from_waker(&waker);
    let mut session = pin!(session);
    while !backend.quitting() {
        if awake.0.swap(false, Ordering::AcqRel) {
            if let Poll::Ready(()) = session.as_mut().poll(&mut context) {
                return;
            }
        } else if !backend.turn() && !awake.0.load(Ordering::Acquire) {
            // A wake-up that lands before the wait begins ends it at once: GLib keeps it.
            backend.wait(IDLE_WAIT);
        }
    }
}

#[cfg(test)]
mod tests;
