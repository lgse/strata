// SPDX-License-Identifier: MIT

use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
    time::Duration,
};

use strata_core::model::Location;

use strata_fs::adapters::RevealRequest;
use strata_platform::single_instance::{
    Activation, CommandLine, Event, Launch, Reveal, RevealMethod,
};

use super::{Asked, Opening, heard, openings, queue, queue_first, revealed, take, take_asked};
use crate::{
    host::offscreen::Offscreen,
    launch::Initial,
    properties,
    test_support::{PATIENCE, folder_of, hosted},
};

const MODULE: &str = "application::tests";

fn argv(list: &[&[u8]]) -> Vec<OsString> {
    std::iter::once(&b"strata"[..])
        .chain(list.iter().copied())
        .map(|argument| OsString::from_vec(argument.to_vec()))
        .collect()
}

fn argument(path: impl Into<PathBuf>) -> Opening {
    Opening::Argument(Location::local(path.into()))
}

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

#[test]
fn a_window_takes_the_token_of_what_asked_for_it() {
    let token = |token: &str| Activation {
        xdg_activation_token: Some(token.to_owned()),
        desktop_startup_id: None,
    };
    let reveal = |startup_id: &str| Reveal {
        method: RevealMethod::Folders,
        uris: vec!["file:///srv/a".to_owned()],
        startup_id: startup_id.to_owned(),
    };
    let folder = Opening::Reveal(RevealRequest {
        directory: Location::local("/srv/a"),
        selection: Vec::new(),
        properties: false,
    });

    let folder_a = tempfile::tempdir().expect("a folder");
    let folder_b = tempfile::tempdir().expect("a folder");
    let launch = |args: Vec<OsString>, activation| {
        let launch = Launch {
            args,
            cwd: Some(PathBuf::from("/")),
            activation,
        };
        Event::CommandLine(CommandLine::unanswered(launch))
    };
    let both = vec![
        OsString::from("strata"),
        folder_a.path().into(),
        folder_b.path().into(),
    ];
    let argument = |path: &Path| Opening::Argument(Location::local(path));
    let uri = |path: &Path| format!("file://{}", path.display());

    heard(launch(both, token("from-the-terminal")));
    heard(launch(
        vec![OsString::from("strata")],
        Activation::default(),
    ));
    heard(Event::Open {
        uris: vec![uri(folder_a.path())],
        hint: String::new(),
        activation: token("from-the-menu"),
    });
    heard(Event::Activate(token("from-the-launcher")));
    heard(Event::Activate(Activation::default()));
    heard(Event::Reveal(reveal("from-the-browser")));
    heard(Event::Reveal(reveal("")));

    let asked = |opening: &Opening, token: Option<&str>| Asked {
        opening: opening.clone(),
        token: token.map(str::to_owned),
    };
    assert_eq!(
        take_asked(),
        vec![
            asked(&argument(folder_a.path()), Some("from-the-terminal")),
            asked(&argument(folder_b.path()), Some("from-the-terminal")),
            asked(&Opening::Plain, None),
            asked(&argument(folder_a.path()), Some("from-the-menu")),
            asked(&Opening::Present, Some("from-the-launcher")),
            asked(&Opening::Present, None),
            asked(&folder, Some("from-the-browser")),
            asked(&folder, None),
        ]
    );
}

#[test]
fn a_reveal_left_before_it_lists_shows_no_properties_later() {
    hosted(
        MODULE,
        "a_reveal_left_before_it_lists_shows_no_properties_later",
        |mut host| {
            let folder = folder_of(&["file.txt"]);
            let elsewhere = folder_of(&["other.txt"]);
            let here = Location::local(folder.path());
            let away = Location::local(elsewhere.path());
            let request = RevealRequest {
                directory: here.clone(),
                selection: vec![Location::local(folder.path().join("file.txt"))],
                properties: true,
            };
            let gone = away.clone();
            host.runtime().update(move |app, _| {
                app.start(Initial::Reveal(request));
                app.browser.navigate_location(gone, true);
            });
            let left = host.run_until(PATIENCE, |host| host.app().location() == Some(&away));
            assert!(left, "the window went to the other folder");

            let back = here.clone();
            host.runtime()
                .update(move |app, _| app.browser.navigate_location(back, true));
            let listed = host.run_until(PATIENCE, |host| {
                host.app().location() == Some(&here)
                    && host.app().browser().column_entry_counts(0).is_some()
            });
            assert!(listed, "the revealed folder listed on the second visit");
            host.run_until(Duration::from_millis(500), |_| false);
            assert_eq!(properties::shown_location(host.app()), None);
        },
    );
}

#[test]
fn a_reveal_that_fails_to_list_uses_its_properties_up_at_the_reload() {
    use std::{cell::Cell, os::unix::fs::PermissionsExt, rc::Rc};

    use strata_fs::app::{Browser, BrowserEvent};

    hosted(
        MODULE,
        "a_reveal_that_fails_to_list_uses_its_properties_up_at_the_reload",
        |mut host| {
            let folder = folder_of(&["file.txt"]);
            let here = Location::local(folder.path());
            let file = Location::local(folder.path().join("file.txt"));
            let mode = |bits| std::fs::Permissions::from_mode(bits);
            std::fs::set_permissions(folder.path(), mode(0o000)).expect("chmod 000");
            let (failed, listed) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(0)));
            let (heard_failed, heard_listed) = (failed.clone(), listed.clone());
            host.app().browser.observe(move |event| match event {
                BrowserEvent::LoadFailed { .. } => heard_failed.set(true),
                BrowserEvent::LoadFinished { .. } => heard_listed.set(heard_listed.get() + 1),
                _ => {}
            });
            let request = RevealRequest {
                directory: here.clone(),
                selection: vec![file.clone()],
                properties: true,
            };
            host.start(Initial::Reveal(request));
            let unreadable = host.run_until(PATIENCE, |_| failed.get());
            std::fs::set_permissions(folder.path(), mode(0o755)).expect("chmod 755");
            assert!(unreadable, "the unreadable folder failed to list");
            assert_eq!(host.app().location(), Some(&here));

            let reload = |host: &mut Offscreen, listed: &Rc<Cell<usize>>| {
                let before = listed.get();
                let browser = host.app().browser.clone();
                host.runtime().update(move |_, _| browser.reload_active());
                let done = host.run_until(PATIENCE, |_| listed.get() > before);
                assert!(done, "the reload listed the folder");
                host.run_until(Duration::from_millis(300), |_| false);
            };
            reload(&mut host, &listed);
            let focused = Browser::focused_item(&host.app().browser).map(|item| item.2.location);
            assert_eq!(focused, None, "the failed load took the reveal's selection");
            assert_eq!(properties::shown_location(host.app()), None);

            let names = ["file.txt".to_owned()];
            let browser = host.app().browser.clone();
            host.runtime().update(move |_, _| {
                let _ = browser.select_entries_by_name_at(0, &names);
            });
            reload(&mut host, &listed);
            let focused = Browser::focused_item(&host.app().browser).map(|item| item.2.location);
            assert_eq!(
                focused,
                Some(file),
                "the file is focused for the second reload"
            );
            assert_eq!(
                properties::shown_location(host.app()),
                None,
                "the request was used up"
            );
        },
    );
}
