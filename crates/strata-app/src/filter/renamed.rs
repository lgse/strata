// SPDX-License-Identifier: MIT

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
    pub(super) fn expect(&mut self, from: &Path, to: Option<&Path>) {
        self.0 = Some(Pending {
            from: from.to_path_buf(),
            to: to.map(Path::to_path_buf),
        });
    }

    pub(super) fn clear(&mut self) {
        self.0 = None;
    }

    pub(super) fn awaits(&self, path: &Path) -> bool {
        self.0.as_ref().is_some_and(|pending| pending.from == path)
    }

    pub(super) fn clear_if(&mut self, from: &Path, to: Option<&Path>) {
        if self.0.as_ref().is_some_and(|pending| pending.is(from, to)) {
            self.0 = None;
        }
    }

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
