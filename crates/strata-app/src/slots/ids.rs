// SPDX-License-Identifier: MIT

//! Entry ids. strata-components names an entry by an `EntryId` that is unique across every
//! column and never names another entry later; Strata names one by its `Location`. One table
//! turns the second into the first, and keeps every pair for the life of the process.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use strata_components::contract::EntryId;
use strata_core::model::Location;

#[derive(Default)]
struct Table {
    by_location: HashMap<Location, EntryId>,
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
    pub(crate) fn known(&self, location: &Location) -> Option<EntryId> {
        self.0.borrow().by_location.get(location).copied()
    }

    /// The id of the entry at `location`, issued the first time it is asked for.
    pub(crate) fn of(&self, location: &Location) -> EntryId {
        let mut table = self.0.borrow_mut();
        if let Some(id) = table.by_location.get(location) {
            return *id;
        }
        table.issued += 1;
        let id = EntryId::new(table.issued);
        table.by_location.insert(location.clone(), id);
        id
    }
}
