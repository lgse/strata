// SPDX-License-Identifier: MIT

//! Names the user types for an item, refused before anything changes where
//! the location would store another name.

use std::path::Path;

use gtk::gio;

use super::{
    fs_ops::{Entry, fs},
    mount_profile::folder_name_rules,
    names::fix::{Refusal, refusal},
    native_fs::open_local_parent_directory,
    placement,
};
use crate::adapters::volume::NameProfile;

pub(super) fn refused(name: &str, rules: &NameProfile, new_folder: bool) -> Option<String> {
    Some(match refusal(name, rules, new_folder)? {
        Refusal::Characters(characters) => rust_i18n::t!(
            "Names in this location can’t contain %{characters}",
            characters = crate::i18n::list(
                characters
                    .into_iter()
                    .map(|character| crate::i18n::quoted(&shown(character)))
            )
        )
        .into_owned(),
        Refusal::LeadingSpace => crate::i18n::tr("Names in this location can’t start with a space"),
        Refusal::TrailingDotOrSpace => {
            crate::i18n::tr("Names in this location can’t end with a space or a period")
        }
        Refusal::Reserved(word) => {
            rust_i18n::t!("“%{name}” is a reserved name in this location", name = word).into_owned()
        }
        Refusal::TooLong => crate::i18n::tr("The name is too long"),
    })
}

pub(crate) async fn refused_in(folder: &gio::File, name: &str, new_folder: bool) -> Option<String> {
    refused(name, &folder_name_rules(folder).await?, new_folder)
}

pub(crate) fn rename_taken(current: &Path, wanted: &Path) -> bool {
    let entry = |path: &Path| {
        let dir = open_local_parent_directory(path.parent()?).ok()?;
        Some((dir, path.file_name()?.to_owned()))
    };
    let Some((wanted_dir, wanted_name)) = entry(wanted) else {
        return false;
    };
    let to = Entry::new(&wanted_dir, &wanted_name);
    if fs().stat(to, false).is_err() {
        return false;
    }
    entry(current)
        .is_none_or(|(dir, name)| placement::alias_of(fs(), Entry::new(&dir, &name), to).is_none())
}

/// Its code point where it shows nothing of its own: control and format characters, spaces,
/// marks that join the character before, and private-use or unassigned code points.
fn shown(character: char) -> String {
    use glib::{Unichar, UnicodeType};
    if matches!(
        character.unicode_type(),
        UnicodeType::Control
            | UnicodeType::Format
            | UnicodeType::Unassigned
            | UnicodeType::PrivateUse
            | UnicodeType::Surrogate
            | UnicodeType::NonSpacingMark
            | UnicodeType::SpacingMark
            | UnicodeType::EnclosingMark
            | UnicodeType::SpaceSeparator
            | UnicodeType::LineSeparator
            | UnicodeType::ParagraphSeparator
    ) {
        format!("U+{:04X}", u32::from(character))
    } else {
        character.to_string()
    }
}
