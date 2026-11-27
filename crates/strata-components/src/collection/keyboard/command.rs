// SPDX-License-Identifier: MIT

use crate::collection::selection::Extend;
use crate::contract::Direction;

/// Up or down, for paging.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Vertical {
    /// Towards the first entry.
    Up,
    /// Towards the last entry.
    Down,
}

/// Where a move goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// The neighbouring entry in this direction, as the geometry lays entries out.
    Step(Direction),
    /// One page, by the geometry's `page_step`.
    Page(Vertical),
    /// Half a page, at least one entry: 10xer Ctrl+U and Ctrl+D.
    HalfPage(Vertical),
    /// The first entry.
    First,
    /// The last entry.
    Last,
}

/// Which kind of 10xer range `v` or `V` starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RangeKind {
    /// `v`: the range adds to the selection as it was when the range began.
    Select,
    /// `V`: the range removes from it.
    Unset,
}

impl RangeKind {
    pub(super) fn extend(self) -> Extend {
        match self {
            RangeKind::Select => Extend::Add,
            RangeKind::Unset => Extend::Remove,
        }
    }
}

/// What a bound key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Command {
    /// Move the cursor and select only its entry; the anchor follows.
    Select(Target),
    /// Select the range from the anchor to the target (Shift).
    Extend(Target),
    /// Move the cursor and keep the selection; an active 10xer range follows it.
    Cursor(Target),
    /// Move the cursor one step and keep the selection: Ctrl+Left and Ctrl+Right in the default map.
    Nudge(Direction),
    /// Flip the cursor's entry in or out of the selection: Ctrl+Space.
    Toggle,
    /// Flip the cursor's entry, then move the cursor to the next entry: 10xer Space.
    ToggleAdvance,
    /// Select every entry: Ctrl+A.
    SelectAll,
    /// Select exactly the entries that weren't: 10xer Ctrl+R.
    Invert,
    /// Start a 10xer range at the cursor, or end it when one of this kind is active: `v`, `V`.
    Range(RangeKind),
    /// Select nothing, keeping the cursor: Escape in the default map.
    Clear,
    /// Open the cursor's entry: Enter, 10xer `o`.
    Activate,
    /// Take the key and do nothing: 10xer Shift+arrows, so nothing else hears them.
    Swallow,
}
