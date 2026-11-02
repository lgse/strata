// SPDX-License-Identifier: MIT

use std::collections::VecDeque;

// Samples kept behind the playhead; everything decoded ahead of it is kept too,
// however far the sink's buffer and device delay push that lead.
pub(super) const HISTORY_SAMPLES: usize = 48_000;

/// Mono PCM indexed by samples since the last restart, matching the sink clock.
#[derive(Default)]
pub(crate) struct PcmHistory {
    samples: VecDeque<f32>,
    end: u64,
}

impl PcmHistory {
    pub(super) fn clear(&mut self) {
        self.samples.clear();
        self.end = 0;
    }

    pub(super) fn push(&mut self, interleaved: &[u8]) {
        for frame in interleaved.as_chunks::<4>().0 {
            let left = i16::from_le_bytes([frame[0], frame[1]]);
            let right = i16::from_le_bytes([frame[2], frame[3]]);
            self.samples
                .push_back((f32::from(left) + f32::from(right)) / 65_536.0);
        }
        self.end += (interleaved.len() / 4) as u64;
    }

    pub(super) fn trim_behind(&mut self, played: u64) {
        let keep_from = played.min(self.end).saturating_sub(HISTORY_SAMPLES as u64);
        let start = self.end - self.samples.len() as u64;
        if keep_from > start {
            self.samples.drain(..(keep_from - start) as usize);
        }
    }

    pub(super) fn window_ending_at(&self, played: u64, window: &mut [f32]) -> bool {
        let played = played.min(self.end);
        let start = self.end - self.samples.len() as u64;
        if played <= start {
            return false;
        }
        let available = (played - start) as usize;
        let copied = available.min(window.len());
        let missing = window.len() - copied;
        window[..missing].fill(0.0);
        for (target, source) in window[missing..]
            .iter_mut()
            .zip(self.samples.range(available - copied..available))
        {
            *target = *source;
        }
        true
    }
}
