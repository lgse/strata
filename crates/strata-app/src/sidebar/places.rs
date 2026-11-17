// SPDX-License-Identifier: MIT

//! What the sidebar lists: the built-in places in the saved order, then the pins, as the GTK
//! sidebar decides them (`src/ui/window/places.rs`, `append_ordered_place` and
//! `append_pinned_places` in `src/ui/window/sidebar_places.rs`). A fix there has to be
//! carried here until the GTK sidebar is deleted.

use std::path::{Path, PathBuf};

use gio::prelude::*;
use glib::UserDirectory;
use strata_components::sidebar::{ItemId, Pin, Place};
use strata_core::{i18n::tr, model::Location};
use strata_desktop::{
    keys::{
        self as key_catalog, Command, Keys, Map,
        bindings::{
            GO_DOCUMENTS, GO_DOWNLOADS, GO_HOME, GO_MUSIC, GO_NETWORK, GO_PICTURES, GO_RECENT,
            GO_TRASH, GO_VIDEOS, PIN_NUMBERS,
        },
    },
    settings::Preferences,
};
use strata_kit::icons::IconName;

use crate::slots::path_customization;

/// Every built-in place, in the order a sidebar with no saved order shows them.
pub(super) const STANDARD_PLACE_IDS: &[&str] = &[
    "home",
    "trash",
    "network",
    "recent",
    "desktop",
    "documents",
    "downloads",
    "music",
    "pictures",
    "videos",
];

/// How a row opens its location (`PlaceNavigation` in `src/ui/window/sidebar.rs`): a place
/// whose destination is known opens at once, and the others are checked first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Navigation {
    Direct,
    Validate,
}

/// A row of the sidebar: its id in the component, and where it goes.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    pub(super) item: ItemId,
    pub(super) location: Location,
    pub(super) navigation: Navigation,
}

/// What the sidebar shows: the component's places and pins, and every row's location in
/// the order the rows are drawn.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Listed {
    pub(super) places: Vec<Place>,
    pub(super) pins: Vec<Pin>,
    pub(super) rows: Vec<Row>,
}

/// What the places depend on beside the settings: the home folder, the user's folders and
/// whether Recent can be opened.
pub(super) struct Environment {
    pub(super) home: PathBuf,
    pub(super) special: fn(UserDirectory) -> Option<PathBuf>,
    pub(super) recent: bool,
    pub(super) local_only: bool,
}

impl Environment {
    /// This process's: `$HOME`, GLib's user folders, and the `recent` scheme of GIO's VFS.
    pub(super) fn current() -> Self {
        Self {
            home: home_directory(),
            special: glib::user_special_dir,
            recent: recent_available(),
            local_only: false,
        }
    }

    pub(super) fn local_only(mut self, local_only: bool) -> Self {
        self.local_only = local_only;
        self
    }

    pub(super) fn is_standard(&self, location: &Location) -> bool {
        let Some(path) = location.native_path() else {
            return false;
        };
        path == self.home
            || [
                UserDirectory::Desktop,
                UserDirectory::Documents,
                UserDirectory::Downloads,
                UserDirectory::Music,
                UserDirectory::Pictures,
                UserDirectory::Videos,
            ]
            .into_iter()
            .filter_map(self.special)
            .any(|standard| standard == path)
    }
}

/// `home_directory` in `src/ui/window/launch.rs`.
fn home_directory() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// `RecentAvailability` in `src/ui/window/sidebar_places.rs`: GIO's VFS opens `recent:` URIs.
/// GTK also asks its `gtk-recent-files-enabled` setting, which this interface does not read,
/// so it counts as on, its default.
fn recent_available() -> bool {
    gio::Vfs::default()
        .supported_uri_schemes()
        .iter()
        .any(|scheme| scheme.as_str().eq_ignore_ascii_case("recent"))
}

/// The saved order with every built-in place in it once (`resolve_place_order`). A place the
/// saved order lacks goes after the nearest place before it in the default order.
pub(super) fn resolve_place_order(persisted: &[String]) -> Vec<&'static str> {
    let mut order: Vec<&'static str> = Vec::new();
    for id in persisted {
        if let Some(known) = STANDARD_PLACE_IDS.iter().copied().find(|known| known == id)
            && !order.contains(&known)
        {
            order.push(known);
        }
    }
    for (index, &id) in STANDARD_PLACE_IDS.iter().enumerate() {
        if !order.contains(&id) {
            let at = STANDARD_PLACE_IDS[..index]
                .iter()
                .rev()
                .find_map(|earlier| order.iter().position(|place| place == earlier))
                .map_or(0, |position| position + 1);
            order.insert(at, id);
        }
    }
    order
}

/// What the sidebar lists for `settings` and the saved `pins`.
pub(super) fn listed(
    settings: &Preferences,
    pins: &[(Location, String)],
    environment: &Environment,
) -> Listed {
    let mut listed = Listed::default();
    for id in resolve_place_order(&settings.sidebar_order) {
        if let Some((place, location, navigation)) = place(id, settings, environment) {
            listed.rows.push(Row {
                item: place.item(),
                location,
                navigation,
            });
            listed.places.push(place);
        }
    }
    let shown = pins.iter().enumerate().filter(|(_, (location, _))| {
        !environment.is_standard(location)
            && (!environment.local_only || location.native_path().is_some())
    });
    for (ordinal, (index, entry)) in shown.enumerate() {
        let pin = build_pin(ordinal, index, entry, settings);
        listed.rows.push(Row {
            item: pin.item(),
            location: entry.0.clone(),
            navigation: Navigation::Validate,
        });
        listed.pins.push(pin);
    }
    listed
}

