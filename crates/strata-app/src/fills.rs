// SPDX-License-Identifier: MIT

//! Metadata fills for the rows a view shows: what the browser does not know of an entry yet
//! (its size, a folder's count, an image's dimensions, a video's length) is asked for while
//! the row is on screen or near it. Shared code: the Icons view asks every row's details
//! ([`Details::All`]), the List view a folder's only, for its item count ([`Details::Folders`],
//! as GTK's List asks with `details = entry.is_directory()`).
//!
//! GTK's rule (`schedule_refresh` in `src/ui/thumbnail/viewport.rs`, at `62bbb1609`; ported as
//! new code and listed in `docs/rune/parity/packages/A1.md`, "Ported from GTK"):
//! - **Once per idle.** A scroll schedules one refresh, however often it moves. Here the
//!   view's `on_visible` only records the range ([`Fills::see`]), and the part's `poll`
//!   asks once ([`Fills::request`]).
//! - **On screen and near it.** GTK asks for rows on screen and within a quarter of the
//!   viewport around it. Here that is the range widened by a quarter of its rows each way.
//! - **Each row once while its fill is out.** GTK's browser drops a request already in
//!   flight (`fill_in_flight`). A row here is asked for again only once what the browser
//!   knows of it changed since it was asked for (its fill landed) and it still lacks
//!   something, or once it left the range and came back, as GTK asks again for an item it
//!   binds again.
//! - **Visible first.** One `prioritize_metadata_fills` puts the rows near the viewport at
//!   the head of the browser's queue.
//! - **Not under an operation.** A folder a copy or move touches is not asked for
//!   (`Browser::operation_touches`).

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

/// Which rows a view asks the details of: a folder's item count, an image's dimensions and a
/// video's length, which strata-fs reads only on a fill asked with details.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Details {
    /// Every row: the Icons view's details line shows all three.
    All,
    /// Folders only, for their item count: the List view's Size column. GTK's List asks with
    /// `details = entry.is_directory()` (`src/ui/browser_modes/list_factory.rs`).
    Folders,
}

impl Details {
    fn of(self, entry: &FileEntry) -> bool {
        match self {
            Self::All => true,
            Self::Folders => entry.is_directory(),
        }
    }
}

/// One view's fills: the range it shows, and the rows whose fills are out.
pub(crate) struct Fills {
    /// Which rows are asked their details too.
    details: Details,
    /// The range the view last reported, and whether a poll has asked for it since.
    range: Range<usize>,
    fresh: bool,
    /// The folder and the listing's version the last poll asked for.
    seen: Option<(usize, Location, u64)>,
    /// The rows asked for, each with what the browser knew of it then.
    asked: HashMap<EntryId, Known>,
}

/// What the browser knows of an entry's metadata: a fill that lands changes it.
#[derive(PartialEq)]
struct Known {
    size: MetadataValue<u64>,
    modified: MetadataValue<i64>,
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
        Self {
            details,
            range: 0..0,
            fresh: false,
            seen: None,
            asked: HashMap::new(),
        }
    }

    /// The view's visible range changed. Nothing is asked until the next poll.
    pub(crate) fn see(&mut self, range: Range<usize>) {
        if range != self.range {
            self.range = range;
            self.fresh = true;
        }
    }

    /// Asks for the fills the rows near the viewport lack, once for each row while it is out:
    /// call it from the part's `poll`. It does nothing when neither the range nor the listing
    /// changed since the last call.
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

    /// Forgets the rows asked for outside `range`, as GTK forgets an item it unbinds
    /// (`cancel_metadata`): a row asked for again when it comes back, if its fill never
    /// landed. The browser has no public way to cancel a queued fill, and GTK cancels none.
    fn forget_beyond(&mut self, listing: &Column, range: Range<usize>) {
        let near: HashSet<EntryId> = range.map(|index| listing.id_at(index)).collect();
        self.asked.retain(|id, _| near.contains(id));
    }

    /// Whether the range or the listing changed since the last poll asked; a new folder
    /// forgets what was asked in the last.
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

    /// Asks for the row at view index `index` if it lacks something and is not asked for
    /// already; gives its location if it lacks something, for the browser's priorities.
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
        if !needs_fill(&entry, details) {
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

/// Whether the browser still lacks something a view shows of `entry`: GTK's test in
/// `schedule_refresh`, from `metadata_needs_fill` (`src/ui/browser/entry.rs`) and
/// `icon_details_need_fill` (`src/ui/browser_modes/metadata.rs`).
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
