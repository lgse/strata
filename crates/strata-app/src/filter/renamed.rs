// SPDX-License-Identifier: MIT

//! A rename of a filter result that the results follow to its new path: GTK's `RenamedResult`
//! (`src/ui/inline_search/renamed.rs`).
//!
//! Each result row's [`EntryId`] comes from its location, so a rename gives the item a new id.
//! Without following it, `Selection::sync` drops the old id as gone and clears the cursor and
//! selection (`renamed-filter-result-keeps-focus`). It holds one rename: the browser runs one
//! rename at a time, and starting one abandons the rename before it, so only the newest is
//! followed.

use std::path::{Path, PathBuf};

use strata_components::contract::EntryId;
use strata_core::model::Location;
use strata_fs::services::SearchItem;

use crate::slots::Ids;

#[derive(Default)]
pub(super) struct RenamedResult(Option<Pending>);

struct Pending {
    from: PathBuf,
    /// `None` for an item moved to another folder: it leaves these results.
    to: Option<PathBuf>,
}

impl Pending {
    fn is(&self, from: &Path, to: Option<&Path>) -> bool {
        self.from == from && self.to.as_deref() == to
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Renamed {
    pub(super) from: PathBuf,
    pub(super) to: PathBuf,
}

impl Renamed {
    pub(super) fn ids(&self, ids: &Ids) -> (EntryId, EntryId) {
        (
            ids.of(&Location::local(&self.from)),
            ids.of(&Location::local(&self.to)),
        )
    }
}

impl RenamedResult {
    /// Expects the result at `from` to be renamed to `to`, or with `None` to move to another
    /// folder.
    pub(super) fn expect(&mut self, from: &Path, to: Option<&Path>) {
        self.0 = Some(Pending {
            from: from.to_path_buf(),
            to: to.map(Path::to_path_buf),
        });
    }

    pub(super) fn clear(&mut self) {
        self.0 = None;
    }

    /// Forgets the rename of `from` to `to` (`None` for a move to another folder), leaving a
    /// newer rename's record. Starting a rename abandons the one before it, whose cleanup then
    /// runs after the newer rename may already have registered.
    pub(super) fn clear_if(&mut self, from: &Path, to: Option<&Path>) {
        if self.0.as_ref().is_some_and(|pending| pending.is(from, to)) {
            self.0 = None;
        }
    }

    /// The rename an update to `items` completes, once the old path has left and `items` list
    /// the new one. Results that still list `from` keep the rename waiting (including a
    /// mid-flight batch while renaming onto an existing sibling); complete results that list
    /// neither path end the follow because the item moved out or its new name no longer
    /// matches the query.
    pub(super) fn settle(&mut self, items: &[SearchItem], complete: bool) -> Option<Renamed> {
        let pending = self.0.as_ref()?;
        let listed = |path: &Path| items.iter().any(|item| item.path == path);
        if listed(&pending.from) {
            return None;
        }
        if let Some(to) = pending.to.as_ref().filter(|to| listed(to)).cloned() {
            let from = self.0.take()?.from;
            return Some(Renamed { from, to });
        }
        if complete {
            self.0 = None;
        }
        None
    }
}

#[cfg(test)]
mod tests;
