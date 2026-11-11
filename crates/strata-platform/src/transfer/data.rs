//! What moves: lazy writers for sources, and the limits on reads.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

type Make = Box<dyn FnOnce() -> Vec<u8> + Send>;

/// One MIME type's data for a source. A lazy writer runs only when a reader asks for its type,
/// once, on a worker thread; its bytes are kept for the life of the source, because readers ask
/// again (Nautilus asked six times for one paste in S2).
#[derive(Clone)]
pub struct Writer(Arc<Inner>);

struct Inner {
    make: Mutex<Option<Make>>,
    bytes: OnceLock<Option<Arc<[u8]>>>,
}

impl Writer {
    /// Produces the bytes when first asked for.
    pub fn lazy(make: impl FnOnce() -> Vec<u8> + Send + 'static) -> Self {
        Writer(Arc::new(Inner {
            make: Mutex::new(Some(Box::new(make))),
            bytes: OnceLock::new(),
        }))
    }

    /// Bytes already at hand.
    pub fn bytes(bytes: impl Into<Arc<[u8]>>) -> Self {
        Writer(Arc::new(Inner {
            make: Mutex::new(None),
            bytes: OnceLock::from(Some(bytes.into())),
        }))
    }

    /// The bytes, running the writer if no one has yet. A second caller waits for the first
    /// run rather than starting its own. None if the writer panicked. Blocks: call it off the
    /// UI thread.
    pub(crate) fn get(&self) -> Option<Arc<[u8]>> {
        self.0
            .bytes
            .get_or_init(|| {
                let make = self
                    .0
                    .make
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()?;
                catch_unwind(AssertUnwindSafe(make)).ok().map(Arc::from)
            })
            .clone()
    }

    /// The bytes if the writer has already run.
    pub(crate) fn ready(&self) -> Option<Option<Arc<[u8]>>> {
        self.0.bytes.get().cloned()
    }
}

/// A source's MIME types, each with its writer, in the order offered.
#[derive(Clone, Default)]
pub struct SourceSpec {
    items: Vec<(String, Writer)>,
}

impl SourceSpec {
    pub fn new() -> Self {
        SourceSpec::default()
    }

    /// Adds a type. A type offered twice keeps its first writer.
    pub fn with(mut self, mime: impl Into<String>, writer: Writer) -> Self {
        let mime = mime.into();
        if self.writer(&mime).is_none() {
            self.items.push((mime, writer));
        }
        self
    }

    pub fn mimes(&self) -> impl Iterator<Item = &str> {
        self.items.iter().map(|(m, _)| m.as_str())
    }

    pub fn writer(&self, mime: &str) -> Option<&Writer> {
        self.items.iter().find(|(m, _)| m == mime).map(|(_, w)| w)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Bounds on one read, so a source can neither exhaust memory nor pin a worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadLimits {
    /// The most bytes taken; more fails the read.
    pub max_bytes: usize,
    /// The longest the source may stay silent, before its first byte or between two.
    pub idle: Duration,
}

impl Default for ReadLimits {
    /// 64 MiB, and 5 s of silence.
    fn default() -> Self {
        ReadLimits {
            max_bytes: 64 << 20,
            idle: Duration::from_secs(5),
        }
    }
}

/// How long a source write may wait for a reader that stopped reading.
pub(crate) const WRITE_IDLE: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests;
