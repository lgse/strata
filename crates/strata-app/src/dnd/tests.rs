// SPDX-License-Identifier: MIT

use std::path::Path;

use rune::{ElementInfo, Key, KeyEvent, Mods};
use strata_components::{
    contract::{
        EntryId,
        dnd::{DragPayload, DropActions, DropData, DropOffer, Dropped, URI_LIST, file_uri},
    },
    tabs::{TabId, tab_key},
};
use strata_core::model::Location;

use crate::{
    commands::{On, Outcome, run},
    host::offscreen::Offscreen,
    shell::tabs::{shown_tab, tab_browser, tabs},
    slots::TabKey,
    test_support::{PATIENCE, fixture, hosted, send_key, view::idle},
};

const MODULE: &str = "dnd::tests";

fn open_listed(host: &mut Offscreen, folder: &Path) {
    host.app().open(Location::local(folder));
    assert!(
        host.run_until(PATIENCE, |h| h
            .app()
            .browser()
            .column_entry_counts(0)
            .is_some()),
        "listing ready"
    );
    idle(host);
}

/// The visible rectangle of the first live element `wanted` picks; `what` names it on failure.
fn visible(host: &mut Offscreen, what: &str, wanted: impl Fn(&ElementInfo) -> bool) -> [f32; 4] {
    host.runtime()
        .elements()
        .into_iter()
        .find(|el| !el.exiting && wanted(el))
        .unwrap_or_else(|| panic!("missing element {what}"))
        .visible
}

fn label_centre(host: &mut Offscreen, text: &str) -> (f32, f32) {
    let [x, y, w, h] = visible(host, &format!("with text `{text}`"), |el| {
        el.text.as_deref() == Some(text)
    });
    (x + w.min(24.0) / 2.0, y + h / 2.0)
}

fn key_centre(host: &mut Offscreen, key: &str) -> (f32, f32) {
    let [x, y, w, h] = visible(host, &format!("`{key}`"), |el| {
        el.key.as_deref() == Some(key)
    });
    (x + w / 2.0, y + h / 2.0)
}

/// Escape during a held drag, then the release at `at`.
fn escape_and_release(host: &mut Offscreen, at: (f32, f32)) {
    send_key(
        host,
        KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        },
    );
    host.turn();
    host.runtime().pointer_up(at.0, at.1);
    host.turn();
    idle(host);
}

fn drag_steps(host: &mut Offscreen, from: (f32, f32), to: (f32, f32), release: bool) {
    host.runtime().pointer_move(from.0, from.1);
    host.turn();
    host.runtime().pointer_down(from.0, from.1);
    host.turn();
    for step in 1..=6 {
        let t = step as f32 / 6.0;
        let x = from.0 + (to.0 - from.0) * t;
        let y = from.1 + (to.1 - from.1) * t;
        host.runtime().pointer_move(x, y);
        host.turn();
    }
    if release {
        host.runtime().pointer_up(to.0, to.1);
        host.turn();
    }
    idle(host);
}

fn wait_path(host: &mut Offscreen, path: &Path, exists: bool) {
    assert!(
        host.run_until(PATIENCE, |_| path.exists() == exists),
        "expected {} exists={exists}",
        path.display()
    );
    idle(host);
}

#[test]
fn columns_row_drag_moves_by_default_copies_with_ctrl_and_cancels_on_escape() {
    hosted(
        MODULE,
        "columns_row_drag_moves_by_default_copies_with_ctrl_and_cancels_on_escape",
        |mut host| {
            let dir = fixture();
            let root = dir.path();
            open_listed(&mut host, root);

            // Escape cancels an active drag without moving the file or clearing selection.
            let from = label_centre(&mut host, "todo.txt");
            let dest = label_centre(&mut host, "archive");
            drag_steps(&mut host, from, dest, false);
            assert!(super::is_dragging(host.app()));
            escape_and_release(&mut host, dest);
            assert!(!super::is_dragging(host.app()));
            assert!(root.join("todo.txt").exists());
            assert!(!root.join("archive/todo.txt").exists());

            // Ctrl+drag copies and preserves the source file.
            host.modifiers(Mods {
                ctrl: true,
                ..Mods::default()
            });
            let from = label_centre(&mut host, "todo.txt");
            let dest = label_centre(&mut host, "archive");
            drag_steps(&mut host, from, dest, true);
            host.modifiers(Mods::default());
            wait_path(&mut host, &root.join("archive/todo.txt"), true);
            assert!(root.join("todo.txt").exists());

            // Plain drag moves the file into another folder.
            let from = label_centre(&mut host, "todo.txt");
            let dest = label_centre(&mut host, "documents");
            drag_steps(&mut host, from, dest, true);
            wait_path(&mut host, &root.join("documents/todo.txt"), true);
            wait_path(&mut host, &root.join("todo.txt"), false);
        },
    );
}

