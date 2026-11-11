// SPDX-License-Identifier: MIT

//! The rules of a field a name is typed into: what an inline rename selects when it opens,
//! and what it says of the text as it changes. Every interface asks them here, so a name is
//! selected and refused alike in each.

#[cfg(test)]
mod tests;

use super::{TypedPath, validate_basename};

/// How a name typed for a local entry may move it; elsewhere it is a single name.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Naming {
    /// A relative path moves the item below its folder.
    #[default]
    Path,
    /// The entry was just created, so a file may also be named as a folder.
    NewItem,
    Single,
}

/// Where a rename's first selection ends, counted in characters from the start of `name`:
/// before the last dot, unless that dot leads the name, and at the end of a name without one.
pub fn rename_stem_end(name: &str) -> i32 {
    let end = name
        .rfind('.')
        .filter(|position| *position > 0)
        .unwrap_or(name.len());
    i32::try_from(name[..end].chars().count()).unwrap_or(i32::MAX)
}

/// Empty fields are an ordinary editing state, although they cannot be submitted.
pub fn name_field_error(name: &str, naming: Naming, is_directory: bool) -> Option<&'static str> {
    match naming {
        _ if name.is_empty() => None,
        Naming::Single => validate_basename(name).err(),
        Naming::Path => TypedPath::parse(name, is_directory, false).err(),
        Naming::NewItem => TypedPath::parse(name, is_directory, true).err(),
    }
}
