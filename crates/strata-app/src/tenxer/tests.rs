// SPDX-License-Identifier: MIT

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;

use crate::{
    App,
    commands::{Origin, XKey},
    host::offscreen::Offscreen,
    preview, shell,
    test_support::{
        PATIENCE, folder_of, hosted, hosted_after, send_key,
        view::{all, text},
    },
};

const MODULE: &str = "tenxer::tests";

pub(super) fn tap(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

pub(super) fn char_key(ch: &str) -> Key {
    Key::Text(ch.to_owned())
}

pub(super) fn has_key(host: &Offscreen, key: &str) -> bool {
    let tree = host.tree();
    !all(&tree, &|node| node["key"] == key).is_empty()
}

pub(super) fn node_name(host: &Offscreen, key: &str) -> Option<String> {
    let tree = host.tree();
    all(&tree, &|node| node["key"] == key)
        .first()
        .map(|node| text(node, "name"))
}

pub(super) fn open_and_focus(host: &mut Offscreen) -> tempfile::TempDir {
    open_names_and_focus(host, &["alpha.txt", "beta.txt", "gamma.txt"])
}

pub(super) fn open_names_and_focus(host: &mut Offscreen, names: &[&str]) -> tempfile::TempDir {
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

#[test]
fn copy_action_and_tab_chords_update_clipboard_dialogs_and_tabs() {
    hosted(
        MODULE,
        "copy_action_and_tab_chords_update_clipboard_dialogs_and_tabs",
        |mut host| {
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| s.tenxer_mode = true);
            });
            let _folder = open_and_focus(&mut host);
            host.app().browser().select(0, 1);
            while host.turn() {}

            // `c n` copies the selected display name; `c c` copies its path.
            tap(&mut host, char_key("c"), Mods::default());
            assert_eq!(node_name(&host, "footer-chord-pill").as_deref(), Some("c-"));
            tap(&mut host, char_key("n"), Mods::default());
            assert_eq!(
                crate::clipboard::fallback_text(host.app()),
                Some("beta.txt")
            );

            // `; e` and `; E` on a plain file both flash "Not an archive"; `; c` opens Compress.
            tap(&mut host, char_key(";"), Mods::default());
            assert_eq!(node_name(&host, "footer-chord-pill").as_deref(), Some(";-"));
            tap(&mut host, char_key("e"), Mods::default());
            assert_eq!(
                node_name(&host, "footer-flash").as_deref(),
                Some("Not an archive")
            );
            tap(&mut host, char_key(";"), Mods::default());
            tap(
                &mut host,
                char_key("E"),
                Mods {
                    shift: true,
                    ..Mods::default()
                },
            );
            assert_eq!(
                node_name(&host, "footer-flash").as_deref(),
                Some("Not an archive")
            );

            // `Shift+Escape` on an armed chord flashes "Unknown chord", whereas plain `Escape` cancels quietly.
            tap(&mut host, char_key("g"), Mods::default());
            tap(
                &mut host,
                Key::Escape,
                Mods {
                    shift: true,
                    ..Mods::default()
                },
            );
            assert_eq!(
                node_name(&host, "footer-flash").as_deref(),
                Some("Unknown chord")
            );

            tap(&mut host, char_key(";"), Mods::default());
            tap(&mut host, char_key("c"), Mods::default());
            assert!(has_key(&host, "compress-1-name"));
            tap(&mut host, Key::Escape, Mods::default());

            // `t n` opens a second tab; arming `g-` and switching tabs via `Ctrl+PageUp` cancels it.
            tap(&mut host, char_key("t"), Mods::default());
            tap(&mut host, char_key("n"), Mods::default());
            assert_eq!(shell::tabs::tabs(host.app()).len(), 2);
            tap(&mut host, char_key("g"), Mods::default());
            assert_eq!(node_name(&host, "footer-chord-pill").as_deref(), Some("g-"));
            tap(&mut host, Key::PageUp, ctrl());
            assert!(!has_key(&host, "footer-chord-pill"));
        },
    );
}

