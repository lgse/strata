// SPDX-License-Identifier: MIT

//! A text field as the engine reads one: its name, what it holds, and where its caret and
//! its selection are.
//!
//! The protocol counts a field in characters, as AT-SPI's text interface does and the GTK
//! harness reports. On the focused field Rune exposes the selection's anchor and caret in
//! UTF-8 bytes of its text (`ElementInfo::edit_range`), which gives character offsets across
//! any line. For an unfocused field Rune's tree reports a line and a count of the clusters it
//! shaped on that line (`é` written as a letter and a combining accent is one cluster and two
//! characters), turned into characters over the first line's grapheme clusters.

use strata_qa::FieldState;
use unicode_segmentation::UnicodeSegmentation;

use super::tree::Seen;

/// The roles of Rune's text fields.
const ROLES: [&str; 4] = [
    "text-input",
    "multiline-text-input",
    "search-input",
    "password-input",
];

/// Rune's number for a field's first line.
const FIRST_LINE: u64 = 1;

pub(super) fn is_one(role: &str) -> bool {
    ROLES.contains(&role)
}

/// How many characters come before cluster `cluster` of `text`.
fn characters_before(text: &str, cluster: u64) -> i32 {
    let cluster = usize::try_from(cluster).unwrap_or(usize::MAX);
    let end = text
        .grapheme_indices(true)
        .nth(cluster)
        .map_or(text.len(), |(start, _)| start);
    i32::try_from(text[..end].chars().count()).unwrap_or(i32::MAX)
}

/// How many characters come before UTF-8 byte `byte` of `text`.
fn characters_to_byte(text: &str, byte: usize) -> Option<i32> {
    let before = text.get(..byte)?;
    i32::try_from(before.chars().count()).ok()
}

/// The two ends of the selection in characters: the one that stays, and the caret.
fn ends(seen: &Seen, text: &str) -> Option<(i32, i32)> {
    if let Some([anchor, caret]) = seen.edit_range {
        return Some((
            characters_to_byte(text, anchor)?,
            characters_to_byte(text, caret)?,
        ));
    }
    match seen.selection {
        Some([FIRST_LINE, anchor, FIRST_LINE, caret]) => Some((
            characters_before(text, anchor),
            characters_before(text, caret),
        )),
        Some(_) => None,
        // An empty field has no line to place a caret on.
        None => text.is_empty().then_some((0, 0)),
    }
}

/// What the field `seen` holds. `None` for a node that is no field, and for an unfocused field
/// whose caret the tree does not place: one on a later line, or with too much text for Rune to
/// report its lines.
pub(super) fn state(seen: &Seen, focused: bool) -> Option<FieldState> {
    if !is_one(&seen.role) {
        return None;
    }
    let text = seen.value.clone().unwrap_or_default();
    let (anchor, caret) = ends(seen, &text)?;
    Some(FieldState {
        name: seen.name.clone(),
        selection: (anchor != caret).then(|| [anchor.min(caret), anchor.max(caret)]),
        caret,
        focused,
        showing: seen.bounds.is_some(),
        text,
    })
}

#[cfg(test)]
mod tests;
