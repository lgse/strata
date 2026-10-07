// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::{self, Propagation},
    prelude::*,
};

use super::{Dispatcher, KeyResult, command_modifiers};
use crate::{
    model::{Location, SortDirection, SortKey},
    ui::{
        browser::{ConflictFocus, Yank},
        tenxer_mode::{Chord, Prompt},
        window::visible_modal_layer,
    },
};

#[derive(Clone, Default)]
pub(super) struct OpenWithLookup {
    generation: Rc<Cell<u64>>,
    task: Rc<RefCell<Option<glib::JoinHandle<()>>>>,
}

impl OpenWithLookup {
    pub(super) fn invalidate(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(task) = self.task.borrow_mut().take() {
            task.abort();
        }
    }
}

pub(super) struct ArmedActions {
    targets: Vec<Location>,
    ids: Vec<String>,
}

fn sort_choice(key: Key) -> Option<(SortKey, SortDirection)> {
    use SortDirection::{Ascending, Descending};
    Some(match key {
        Key::a => (SortKey::Name, Ascending),
        Key::A => (SortKey::Name, Descending),
        Key::m => (SortKey::Modified, Ascending),
        Key::M => (SortKey::Modified, Descending),
        Key::s => (SortKey::Size, Ascending),
        Key::S => (SortKey::Size, Descending),
        Key::e => (SortKey::Type, Ascending),
        Key::E => (SortKey::Type, Descending),
        _ => return None,
    })
}

fn action_slot(key: Key) -> Option<(char, usize)> {
    let digit = match key {
        Key::_0 | Key::KP_0 => 0,
        Key::_1 | Key::KP_1 => 1,
        Key::_2 | Key::KP_2 => 2,
        Key::_3 | Key::KP_3 => 3,
        Key::_4 | Key::KP_4 => 4,
        Key::_5 | Key::KP_5 => 5,
        Key::_6 | Key::KP_6 => 6,
        Key::_7 | Key::KP_7 => 7,
        Key::_8 | Key::KP_8 => 8,
        Key::_9 | Key::KP_9 => 9,
        _ => return None,
    };
    let slot = if digit == 0 { 10 } else { digit as usize };
    Some((char::from_digit(digit, 10)?, slot))
}

fn slot_key(index: usize) -> String {
    ((index + 1) % 10).to_string()
}

