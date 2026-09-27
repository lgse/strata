// SPDX-License-Identifier: MIT

//! 10xer file commands from the listing: yank, cut, unyank, paste, delete,
//! create, and the **c** copy chord.

use std::rc::Rc;

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::Propagation,
};

use super::{Dispatcher, KeyResult, command_modifiers};
use crate::ui::{
    browser::{ConflictFocus, Yank},
    tenxer_mode::{Chord, Prompt},
};

impl Dispatcher {
    pub(super) fn tenxer_file_keys(&self, key: Key, modifiers: Modifiers) -> KeyResult {
        if !self.view.item_view_has_focus() {
            return None;
        }
        let mods = command_modifiers(modifiers);
        if !mods.is_empty() && mods != Modifiers::SHIFT_MASK {
            return None;
        }
        let shift = mods == Modifiers::SHIFT_MASK;
        match key {
            Key::Delete | Key::KP_Delete => self.delete_targets(shift),
            Key::y if !shift => self.yank(false),
            Key::x if !shift => self.yank(true),
            Key::Y | Key::X => self.view.unyank(),
            Key::p if !shift => self.paste(ConflictFocus::KeepBoth),
            Key::P => self.paste(ConflictFocus::Replace),
            Key::d if !shift => self.delete_targets(false),
            Key::D => self.delete_targets(true),
            Key::a if !shift => self.open_create_prompt(),
            Key::c if !shift => self.shortcuts.arm_chord(Chord::Copy),
            _ => return None,
        }
        Some(Propagation::Stop)
    }

    fn yank(&self, cut: bool) {
        match self.view.yank_targets(cut) {
            Yank::Nothing => self.shortcuts.show_feedback(if cut {
                "Nothing to cut"
            } else {
                "Nothing to yank"
            }),
            Yank::Refused => self.shortcuts.show_feedback("Can\u{2019}t cut these items"),
            Yank::Done => {}
        }
    }

    /// Paste leaves pointer or keyboard ownership of the destination as it is.
    fn paste(&self, focus: ConflictFocus) {
        let shortcuts = self.shortcuts.clone();
        self.view.paste_preferring(
            focus,
            Rc::new(move || shortcuts.show_feedback("Nothing to paste")),
        );
    }

    fn delete_targets(&self, permanent: bool) {
        if !self.view.confirm_targets_delete(permanent) {
            self.shortcuts.show_feedback("Nothing to delete");
        }
    }

    fn open_create_prompt(&self) {
        if self.view.can_create_entry() {
            self.shortcuts.open_prompt(Prompt::Create);
        } else {
            self.shortcuts
                .show_feedback("Can\u{2019}t create items here");
        }
    }

    /// **c c** / **c n**.
    pub(super) fn complete_copy(&self, key: Key) -> bool {
        let names = match key {
            Key::c => false,
            Key::n => true,
            _ => return false,
        };
        if !self.view.copy_target_text(names) {
            self.shortcuts.show_feedback("Nothing to copy");
        }
        true
    }
}
