// SPDX-License-Identifier: MIT

//! Armed 10xer chords. Once a chord's mark shows in the footer, the next key
//! completes or cancels that chord and reaches no other handler. Text-size
//! shortcuts, shortcut-reference keys, and Escape while autoscroll is running
//! are earlier capture handlers: they drop the mark first, then run, the same
//! way Ctrl+, opens Settings over a canceled chord.

use std::rc::Rc;

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::{self, Propagation},
};

use super::{Dispatcher, KeyResult, command_modifiers, items::is_modifier_key};
use crate::{
    app::Browser,
    model::Location,
    ui::{
        tenxer_mode::{Chord, Prompt},
        window::home_directory,
    },
};

#[derive(Debug, PartialEq, Eq)]
pub(in crate::ui::window) enum GoTarget {
    FirstItem,
    /// The folder holding the search hit under the cursor.
    HitFolder,
    Prompt,
    Pin(bool),
    /// `validate` routes URI places through mount-aware validation.
    Place {
        location: Location,
        validate: bool,
    },
    Missing(String),
}

/// Resolves the second key of **g**. `pins` are the visible PINNED rows in
/// sidebar display order.
pub(in crate::ui::window) fn go_target(key: Key, pins: &[Location]) -> Option<GoTarget> {
    let place = |location, validate| Some(GoTarget::Place { location, validate });
    match key {
        Key::g => Some(GoTarget::FirstItem),
        Key::f => Some(GoTarget::HitFolder),
        Key::space | Key::KP_Space => Some(GoTarget::Prompt),
        Key::plus | Key::KP_Add => Some(GoTarget::Pin(true)),
        Key::minus | Key::KP_Subtract => Some(GoTarget::Pin(false)),
        Key::h => place(Location::local(home_directory()), false),
        Key::c => config_folder(),
        Key::t => place(Location::uri("trash:///"), false),
        Key::n => place(Location::uri("network:///"), true),
        Key::r => place(Location::uri("recent:///"), false),
        Key::d => user_folder(glib::UserDirectory::Downloads, "Downloads"),
        Key::k => user_folder(glib::UserDirectory::Documents, "Documents"),
        Key::m => user_folder(glib::UserDirectory::Music, "Music"),
        Key::p => user_folder(glib::UserDirectory::Pictures, "Pictures"),
        Key::v => user_folder(glib::UserDirectory::Videos, "Videos"),
        _ => {
            let number = pin_number(key)?;
            Some(match pins.get(number - 1) {
                Some(location) => GoTarget::Place {
                    location: location.clone(),
                    validate: true,
                },
                None => GoTarget::Missing(format!("No pin {number}")),
            })
        }
    }
}

fn config_folder() -> Option<GoTarget> {
    let path = home_directory().join(".config");
    Some(if path.is_dir() {
        GoTarget::Place {
            location: Location::local(path),
            validate: false,
        }
    } else {
        GoTarget::Missing("No .config folder".into())
    })
}

fn user_folder(directory: glib::UserDirectory, name: &str) -> Option<GoTarget> {
    Some(
        match glib::user_special_dir(directory).filter(|path| path.is_dir()) {
            Some(path) => GoTarget::Place {
                location: Location::local(path),
                validate: false,
            },
            None => GoTarget::Missing(format!("No {name} folder")),
        },
    )
}