#[test]
fn go_chord_toggles_sidebar_keycaps_and_letter_bookmarks_save_and_jump() {
    hosted(
        MODULE,
        "go_chord_toggles_sidebar_keycaps_and_letter_bookmarks_save_and_jump",
        |mut host| {
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| s.tenxer_mode = true);
            });
            let first = open_and_focus(&mut host);
            let second = folder_of(&["other.txt"]);

            // Arming `g` turns sidebar keycaps on; Escape turns them back off.
            tap(&mut host, char_key("g"), Mods::default());
            assert!(host.app().sidebar.is_started());
            tap(&mut host, Key::Escape, Mods::default());
            assert!(!has_key(&host, "footer-chord-pill"));

            // `m a` saves the listed folder; navigating away and pressing `' a` returns there.
            tap(&mut host, char_key("m"), Mods::default());
            assert_eq!(node_name(&host, "footer-chord-pill").as_deref(), Some("m-"));
            tap(&mut host, char_key("a"), Mods::default());
            assert!(host.run_until(PATIENCE, |h| {
                node_name(h, "footer-flash").as_deref() == Some("Saved bookmark a")
            }));

            host.app().open(Location::local(second.path()));
            assert!(host.run_until(PATIENCE, |h| {
                h.app().browser().active_location() == Some(Location::local(second.path()))
            }));
            while host.turn() {}

            tap(&mut host, char_key("'"), Mods::default());
            assert_eq!(node_name(&host, "footer-chord-pill").as_deref(), Some("'-"));
            tap(&mut host, char_key("a"), Mods::default());
            assert!(host.run_until(PATIENCE, |h| {
                h.app().browser().active_location() == Some(Location::local(first.path()))
            }));
        },
    );
}

#[test]
fn tenxer_sidebar_keyboard_activation_selects_nothing_and_returns_focus_to_view() {
    hosted(
        MODULE,
        "tenxer_sidebar_keyboard_activation_selects_nothing_and_returns_focus_to_view",
        |mut host| {
            let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("test home"));
            for name in ["a.txt", "b.txt"] {
                std::fs::write(home.join(name), name).expect("a file at home");
            }
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| s.tenxer_mode = true);
            });
            let _folder = open_and_focus(&mut host);
            assert!(host.run_until(PATIENCE, |h| has_key(h, "sidebar-place-home")));

            host.runtime().update(|_, tx| {
                tx.focus("sidebar-place-home", true);
            });
            while host.turn() {}

            tap(&mut host, char_key("l"), Mods::default());
            assert!(host.run_until(PATIENCE, |h| {
                h.app().browser().active_location() == Some(Location::local(&home))
                    && h.app().browser().column_entry_counts(0).is_some()
            }));
            while host.turn() {}
            assert!(host.app().browser().selected_positions(0).is_empty());
            let tree = host.tree();
            let focused_view = !all(&tree, &|node| {
                text(node, "key").starts_with(crate::columns::VIEW)
                    && node["states"]
                        .as_array()
                        .is_some_and(|states| states.iter().any(|state| state == "focused"))
            })
            .is_empty();
            assert!(focused_view, "focus returned to the Columns view");
        },
    );
}

#[test]
fn sidebar_routes_russian_l_and_unclaimed_keys_to_escape_and_listing() {
    hosted(
        MODULE,
        "sidebar_routes_russian_l_and_unclaimed_keys_to_escape_and_listing",
        |mut host| {
            let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("test home"));
            std::fs::write(home.join("a.txt"), "a").expect("a file at home");
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| {
                    s.tenxer_mode = true;
                    s.single_click_previews = false;
                });
            });
            let _folder = open_and_focus(&mut host);
            assert!(host.run_until(PATIENCE, |h| has_key(h, "sidebar-place-home")));

            // Select an item, focus the sidebar, and verify Escape clears the selection while keeping sidebar focus.
            host.app().browser().select(0, 1);
            while host.turn() {}
            host.runtime().update(|_, tx| {
                tx.focus("sidebar-place-home", true);
            });
            while host.turn() {}

            tap(&mut host, Key::Escape, Mods::default());
            assert!(host.app().browser().selected_positions(0).is_empty());

            // Ctrl+B is swallowed by `claims_unbound_command` instead of collapsing the sidebar.
            tap(&mut host, Key::Command('b'), ctrl());
            assert!(shell::preferences(host.app()).sidebar_expanded);

            // Pressing `g` from the sidebar focuses the listing and arms `g-`.
            tap(&mut host, char_key("g"), Mods::default());
            assert_eq!(node_name(&host, "footer-chord-pill").as_deref(), Some("g-"));
            let focused = host
                .runtime()
                .elements()
                .into_iter()
                .find(|e| e.focused)
                .and_then(|e| e.key);
            assert!(
                focused
                    .as_deref()
                    .is_some_and(|key| key.starts_with(crate::columns::VIEW))
            );
            tap(&mut host, Key::Escape, Mods::default());

            // Physical `L` (X keycode 46, Cyrillic 'д' on `ru`) activates the focused sidebar row.
            host.runtime().update(|_, tx| {
                tx.focus("sidebar-place-home", true);
            });
            while host.turn() {}
            host.set_key_origin(Some(Origin::from_x(XKey {
                keycode: 46,
                base: Some('д'),
            })));
            tap(&mut host, char_key("д"), Mods::default());
            host.set_key_origin(None);
            assert!(host.run_until(PATIENCE, |h| {
                h.app().browser().active_location() == Some(Location::local(&home))
            }));
        },
    );
}

