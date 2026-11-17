// SPDX-License-Identifier: MIT

//! What a conflict finding means for the user, naming the side that loses the key where the
//! checker knows it: GTK's `message` (`src/ui/settings/keyboard.rs`), word for word.

use strata_core::i18n::tr;
use strata_desktop::keys::{
    self, Binding, Command, Keys, Map, Modifiers,
    bindings::{ACTIONS_NUMBERS, PIN_NUMBERS, TAB_NUMBERS},
    conflicts::{self, FieldLimit, Finding, Fixed, Kind, Outcome},
};

use super::{names::title, sections::trigger};

/// A binding's label as the reference and the chips show it.
pub(super) fn key_label(binding: Binding) -> String {
    keys::label(&Keys::One(binding.shown()))
}

pub(super) fn message(map: Map, command: Command, finding: Finding) -> String {
    let keys = key_label(finding.binding);
    let keys = keys.as_str();
    match finding.kind {
        Kind::Duplicate(other) => duplicate((map, command), finding.binding, other, keys),
        Kind::ShadowsChordGroup(group) => rust_i18n::t!(
            "%{keys} also starts “%{group}”, so either this key or that chord never runs.",
            keys = keys,
            group = trigger(group).map(title).unwrap_or_default()
        )
        .into_owned(),
        Kind::ShiftedSortStep => shifted(
            "%{shift} with a sort key reverses the sort, so %{keys} cannot be one.",
            keys,
        ),
        Kind::SortStepCannotReverse => {
            shifted("%{keys} sorts, but %{shift} cannot reverse it.", keys)
        }
        Kind::ContextOverlap(other) => rust_i18n::t!(
            "“%{other}” also uses %{keys}; where both apply, only one of them acts.",
            keys = keys,
            other = title(other)
        )
        .into_owned(),
        Kind::FixedKey(Fixed::Digits(digits)) => digit_range(map, digits, keys),
        Kind::NoPath => tr(match map {
            Map::Default => "Only 10xer mode runs this action, so its keys do nothing here.",
            Map::Tenxer => "10xer mode does not run this action, so its keys do nothing here.",
        }),
        other => rust_i18n::t!(plain_message(other), keys = keys).into_owned(),
    }
}

fn duplicate(
    (map, command): (Map, Command),
    binding: Binding,
    other: Command,
    keys: &str,
) -> String {
    let message = match conflicts::duplicate_outcome(map, command, binding, other) {
        Outcome::Wins => "Takes %{keys} from “%{other}”.",
        Outcome::Loses => "“%{other}” takes %{keys} first.",
        Outcome::Either => "“%{other}” also uses %{keys}.",
    };
    rust_i18n::t!(message, keys = keys, other = title(other)).into_owned()
}

fn shifted(message: &str, keys: &str) -> String {
    let shift = keys::modifiers_label(Modifiers::SHIFT_MASK);
    rust_i18n::t!(message, keys = keys, shift = shift).into_owned()
}

/// The message of a finding that names only its keys.
fn plain_message(kind: Kind) -> &'static str {
    match kind {
        Kind::TypeToSearchShadowed => "%{keys} does nothing while type-to-search is on.",
        Kind::TextFieldLimit(FieldLimit::Typing) => {
            "Doesn't work while a text field or menu has focus."
        }
        Kind::TextFieldLimit(FieldLimit::FieldKey) => {
            "Text fields use %{keys} for editing, so there it usually edits text instead."
        }
        Kind::ReservedByGtk => {
            "File lists use %{keys} to change the selection, which stops working where this key acts."
        }
        Kind::InputMethodKey => {
            "Input methods need %{keys} while you type, so it would interrupt typing."
        }
        Kind::ChooserMnemonic => "The file chooser may use %{keys} for its accept button instead.",
        Kind::FixedKey(Fixed::SelectionToggle) => {
            "%{keys} selects the item under the cursor, so it never runs this action."
        }
        Kind::FixedKey(Fixed::MediaKeys) => {
            "Media previews use %{keys} for playback first, so it never runs this action there."
        }
        Kind::FixedKey(Fixed::Backspace) => {
            "%{keys} goes to the parent folder and deletes text, which stops working where this key acts."
        }
        _ => "%{keys} never reaches this action, so it does nothing.",
    }
}

fn digit_range(map: Map, digits: keys::Digits, keys: &str) -> String {
    let message = if digits == TAB_NUMBERS {
        "%{digits} select tabs before anything else reads them, so %{keys} never runs this action."
    } else if digits == PIN_NUMBERS {
        "%{digits} open pinned folders, and this key takes %{keys} from them."
    } else if digits == ACTIONS_NUMBERS {
        "%{digits} run custom actions, and this key takes %{keys} from them."
    } else {
        "%{digits} select tabs, and this key takes %{keys} from them."
    };
    let locale = rust_i18n::locale();
    let mut range = keys::label_in_map(&locale, &Keys::Digits(digits), map);
    if range.is_empty() {
        // The chord's trigger has no key in this table.
        range = keys::label_in_map(&locale, &Keys::Digits(digits.step()), map);
    }
    rust_i18n::t!(message, keys = keys, digits = range).into_owned()
}
