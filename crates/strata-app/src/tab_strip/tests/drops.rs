// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use rune::{Key, KeyEvent, Mods};
use strata_components::{
    contract::{
        Dnd,
        dnd::{
            DragPayload, DropAction, DropActions, DropData, DropOffer, DropZone, Dropped, URI_LIST,
            file_uri,
        },
        entry::EntryId,
    },
    tabs::{TabId, TabOpen, tab_key},
};
use strata_core::model::Location;

use super::{act, ctrl, located, opened, select};
use crate::{
    App,
    commands::{On, Outcome, run},
    error_dialog::{self, Error},
    host::offscreen::Offscreen,
    shell::tabs::{shown_tab, tabs},
    slots::TabKey,
    tab_strip::{open_in_new_tab, set_dnd, set_open_drag},
    test_support::{PATIENCE, fixture, hosted, send_key, view::idle},
};

const MODULE: &str = "tab_strip::tests::drops";

thread_local! {
    static ZONES: RefCell<HashMap<String, DropZone<App>>> = RefCell::new(HashMap::new());
}

fn recording_dnd() -> Dnd<App> {
    Dnd::custom_keyed(
        |el, key, zone| {
            ZONES.with(|zones| {
                zones.borrow_mut().insert(key.key().to_owned(), zone);
            });
            el
        },
        |el, _, _| el,
    )
}

fn zone_for(key: &str) -> DropZone<App> {
    ZONES.with(|zones| {
        zones
            .borrow()
            .get(key)
            .cloned()
            .unwrap_or_else(|| panic!("missing drop zone `{key}`"))
    })
}

fn centre_of(host: &mut Offscreen, key: &str) -> (f32, f32) {
    let elements = host.runtime().elements();
    let element = elements
        .iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing element `{key}`"));
    let [x, y, w, h] = element.visible;
    (x + w / 2.0, y + h / 2.0)
}

fn tab_centre(host: &mut Offscreen, tab: TabKey) -> (f32, f32) {
    centre_of(host, &tab_key("tabs", TabId(tab.0)))
}

fn drop_on_tab(host: &mut Offscreen, tab: TabKey, paths: &[PathBuf], mods: Mods) {
    let at = tab_centre(host, tab);
    host.modifiers(mods);
    host.runtime().drag_files(paths.to_vec(), Some(at));
    host.turn();
    host.runtime().drop_files(paths.to_vec(), Some(at));
    host.turn();
    host.modifiers(Mods::default());
    idle(host);
}

fn wait_exists(host: &mut Offscreen, path: &Path, exists: bool) {
    let settled = host.run_until(PATIENCE, |_| path.exists() == exists);
    assert!(settled, "expected {} exists={exists}", path.display());
    idle(host);
}

fn local_drag(paths: &[PathBuf], serial: u64) -> (DropOffer, DropData) {
    let uris: Vec<String> = paths.iter().map(|p| file_uri(p)).collect();
    let entries = (1..=uris.len() as u64).map(EntryId::new).collect();
    let payload = DragPayload::new(entries, uris.clone());
    let offer = DropOffer::new(vec![URI_LIST.into()], DropActions::COPY | DropActions::MOVE)
        .local(true)
        .uris(uris)
        .serial(serial);
    let data = DropData::files(paths).local(payload);
    (offer, data)
}

