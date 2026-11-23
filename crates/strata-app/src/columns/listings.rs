// SPDX-License-Identifier: MIT

use std::{
    borrow::Cow,
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    rc::Rc,
    time::Duration,
};

use rune::Tx;
use strata_components::{
    columns::ColumnSources,
    contract::{EntryId, EntrySource},
};
use strata_core::{model::Location, util::DateFormat};

use crate::{
    details,
    listing::{Column, Words},
    slots::Worn,
};

const SPINNER_DELAY: Duration = Duration::from_millis(120);

#[derive(Default)]
struct SpinnerDelay {
    of: Option<Location>,
    since: f64,
    shown: bool,
}

impl SpinnerDelay {
    fn watch(&mut self, column: &Column, clock: f64, tx: &mut Tx) -> bool {
        if !column.is_loading() {
            *self = Self::default();
            return false;
        }
        if column.is_busy() {
            let was = self.shown;
            self.of = Some(column.location().clone());
            self.shown = true;
            return !was;
        }
        if self.of.as_ref() != Some(column.location()) {
            *self = Self {
                of: Some(column.location().clone()),
                since: clock,
                shown: false,
            };
            tx.wake_after(SPINNER_DELAY);
            return false;
        }
        if self.shown {
            return false;
        }
        let left = SPINNER_DELAY.as_secs_f64() - (clock - self.since);
        if left > 1e-6 {
            tx.wake_after(Duration::from_secs_f64(left));
            return false;
        }
        self.shown = true;
        true
    }
}

/// The open columns, root first, and the words for what a row is.
pub(super) struct Listings {
    pub(super) columns: Vec<Column>,
    spinners: Vec<SpinnerDelay>,
    pub(super) worn: Rc<Worn>,
    described: Words,
    /// A filter's results, and the column whose rows they replace.
    hits: Option<(usize, Column)>,
    hit_paths: HashMap<EntryId, String>,
    /// The results come from the subfolders too: each shows its path under its name.
    hit_details: bool,
    /// Moves each time results are shown or dropped, so two sets never share a version.
    swaps: u64,
    /// Active date format, synced with preferences for hover tooltip versioning.
    date_format: DateFormat,
    /// Relative-date refresh epoch, bumped every 30 s while dates are relative.
    relative_epoch: u64,
}

impl Listings {
    pub(super) fn translated(worn: Rc<Worn>) -> Self {
        Self {
            columns: Vec::new(),
            spinners: Vec::new(),
            worn,
            described: Words::translated(),
            hits: None,
            hit_paths: HashMap::new(),
            hit_details: false,
            swaps: 0,
            date_format: DateFormat::Relative,
            relative_epoch: 0,
        }
    }

    pub(super) fn sync_date_format(&mut self, format: DateFormat) {
        self.date_format = format;
    }

    pub(super) fn date_format(&self) -> DateFormat {
        self.date_format
    }

    pub(super) fn set_relative_epoch(&mut self, epoch: u64) {
        self.relative_epoch = epoch;
    }

    pub(super) fn has_rows(&self) -> bool {
        (0..self.columns.len()).any(|depth| self.shown(depth).is_some_and(|c| !c.is_empty()))
    }

    pub(super) fn watch_spinners(&mut self, clock: f64, tx: &mut Tx) -> bool {
        self.spinners
            .resize_with(self.columns.len(), SpinnerDelay::default);
        let mut changed = false;
        for (delay, column) in self.spinners.iter_mut().zip(&self.columns) {
            changed |= delay.watch(column, clock, tx);
        }
        changed
    }

    pub(super) fn set_worn(&mut self, worn: &Rc<Worn>) {
        self.worn = worn.clone();
        for column in &mut self.columns {
            column.set_worn(worn);
        }
        if let Some((_, hits)) = &mut self.hits {
            hits.set_worn(worn);
        }
    }

    /// What column `depth` shows: a filter's results, or its folder's rows.
    pub(super) fn shown(&self, depth: usize) -> Option<&Column> {
        match &self.hits {
            Some((at, hits)) if *at == depth => Some(hits),
            _ => self.columns.get(depth),
        }
    }

    /// Whether a filter's results show in the place of column `depth`'s rows.
    pub(super) fn hits_at(&self, depth: usize) -> bool {
        self.hits.as_ref().is_some_and(|(at, _)| *at == depth)
    }

    /// Shows `hits` in the place of column `depth`'s rows, or replaces the results shown there.
    pub(super) fn show_hits(&mut self, depth: usize, mut hits: Column) {
        hits.set_worn(&self.worn);
        self.swaps = self.swaps.wrapping_add(1);
        self.hits = Some((depth, hits));
    }

