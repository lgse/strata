// SPDX-License-Identifier: MIT

//! The sidebar's pin gaps as the QA backend presents them. GTK's sidebar puts an empty group
//! named "PINNED" in each insertion gap between the pins (`append_gap` in
//! `src/ui/window/pinned_drop.rs`), and the scenarios aim drags at them by that name and
//! index. strata-components' C8 keeps its gaps out of the accessibility tree on purpose, so
//! the snapshot presents each drawn gap as GTK's group, by the coordinator's ruling
//! (`docs/rune/parity/packages/A3.md`, "Open"), from the elements C8 keys `sidebar-gap-<i>`.

use rune::ElementInfo;
use strata_core::i18n::tr;
use strata_qa::Rect;

use super::tree::{Seen, Snapshot};

/// The key C8 gives gap `i` under the sidebar's key (`crate::sidebar`).
const GAP: &str = "sidebar-gap-";

/// The pin gaps the sidebar draws, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Gaps(Vec<Rect>);

impl Gaps {
    /// The gaps among `elements`, by their index in the sidebar.
    pub(super) fn drawn(elements: &[ElementInfo]) -> Self {
        let mut gaps: Vec<(usize, Rect)> = elements
            .iter()
            .filter(|element| !element.exiting)
            .filter_map(|element| {
                let index = element.key.as_deref()?.strip_prefix(GAP)?.parse().ok()?;
                let [x, y, width, height] = element.rect.map(f64::from);
                Some((
                    index,
                    Rect {
                        x,
                        y,
                        width,
                        height,
                    },
                ))
            })
            .collect();
        gaps.sort_by_key(|(index, _)| *index);
        Self(gaps.into_iter().map(|(_, rect)| rect).collect())
    }

    /// Presents each gap under the window as GTK's group named "PINNED".
    pub(super) fn present(&self, snapshot: &mut Snapshot) {
        let name = tr("PINNED");
        for (index, bounds) in self.0.iter().enumerate() {
            let seen = Seen {
                role: "group".to_owned(),
                name: name.clone(),
                bounds: Some(*bounds),
                ..Seen::default()
            };
            snapshot.push_under_window(&format!("@{GAP}{index}"), seen);
        }
    }
}
