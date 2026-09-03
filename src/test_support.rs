// SPDX-License-Identifier: GPL-3.0-or-later

#![cfg(test)]

use std::{
    io::Write,
    sync::{Arc, LockResult, Mutex, MutexGuard, Once},
};

use tracing_subscriber::fmt::MakeWriter;

pub(crate) struct TestMutex(Mutex<()>);

impl TestMutex {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(()))
    }

    pub(crate) fn lock(&self) -> LockResult<MutexGuard<'_, ()>> {
        match self.0.lock() {
            Ok(guard) => Ok(guard),
            Err(error) => {
                self.0.clear_poison();
                Ok(error.into_inner())
            }
        }
    }
}

/// Serializes tests that drive `glib::MainContext::default()` directly, since it is a
/// process-wide singleton and concurrent access from the test harness's per-test threads panics
/// with a GLib thread-affinity error. A single shared lock, not one static per module: two
/// separate locks each covering only their own module's tests do not prevent a test in one
/// module from racing a test in another, since neither knows about the other's lock. Poisoning is
/// cleared because the mutex protects no state and should not turn one failure into a cascade.
pub(crate) static ASYNC_MAIN_CONTEXT_DEFAULT: TestMutex = TestMutex::new();

#[derive(Clone, Default)]
pub(crate) struct LogWriter(Arc<Mutex<Vec<u8>>>);

pub(crate) struct LogWriterGuard<'a>(MutexGuard<'a, Vec<u8>>);

impl Write for LogWriterGuard<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl<'a> MakeWriter<'a> for LogWriter {
    type Writer = LogWriterGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriterGuard(self.0.lock().unwrap_or_else(|error| error.into_inner()))
    }
}

impl LogWriter {
    fn output(&self) -> String {
        let output = self.0.lock().unwrap_or_else(|error| error.into_inner());
        String::from_utf8_lossy(&output).into_owned()
    }
}

/// Serializes log captures, since each installs its subscriber for one thread but
/// asserts on events the whole binary can emit.
static LOG_CAPTURE: TestMutex = TestMutex::new();

static GLOBAL_SINK: Once = Once::new();

/// Installs a discarding global subscriber, once per test binary.
///
/// `tracing` caches a callsite's interest the first time it is reached, and a
/// callsite first reached while the current thread has no subscriber is cached as
/// uninteresting for every thread. Tests run in parallel, so an unrelated test
/// logging from the same callsite can silence it inside a capture running beside
/// it. Keeping a permissive global subscriber installed means no thread is ever
/// without one, so callsites stay interesting and each event is resolved against
/// whichever subscriber its own thread has.
fn install_global_sink() {
    GLOBAL_SINK.call_once(|| {
        let sink = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(std::io::sink)
            .finish();
        let _installed = tracing::subscriber::set_global_default(sink);
    });
}

/// Runs `action` against a private subscriber and returns everything it logged,
/// so privacy assertions can read the rendered events rather than trusting the
/// call sites.
pub(crate) fn capture_logs(action: impl FnOnce()) -> String {
    install_global_sink();
    let _guard = LOG_CAPTURE.lock();
    let writer = LogWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, action);
    writer.output()
}

/// Finds the rendered event carrying `message`, panicking with the whole
/// capture when it is missing so a failure shows what was logged instead.
pub(crate) fn captured_event<'a>(output: &'a str, message: &str) -> &'a str {
    output
        .lines()
        .find(|line| line.contains(message))
        .unwrap_or_else(|| panic!("missing {message:?} event in:\n{output}"))
}
