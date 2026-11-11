//! The bookkeeping behind [`Change`]s: this process's copies and where each stands, and what
//! each selection offers. Pure: the hub feeds it what the platform reported, in order.
//!
//! A copy holds once its own selection comes back (the compositor's echo, or X11's owner
//! change) while the platform still says the source holds it. Several copies of ours can be
//! in flight at once (a copy, then another before the first's echo), so the echo is matched
//! to the newest copy that still holds and offers exactly the echoed types (in any order).
//!
//! That rule is guarded by the unit tests in `book/tests.rs`, not by the session tests: every
//! platform here cancels a source before it announces the next owner, on the same
//! connection, so no session can deliver another client's offer while one of our copies
//! still holds, which is what the rule's checks (the types, and `holds`) are for.

use std::collections::{HashMap, VecDeque};

use super::api::{Change, CopyId, CopyState, Lost, Offered, Selection};

/// How many finished copies keep their state for [`Book::state`].
const KEEP_FINISHED: usize = 256;

struct Entry {
    selection: Selection,
    mimes: Vec<String>,
    state: CopyState,
    /// [`Book::clearing`] ran: its cancel is the clear's.
    clearing: bool,
    /// Another client's offer was heard while this copy was still pending: whatever became
    /// of the write, the selection has moved on.
    superseded: bool,
}

#[derive(Default)]
pub(super) struct Book {
    copies: HashMap<CopyId, Entry>,
    finished: VecDeque<CopyId>,
    offers: HashMap<Selection, Offered>,
}

impl Book {
    /// A copy went out.
    pub(super) fn copied(&mut self, selection: Selection, copy: CopyId, mimes: Vec<String>) {
        self.copies.insert(
            copy,
            Entry {
                selection,
                mimes,
                state: CopyState::Pending,
                clearing: false,
                superseded: false,
            },
        );
    }

    /// `selection` now offers `mimes`, or nothing. `holds` says whether the platform still
    /// counts a copy as holding.
    pub(super) fn selection(
        &mut self,
        selection: Selection,
        mimes: Option<Vec<String>>,
        holds: &dyn Fn(CopyId) -> bool,
    ) -> Vec<Change> {
        let Some(mimes) = mimes else {
            return match self.offers.remove(&selection) {
                Some(_) => vec![Change::Emptied { selection }],
                None => Vec::new(),
            };
        };
        let ours = self
            .copies
            .iter()
            .filter(|(id, e)| {
                e.selection == selection
                    && !matches!(e.state, CopyState::Lost(_))
                    && same_types(&e.mimes, &mimes)
                    && holds(**id)
            })
            .map(|(id, _)| *id)
            .max();
        let mut changes = Vec::new();
        match ours {
            Some(copy) => {
                if let Some(e) = self.copies.get_mut(&copy)
                    && e.state == CopyState::Pending
                {
                    e.state = CopyState::Holding;
                    changes.push(Change::Holding { selection, copy });
                }
            }
            None => {
                for e in self.copies.values_mut() {
                    if e.selection == selection && e.state == CopyState::Pending {
                        e.superseded = true;
                    }
                }
                changes.push(Change::Offered {
                    selection,
                    mimes: mimes.clone(),
                });
            }
        }
        self.offers.insert(selection, Offered { mimes, ours });
        changes
    }

    /// The platform says `copy` no longer holds its selection. `refused`: the platform judged
    /// its write dropped (a data-device write with no echo).
    pub(super) fn cancelled(&mut self, copy: CopyId, refused: bool) -> Vec<Change> {
        let Some(e) = self.copies.get(&copy) else {
            return Vec::new();
        };
        if matches!(e.state, CopyState::Lost(_)) {
            return Vec::new();
        }
        let selection = e.selection;
        let newer = self.copies.iter().any(|(id, other)| {
            *id > copy && other.selection == selection && !matches!(other.state, CopyState::Lost(_))
        });
        let why = if e.clearing {
            Lost::Cleared
        } else if newer {
            Lost::Replaced
        } else if refused {
            Lost::Refused
        } else {
            Lost::Taken
        };
        self.finish(copy, why);
        if let Some(o) = self.offers.get_mut(&selection)
            && o.ours == Some(copy)
        {
            o.ours = None;
        }
        vec![Change::Lost {
            selection,
            copy,
            why,
        }]
    }

    /// A clear of `copy` is going out: true if it still holds, or may.
    pub(super) fn clearing(&mut self, copy: CopyId, on: bool) -> bool {
        match self.copies.get_mut(&copy) {
            Some(e) if !matches!(e.state, CopyState::Lost(_)) => {
                e.clearing = on;
                true
            }
            _ => false,
        }
    }

    /// The platform connection failed: every copy is lost, every selection empty.
    pub(super) fn failed(&mut self) -> Vec<Change> {
        let mut live: Vec<CopyId> = self
            .copies
            .iter()
            .filter(|(_, e)| !matches!(e.state, CopyState::Lost(_)))
            .map(|(id, _)| *id)
            .collect();
        live.sort();
        let mut changes = Vec::new();
        for copy in live {
            let selection = self.copies[&copy].selection;
            self.finish(copy, Lost::Failed);
            changes.push(Change::Lost {
                selection,
                copy,
                why: Lost::Failed,
            });
        }
        let mut emptied: Vec<Selection> = self.offers.drain().map(|(s, _)| s).collect();
        emptied.sort_by_key(|s| *s == Selection::Primary);
        changes.extend(
            emptied
                .into_iter()
                .map(|selection| Change::Emptied { selection }),
        );
        changes
    }

    pub(super) fn state(&self, copy: CopyId) -> Option<CopyState> {
        self.copies.get(&copy).map(|e| e.state)
    }

    pub(super) fn offered(&self, selection: Selection) -> Option<&Offered> {
        self.offers.get(&selection)
    }

    /// The copy of ours a read of `selection` gets, if any: the newest one holding, or still
    /// pending with no other client's offer heard since (a read right after a copy gets the
    /// copy, but a write that never took doesn't shadow what came after it).
    pub(super) fn serving(&self, selection: Selection) -> Option<CopyId> {
        self.live(selection).into_iter().find(|c| {
            let e = &self.copies[c];
            e.state == CopyState::Holding || !e.superseded
        })
    }

    /// The live copies of `selection`, newest first.
    pub(super) fn live(&self, selection: Selection) -> Vec<CopyId> {
        let mut live: Vec<CopyId> = self
            .copies
            .iter()
            .filter(|(_, e)| e.selection == selection && !matches!(e.state, CopyState::Lost(_)))
            .map(|(id, _)| *id)
            .collect();
        live.sort_by(|a, b| b.cmp(a));
        live
    }

    fn finish(&mut self, copy: CopyId, why: Lost) {
        if let Some(e) = self.copies.get_mut(&copy) {
            e.state = CopyState::Lost(why);
            e.mimes = Vec::new();
        }
        self.finished.push_back(copy);
        while self.finished.len() > KEEP_FINISHED {
            if let Some(old) = self.finished.pop_front() {
                self.copies.remove(&old);
            }
        }
    }
}

/// The same types, in any order: a platform may list an offer's types in another order than
/// they were offered.
fn same_types(a: &[String], b: &[String]) -> bool {
    use std::collections::BTreeSet;
    a.iter().collect::<BTreeSet<_>>() == b.iter().collect::<BTreeSet<_>>()
}

#[cfg(test)]
mod tests;
