// SPDX-License-Identifier: MIT

//! Mono PCM sample history synchronized to the audio sink's heard playback position,
//! matching GTK's `PcmHistory` (`src/ui/media/history.rs`).

use std::collections::VecDeque;

/// Samples kept behind the playhead (1 second at 48 kHz); everything decoded ahead of the
/// playhead stays queued too, however far the sink's buffer and device delay push that lead.
pub(super) const HISTORY_SAMPLES: usize = 48_000;

/// Mono PCM indexed by samples since the last restart/flush, matching `PcmOutput::position_frames`.
#[derive(Default)]
pub(super) struct PcmHistory {
    samples: VecDeque<f32>,
    end: u64,
}

impl PcmHistory {
    pub(super) fn clear(&mut self) {
        self.samples.clear();
        self.end = 0;
    }

    /// Appends interleaved 48 kHz stereo S16LE PCM frames downmixed to mono `[-1.0, 1.0]`.
    pub(super) fn push(&mut self, interleaved: &[u8]) {
        for frame in interleaved.as_chunks::<4>().0 {
            let left = i16::from_le_bytes([frame[0], frame[1]]);
            let right = i16::from_le_bytes([frame[2], frame[3]]);
            self.samples
                .push_back((f32::from(left) + f32::from(right)) / 65_536.0);
        }
        self.end += (interleaved.len() / 4) as u64;
    }

    /// Drops samples further than [`HISTORY_SAMPLES`] behind `played`, preserving all unplayed
    /// samples ahead of `played` across stalls.
    pub(super) fn trim_behind(&mut self, played: u64) {
        let keep_from = played.min(self.end).saturating_sub(HISTORY_SAMPLES as u64);
        let start = self.end - self.samples.len() as u64;
        if keep_from > start {
            self.samples.drain(..(keep_from - start) as usize);
        }
    }

    /// Fills `window` with the mono samples ending at `played`. Returns `false` when no samples
    /// are available at or before `played`.
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

#[cfg(test)]
mod tests;
