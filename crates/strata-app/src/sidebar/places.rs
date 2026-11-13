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
use strata_desktop::settings::Preferences;
use strata_kit::icons::IconName;

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
}

impl Environment {
    /// This process's: `$HOME`, GLib's user folders, and the `recent` scheme of GIO's VFS.
    pub(super) fn current() -> Self {
        Self {
            home: home_directory(),
            special: glib::user_special_dir,
            recent: recent_available(),
        }
    }

    fn is_standard(&self, location: &Location) -> bool {
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
    let shown = pins
        .iter()
        .enumerate()
        .filter(|(_, (location, _))| !environment.is_standard(location));
    for (index, (location, name)) in shown {
        let pin = Pin::new(index.to_string(), name.as_str())
            .accepts_drops(location.native_path().is_some())
            .description(location.display_path());
        listed.rows.push(Row {
            item: pin.item(),
            location: location.clone(),
            navigation: Navigation::Validate,
        });
        listed.pins.push(pin);
    }
    listed
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
            (place, location, Navigation::Validate)
        }),
        "trash" => settings.sidebar_show_trash.then(|| {
            let place = Place::trash(tr("Trash"), false).description("trash:///");
            (place, Location::uri("trash:///"), Navigation::Direct)
        }),
        "network" => settings.sidebar_show_network.then(|| {
            let location = Location::uri("network:///");
            let place = Place::other(id, tr("Network"), IconName::Network)
                .description(location.display_path());
            (place, location, Navigation::Validate)
        }),
        "recent" => (settings.sidebar_show_recent && environment.recent).then(|| {
            let place = Place::other(id, tr("Recent"), IconName::Recent).description("recent:///");
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
    let location = Location::local(path);
    let place = Place::folder(id, tr(name), icon).description(location.display_path());
    Some((place, location, Navigation::Direct))
}

/// `should_show_standard_place`: a Desktop that is the home folder is not listed twice.
fn shows_standard_place(id: &str, path: &Path, home: &Path) -> bool {
    id != "desktop" || path != home
}

#[cfg(test)]
mod tests;
