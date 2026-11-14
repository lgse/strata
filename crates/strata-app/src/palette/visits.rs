// SPDX-License-Identifier: MIT

//! The folders Ctrl+Shift+K jumps to: GTK's `VisitRecorder` and `install_history_recorder`
//! (`src/ui/window/composition/search.rs`), which record each local folder a column of the
//! browser finished listing into strata-fs's [`NavigationHistory`].
//!
//! A column's folder waits from its `ColumnAdded` until its `LoadFinished`, and a failed
//! load leaves it waiting, so a later retry that lists it records the visit. A reset or a
//! truncation forgets what it drops. Only a folder with a local path is recorded.
//!
//! The browser's observer only copies each event ([`Visit::of`]) and queues it; the part
//! records it as the queue is applied, so a hidden tab's visits are recorded when the tab is
//! next shown, where GTK records them as they finish.

use std::path::PathBuf;

use strata_core::model::Location;
use strata_fs::app::BrowserEvent;

use crate::App;

#[cfg(test)]
mod tests;

/// What an event says about the columns' folders, copied out of the browser.
#[derive(Clone, Debug)]
pub(super) enum Visit {
    Reset,
    Truncated(usize),
    Added { depth: usize, location: Location },
    Loaded(usize),
}

impl Visit {
    pub(super) fn of(event: &BrowserEvent) -> Option<Self> {
        Some(match event {
            BrowserEvent::Reset => Self::Reset,
            BrowserEvent::ColumnsTruncated { len, .. } => Self::Truncated(*len),
            BrowserEvent::ColumnAdded { depth, location } => Self::Added {
                depth: *depth,
                location: location.clone(),
            },
            BrowserEvent::LoadFinished { depth, .. } => Self::Loaded(*depth),
            _ => return None,
        })
    }
}

/// Each column's folder that has not finished listing yet, by depth.
#[derive(Default)]
pub(super) struct Visits {
    pending: Vec<Option<Location>>,
}

impl Visits {
    /// The local folder `visit` finished listing, when it did.
    pub(super) fn handle(&mut self, visit: Visit) -> Option<PathBuf> {
        match visit {
            Visit::Reset => self.pending.clear(),
            Visit::Truncated(len) => self.pending.truncate(len),
            Visit::Added { depth, location } => {
                if self.pending.len() <= depth {
                    self.pending.resize(depth + 1, None);
                }
                self.pending[depth] = Some(location);
            }
            Visit::Loaded(depth) => {
                return self
                    .pending
                    .get_mut(depth)
                    .and_then(Option::take)
                    .and_then(|location| location.native_path().map(std::path::Path::to_path_buf));
            }
        }
        None
    }
}

/// The queued `visit`, in this tab: a folder it finished listing goes into the history.
pub(super) fn heard(app: &mut App, visit: Visit) {
    let palette = &mut app.palette;
    if let Some(path) = palette.visits.handle(visit) {
        palette.history.record(&path);
    }
}