#[test]
fn hovering_an_inactive_tab_selects_it_after_the_delay_while_trash_active_and_leave_do_not() {
    hosted(
        MODULE,
        "hovering_an_inactive_tab_selects_it_after_the_delay_while_trash_active_and_leave_do_not",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            let first = shown_tab(host.app());
            let docs = Location::local(folder.path().join("documents"));
            let trash = Location::uri("trash:///");
            act(&mut host, |app, tx| {
                open_in_new_tab(app, docs.clone(), tx);
                open_in_new_tab(app, trash.clone(), tx);
            });
            let _ = located(&mut host, 3);
            let second = tabs(host.app())[1];
            let trash_tab = tabs(host.app())[2];
            assert!(act(&mut host, |app, tx| select(app, second, tx)));

            let dragged = vec![folder.path().join("todo.txt")];
            for ignored in [second, trash_tab] {
                let at = tab_centre(&mut host, ignored);
                host.runtime().drag_files(dragged.clone(), Some(at));
                host.run_until(Duration::from_millis(550), |_| false);
                assert_eq!(shown_tab(host.app()), second, "stayed on active tab");
            }

            let first_at = tab_centre(&mut host, first);
            host.runtime().drag_files(dragged.clone(), Some(first_at));
            host.run_until(Duration::from_millis(120), |_| false);
            host.runtime().drag_files_to(Some((720.0, 500.0)));
            host.run_until(Duration::from_millis(450), |_| false);
            assert_eq!(shown_tab(host.app()), second, "leaving cancels the spring");

            host.runtime().drag_files(dragged, Some(first_at));
            let sprung = host.run_until(Duration::from_secs(2), |h| shown_tab(h.app()) == first);
            assert!(sprung, "holding over an inactive folder tab selects it");
        },
    );
}

#[test]
fn dropping_files_and_folders_on_a_tab_moves_or_ctrl_copies_and_refuses_same_parent_and_modals() {
    hosted(
        MODULE,
        "dropping_files_and_folders_on_a_tab_moves_or_ctrl_copies_and_refuses_same_parent_and_modals",
        |mut host| {
            let folder = fixture();
            let root = folder.path();
            let dest = root.join("archive");
            opened(&mut host, &dest);
            let dest_tab = shown_tab(host.app());
            act(&mut host, |app, tx| {
                open_in_new_tab(app, Location::local(root), tx);
            });
            let _ = located(&mut host, 2);
            let root_tab = shown_tab(host.app());

            assert!(act(&mut host, |app, tx| select(app, dest_tab, tx)));
            drop_on_tab(
                &mut host,
                root_tab,
                &[root.join("todo.txt")],
                Mods::default(),
            );
            assert_eq!(
                shown_tab(host.app()),
                dest_tab,
                "a same-parent drop is refused without switching tabs"
            );
            assert!(root.join("todo.txt").exists());

            assert!(act(&mut host, |app, tx| select(app, root_tab, tx)));
            act(&mut host, |app, _| {
                error_dialog::show(app, Error::new("Blocked", "Modal open"));
            });
            drop_on_tab(
                &mut host,
                dest_tab,
                &[root.join("todo.txt")],
                Mods::default(),
            );
            assert_eq!(shown_tab(host.app()), root_tab, "modal blocks tab drops");
            assert!(!dest.join("todo.txt").exists());
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Escape,
                    mods: Mods::default(),
                },
            );
            idle(&mut host);

            drop_on_tab(
                &mut host,
                dest_tab,
                &[root.join("todo.txt"), root.join("pictures")],
                Mods::default(),
            );
            assert_eq!(
                shown_tab(host.app()),
                dest_tab,
                "drop selects destination tab"
            );
            wait_exists(&mut host, &dest.join("todo.txt"), true);
            wait_exists(&mut host, &dest.join("pictures/photo.txt"), true);
            assert!(!root.join("todo.txt").exists());
            assert!(!root.join("pictures").exists());

            let undone = act(&mut host, |app, tx| run(app, "undo", On::Focus, tx));
            assert_eq!(undone, Outcome::Ran);
            wait_exists(&mut host, &root.join("todo.txt"), true);
            wait_exists(&mut host, &root.join("pictures/photo.txt"), true);
            assert!(!dest.join("todo.txt").exists());

            assert!(act(&mut host, |app, tx| select(app, root_tab, tx)));
            drop_on_tab(&mut host, dest_tab, &[root.join("readme.md")], ctrl());
            assert_eq!(shown_tab(host.app()), dest_tab);
            wait_exists(&mut host, &dest.join("readme.md"), true);
            assert!(root.join("readme.md").exists(), "Ctrl+drop copies");
        },
    );
}

