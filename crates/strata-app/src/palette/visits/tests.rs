// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use super::*;

fn local(path: &str) -> Location {
    Location::local(PathBuf::from(path))
}

fn added(depth: usize, path: &str) -> Visit {
    Visit::Added {
        depth,
        location: local(path),
    }
}

/// Each case's events, in order, and the folders they record.
#[test]
fn a_column_is_recorded_once_its_folder_finishes_listing() {
    let cases: [(&str, Vec<Visit>, &[&str]); 6] = [
        (
            "a folder that lists",
            vec![added(0, "/home/me"), Visit::Loaded(0)],
            &["/home/me"],
        ),
        (
            "nothing before it lists",
            vec![added(0, "/home/me")],
            &[],
        ),
        (
            "another column's load records nothing",
            vec![added(1, "/home/me/a"), Visit::Loaded(0), Visit::Loaded(1)],
            &["/home/me/a"],
        ),
        (
            "once per load",
            vec![added(0, "/home/me"), Visit::Loaded(0), Visit::Loaded(0)],
            &["/home/me"],
        ),
        (
            "a truncation forgets the columns it drops",
            vec![
                added(0, "/home/me"),
                added(1, "/home/me/a"),
                Visit::Truncated(1),
                Visit::Loaded(1),
                Visit::Loaded(0),
            ],
            &["/home/me"],
        ),
        (
            "a reset forgets every column",
            vec![added(0, "/home/me"), Visit::Reset, Visit::Loaded(0)],
            &[],
        ),
    ];
    for (case, visits, expected) in cases {
        let mut recorder = Visits::default();
        let recorded: Vec<PathBuf> = visits
            .into_iter()
            .filter_map(|visit| recorder.handle(visit))
            .collect();
        let expected: Vec<PathBuf> = expected.iter().map(PathBuf::from).collect();
        assert_eq!(recorded, expected, "{case}");
    }
}

#[test]
fn a_folder_without_a_local_path_is_not_recorded() {
    let mut recorder = Visits::default();
    assert_eq!(
        recorder.handle(Visit::Added {
            depth: 0,
            location: Location::uri("trash:///"),
        }),
        None
    );
    assert_eq!(recorder.handle(Visit::Loaded(0)), None);
}
