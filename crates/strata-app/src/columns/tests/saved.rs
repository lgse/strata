// SPDX-License-Identifier: MIT

//! The saved settings this interface writes back.

use std::time::Duration;

use strata_components::columns::ColumnsIntent;

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

/// Toggles the hidden files on and off over a `settings.toml` that holds `seed`, then hands
/// the saved file to `check`.
fn toggle_hidden_files_over(test: &'static str, seed: &'static str, check: fn(&toml::Table, &str)) {
    hosted_after(
        MODULE,
        test,
        move || save_settings(seed),
        move |mut host| {
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
            check(&table, &file);
        },
    );
}

/// `hidden_files/hidden-files-persist-across-restart`: Ctrl+. saves `show_hidden`, and the
/// rest of a file that reads without repair stays as another Strata process left it.
#[test]
fn the_hidden_files_choice_is_saved_and_nothing_else_is_rewritten() {
    toggle_hidden_files_over(
        "saved::the_hidden_files_choice_is_saved_and_nothing_else_is_rewritten",
        "mode = \"theme\"\ntheme = \"azure-glow\"\ngroup_by_type = true\nfrom_a_later_strata = \"kept\"\n",
        |table, file| {
            // Only what this process changed: a default it never touched is not written.
            assert_eq!(table.get("sort_key"), None, "{file}");
        },
    );
}

/// A file without `mode` or `theme` does not read whole and is salvaged entry by entry. As
/// in the GTK application, what loading filled in is saved with the first change.
#[test]
fn a_salvaged_file_is_saved_with_what_loading_filled_in() {
    toggle_hidden_files_over(
        "saved::a_salvaged_file_is_saved_with_what_loading_filled_in",
        "group_by_type = true\nfrom_a_later_strata = \"kept\"\n",
        |table, file| {
            for filled in ["mode", "theme", "sort_key"] {
                assert!(table.contains_key(filled), "{filled} was not saved: {file}");
            }
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

/// A key is written until its save is known to have succeeded, and then left alone: what
/// another Strata process saves there afterwards stays through this one's next save. The
/// save's result reaches the bookkeeping through the window's queue, and a part's change
/// goes through `shell::saved`.
#[test]
fn a_saved_key_is_left_to_another_process_once_its_save_has_succeeded() {
    hosted_after(
        MODULE,
        "saved::a_saved_key_is_left_to_another_process_once_its_save_has_succeeded",
        || save_settings("mode = \"theme\"\ntheme = \"azure-glow\"\n"),
        |mut host| {
            let folder = fixture();
            open(&mut host, folder.path());
            press(&mut host, "ctrl+period");
            let saved = host.run_until(PATIENCE, |_| saved_show_hidden() == Some(true));
            assert!(saved, "show_hidden was not saved");
            // The writer answers after the file is in place, and the answer is heard in a turn.
            host.run_until(Duration::from_millis(300), |_| false);

            let ours = std::fs::read_to_string(settings_file()).expect("settings");
            let theirs = ours.replace("show_hidden = true", "show_hidden = false");
            assert_ne!(theirs, ours, "{ours}");
            std::fs::write(settings_file(), theirs).expect("another process's save");

            host.runtime().update(|app, _| {
                crate::shell::saved(app).change(|settings| settings.folder_peeking = true);
            });
            let peeking = || {
                let file = std::fs::read_to_string(settings_file()).ok()?;
                file.parse::<toml::Table>()
                    .ok()?
                    .get("folder_peeking")?
                    .as_bool()
            };
            let saved = host.run_until(PATIENCE, |_| peeking() == Some(true));
            assert!(saved, "the part's change was not saved");
            assert_eq!(saved_show_hidden(), Some(false));
        },
    );
}

fn saved_column_width() -> Option<i64> {
    let saved = std::fs::read_to_string(settings_file()).ok()?;
    saved
        .parse::<toml::Table>()
        .ok()?
        .get("browser_column_width")?
        .as_integer()
}

/// `view_switching/column-autofit-stays-inside`: the width a resize or an autofit ends at is
/// saved in whole base points, never narrower than a standard column, as GTK's
/// `remember_column_width` saves it.
#[test]
fn a_resized_columns_width_is_saved_whole_and_never_below_a_standard_column() {
    hosted_after(
        MODULE,
        "saved::a_resized_columns_width_is_saved_whole_and_never_below_a_standard_column",
        || save_settings("mode = \"theme\"\ntheme = \"azure-glow\"\n"),
        |mut host| {
            let folder = fixture();
            open(&mut host, folder.path());

            for (width, kept) in [(437.6, 438), (120.0, 300)] {
                host.runtime().update(|app, tx| {
                    crate::columns::intents::heard(app, ColumnsIntent::Width(width), tx);
                });
                let saved = host.run_until(PATIENCE, |_| saved_column_width() == Some(kept));
                assert!(saved, "{width} was saved as {:?}", saved_column_width());
            }
        },
    );
}
