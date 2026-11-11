//! How entries pack into Rune's virtual lines, and the header lines of groups.
//!
//! Rune virtualizes vertical rows only, so a grid (C3) packs several entries into each virtual
//! line. The list and each Columns column put one entry on each line ([`OnePerLine`]).
//!
//! # Group header lines
//!
//! ListTable's grouped mode (C4's file-type groups) puts a header line above each group's
//! entries ([`Groups`], `CollectionState::set_groups`). Groups are listing data, so they live
//! in the collection's state beside the selection, set with each listing, rather than in the
//! [`Lines`] rule: the element's builds and handlers and the state's own restores and reveals
//! all read the same value. The element maps lines to entries through prefix sums over the groups:
//! O(log groups) a lookup and nothing per entry. Header lines are keyed `head-{key}`
//! (`keys::header_key`), hold no entry, and the geometry ([`GroupedRows`]) gives presses, the
//! marquee and Page Up and Down the same lines.
//!
//! Every line, header lines included, is the row height tall: Rune's virtual list has one
//! height estimate, and rows it hasn't laid out take it, so a header of any other height would
//! put rows where the index geometry doesn't (Rune draft [U-C1f-1]).
//!
//! The header of the group at the top of the viewport sticks there while its group scrolls,
//! and the next group's header pushes it out. Rune keeps a sticky element only inside its own
//! virtual row, which would put a whole group in one row, so the stuck header is the element's
//! own overlay: a live value moves it as the view scrolls, and the view builds again to
//! relabel it when the top group changes (see the element's Decisions).
//!
//! [U-C1f-1]: crate::collection::element#rune-drafts-not-filed

use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use rune::{Color, TextStyle};
use strata_kit::look::Look;

use crate::contract::{
    CollectionGeometry, Direction, EntrySource, IndexRanges, Point, Rect, UniformRows,
};

#[cfg(test)]
mod tests;

/// How many entries each virtual line holds, and the geometry they make.
///
/// Every line is the element's row height tall, so the geometry's rects must match what the
/// element draws: line `l` spans `[l·h, (l+1)·h)`, and its entries share the width equally,
/// in index order, inside the line's [`Lines::inset`] on each side.
///
/// Used by: C1e ([`OnePerLine`]), C3 (a grid of N per line).
pub trait Lines {
    /// How many entries each line holds in a viewport `width` points wide: at least 1, the
    /// same for every line. The first build has no width yet (0): return 1 or a guess; the
    /// element builds again once the viewport's width gives another count.
    fn per_line(&self, width: f32) -> usize;

    /// The space left and right of a line holding several entries, in points: the entries
    /// share the width inside it, and the geometry must put them there too. 0 by default, and
    /// for lines of one entry, which keep the row's own inset.
    fn inset(&self) -> f32 {
        0.0
    }

    /// The geometry of `len` entries in lines `row_height` tall, in a viewport `width` by
    /// `viewport_height` points, in content coordinates.
    fn geometry(
        &self,
        len: usize,
        row_height: f32,
        width: f32,
        viewport_height: f32,
    ) -> Box<dyn CollectionGeometry>;
}

/// One entry per line: the list and each Columns column.
#[derive(Clone, Copy, Debug, Default)]
pub struct OnePerLine;

impl Lines for OnePerLine {
    fn per_line(&self, _width: f32) -> usize {
        1
    }

    fn geometry(
        &self,
        len: usize,
        row_height: f32,
        width: f32,
        viewport_height: f32,
    ) -> Box<dyn CollectionGeometry> {
        Box::new(UniformRows::new(len, row_height, width).viewport_height(viewport_height))
    }
}

/// How many lines hold `len` entries, `per_line` to a line.
pub(super) fn line_count(len: usize, per_line: usize) -> usize {
    len.div_ceil(per_line.max(1))
}

/// One run of entries under a header line: C4's file-type group ("Images").
///
/// Used by: C4, hosts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Group {
    /// Names the header line (`head-{key}`): unique in the collection, and the same across
    /// regroups for the same group.
    pub key: u64,
    /// What the header says, and what assistive technology reads.
    pub label: Arc<str>,
    /// How many entries it holds, in display order after the group before it.
    pub len: usize,
}

impl Group {
    /// The group keyed `key`, headed `label`, of the next `len` entries.
    pub fn new(key: u64, label: impl Into<Arc<str>>, len: usize) -> Self {
        Self {
            key,
            label: label.into(),
            len,
        }
    }
}