#[test]
fn compress_chord_targets_keyboard_focused_column_over_hovered_column() {
    hosted(
        MODULE,
        "compress_chord_targets_keyboard_focused_column_over_hovered_column",
        |mut host| {
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| s.tenxer_mode = true);
            });
            let root = folder_of(&["root.txt"]);
            let child = root.path().join("sub");
            std::fs::create_dir(&child).expect("child dir");
            std::fs::write(child.join("nested.txt"), "nested").expect("nested file");

            host.app().open(Location::local(root.path()));
            assert!(host.run_until(PATIENCE, |h| {
                h.app()
                    .browser()
                    .column_entry_counts(0)
                    .is_some_and(|counts| counts.total == 2)
            }));
            while host.turn() {}

            // Focus depth 0 (`sub` is sorted before `root.txt`), descend into `sub` at depth 1,
            // and hold the header target on depth 0 to simulate a hovered parent column.
            host.runtime().update(|app, tx| {
                let view = shell::view_focus(app).expect("a view").to_owned();
                tx.focus(view, true);
            });
            while host.turn() {}
            tap(&mut host, char_key("l"), Mods::default());
            assert!(host.run_until(PATIENCE, |h| {
                h.app()
                    .browser()
                    .column_entry_counts(1)
                    .is_some_and(|counts| counts.total == 1)
            }));
            while host.turn() {}
            host.runtime().update(|app, _| {
                app.columns.hold_header(Some(0));
            });
            while host.turn() {}
            assert_eq!(host.app().columns.targeted(), 0);
            assert_eq!(host.app().columns.active_depth(), 1);

            tap(&mut host, char_key(";"), Mods::default());
            tap(&mut host, char_key("c"), Mods::default());
            let tree = host.tree();
            let field = all(&tree, &|node| node["key"] == "compress-1-name");
            assert_eq!(field.len(), 1);
            assert_eq!(text(field[0], "value"), "nested.txt");
        },
    );
}

/// The chrome 10xer hides rebuilds the view around the listing; the listing keeps the keyboard
/// through the switch, as GTK's does.
#[test]
fn toggling_tenxer_mode_keeps_the_keyboard_in_the_listing() {
    crate::test_support::hosted_after(
        MODULE,
        "toggling_tenxer_mode_keeps_the_keyboard_in_the_listing",
        || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            std::fs::write(file, "browser_mode = \"icons\"\n").expect("settings");
        },
        |mut host| {
            let _folder = open_and_focus(&mut host);
            let focus = |host: &mut Offscreen| {
                let mut focused = None;
                host.runtime().update(|_: &mut App, tx: &mut Tx| {
                    focused = tx.focused().map(str::to_owned);
                });
                focused
            };
            let toggle = Mods {
                ctrl: true,
                cmd: true,
                shift: true,
                ..Mods::default()
            };
            for enabled in [true, false] {
                tap(&mut host, Key::Command('m'), toggle);
                assert_eq!(shell::preferences(host.app()).tenxer_mode, enabled);
                assert_eq!(focus(&mut host).as_deref(), Some("icons"));
            }
        },
    );
}

