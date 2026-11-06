// SPDX-License-Identifier: MIT

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use super::{Renamed, RenamedResult, Settled};
use crate::services::SearchItem;

/// What an update makes of the pending rename.
#[derive(Clone, Copy)]
enum Outcome {
    Waits,
    Follows,
    Leaves,
}

/// Result names an update lists, and what it settles.
type Update = (&'static [&'static str], Outcome);

const FROM: &str = "/work/c.json";
const TO: &str = "/work/0.md";

fn results(names: &[&str]) -> Vec<SearchItem> {
    names
        .iter()
        .map(|name| SearchItem::for_test(Path::new("/work").join(name), false))
        .collect()
}

fn settled(outcome: Outcome) -> Option<Settled> {
    match outcome {
        Outcome::Waits => None,
        Outcome::Follows => Some(Settled::Followed(Renamed {
            from: PathBuf::from(FROM),
            to: PathBuf::from(TO),
        })),
        Outcome::Leaves => Some(Settled::Left),
    }
}

#[test]
fn renamed_result_follows_the_first_update_listing_its_new_path() {
    use Outcome::{Follows, Leaves, Waits};
    // Each case renames `c.json` to `0.md`, or with no new path moves it to another folder.
    let cases: &[(&str, Option<&str>, &[Update])] = &[
        (
            "the new path replaces the old one",
            Some(TO),
            &[(&["d.txt", "0.md", "a.csv"], Follows)],
        ),
        (
            "an update from before the rename keeps it waiting",
            Some(TO),
            &[
                (&["d.txt", "c.json", "a.csv"], Waits),
                (&["0.md", "d.txt", "a.csv"], Follows),
            ],
        ),
        (
            "a new name the query no longer matches ends it",
            Some(TO),
            &[
                (&["d.txt", "a.csv"], Waits),
                (&["d.txt", "0.md", "a.csv"], Waits),
            ],
        ),
        (
            "it follows only once",
            Some(TO),
            &[(&["0.md", "d.txt"], Follows), (&["0.md", "d.txt"], Waits)],
        ),
        (
            "an item moved to another folder leaves once its old path goes",
            None,
            &[
                (&["d.txt", "c.json", "a.csv"], Waits),
                (&["d.txt", "a.csv"], Leaves),
                (&["d.txt", "a.csv"], Waits),
            ],
        ),
    ];
    for (case, to, updates) in cases {
        let pending = RenamedResult::default();
        pending.expect(Path::new(FROM), to.map(Path::new));
        for (update, (names, outcome)) in updates.iter().enumerate() {
            assert_eq!(
                pending.settle(&results(names)),
                settled(*outcome),
                "{case}: update {update}"
            );
        }
    }
}

#[test]
fn pruning_moves_the_renamed_result_to_its_new_path_and_drops_other_missing_ones() {
    let gone: HashSet<_> = ["c.json", "a.csv"]
        .iter()
        .map(|name| Path::new("/work").join(name))
        .collect();
    let listed = |items: Vec<SearchItem>| {
        items
            .into_iter()
            .map(|item| (item.name, item.path))
            .collect::<Vec<_>>()
    };
    let pending = RenamedResult::default();
    assert_eq!(
        listed(pending.prune(results(&["d.txt", "c.json", "a.csv"]), &gone, |_| true)),
        [("d.txt".to_owned(), PathBuf::from("/work/d.txt"))],
        "without a rename every missing result is dropped"
    );
    pending.expect(Path::new(FROM), Some(Path::new(TO)));
    assert_eq!(
        listed(pending.prune(results(&["d.txt", "c.json", "a.csv"]), &gone, |_| true)),
        [
            ("d.txt".to_owned(), PathBuf::from("/work/d.txt")),
            ("0.md".to_owned(), PathBuf::from("/work/0.md")),
        ],
        "the renamed result keeps its place under its new name"
    );
    assert_eq!(
        listed(
            pending.prune(results(&["d.txt", "c.json", "a.csv"]), &gone, |item| !item
                .name
                .starts_with('0'))
        ),
        [("d.txt".to_owned(), PathBuf::from("/work/d.txt"))],
        "a new name the view would not show, such as a hidden one, leaves the results"
    );
    pending.expect(Path::new(FROM), None);
    assert_eq!(
        listed(pending.prune(results(&["d.txt", "c.json", "a.csv"]), &gone, |_| true)),
        [("d.txt".to_owned(), PathBuf::from("/work/d.txt"))],
        "an item moved to another folder is dropped, not kept as a phantom"
    );
}

#[test]
fn forgetting_an_abandoned_rename_keeps_the_newer_one() {
    let older = (Path::new("/work/a.csv"), Some(Path::new("/work/a.md")));
    let newer = (Path::new(FROM), Some(Path::new(TO)));
    let pending = RenamedResult::default();
    pending.expect(older.0, older.1);
    pending.expect(newer.0, newer.1);
    pending.clear_if(older.0, older.1);
    assert_eq!(
        pending.settle(&results(&["0.md", "d.txt"])),
        settled(Outcome::Follows),
        "the abandoned rename's cleanup must leave the newer rename followed"
    );
    pending.expect(newer.0, newer.1);
    pending.clear_if(newer.0, None);
    assert_eq!(
        pending.settle(&results(&["0.md", "d.txt"])),
        settled(Outcome::Follows),
        "forgetting a move of the same item must leave its rename followed"
    );
    pending.expect(newer.0, newer.1);
    pending.clear_if(newer.0, newer.1);
    assert_eq!(
        pending.settle(&results(&["0.md", "d.txt"])),
        None,
        "a failed rename forgets its own follow"
    );
}
