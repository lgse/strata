// SPDX-License-Identifier: MIT

//! Silent real-time paced fallback backend for headless environments and post-disconnect recovery.

use std::time::{Duration, Instant};

use super::{BackendKind, PcmSpec, backend::AudioBackend};

/// Maximum catch-up lag before re-anchoring the pacing deadline after a pause or stall.
const REANCHOR_LAG: Duration = Duration::from_millis(100);

/// Software pacing sink that drains PCM at the stream's exact sample rate without audio hardware.
pub(super) struct SilentBackend {
    spec: PcmSpec,
    next_deadline: Option<Instant>,
}

impl SilentBackend {
    pub(super) fn new(spec: PcmSpec) -> Self {
        Self {
            spec,
            next_deadline: None,
        }
    }
}

impl AudioBackend for SilentBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Silent
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), ()> {
        let frames = (bytes.len() / self.spec.frame_bytes().max(1)) as u64;
        if frames == 0 {
            return Ok(());
        }
        let duration = self.spec.frames_to_duration(frames);
        let now = Instant::now();
        let base = self
            .next_deadline
            .filter(|&deadline| now.saturating_duration_since(deadline) < REANCHOR_LAG)
            .unwrap_or(now);
        let target = base + duration;
        if let Some(wait) = target.checked_duration_since(now) {
            std::thread::sleep(wait);
        }
        self.next_deadline = Some(target);
        Ok(())
    }

    fn latency_us(&mut self) -> Option<u64> {
        Some(0)
    }

    fn flush(&mut self) -> Result<(), ()> {
        self.next_deadline = None;
        Ok(())
    }
}
