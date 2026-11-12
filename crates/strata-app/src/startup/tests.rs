// SPDX-License-Identifier: MIT

use strata_desktop::keys::{
    self, Backend, Context, Key, KeyPress, KeyPressParts, Keys, Map, Modifiers, Typed,
    bindings::COPY, label_in_map, rewrite,
};

use super::{UiThread, every_binding_fires};

const MODULE: &str = "startup::tests";

#[test]
fn a_window_matches_and_labels_catalog_keys_without_a_toolkit() {
    crate::test_support::hosted(
        MODULE,
        "a_window_matches_and_labels_catalog_keys_without_a_toolkit",
        |_| {
            let press = KeyPress::from_parts(KeyPressParts {
                key: Key::c,
                modifiers: Modifiers::CONTROL_MASK,
                held: Modifiers::CONTROL_MASK,
                typed: Typed::Character('c'),
                digit: None,
                fallback: false,
            });
            assert!(rewrite(press, Context { typing: false }).is(COPY));
            assert_eq!(
                label_in_map("en", &Keys::One(COPY.shown()), Map::Default),
                "Ctrl+C"
            );
        },
    );
}

/// Some other key library, as a process that linked two interfaces would have.
struct Foreign;

impl Backend for Foreign {
    fn key_name(&self, _: Key) -> Option<String> {
        None
    }

    fn key_from_name(&self, _: &str) -> Option<Key> {
        None
    }

    fn key_to_unicode(&self, _: Key) -> Option<char> {
        None
    }

    fn key_from_unicode(&self, _: char) -> Key {
        Key::VoidSymbol
    }

    fn key_to_lower(&self, key: Key) -> Key {
        key
    }

    fn key_to_upper(&self, key: Key) -> Key {
        key
    }

    fn accelerator_parse(&self, _: &str) -> Option<(Key, Modifiers)> {
        None
    }

    fn accelerator_name(&self, _: Key, _: Modifiers) -> String {
        String::new()
    }
}

#[test]
fn the_ui_thread_is_not_claimed_over_another_key_library() {
    strata_test_support::isolated_test(
        "startup::tests::the_ui_thread_is_not_claimed_over_another_key_library",
        std::iter::empty::<(&str, &str)>(),
        || true,
        || {
            assert!(keys::install(&Foreign, every_binding_fires));
            let refused = UiThread::claim().err().expect("the claim is refused");
            assert!(refused.contains("key library"), "{refused}");
        },
    );
}
