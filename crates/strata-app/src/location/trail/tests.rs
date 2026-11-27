// SPDX-License-Identifier: MIT

use strata_core::model::Location;

use super::of;

fn labels(location: &str, home: &str) -> Vec<(String, bool)> {
    let (trail, _) = of(&Location::local(location), &Location::local(home));
    trail
        .crumbs
        .into_iter()
        .map(|crumb| (crumb.label, crumb.joins_next))
        .collect()
}

#[test]
fn a_folder_under_home_starts_at_the_tilde() {
    assert_eq!(
        labels("/home/ada/Documents/drafts", "/home/ada"),
        [
            ("~".to_owned(), false),
            ("Documents".to_owned(), false),
            ("drafts".to_owned(), false),
        ]
    );
}

#[test]
fn a_folder_outside_home_starts_at_the_root_which_takes_no_separator() {
    assert_eq!(
        labels("/srv/media", "/home/ada"),
        [
            ("/".to_owned(), true),
            ("srv".to_owned(), false),
            ("media".to_owned(), false),
        ]
    );
}

#[test]
fn each_crumb_opens_its_own_folder_and_the_entry_starts_on_the_full_path() {
    let shown = Location::local("/home/ada/Documents");
    let (trail, folders) = of(&shown, &Location::local("/home/ada"));
    assert_eq!(trail.text, shown.display_path());
    for crumb in &trail.crumbs {
        let index: usize = crumb.id.parse().expect("an index");
        assert_eq!(crumb.description, folders[index].display_path());
    }
    assert_eq!(folders.last(), Some(&shown));
}

#[test]
fn the_administrator_lock_marks_the_root_only_as_an_ancestor() {
    let home = Location::local("/home/ada");
    let badges = |uri: &str| -> Vec<bool> {
        let (trail, _) = of(&Location::uri(uri), &home);
        trail
            .crumbs
            .iter()
            .map(|crumb| crumb.badge.is_some())
            .collect()
    };
    assert_eq!(badges("admin:///etc"), [true, false]);
    assert_eq!(badges("admin:///"), [false]);
}
