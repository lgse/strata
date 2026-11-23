// SPDX-License-Identifier: MIT

//! [`ColumnsIntent`]: what the view tells the host, which applies Strata's policy.

use crate::collection::element::Intent;
use crate::contract::{Dropped, EntryId, Rect};

/// What the view asks of the host, or tells it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ColumnsIntent {
    /// An intent of column `depth`'s collection.
    Collection {
        /// The column.
        depth: usize,
        /// What its controllers asked.
        intent: Intent,
    },
    /// Column `depth`'s cursor moved, after an input moved it: the preview and mirroring follow it.
    Cursor {
        /// The column.
        depth: usize,
        /// The cursor's entry now.
        id: Option<EntryId>,
    },
    /// Column `depth`'s selection changed, after an input changed it.
    Selection {
        /// The column.
        depth: usize,
    },
    /// Right (or `l`) on a folder whose column isn't open.
    EnterFolder {
        /// The column the cursor is in.
        depth: usize,
        /// The folder.
        id: EntryId,
    },
    /// A key made column `depth` the active one and changed nothing else the host hears of.
    Active {
        /// The column that now holds the keyboard cursor.
        depth: usize,
    },
    /// Left (or `h`) in the root column: the host moves on (the sidebar, the parent folder).
    LeftOfRoot,
    /// Close the columns from `depth` on: the X, Escape with nothing selected, a click on empty space in a column before them.
    CloseFrom(usize),
    /// A secondary press on column `depth`'s title or blank header space.
    HeaderMenu {
        /// The column.
        depth: usize,
        /// Where to open it.
        anchor: Rect,
    },
    /// A resize of a column's edge ended: the width, in base points (at the default text size), that new columns open at.
    Width(f32),
    /// A drop on column `depth`: on the folder `target`, or its background (`None`, the folder the column shows).
    Dropped {
        /// The column.
        depth: usize,
        /// The folder row, if any.
        target: Option<EntryId>,
        /// What was dropped.
        dropped: Dropped,
    },
}
