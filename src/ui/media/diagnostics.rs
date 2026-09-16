// SPDX-License-Identifier: MIT

use std::sync::atomic::{AtomicUsize, Ordering};

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(super) fn live_bytes() -> usize {
    LIVE_BYTES.load(Ordering::Relaxed)
}

pub(super) struct Pixels(Vec<u8>);

impl Pixels {
    pub fn new(pixels: Vec<u8>) -> Self {
        let live_bytes = LIVE_BYTES.fetch_add(pixels.len(), Ordering::Relaxed) + pixels.len();
        tracing::trace!(live_bytes, "decoded texture bytes allocated");
        Self(pixels)
    }
}

impl AsRef<[u8]> for Pixels {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for Pixels {
    fn drop(&mut self) {
        let live_bytes = LIVE_BYTES.fetch_sub(self.0.len(), Ordering::Relaxed) - self.0.len();
        tracing::trace!(live_bytes, "decoded texture bytes released");
    }
}