/// The groups a collection draws its entries in, in display order: each a header line, then
/// its entries.
///
/// Build it once per regroup (O(groups)) and give it to the collection's state with the
/// listing (`CollectionState::set_groups`, before `enter` or `sync`); the element reads it at
/// O(log groups) a lookup and nothing per entry. Empty groups are dropped. Groups that don't
/// cover the source exactly (their lengths don't add up to its length) draw it ungrouped.
///
/// Used by: C4, hosts.
#[derive(Clone, Debug, Default)]
pub struct Groups {
    groups: Rc<[Group]>,
    /// Each group's first entry.
    starts: Rc<[usize]>,
    /// Each group's header line.
    heads: Rc<[usize]>,
    len: usize,
    fingerprint: u64,
}

impl Groups {
    /// `groups`, in display order.
    pub fn new(groups: impl IntoIterator<Item = Group>) -> Self {
        let groups: Vec<Group> = groups.into_iter().filter(|g| g.len > 0).collect();
        debug_assert!(
            {
                let mut keys: Vec<u64> = groups.iter().map(|g| g.key).collect();
                keys.sort_unstable();
                keys.windows(2).all(|w| w[0] != w[1])
            },
            "group keys must be unique"
        );
        let mut starts = Vec::with_capacity(groups.len());
        let mut heads = Vec::with_capacity(groups.len());
        let mut len = 0;
        let mut h = std::hash::DefaultHasher::new();
        for (g, group) in groups.iter().enumerate() {
            starts.push(len);
            heads.push(len + g);
            len += group.len;
            group.hash(&mut h);
        }
        Self {
            groups: groups.into(),
            starts: starts.into(),
            heads: heads.into(),
            len,
            fingerprint: h.finish(),
        }
    }

    /// Whether there are no groups (the collection draws ungrouped).
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The groups, in display order.
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// How many entries the groups cover.
    pub fn len(&self) -> usize {
        self.len
    }

    /// The group entry `index` is in.
    pub fn group_of(&self, index: usize) -> Option<usize> {
        (index < self.len).then(|| self.starts.partition_point(|&s| s <= index) - 1)
    }

    /// The entries of group `group`.
    pub fn entries(&self, group: usize) -> Range<usize> {
        let start = self.starts[group];
        start..start + self.groups[group].len
    }

    /// A hash of every group's key, label and length: what keys the lines.
    pub(super) fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// How many lines the groups take: a header each, and an entry a line.
    fn line_count(&self) -> usize {
        self.len + self.groups.len()
    }

    /// The group line `line` is in (its header or one of its entries).
    fn group_at_line(&self, line: usize) -> usize {
        self.heads.partition_point(|&h| h <= line).saturating_sub(1)
    }

    /// What line `line` holds.
    fn line(&self, line: usize) -> Line {
        let g = self.group_at_line(line);
        if line == self.heads[g] {
            Line::Header(g)
        } else {
            let i = line - g - 1;
            Line::Entries(i..i + 1)
        }
    }

    /// The line entry `index` is on.
    fn line_of(&self, index: usize) -> usize {
        index + self.group_of(index).map_or(self.groups.len(), |g| g + 1)
    }

    /// The first entry on or after line `line`; the entry count past the last line.
    fn entry_from(&self, line: usize) -> usize {
        if line >= self.line_count() {
            return self.len;
        }
        match self.line(line) {
            Line::Header(g) => self.starts[g],
            Line::Entries(r) => r.start,
        }
    }

    /// The entries on lines `lines`.
    fn entries_in(&self, lines: Range<usize>) -> Range<usize> {
        let start = self.entry_from(lines.start);
        start..self.entry_from(lines.end).max(start)
    }

    /// How far down from content y `offset` the stuck header covers the content, in lines
    /// `h` tall: a line's height, less what the next group's header has pushed it out by.
    fn stuck_extent(&self, offset: f32, h: f32) -> f32 {
        if self.is_empty() || h <= 0.0 || !h.is_finite() {
            return 0.0;
        }
        let g = self.top_group(offset, h);
        let next = self
            .heads
            .get(g + 1)
            .map_or(f32::INFINITY, |&l| l as f32 * h);
        (next - offset.max(0.0)).clamp(0.0, h)
    }

    /// The group whose lines show at content y `offset`.
    fn top_group(&self, offset: f32, h: f32) -> usize {
        let line = (offset.max(0.0) / h) as usize;
        self.group_at_line(line.min(self.line_count().saturating_sub(1)))
    }
}

/// What a virtual line holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Line {
    /// Group `g`'s header.
    Header(usize),
    /// These entries.
    Entries(Range<usize>),
}