#[test]
fn return_presses_new_tab_in_the_header_and_in_the_strip() {
    hosted(
        MODULE,
        "return_presses_new_tab_in_the_header_and_in_the_strip",
        |mut host| {
            let folder = fixture();
            opened(&mut host, folder.path());
            assert_eq!(tabs(host.app()).len(), 1);

            act(&mut host, |_, tx| tx.focus("header-new-tab", true));
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Enter,
                    mods: Mods::default(),
                },
            );
            host.turn();
            host.turn();
            idle(&mut host);
            assert_eq!(
                tabs(host.app()).len(),
                2,
                "Return on header New tab opened a tab"
            );
            let _ = located(&mut host, 2);

            act(&mut host, |_, tx| tx.focus("tabs-new", true));
            send_key(
                &mut host,
                KeyEvent {
                    key: Key::Enter,
                    mods: Mods::default(),
                },
            );
            host.turn();
            host.turn();
            idle(&mut host);
            assert_eq!(
                tabs(host.app()).len(),
                3,
                "Return on strip New tab opened a tab"
            );
        },
    );
}

#[test]
fn in_process_sidebar_and_folder_drags_open_tabs_on_header_new_tab_and_strip() {
    hosted(
        MODULE,
        "in_process_sidebar_and_folder_drags_open_tabs_on_header_new_tab_and_strip",
        |mut host| {
            ZONES.with(|zones| zones.borrow_mut().clear());
            let folder = fixture();
            let root = folder.path();
            opened(&mut host, root);
            act(&mut host, |app, _| set_dnd(app, recording_dnd()));

            let docs = root.join("documents");
            let (offer, data) = local_drag(std::slice::from_ref(&docs), 1);
            act(&mut host, |app, _| set_open_drag(app, Some(TabOpen::AtEnd)));
            let header_zone = zone_for("header-new-tab");
            assert_eq!(
                header_zone.choose(host.app(), &offer, Mods::default()),
                Some(DropAction::Copy)
            );
            act(&mut host, |app, tx| {
                header_zone.deliver(app, Dropped::new(offer, data, Mods::default()), tx);
                set_open_drag(app, None);
            });
            assert_eq!(
                located(&mut host, 2),
                [Some(Location::local(root)), Some(Location::local(&docs))]
            );

            let trash = Location::uri("trash:///");
            act(&mut host, |app, tx| open_in_new_tab(app, trash.clone(), tx));
            let _ = located(&mut host, 3);
            let trash_key = tab_key("tabs", TabId(tabs(host.app())[2].0));

            let view_dirs = [root.join("pictures"), root.join("archive")];
            let (offer, data) = local_drag(&view_dirs, 2);
            act(&mut host, |app, _| set_open_drag(app, Some(TabOpen::AtEnd)));
            let trash_zone = zone_for(&trash_key);
            assert_eq!(trash_zone.choose(host.app(), &offer, Mods::default()), None);
            assert!(trash_zone.rejects(), "Trash stops folder-only view drags");

            let strip_zone = zone_for("tabs");
            act(&mut host, |app, tx| {
                strip_zone.deliver(app, Dropped::new(offer, data, Mods::default()), tx);
                set_open_drag(app, None);
            });
            assert_eq!(
                located(&mut host, 5),
                [
                    Some(Location::local(root)),
                    Some(Location::local(&docs)),
                    Some(trash.clone()),
                    Some(Location::local(&view_dirs[0])),
                    Some(Location::local(&view_dirs[1])),
                ]
            );
            assert_eq!(Some(&shown_tab(host.app())), tabs(host.app()).last());

            let pinned = tempfile::tempdir().expect("pinned");
            let (offer, data) = local_drag(&[pinned.path().to_path_buf()], 3);
            act(&mut host, |app, _| set_open_drag(app, Some(TabOpen::AtGap)));
            let first_zone = zone_for(&tab_key("tabs", TabId(tabs(host.app())[0].0)));
            assert_eq!(
                first_zone.choose(host.app(), &offer, Mods::default()),
                Some(DropAction::Copy)
            );
            act(&mut host, |app, tx| {
                first_zone.deliver(app, Dropped::new(offer, data, Mods::default()), tx);
                set_open_drag(app, None);
            });
            assert_eq!(tabs(host.app()).len(), 6);
            assert_eq!(
                located(&mut host, 6)[1],
                Some(Location::local(pinned.path())),
                "sidebar AtGap drag opens after tab 0 when no move gap built"
            );
        },
    );
}
