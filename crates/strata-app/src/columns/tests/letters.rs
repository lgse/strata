// SPDX-License-Identifier: MIT

//! What a plain letter does in the Columns view, by the saved "Type to search" choice.

use super::{MODULE, fixture, open, press, save_settings, wait};
use crate::test_support::hosted_after;

/// `keyboard_navigation/arrow-keys-move-focus-remaining` (`columns-hjkl`): its header turns
/// "Type to search" off, and `j` and `k` are then Down and Up.
#[test]
fn with_type_to_search_off_the_letters_move_as_the_arrows_do() {
    hosted_after(
        MODULE,
        "letters::with_type_to_search_off_the_letters_move_as_the_arrows_do",
        || save_settings("type_to_search = false\n"),
        |mut host| {
            let folder = fixture();
            open(&mut host, folder.path());
            press(&mut host, "End");
            press(&mut host, "Up");
            wait(&mut host, "readme.md", |seen| {
                seen.focused.as_deref() == Some("readme.md")
            });

            press(&mut host, "j");

            let moved = wait(&mut host, "j to move down", |seen| {
                seen.focused.as_deref() == Some("todo.txt")
            });
            assert_eq!(moved.panes[0].selected(), ["todo.txt"]);

            press(&mut host, "k");

            wait(&mut host, "k to move up", |seen| {
                seen.focused.as_deref() == Some("readme.md")
            });
        },
    );
}

#[test]
fn with_type_to_search_on_a_letter_moves_nothing() {
    hosted_after(
        MODULE,
        "letters::with_type_to_search_on_a_letter_moves_nothing",
        || save_settings("type_to_search = true\n"),
        |mut host| {
            let folder = fixture();
            open(&mut host, folder.path());

            press(&mut host, "j");

            // The letter opens the filter on it (type-to-search) and moves no cursor.
            let focused = host.app().browser.focused_item();
            let name = focused.map(|(_, _, entry)| entry.display_name);
            assert_eq!(name.as_deref(), Some("archive"));
        },
    );
}