/// How the element's lines map to entries, as one build laid them out: packed `per` to a line,
/// or grouped under header lines (one entry a line).
#[derive(Clone, Debug)]
pub(super) enum LineMap {
    Packed(usize),
    Grouped(Groups),
}

impl LineMap {
    /// The map for `len` entries packed `per` to a line, grouped by `groups` when they cover
    /// them and there is one entry a line.
    pub fn new(per: usize, groups: Option<&Groups>, len: usize) -> Self {
        match groups {
            Some(g) if per <= 1 && !g.is_empty() && g.len() == len => Self::Grouped(g.clone()),
            _ => Self::Packed(per.max(1)),
        }
    }

    /// The groups, when grouped.
    pub fn groups(&self) -> Option<&Groups> {
        match self {
            Self::Grouped(g) => Some(g),
            Self::Packed(_) => None,
        }
    }

    /// Entries a line.
    pub fn per(&self) -> usize {
        match self {
            Self::Packed(per) => *per,
            Self::Grouped(_) => 1,
        }
    }

    /// What keys the lines beyond the listing version and its length.
    pub fn fingerprint(&self) -> u64 {
        self.groups().map_or(0, Groups::fingerprint)
    }

    /// How many lines `len` entries take.
    pub fn count(&self, len: usize) -> usize {
        match self {
            Self::Packed(per) => line_count(len, *per),
            Self::Grouped(g) => g.line_count(),
        }
    }

    /// What line `line` holds, of `len` entries.
    pub fn line(&self, line: usize, len: usize) -> Line {
        match self {
            Self::Packed(per) => {
                let start = line * per;
                Line::Entries(start.min(len)..(start + per).min(len))
            }
            Self::Grouped(g) => g.line(line),
        }
    }

    /// The line entry `index` is on.
    pub fn line_of(&self, index: usize) -> usize {
        match self {
            Self::Packed(per) => index / per,
            Self::Grouped(g) => g.line_of(index),
        }
    }

    /// The entries on lines `lines`, of `len`.
    pub fn entries_in(&self, lines: Range<usize>, len: usize) -> Range<usize> {
        match self {
            Self::Packed(per) => (lines.start * per).min(len)..(lines.end * per).min(len),
            Self::Grouped(g) => g.entries_in(lines),
        }
    }

    /// Line `line`'s key: its first entry's (`row-{id}`, or `line-{id}` packed), or its
    /// group's header's.
    pub fn line_key(&self, source: &dyn EntrySource, line: usize) -> String {
        match self.line(line, source.len()) {
            Line::Header(g) => {
                super::keys::header_key(self.groups().map_or(0, |gs| gs.groups[g].key))
            }
            Line::Entries(r) => super::keys::line_key(source.id_at(r.start), self.per()),
        }
    }

    /// The key of the line before entry `index`'s, when grouped: what a reveal shows first, so
    /// the entry lands clear of the stuck header. None packed, or for the first line.
    pub fn key_above(&self, source: &dyn EntrySource, index: usize) -> Option<String> {
        let line = self.line_of(index).checked_sub(1)?;
        self.groups()?;
        Some(self.line_key(source, line))
    }

    /// How far down from content y `offset` the stuck header covers the content: 0 ungrouped.
    pub fn stuck_extent(&self, offset: f32, h: f32) -> f32 {
        self.groups().map_or(0.0, |g| g.stuck_extent(offset, h))
    }

    /// The group line `line` is in, when grouped.
    pub fn group_at_line(&self, line: usize) -> Option<usize> {
        self.groups()
            .filter(|g| line < g.line_count())
            .map(|g| g.group_at_line(line))
    }

    /// The group whose lines show at content y `offset`, when grouped.
    pub fn top_group(&self, offset: f32, h: f32) -> Option<usize> {
        self.groups()
            .filter(|_| h > 0.0 && h.is_finite())
            .map(|g| g.top_group(offset, h))
    }
}

/// Grouped entries, one to a line, each group's header line above them, every line
/// `row_height` tall.
///
/// Header lines hold no entry: [`CollectionGeometry::index_at`] there is `None`,
/// [`CollectionGeometry::range_in`] skips them, and Up and Down step over them. Left and Right
/// have no neighbours. [`CollectionGeometry::page_step`] is the whole rows that fit below the
/// stuck header, minus one so a row of overlap stays in view, and at least 1.
///
/// Used by: C1e (a grouped collection's geometry), C4, C21.
#[derive(Clone, Debug)]
pub struct GroupedRows {
    groups: Groups,
    row_height: f32,
    width: f32,
    viewport_height: f32,
}

