// SPDX-License-Identifier: MIT

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

pub(crate) fn entry_type_of(_folder: &Location, kind: EntryKind, name: &str) -> EntryType {
    entry_type_for_name(kind, name)
}

#[cfg(test)]
mod tests;
