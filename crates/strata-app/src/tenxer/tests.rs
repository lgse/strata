// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use crate::{
    App,
    commands::{Origin, XKey},
    host::offscreen::Offscreen,
    preview, shell,
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
    open_names_and_focus(host, &["alpha.txt", "beta.txt", "gamma.txt"])
}

fn open_names_and_focus(host: &mut Offscreen, names: &[&str]) -> tempfile::TempDir {
    let folder = folder_of(names);
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

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        cmd: true,
        ..Mods::default()
    }
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
            host.set_key_origin(Some(Origin::from_x(XKey {
                keycode: 44,
                base: Some('о'),
            })));
            tap(&mut host, char_key("о"), Mods::default());
            host.set_key_origin(None);

            let cursor = host.app().browser().focused_item().map(|(_, pos, _)| pos);
            assert_eq!(cursor, Some(1));
            assert_eq!(host.app().browser().selected_positions(0), vec![0]);
        },
    );
}

#[test]
fn ctrl_d_ctrl_f_ctrl_b_and_ctrl_r_run_tenxer_collection_commands() {
    hosted(
        MODULE,
        "ctrl_d_ctrl_f_ctrl_b_and_ctrl_r_run_tenxer_collection_commands",
        |mut host| {
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| s.tenxer_mode = true);
            });
            let names: Vec<String> = (0..40).map(|i| format!("item-{i:02}.txt")).collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let _folder = open_names_and_focus(&mut host, &refs);
            host.app().browser().select(0, 0);
            while host.turn() {}

            tap(&mut host, Key::Command('d'), ctrl());
            let half = host
                .app()
                .browser()
                .focused_item()
                .map(|(_, pos, _)| pos)
                .expect("cursor");
            assert!(half > 0 && half < 39, "Ctrl+D moves half a page: {half}");
            assert_eq!(host.app().browser().selected_positions(0), vec![0]);

            tap(&mut host, Key::Command('f'), ctrl());
            let full = host
                .app()
                .browser()
                .focused_item()
                .map(|(_, pos, _)| pos)
                .expect("cursor");
            assert!(full > half, "Ctrl+F pages down: {half} -> {full}");
            assert!(crate::filter::tenxer_mark(host.app()).is_none());

            let prefs_before = shell::preferences(host.app()).clone();
            tap(&mut host, Key::Command('b'), ctrl());
            let back = host
                .app()
                .browser()
                .focused_item()
                .map(|(_, pos, _)| pos)
                .expect("cursor");
            assert!(back < full, "Ctrl+B pages up: {full} -> {back}");
            assert_eq!(shell::preferences(host.app()), &prefs_before);

            tap(&mut host, Key::Command('r'), ctrl());
            let expected: Vec<usize> = (1..40).collect();
            assert_eq!(host.app().browser().selected_positions(0), expected);
            assert!(!host.app().rename.state.is_open());
        },
    );
}

#[test]
fn l_and_right_on_file_keep_open_preview_instead_of_closing_it() {
    hosted(
        MODULE,
        "l_and_right_on_file_keep_open_preview_instead_of_closing_it",
        |mut host| {
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| {
                    s.tenxer_mode = true;
                    s.single_click_previews = false;
                });
            });
            let _folder = open_and_focus(&mut host);
            host.app().browser().select(0, 0);
            while host.turn() {}

            tap(&mut host, char_key("l"), Mods::default());
            assert!(preview::panel_pressed(host.app()));
            assert!(has_key(&host, "preview-close"));

            // Pressing `l` or `Right` again on a file keeps the preview drawer open.
            tap(&mut host, char_key("l"), Mods::default());
            assert!(has_key(&host, "preview-close"));
            tap(&mut host, Key::Right, Mods::default());
            assert!(has_key(&host, "preview-close"));
        },
    );
}
