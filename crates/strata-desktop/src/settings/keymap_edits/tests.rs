// SPDX-License-Identifier: MIT

//! Holds [`KeymapOverrides`] edits and start-up conflict reporting through `keys::Xkb`
//! in a process of their own.

use std::path::Path;

use super::report_keymap_conflicts;
use crate::{
    keys::{
        Binding, Key, Map, Xkb,
        bindings::{COPY, REFRESH, RENAME},
        conflicts::{self, Policy, Severity},
        install,
    },
    settings::{KeymapOverrides, resolve_keymap},
};

fn with_xkb(test: &str, run: impl FnOnce()) {
    crate::test_support::in_own_process(&format!("settings::keymap_edits::tests::{test}"), || {
        assert!(install(&Xkb, conflicts::never_fires));
        run();
    });
}

#[test]
fn edits_normalize_deduplicate_drop_shipped_and_keep_unknown_ids() {
    with_xkb(
        "edits_normalize_deduplicate_drop_shipped_and_keep_unknown_ids",
        || {
            let mut overrides: KeymapOverrides = toml::from_str(
                r#"
[default]
future_cmd = ["<Control>q"]
"#,
            )
            .expect("overrides");
            overrides.set_bindings(
                Map::Default,
                RENAME,
                vec![
                    Binding::ctrl(Key::E),
                    Binding::ctrl(Key::e),
                    Binding::plain(Key::F2),
                ],
            );
            overrides.set_bindings(Map::Default, REFRESH, Vec::new());
            overrides.set_bindings(Map::Tenxer, RENAME, vec![Binding::shift(Key::r)]);

            let resolved = resolve_keymap(&overrides, None);
            assert_eq!(
                resolved.bindings(Map::Default, RENAME),
                [Binding::ctrl(Key::e), Binding::plain(Key::F2)]
            );
            assert_eq!(resolved.bindings(Map::Default, REFRESH), []);
            assert_eq!(
                resolved.bindings(Map::Tenxer, RENAME),
                [Binding::shift(Key::r)]
            );

            // Setting the shipped bindings removes the override entry.
            overrides.set_bindings(Map::Default, RENAME, RENAME.defaults(Map::Default).to_vec());
            assert!(!overrides.default.contains_key(RENAME.id()));

            overrides.reset_binding(Map::Tenxer, RENAME);
            assert!(overrides.tenxer.is_empty());

            overrides.reset_map(Map::Default);
            assert!(!overrides.default.contains_key(REFRESH.id()));
            assert!(overrides.default.contains_key("future_cmd"));
        },
    );
}

#[test]
fn conflict_check_and_log_work_through_xkb_backend() {
    with_xkb("conflict_check_and_log_work_through_xkb_backend", || {
        let mut overrides = KeymapOverrides::default();
        overrides.set_bindings(
            Map::Default,
            RENAME,
            vec![Binding::plain(Key::F2), Binding::ctrl(Key::c)],
        );
        let keymap = resolve_keymap(&overrides, None);
        let policy = Policy {
            type_to_search: true,
        };
        let findings =
            conflicts::check(&keymap, Map::Default, RENAME, Binding::ctrl(Key::c), policy);
        assert!(findings.iter().any(|finding| {
            finding.severity == Severity::Error && finding.kind == conflicts::Kind::Duplicate(COPY)
        }));

        let file = Path::new("/settings/settings.toml");
        let logged = strata_test_support::capture_logs(|| {
            report_keymap_conflicts(&keymap, file, policy);
        });
        let line = strata_test_support::captured_event(
            &logged,
            "keymap entry has a key that never fires there or stops another key working",
        );
        assert!(
            line.contains("/settings/settings.toml")
                && line.contains("rename")
                && line.contains("<Control>c"),
            "{line}"
        );
    });
}
