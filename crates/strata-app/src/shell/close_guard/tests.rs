// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use strata_core::{model::Location, storage::config_directory};
use strata_fs::services::{InvocationSource, JobRequest};

use super::{Refusal, refusal};
use crate::{
    App, custom_actions,
    host::offscreen::Offscreen,
    test_support::{PATIENCE, fixture, held, hosted},
};

const MODULE: &str = "shell::close_guard::tests";

/// Opens `folder` and waits until it lists `todo.txt`.
fn listed(host: &mut Offscreen, folder: &std::path::Path) {
    host.app().open(Location::local(folder));
    let listed = |host: &Offscreen| {
        let browser = host.app().browser();
        let has = |entries: &[strata_core::model::FileEntry]| {
            entries.iter().any(|entry| entry.display_name == "todo.txt")
        };
        browser.with_column_entries(0, has).unwrap_or(false)
    };
    assert!(host.run_until(PATIENCE, listed), "the folder listed");
}

fn refusals(app: &App) -> [Option<Refusal>; 2] {
    [refusal(app, false), refusal(app, true)]
}

/// GTK's tab guard refuses any window's close while a tab's file operation runs.
#[test]
fn a_running_file_operation_refuses_every_close() {
    hosted(
        MODULE,
        "a_running_file_operation_refuses_every_close",
        |mut host| {
            let folder = fixture();
            listed(&mut host, folder.path());
            assert_eq!(refusals(host.app()), [None, None]);

            host.runtime()
                .update(|app, _| held::busy(&app.browser, "todo.txt"));

            assert_eq!(
                refusals(host.app()),
                [Some(Refusal::Operations), Some(Refusal::Operations)]
            );
        },
    );
}

/// A custom action that runs until it is cancelled.
fn write_waiting_action() {
    let dir = config_directory().join("actions").join("wait");
    std::fs::create_dir_all(&dir).expect("the action's folder");
    let manifest = "schema_version = 1\nid = \"wait\"\nname = \"Wait\"\nmenu = \"top\"\n[when]\nkinds = [\"file\", \"folder\"]\n[run]\nruntime = \"command\"\nprogram = \"/bin/sleep\"\nargs = [\"30\"]\n";
    std::fs::write(dir.join("action.toml"), manifest).expect("action.toml");
}

/// GTK's jobs guard refuses only the last window's close while a job runs or waits: the
/// application would end with it.
#[test]
fn a_running_job_refuses_only_the_last_window_s_close() {
    hosted(
        MODULE,
        "a_running_job_refuses_only_the_last_window_s_close",
        |mut host| {
            write_waiting_action();
            let folder = fixture();
            listed(&mut host, folder.path());
            let target: PathBuf = folder.path().join("todo.txt");
            let parent = folder.path().to_owned();
            let mut started = false;
            host.runtime().update(|app, _| {
                custom_actions::shared_registry(app);
            });
            let actions = |app: &App| {
                let catalog = app.custom_actions.catalog();
                catalog.map_or_else(Vec::new, |catalog| catalog.actions().to_vec())
            };
            let loaded = host.run_until(PATIENCE, |host| actions(host.app()).len() == 1);
            assert!(loaded, "the action loaded");
            host.runtime().update(|app, _| {
                let action = actions(app)[0].clone();
                let request = JobRequest {
                    action,
                    inputs: vec![target],
                    parent,
                    source: InvocationSource::Selection,
                };
                started = custom_actions::shared_jobs(app).enqueue(request).is_ok();
            });
            assert!(started, "the job started");

            assert_eq!(refusals(host.app()), [None, Some(Refusal::Jobs)]);

            host.runtime().update(|app, _| {
                let jobs = custom_actions::shared_jobs(app);
                for job in jobs.snapshot() {
                    jobs.cancel(job.id);
                }
            });
        },
    );
}