#[test]
fn hovering_inactive_tab_springs_tab_and_keeps_drag_alive_until_drop() {
    hosted(
        MODULE,
        "hovering_inactive_tab_springs_tab_and_keeps_drag_alive_until_drop",
        |mut host| {
            let dir = fixture();
            let root = dir.path();
            let archive = root.join("archive");
            open_listed(&mut host, root);
            let first: TabKey = shown_tab(host.app());

            host.runtime().update(|app, tx| {
                assert_eq!(run(app, "tab-new", On::Focus, tx), Outcome::Ran);
            });
            idle(&mut host);
            let second = shown_tab(host.app());
            host.app().open(Location::local(&archive));
            assert!(host.run_until(PATIENCE, |h| {
                h.app().browser().active_location() == Some(Location::local(&archive))
            }));
            idle(&mut host);

            let first_centre = key_centre(&mut host, &tab_key("tabs", TabId(first.0)));
            host.runtime().pointer_move(first_centre.0, first_centre.1);
            host.runtime().pointer_down(first_centre.0, first_centre.1);
            host.runtime().pointer_up(first_centre.0, first_centre.1);
            host.turn();
            idle(&mut host);
            assert_eq!(shown_tab(host.app()), first);

            let from = label_centre(&mut host, "todo.txt");
            let second_centre = key_centre(&mut host, &tab_key("tabs", TabId(second.0)));
            for cancelled in [true, false] {
                drag_steps(&mut host, from, second_centre, false);
                assert!(
                    host.run_until(PATIENCE, |h| shown_tab(h.app()) == second),
                    "the hover springs the tab"
                );
                assert!(super::is_dragging(host.app()));
                if !cancelled {
                    break;
                }
                escape_and_release(&mut host, second_centre);
                assert!(!super::is_dragging(host.app()));
                host.runtime().update(move |app, tx| {
                    assert!(crate::tab_strip::select(app, first, tx));
                });
                idle(&mut host);
            }

            // Move into the sprung tab's listing (exercising `keep_alive_drag`) and release.
            host.runtime().pointer_move(400.0, 220.0);
            host.turn();
            host.runtime().pointer_up(400.0, 220.0);
            host.turn();
            idle(&mut host);
            wait_path(&mut host, &archive.join("todo.txt"), true);
            wait_path(&mut host, &root.join("todo.txt"), false);
            assert_eq!(tabs(host.app()).len(), 2);
            assert_eq!(
                tab_browser(host.app(), second).and_then(|b| b.active_location()),
                Some(Location::local(&archive))
            );
        },
    );
}

#[test]
fn dragging_folder_across_inactive_tab_onto_tabs_new_does_not_spring_tab() {
    hosted(
        MODULE,
        "dragging_folder_across_inactive_tab_onto_tabs_new_does_not_spring_tab",
        |mut host| {
            let dir = fixture();
            let root = dir.path();
            open_listed(&mut host, root);
            let first = shown_tab(host.app());

            let from = label_centre(&mut host, "documents");
            let new_btn = key_centre(&mut host, "header-new-tab");
            drag_steps(&mut host, from, new_btn, true);
            assert_eq!(tabs(host.app()).len(), 2);

            let second = shown_tab(host.app());
            host.runtime().update(move |app, tx| {
                assert!(crate::tab_strip::select(app, first, tx));
            });
            idle(&mut host);

            let from_pic = label_centre(&mut host, "pictures");
            drag_steps(&mut host, from_pic, (1156.0, 18.0), true);
            assert_eq!(tabs(host.app()).len(), 3);

            host.runtime().update(move |app, tx| {
                assert!(crate::tab_strip::select(app, second, tx));
            });
            idle(&mut host);
            host.runtime().update(move |app, tx| {
                assert!(crate::tab_strip::select(app, first, tx));
            });
            idle(&mut host);
            assert_eq!(shown_tab(host.app()), first);

            let from = label_centre(&mut host, "archive");
            let tabs_new = key_centre(&mut host, "tabs-new");
            drag_steps(&mut host, from, tabs_new, false);
            host.runtime().frame(0.8);
            host.turn();
            idle(&mut host);
            assert_eq!(
                shown_tab(host.app()),
                first,
                "dragging across an inactive tab to tabs-new must not spring the tab"
            );
            escape_and_release(&mut host, tabs_new);
            assert_eq!(tabs(host.app()).len(), 3);
            assert_eq!(shown_tab(host.app()), first);
        },
    );
}

