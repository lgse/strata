//! [`ListTableState`]: what the table keeps between frames, in the host's state.

use std::cell::Cell;

use super::column::ColumnId;
use super::layout::DropSite;
use crate::contract::Rect;
use crate::scroll::{Bar, BarMode, ScrollFeed};

/// Where the columns menu is open.
///
/// Used by: C4, hosts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuAt {
    /// Under the Columns button ([`super::ListTable::columns_button`]).
    Button,
    /// Over the headings: at the pointer after a right-click on them, or under their right end
    /// (Strata's Appearance → "Customize columns…").
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

/// The table's own state, kept by the host for the life of the view and handed to the table
/// by accessor: where the table shows, a heading drag or resize in progress, and where the
/// columns menu is open.
///
/// Used by: hosts.
#[derive(Debug, Default)]
pub struct ListTableState {
    /// The width the table has, as last laid out.
    pub(super) width: f32,
    /// The heading strip's rect in window points, as last laid out.
    pub(super) strip: Rect,
    pub(super) drag: Option<HeadingDrag>,
    /// The press that follows a reorder drag's release on the same heading, in the same frame,
    /// isn't a sort click (Rune draft R-C4-1). Set by the release, taken by that press, and
    /// cleared by the next build whether or not one came: a cell, as the build reads the
    /// state shared.
    pub(super) suppress_press: Cell<bool>,
    pub(super) resize: Option<Resize>,
    pub(super) menu: Option<MenuAt>,
    /// The rows' scroll bar (C24) and where the rows stand.
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

    /// Drops a heading drag or resize in progress, as Strata does when its headings unmap, and
    /// lets go of both scroll bars (C24's `Bar::release`: a thumb drag cut off, or the pointer
    /// over a bar). Call it when the table leaves the view (a view switch) mid-gesture: Rune
    /// tells an element removed under a drag nothing (Rune draft R-C4-2).
    pub fn reset(&mut self) {
        self.drag = None;
        self.resize = None;
        self.suppress_press.set(false);
        self.bar.release();
        self.sideways_bar.release();
    }

    /// The end of the frame's input, as the view builds: a sort click a reorder drag's release
    /// held back has had its frame to come.
    pub(super) fn end_frame(&self) {
        self.suppress_press.set(false);
    }

    /// Opens the columns menu at `at`: [`MenuAt::Headings`]`(None)` is Strata's "Customize
    /// columns…".
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

    /// Shows `column`'s heading as dragged past the reorder threshold, dropping at `site`, as
    /// if the person were dragging it: for gallery scenes and tests that draw a drag in
    /// progress. The next press on a heading replaces it.
    pub fn show_reorder(&mut self, column: ColumnId, site: Option<DropSite>) {
        self.drag = Some(HeadingDrag {
            column,
            active: true,
            site,
        });
    }

    /// Where the rows stand: give it to C24's `AutoscrollPointer::new` for the collection.
    pub fn feed(&self) -> &ScrollFeed {
        &self.feed
    }

    /// Where the sideways scroll stands (see [`super::ListTable::sideways_target`]).
    pub fn sideways_feed(&self) -> &ScrollFeed {
        &self.sideways_feed
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
