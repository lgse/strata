// SPDX-License-Identifier: MIT

//! Node paths, kept handles across browser replacements, and scroll limits for [`super::tree`].

use std::rc::Rc;

use strata_components::{collection::element::row_key, contract::EntryId};
use strata_qa::{Orientation, Scroll};

/// Less than this far to go is nowhere: Rune's tree rounds to a tenth of a point.
pub(super) const SCROLLS_AT_LEAST: f64 = 0.5;

/// Whether `key` is the collection's key for an entry's row ([`row_key`]). A list holds
/// other nodes of an entry's role: the List table's headings are a `row` of its grid too.
pub(super) fn is_entry_key(key: &str) -> bool {
    let digits = key.trim_start_matches(|c: char| !c.is_ascii_digit());
    let id = digits.parse::<u64>();
    id.is_ok_and(|id| row_key(EntryId::new(id)) == key)
}

/// A node of the tree: the keys from the window down to it. An element without a key stands
/// under its place among its siblings. A row's key holds its entry's id, so the path of a
/// row a list has given to another entry is another path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Node(pub(super) Rc<str>);

/// What `remember` holds of a node: its path, and the role it had, so a different element
/// that later stands at the same place is not taken for it; and for an entry's row, the
/// browser's replacements as of then (`qa::replaced`), which the backend fills in.
#[derive(Clone, Debug)]
pub(crate) struct Kept {
    pub(super) node: Node,
    pub(super) role: String,
    pub(super) since: u64,
}

impl Kept {
    /// Kept after the browser's `count`th replacement.
    pub(super) fn after(self, count: u64) -> Self {
        Self {
            since: count,
            ..self
        }
    }

    pub(super) fn since(&self) -> u64 {
        self.since
    }

    pub(super) fn row_key(&self) -> Option<&str> {
        let key = self.node.0.rsplit('/').next()?;
        is_entry_key(key).then_some(key)
    }

    /// The entry whose row this is, read from the row's key.
    pub(super) fn entry(&self) -> Option<EntryId> {
        let key = self.row_key()?;
        let digits = key.trim_start_matches(|c: char| !c.is_ascii_digit());
        digits.parse().ok().map(EntryId::new)
    }
}

/// How far a scroll view has scrolled, whether it can, and how much it shows: along x, then
/// y, in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Scrolled {
    pub(super) offset: [f64; 2],
    pub(super) scrolls: [bool; 2],
    pub(super) page: [f64; 2],
}

impl Scrolled {
    fn axis(along: Orientation) -> usize {
        match along {
            Orientation::Horizontal => 0,
            Orientation::Vertical => 1,
        }
    }

    pub(super) fn along(&self, along: Orientation) -> Scroll {
        let axis = Self::axis(along);
        Scroll {
            value: self.offset[axis],
            page: self.page[axis],
            scrollable: self.scrolls[axis],
        }
    }

    /// Where the scroll bar GTK would show stands: a scrolled window shows the vertical one
    /// when its content is taller than it, else the horizontal one when it is wider, else
    /// none.
    pub(super) fn bar(&self) -> Option<f64> {
        [1, 0]
            .into_iter()
            .find(|axis| self.scrolls[*axis])
            .map(|axis| self.offset[axis])
    }
}
