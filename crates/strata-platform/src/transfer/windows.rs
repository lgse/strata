// SPDX-License-Identifier: MIT

use std::sync::{Mutex, MutexGuard, PoisonError};

use rune::WindowHandle;

/// What the list needs of a window: [`WindowHandle`], or a test's stand-in.
pub(crate) trait Window: Clone + Send + 'static {
    fn key(&self) -> &str;
    /// Whether its native window exists. On the UI thread.
    fn is_open(&self) -> bool;
}

impl Window for WindowHandle {
    fn key(&self) -> &str {
        WindowHandle::key(self)
    }

    fn is_open(&self) -> bool {
        self.with_raw_handle(|_| ()).is_ok()
    }
}

pub(crate) struct Windows<W: Window = WindowHandle> {
    entries: Mutex<Vec<W>>,
}

impl<W: Window> Default for Windows<W> {
    fn default() -> Self {
        Windows {
            entries: Mutex::new(Vec::new()),
        }
    }
}

impl<W: Window> Windows<W> {
    fn lock(&self) -> MutexGuard<'_, Vec<W>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn attach(&self, window: W) {
        {
            let mut entries = self.lock();
            entries.retain(|w| w.key() != window.key());
            entries.push(window);
        }
        self.prune();
    }

    /// Drops the window keyed `key`: it closed. True if it was attached.
    #[cfg_attr(
        not(any(test, feature = "foreign-parent")),
        expect(dead_code, reason = "P4 detaches a window as it closes")
    )]
    pub(crate) fn detach(&self, key: &str) -> bool {
        let mut entries = self.lock();
        let before = entries.len();
        entries.retain(|w| w.key() != key);
        entries.len() != before
    }

    /// The windows currently tracked.
    pub(crate) fn len(&self) -> usize {
        self.lock().len()
    }

    /// The key of the first window that matches. On the UI thread, if `f` needs it.
    pub(crate) fn find(&self, f: impl Fn(&W) -> bool) -> Option<String> {
        self.lock()
            .iter()
            .find(|w| f(w))
            .map(|w| w.key().to_owned())
    }

    pub(crate) fn prune(&self) {
        self.lock().retain(Window::is_open);
    }
}

#[cfg(test)]
mod tests;
