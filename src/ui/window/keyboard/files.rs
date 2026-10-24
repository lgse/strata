// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{
    glib::{self, Propagation},
    prelude::*,
};

use super::{Dispatcher, KeyResult};
use crate::{
    model::{Location, SortDirection, SortKey},
    services::InvocationSource,
    ui::{
        browser::{ConflictFocus, Yank},
        keys::{Digits, KeyPress, bindings::*},
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

fn sort_choice(press: &KeyPress) -> Option<(SortKey, SortDirection)> {
    use SortDirection::{Ascending, Descending};
    [
        (SORT_BY_NAME, (SortKey::Name, Ascending)),
        (SORT_BY_NAME_REVERSED, (SortKey::Name, Descending)),
        (SORT_BY_MODIFIED, (SortKey::Modified, Ascending)),
        (SORT_BY_MODIFIED_REVERSED, (SortKey::Modified, Descending)),
        (SORT_BY_SIZE, (SortKey::Size, Ascending)),
        (SORT_BY_SIZE_REVERSED, (SortKey::Size, Descending)),
        (SORT_BY_TYPE, (SortKey::Type, Ascending)),
        (SORT_BY_TYPE_REVERSED, (SortKey::Type, Descending)),
    ]
    .into_iter()
    .find(|(binding, _)| press.is(*binding))
    .map(|(_, choice)| choice)
}

/// 0 is slot 10.
fn action_slot(press: &KeyPress, digits: Digits) -> Option<(u8, usize)> {
    let digit = press.digit_in(digits)?;
    let slot = if digit == 0 { 10 } else { usize::from(digit) };
    Some((digit, slot))
}

fn slot_key(index: usize) -> String {
    use crate::ui::keys::{self, bindings::ACTIONS_NUMBERS};
    keys::digit_label(ACTIONS_NUMBERS.step(), ((index + 1) % 10) as u8)
}

impl Dispatcher {
    pub(super) fn tenxer_file_keys(&self, press: &KeyPress) -> KeyResult {
        if !self.view.item_view_has_focus() {
            return None;
        }
        // Result navigation also consumes Ctrl+H; toggling must win.
        if press.is_any(&[HIDDEN_FILES, HIDDEN_FILES_PERIOD, TENXER_HIDDEN_FILES]) {
            self.view.toggle_hidden_from_keys();
        } else if press.is(SORT_BY) {
            self.shortcuts.arm_chord(Chord::Sort);
        } else if press.is(ACTIONS) {
            self.arm_actions(InvocationSource::Selection);
        } else if press.is_any(&[TENXER_RENAME, RENAME]) {
            self.open_rename_prompt();
        } else if press.is(OPEN_WITH) {
            self.open_with_targets();
        } else if press.is_any(&[DELETE, TENXER_DELETE]) {
            self.delete_targets(false);
        } else if press.is_any(&[DELETE_PERMANENTLY, TENXER_DELETE_PERMANENTLY]) {
            self.delete_targets(true);
        } else if press.is(YANK) {
            self.yank(false);
        } else if press.is(YANK_CUT) {
            self.yank(true);
        } else if press.is_any(&[UNYANK, UNYANK_X]) {
            self.view.unyank();
        } else if press.is(PASTE_KEEP_BOTH) {
            self.paste(ConflictFocus::KeepBoth);
        } else if press.is(PASTE_REPLACE) {
            self.paste(ConflictFocus::Replace);
        } else if press.is(CREATE) {
            self.open_create_prompt();
        } else if press.is(COPY_AS) {
            self.shortcuts.arm_chord(Chord::Copy);
        } else if press.is(TAB_CHORD) && self.chooser.is_none() {
            self.shortcuts.arm_chord(Chord::Tabs);
        } else if press.is(MOVE_TO) {
            self.open_transfer_prompt(Prompt::MoveTo);
        } else if press.is(COPY_TO) {
            self.open_transfer_prompt(Prompt::CopyTo);
        } else if press.is(RESTORE) {
            self.restore_targets();
        } else {
            return None;
        }
        Some(Propagation::Stop)
    }

    fn yank(&self, cut: bool) {
        match self.view.yank_targets(cut) {
            Yank::Nothing => self.shortcuts.show_feedback(&crate::i18n::tr(if cut {
                "Nothing to cut"
            } else {
                "Nothing to yank"
            })),
            Yank::Refused => self
                .shortcuts
                .show_feedback(&crate::i18n::tr("Can’t cut these items")),
            Yank::Done => {}
        }
    }

    fn paste(&self, focus: ConflictFocus) {
        let shortcuts = self.shortcuts.clone();
        self.view.paste_preferring(
            focus,
            Rc::new(move || shortcuts.show_feedback(&crate::i18n::tr("Nothing to paste"))),
        );
    }

    fn delete_targets(&self, permanent: bool) {
        if !self.view.confirm_targets_delete(permanent) {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to delete"));
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
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to rename"));
            return;
        };
        if !crate::ui::browser::can_rename(&entry) {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Can’t rename items here"));
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
            Err(reason) => self.shortcuts.show_feedback(&crate::i18n::tr(reason)),
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
            TargetCommand::Nothing => self
                .shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to restore")),
            TargetCommand::Refused => self
                .shortcuts
                .show_feedback(&crate::i18n::tr("Only items in Trash can be restored")),
            TargetCommand::Started => {}
        }
    }

    fn open_create_prompt(&self) {
        if self.view.can_create_entry() {
            self.shortcuts.open_prompt(Prompt::Create);
        } else {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Can’t create items here"));
        }
    }

    pub(super) fn complete_copy(&self, press: &KeyPress) -> bool {
        let names = if press.is(COPY_AS_PATH) {
            false
        } else if press.is(COPY_AS_NAME) {
            true
        } else {
            return false;
        };
        if !self.view.copy_target_text(names) {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to copy"));
        }
        true
    }
}

impl Dispatcher {
    pub(super) fn complete_sort(&self, press: &KeyPress) -> bool {
        let Some((sort_key, direction)) = sort_choice(press) else {
            return false;
        };
        self.view.sort_focused_pane(sort_key, direction);
        true
    }

    fn arm_actions(&self, source: InvocationSource) {
        use crate::ui::keys::{
            self, Keys,
            bindings::{
                ACTIONS_COMPRESS, ACTIONS_EXTRACT, ACTIONS_EXTRACT_TO, ACTIONS_FOLDER,
                ACTIONS_NUMBERS, ACTIONS_TERMINAL,
            },
        };
        let numbered = self.view.numbered_actions(source);
        let mut rows = if numbered.actions.is_empty() {
            vec![(
                keys::label(&Keys::Digits(ACTIONS_NUMBERS.step())),
                crate::i18n::tr("No matching actions"),
            )]
        } else {
            numbered
                .actions
                .iter()
                .enumerate()
                .map(|(index, action)| (slot_key(index), action.name().to_owned()))
                .collect()
        };
        let chord = match source {
            InvocationSource::Selection => {
                rows.extend(
                    [
                        (ACTIONS_FOLDER, "Actions for this folder\u{2026}"),
                        (ACTIONS_TERMINAL, "Open terminal here"),
                        (ACTIONS_COMPRESS, "Compress\u{2026}"),
                        (ACTIONS_EXTRACT, "Extract here"),
                        (ACTIONS_EXTRACT_TO, "Extract to\u{2026}"),
                    ]
                    .map(|(step, action)| (keys::label(&Keys::One(step)), crate::i18n::tr(action))),
                );
                Chord::Action
            }
            InvocationSource::Background => Chord::FolderAction,
        };
        self.armed_actions.replace(Some(ArmedActions {
            ids: numbered
                .actions
                .iter()
                .map(|action| action.id().to_owned())
                .collect(),
            targets: numbered.targets,
        }));
        self.shortcuts.arm_chord_with(chord, rows);
    }

    pub(super) fn complete_action(&self, press: &KeyPress) -> bool {
        if press.is(ACTIONS_FOLDER) {
            self.arm_actions(InvocationSource::Background);
            return true;
        }
        if self.complete_lettered_action(press) {
            self.armed_actions.take();
            return true;
        }
        self.complete_numbered_action(press, InvocationSource::Selection)
    }

    pub(super) fn complete_numbered_action(
        &self,
        press: &KeyPress,
        source: InvocationSource,
    ) -> bool {
        let (digits, changed) = match source {
            InvocationSource::Selection => (ACTIONS_NUMBERS, "Selection changed"),
            InvocationSource::Background => (ACTIONS_FOLDER_NUMBERS, "Folder changed"),
        };
        let Some((digit, slot)) = action_slot(press, digits) else {
            return false;
        };
        let Some(armed) = self.armed_actions.take() else {
            return false;
        };
        let numbered = self.view.numbered_actions(source);
        if numbered.targets != armed.targets {
            self.shortcuts.show_feedback(&crate::i18n::tr(changed));
            return true;
        }
        let Some(action) = numbered.actions.get(slot - 1).cloned() else {
            self.shortcuts
                .show_feedback(&rust_i18n::t!("No action %{digit}", digit = digit));
            return true;
        };
        if armed.ids.get(slot - 1).map(String::as_str) != Some(action.id()) {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Actions changed"));
            return true;
        }
        self.view.run_numbered_action(numbered, action);
        true
    }

    fn complete_lettered_action(&self, press: &KeyPress) -> bool {
        use crate::ui::browser::TargetCommand;
        if press.is(ACTIONS_TERMINAL) {
            if !self.view.open_focused_folder_terminal() {
                self.shortcuts
                    .show_feedback(&crate::i18n::tr("Can’t open a terminal here"));
            }
        } else if press.is(ACTIONS_COMPRESS) {
            match self.view.compress_targets() {
                TargetCommand::Nothing => self
                    .shortcuts
                    .show_feedback(&crate::i18n::tr("Nothing to compress")),
                TargetCommand::Refused => self
                    .shortcuts
                    .show_feedback(&crate::i18n::tr("Can’t compress these items")),
                TargetCommand::Started => {}
            }
        } else if press.is(ACTIONS_EXTRACT) {
            match self.view.extract_target() {
                Ok(entry) => self.view.extract_here(entry),
                Err(reason) => self.shortcuts.show_feedback(&crate::i18n::tr(reason)),
            }
        } else if press.is(ACTIONS_EXTRACT_TO) {
            match self.view.extract_target() {
                Ok(entry) => self.open_destination_prompt(Prompt::ExtractTo, vec![entry]),
                Err(reason) => self.shortcuts.show_feedback(&crate::i18n::tr(reason)),
            }
        } else {
            return false;
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
        let report = Rc::new(move |reason: &str| shortcuts.show_feedback(&crate::i18n::tr(reason)));
        match self.view.open_targets_with(current, report) {
            Some(task) => {
                self.open_with.task.replace(Some(task));
            }
            None => self
                .shortcuts
                .show_feedback(&crate::i18n::tr("Nothing to open")),
        }
    }
}