impl Dispatcher {
    pub(super) fn tenxer_file_keys(&self, key: Key, modifiers: Modifiers) -> KeyResult {
        if !self.view.item_view_has_focus() {
            return None;
        }
        // Result navigation also consumes Ctrl+H; toggling must win.
        if crate::ui::window::is_toggle_hidden_shortcut(key, modifiers) {
            self.view.toggle_hidden_from_keys();
            return Some(Propagation::Stop);
        }
        let mods = command_modifiers(modifiers);
        if !mods.is_empty() && mods != Modifiers::SHIFT_MASK {
            return None;
        }
        let shift = mods == Modifiers::SHIFT_MASK;
        match key {
            // Some layouts type these with Shift.
            Key::comma => self.shortcuts.arm_chord(Chord::Sort),
            Key::semicolon => self.arm_actions(),
            Key::period => self.view.toggle_hidden_from_keys(),
            Key::r | Key::F2 if !shift => self.open_rename_prompt(),
            Key::O => self.open_with_targets(),
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
            Key::t if !shift && self.chooser.is_none() => self.shortcuts.arm_chord(Chord::Tabs),
            Key::M => self.open_transfer_prompt(Prompt::MoveTo),
            Key::C => self.open_transfer_prompt(Prompt::CopyTo),
            Key::R => self.restore_targets(),
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

    /// Pre-fills the name with the stem of a file, or all of a folder's name,
    /// selected as inline rename does. The item is fixed now, so a later
    /// cursor move cannot redirect the rename.
    fn open_rename_prompt(&self) {
        if self.chooser_edit_name() {
            return;
        }
        let Some(entry) = self.view.focused_target() else {
            self.shortcuts.show_feedback("Nothing to rename");
            return;
        };
        if !crate::ui::browser::can_rename(&entry) {
            self.shortcuts
                .show_feedback("Can\u{2019}t rename items here");
            return;
        }
        let name = entry.display_name.clone();
        let end = if entry.is_directory() {
            -1
        } else {
            crate::ui::collection_edit::rename_stem_end(&name)
        };
        if self.shortcuts.open_prompt_with(Prompt::Rename, &name) {
            self.shortcuts.select_prompt_region(0, end);
        }
        self.rename_target.replace(Some(entry));
    }

    fn open_transfer_prompt(&self, kind: Prompt) {
        match self.view.transfer_targets(kind == Prompt::MoveTo) {
            Ok(targets) => self.open_destination_prompt(kind, targets),
            Err(reason) => self.shortcuts.show_feedback(reason),
        }
    }

    fn open_destination_prompt(&self, kind: Prompt, targets: Vec<crate::model::FileEntry>) {
        if self.shortcuts.open_prompt(kind) {
            self.destination_targets.replace(targets);
        }
    }

    fn restore_targets(&self) {
        use crate::ui::browser::TargetCommand;
        match self.view.restore_targets() {
            TargetCommand::Nothing => self.shortcuts.show_feedback("Nothing to restore"),
            TargetCommand::Refused => self
                .shortcuts
                .show_feedback("Only items in Trash can be restored"),
            TargetCommand::Started => {}
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

impl Dispatcher {
    pub(super) fn complete_sort(&self, key: Key) -> bool {
        let Some((sort_key, direction)) = sort_choice(key) else {
            return false;
        };
        self.view.sort_focused_pane(sort_key, direction);
        true
    }

    fn arm_actions(&self) {
        let numbered = self.view.numbered_actions();
        let mut rows = if numbered.actions.is_empty() {
            vec![("1\u{2013}0".to_owned(), "No matching actions".to_owned())]
        } else {
            numbered
                .actions
                .iter()
                .enumerate()
                .map(|(index, action)| (slot_key(index), action.name().to_owned()))
                .collect()
        };
        rows.extend(
            [
                ("t", "Open terminal here"),
                ("c", "Compress\u{2026}"),
                ("e", "Extract here"),
                ("E", "Extract to\u{2026}"),
            ]
            .map(|(key, action)| (key.to_owned(), action.to_owned())),
        );
        self.armed_actions.replace(Some(ArmedActions {
            ids: numbered
                .actions
                .iter()
                .map(|action| action.id().to_owned())
                .collect(),
            targets: numbered.targets,
        }));
        self.shortcuts.arm_chord_with(Chord::Action, rows);
    }

    pub(super) fn complete_action(&self, key: Key) -> bool {
        if self.complete_folder_action(key) {
            self.armed_actions.take();
            return true;
        }
        let Some((digit, slot)) = action_slot(key) else {
            return false;
        };
        let Some(armed) = self.armed_actions.take() else {
            return false;
        };
        let numbered = self.view.numbered_actions();
        if numbered.targets != armed.targets {
            self.shortcuts.show_feedback("Selection changed");
            return true;
        }
        let Some(action) = numbered.actions.get(slot - 1).cloned() else {
            self.shortcuts.show_feedback(&format!("No action {digit}"));
            return true;
        };
        if armed.ids.get(slot - 1).map(String::as_str) != Some(action.id()) {
            self.shortcuts.show_feedback("Actions changed");
            return true;
        }
        self.view.run_numbered_action(numbered, action);
        true
    }

    fn complete_folder_action(&self, key: Key) -> bool {
        use crate::ui::browser::TargetCommand;
        match key {
            Key::t => {
                if !self.view.open_focused_folder_terminal() {
                    self.shortcuts
                        .show_feedback("Can\u{2019}t open a terminal here");
                }
            }
            Key::c => match self.view.compress_targets() {
                TargetCommand::Nothing => self.shortcuts.show_feedback("Nothing to compress"),
                TargetCommand::Refused => self
                    .shortcuts
                    .show_feedback("Can\u{2019}t compress these items"),
                TargetCommand::Started => {}
            },
            Key::e => match self.view.extract_target() {
                Ok(entry) => self.view.extract_here(entry),
                Err(reason) => self.shortcuts.show_feedback(reason),
            },
            Key::E => match self.view.extract_target() {
                Ok(entry) => self.open_destination_prompt(Prompt::ExtractTo, vec![entry]),
                Err(reason) => self.shortcuts.show_feedback(reason),
            },
            _ => return false,
        }
        true
    }

    fn open_with_targets(&self) {
        self.open_with.invalidate();
        let generation = self.open_with.generation.clone();
        let expected = generation.get();
        let window = self.window.downgrade();
        let focus = gtk::prelude::RootExt::focus(&self.window).map(|widget| widget.downgrade());
        let current = Rc::new(move || {
            generation.get() == expected
                && window.upgrade().is_some_and(|window| {
                    window.is_visible()
                        && visible_modal_layer(&window).is_none()
                        && gtk::prelude::RootExt::focus(&window)
                            == focus.as_ref().and_then(glib::WeakRef::upgrade)
                })
        });
        let shortcuts = self.shortcuts.clone();
        let report = Rc::new(move |reason: &str| shortcuts.show_feedback(reason));
        match self.view.open_targets_with(current, report) {
            Some(task) => {
                self.open_with.task.replace(Some(task));
            }
            None => self.shortcuts.show_feedback("Nothing to open"),
        }
    }
}