/// `;` lists the actions by number; its digit runs the one listed, unless the selection moved
/// since it armed ("Selection changed") or the slot is empty (`complete_numbered_action`).
#[test]
fn numbered_action_chords_run_the_listed_action_on_the_armed_targets() {
    let ran = || strata_core::storage::config_directory().join("ran.txt");
    hosted_after(
        MODULE,
        "numbered_action_chords_run_the_listed_action_on_the_armed_targets",
        move || {
            let dir = strata_core::storage::config_directory()
                .join("actions")
                .join("record");
            std::fs::create_dir_all(&dir).expect("action folder");
            let body = format!(
                "schema_version = 1\nid = \"record\"\nname = \"Record\"\nmenu = \"top\"\n\n\
                 [when]\nextensions = [\"txt\"]\n\n[run]\nruntime = \"command\"\n\
                 program = \"/bin/sh\"\n\
                 args = [\"-c\", \"printf '%s\\\\n' \\\"$@\\\" >> '{}'\", \"sh\", \"{{paths}}\"]\n",
                ran().display()
            );
            std::fs::write(dir.join("action.toml"), body).expect("action.toml");
        },
        move |mut host| {
            host.runtime().update(|app, _| {
                shell::saved(app).change(|s| s.tenxer_mode = true);
            });
            let folder = open_and_focus(&mut host);
            let flash = |host: &Offscreen| node_name(host, "footer-flash");
            host.app().browser().select(0, 0);
            while host.turn() {}

            tap(&mut host, char_key(";"), Mods::default());
            host.app().browser().select(0, 1);
            while host.turn() {}
            tap(&mut host, char_key("1"), Mods::default());
            assert_eq!(flash(&host).as_deref(), Some("Selection changed"));

            tap(&mut host, char_key(";"), Mods::default());
            tap(&mut host, char_key("2"), Mods::default());
            assert_eq!(flash(&host).as_deref(), Some("No action 2"));
            assert!(!ran().exists(), "no action ran yet");

            tap(&mut host, char_key(";"), Mods::default());
            tap(&mut host, char_key("1"), Mods::default());
            assert!(host.run_until(PATIENCE, |_| ran().exists()), "slot 1 ran");
            let beta = folder.path().join("beta.txt");
            let recorded = std::fs::read_to_string(ran()).expect("recorded");
            assert_eq!(recorded, format!("{}\n", beta.display()));
        },
    );
}

/// A folder holding `pictures` (`diagram.txt` and `photo.txt`) and `readme.md`, opened with
/// the keyboard in its listing.
fn open_pictures(host: &mut Offscreen) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("folder");
    let pictures = root.path().join("pictures");
    std::fs::create_dir(&pictures).expect("folder");
    for name in ["diagram.txt", "photo.txt"] {
        std::fs::write(pictures.join(name), name).expect("file");
    }
    std::fs::write(root.path().join("readme.md"), "readme").expect("file");
    host.app().open(Location::local(root.path()));
    assert!(host.run_until(PATIENCE, |h| {
        h.app().browser().column_entry_counts(0).is_some()
    }));
    while host.turn() {}
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let view = shell::view_focus(app).expect("a view").to_owned();
        tx.focus(view, true);
    });
    while host.turn() {}
    root
}

/// A folder came back to with its first entry selected and the cursor past it: the browser's
/// load picked that first entry, so it read the restored selection as its own pick echoed and
/// kept the load's cursor. The view showed the remembered one, and Return or Delete would act
/// on another entry than the one drawn. Both now hold the remembered cursor.
#[test]
fn a_revisit_keeps_the_view_and_the_browser_on_one_cursor() {
    hosted_after(
        MODULE,
        "a_revisit_keeps_the_view_and_the_browser_on_one_cursor",
        || {
            let file = strata_core::storage::config_directory().join("settings.toml");
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
            let saved = "browser_mode = \"list\"\ntenxer_mode = true\ntype_to_search = false\n";
            std::fs::write(file, saved).expect("settings");
        },
        |mut host| {
            let _root = open_pictures(&mut host);
            let named = |host: &Offscreen| {
                host.app()
                    .browser()
                    .focused_item()
                    .map(|(_, _, entry)| entry.display_name)
            };
            let entered = |name: &'static str| {
                move |h: &Offscreen| {
                    h.app()
                        .browser()
                        .active_location()
                        .and_then(|l| l.file_name())
                        == Some(std::ffi::OsString::from(name))
                }
            };
            assert_eq!(named(&host).as_deref(), Some("pictures"));
            tap(&mut host, char_key("l"), Mods::default());
            assert!(host.run_until(PATIENCE, entered("pictures")));
            while host.turn() {}
            tap(&mut host, char_key(" "), Mods::default());
            assert_eq!(named(&host).as_deref(), Some("photo.txt"), "Space moved on");

            tap(&mut host, char_key("h"), Mods::default());
            assert!(host.run_until(PATIENCE, |h| named(h).as_deref() == Some("pictures")));
            tap(&mut host, char_key("l"), Mods::default());
            assert!(host.run_until(PATIENCE, entered("pictures")));
            while host.turn() {}

            use crate::single::SinglePane;
            let app = host.app();
            let view = crate::list::List::single(app)
                .pane
                .collection()
                .selection
                .cursor();
            let browser = app
                .browser()
                .focused_item()
                .map(|(_, _, entry)| shell::id_of(app, &entry.location));
            assert_eq!(view, browser, "the view's cursor is the browser's");
            assert_eq!(
                named(&host).as_deref(),
                Some("photo.txt"),
                "the remembered one"
            );
        },
    );
}
