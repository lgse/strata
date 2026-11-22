// SPDX-License-Identifier: MIT

//! A row's file type, as the List view groups rows by it: `entry_type`'s description in
//! strata-fs, which GTK's grouped List orders and heads its groups by
//! (`entry_model_value` and `type_group_header_factory` in `src/ui`). A row keeps a small key
//! into one table of the descriptions met, not a string of its own.
//!
//! - **One table, on the interface thread only:** a thread-local, filled and read where the
//!   browser's events are read and the views built, never from a worker.
//! - **Bounded** by the descriptions there are: one for each type the desktop's MIME database
//!   describes, plus Folder, Broken link and Other. A name is added once and never removed.
//! - **A key's number means nothing:** it is the order types were first met. The order of
//!   groups comes from strata-fs's `compare_type_groups`, never from the key.

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use strata_core::{
    i18n::tr,
    model::{EntryKind, Location},
};
use strata_fs::services::{EntryType, entry_type_description_for_name, entry_type_for_name};

/// A file type's description, by its key in the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TypeGroup(u32);

#[derive(Default)]
struct Names {
    names: Vec<Rc<str>>,
    translated: Vec<Arc<str>>,
    keys: HashMap<Rc<str>, u32>,
}

thread_local! {
    static NAMES: RefCell<Names> = RefCell::new(Names::default());
}

impl TypeGroup {
    /// The type of the entry of `kind` named `name` in `folder`, as GTK's grouped List reads
    /// it: strata-fs's `entry_type`, over the two fields it reads (its kind and its name), since
    /// a listing keeps no `FileEntry`.
    pub(crate) fn of_row(_folder: &Location, kind: EntryKind, name: &str) -> Self {
        let description = entry_type_description_for_name(kind, name);
        Self::intern_rc(description)
    }

    fn intern_rc(description: Rc<str>) -> Self {
        NAMES.with_borrow_mut(|table| {
            if let Some(&key) = table.keys.get(description.as_ref()) {
                return Self(key);
            }
            let key = u32::try_from(table.names.len()).unwrap_or(u32::MAX);
            let label: Arc<str> = Arc::from(tr(description.as_ref()));
            table.names.push(description.clone());
            table.translated.push(label);
            table.keys.insert(description, key);
            Self(key)
        })
    }

    /// The type's description, untranslated: what strata-fs orders groups by
    /// (`compare_type_groups`) and the catalogs translate.
    pub(crate) fn description(self) -> Rc<str> {
        NAMES.with_borrow(|table| {
            table
                .names
                .get(self.0 as usize)
                .cloned()
                .unwrap_or_else(|| Rc::from(""))
        })
    }

    /// The type's translated heading label, cached once per interned type.
    pub(crate) fn translated_label(self) -> Arc<str> {
        NAMES.with_borrow(|table| {
            table
                .translated
                .get(self.0 as usize)
                .cloned()
                .unwrap_or_else(|| Arc::from(""))
        })
    }

    /// How many types the table holds.
    #[cfg(test)]
    pub(crate) fn interned() -> usize {
        NAMES.with_borrow(|table| table.names.len())
    }

    /// The key as a group's, for the collection's group headers.
    pub(crate) fn key(self) -> u64 {
        u64::from(self.0)
    }
}

/// strata-fs's `entry_type` of the entry of `kind` named `name`, from the two fields
/// `entry_type` reads (`kind` and `display_name`), avoiding a `Location::child` path join.
pub(crate) fn entry_type_of(_folder: &Location, kind: EntryKind, name: &str) -> EntryType {
    entry_type_for_name(kind, name)
}

#[cfg(test)]
mod tests;
