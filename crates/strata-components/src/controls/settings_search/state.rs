//! The search's state: the field, the cached matches, and the pending jump's highlight.

use rune::{TextEdit, Tx};

use super::index::SearchIndex;
use super::matching::{Matches, Prepared, normalized};

/// How long a jumped-to row's highlight shows, in seconds: full for [`FLASH_HOLD`], faded out
/// by this (OWNER ruling; Strata highlights nothing).
pub const FLASH_FOR: f64 = 1.5;
/// How long the highlight shows at full strength before it fades, in seconds.
pub const FLASH_HOLD: f64 = 0.6;

/// A row's highlight after a jump.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Flash {
    pub row: String,
    /// Keys the highlight, so a new jump replays it.
    pub generation: u64,
    /// When it showed, on `Tx::clock`.
    pub since: f64,
}

/// The settings search's state, held by the host.
#[derive(Clone, Debug, Default)]
pub struct SettingsSearchState {
    pub(super) field: TextEdit,
    prepared: Option<Prepared>,
    /// The normalised query `matches` answer, and whether there is one.
    query: String,
    /// The field's text as last searched: any change of it is a new query, as Strata's every
    /// text change opens the best page again, even one that normalises the same.
    searched: String,
    matches: Option<Matches>,
    /// The index revision `matches` were found in.
    revision: Option<u64>,
    pub(super) flash: Option<Flash>,
    generation: u64,
}

/// A search found by [`SettingsSearchState::plan`], to store.
pub(super) struct Refresh {
    prepared: Option<Prepared>,
    query: String,
    searched: String,
    matches: Option<Matches>,
    revision: u64,
    change: Change,
}

/// What a change of query asks the host to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Change {
    /// The query changed: open this page (none: stay), and scroll the page to its top.
    Query(Option<String>),
    /// The index changed under the same query: only the shown rows did.
    Rows,
}

impl SettingsSearchState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The field's text, as typed.
    pub fn text(&self) -> &str {
        &self.field.text
    }

    pub fn edit(&self) -> &TextEdit {
        &self.field
    }

    /// The field, for a C21 `FieldMenu`'s edits. Call [`super::SettingsSearch::refresh`] after.
    pub fn edit_mut(&mut self) -> &mut TextEdit {
        &mut self.field
    }

    /// Replace the query, caret at its end. The host's next tick searches it.
    pub fn set_text(&mut self, text: &str) {
        self.field.replace_text(text);
    }

    /// The normalised query the shown rows answer; empty when nothing filters.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Whether a query is filtering the rows.
    pub fn searching(&self) -> bool {
        self.matches.is_some()
    }

    /// Whether a query found nothing: the host shows the no-results view.
    pub fn no_results(&self) -> bool {
        self.matches.as_ref().is_some_and(Matches::none)
    }

    /// How many rows the query shows; `None` when nothing filters.
    pub fn count(&self) -> Option<usize> {
        self.matches.as_ref().map(Matches::count)
    }

    /// The row a jump highlights now, if any.
    pub fn highlighted(&self) -> Option<&str> {
        self.flash.as_ref().map(|f| f.row.as_str())
    }

    pub(super) fn matches(&self) -> Option<&Matches> {
        self.matches.as_ref()
    }

    pub(super) fn prepared(&self) -> Option<&Prepared> {
        self.prepared.as_ref()
    }

    /// Search the field's text in `index` if it or the index changed since the last search:
    /// what to store, found without changing anything (the host's state holds the index too).
    pub(super) fn plan(&self, index: &SearchIndex) -> Option<Refresh> {
        let query = normalized(&self.field.text);
        let fresh = self
            .prepared
            .as_ref()
            .filter(|p| p.revision == index.revision)
            .is_none()
            .then(|| Prepared::new(index));
        let changed_query = self.field.text != self.searched;
        if !changed_query && self.revision == Some(index.revision) {
            return None;
        }
        let prepared = fresh.as_ref().or(self.prepared.as_ref())?;
        let matches = (!query.is_empty()).then(|| prepared.find(&query));
        let change = if changed_query {
            let best = matches.as_ref().and_then(|m| m.best).map(|(.., page)| page);
            // Clearing a query that found nothing leaves the no-results view for the first page.
            let page = best.or_else(|| (self.no_results() && matches.is_none()).then_some(0));
            Change::Query(page.and_then(|p| index.pages.get(p)).map(|p| p.key.clone()))
        } else {
            Change::Rows
        };
        Some(Refresh {
            prepared: fresh,
            query,
            searched: self.field.text.clone(),
            matches,
            revision: index.revision,
            change,
        })
    }

    /// Store a search [`SettingsSearchState::plan`] found.
    pub(super) fn store(&mut self, r: Refresh) -> Change {
        if let Some(p) = r.prepared {
            self.prepared = Some(p);
        }
        if matches!(r.change, Change::Query(_)) {
            self.flash = None;
        }
        self.query = r.query;
        self.searched = r.searched;
        self.matches = r.matches;
        self.revision = Some(r.revision);
        r.change
    }

    /// Start a row's highlight.
    pub(super) fn flash(&mut self, row: String, tx: &mut Tx) {
        self.generation += 1;
        self.flash = Some(Flash {
            row,
            generation: self.generation,
            since: tx.clock(),
        });
        tx.wake_at(tx.clock() + FLASH_FOR);
    }

    /// Drop a highlight that has shown for [`FLASH_FOR`], asking for a wake when it is due.
    /// True when the view changes.
    pub(super) fn tick_flash(&mut self, now: f64, tx: &mut Tx) -> bool {
        let Some(flash) = &self.flash else {
            return false;
        };
        if flash.since + FLASH_FOR - now <= 1e-9 {
            self.flash = None;
            return true;
        }
        tx.wake_at(flash.since + FLASH_FOR);
        false
    }
}
