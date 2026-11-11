//! [`ColumnsIntent`]: what the view tells the host, which applies Strata's policy.

use crate::collection::element::Intent;
use crate::contract::{Dropped, EntryId, Rect};

/// What the view asks of the host, or tells it. The view moves the keyboard cursor between
/// columns, reveals the active one and draws the open path itself; opening, closing and
/// navigating are the host's, which then calls [`ColumnsState`](super::ColumnsState)'s
/// lifecycle methods.
///
/// Variants may be added.
///
/// Used by: hosts.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ColumnsIntent {
    /// An intent of column `depth`'s collection: open, a click (previews follow it), open in a
    /// new tab, a file drag, a rename, a context menu, an edge going up or down, anything else.
    /// Not sent: Left and Right edges (the view moves between columns) and the activation of
    /// the folder whose column is already open (the view focuses and reveals that column,
    /// `browser/view-modes/columns`).
    Collection {
        /// The column.
        depth: usize,
        /// What its controllers asked.
        intent: Intent,
    },
    /// Column `depth`'s cursor moved, after an input moved it: the preview and mirroring
    /// follow it (Strata's policy, with its 75 ms wait).
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
    /// Right (or `l`) on a folder whose column isn't open: open it as column `depth + 1`, with
    /// the keyboard cursor on its first entry
    /// ([`Focus::FirstEntry`](super::Focus::FirstEntry)).
    EnterFolder {
        /// The column the cursor is in.
        depth: usize,
        /// The folder.
        id: EntryId,
    },
    /// Left (or `h`) in the root column: the host moves on (the sidebar, the parent folder).
    LeftOfRoot,
    /// Close the columns from `depth` on: the X, Escape with nothing selected, a click on
    /// empty space in a column before them.
    CloseFrom(usize),
    /// A secondary press on column `depth`'s title or blank header space: its folder's menu,
    /// the same as its empty area's, at `anchor` (window points).
    HeaderMenu {
        /// The column.
        depth: usize,
        /// Where to open it.
        anchor: Rect,
    },
    /// A resize of a column's edge ended: the width, in base points (at the default text
    /// size), that new columns open at. The host saves it (`browser_column_width`, or
    /// `chooser_column_width` in the chooser).
    Width(f32),
    /// A drop on column `depth`: on the folder `target`, or its background (`None`, the folder
    /// the column shows).
    Dropped {
        /// The column.
        depth: usize,
        /// The folder row, if any.
        target: Option<EntryId>,
        /// What was dropped.
        dropped: Dropped,
    },
}
