// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::Path,
    rc::Rc,
};

use strata_components::contract::{EntryId, EntrySource};
use strata_core::model::{FileEntry, Location, MetadataValue};
use strata_fs::app::Browser;
use strata_preview::services::{is_image_path, is_media_path};

use crate::listing::Column;

/// What [`Fills`] needs of the browser: [`Browser`] in the application, a recorder in tests.
pub(crate) trait Filler {
    fn entry_at(&self, depth: usize, position: usize) -> Option<FileEntry>;
    /// Asks for the fill of the entry at `(depth, position)`.
    fn request(&self, at: (usize, usize), location: Location, details: bool);
    fn prioritize(&self, depth: usize, near: &[Location]);
    fn touched(&self, location: &Location) -> bool;
}

impl Filler for Rc<Browser> {
    fn entry_at(&self, depth: usize, position: usize) -> Option<FileEntry> {
        Browser::entry_at(self, depth, position)
    }

    fn request(&self, (depth, position): (usize, usize), location: Location, details: bool) {
        self.request_visible_metadata_fill(depth, position, location, details);
    }

    fn prioritize(&self, depth: usize, near: &[Location]) {
        self.prioritize_metadata_fills(depth, near);
    }

    fn touched(&self, location: &Location) -> bool {
        Browser::operation_touches(location)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Details {
    /// Every row: the Icons view's details line shows all three.
    All,
    Folders,
    /// No row: Columns while sizes are hidden.
    Off,
}

/// Which rows near the viewport a view puts to the shared test at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Register {
    /// Every row: the Icons and List views.
    Every,
    Unfilled,
}

impl Details {
    fn of(self, entry: &FileEntry) -> bool {
        match self {
            Self::All => true,
            Self::Folders => entry.is_directory(),
            Self::Off => false,
        }
    }
}

pub(crate) const SCROLL_SETTLE: f64 = 0.150;

#[derive(Clone, Default)]
pub(crate) struct ScrollSettle {
    folder: Option<Location>,
    moved_at: Option<f64>,
    moved: bool,
    was_dragging: bool,
    last: Range<usize>,
}

impl ScrollSettle {
    /// Records the folder currently shown: a new folder starts unscrolled.
    pub(crate) fn visit(&mut self, folder: &Location) {
        if self.folder.as_ref() != Some(folder) {
            let switched = self.folder.replace(folder.clone()).is_some();
            self.moved = false;
            self.moved_at = None;
            self.was_dragging = false;
            if switched {
                self.last = 0..0;
            }
        }
    }

    pub(crate) fn see(&mut self, range: &Range<usize>) -> bool {
        let scrolled = !self.last.is_empty() && self.last.start != range.start;
        self.moved |= scrolled;
        self.last = range.clone();
        scrolled
    }

    pub(crate) fn settled(&mut self, clock: f64, dragging: bool) -> Result<(), f64> {
        if dragging {
            self.was_dragging = true;
        } else if std::mem::take(&mut self.was_dragging) {
            self.moved = false;
            self.moved_at = None;
            return Ok(());
        }
        self.wait_still(clock)
    }

    pub(crate) fn settled_for_drag(&mut self, clock: f64, dragging: bool) -> Result<(), f64> {
        if !dragging {
            self.was_dragging = false;
            self.moved = false;
            self.moved_at = None;
            return Ok(());
        }
        self.was_dragging = true;
        self.wait_still(clock)
    }

    fn wait_still(&mut self, clock: f64) -> Result<(), f64> {
        if std::mem::take(&mut self.moved) {
            self.moved_at = Some(clock);
        }
        match self.moved_at {
            Some(at) if clock - at < SCROLL_SETTLE => Err(SCROLL_SETTLE - (clock - at)),
            _ => {
                self.moved_at = None;
                Ok(())
            }
        }
    }
}

/// Timing and thumb-drag state for a poll tick checking scroll settle.
#[derive(Clone, Copy)]
pub(crate) struct ScrollPoll {
    pub(crate) clock: f64,
    pub(crate) dragging: bool,
}

/// One view's fills: the range it shows, and the rows whose fills are out.
pub(crate) struct Fills {
    /// Which rows are asked their details too.
    details: Details,
    register: Register,
    /// The range the view last reported, and whether a poll has asked for it since.
    range: Range<usize>,
    fresh: bool,
    /// The folder and the listing's version the last poll asked for.
    seen: Option<(usize, Location, u64)>,
    /// The rows asked for, each with what the browser knew of it then.
    asked: HashMap<EntryId, Known>,
    /// Scroll-settle tracker shared across Columns, List, and Icons.
    settle: ScrollSettle,
}

/// What the browser knows of an entry's metadata: a fill that lands changes it.
#[derive(PartialEq)]
struct Known {
    size: MetadataValue<u64>,
    modified: MetadataValue<i64>,
    created: MetadataValue<i64>,
    mode: MetadataValue<u32>,
    dimensions: MetadataValue<(u32, u32)>,
    child_count: MetadataValue<u64>,
    duration: MetadataValue<u64>,
    listable: MetadataValue<bool>,
}

impl Known {
    fn of(entry: &FileEntry) -> Self {
        Self {
            size: entry.size.clone(),
            modified: entry.modified_unix_seconds.clone(),
            created: entry.created_unix_seconds.clone(),
            mode: entry.mode.clone(),
            dimensions: entry.image_dimensions.clone(),
            child_count: entry.child_count.clone(),
            duration: entry.duration_seconds.clone(),
            listable: entry.listable.clone(),
        }
    }
}

impl Fills {
    pub(crate) fn new(details: Details) -> Self {
        Self::registering(details, Register::Every)
    }

