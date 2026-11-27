// SPDX-License-Identifier: MIT

//! Node paths, kept handles across browser replacements, and scroll limits for [`super::tree`].

use std::rc::Rc;

use strata_components::{collection::element::row_key, contract::EntryId};
use strata_qa::{Orientation, Scroll};

/// Less than this far to go is nowhere: Rune's tree rounds to a tenth of a point.
pub(super) const SCROLLS_AT_LEAST: f64 = 0.5;

pub(super) fn entry_id_of(key: &str) -> Option<EntryId> {
    let digits = key.trim_start_matches(|c: char| !c.is_ascii_digit());
    let id = digits.parse::<u64>().ok()?;
    let entry = EntryId::new(id);
    (row_key(entry) == key).then_some(entry)
}

pub(super) fn is_entry_key(key: &str) -> bool {
    entry_id_of(key).is_some()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Node(pub(super) Rc<str>);

impl Node {
    /// Whether this node is `root` or stands under it.
    pub(super) fn is_within(&self, root: &Node) -> bool {
        self.0
            .strip_prefix(&*root.0)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }
}

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

    pub(super) fn bar(&self) -> Option<f64> {
        [1, 0]
            .into_iter()
            .find(|axis| self.scrolls[*axis])
            .map(|axis| self.offset[axis])
    }
}
