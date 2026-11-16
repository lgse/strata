// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods, PointerButton};
use serde_json::Value;
use strata_components::scroll::autoscroll::SHIELD;
use strata_core::model::Location;

use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{PATIENCE, send_key, view::hosted_in},
};

const MODULE: &str = "pointer::tests";

fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

fn node<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    if tree["key"] == key {
        return Some(tree);
    }
    let children = tree["children"].as_array().map_or(&[][..], Vec::as_slice);
    children.iter().find_map(|child| node(child, key))
}

fn visible_rect(host: &mut Offscreen, key: &str) -> [f32; 4] {
    host.runtime()
        .elements()
        .into_iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("{key} is mounted"))
        .visible
}

fn make_many_files(count: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..count {
        std::fs::write(dir.path().join(format!("{i:03}.txt")), b"line\n").expect("write");
    }
    dir
}

fn open_and_wait(host: &mut Offscreen, dir: &tempfile::TempDir, count: usize) {
    let location = Location::local(dir.path());
    host.app().open(location.clone());
    let listed = host.run_until(PATIENCE, |host| {
        host.app().browser().active_location().as_ref() == Some(&location)
            && host.app().list.source().len() == count
    });
    assert!(listed, "listed {count} entries");
    idle(host);
}

#[test]
fn middle_click_on_scrollable_background_starts_autoscroll_shield_and_escape_stops_it() {
    let name = "middle_click_on_scrollable_background_starts_autoscroll_shield_and_escape_stops_it";
    hosted_in(MODULE, "list", name, |mut host| {
        let dir = make_many_files(80);
        open_and_wait(&mut host, &dir, 80);

        let vp = host.app().list.single.pane.collection().viewport();
        let (ax, ay) = (vp.x + 2.0, vp.y + vp.h / 2.0);
        host.runtime().pointer_move(ax, ay);
        host.runtime().button_down(ax, ay, PointerButton::Middle);
        host.turn();
        host.runtime().button_up(ax, ay, PointerButton::Middle);
        idle(&mut host);

        assert!(host.app().list.autoscroll().active(), "autoscroll started");
        let tree = host.tree();
        let shield = node(&tree, SHIELD).expect("shield mounted");
        assert_eq!(shield["name"], "Autoscroll");

        host.runtime().pointer_move(ax, ay + 120.0);
        for _ in 0..10 {
            host.turn();
            host.runtime().frame(1.0 / 60.0);
        }
        let scrolled = host.app().list.single.pane.collection().scroll_offset();
        assert!(scrolled > 0.0, "autoscroll moved the viewport: {scrolled}");

        send_key(
            &mut host,
            KeyEvent {
                key: Key::Escape,
                mods: Mods::default(),
            },
        );
        idle(&mut host);
        assert!(!host.app().list.autoscroll().active(), "Escape stopped it");
        assert!(node(&host.tree(), SHIELD).is_none(), "shield unmounted");
    });
}

#[test]
fn sidebar_blank_drag_bands_scrolled_list_without_resetting_scroll() {
    let name = "sidebar_blank_drag_bands_scrolled_list_without_resetting_scroll";
    hosted_in(MODULE, "list", name, |mut host| {
        let dir = make_many_files(80);
        open_and_wait(&mut host, &dir, 80);

        host.runtime().update(|_: &mut App, tx| {
            tx.scroll_to_instant("list/viewport", 0.0, 260.0);
            tx.focus("sidebar-place-home", true);
        });
        idle(&mut host);
        let before = host.app().list.single.pane.collection().scroll_offset();
        assert!(
            (before - 260.0).abs() < 1.0,
            "scrolled before drag: {before}"
        );

        let [sx, sy, sw, sh] = visible_rect(&mut host, "sidebar-scroll");
        let vp = host.app().list.single.pane.collection().viewport();
        let start = (sx + sw / 2.0, (sy + sh - 30.0).min(vp.y + vp.h - 40.0));
        let end = (vp.x + 80.0, start.1 - 52.0);

        host.runtime().pointer_move(start.0, start.1);
        host.runtime().pointer_down(start.0, start.1);
        host.turn();
        for step in 1..=8 {
            let k = step as f32 / 8.0;
            host.runtime().pointer_move(
                start.0 + (end.0 - start.0) * k,
                start.1 + (end.1 - start.1) * k,
            );
            host.turn();
        }
        host.runtime().pointer_up(end.0, end.1);
        idle(&mut host);

        let after = host.app().list.single.pane.collection().scroll_offset();
        assert!(
            (after - before).abs() < 1.0,
            "sidebar marquee kept scroll offset ({before} -> {after})"
        );
        let selected: Vec<String> = host
            .app()
            .browser()
            .selected_entries()
            .into_iter()
            .map(|entry| entry.display_name)
            .collect();
        assert!(
            selected.len() >= 2 && selected.iter().all(|name| name.as_str() >= "010.txt"),
            "selected only scrolled rows: {selected:?}"
        );
    });
}