impl GroupedRows {
    /// `groups` in lines `row_height` by `width`, in a zero viewport.
    pub fn new(groups: Groups, row_height: f32, width: f32) -> Self {
        Self {
            groups,
            row_height,
            width,
            viewport_height: 0.0,
        }
    }

    /// The same rows, in a viewport `height` points tall.
    pub fn viewport_height(mut self, height: f32) -> Self {
        self.viewport_height = height;
        self
    }

    fn drawable(&self) -> bool {
        self.groups.len > 0
            && self.row_height > 0.0
            && self.row_height.is_finite()
            && self.width > 0.0
    }

    /// The line content y `y` falls on, clamped to `0..=lines`, rounded down or up.
    fn line_at(&self, y: f32, up: bool) -> usize {
        let line = f64::from(y) / f64::from(self.row_height);
        let line = if up { line.ceil() } else { line.floor() };
        let lines = self.groups.line_count();
        if line.is_nan() || line <= 0.0 {
            0
        } else if line >= lines as f64 {
            lines
        } else {
            line as usize
        }
    }
}

impl CollectionGeometry for GroupedRows {
    fn rect_of(&self, index: usize) -> Rect {
        let y = self.groups.line_of(index) as f64 * f64::from(self.row_height);
        Rect::new(0.0, y as f32, self.width, self.row_height)
    }

    fn index_at(&self, point: Point) -> Option<usize> {
        let across = point.x >= 0.0 && point.x < self.width;
        if !self.drawable() || !across || point.y.is_nan() || point.y < 0.0 {
            return None;
        }
        let line = (f64::from(point.y) / f64::from(self.row_height)).floor();
        if line >= self.groups.line_count() as f64 {
            return None;
        }
        match self.groups.line(line as usize) {
            Line::Header(_) => None,
            Line::Entries(r) => Some(r.start),
        }
    }

    fn range_in(&self, rect: Rect) -> IndexRanges {
        let r = Rect::from_corners(
            Point::new(rect.x, rect.y),
            Point::new(rect.right(), rect.bottom()),
        );
        if !self.drawable() {
            return IndexRanges::new();
        }
        let across = if r.w == 0.0 {
            r.x >= 0.0 && r.x < self.width
        } else {
            r.x < self.width && r.right() > 0.0
        };
        if !across {
            return IndexRanges::new();
        }
        if r.h == 0.0 {
            return self
                .index_at(Point::new(0.0, r.y))
                .map_or_else(IndexRanges::new, |i| (i..i + 1).into());
        }
        let lines = self.line_at(r.y, false)..self.line_at(r.bottom(), true);
        self.groups.entries_in(lines).into()
    }

    fn neighbor(&self, index: usize, direction: Direction) -> Option<usize> {
        let len = self.groups.len;
        if index >= len {
            return None;
        }
        match direction {
            Direction::Up => index.checked_sub(1),
            Direction::Down => (index + 1 < len).then_some(index + 1),
            Direction::Left | Direction::Right => None,
        }
    }

    fn page_step(&self) -> usize {
        let rows = (f64::from(self.viewport_height) / f64::from(self.row_height)).floor();
        if rows >= 1.0 && rows.is_finite() && self.row_height > 0.0 {
            // One row is the stuck header's, and one stays in view across the jump.
            (rows as usize).saturating_sub(2).max(1)
        } else {
            1
        }
    }
}

/// How header lines and the stuck header draw: the label's font and colour, the divider
/// under it, the stuck header's fill (it covers rows, so it must be opaque), and where the
/// label sits: inset from the left, and above the bottom edge.
///
/// Used by: C4 (its tokens), hosts.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct GroupStyle {
    /// The label's font.
    pub font: TextStyle,
    /// The label's colour.
    pub color: Color,
    /// A hairline under the header, if any.
    pub divider: Option<Color>,
    /// The stuck header's background.
    pub fill: Color,
    /// The label's inset from the left edge.
    pub inset: f32,
    /// The space under the label: it sits at the bottom of its line, over the divider.
    pub bottom: f32,
}

impl GroupStyle {
    /// From the look, after Strata's `.type-group-heading` (`src/style.css`, `upstream/main`
    /// 35ee677ab): small medium text in the muted colour, low in its line over a muted
    /// hairline, at the row content's left edge; the stuck header filled with the base
    /// background.
    pub fn from_look(look: &Look) -> Self {
        let kit = &look.kit;
        Self {
            font: kit.ty.small_medium,
            color: kit.c.text_muted,
            divider: Some(kit.c.border_muted),
            fill: kit.c.bg_base,
            inset: strata_kit::slots::collection::row_inset(look) + look.space.xs,
            bottom: look.space.xs,
        }
    }
}
