// SPDX-License-Identifier: MIT

use strata_core::model::Location;

use super::{neighbour, reorder_pinned_places, reorder_places};

#[test]
fn a_move_lands_next_to_its_new_neighbour() {
    // (block length, from, to, neighbour and after)
    let cases = [
        (3, 2, 0, Some((0, false))),
        (3, 0, 2, Some((2, true))),
        (3, 2, 1, Some((0, true))),
        (3, 0, 1, Some((1, true))),
        (3, 1, 1, None),
        (1, 0, 0, None),
        (3, 3, 0, None),
    ];
    for (len, from, to, expected) in cases {
        assert_eq!(
            neighbour(len, from, to),
            expected,
            "{len} rows, {from} to {to}"
        );
    }
}

#[test]
fn a_place_moves_in_the_whole_saved_order_and_hidden_places_keep_theirs() {
    // Network sits between Trash and Desktop, hidden from the block.
    let mut order = vec!["home", "trash", "network", "desktop"];
    assert!(reorder_places(&mut order, "home", "trash", true));
    assert_eq!(order, ["trash", "home", "network", "desktop"]);
    assert!(reorder_places(&mut order, "desktop", "trash", false));
    assert_eq!(order, ["desktop", "trash", "home", "network"]);
    assert!(!reorder_places(&mut order, "home", "home", true));
    assert!(!reorder_places(&mut order, "gone", "home", true));
    assert_eq!(order, ["desktop", "trash", "home", "network"]);
}

fn names(places: &[(Location, String)]) -> Vec<&str> {
    places.iter().map(|(_, name)| name.as_str()).collect()
}

#[test]
fn a_pin_moves_beside_its_target_and_a_move_into_its_own_place_changes_nothing() {
    let pins = || {
        ["one", "two", "three"]
            .map(|name| {
                (
                    Location::uri(format!("sftp://host/{name}")),
                    name.to_owned(),
                )
            })
            .to_vec()
    };
    // (source, target, after, the order after, moved)
    let cases: [(usize, usize, bool, [&str; 3], bool); 4] = [
        (2, 0, true, ["one", "three", "two"], true),
        (2, 0, false, ["three", "one", "two"], true),
        (0, 2, true, ["two", "three", "one"], true),
        (0, 1, false, ["one", "two", "three"], false),
    ];
    for (source, target, after, expected, moved) in cases {
        let mut places = pins();
        assert_eq!(
            reorder_pinned_places(&mut places, source, target, after),
            moved,
            "{source} by {target}, after {after}"
        );
        assert_eq!(
            names(&places),
            expected,
            "{source} by {target}, after {after}"
        );
    }
}
