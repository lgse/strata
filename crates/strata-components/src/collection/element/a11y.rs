// SPDX-License-Identifier: MIT

use rune::{El, LiveRegion, el, text};

use super::keys::STATUS;

/// The roles a collection and its rows take.
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

    /// A list of items, heard as AT-SPI's `list` and `list item`: C4, as native List view is a `GtkListView`.
    pub const LIST: Roles = Roles {
        container: "list",
        line: None,
        item: "listitem",
        header: "heading",
    };

    /// A grid of rows, whose cells the content slot draws.
    pub const GRID_ROWS: Roles = Roles {
        container: "grid",
        line: None,
        item: "row",
        header: "heading",
    };

    /// A grid of rows of cells, one entry per cell.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Announcement {
    /// How many entries are selected now.
    pub selected: usize,
    /// How many entries there are.
    pub total: usize,
}

/// The default wording: a count once more than one entry is selected (one selected entry says so itself.
pub fn default_announcement(a: &Announcement) -> String {
    if a.selected > 1 {
        format!("{} items selected", a.selected)
    } else {
        String::new()
    }
}

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
