// SPDX-License-Identifier: MIT

//! [`ListTableState`]: what the table keeps between frames, in the host's state.

use super::column::ColumnId;
use super::layout::DropSite;
use crate::contract::Rect;
use crate::scroll::{Bar, BarMode, ScrollFeed};

/// Where the columns menu is open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuAt {
    /// Under the Columns button ([`super::ListTable::columns_button`]).
    Button,
    /// Over the headings: at the pointer after a right-click on them, or under their right end.
    Headings(Option<(f32, f32)>),
}

/// A heading pressed, and whether it has become a reorder drag.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct HeadingDrag {
    pub column: ColumnId,
    /// Past the reorder threshold.
    pub active: bool,
    /// Where it would drop now, if anywhere.
    pub site: Option<DropSite>,
}

/// A resize in progress: the column, its width at the press and now, in base points.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Resize {
    pub column: ColumnId,
    pub start: f32,
    pub live: f32,
}

/// The table's own state, kept by the host for the life of the view and handed to the table by accessor.
#[derive(Debug, Default)]
pub struct ListTableState {
    /// The width the table has, as last laid out.
    pub(super) width: f32,
    /// The heading strip's rect in window points, as last laid out.
    pub(super) strip: Rect,
    pub(super) drag: Option<HeadingDrag>,
    pub(super) resize: Option<Resize>,
    pub(super) menu: Option<MenuAt>,
    pub(super) bar: Bar,
    pub(super) feed: ScrollFeed,
    /// The sideways scroll's bar and where it stands.
    pub(super) sideways_bar: Bar,
    pub(super) sideways_feed: ScrollFeed,
}

impl ListTableState {
    /// Nothing in progress.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops a heading drag or resize in progress, as Strata does when its headings unmap, and lets go of both scroll bars.
    pub fn reset(&mut self) {
        self.drag = None;
        self.resize = None;
        self.bar.release();
        self.sideways_bar.release();
    }

    /// Opens the columns menu at `at`: `MenuAt::Headings``(None)` is Strata's "Customize columns…".
    pub fn open_columns_menu(&mut self, at: MenuAt) {
        self.menu = Some(at);
    }

    /// Closes the columns menu.
    pub fn close_columns_menu(&mut self) {
        self.menu = None;
    }

    /// Where the columns menu is open, if it is.
    pub fn columns_menu(&self) -> Option<MenuAt> {
        self.menu
    }

    /// The heading being dragged past the reorder threshold, and where it would drop.
    pub fn reordering(&self) -> Option<(&ColumnId, Option<&DropSite>)> {
        self.drag
            .as_ref()
            .filter(|d| d.active)
            .map(|d| (&d.column, d.site.as_ref()))
    }

    /// Shows `column`'s heading as dragged past the reorder threshold, dropping at `site`.
    pub fn show_reorder(&mut self, column: ColumnId, site: Option<DropSite>) {
        self.drag = Some(HeadingDrag {
            column,
            active: true,
            site,
        });
    }

    /// Where the rows stand: give it to `AutoscrollPointer::new` for the collection.
    pub fn feed(&self) -> &ScrollFeed {
        &self.feed
    }

    /// Where the sideways scroll stands (see [`super::ListTable::sideways_target`]).
    pub fn sideways_feed(&self) -> &ScrollFeed {
        &self.sideways_feed
    }

    /// Whether the rows' vertical scrollbar thumb is currently dragged.
    pub fn is_dragging(&self) -> bool {
        self.bar.is_dragging()
    }

    /// Whether the sideways scroll currently overflows, reserving space for the horizontal bar beneath the rows.
    pub fn sideways_overflows(&self) -> bool {
        self.sideways_bar.overflows()
    }

    /// When both scroll bars show: overlay (Strata's, the default) or always.
    pub fn set_bar_mode(&mut self, mode: BarMode) {
        self.bar.set_mode(mode);
        self.sideways_bar.set_mode(mode);
    }

    /// The column being resized and its width now, in base points.
    pub fn resizing(&self) -> Option<(&ColumnId, f32)> {
        self.resize.as_ref().map(|r| (&r.column, r.live))
    }
}
