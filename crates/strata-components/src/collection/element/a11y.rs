//! What assistive technology hears: the roles, and the live status line that reads out
//! selection changes a row's own state doesn't say.
//!
//! The cursor's row is the container's active descendant (focus stays on the container). Rune
//! gives each mounted row of a virtual list its place in the set, and the rows group the set's
//! size; grid cells state their own, and so do the rows of a grouped collection, whose header
//! lines would otherwise count. A row's selected state comes from `focus_selected`.
//!
//! A group's header line is a heading named by its label, a sibling of the rows (Rune can't
//! make rows of a virtual list members of a header: Rune draft [U-C1f-3]). It carries Rune's
//! virtual-list index as its position, which Rune gives every virtual row (Rune draft
//! [U-C1f-2]). The stuck copy over the viewport is hidden from assistive technology: the
//! header line itself is in the tree while it is mounted.
//!
//! [U-C1f-2]: crate::collection::element#rune-drafts-not-filed
//! [U-C1f-3]: crate::collection::element#rune-drafts-not-filed

use rune::{El, LiveRegion, el, text};

use super::keys::STATUS;

/// The roles a collection and its rows take.
///
/// Used by: C1e (the list and columns), C3 ([`Roles::GRID_CELLS`]), C4 ([`Roles::GRID_ROWS`]).
///
/// Fields may be added: start from a constant or [`Roles::new`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Roles {
    /// The container's role.
    pub container: &'static str,
    /// A virtual line's role, when a line holds several entries.
    pub line: Option<&'static str>,
    /// Each entry's role.
    pub item: &'static str,
    /// A group's header line's role (`CollectionState::set_groups`).
    pub header: &'static str,
}

impl Roles {
    /// A container of `item`s, `line` grouping a line of several, `header` heading a group.
    pub const fn new(
        container: &'static str,
        line: Option<&'static str>,
        item: &'static str,
        header: &'static str,
    ) -> Self {
        Self {
            container,
            line,
            item,
            header,
        }
    }

    /// A listbox of options: the list and each Columns column.
    pub const LISTBOX: Roles = Roles {
        container: "listbox",
        line: None,
        item: "option",
        header: "heading",
    };

    /// A grid of rows, whose cells the content slot draws (C4).
    pub const GRID_ROWS: Roles = Roles {
        container: "grid",
        line: None,
        item: "row",
        header: "heading",
    };

    /// A grid of rows of cells, one entry per cell (C3).
    pub const GRID_CELLS: Roles = Roles {
        container: "grid",
        line: Some("row"),
        item: "gridcell",
        header: "heading",
    };
}

impl Default for Roles {
    fn default() -> Self {
        Self::LISTBOX
    }
}

/// What changed, for the status line.
///
/// Fields may be added.
///
/// Used by: hosts (their wording, through the element's `announce`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Announcement {
    /// How many entries are selected now.
    pub selected: usize,
    /// How many entries there are.
    pub total: usize,
}

/// The default wording: a count once more than one entry is selected (one selected entry
/// says so itself, through its row's selected state), else nothing. It leaves out the total,
/// which changes with the listing and would re-announce an unchanged selection.
pub fn default_announcement(a: &Announcement) -> String {
    if a.selected > 1 {
        format!("{} items selected", a.selected)
    } else {
        String::new()
    }
}

/// The status line: a polite live region, kept in the layout but one point square, so it
/// draws nothing a person sees.
pub(super) fn status(line: &str) -> El {
    el().pin(Some(0.0), Some(0.0), None, None)
        .w(1.0)
        .h(1.0)
        .clip_soft(0.0)
        .child(
            text(line.to_owned())
                .key(STATUS)
                .live_region(LiveRegion::Polite),
        )
}
