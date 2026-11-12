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

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};
use strata_fs::services::entry_type;

/// A file type's description, by its key in the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TypeGroup(u32);

#[derive(Default)]
struct Names {
    names: Vec<Rc<str>>,
    keys: HashMap<Rc<str>, u32>,
}

thread_local! {
    static NAMES: RefCell<Names> = RefCell::new(Names::default());
}

impl TypeGroup {
    /// The type of the entry of `kind` named `name` in `folder`, as GTK's grouped List reads
    /// it: strata-fs's `entry_type`, over the two fields it reads (its kind and its name), since
    /// a listing keeps no `FileEntry`.
    pub(crate) fn of_row(folder: &Location, kind: EntryKind, name: &str) -> Self {
        let location = folder
            .child(std::ffi::OsStr::new(name))
            .unwrap_or_else(|| folder.clone());
        Self::of(&entry(&location, kind, name))
    }

    /// The type of `entry`, as GTK's grouped List reads it.
    pub(crate) fn of(entry: &FileEntry) -> Self {
        let kind = entry_type(entry);
        let description = kind.description();
        NAMES.with_borrow_mut(|table| {
            if let Some(key) = table.keys.get(description) {
                return Self(*key);
            }
            let key = u32::try_from(table.names.len()).unwrap_or(u32::MAX);
            let name: Rc<str> = Rc::from(description);
            table.names.push(name.clone());
            table.keys.insert(name, key);
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

/// An entry at `location` with what `entry_type` reads, its kind and its name, and nothing else
/// known.
fn entry(location: &Location, kind: EntryKind, name: &str) -> FileEntry {
    FileEntry {
        location: location.clone(),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.to_owned(),
        kind,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[cfg(test)]
mod tests;
