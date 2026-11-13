// SPDX-License-Identifier: MIT

//! Which rows the browser has replaced since a step remembered one. GTK replaces a column's
//! model whole when the browser replaces its entries (`EntriesReplaced`, which a reload such
//! as F5 ends in: `on_entries_replaced` in `src/ui/browser/events/listing.rs`), so each of its
//! rows is a new widget, while the events that insert, splice or fill rows keep them. Rune's
//! row keeps its node, since the collection keys a row by its entry's id and an id is the
//! entry's location. So the backend counts the replacements and tells a row of a column
//! replaced since it was remembered from the row it was, as GTK's `replaced` and recall do.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

use strata_components::contract::EntryId;
use strata_core::model::Location;
use strata_fs::app::{Browser, BrowserEvent};

use crate::{App, shell};

#[derive(Default)]
pub(super) struct Replacements {
    /// How many replacements the browser has made.
    count: Cell<u64>,
    /// The last replacement each entry was in, by that count.
    last: RefCell<HashMap<Location, u64>>,
}

impl Replacements {
    /// Counts the replacements `browser` makes from now on.
    pub(super) fn watch(browser: &Rc<Browser>) -> Rc<Self> {
        let replacements = Rc::new(Self::default());
        let (mine, watched) = (Rc::downgrade(&replacements), Rc::downgrade(browser));
        browser.observe(move |event| {
            if let BrowserEvent::EntriesReplaced { depth, .. } = event {
                record(&mine, &watched, *depth);
            }
        });
        replacements
    }

    /// The count as of now, which a remembered node keeps.
    pub(super) fn now(&self) -> u64 {
        self.count.get()
    }

    /// Whether the entry the components call `id` was in a replacement after `since`.
    pub(super) fn replaced_since(&self, app: &App, id: EntryId, since: u64) -> bool {
        let last = self.last.borrow();
        last.iter()
            .any(|(location, at)| *at > since && shell::known_id(app, location) == Some(id))
    }
}

fn record(replacements: &Weak<Replacements>, browser: &Weak<Browser>, depth: usize) {
    let (Some(replacements), Some(browser)) = (replacements.upgrade(), browser.upgrade()) else {
        return;
    };
    let count = replacements.count.get() + 1;
    replacements.count.set(count);
    let mut last = replacements.last.borrow_mut();
    browser.with_column_entries(depth, |entries| {
        for entry in entries {
            last.insert(entry.location.clone(), count);
        }
    });
}
