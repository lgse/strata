// SPDX-License-Identifier: MIT

use std::{marker::PhantomData, ops::Deref};

use gtk::glib;

use super::{next_source_id_in, source_at};

/// A test's own main context, for driving async code off the process-wide default context.
///
/// A GIO call started on the context keeps it alive from its `GTask`, and the pool thread that
/// ran the task may drop that last reference after the test has finished. Finalizing the context
/// there would drop any local task or callback still attached to it on the wrong thread and
/// abort the whole test process (#1963). Dropping this handle therefore destroys every source
/// left on the context first, on the thread that created them.
///
/// Cloning the `glib::MainContext` this derefs to does not keep its sources alive: they are
/// destroyed when this handle drops, whoever still holds a clone.
pub(crate) struct PrivateContext {
    context: glib::MainContext,
    _owning_thread: PhantomData<*const ()>,
}

impl PrivateContext {
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place a test creates a private context"
    )]
    pub(crate) fn new() -> Self {
        Self {
            context: glib::MainContext::new(),
            _owning_thread: PhantomData,
        }
    }

    /// Destroys the sources with ids from `first` up to the next free id, which it returns, or
    /// `None` when there was nothing left to destroy.
    fn destroy_sources_from(&self, first: u32) -> Option<u32> {
        let end = next_source_id_in(&self.context);
        let mut destroyed = false;
        for id in first..end {
            if let Some(source) = source_at(&self.context, id) {
                source.destroy();
                destroyed = true;
            }
        }
        destroyed.then_some(end)
    }
}

impl Deref for PrivateContext {
    type Target = glib::MainContext;

    fn deref(&self) -> &glib::MainContext {
        &self.context
    }
}

impl Drop for PrivateContext {
    /// Repeats until a pass destroys nothing, since dropping a source's future or callback may
    /// attach another one.
    fn drop(&mut self) {
        let mut first = 1;
        while let Some(next) = self.destroy_sources_from(first) {
            first = next;
        }
    }
}
