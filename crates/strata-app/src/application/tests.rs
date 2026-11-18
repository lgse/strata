// SPDX-License-Identifier: MIT

use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

use strata_core::model::Location;

use strata_fs::adapters::RevealRequest;
use strata_platform::single_instance::{Reveal, RevealMethod};

use super::{Opening, openings, queue, queue_first, revealed, take};
use crate::{
    host::offscreen::Offscreen,
    launch::Initial,
    properties,
    test_support::{PATIENCE, folder_of, hosted},
};

const MODULE: &str = "application::tests";

fn argv(list: &[&[u8]]) -> Vec<OsString> {
    std::iter::once(&b"strata-rune"[..])
        .chain(list.iter().copied())
        .map(|argument| OsString::from_vec(argument.to_vec()))
        .collect()
}

fn argument(path: impl Into<PathBuf>) -> Opening {
    Opening::Argument(Location::local(path.into()))
}

/// A launch's arguments after the program, whether another process handed it over, and the
/// windows it asks for.
type Case = (&'static [&'static [u8]], bool, Vec<Opening>);

#[test]
fn a_launch_asks_for_a_window_for_each_path_it_names() {
    let cwd = Path::new("/srv/launch");
    let cases: [Case; 7] = [
        (&[], false, vec![Opening::Plain]),
        (&[], true, vec![Opening::Present]),
        (
            &[b"first", b"/srv/second"],
            true,
            vec![argument("/srv/launch/first"), argument("/srv/second")],
        ),
        (
            &[b"caf\xe9"],
            true,
            vec![argument(PathBuf::from(OsString::from_vec(
                b"/srv/launch/caf\xe9".to_vec(),
            )))],
        ),
        (&[b"file:///srv/third"], false, vec![argument("/srv/third")]),
        // The launch read its options before it was handed over: what is left of them is no
        // window, and `--` ends them.
        (
            &[b"--unknown", b"--", b"-dashed", b"--"],
            true,
            vec![argument("/srv/launch/-dashed"), argument("/srv/launch/--")],
        ),
        (&[b"--unknown"], false, vec![Opening::Plain]),
    ];
    for (arguments, handed_over, expected) in cases {
        let arguments = argv(arguments);

        let asked = openings(&arguments, Some(cwd), handed_over);

        assert_eq!(asked, expected, "{arguments:?}");
    }
}

#[test]
fn the_host_takes_what_was_asked_in_order_the_first_launch_first() {
    queue([argument("/srv/later")]);
    queue_first([Opening::Plain, argument("/srv/own")]);

    assert_eq!(
        take(),
        vec![Opening::Plain, argument("/srv/own"), argument("/srv/later")]
    );
    assert_eq!(take(), Vec::new());
}

fn selected(host: &Offscreen) -> Vec<Location> {
    let entries = host.app().browser().selected_entries();
    entries.into_iter().map(|entry| entry.location).collect()
}

