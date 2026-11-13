// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use glib::UserDirectory;
use strata_components::sidebar::ItemId;
use strata_core::model::Location;
use strata_desktop::settings::Preferences;

use super::{Environment, Navigation, listed, resolve_place_order};

fn order(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn a_saved_order_keeps_its_places_and_gains_the_missing_ones_after_their_neighbours() {
    let cases: [(&[&str], &[&str]); 3] = [
        // An order saved before Music existed gains it after Downloads.
        (
            &[
                "home",
                "trash",
                "network",
                "recent",
                "desktop",
                "documents",
                "downloads",
                "pictures",
                "videos",
            ],
            &[
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
            ],
        ),
        // A moved place stays moved; unknown and repeated ids are dropped.
        (
            &["trash", "home", "trash", "gone", "network"],
            &[
                "trash",
                "home",
                "network",
                "recent",
                "desktop",
                "documents",
                "downloads",
                "music",
                "pictures",
                "videos",
            ],
        ),
        // A place with no earlier neighbour saved goes first.
        (
            &["videos"],
            &[
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
            ],
        ),
    ];
    for (saved, expected) in cases {
        assert_eq!(resolve_place_order(&order(saved)), expected, "{saved:?}");
    }
}

/// A home at `/home/user`, with Documents and a Desktop that is the home folder.
fn environment(recent: bool) -> Environment {
    fn special(directory: UserDirectory) -> Option<PathBuf> {
        match directory {
            UserDirectory::Desktop => Some(PathBuf::from("/home/user")),
            UserDirectory::Documents => Some(PathBuf::from("/home/user/Documents")),
            _ => None,
        }
    }
    Environment {
        home: PathBuf::from("/home/user"),
        special,
        recent,
    }
}

fn place_ids(settings: &Preferences, environment: &Environment) -> Vec<String> {
    listed(settings, &[], environment)
        .places
        .iter()
        .map(|place| place.id.clone())
        .collect()
}

#[test]
fn places_show_as_the_settings_and_the_users_folders_allow() {
    let settings = Preferences::default();
    assert_eq!(
        place_ids(&settings, &environment(false)),
        ["home", "trash", "network", "documents"],
        "no Recent without its backend, no Desktop that is the home folder, no folder unset"
    );
    assert_eq!(
        place_ids(&settings, &environment(true)),
        ["home", "trash", "network", "recent", "documents"]
    );
    let hidden = Preferences {
        sidebar_show_trash: false,
        sidebar_show_documents: false,
        sidebar_show_recent: false,
        sidebar_order: order(&["network", "home"]),
        ..Preferences::default()
    };
    assert_eq!(place_ids(&hidden, &environment(true)), ["network", "home"]);
}

#[test]
fn every_row_opens_its_location_the_way_gtk_does() {
    let listed = listed(&Preferences::default(), &[], &environment(true));
    let rows: Vec<(&str, String, Navigation)> = listed
        .rows
        .iter()
        .map(|row| (row.item.id(), row.location.display_path(), row.navigation))
        .collect();
    let home = Location::local("/home/user").display_path();
    let documents = Location::local("/home/user/Documents").display_path();
    let network = Location::uri("network:///").display_path();
    let trash = Location::uri("trash:///").display_path();
    let recent = Location::uri("recent:///").display_path();
    assert_eq!(
        rows,
        [
            ("home", home, Navigation::Validate),
            ("trash", trash, Navigation::Direct),
            ("network", network, Navigation::Validate),
            ("recent", recent, Navigation::Direct),
            ("documents", documents, Navigation::Direct),
        ]
    );
    let described: Vec<Option<&str>> = listed
        .places
        .iter()
        .map(|place| place.description.as_deref())
        .collect();
    assert_eq!(described[1], Some("trash:///"));
    assert_eq!(described[3], Some("recent:///"));
}

#[test]
fn pins_leave_out_the_standard_folders_and_take_drops_only_when_local() {
    let pins = [
        (Location::local("/home/user"), "Home again".to_owned()),
        (Location::local("/srv/work"), "Work".to_owned()),
        (Location::uri("sftp://host/share"), "Share".to_owned()),
        (Location::local("/home/user/Documents"), "Docs".to_owned()),
    ];
    let listed = listed(&Preferences::default(), &pins, &environment(false));
    let shown: Vec<(&str, &str, bool)> = listed
        .pins
        .iter()
        .map(|pin| (pin.id.as_str(), pin.label.as_str(), pin.accepts_drops))
        .collect();
    assert_eq!(shown, [("1", "Work", true), ("2", "Share", false)]);
    let work = listed
        .rows
        .iter()
        .find(|row| row.item == ItemId::Pin("1".into()))
        .expect("the pin's row");
    assert_eq!(work.location, Location::local(Path::new("/srv/work")));
    assert_eq!(work.navigation, Navigation::Validate);
    assert_eq!(
        listed.pins[0].description.as_deref(),
        Some(Location::local("/srv/work").display_path().as_str())
    );
}
