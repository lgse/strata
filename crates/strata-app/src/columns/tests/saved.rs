// SPDX-License-Identifier: MIT

//! The saved settings this interface writes back.

use std::time::Duration;

use super::{MODULE, PATIENCE, fixture, open, press, save_settings, seen, settings_file, wait};
use crate::test_support::hosted_after;

fn saved_show_hidden() -> Option<bool> {
    let saved = std::fs::read_to_string(settings_file()).ok()?;
    saved
        .parse::<toml::Table>()
        .ok()?
        .get("show_hidden")?
        .as_bool()
}

/// `hidden_files/hidden-files-persist-across-restart`: Ctrl+. saves `show_hidden`, and the
/// rest of the file stays as another Strata process left it.
#[test]
fn the_hidden_files_choice_is_saved_and_nothing_else_is_rewritten() {
    hosted_after(
        MODULE,
        "saved::the_hidden_files_choice_is_saved_and_nothing_else_is_rewritten",
        || save_settings("group_by_type = true\nfrom_a_later_strata = \"kept\"\n"),
        |mut host| {
            let folder = fixture();
            open(&mut host, folder.path());

            for shown in [true, false] {
                press(&mut host, "ctrl+period");

                let saved = host.run_until(PATIENCE, |_| saved_show_hidden() == Some(shown));
                assert!(saved, "show_hidden was not saved as {shown}");
                assert_eq!(seen(&host).panes[0].names().contains(&".hidden.txt"), shown);
            }
            let file = std::fs::read_to_string(settings_file()).expect("settings");
            let table: toml::Table = file.parse().expect("a table");
            assert_eq!(
                table.get("group_by_type"),
                Some(&toml::Value::Boolean(true))
            );
            assert_eq!(
                table
                    .get("from_a_later_strata")
                    .and_then(toml::Value::as_str),
                Some("kept")
            );
            // Only what this process changed: a default it never touched is not written.
            assert_eq!(table.get("sort_key"), None, "{file}");
        },
    );
}

/// A file that was there and could not be read is the user's to repair: nothing replaces it.
#[test]
fn an_unreadable_settings_file_is_never_saved_over() {
    const BROKEN: &str = "show_hidden = [unclosed\n";
    hosted_after(
        MODULE,
        "saved::an_unreadable_settings_file_is_never_saved_over",
        || save_settings(BROKEN),
        |mut host| {
            let folder = fixture();
            open(&mut host, folder.path());

            press(&mut host, "ctrl+period");

            wait(&mut host, "the hidden entries", |seen| {
                seen.panes[0].names().contains(&".hidden.txt")
            });
            host.run_until(Duration::from_millis(300), |_| false);
            let file = std::fs::read_to_string(settings_file()).expect("settings");
            assert_eq!(file, BROKEN);
        },
    );
}
