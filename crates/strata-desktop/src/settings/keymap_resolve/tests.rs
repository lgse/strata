// SPDX-License-Identifier: MIT

//! The GTK application's preference tests hold this through GDK's key library
//! (`src/ui/preferences/tests/keymap.rs`). These hold it through the Rune interface's,
//! `keys::Xkb`, which they install in a process of their own.

use std::path::Path;

use super::resolve_keymap;
use crate::{
    keys::{
        Binding, Command, Key, Map, Xkb,
        bindings::{COPY, MOVE_TO_TRASH, RENAME},
        install,
    },
    settings::KeymapOverrides,
};

fn every_binding_fires(_: Map, _: Command, _: Binding, _: bool) -> bool {
    false
}

fn with_xkb(test: &str, run: impl FnOnce()) {
    crate::test_support::in_own_process(&format!("settings::keymap_resolve::tests::{test}"), || {
        assert!(install(&Xkb, every_binding_fires));
        run();
    });
}

fn overrides(file: &str) -> KeymapOverrides {
    toml::from_str(file).expect("keymap overrides")
}

const SAVED: &str = r#"
[default]
rename = ["<Control>e", "F2"]
move_to_trash = []
copy = ["<Control><Frobnicate>c"]
go.home = ["<Alt>Home"]
no_such_command = ["<Control>q"]
select_all = "<Control>a"
cut = ["X"]

[tenxer]
rename = ["<Shift>r"]
"#;

#[test]
fn a_saved_keymap_resolves_through_the_xkbcommon_backend() {
    with_xkb("a_saved_keymap_resolves_through_the_xkbcommon_backend", || {
        let keymap = resolve_keymap(&overrides(SAVED), None);
        assert_eq!(
            keymap.bindings(Map::Default, RENAME),
            [Binding::ctrl(Key::e), Binding::plain(Key::F2)]
        );
        assert_eq!(keymap.bindings(Map::Tenxer, RENAME), [Binding::shift(Key::r)]);
        assert_eq!(keymap.bindings(Map::Default, MOVE_TO_TRASH), []);
        // An accelerator it cannot read keeps the command's shipped keys.
        assert_eq!(
            keymap.bindings(Map::Default, COPY),
            COPY.defaults(Map::Default)
        );
        assert!(!keymap.overrides(Map::Tenxer, MOVE_TO_TRASH));
    });
}

#[test]
fn what_cannot_apply_is_logged_against_the_file_only_when_one_is_given() {
    with_xkb(
        "what_cannot_apply_is_logged_against_the_file_only_when_one_is_given",
        || {
            let saved = overrides(SAVED);
            let file = Path::new("/settings/settings.toml");
            let logged = strata_test_support::capture_logs(|| {
                resolve_keymap(&saved, Some(file));
            });
            let warnings = [
                "keymap entry has an accelerator Strata cannot read",
                "keymap entry is a table",
                "keymap entry names no command",
                "keymap entry is not a list of accelerators",
                "keymap accelerator names an uppercase letter without <Shift>",
            ];
            for warning in warnings {
                let line = strata_test_support::captured_event(&logged, warning);
                assert!(line.contains("/settings/settings.toml"), "{line}");
            }
            assert_eq!(logged.lines().count(), warnings.len(), "{logged}");
            let silent = strata_test_support::capture_logs(|| {
                resolve_keymap(&saved, None);
            });
            assert_eq!(silent, "");
        },
    );
}
