// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use strata_fs::services::SearchItem;

use super::{Renamed, RenamedResult};

/// Result names an update lists, whether the index finished walking, and whether it follows.
type Update = (&'static [&'static str], bool, bool);

const FROM: &str = "/work/c.json";
const TO: &str = "/work/0.md";

fn results(names: &[&str]) -> Vec<SearchItem> {
    names
        .iter()
        .map(|name| SearchItem::for_test(Path::new("/work").join(name), false))
        .collect()
}

fn followed(to: &str) -> Option<Renamed> {
    Some(Renamed {
        from: PathBuf::from(FROM),
        to: PathBuf::from(to),
    })
}

#[test]
fn renamed_result_follows_the_first_update_listing_its_new_path() {
    let cases: &[(&str, Option<&str>, &[Update])] = &[
        (
            "the new path replaces the old one",
            Some(TO),
            &[(&["d.txt", "0.md", "a.csv"], true, true)],
        ),
        (
            "an update from before the rename keeps it waiting",
            Some(TO),
            &[
                (&["d.txt", "c.json", "a.csv"], true, false),
                (&["0.md", "d.txt", "a.csv"], true, true),
            ],
        ),
        (
            "a colliding target already listed keeps waiting while the old path stays",
            Some("/work/b.csv"),
            &[(&["a.csv", "b.csv", "c.json", "d.txt"], true, false)],
        ),
        (
            "an incomplete walk without the item waits, and a complete miss ends it",
            Some(TO),
            &[
                (&["d.txt", "a.csv"], false, false),
                (&["d.txt", "a.csv"], true, false),
                (&["d.txt", "0.md", "a.csv"], true, false),
            ],
        ),
        (
            "it follows only once",
            Some(TO),
            &[
                (&["0.md", "d.txt"], true, true),
                (&["0.md", "d.txt"], true, false),
            ],
        ),
        (
            "an item moved to another folder never follows",
            None,
            &[
                (&["d.txt", "c.json", "a.csv"], true, false),
                (&["d.txt", "a.csv"], true, false),
            ],
        ),
    ];
    for (case, to, updates) in cases {
        let mut pending = RenamedResult::default();
        pending.expect(Path::new(FROM), to.map(Path::new));
        for (index, (names, complete, follows)) in updates.iter().enumerate() {
            let expected = follows
                .then(|| followed(to.expect("followed target")))
                .flatten();
            assert_eq!(
                pending.settle(&results(names), *complete),
                expected,
                "{case}: update {index}"
            );
        }
    }
}

#[test]
fn forgetting_an_abandoned_rename_keeps_the_newer_one() {
    let older = (Path::new("/work/a.csv"), Some(Path::new("/work/a.md")));
    let newer = (Path::new(FROM), Some(Path::new(TO)));
    let mut pending = RenamedResult::default();
    pending.expect(older.0, older.1);
    pending.expect(newer.0, newer.1);
    pending.clear_if(older.0, older.1);
    assert_eq!(
        pending.settle(&results(&["0.md", "d.txt"]), true),
        followed(TO),
        "the abandoned rename's cleanup must leave the newer rename followed"
    );

    pending.expect(newer.0, newer.1);
    pending.clear_if(newer.0, None);
    assert_eq!(
        pending.settle(&results(&["0.md", "d.txt"]), true),
        followed(TO),
        "forgetting a move of the same item must leave its rename followed"
    );

    pending.expect(newer.0, newer.1);
    pending.clear_if(newer.0, newer.1);
    assert_eq!(
        pending.settle(&results(&["0.md", "d.txt"]), true),
        None,
        "a failed rename forgets its own follow"
    );
}
