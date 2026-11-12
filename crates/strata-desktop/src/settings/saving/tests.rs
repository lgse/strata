// SPDX-License-Identifier: MIT

use std::{
    fs,
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

use super::*;
use crate::storage::writer;

/// Runs `run` in a process of its own with `settings` as its settings file, if any.
fn isolated(name: &str, settings: Option<&str>, run: impl FnOnce()) {
    strata_test_support::isolated_test(
        &format!("settings::saving::tests::{name}"),
        std::iter::empty::<(&str, &str)>(),
        || true,
        || {
            if let Some(contents) = settings {
                let path = settings_path();
                fs::create_dir_all(path.parent().expect("a configuration folder"))
                    .expect("configuration folder");
                fs::write(path, contents).expect("settings fixture");
            }
            run();
        },
    );
}

/// The result of a save the writer has finished, polled by hand: delivering it needs no
/// executor and no dispatch. (`writer::settle` itself iterates GLib's default context.)
fn finished(save: PendingSave) -> FinishedSave {
    writer::settle();
    let mut finished = pin!(save.finished());
    match finished
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(finished) => finished,
        Poll::Pending => panic!("the writer settled and the save has no result"),
    }
}

/// A file that reads without repair. One that lacks these two is salvaged entry by entry,
/// and every value salvage filled in is then saved with the first change.
const COMPLETE: &str = "mode = \"theme\"\ntheme = \"azure-glow\"\n";

fn saved_table() -> toml::Table {
    fs::read_to_string(settings_path())
        .expect("settings file")
        .parse()
        .expect("a table")
}

fn saved_file() -> Option<String> {
    fs::read_to_string(settings_path()).ok()
}

#[test]
fn a_commit_writes_only_what_changed_and_keeps_the_rest_of_the_file() {
    isolated(
        "a_commit_writes_only_what_changed_and_keeps_the_rest_of_the_file",
        Some(&format!(
            "{COMPLETE}group_by_type = true\nfrom_a_later_strata = \"kept\"\n"
        )),
        || {
            let (mut preferences, mut saving) = load();
            assert!(preferences.group_by_type);

            preferences.show_hidden = true;
            let commit = saving.commit(&preferences);
            assert!(commit.changed && commit.problem.is_none());
            assert!(saving.unsaved() && saving.setting_unsaved("show_hidden"));
            let outcome = saving.saved(finished(commit.save.expect("a queued save")));
            assert!(matches!(outcome, SaveOutcome::Saved));

            let table = saved_table();
            assert_eq!(table.get("show_hidden"), Some(&toml::Value::Boolean(true)));
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
            assert_eq!(
                table.get("sort_key"),
                None,
                "a default nobody changed is not written"
            );
            assert!(!saving.unsaved() && !saving.setting_unsaved("show_hidden"));

            let again = saving.commit(&preferences);
            assert!(!again.changed && again.save.is_none() && again.problem.is_none());
        },
    );
}

#[test]
fn an_unreadable_file_is_never_saved_over() {
    const BROKEN: &str = "show_hidden = [unclosed\n";
    isolated(
        "an_unreadable_file_is_never_saved_over",
        Some(BROKEN),
        || {
            let (mut preferences, mut saving) = load();
            assert!(saving.load_failure().is_some());

            preferences.show_hidden = true;
            let commit = saving.commit(&preferences);
            assert!(commit.changed && commit.save.is_none());
            assert!(matches!(
                commit.problem,
                Some(SaveProblem::UnreadableAtStartup { .. })
            ));
            // The problem is told with the change that met it, not with every commit.
            let again = saving.commit(&preferences);
            assert!(!again.changed && again.save.is_none() && again.problem.is_none());

            writer::settle();
            assert_eq!(saved_file().as_deref(), Some(BROKEN));
        },
    );
}

#[test]
fn a_failed_save_is_tried_again_by_a_commit_that_changes_nothing() {
    isolated(
        "a_failed_save_is_tried_again_by_a_commit_that_changes_nothing",
        None,
        || {
            let (mut preferences, mut saving) = load();
            fs::create_dir_all(settings_path()).expect("a folder where the file belongs");

            preferences.folder_peeking = true;
            let commit = saving.commit(&preferences);
            let outcome = saving.saved(finished(commit.save.expect("a queued save")));
            assert!(matches!(outcome, SaveOutcome::Failed { .. }));
            assert!(saving.unsaved() && saving.setting_unsaved("folder_peeking"));

            fs::remove_dir(settings_path()).expect("remove the folder");
            let retry = saving.commit(&preferences);
            assert!(!retry.changed);
            let outcome = saving.saved(finished(retry.save.expect("a retried save")));
            assert!(matches!(outcome, SaveOutcome::Saved));
            assert_eq!(
                saved_table().get("folder_peeking"),
                Some(&toml::Value::Boolean(true))
            );
            assert!(!saving.unsaved());
        },
    );
}

#[test]
fn only_the_latest_save_decides_whether_one_is_owed() {
    isolated(
        "only_the_latest_save_decides_whether_one_is_owed",
        None,
        || {
            let (mut preferences, mut saving) = load();
            preferences.folder_peeking = true;
            let first = saving.commit(&preferences).save.expect("a queued save");
            preferences.show_hidden = true;
            let second = saving.commit(&preferences).save.expect("a second save");

            assert!(matches!(
                saving.saved(finished(first)),
                SaveOutcome::Superseded
            ));
            assert!(
                saving.unsaved(),
                "the first result does not settle the second"
            );
            assert!(matches!(saving.saved(finished(second)), SaveOutcome::Saved));
            assert!(!saving.unsaved());

            let table = saved_table();
            assert_eq!(
                table.get("folder_peeking"),
                Some(&toml::Value::Boolean(true))
            );
            assert_eq!(table.get("show_hidden"), Some(&toml::Value::Boolean(true)));
        },
    );
}

