// SPDX-License-Identifier: MIT

//! 10xer's `g` chord targets (`gg`, `gf`, `g+`/`g-`, places and visible pins), ported from
//! `src/ui/window/keyboard/chords.rs` and `src/ui/browser/file_commands.rs`.

use std::{path::PathBuf, rc::Rc};

use glib::UserDirectory;
use rune::Tx;
use strata_components::collection::keyboard::{Command as Nav, Target};
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_desktop::keys::{self as key_catalog, KeyPress, Modifiers, bindings::*};
use strata_fs::{
    adapters::{bookmarks, mount_lanes, probes},
    app::Browser,
    services::{LocationValidationError, io_error_message},
};

use super::{focused_listing_depth, focused_target, run_collection_command};
use crate::{
    App,
    clipboard::paths,
    error_dialog::{self, Error},
    filter, launch, shell,
    sidebar::{self, PinState},
    slots::Later,
};

#[derive(Debug, PartialEq, Eq)]
enum GoTarget {
    FirstItem,
    HitFolder,
    Prompt,
    Pin(bool),
    Place { location: Location, validate: bool },
    Folder { location: Location, missing: String },
    Missing(String),
}

/// Whether `press` completes a `g` chord.
pub(super) fn is_target(app: &App, press: &KeyPress) -> bool {
    go_target(press, &app.sidebar.visible_pins()).is_some()
}

fn go_target(press: &KeyPress, pins: &[Location]) -> Option<GoTarget> {
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
    } else {
        builtin_or_pin_target(press, pins)
    }
}

fn builtin_or_pin_target(press: &KeyPress, pins: &[Location]) -> Option<GoTarget> {
    let place = |location, validate| Some(GoTarget::Place { location, validate });
    if press.is(GO_HOME) {
        place(Location::local(launch::home_directory()), false)
    } else if press.is(GO_CONFIG) {
        Some(GoTarget::Folder {
            location: Location::local(launch::home_directory().join(".config")),
            missing: tr("No .config folder"),
        })
    } else if press.is(GO_TRASH) {
        place(Location::uri("trash:///"), false)
    } else if press.is(GO_NETWORK) {
        place(Location::uri("network:///"), true)
    } else if press.is(GO_RECENT) {
        place(Location::uri("recent:///"), false)
    } else if let Some((dir, missing)) = special_user_folder(press) {
        Some(user_folder(dir, missing))
    } else {
        pin_number_target(press, pins)
    }
}

fn special_user_folder(press: &KeyPress) -> Option<(UserDirectory, &'static str)> {
    [
        (
            GO_DOWNLOADS,
            UserDirectory::Downloads,
            "No Downloads folder",
        ),
        (
            GO_DOCUMENTS,
            UserDirectory::Documents,
            "No Documents folder",
        ),
        (GO_MUSIC, UserDirectory::Music, "No Music folder"),
        (GO_PICTURES, UserDirectory::Pictures, "No Pictures folder"),
        (GO_VIDEOS, UserDirectory::Videos, "No Videos folder"),
    ]
    .into_iter()
    .find(|(cmd, ..)| press.is(*cmd))
    .map(|(_, dir, missing)| (dir, missing))
}

fn user_folder(directory: UserDirectory, missing: &str) -> GoTarget {
    let missing = tr(missing);
    match glib::user_special_dir(directory) {
        Some(path) => GoTarget::Folder {
            location: Location::local(path),
            missing,
        },
        None => GoTarget::Missing(missing),
    }
}

fn pin_number_target(press: &KeyPress, pins: &[Location]) -> Option<GoTarget> {
    let number = usize::from(press.digit_in(PIN_NUMBERS)?);
    Some(match pins.get(number - 1) {
        Some(location) => GoTarget::Place {
            location: location.clone(),
            validate: true,
        },
        None => GoTarget::Missing(rust_i18n::t!("No pin %{number}", number = number).into_owned()),
    })
}

pub(super) fn complete(app: &mut App, press: &KeyPress, tx: &mut Tx) -> bool {
    let pins = app.sidebar.visible_pins();
    let Some(target) = go_target(press, &pins) else {
        return false;
    };
    match target {
        GoTarget::FirstItem => {
            run_collection_command(app, Nav::Cursor(Target::First), tx);
        }
        GoTarget::HitFolder => {
            if !filter::reveal_search_hit(app, tx) {
                app.tenxer.flash.show(tr("Nothing to reveal"), tx);
            }
        }
        GoTarget::Prompt => super::prompts::open_go(app, tx),
        GoTarget::Pin(pin) => change_pin(app, pin, tx),
        GoTarget::Place { location, validate } => {
            shell::publish(app);
            let browser = app.browser.clone();
            if validate {
                browser.navigate_location(location, true);
            } else {
                browser.navigate_with_selection(location, true);
            }
        }
        GoTarget::Folder { location, missing } => {
            open_checked(app, location, missing);
        }
        GoTarget::Missing(message) => app.tenxer.flash.show(message, tx),
    }
    true
}

