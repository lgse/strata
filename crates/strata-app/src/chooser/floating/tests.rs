// SPDX-License-Identifier: MIT

use std::{future::Future, path::Path};

use rune::Tx;
use strata_core::model::Location;
use strata_test_support::PrivateContext;

use super::{FloatingRequest, existing_folder, take, transferable};
use crate::{
    App,
    commands::{self, On, Outcome},
    host::offscreen::Offscreen,
    portal::{ChooserKind, PortalError, SelectedResult, local_uri},
    shell,
    test_support::{PATIENCE, hosted, view::idle},
};

const MODULE: &str = "chooser::floating::tests";

fn run_async<T>(future: impl Future<Output = T>) -> T {
    let context = PrivateContext::new();
    context
        .with_thread_default(|| context.block_on(future))
        .expect("test main context")
}

/// Opens `root` and waits for it to list `name`.
fn open_listing(host: &mut Offscreen, root: &Path, name: &str) {
    host.app().open(Location::local(root));
    let wanted = Location::local(root.join(name));
    let listed = |host: &Offscreen| {
        let browser = host.app().browser();
        let count = browser
            .column_entry_counts(0)
            .map_or(0, |counts| counts.total);
        browser.active_location().as_ref() == Some(&Location::local(root))
            && browser
                .with_entries(0, 0..count, |entries| {
                    entries.iter().any(|entry| entry.location == wanted)
                })
                .unwrap_or(false)
    };
    assert!(host.run_until(PATIENCE, listed), "the folder lists {name}");
    while host.turn() {}
}

/// Runs the row `command` on `path` and takes the chooser it asked for.
fn ask(host: &mut Offscreen, command: &'static str, path: &Path) -> Option<FloatingRequest> {
    ask_at(host, (command, 0), path)
}