#[test]
fn a_save_nobody_reports_is_written_again_with_the_next_one() {
    isolated(
        "a_save_nobody_reports_is_written_again_with_the_next_one",
        None,
        || {
            let (mut preferences, mut saving) = load();
            preferences.folder_peeking = true;
            drop(saving.commit(&preferences));
            writer::settle();
            assert!(saving.unsaved() && saving.setting_unsaved("folder_peeking"));

            // Another process takes the value back; this one still owes its own.
            fs::write(settings_path(), "folder_peeking = false\n").expect("another process");
            preferences.show_hidden = true;
            let commit = saving.commit(&preferences);
            let outcome = saving.saved(finished(commit.save.expect("a queued save")));
            assert!(matches!(outcome, SaveOutcome::Saved));

            let table = saved_table();
            assert_eq!(
                table.get("folder_peeking"),
                Some(&toml::Value::Boolean(true))
            );
            assert_eq!(table.get("show_hidden"), Some(&toml::Value::Boolean(true)));
            assert!(!saving.unsaved() && !saving.setting_unsaved("folder_peeking"));
        },
    );
}

#[test]
fn values_adopted_from_another_process_are_not_saved_back() {
    isolated(
        "values_adopted_from_another_process_are_not_saved_back",
        Some(COMPLETE),
        || {
            let (mut preferences, mut saving) = load();
            let before = saving.revision();

            preferences.icons_thumbnail_size += 16;
            assert!(saving.record_adopted(&preferences, &["icons_thumbnail_size"]));
            assert_ne!(saving.revision(), before);
            assert!(!saving.unsaved() && !saving.setting_unsaved("icons_thumbnail_size"));
            let unchanged = saving.commit(&preferences);
            assert!(!unchanged.changed && unchanged.save.is_none());

            preferences.show_hidden = true;
            let commit = saving.commit(&preferences);
            let outcome = saving.saved(finished(commit.save.expect("a queued save")));
            assert!(matches!(outcome, SaveOutcome::Saved));
            let table = saved_table();
            assert_eq!(table.get("show_hidden"), Some(&toml::Value::Boolean(true)));
            assert_eq!(
                table.get("icons_thumbnail_size"),
                None,
                "the other process's file holds it; this one never wrote it"
            );
        },
    );
}

#[test]
fn a_start_up_repair_is_no_change_and_is_saved_with_the_next_one() {
    isolated(
        "a_start_up_repair_is_no_change_and_is_saved_with_the_next_one",
        Some(COMPLETE),
        || {
            let (mut preferences, mut saving) = load();
            let before = saving.revision();

            preferences.theme = "nord".to_owned();
            saving.rebase(&preferences);
            assert_eq!(saving.revision(), before);
            let unchanged = saving.commit(&preferences);
            assert!(!unchanged.changed && unchanged.save.is_none());
            writer::settle();
            assert_eq!(
                saved_file().as_deref(),
                Some(COMPLETE),
                "a repair alone writes nothing"
            );

            preferences.show_hidden = true;
            let commit = saving.commit(&preferences);
            let outcome = saving.saved(finished(commit.save.expect("a queued save")));
            assert!(matches!(outcome, SaveOutcome::Saved));
            let table = saved_table();
            assert_eq!(
                table.get("theme").and_then(toml::Value::as_str),
                Some("nord")
            );
            assert_eq!(table.get("show_hidden"), Some(&toml::Value::Boolean(true)));
            assert_eq!(
                table.get("sort_key"),
                None,
                "the file is merged into, so the theme is there because the repair was recorded"
            );
        },
    );
}

/// A file without `mode` does not read whole: it is salvaged entry by entry, and what
/// salvage filled in counts as repaired. The first change then saves every value the file
/// lacked, beside the one that changed. The GTK application has always done so.
#[test]
fn a_salvaged_file_is_saved_with_every_value_loading_filled_in() {
    isolated(
        "a_salvaged_file_is_saved_with_every_value_loading_filled_in",
        Some("theme = \"nord\"\ngroup_by_type = true\nfrom_a_later_strata = \"kept\"\n"),
        || {
            let (mut preferences, mut saving) = load();
            assert!(saving.load_failure().is_none());
            assert_eq!(preferences.theme, "nord");
            assert!(saving.setting_unsaved("mode") && saving.setting_unsaved("sort_key"));
            assert!(!saving.setting_unsaved("theme") && !saving.unsaved());

            preferences.show_hidden = true;
            let commit = saving.commit(&preferences);
            let outcome = saving.saved(finished(commit.save.expect("a queued save")));
            assert!(matches!(outcome, SaveOutcome::Saved));

            let table = saved_table();
            let every_default =
                toml::Table::try_from(Preferences::default()).expect("the defaults as a table");
            for (key, default) in &every_default {
                let expected = match key.as_str() {
                    "theme" => toml::Value::String("nord".to_owned()),
                    "group_by_type" | "show_hidden" => toml::Value::Boolean(true),
                    _ => default.clone(),
                };
                assert_eq!(table.get(key), Some(&expected), "{key}");
            }
            assert_eq!(
                table
                    .get("from_a_later_strata")
                    .and_then(toml::Value::as_str),
                Some("kept")
            );
        },
    );
}
