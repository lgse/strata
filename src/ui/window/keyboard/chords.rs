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
    services::LocationValidationError,
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
            Chord::BookmarkSet | Chord::BookmarkGo => self.complete_bookmark(browser, chord, press),
        };
        if !completed {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Unknown chord"));
        }
        Some(Propagation::Stop)
    }

    pub(super) fn arm_bookmark_go(&self) {
        self.shortcuts.arm_chord(Chord::BookmarkGo);
        let shortcuts = self.shortcuts.clone();
        let revision = self.bookmark_revision.clone();
        let expected = revision.get();
        let preferences = self.type_to_search.preferences.clone();
        let view = self.view.clone();
        let generation = view.browser().navigation_generation();
        glib::MainContext::default().spawn_local(async move {
            let result = crate::services::letter_bookmarks::list().await;
            if revision.get() != expected
                || shortcuts.armed_chord() != Some(Chord::BookmarkGo)
                || !preferences.tenxer_mode()
                || !view.item_view_has_focus()
                || view.browser().navigation_generation() != generation
            {
                return;
            }
            match result {
                Ok(entries) => {
                    let rows = entries
                        .into_iter()
                        .filter_map(|(letter, path)| {
                            let (_, binding) =
                                BOOKMARK_LETTERS.iter().find(|(slot, _)| *slot == letter)?;
                            Some((
                                crate::ui::keys::label(&crate::ui::keys::Keys::One(*binding)),
                                path.display().to_string(),
                            ))
                        })
                        .collect();
                    shortcuts.arm_chord_with(Chord::BookmarkGo, rows);
                }
                Err(error) => shortcuts.show_feedback(&rust_i18n::t!(
                    "Could not access bookmarks: %{error}",
                    error = crate::services::io_error_detail(&error)
                )),
            }
        });
    }

    fn complete_bookmark(&self, browser: &Rc<Browser>, chord: Chord, press: &KeyPress) -> bool {
        let Some((letter, binding)) = BOOKMARK_LETTERS
            .iter()
            .copied()
            .find(|(_, binding)| press.is(*binding))
        else {
            return false;
        };
        if self.chooser.is_some() {
            self.shortcuts
                .show_feedback(&crate::i18n::tr(super::chooser::UNAVAILABLE));
            return true;
        }
        let directory = self.view.keyboard_bookmark_folder();
        if chord == Chord::BookmarkSet && directory.is_none() {
            self.shortcuts
                .show_feedback(&crate::i18n::tr("Only local folders can be bookmarked"));
            return true;
        }
        let weak_browser = Rc::downgrade(browser);
        let generation = if chord == Chord::BookmarkGo {
            browser.bump_navigation_generation()
        } else {
            browser.navigation_generation()
        };
        let view = self.view.clone();
        let origin = directory.clone();
        let revision = self.bookmark_revision.clone();
        let expected = revision.get();
        let shortcuts = self.shortcuts.clone();
        let preferences = self.type_to_search.preferences.clone();
        let key = crate::ui::keys::label(&crate::ui::keys::Keys::One(binding));
        glib::MainContext::default().spawn_local(async move {
            let result = if let Some(directory) = directory.filter(|_| chord == Chord::BookmarkSet)
            {
                crate::services::letter_bookmarks::set(letter, directory)
                    .await
                    .map(|()| None)
            } else {
                crate::services::letter_bookmarks::get(letter).await
            };
            let unavailable = if let Ok(Some(path)) = &result {
                glib::future_with_timeout(crate::adapters::mount_lanes::STALLED_AFTER, async {
                    match crate::adapters::probes::folder(path).await {
                        Err(LocationValidationError::NotDirectory) => true,
                        Err(LocationValidationError::Missing) => {
                            crate::adapters::probes::folder_was_deleted(path).await
                        }
                        _ => false,
                    }
                })
                .await
                .unwrap_or(false)
            } else {
                false
            };
            let Some(browser) = weak_browser.upgrade() else {
                return;
            };
            if revision.get() != expected
                || browser.navigation_generation() != generation
                || !preferences.tenxer_mode()
                || !view.item_view_has_focus()
                || view.keyboard_bookmark_folder() != origin
            {
                return;
            }
            match result {
                Ok(_) if chord == Chord::BookmarkSet => {
                    shortcuts.show_feedback(&rust_i18n::t!("Saved bookmark %{key}", key = key))
                }
                Ok(Some(_)) if unavailable => shortcuts
                    .show_feedback(&rust_i18n::t!("Bookmark %{key} is unavailable", key = key)),
                Ok(Some(path)) => {
                    view.keyboard_navigation();
                    browser.navigate_location(Location::local(path), true);
                }
                Ok(None) => {
                    shortcuts.show_feedback(&rust_i18n::t!("No bookmark %{key}", key = key))
                }
                Err(error) => shortcuts.show_feedback(&rust_i18n::t!(
                    "Could not access bookmarks: %{error}",
                    error = crate::services::io_error_detail(&error)
                )),
            }
        });
        true
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
        self.shortcuts.show_feedback(&feedback);
    }
}
