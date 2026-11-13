// SPDX-License-Identifier: MIT

use strata_components::sidebar::Landing;
use strata_core::model::Location;

use super::{move_pin_in, place_target, reorder_pinned_places, reorder_places};

/// A place dropped where GTK's sidebar takes it moves in the whole saved order, beside the row
/// the drop names: with Network hidden between Trash and Desktop, a drop between Trash and
/// Desktop puts Desktop after Trash, ahead of Network, as GTK's gap "after the row above" does.
#[test]
fn a_place_moves_beside_the_row_its_drop_names_in_the_whole_saved_order() {
    // The rows shown: Home, Trash, Desktop. (from, landing, the saved order after)
    let cases: [(usize, Landing, [&str; 4]); 5] = [
        (2, Landing::Gap(2), ["home", "trash", "desktop", "network"]),
        (
            0,
            Landing::Row {
                index: 1,
                after: true,
            },
            ["trash", "home", "network", "desktop"],
        ),
        (
            2,
            Landing::Row {
                index: 0,
                after: false,
            },
            ["desktop", "home", "trash", "network"],
        ),
        (1, Landing::Gap(0), ["trash", "home", "network", "desktop"]),
        (1, Landing::Gap(3), ["home", "network", "desktop", "trash"]),
    ];
    let shown = ["home", "trash", "desktop"];
    for (from, landing, expected) in cases {
        let mut order = vec!["home", "trash", "network", "desktop"];
        let (target, after) = place_target(landing);
        assert!(
            reorder_places(&mut order, shown[from], shown[target], after),
            "{landing:?}"
        );
        assert_eq!(order, expected, "{} dropped at {landing:?}", shown[from]);
    }
}

#[test]
fn a_place_dropped_on_itself_or_its_own_gap_stays() {
    let shown = ["home", "trash", "desktop"];
    for (from, landing) in [
        (
            1,
            Landing::Row {
                index: 1,
                after: false,
            },
        ),
        (1, Landing::Gap(2)),
        (0, Landing::Gap(0)),
    ] {
        let mut order = vec!["home", "trash", "network", "desktop"];
        let (target, after) = place_target(landing);
        assert!(
            !reorder_places(&mut order, shown[from], shown[target], after),
            "{landing:?}"
        );
    }
}

fn pins(names: &[&str]) -> Vec<(Location, String)> {
    let location = |name: &str| match name {
        "home" => Location::local("/home/user"),
        name => Location::uri(format!("sftp://host/{name}")),
    };
    names
        .iter()
        .map(|name| (location(name), (*name).to_owned()))
        .collect()
}

fn names(places: &[(Location, String)]) -> Vec<&str> {
    places.iter().map(|(_, name)| name.as_str()).collect()
}

/// A pin dropped where GTK's sidebar takes it moves in the bookmarks file beside the pin the drop
/// names, or into a gap before the pin after it: with a pin of the home folder, which the
/// sidebar does not show, between `a` and `b`, the results are GTK's.
#[test]
fn a_pin_moves_beside_the_pin_its_drop_names_in_the_whole_file() {
    let shown: Vec<Location> = pins(&["a", "b"]).into_iter().map(|(at, _)| at).collect();
    // (the pin dragged, landing, the file after, moved)
    let cases: [(usize, Landing, [&str; 3], bool); 5] = [
        (
            0,
            Landing::Row {
                index: 1,
                after: false,
            },
            ["home", "a", "b"],
            true,
        ),
        (
            0,
            Landing::Row {
                index: 1,
                after: true,
            },
            ["home", "b", "a"],
            true,
        ),
        (0, Landing::Gap(2), ["home", "b", "a"], true),
        (1, Landing::Gap(0), ["b", "a", "home"], true),
        (1, Landing::Gap(1), ["a", "home", "b"], false),
    ];
    for (from, landing, expected, moved) in cases {
        let mut places = pins(&["a", "home", "b"]);
        assert_eq!(
            move_pin_in(&mut places, &shown[from], landing, &shown),
            moved,
            "{landing:?}"
        );
        assert_eq!(names(&places), expected, "pin {from} at {landing:?}");
    }
}

#[test]
fn a_pin_moves_beside_its_target_and_a_move_into_its_own_place_changes_nothing() {
    // (source, target, after, the order after, moved)
    let cases: [(usize, usize, bool, [&str; 3], bool); 4] = [
        (2, 0, true, ["one", "three", "two"], true),
        (2, 0, false, ["three", "one", "two"], true),
        (0, 2, true, ["two", "three", "one"], true),
        (0, 1, false, ["one", "two", "three"], false),
    ];
    for (source, target, after, expected, moved) in cases {
        let mut places = pins(&["one", "two", "three"]);
        let did = reorder_pinned_places(&mut places, source, target, after);
        assert_eq!(did, moved, "{source} by {target}, after {after}");
        assert_eq!(
            names(&places),
            expected,
            "{source} by {target}, after {after}"
        );
    }
}
