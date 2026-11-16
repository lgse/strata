// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use crate::{
    App,
    commands::Origin,
    host::offscreen::Offscreen,
    shell,
    test_support::{
        PATIENCE, folder_of, hosted, send_key,
        view::{all, text},
    },
};

const MODULE: &str = "tenxer::tests";

fn tap(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn char_key(ch: &str) -> Key {
    Key::Text(ch.to_owned())
}

fn has_key(host: &Offscreen, key: &str) -> bool {
    let tree = host.tree();
    !all(&tree, &|node| node["key"] == key).is_empty()
}

fn node_name(host: &Offscreen, key: &str) -> Option<String> {
    let tree = host.tree();
    all(&tree, &|node| node["key"] == key)
        .first()
        .map(|node| text(node, "name"))
}

fn open_and_focus(host: &mut Offscreen) -> tempfile::TempDir {
    let folder = folder_of(&["alpha.txt", "beta.txt", "gamma.txt"]);
    host.app().open(Location::local(folder.path()));
    assert!(host.run_until(PATIENCE, |h| {
        h.app().browser().column_entry_counts(0).is_some()
    }));
    while host.turn() {}
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let view = shell::view_focus(app).expect("a view").to_owned();
        tx.focus(view, true);
    });
    while host.turn() {}
    folder
}

#[test]
fn ctrl_shift_m_toggles_chrome_footer_and_clears_session_on_exit() {
    hosted(
        MODULE,
        "ctrl_shift_m_toggles_chrome_footer_and_clears_session_on_exit",
        |mut host| {
            let _folder = open_and_focus(&mut host);
            assert!(has_key(&host, "header-search"));
            assert!(!has_key(&host, "footer-tenxer"));

            let toggle_mods = Mods {
                ctrl: true,
                cmd: true,
                shift: true,
                ..Mods::default()
            };
            tap(&mut host, Key::Command('m'), toggle_mods);
            assert!(shell::preferences(host.app()).tenxer_mode);
            assert!(!has_key(&host, "header-search"));
            assert!(has_key(&host, "footer-tenxer"));

            // Select the first row, start a VISUAL range, and arm `g-`.
            host.app().browser().select(0, 0);
            while host.turn() {}
            tap(&mut host, char_key("v"), Mods::default());
            assert_eq!(
                node_name(&host, "footer-range").as_deref(),
                Some("Visual select")
            );
            tap(&mut host, char_key("g"), Mods::default());
            assert_eq!(node_name(&host, "footer-chord-pill").as_deref(), Some("g-"));

            // Leaving 10xer mode restores chrome and clears both the chord and the range.
            tap(&mut host, Key::Command('m'), toggle_mods);
            assert!(!shell::preferences(host.app()).tenxer_mode);
            assert!(has_key(&host, "header-search"));
            assert!(!has_key(&host, "footer-tenxer"));
            assert!(!has_key(&host, "footer-chord-pill"));
            assert!(!has_key(&host, "footer-range"));
        },
    );
}

#[test]
fn gg_jumps_to_first_preserving_selection_and_unknown_chord_flashes() {
    hosted(
        MODULE,
        "gg_jumps_to_first_preserving_selection_and_unknown_chord_flashes",
        |mut host| {
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| s.tenxer_mode = true);
            });
            let _folder = open_and_focus(&mut host);
            host.app().browser().select(0, 2);
            while host.turn() {}

            // `g` then `z` is unknown: cancels chord and flashes "Unknown chord".
            tap(&mut host, char_key("g"), Mods::default());
            assert_eq!(node_name(&host, "footer-chord-pill").as_deref(), Some("g-"));
            tap(&mut host, char_key("z"), Mods::default());
            assert!(!has_key(&host, "footer-chord-pill"));
            assert_eq!(
                node_name(&host, "footer-flash").as_deref(),
                Some("Unknown chord")
            );

            // `g g` moves the cursor to 0 while keeping index 2 selected.
            tap(&mut host, char_key("g"), Mods::default());
            tap(&mut host, char_key("g"), Mods::default());
            let cursor = host.app().browser().focused_item().map(|(_, pos, _)| pos);
            assert_eq!(cursor, Some(0));
            assert_eq!(host.app().browser().selected_positions(0), vec![2]);
        },
    );
}

#[test]
fn russian_layout_origin_moves_cursor_without_replacing_selection() {
    hosted(
        MODULE,
        "russian_layout_origin_moves_cursor_without_replacing_selection",
        |mut host| {
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| s.tenxer_mode = true);
            });
            let _folder = open_and_focus(&mut host);
            host.app().browser().select(0, 0);
            while host.turn() {}

            // X keycode 44 is physical J, which types Cyrillic 'о' unshifted on `ru`.
            host.set_key_origin(Some(Origin::from_x_keycode(44, Some('о'))));
            tap(&mut host, char_key("о"), Mods::default());
            host.set_key_origin(None);

            let cursor = host.app().browser().focused_item().map(|(_, pos, _)| pos);
            assert_eq!(cursor, Some(1));
            assert_eq!(host.app().browser().selected_positions(0), vec![0]);
        },
    );
}