/// As `classify` in `src/ui/window/open_argument.rs`: a folder opens; a file, a link whose
/// target is gone, or one of two names that show alike, shows alone selected in its folder.
#[test]
fn an_argument_opens_its_folder_or_shows_its_file_selected_in_its_folder() {
    hosted(
        MODULE,
        "an_argument_opens_its_folder_or_shows_its_file_selected_in_its_folder",
        |mut host| {
            let folder = folder_of(&["file.txt", "other.txt"]);
            let root = folder.path();
            std::fs::create_dir(root.join("inner")).expect("a folder");
            std::os::unix::fs::symlink(root.join("gone.txt"), root.join("dangling.txt"))
                .expect("a dangling link");
            let alike = |byte: u8| {
                root.join(OsString::from_vec(
                    [b"alike-", &[byte][..], b".txt"].concat(),
                ))
            };
            for name in [alike(0xe8), alike(0xe9)] {
                std::fs::write(name, "alike").expect("a file whose name is not UTF-8");
            }
            let here = Location::local(root);
            let cases = [
                (
                    root.join("inner"),
                    Location::local(root.join("inner")),
                    None,
                ),
                (
                    root.join("file.txt"),
                    here.clone(),
                    Some(root.join("file.txt")),
                ),
                (
                    root.join("dangling.txt"),
                    here.clone(),
                    Some(root.join("dangling.txt")),
                ),
                (alike(0xe9), here.clone(), Some(alike(0xe9))),
            ];
            for (path, shown, chosen) in cases {
                let chosen: Vec<Location> = chosen.into_iter().map(Location::local).collect();
                host.app().open(Location::local(std::env::temp_dir()));

                host.start(Initial::Argument(Location::local(&path)));
                let opened = host.run_until(PATIENCE, |host| {
                    host.app().location() == Some(&shown)
                        && (chosen.is_empty() || selected(host) == chosen)
                });

                assert!(opened, "{path:?} showed {:?}", host.app().location());
                if !chosen.is_empty() {
                    assert_eq!(selected(&host), chosen, "{path:?}");
                }
            }
        },
    );
}

/// A FileManager1 call opens a window per folder, as GTK groups it: the folder itself for
/// `ShowFolders`, and for the items the folder they are in, with them selected.
#[test]
fn a_file_manager_call_asks_for_a_window_per_folder() {
    let call = |method, uris: &[&str]| Reveal {
        method,
        uris: uris.iter().map(|uri| (*uri).to_owned()).collect(),
        startup_id: String::new(),
    };
    let reveal = |directory: &str, selection: &[&str], properties| {
        Opening::Reveal(RevealRequest {
            directory: Location::local(directory),
            selection: selection.iter().map(Location::local).collect(),
            properties,
        })
    };
    let cases = [
        (
            call(RevealMethod::Folders, &["file:///srv/a", "file:///srv/b"]),
            vec![reveal("/srv/a", &[], false), reveal("/srv/b", &[], false)],
        ),
        (
            call(
                RevealMethod::Items,
                &[
                    "file:///srv/a/one.txt",
                    "file:///srv/b/two.txt",
                    "file:///srv/a/three.txt",
                ],
            ),
            vec![
                reveal("/srv/a", &["/srv/a/one.txt", "/srv/a/three.txt"], false),
                reveal("/srv/b", &["/srv/b/two.txt"], false),
            ],
        ),
        (
            call(RevealMethod::ItemProperties, &["file:///srv/a/one.txt"]),
            vec![reveal("/srv/a", &["/srv/a/one.txt"], true)],
        ),
    ];
    for (call, expected) in cases {
        assert_eq!(revealed(&call), expected, "{call:?}");
    }
}

/// `ShowItemProperties` shows the revealed item's Properties once its folder has listed, and
/// `ShowItems` shows none.
#[test]
fn a_reveal_selects_its_items_and_shows_their_properties_when_asked() {
    hosted(
        MODULE,
        "a_reveal_selects_its_items_and_shows_their_properties_when_asked",
        |mut host| {
            let folder = folder_of(&["file.txt", "other.txt"]);
            let here = Location::local(folder.path());
            let file = Location::local(folder.path().join("file.txt"));
            for properties in [false, true] {
                host.app().open(Location::local(std::env::temp_dir()));
                let request = RevealRequest {
                    directory: here.clone(),
                    selection: vec![file.clone()],
                    properties,
                };

                host.start(Initial::Reveal(request));
                let revealed = host.run_until(PATIENCE, |host| {
                    host.app().location() == Some(&here)
                        && selected(host) == vec![file.clone()]
                        && (!properties || properties::shown_location(host.app()).is_some())
                });

                assert!(
                    revealed,
                    "properties {properties}: {:?}",
                    host.app().location()
                );
                let shown = properties::shown_location(host.app()).cloned();
                assert_eq!(
                    shown,
                    properties.then(|| file.clone()),
                    "properties {properties}"
                );
            }
        },
    );
}
