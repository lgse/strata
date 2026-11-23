// SPDX-License-Identifier: MIT

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
