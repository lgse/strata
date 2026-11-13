// SPDX-License-Identifier: MIT

use std::path::Path;

use strata_core::model::Location;

use super::{MOST, index_of, model};

fn rows(back: &[Location]) -> Vec<(String, Option<String>, Option<usize>)> {
    let home = Path::new("/home/ada");
    let menu = model("BACK TO", back, home);
    assert_eq!(menu.header, ["BACK TO"]);
    menu.sections
        .iter()
        .flat_map(|section| &section.items)
        .map(|item| {
            let index = index_of(&item.action);
            (item.label.clone(), item.description.clone(), index)
        })
        .collect()
}

/// `fill_back_history`: the most recent first, each by its name or `~` for home, described
/// by its path under `~`, and each naming how far back it is.
#[test]
fn each_entry_is_named_described_and_knows_how_far_back_it_is() {
    let back = [
        Location::local("/home/ada/Documents"),
        Location::local("/home/ada"),
        Location::local("/srv/media"),
    ];
    assert_eq!(
        rows(&back),
        [
            (
                "Documents".to_owned(),
                Some("~/Documents".to_owned()),
                Some(0)
            ),
            ("~".to_owned(), Some("~/".to_owned()), Some(1)),
            ("media".to_owned(), Some("/srv/media".to_owned()), Some(2)),
        ]
    );
}

/// A longer history lists its most recent entries only; Back reaches the rest.
#[test]
fn a_long_history_lists_its_most_recent_entries() {
    let back: Vec<_> = (0..MOST + 5)
        .map(|n| Location::local(format!("/srv/{n}")))
        .collect();
    let listed = rows(&back);
    assert_eq!(listed.len(), MOST);
    assert_eq!(listed.last().map(|row| row.0.as_str()), Some("49"));
}