fn build_pin(
    ordinal: usize,
    index: usize,
    (location, name): &(Location, String),
    settings: &Preferences,
) -> Pin {
    let pin = Pin::new(index.to_string(), name.as_str())
        .accepts_drops(location.native_path().is_some())
        .description(location.display_path());
    let pin = match pin_keycap(ordinal) {
        Some(cap) => pin.keycap(cap),
        None => pin,
    };
    decorate_pin(pin, settings, location.native_path())
}

fn with_place_keycap(place: Place, step: Command) -> Place {
    let cap =
        key_catalog::label_in_map(&rust_i18n::locale(), &Keys::One(step.shown()), Map::Tenxer);
    if cap.is_empty() {
        place
    } else {
        place.keycap(cap)
    }
}

fn pin_keycap(ordinal: usize) -> Option<String> {
    let pin = u8::try_from(ordinal + 1)
        .ok()
        .filter(|pin| PIN_NUMBERS.contains(*pin))?;
    let cap = key_catalog::digit_label(PIN_NUMBERS.step(), pin);
    (!cap.is_empty()).then_some(cap)
}

fn standard_place_chord_key(id: &str) -> Option<Command> {
    match id {
        "documents" => Some(GO_DOCUMENTS),
        "downloads" => Some(GO_DOWNLOADS),
        "music" => Some(GO_MUSIC),
        "pictures" => Some(GO_PICTURES),
        "videos" => Some(GO_VIDEOS),
        _ => None,
    }
}

/// One built-in place, or none when the settings hide it or it has no folder.
fn place(
    id: &'static str,
    settings: &Preferences,
    environment: &Environment,
) -> Option<(Place, Location, Navigation)> {
    match id {
        "home" => settings.sidebar_show_home.then(|| {
            let location = Location::local(&environment.home);
            let place =
                Place::folder(id, tr("Home"), IconName::Home).description(location.display_path());
            let place = with_place_keycap(place, GO_HOME);
            let place = decorate_place(place, settings, &environment.home);
            (place, location, Navigation::Validate)
        }),
        "trash" => (!environment.local_only && settings.sidebar_show_trash).then(|| {
            let place = Place::trash(tr("Trash"), false).description("trash:///");
            let place = with_place_keycap(place, GO_TRASH);
            (place, Location::uri("trash:///"), Navigation::Direct)
        }),
        "network" => (!environment.local_only && settings.sidebar_show_network).then(|| {
            let location = Location::uri("network:///");
            let place = Place::other(id, tr("Network"), IconName::Network)
                .description(location.display_path());
            let place = with_place_keycap(place, GO_NETWORK);
            (place, location, Navigation::Validate)
        }),
        "recent" => (settings.sidebar_show_recent && environment.recent).then(|| {
            let place = Place::other(id, tr("Recent"), IconName::Recent).description("recent:///");
            let place = with_place_keycap(place, GO_RECENT);
            (place, Location::uri("recent:///"), Navigation::Direct)
        }),
        _ => standard_place(id, settings, environment),
    }
}

/// A user folder: Desktop, Documents and the rest (`append_standard_place`).
fn standard_place(
    id: &'static str,
    settings: &Preferences,
    environment: &Environment,
) -> Option<(Place, Location, Navigation)> {
    let (icon, name, directory, shown) = match id {
        "desktop" => (
            IconName::Folder,
            "Desktop",
            UserDirectory::Desktop,
            settings.sidebar_show_desktop,
        ),
        "documents" => (
            IconName::Documents,
            "Documents",
            UserDirectory::Documents,
            settings.sidebar_show_documents,
        ),
        "downloads" => (
            IconName::Downloads,
            "Downloads",
            UserDirectory::Downloads,
            settings.sidebar_show_downloads,
        ),
        "music" => (
            IconName::Music,
            "Music",
            UserDirectory::Music,
            settings.sidebar_show_music,
        ),
        "pictures" => (
            IconName::Pictures,
            "Pictures",
            UserDirectory::Pictures,
            settings.sidebar_show_pictures,
        ),
        "videos" => (
            IconName::Videos,
            "Videos",
            UserDirectory::Videos,
            settings.sidebar_show_videos,
        ),
        _ => return None,
    };
    let path = shown
        .then(|| (environment.special)(directory))
        .flatten()
        .filter(|path| shows_standard_place(id, path, &environment.home))?;
    let location = Location::local(&path);
    let place = Place::folder(id, tr(name), icon).description(location.display_path());
    let place = match standard_place_chord_key(id) {
        Some(step) => with_place_keycap(place, step),
        None => place,
    };
    let place = decorate_place(place, settings, &path);
    Some((place, location, Navigation::Direct))
}

fn decorate_place(place: Place, settings: &Preferences, path: &Path) -> Place {
    let (color, glyph) = path_customization(settings, path);
    let place = place.tint(color);
    match glyph {
        Some(glyph) => place.icon(glyph.to_row_icon()),
        None => place,
    }
}

fn decorate_pin(pin: Pin, settings: &Preferences, path: Option<&Path>) -> Pin {
    let Some(path) = path else {
        return pin;
    };
    let (color, glyph) = path_customization(settings, path);
    let pin = pin.tint(color);
    match glyph {
        Some(glyph) => pin.icon(glyph.to_row_icon()),
        None => pin,
    }
}

/// `should_show_standard_place`: a Desktop that is the home folder is not listed twice.
fn shows_standard_place(id: &str, path: &Path, home: &Path) -> bool {
    id != "desktop" || path != home
}

#[cfg(test)]
mod tests;