/// [`ask`], on a row of the column at `depth`.
fn ask_at(
    host: &mut Offscreen,
    (command, depth): (&'static str, usize),
    path: &Path,
) -> Option<FloatingRequest> {
    let id = shell::id_of(host.app(), &Location::local(path));
    host.runtime().update(move |app: &mut App, tx: &mut Tx| {
        assert_eq!(
            commands::run(app, command, On::Row((depth, id)), tx),
            Outcome::Ran
        );
    });
    idle(host);
    take(host.app_mut())
}

fn copy_through_chooser(host: &mut Offscreen, (depth, source): (usize, &Path), folder: &Path) {
    let request = ask_at(host, ("copy-to", depth), source).expect("a chooser");
    answer(host, request, Some(folder));
    let copied = folder.join(source.file_name().expect("a name"));
    assert!(
        host.run_until(PATIENCE, |_| copied.exists()),
        "the copy lands"
    );
    idle(host);
}

fn answer(host: &mut Offscreen, request: FloatingRequest, folder: Option<&Path>) {
    let result = match folder {
        Some(folder) => Ok(SelectedResult {
            uris: vec![local_uri(folder).expect("a local URI")],
            ..SelectedResult::default()
        }),
        None => Err(PortalError::Cancelled("dismissed".into())),
    };
    (request.completion)(result);
    idle(host);
}

fn assert_folder_chooser(
    host: &Offscreen,
    request: &FloatingRequest,
    (title, accept): (&str, &str),
    base: &Path,
) {
    let asked = &request.request;
    let browser = host.app().window_size();
    assert!(browser.is_some(), "the browser window has been laid out");
    assert_eq!(
        asked.parent_size_hint, browser,
        "{title}: sized against the browser"
    );
    assert_eq!(
        (asked.title.as_str(), asked.accept_label.as_str()),
        (title, accept)
    );
    assert_eq!(
        (&asked.kind, asked.initial_directory.as_path()),
        (
            &ChooserKind::Open {
                directory: true,
                multiple: false
            },
            base
        )
    );
}

#[test]
fn the_destination_rows_ask_for_a_folder_chooser_and_act_on_its_answer() {
    hosted(
        MODULE,
        "the_destination_rows_ask_for_a_folder_chooser_and_act_on_its_answer",
        |mut host| {
            let temp = tempfile::tempdir().expect("a folder");
            let dest = temp.path().join("dest");
            std::fs::create_dir(&dest).expect("dest");
            for name in ["copied.txt", "moved.txt", "bundle.zip"] {
                std::fs::write(temp.path().join(name), "x\n").expect("a file");
            }
            for (command, name, title, accept) in [
                ("copy-to", "copied.txt", "Copy to", "Copy here"),
                ("move-to", "moved.txt", "Move to", "Move here"),
                ("extract-to", "bundle.zip", "Extract to", "Extract here"),
            ] {
                open_listing(&mut host, temp.path(), name);
                let source = temp.path().join(name);
                let request = ask(&mut host, command, &source).expect(command);
                assert_folder_chooser(&host, &request, (title, accept), temp.path());
                assert!(
                    ask(&mut host, command, &source).is_none(),
                    "{command}: one chooser at a time"
                );
                assert!(
                    super::take_raise(host.app_mut()),
                    "{command}: asked again, the open one is brought forward"
                );
                let chosen = (command != "extract-to").then_some(dest.as_path());
                answer(&mut host, request, chosen);
            }
            let landed =
                |_: &Offscreen| dest.join("moved.txt").exists() && dest.join("copied.txt").exists();
            assert!(
                host.run_until(PATIENCE, landed),
                "both transfers land in the folder chosen"
            );
            assert!(
                temp.path().join("copied.txt").exists(),
                "a copy leaves its source"
            );
            assert!(
                !temp.path().join("moved.txt").exists(),
                "a move takes its source"
            );
            open_listing(&mut host, temp.path(), "bundle.zip");
            let again = ask(&mut host, "extract-to", &temp.path().join("bundle.zip"));
            assert!(
                again.is_some(),
                "a cancelled chooser lets the next one open"
            );
        },
    );
}

#[test]
fn a_chosen_copy_leaves_the_browser_on_its_destination() {
    hosted(
        MODULE,
        "a_chosen_copy_leaves_the_browser_on_its_destination",
        |mut host| {
            let temp = tempfile::tempdir().expect("a folder");
            let (dest, sub) = (temp.path().join("dest"), temp.path().join("sub"));
            for folder in [&dest, &sub] {
                std::fs::create_dir(folder).expect("a folder");
            }
            std::fs::write(temp.path().join("top.txt"), "x\n").expect("a file");
            std::fs::write(sub.join("inner.txt"), "x\n").expect("a file");
            open_listing(&mut host, temp.path(), "top.txt");
            copy_through_chooser(&mut host, (0, &temp.path().join("top.txt")), &dest);
            assert_eq!(
                host.app().browser().active_location(),
                Some(Location::local(&dest)),
                "a folder the shown one holds"
            );
            open_listing(&mut host, temp.path(), "sub");
            host.app().browser().descend(0, Location::local(&sub));
            let opened = |host: &Offscreen| {
                host.app().browser().active_location() == Some(Location::local(&sub))
            };
            assert!(
                host.run_until(PATIENCE, opened),
                "the subfolder's column opens"
            );
            idle(&mut host);
            copy_through_chooser(&mut host, (1, &sub.join("inner.txt")), temp.path());
            assert_eq!(
                host.app().browser().active_location(),
                Some(Location::local(temp.path())),
                "the open ancestor column"
            );
        },
    );
}

#[test]
fn only_existing_folders_are_destinations() {
    let temp = tempfile::tempdir().expect("a folder");
    let file = temp.path().join("file.txt");
    std::fs::write(&file, "x\n").expect("a file");
    let refused = Err("Choose an existing folder.".to_owned());
    for (path, expected) in [
        (temp.path().to_path_buf(), Ok(temp.path().to_path_buf())),
        (temp.path().join("missing"), refused.clone()),
        (file, refused),
    ] {
        assert_eq!(run_async(existing_folder(&path)), expected, "{path:?}");
    }
}

#[test]
fn a_move_leaves_out_what_cannot_be_removed_from_where_it_is() {
    let local = Location::local("/tmp/a");
    let inside_trashed = Location::uri("trash:///folder/inner");
    for (sources, move_sources, allowed) in [
        (vec![], false, false),
        (vec![local.clone()], false, true),
        (vec![local.clone()], true, true),
        (vec![local.clone(), inside_trashed.clone()], false, true),
        (vec![local, inside_trashed], true, false),
    ] {
        assert_eq!(
            transferable(&sources, move_sources),
            allowed,
            "{sources:?} move={move_sources}"
        );
    }
}
