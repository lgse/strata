// SPDX-License-Identifier: MIT

/// A cancellable directory load. Dropping it cancels any unfinished provider work.
pub struct LoadHandle {
    cancel: Option<Box<dyn FnOnce()>>,
}

impl LoadHandle {
    pub fn new(cancel: impl FnOnce() + 'static) -> Self {
        Self {
            cancel: Some(Box::new(cancel)),
        }
    }
}

impl Drop for LoadHandle {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}
