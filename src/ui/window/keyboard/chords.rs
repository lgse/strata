// SPDX-License-Identifier: MIT

//! Armed 10xer chords. Once a chord's mark shows in the footer, the next key
//! completes or cancels that chord and reaches no other handler. Text-size
//! shortcuts, shortcut-reference keys, and Escape while autoscroll is running
//! are earlier capture handlers: they drop the mark first, then run, the same
//! way Ctrl+, opens Settings over a canceled chord.

use std::rc::Rc;

use gtk::{
    gdk::ModifierType as Modifiers,
    glib::{self, Propagation},
};

use super::{Dispatcher, KeyResult};
use crate::{
    app::Browser,
    model::Location,
    ui::{
        keys::{KeyPress, bindings::*},
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
    Folder {
        location: Location,
        missing: String,
    },
    Missing(String),
}

/// Resolves the second key of **g**. `pins` are the visible PINNED rows in
/// sidebar display order.
pub(in crate::ui::window) fn go_target(press: &KeyPress, pins: &[Location]) -> Option<GoTarget> {
    let place = |location, validate| Some(GoTarget::Place { location, validate });
    if press.is(GO_FIRST) {
        Some(GoTarget::FirstItem)
    } else if press.is(GO_FOLLOW) {
        Some(GoTarget::HitFolder)
    } else if press.is(GO_PROMPT.ignoring(Modifiers::SHIFT_MASK)) {
        Some(GoTarget::Prompt)
    } else if press.is(GO_PIN) {
        Some(GoTarget::Pin(true))
    } else if press.is(GO_UNPIN) {
        Some(GoTarget::Pin(false))
    } else if press.is(GO_HOME) {
        place(Location::local(home_directory()), false)
    } else if press.is(GO_CONFIG) {
        config_folder()
    } else if press.is(GO_TRASH) {
        place(Location::uri("trash:///"), false)
    } else if press.is(GO_NETWORK) {
        place(Location::uri("network:///"), true)
    } else if press.is(GO_RECENT) {
        place(Location::uri("recent:///"), false)
    } else if press.is(GO_DOWNLOADS) {
        user_folder(glib::UserDirectory::Downloads, "No Downloads folder")
    } else if press.is(GO_DOCUMENTS) {
        user_folder(glib::UserDirectory::Documents, "No Documents folder")
    } else if press.is(GO_MUSIC) {
        user_folder(glib::UserDirectory::Music, "No Music folder")
    } else if press.is(GO_PICTURES) {
        user_folder(glib::UserDirectory::Pictures, "No Pictures folder")
    } else if press.is(GO_VIDEOS) {
        user_folder(glib::UserDirectory::Videos, "No Videos folder")
    } else {
        let number = usize::from(press.digit_in(PIN_NUMBERS)?);
        Some(match pins.get(number - 1) {
            Some(location) => GoTarget::Place {
                location: location.clone(),
                validate: true,
            },
            None => {
                GoTarget::Missing(rust_i18n::t!("No pin %{number}", number = number).into_owned())
            }
        })
    }
}

fn config_folder() -> Option<GoTarget> {
    Some(GoTarget::Folder {
        location: Location::local(home_directory().join(".config")),
        missing: crate::i18n::tr("No .config folder"),
    })
}

fn user_folder(directory: glib::UserDirectory, missing: &str) -> Option<GoTarget> {
    let missing = crate::i18n::tr(missing);
    Some(match glib::user_special_dir(directory) {
        Some(path) => GoTarget::Folder {
            location: Location::local(path),
            missing,
        },
        None => GoTarget::Missing(missing),
    })
}

impl Dispatcher {
    /// Runs ahead of every other 10xer key so a pending chord has one consumer.
    pub(super) fn tenxer_chord(&self, browser: &Rc<Browser>, press: &KeyPress) -> KeyResult {
        let chord = self.shortcuts.armed_chord()?;
        if press.is_modifier() {
            return Some(Propagation::Proceed);
        }
        self.shortcuts.cancel_chord();
        // Settings opens over a canceled chord.
        if press.is(SETTINGS) {
            return None;
        }
        if press.is(ESCAPE) {
            return Some(Propagation::Stop);
        }
        let completed = match chord {
            Chord::Sort => self.complete_sort(press),
            Chord::Go => self.complete_go(browser, press),
            Chord::PreviewTop if press.is(GO_FIRST) => self.preview_to_top(),
            Chord::PreviewTop => {
                go_target(press, &[]).is_some() && {
                    self.return_from_preview(browser);
                    self.complete_go(browser, press)
                }
            }
            Chord::Copy => self.complete_copy(press),
            Chord::Action => self.complete_action(press),
            Chord::Tabs => self.complete_tab(press),
        };
        if !completed {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Unknown chord"));
        }
        Some(Propagation::Stop)
    }

    fn complete_tab(&self, press: &KeyPress) -> bool {
        let (action, parameter) = if press.is(TAB_CHORD_NEW) {
            ("win.new-tab", None)
        } else if press.is(TAB_CHORD_CLOSE) {
            ("win.close-tab", None)
        } else if press.is(TAB_CHORD_PREVIOUS) {
            ("win.previous-tab", None)
        } else if let Some(digit) = press.digit_in(TAB_CHORD_NUMBERS) {
            let index = super::super::composition::tab_index(digit);
            ("win.select-tab", Some(glib::Variant::from(index as u32)))
        } else {
            return false;
        };
        gtk::prelude::WidgetExt::activate_action(&self.window, action, parameter.as_ref()).is_ok()
    }

    fn complete_go(&self, browser: &Rc<Browser>, press: &KeyPress) -> bool {
        let Some(target) = go_target(press, &self.sidebar.state.visible_pins()) else {
            return false;
        };
        match target {
            GoTarget::FirstItem => self.view.move_displayed_cursor(-1, usize::MAX),
            GoTarget::HitFolder => {
                if !self.view.reveal_listing_search_hit() {
                    self.shortcuts
                        .show_feedback(&crate::i18n::tr("Nothing to reveal"));
                }
            }
            GoTarget::Prompt => {
                self.shortcuts.open_prompt(Prompt::Go);
            }
            GoTarget::Pin(_) if self.chooser.is_some() => {
                self.shortcuts
                    .show_feedback(&crate::i18n::tr(super::chooser::UNAVAILABLE));
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
            GoTarget::Folder { location, missing } => {
                self.view.keyboard_navigation();
                let shortcuts = self.shortcuts.clone();
                super::super::open_checked(browser, location, move || {
                    shortcuts.show_feedback(&missing);
                });
            }
            GoTarget::Missing(message) => self.shortcuts.show_feedback(&message),
        }
        true
    }
}

impl Dispatcher {
    fn change_pin(&self, pin: bool) {
        use crate::ui::browser::PinChange;
        let sidebar = Rc::downgrade(&self.sidebar.state);
        let shortcuts = self.shortcuts.clone();
        self.view.change_keyboard_pin(pin, move |change| {
            let Some(sidebar) = sidebar.upgrade() else {
                return;
            };
            let feedback = match change {
                PinChange::Pinned(location, name) => {
                    match sidebar
                        .visible_pins()
                        .iter()
                        .position(|pinned| *pinned == location)
                        .filter(|index| *index < 9)
                    {
                        Some(index) => rust_i18n::t!(
                            "Pinned “%{name}” as %{keys}",
                            name = name,
                            keys = crate::ui::keys::digit_label(
                                crate::ui::keys::bindings::PIN_NUMBERS,
                                (index + 1) as u8
                            )
                        )
                        .into_owned(),
                        None => rust_i18n::t!("Pinned “%{name}”", name = name).into_owned(),
                    }
                }
                PinChange::Unpinned(name) => {
                    rust_i18n::t!("Unpinned “%{name}”", name = name).into_owned()
                }
                PinChange::AlreadyPinned(name) => {
                    rust_i18n::t!("“%{name}” is already pinned", name = name).into_owned()
                }
                PinChange::NotPinned(name) => {
                    rust_i18n::t!("“%{name}” isn’t pinned", name = name).into_owned()
                }
                PinChange::Refused(name) => {
                    rust_i18n::t!("Can’t pin “%{name}”", name = name).into_owned()
                }
                PinChange::Nothing if pin => crate::i18n::tr("Nothing to pin"),
                PinChange::Nothing => crate::i18n::tr("Nothing to unpin"),
            };
            shortcuts.show_feedback(&feedback);
        });
    }
}