#[test]
fn dragging_folder_back_over_itself_does_not_spring_open_while_other_folder_does() {
    hosted(
        MODULE,
        "dragging_folder_back_over_itself_does_not_spring_open_while_other_folder_does",
        |mut host| {
            let dir = fixture();
            let root = dir.path();
            open_listed(&mut host, root);

            let docs = label_centre(&mut host, "documents");
            let todo = label_centre(&mut host, "todo.txt");
            let archive = label_centre(&mut host, "archive");
            // Cross the drag threshold over a plain file so a slow tick cannot spring a folder.
            drag_steps(&mut host, docs, todo, false);
            // Return the held drag back over its own folder and dwell past the 750 ms spring delay.
            host.runtime().pointer_move(docs.0, docs.1);
            host.turn();
            host.runtime().frame(0.9);
            host.turn();
            idle(&mut host);
            assert_eq!(
                host.app().browser().active_location(),
                Some(Location::local(root)),
                "returning a dragged folder over itself must not spring-open it"
            );

            // Moving over a different folder still spring-opens it once async descent completes.
            host.runtime().pointer_move(archive.0, archive.1);
            host.turn();
            host.runtime().frame(0.9);
            let wanted = Location::local(root.join("archive"));
            assert!(
                host.run_until(PATIENCE, |h| {
                    h.app().browser().active_location() == Some(wanted.clone())
                }),
                "hovering a different folder must still spring-open it; active = {:?}",
                host.app().browser().active_location()
            );
            idle(&mut host);
            escape_and_release(&mut host, archive);
        },
    );
}

struct NarrowedDrop<'a> {
    source: &'a Path,
    dest: &'a Path,
    local_actions: Option<DropActions>,
    mods: Mods,
}

fn commit_narrowed_drop(host: &mut Offscreen, drop: NarrowedDrop<'_>) {
    let uri = file_uri(drop.source);
    let offer = DropOffer::new(vec![URI_LIST.into()], DropActions::COPY)
        .local(drop.local_actions.is_some())
        .uris(vec![uri.clone()]);
    let mut data = DropData::files(&[drop.source.to_path_buf()]);
    if let Some(actions) = drop.local_actions {
        data = data.local(DragPayload::new(vec![EntryId::new(1)], vec![uri]).actions(actions));
    }
    let dropped = Dropped::new(offer, data, drop.mods);
    let destination = Location::local(drop.dest);
    host.runtime().update(move |app, tx| {
        assert!(super::commit_file_drop(app, destination, &dropped, tx));
    });
    host.turn();
}

#[test]
fn in_app_same_volume_drop_keeps_move_when_offer_is_narrowed_to_copy() {
    hosted(
        MODULE,
        "in_app_same_volume_drop_keeps_move_when_offer_is_narrowed_to_copy",
        |mut host| {
            let dir = fixture();
            let root = dir.path();
            let archive = root.join("archive");
            let documents = root.join("documents");
            open_listed(&mut host, root);

            // In-app drag offering COPY|MOVE whose platform offer was narrowed to COPY still moves.
            let moved = root.join("todo.txt");
            commit_narrowed_drop(
                &mut host,
                NarrowedDrop {
                    source: &moved,
                    dest: &archive,
                    local_actions: Some(DropActions::COPY | DropActions::MOVE),
                    mods: Mods::default(),
                },
            );
            wait_path(&mut host, &archive.join("todo.txt"), true);
            wait_path(&mut host, &moved, false);

            // Ctrl at drop still copies even when the in-app source offered COPY|MOVE.
            let ctrl_copied = root.join("readme.md");
            commit_narrowed_drop(
                &mut host,
                NarrowedDrop {
                    source: &ctrl_copied,
                    dest: &archive,
                    local_actions: Some(DropActions::COPY | DropActions::MOVE),
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                },
            );
            wait_path(&mut host, &archive.join("readme.md"), true);
            assert!(ctrl_copied.exists());

            // An in-app source that offered COPY only still copies.
            commit_narrowed_drop(
                &mut host,
                NarrowedDrop {
                    source: &ctrl_copied,
                    dest: &documents,
                    local_actions: Some(DropActions::COPY),
                    mods: Mods::default(),
                },
            );
            wait_path(&mut host, &documents.join("readme.md"), true);
            assert!(ctrl_copied.exists());

            // An external drop narrowed/limited to COPY still copies.
            let external = archive.join("todo.txt");
            commit_narrowed_drop(
                &mut host,
                NarrowedDrop {
                    source: &external,
                    dest: &documents,
                    local_actions: None,
                    mods: Mods::default(),
                },
            );
            wait_path(&mut host, &documents.join("todo.txt"), true);
            assert!(external.exists());
        },
    );
}
