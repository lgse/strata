// SPDX-License-Identifier: MIT

use std::{marker::PhantomData, ops::Deref};

use super::{next_source_id_in, source_at};

pub struct PrivateContext {
    context: glib::MainContext,
    _owning_thread: PhantomData<*const ()>,
}

impl Default for PrivateContext {
    fn default() -> Self {
        Self::new()
    }
}

impl PrivateContext {
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place a test creates a private context"
    )]
    pub fn new() -> Self {
        Self {
            context: glib::MainContext::new(),
            _owning_thread: PhantomData,
        }
    }

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
    fn drop(&mut self) {
        let mut first = 1;
        while let Some(next) = self.destroy_sources_from(first) {
            first = next;
        }
    }
}
