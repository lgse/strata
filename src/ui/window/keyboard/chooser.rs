// SPDX-License-Identifier: MIT

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::Propagation,
};

use super::{Dispatcher, KeyResult, command_modifiers};
use crate::{app::Browser, model::Location};

pub(super) const UNAVAILABLE: &str = "Not available in the file chooser";
const SINGLE: &str = "Only one item can be chosen";
const LOCAL_ONLY: &str = "Only local folders can be opened here";

pub(in crate::ui::window) fn refusal(
    key: Key,
    modifiers: Modifiers,
    multiple: bool,
    listing_focused: bool,
) -> Option<&'static str> {
    let mods = command_modifiers(modifiers);
    let plain = mods.is_empty();
    let shift = mods == Modifiers::SHIFT_MASK;
    let control = mods == Modifiers::CONTROL_MASK;
    if key == Key::Q && shift {
        return Some(UNAVAILABLE);
    }
    if !listing_focused {
        return None;
    }
    let unavailable = match key {
        Key::y | Key::p | Key::i => plain,
        Key::x => plain || control,
        Key::c | Key::v => control,
        Key::semicolon => plain || shift,
        Key::Y | Key::X | Key::P | Key::O | Key::M | Key::C | Key::R => shift,
        _ => false,
    };
    if unavailable {
        return Some(UNAVAILABLE);
    }
    let fills = match key {
        Key::space | Key::v => plain,
        Key::V => shift,
        Key::a | Key::A | Key::r | Key::R => control,
        _ => false,
    };
    (fills && !multiple).then_some(SINGLE)
}

pub(in crate::ui::window) fn opens_here(location: &Location) -> bool {
    location.native_path().is_some() || location.is_recent_root()
}

impl Dispatcher {
    pub(super) fn chooser_refusal(&self, key: Key, modifiers: Modifiers) -> KeyResult {
        let policy = self.chooser.as_ref()?;
        let reason = refusal(
            key,
            modifiers,
            policy.multiple,
            self.view.item_view_has_focus(),
        )?;
        self.shortcuts.show_feedback(&crate::i18n::tr(reason));
        Some(Propagation::Stop)
    }

    pub(super) fn refuse_remote_place(&self, location: &Location) -> bool {
        if self.chooser.is_none() || opens_here(location) {
            return false;
        }
        self.shortcuts.show_feedback(&crate::i18n::tr(LOCAL_ONLY));
        true
    }

    pub(super) fn chooser_save(&self, key: Key, modifiers: Modifiers) -> KeyResult {
        let save = self.chooser.as_ref()?.save.as_ref()?;
        if !matches!(key, Key::Return | Key::KP_Enter)
            || !command_modifiers(modifiers).is_empty()
            || !self.view.item_view_has_focus()
        {
            return None;
        }
        save();
        Some(Propagation::Stop)
    }

    pub(super) fn chooser_edit_name(&self) -> bool {
        let Some(edit_name) = self
            .chooser
            .as_ref()
            .and_then(|policy| policy.edit_name.as_ref())
        else {
            return false;
        };
        edit_name();
        true
    }

    pub(super) fn chooser_confirm(&self, browser: &Browser) -> bool {
        let Some(policy) = self.chooser.as_ref() else {
            return false;
        };
        let entry = if self.view.selected_search_results().is_some() {
            self.view.selected_search_result()
        } else {
            browser.focused_entry()
        };
        let Some(entry) = entry.filter(|entry| !entry.is_directory()) else {
            return false;
        };
        (policy.confirm)(entry);
        true
    }

    pub(super) fn chooser_cancel(&self) -> bool {
        let Some(policy) = self.chooser.as_ref() else {
            return false;
        };
        (policy.cancel)();
        true
    }
}