fn pin_number(key: Key) -> Option<usize> {
    let digit = match key {
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
    Some(digit)
}

impl Dispatcher {
    /// Runs ahead of every other 10xer key so a pending chord has one consumer.
    pub(super) fn tenxer_chord(
        &self,
        browser: &Rc<Browser>,
        key: Key,
        modifiers: Modifiers,
    ) -> KeyResult {
        let chord = self.shortcuts.armed_chord()?;
        if is_modifier_key(key) {
            return Some(Propagation::Proceed);
        }
        let mods = command_modifiers(modifiers);
        self.shortcuts.cancel_chord();
        // Settings opens over a canceled chord.
        if key == Key::comma && mods == Modifiers::CONTROL_MASK {
            return None;
        }
        if key == Key::Escape && mods.is_empty() {
            return Some(Propagation::Stop);
        }
        // '+' requires Shift on many keyboard layouts.
        let completed = match chord {
            _ if !(mods - Modifiers::SHIFT_MASK).is_empty() => false,
            Chord::Sort => self.complete_sort(key),
            Chord::Go => self.complete_go(browser, key),
            Chord::PreviewTop if key == Key::g => self.preview_to_top(),
            Chord::PreviewTop => {
                go_target(key, &[]).is_some() && {
                    self.return_from_preview(browser);
                    self.complete_go(browser, key)
                }
            }
            Chord::Copy => self.complete_copy(key),
            Chord::Action => self.complete_action(key),
            Chord::Tabs => self.complete_tab(key),
        };
        if !completed {
            self.shortcuts.show_feedback("Unknown chord");
        }
        Some(Propagation::Stop)
    }

    fn complete_tab(&self, key: Key) -> bool {
        let (action, parameter) = match key {
            Key::n => ("win.new-tab", None),
            Key::x => ("win.close-tab", None),
            Key::t => ("win.previous-tab", None),
            _ => {
                let Some(index) = super::super::composition::tab_index(key) else {
                    return false;
                };
                ("win.select-tab", Some(glib::Variant::from(index as u32)))
            }
        };
        gtk::prelude::WidgetExt::activate_action(&self.window, action, parameter.as_ref()).is_ok()
    }

    fn complete_go(&self, browser: &Rc<Browser>, key: Key) -> bool {
        let Some(target) = go_target(key, &self.sidebar.state.visible_pins()) else {
            return false;
        };
        match target {
            GoTarget::FirstItem => self.view.move_displayed_cursor(-1, usize::MAX),
            GoTarget::HitFolder => {
                if !self.view.reveal_listing_search_hit() {
                    self.shortcuts.show_feedback("Nothing to reveal");
                }
            }
            GoTarget::Prompt => {
                self.shortcuts.open_prompt(Prompt::Go);
            }
            GoTarget::Pin(_) if self.chooser.is_some() => {
                self.shortcuts.show_feedback(super::chooser::UNAVAILABLE);
            }
            GoTarget::Pin(pin) => self.change_pin(pin),
            GoTarget::Place { location, .. } if self.refuse_remote_place(&location) => {}
            GoTarget::Place { location, validate } => {
                self.view.keyboard_navigation();
                if validate {
                    browser.navigate_location(location, true);
                } else {
                    browser.navigate_with_selection(location, true);
                }
            }
            GoTarget::Missing(message) => self.shortcuts.show_feedback(&message),
        }
        true
    }
}

impl Dispatcher {
    fn change_pin(&self, pin: bool) {
        use crate::ui::browser::PinChange;
        let feedback = match self.view.change_keyboard_pin(pin) {
            PinChange::Pinned(location, name) => {
                match self
                    .sidebar
                    .state
                    .visible_pins()
                    .iter()
                    .position(|pinned| *pinned == location)
                    .filter(|index| *index < 9)
                {
                    Some(index) => format!("Pinned \u{201c}{name}\u{201d} as g {}", index + 1),
                    None => format!("Pinned \u{201c}{name}\u{201d}"),
                }
            }
            PinChange::Unpinned(name) => format!("Unpinned \u{201c}{name}\u{201d}"),
            PinChange::AlreadyPinned(name) => {
                format!("\u{201c}{name}\u{201d} is already pinned")
            }
            PinChange::NotPinned(name) => format!("\u{201c}{name}\u{201d} isn\u{2019}t pinned"),
            PinChange::Refused(name) => format!("Can\u{2019}t pin \u{201c}{name}\u{201d}"),
            PinChange::Nothing if pin => "Nothing to pin".to_owned(),
            PinChange::Nothing => "Nothing to unpin".to_owned(),
        };
        self.shortcuts.show_feedback(&feedback);
    }
}
