//! Whether a data-device selection write took.
//!
//! wlroots ignores `wl_data_device.set_selection` with a serial older than the current
//! selection's, silently: no error, no event (S2 Q5). A write that took is echoed to our own
//! device as a `selection` event, at once, while we have keyboard focus. So after a write made
//! with focus, P0 sends `wl_display.sync`: the compositor answers it after handling the write,
//! so a `done` with no `selection` event since means the write was dropped. Without focus no
//! echo comes either way, and the write isn't judged.

use std::collections::VecDeque;

/// The writes waiting for their sync, oldest first, each with whether a `selection` event has
/// answered it. The compositor handles each write and then its sync, in order, so a
/// `selection` event between two syncs' answers belongs to the oldest write still waiting.
#[derive(Debug, Default)]
pub(super) struct Echoes {
    waiting: VecDeque<(u64, bool)>,
    /// The newest writes judged dropped, so a cancel can be told from a loss.
    dropped: VecDeque<u64>,
}

/// How many dropped writes [`Echoes::was_dropped`] remembers.
const KEEP_DROPPED: usize = 64;

impl Echoes {
    /// A write by `source` went out with a sync after it.
    pub(super) fn arm(&mut self, source: u64) {
        self.waiting.push_back((source, false));
    }

    /// The data device heard a `selection` event: the oldest write waiting has its answer.
    pub(super) fn selection(&mut self) {
        // Even a second event before the oldest's sync answers: another client's copy then,
        // which says nothing of the writes after it.
        if let Some((_, seen)) = self.waiting.front_mut() {
            *seen = true;
        }
    }

    /// The sync after `source`'s write came back: true when the write was dropped, as no
    /// `selection` event answered it.
    pub(super) fn done(&mut self, source: u64) -> bool {
        let dropped = match self.waiting.iter().position(|(s, _)| *s == source) {
            Some(i) => self.waiting.remove(i).is_some_and(|(_, seen)| !seen),
            None => false,
        };
        if dropped {
            self.dropped.push_back(source);
            if self.dropped.len() > KEEP_DROPPED {
                self.dropped.pop_front();
            }
        }
        dropped
    }

    /// Whether `source`'s write was judged dropped (among the newest ones).
    pub(super) fn was_dropped(&self, source: u64) -> bool {
        self.dropped.contains(&source)
    }
}

#[cfg(test)]
mod tests;
