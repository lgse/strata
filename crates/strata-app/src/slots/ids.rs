// SPDX-License-Identifier: MIT

//! Entry ids. strata-components names an entry by an `EntryId` that is unique across every
//! column and never names another entry later; Strata names one by its `Location`. One table
//! turns the second into the first, and keeps every pair for the life of the process.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    hash::{BuildHasher, Hasher, RandomState},
    rc::Rc,
    sync::OnceLock,
};

use strata_components::contract::EntryId;
use strata_core::model::Location;

/// Per-process seeded fast hasher for `Location` path keys in [`Table`].
#[derive(Clone, Default)]
struct LocationBuildHasher;

struct LocationHasher(u64);

fn location_hash_seed() -> u64 {
    static SEED: OnceLock<u64> = OnceLock::new();
    *SEED.get_or_init(|| RandomState::new().build_hasher().finish())
}

impl BuildHasher for LocationBuildHasher {
    type Hasher = LocationHasher;

    fn build_hasher(&self) -> Self::Hasher {
        LocationHasher(location_hash_seed())
    }
}

impl Hasher for LocationHasher {
    fn finish(&self) -> u64 {
        let mut h = self.0;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51afd7ed558ccd);
        h ^= h >> 33;
        h
    }

    fn write(&mut self, bytes: &[u8]) {
        const K: u64 = 0x517cc1b727220a95;
        let mut h = self.0;
        let (chunks, rem) = bytes.as_chunks::<8>();
        for &chunk in chunks {
            let word = u64::from_ne_bytes(chunk);
            h = (h.rotate_left(5) ^ word).wrapping_mul(K);
        }
        if !rem.is_empty() {
            let mut tail = [0_u8; 8];
            tail[..rem.len()].copy_from_slice(rem);
            let word = u64::from_ne_bytes(tail);
            h = (h.rotate_left(5) ^ word ^ (rem.len() as u64)).wrapping_mul(K);
        }
        self.0 = h;
    }

    fn write_u8(&mut self, value: u8) {
        self.0 = (self.0.rotate_left(5) ^ u64::from(value)).wrapping_mul(0x517cc1b727220a95);
    }

    fn write_usize(&mut self, value: usize) {
        self.0 = (self.0.rotate_left(5) ^ (value as u64)).wrapping_mul(0x517cc1b727220a95);
    }
}

type LocationMap<V> = HashMap<Location, V, LocationBuildHasher>;

#[derive(Default)]
struct Table {
    by_location: LocationMap<EntryId>,
    issued: u64,
}

/// The table. The shell makes one for the window and hands it to every part
/// ([`super::Setup`]): a view's listener names the rows it copies with it, and its adapter the
/// folder it shows.
///
/// A rename gives an entry a new location, and so a new id: `Browser` says which entry is
/// selected after one, and the adapter writes that to the view.
#[derive(Clone, Default)]
pub(crate) struct Ids(Rc<RefCell<Table>>);

impl Ids {
    /// The id the entry at `location` has been given, if it has been: a reader that must not
    /// issue one, such as the QA backend, asks this.
    #[cfg(feature = "qa")]
    pub(crate) fn known(&self, location: &Location) -> Option<EntryId> {
        self.0.borrow().by_location.get(location).copied()
    }

    /// The id of the entry at `location`, issued the first time it is asked for.
    pub(crate) fn of(&self, location: &Location) -> EntryId {
        let mut table = self.0.borrow_mut();
        table.of(location)
    }

    /// Borrows the id table once for a batch of lookups, reserving room for `reserve` entries.
    pub(crate) fn with_batch<R>(&self, reserve: usize, f: impl FnOnce(&mut IdBatch<'_>) -> R) -> R {
        let mut table = self.0.borrow_mut();
        table.by_location.reserve(reserve);
        f(&mut IdBatch { table: &mut table })
    }

    /// Issues an id for a virtual row with no [`Location`] (such as an archive preview member),
    /// unique across the process.
    pub(crate) fn fresh(&self) -> EntryId {
        let mut table = self.0.borrow_mut();
        table.issued += 1;
        EntryId::new(table.issued)
    }

    /// How many `Location -> EntryId` pairs are currently retained in the table.
    pub(crate) fn len(&self) -> usize {
        self.0.borrow().by_location.len()
    }

    /// Drops cached `Location -> EntryId` mappings whose ids are not in `keep`, shrinking the
    /// table while leaving the monotonic id counter untouched so dropped ids are never reused.
    pub(crate) fn retain_ids(&self, keep: &HashSet<EntryId>) -> usize {
        let mut table = self.0.borrow_mut();
        let before = table.by_location.len();
        table.by_location.retain(|_, id| keep.contains(id));
        let min_capacity = table.by_location.len().saturating_mul(2);
        table.by_location.shrink_to(min_capacity);
        before - table.by_location.len()
    }
}

#[cfg(test)]
mod tests;

impl Table {
    fn of(&mut self, location: &Location) -> EntryId {
        if let Some(&id) = self.by_location.get(location) {
            return id;
        }
        self.issued += 1;
        let id = EntryId::new(self.issued);
        self.by_location.insert(location.clone(), id);
        id
    }
}

/// A single-borrow batch handle into [`Ids`].
pub(crate) struct IdBatch<'a> {
    table: &'a mut Table,
}

impl IdBatch<'_> {
    pub(crate) fn of(&mut self, location: &Location) -> EntryId {
        self.table.of(location)
    }
}