    pub(crate) fn unfilled(details: Details) -> Self {
        Self::registering(details, Register::Unfilled)
    }

    fn registering(details: Details, register: Register) -> Self {
        Self {
            details,
            register,
            range: 0..0,
            fresh: false,
            seen: None,
            asked: HashMap::new(),
            settle: ScrollSettle::default(),
        }
    }

    pub(crate) fn set_details(&mut self, details: Details) {
        if self.details != details {
            self.details = details;
            self.asked.clear();
            self.fresh = true;
        }
    }

    pub(crate) fn invalidate(&mut self) {
        self.asked.clear();
        self.fresh = true;
    }

    /// The view's visible range changed. Nothing is asked until the next poll.
    pub(crate) fn see(&mut self, range: Range<usize>) {
        self.settle.see(&range);
        if range != self.range {
            self.range = range;
            self.fresh = true;
        }
    }

    pub(crate) fn request_settled(
        &mut self,
        browser: &impl Filler,
        depth: usize,
        listing: &Column,
        poll: ScrollPoll,
    ) -> Option<f64> {
        self.settle.visit(listing.location());
        match self.settle.settled(poll.clock, poll.dragging) {
            Ok(()) => {
                self.request(browser, depth, listing);
                None
            }
            Err(left) => Some(left),
        }
    }

    #[cfg(test)]
    pub(crate) fn settle_mut(&mut self) -> &mut ScrollSettle {
        &mut self.settle
    }

    pub(crate) fn request(&mut self, browser: &impl Filler, depth: usize, listing: &Column) {
        if !self.due(depth, listing) {
            return;
        }
        let shown = Shown { depth, listing };
        let range = widened(&self.range, listing.len());
        self.forget_beyond(listing, range.clone());
        let mut near = Vec::new();
        for index in range {
            if let Some(location) = self.consider(browser, &shown, index) {
                near.push(location);
            }
        }
        if !near.is_empty() {
            browser.prioritize(depth, &near);
        }
    }

    fn forget_beyond(&mut self, listing: &Column, range: Range<usize>) {
        let near: HashSet<EntryId> = range.map(|index| listing.id_at(index)).collect();
        self.asked.retain(|id, _| near.contains(id));
    }

    fn due(&mut self, depth: usize, listing: &Column) -> bool {
        let seen = (depth, listing.location().clone(), listing.version());
        let same_folder = self
            .seen
            .as_ref()
            .is_some_and(|(d, location, _)| (*d, location) == (depth, listing.location()));
        if !same_folder {
            self.asked.clear();
        }
        if !self.fresh && self.seen.as_ref() == Some(&seen) {
            return false;
        }
        self.fresh = false;
        self.seen = Some(seen);
        true
    }

    fn consider(
        &mut self,
        browser: &impl Filler,
        shown: &Shown<'_>,
        index: usize,
    ) -> Option<Location> {
        let position = shown.listing.position_at(index)?;
        let entry = browser.entry_at(shown.depth, position)?;
        let id = shown.listing.id_at(index);
        let details = self.details.of(&entry);
        if !self.registers(&entry, details) || !needs_fill(&entry, details) {
            self.asked.remove(&id);
            return None;
        }
        let held = entry.is_directory() && browser.touched(&entry.location);
        let known = Known::of(&entry);
        if !held && self.asked.get(&id) != Some(&known) {
            self.asked.insert(id, known);
            browser.request((shown.depth, position), entry.location.clone(), details);
        }
        Some(entry.location)
    }

    /// Whether the view puts `entry` to the shared test at all ([`Register`]).
    fn registers(&self, entry: &FileEntry, details: bool) -> bool {
        match self.register {
            Register::Every => true,
            Register::Unfilled => metadata_needs_fill(entry) || details,
        }
    }
}

/// The folder a poll asks for: its depth in the browser, and the view's listing of it.
struct Shown<'a> {
    depth: usize,
    listing: &'a Column,
}

/// `range` with a quarter of its rows more on each side, within `len`.
fn widened(range: &Range<usize>, len: usize) -> Range<usize> {
    let margin = range.len().div_ceil(4);
    range.start.saturating_sub(margin)..range.end.saturating_add(margin).min(len)
}

fn needs_fill(entry: &FileEntry, details: bool) -> bool {
    metadata_needs_fill(entry)
        || entry.mode == MetadataValue::Unknown
        || (details && details_need_fill(entry))
}

fn metadata_needs_fill(entry: &FileEntry) -> bool {
    let folder = entry.is_directory();
    entry.modified_unix_seconds == MetadataValue::Unknown
        || (!folder && entry.size == MetadataValue::Unknown)
        || (folder
            && entry.location.native_path().is_some()
            && entry.listable == MetadataValue::Unknown)
}

fn details_need_fill(entry: &FileEntry) -> bool {
    if entry.is_directory() {
        return entry.child_count == MetadataValue::Unknown;
    }
    let path = Path::new(&entry.native_name);
    let dimensions = entry.image_dimensions == MetadataValue::Unknown && is_image_path(path);
    let duration = entry.duration_seconds == MetadataValue::Unknown && is_media_path(path);
    dimensions || duration
}

#[cfg(test)]
mod tests;
