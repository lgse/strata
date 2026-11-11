// SPDX-License-Identifier: MIT

//! The helper thread: it waits for what the context waits for, and says when that came.

use std::io::{self, Read as _, Write as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The helper thread's name, for debuggers and thread listings.
pub const THREAD_NAME: &str = "strata-glib-loop";

/// What a context waits for: `g_main_context_query`'s answer.
pub(super) struct Wait {
    pub(super) fds: Vec<glib::ffi::GPollFD>,
    /// Milliseconds; negative waits for the descriptors alone.
    pub(super) timeout: i32,
}

#[derive(Default)]
struct Asked {
    /// The newest wait, until the thread takes it.
    wait: Option<Wait>,
    /// Counts the waits asked, so the thread can tell that one it is serving went stale.
    asked: u64,
    stop: bool,
}

#[derive(Default)]
struct Shared {
    asked: Mutex<Asked>,
    changed: Condvar,
}

/// Where a wait the thread is serving stands.
enum Standing {
    Current,
    Replaced,
    Stopped,
}

impl Shared {
    /// Nothing behind the lock can be left half-written, so a poisoned lock is still good.
    fn lock(&self) -> MutexGuard<'_, Asked> {
        self.asked.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Blocks until the interface thread asks for a wait, and returns it with its number.
    /// `None` once the loop ends.
    fn next_wait(&self) -> Option<(Wait, u64)> {
        let mut asked = self.lock();
        loop {
            if asked.stop {
                return None;
            }
            if let Some(wait) = asked.wait.take() {
                return Some((wait, asked.asked));
            }
            asked = self
                .changed
                .wait(asked)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn standing_of(&self, serving: u64) -> Standing {
        let asked = self.lock();
        if asked.stop {
            Standing::Stopped
        } else if asked.asked == serving {
            Standing::Current
        } else {
            Standing::Replaced
        }
    }
}

pub(super) struct Waiter {
    shared: Arc<Shared>,
    /// Written to end the thread's wait early: a new wait, or the end.
    interrupt: UnixStream,
    thread: Option<JoinHandle<()>>,
}

impl Waiter {
    pub(super) fn start(notify: Box<dyn Fn() + Send>) -> io::Result<Self> {
        let (interrupt, interrupted) = UnixStream::pair()?;
        interrupt.set_nonblocking(true)?;
        interrupted.set_nonblocking(true)?;
        let shared = Arc::new(Shared::default());
        let thread = std::thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn({
                let shared = shared.clone();
                move || run(&shared, interrupted, &*notify)
            })?;
        Ok(Self {
            shared,
            interrupt,
            thread: Some(thread),
        })
    }

    /// Replaces what the thread waits for.
    pub(super) fn wait_for(&self, wait: Wait) {
        {
            let mut asked = self.shared.lock();
            asked.wait = Some(wait);
            asked.asked += 1;
        }
        self.interrupt_wait();
    }

    fn interrupt_wait(&self) {
        self.shared.changed.notify_one();
        // A full socket already holds an interruption the thread has yet to read.
        let _ = (&self.interrupt).write(&[1]);
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.interrupt_wait();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// How the thread's wait for one answer of the context ended.
enum Served {
    /// What the context waits for came.
    Came,
    /// The interface thread asked again first, so it has dispatched since.
    Replaced,
    Stopped,
}

fn run(shared: &Shared, mut interrupted: UnixStream, notify: &dyn Fn()) {
    while let Some((wait, serving)) = shared.next_wait() {
        match serve(shared, wait, serving, &mut interrupted) {
            Served::Came => notify(),
            Served::Replaced => {}
            Served::Stopped => return,
        }
    }
}

/// Polls for `wait`, the one numbered `serving`, until it comes, a newer one replaces it, or
/// the loop ends.
fn serve(shared: &Shared, mut wait: Wait, serving: u64, interrupted: &mut UnixStream) -> Served {
    let deadline = u64::try_from(wait.timeout)
        .ok()
        .map(|millis| Instant::now() + Duration::from_millis(millis));
    wait.fds.push(glib::ffi::GPollFD {
        fd: interrupted.as_raw_fd(),
        events: glib::ffi::G_IO_IN as u16,
        revents: 0,
    });
    loop {
        let came = poll(&mut wait.fds, deadline);
        drain(interrupted);
        match shared.standing_of(serving) {
            Standing::Stopped => return Served::Stopped,
            Standing::Replaced => return Served::Replaced,
            Standing::Current if came => return Served::Came,
            // Only an interruption came, written for a wait this one already replaced.
            Standing::Current => {}
        }
    }
}

/// Reads away the interruptions written so far, so the next poll waits again.
fn drain(interrupted: &mut UnixStream) {
    let mut bytes = [0; 64];
    while matches!(interrupted.read(&mut bytes), Ok(read) if read > 0) {}
}

/// Blocks until `deadline` or until one of `fds` is ready. Returns whether what the context
/// waits for came: its deadline, or any descriptor but the last, which is the interruption.
#[expect(
    unsafe_code,
    reason = "the descriptors come from g_main_context_query as GPollFD records, which only \
              g_poll takes; std has no poll"
)]
fn poll(fds: &mut [glib::ffi::GPollFD], deadline: Option<Instant>) -> bool {
    for record in fds.iter_mut() {
        record.revents = 0;
    }
    let left = deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
    // Rounded up, so a wait never ends before its deadline and spins until it.
    let timeout = left.map_or(-1, |left| {
        i32::try_from(left.as_micros().div_ceil(1000)).unwrap_or(i32::MAX)
    });
    // SAFETY: `fds` points at `fds.len()` initialized records, the count passed with it, and
    // `g_poll` only reads their `fd` and `events` and writes their `revents`. A descriptor
    // the other thread has closed since makes poll(2) report it, never touch freed memory.
    let ready = unsafe { glib::ffi::g_poll(fds.as_mut_ptr(), fds.len() as u32, timeout) };
    let context_fds = &fds[..fds.len().saturating_sub(1)];
    match ready {
        0 => true,
        // An error such as EINTR says nothing came; the caller polls again.
        ready if ready < 0 => false,
        _ => context_fds.iter().any(|record| record.revents != 0),
    }
}