    /// Gives the column the results showed in its rows back. The depth, when there were any.
    pub(super) fn drop_hits(&mut self) -> Option<usize> {
        self.take_hits().map(|(depth, _)| depth)
    }

    /// Takes the results out, and the column they showed in, when there were any.
    pub(super) fn take_hits(&mut self) -> Option<(usize, Column)> {
        let taken = self.hits.take()?;
        self.swaps = self.swaps.wrapping_add(1);
        self.hit_paths.clear();
        Some(taken)
    }

    /// Each result's path, by its id, and whether they show as detail lines.
    pub(super) fn describe_hits(&mut self, paths: HashMap<EntryId, String>, details: bool) {
        self.hit_paths = paths;
        self.hit_details = details;
    }

    /// The path of the result at `index` of column `depth`, while results show there.
    fn hit_path(&self, depth: usize, index: usize) -> Option<&str> {
        let (_, hits) = self.hits.as_ref().filter(|(at, _)| *at == depth)?;
        self.hit_paths.get(&hits.id_at(index)).map(String::as_str)
    }

    pub(super) fn drop_hits_from(&mut self, from: usize) {
        if self.hits.as_ref().is_some_and(|(at, _)| *at >= from) {
            self.drop_hits();
        }
    }

    pub(super) fn collect_ids(&self, out: &mut impl Extend<EntryId>) {
        for column in &self.columns {
            column.collect_ids(out);
        }
        if let Some((_, hits)) = &self.hits {
            hits.collect_ids(out);
        }
    }
}

impl ColumnSources for Listings {
    fn len(&self) -> usize {
        self.columns.len()
    }

    fn source(&self, depth: usize) -> &dyn EntrySource {
        match self.shown(depth) {
            Some(column) => column,
            None => &self.columns[depth],
        }
    }

    fn version(&self, depth: usize) -> u64 {
        let version = self.shown(depth).map_or(0, Column::key_version);
        version.wrapping_add(self.swaps << 40)
    }

    fn title(&self, depth: usize) -> &str {
        self.columns[depth].title()
    }

    fn title_description(&self, depth: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(self.columns[depth].location().display_path()))
    }

    fn loading(&self, depth: usize) -> bool {
        self.columns[depth].is_loading()
    }

    fn spinner(&self, depth: usize) -> bool {
        let column = &self.columns[depth];
        column.is_busy()
            || (column.is_loading() && self.spinners.get(depth).is_some_and(|delay| delay.shown))
    }

    /// None while results show in column `depth`: the next column is no folder of theirs.
    fn open_child(&self, depth: usize) -> Option<EntryId> {
        if self.hits_at(depth) {
            return None;
        }
        self.columns.get(depth + 1)?.from()
    }

    fn badge(&self, depth: usize, index: usize) -> Option<Cow<'_, str>> {
        let column = self.shown(depth)?;
        let row = column.row_at(index)?;
        let text = details::size_cell(row.is_folder(), row.facts());
        (!text.is_empty()).then_some(Cow::Owned(text))
    }

    fn description(&self, depth: usize, index: usize) -> Option<Cow<'_, str>> {
        let description = self.shown(depth)?.description(index, &self.described)?;
        Some(Cow::Borrowed(description))
    }

    fn has_details(&self, depth: usize) -> bool {
        self.hit_details && self.hits_at(depth)
    }

    /// A result's path under its name, while the results come from the subfolders too.
    fn detail(&self, depth: usize, index: usize) -> Option<Cow<'_, str>> {
        self.has_details(depth)
            .then(|| self.hit_path(depth, index))
            .flatten()
            .map(Cow::Borrowed)
    }

    fn tooltip_version(&self, depth: usize, index: usize) -> u64 {
        let mut hasher = DefaultHasher::new();
        let modified = self
            .shown(depth)
            .and_then(|column| column.row_at(index))
            .and_then(|row| row.facts().modified);
        modified.hash(&mut hasher);
        self.date_format.as_str().hash(&mut hasher);
        if self.date_format == DateFormat::Relative {
            self.relative_epoch.hash(&mut hasher);
        }
        hasher.finish()
    }

    fn tooltip(&self, depth: usize, index: usize) -> Option<Cow<'_, str>> {
        let row = self.shown(depth)?.row_at(index)?;
        let date = details::modified_tooltip_date(row.facts())?;
        details::entry_tooltip_text(row.name(), false, Some(&date)).map(Cow::Owned)
    }
}

#[cfg(test)]
mod tests;
