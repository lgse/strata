// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use gtk::glib;

use super::{
    RemovableDestination, is_standard_place_location, load_pinned_places, removable_destinations,
    resolve_place_order, should_show_standard_place, sidebar_standard_place_visible,
    standard_place,
};
use crate::model::Location;
use crate::ui::preferences::PreferenceManager;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum PlaceGroup {
    Standard,
    Pinned,
    Device,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) struct PlaceShortcut {
    pub icon: &'static str,
    pub name: String,
    pub path: PathBuf,
    pub group: PlaceGroup,
}

/// Local folders a destination picker offers, following the sidebar's order
/// and visibility. Home is always offered.
pub(in crate::ui) fn destination_places(
    manager: &PreferenceManager,
    home: &Path,
) -> Vec<PlaceShortcut> {
    collect_places(
        &resolve_place_order(&manager.sidebar_order()),
        |id| sidebar_standard_place_visible(manager, id),
        glib::user_special_dir,
        home,
        load_pinned_places()
            .unwrap_or_default()
            .into_iter()
            .filter(|(location, _)| !is_standard_place_location(location))
            .collect(),
        removable_destinations(),
    )
}

pub(super) fn collect_places(
    order: &[&str],
    visible: impl Fn(&str) -> bool,
    special_dir: impl Fn(glib::UserDirectory) -> Option<PathBuf>,
    home: &Path,
    pinned: Vec<(Location, String)>,
    devices: Vec<RemovableDestination>,
) -> Vec<PlaceShortcut> {
    let mut places = vec![PlaceShortcut {
        icon: crate::assets::icons::HOME,
        name: "Home".to_owned(),
        path: home.to_path_buf(),
        group: PlaceGroup::Standard,
    }];
    for &id in order {
        let Some((icon, name, directory)) = standard_place(id) else {
            continue;
        };
        if !visible(id) {
            continue;
        }
        if let Some(path) =
            special_dir(directory).filter(|path| should_show_standard_place(id, path, home))
        {
            places.push(PlaceShortcut {
                icon,
                name: name.to_owned(),
                path,
                group: PlaceGroup::Standard,
            });
        }
    }
    places.extend(pinned.into_iter().filter_map(|(location, name)| {
        Some(PlaceShortcut {
            icon: crate::assets::icons::FOLDER,
            name,
            path: location.native_path()?.to_path_buf(),
            group: PlaceGroup::Pinned,
        })
    }));
    places.extend(devices.into_iter().map(|device| PlaceShortcut {
        icon: crate::assets::icons::HARD_DRIVE,
        name: device.name,
        path: device.root,
        group: PlaceGroup::Device,
    }));
    let mut seen = Vec::new();
    places.retain(|place| {
        if !place.path.is_dir() || seen.contains(&place.path) {
            return false;
        }
        seen.push(place.path.clone());
        true
    });
    places
}
