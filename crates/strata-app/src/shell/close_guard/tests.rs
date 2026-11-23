// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use strata_core::{model::Location, storage::config_directory};
use strata_fs::services::{InvocationSource, JobRequest};

use super::{Refusal, refusal};
use crate::{
    App, custom_actions,
    host::offscreen::Offscreen,
    shell,
    startup::{self, UiThread},
    test_support::{PATIENCE, fixture, held, hosted, in_own_process},
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

fn start_waiting_job(host: &mut Offscreen, folder: &std::path::Path) {
    listed(host, folder);
    let target: PathBuf = folder.join("todo.txt");
    let parent = folder.to_owned();
    host.runtime().update(|app, _| {
        custom_actions::shared_registry(app);
    });
    let actions = |app: &App| {
        let catalog = app.custom_actions.catalog();
        catalog.map_or_else(Vec::new, |catalog| catalog.actions().to_vec())
    };
    let loaded = host.run_until(PATIENCE, |host| actions(host.app()).len() == 1);
    assert!(loaded, "the action loaded");
    let mut started = false;
    host.runtime().update(|app, _| {
        let request = JobRequest {
            action: actions(app)[0].clone(),
            inputs: vec![target],
            parent,
            source: InvocationSource::Selection,
        };
        started = custom_actions::shared_jobs(app).enqueue(request).is_ok();
    });
    assert!(started, "the job was queued");
    let running = host.run_until(PATIENCE, |host| running(host.app()) == 1);
    assert!(running, "the job runs");
}

fn running(app: &App) -> usize {
    custom_actions::job_service(app).map_or(0, |jobs| jobs.running_count())
}

fn cancel_jobs(host: &mut Offscreen) {
    host.runtime().update(|app, _| {
        let jobs = custom_actions::shared_jobs(app);
        for job in jobs.snapshot() {
            jobs.cancel(job.id);
        }
    });
}

#[test]
fn a_running_job_refuses_only_the_last_window_s_close() {
    hosted(
        MODULE,
        "a_running_job_refuses_only_the_last_window_s_close",
        |mut host| {
            write_waiting_action();
            let folder = fixture();
            start_waiting_job(&mut host, folder.path());

            assert_eq!(refusals(host.app()), [None, Some(Refusal::Jobs)]);

            cancel_jobs(&mut host);
        },
    );
}

#[test]
fn a_job_outlives_the_window_it_started_in() {
    in_own_process(
        &format!("{MODULE}::a_job_outlives_the_window_it_started_in"),
        &[],
        || {
            write_waiting_action();
            let folder = fixture();
            let ui = UiThread::claim().expect("the UI thread");
            let mut first = Offscreen::new(ui, startup::load_settings());
            let mut second = Offscreen::new(ui, shell::share_settings(first.app()));
            while second.turn() {}
            start_waiting_job(&mut first, folder.path());
            assert_eq!(running(second.app()), 1, "the second window sees the job");

            drop(first);

            while second.turn() {}
            assert_eq!(running(second.app()), 1, "the job still runs");
            assert_eq!(refusal(second.app(), true), Some(Refusal::Jobs));
            cancel_jobs(&mut second);
            let ended = second.run_until(PATIENCE, |host| refusal(host.app(), true).is_none());
            assert!(
                ended,
                "the second window drives the cancelled job to its end"
            );
        },
    );
}