/// `open_checked` (`src/ui/window/launch.rs`): checks local folder existence before navigating,
/// flashing `missing` if it is gone.
fn open_checked(app: &mut App, location: Location, missing: String) {
    shell::publish(app);
    let browser: Rc<Browser> = app.browser.clone();
    let Some(path) = location.native_path().map(PathBuf::from) else {
        browser.navigate_location(location, true);
        return;
    };
    let generation = browser.bump_navigation_generation();
    let weak = Rc::downgrade(&browser);
    let later = app.tenxer.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let gone = glib::future_with_timeout(mount_lanes::STALLED_AFTER, async {
            match probes::folder(&path).await {
                Err(LocationValidationError::NotDirectory) => true,
                Err(LocationValidationError::Missing) => probes::folder_was_deleted(&path).await,
                _ => false,
            }
        })
        .await
        .unwrap_or(false);
        let Some(browser) = weak.upgrade() else {
            return;
        };
        if browser.navigation_generation() != generation {
            return;
        }
        later.then(move |app, tx| {
            if app.browser.navigation_generation() != generation {
                return false;
            }
            if gone {
                app.tenxer.flash.show(missing, tx);
            } else {
                app.browser.clone().navigate_location(location, true);
                shell::settle(app, tx);
            }
            true
        });
    });
}

fn keyboard_pin_folder(app: &mut App) -> Option<(Location, String)> {
    if let Some(entry) = focused_target(app).filter(FileEntry::is_directory) {
        return Some((entry.location, entry.display_name));
    }
    let location = focused_listing_depth(app).and_then(|depth| app.browser.location_at(depth))?;
    let name = location.display_name();
    Some((location, name))
}

fn change_pin(app: &mut App, pin: bool, tx: &mut Tx) {
    let Some((location, name)) = keyboard_pin_folder(app) else {
        let message = tr(if pin {
            "Nothing to pin"
        } else {
            "Nothing to unpin"
        });
        app.tenxer.flash.show(message, tx);
        return;
    };
    let state = if paths::is_trash(&location) {
        PinState::Unavailable
    } else {
        app.sidebar.pin_state(&location)
    };
    match (pin, state) {
        (true, PinState::Available) => save_pin(app.tenxer.later.clone(), location, name),
        (true, PinState::Pinned) => {
            app.tenxer.flash.show(
                rust_i18n::t!("“%{name}” is already pinned", name = name),
                tx,
            );
        }
        (true, PinState::Unavailable) => {
            app.tenxer
                .flash
                .show(rust_i18n::t!("Can’t pin “%{name}”", name = name), tx);
        }
        (false, PinState::Pinned) => {
            remove_pin(app.tenxer.later.clone(), location);
            app.tenxer
                .flash
                .show(rust_i18n::t!("Unpinned “%{name}”", name = name), tx);
        }
        (false, _) => {
            app.tenxer
                .flash
                .show(rust_i18n::t!("“%{name}” isn’t pinned", name = name), tx);
        }
    }
}

fn save_pin(later: Later<App>, location: Location, name: String) {
    let added_location = location.clone();
    let added_name = name.clone();
    let edit = move |contents: &[u8]| {
        let mut places = bookmarks::parse_pinned_places(contents);
        if places.iter().any(|(pinned, _)| *pinned == added_location) {
            return None;
        }
        places.push((added_location.clone(), added_name.clone()));
        Some(bookmarks::serialize_pinned_places(&places).into_bytes())
    };
    glib::MainContext::default().spawn_local(async move {
        let result = bookmarks::update(edit).await;
        later.then(move |app, tx| {
            sidebar::sync_pins_from_store(app);
            match result {
                Ok(_) => {
                    let message = pinned_feedback(app, &location, &name);
                    app.tenxer.flash.show(message, tx);
                }
                Err(error) => {
                    error_dialog::show(
                        app,
                        Error::new(
                            tr("Unable to update pinned folders"),
                            io_error_message(&error),
                        ),
                    );
                    app.tenxer
                        .flash
                        .show(rust_i18n::t!("Can’t pin “%{name}”", name = name), tx);
                }
            }
            true
        });
    });
}

fn pinned_feedback(app: &App, location: &Location, name: &str) -> String {
    let slot = app
        .sidebar
        .visible_pins()
        .iter()
        .position(|pinned| pinned == location)
        .filter(|index| *index < 9);
    match slot {
        Some(index) => rust_i18n::t!(
            "Pinned “%{name}” as %{keys}",
            name = name,
            keys = key_catalog::digit_label(PIN_NUMBERS, (index + 1) as u8)
        )
        .into_owned(),
        None => rust_i18n::t!("Pinned “%{name}”", name = name).into_owned(),
    }
}

fn remove_pin(later: Later<App>, location: Location) {
    let edit = move |contents: &[u8]| {
        let mut places = bookmarks::parse_pinned_places(contents);
        let before = places.len();
        places.retain(|(pinned, _)| *pinned != location);
        (places.len() != before).then(|| bookmarks::serialize_pinned_places(&places).into_bytes())
    };
    glib::MainContext::default().spawn_local(async move {
        let result = bookmarks::update(edit).await;
        later.then(move |app, _| {
            sidebar::sync_pins_from_store(app);
            if let Err(error) = result {
                error_dialog::show(
                    app,
                    Error::new(
                        tr("Unable to update pinned folders"),
                        io_error_message(&error),
                    ),
                );
            }
            true
        });
    });
}
