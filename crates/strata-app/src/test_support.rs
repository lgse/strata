// SPDX-License-Identifier: MIT

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    time::Duration,
};

use rune::{KeyEvent, Mods};
use strata_components::rename::Polled;
use strata_fs::app::VisibleDepths;

#[cfg(feature = "qa")]
use crate::slots::Probe;
use crate::{
    App,
    host::offscreen::Offscreen,
    settings::Saved,
    slots::{Build, Editing, Entry, Rows, View},
    startup::{self, UiThread},
};

pub(crate) mod held;
#[cfg(test)]
mod tests;
pub(crate) mod view;

/// How long a test waits for the browser before it fails.
pub(crate) const PATIENCE: Duration = Duration::from_secs(30);

/// Where a test run in a process of its own says that it ran.
const RAN: &str = "STRATA_TEST_RAN";

pub(crate) fn in_own_process(test: &str, env: &[(&str, &OsStr)], body: impl FnOnce()) {
    let own = tempfile::tempdir().expect("a folder for the marker");
    // The parent picks the place, and the child is told it.
    let marker = std::env::var_os(RAN).map_or_else(|| own.path().join("ran"), PathBuf::from);
    let env = env.iter().copied().chain([(RAN, marker.as_os_str())]);
    strata_test_support::isolated_test(
        test,
        env,
        || true,
        || {
            std::fs::write(&marker, test).expect("the marker");
            body();
        },
    );
    assert!(
        marker.exists(),
        "`{test}` ran nowhere: no test of this executable has that path, so the child ran none \
         and exited well. Give the test's whole path, with every module it is in."
    );
}

fn hosted_on(
    (module, name, env): (&str, &str, &[(&str, &OsStr)]),
    prepare: impl FnOnce(),
    host: impl FnOnce(UiThread, Saved) -> Offscreen,
    run: impl FnOnce(Offscreen),
) {
    in_own_process(&format!("{module}::{name}"), env, || {
        prepare();
        let settings = startup::load_settings();
        let ui = UiThread::claim().expect("the UI thread");
        run(host(ui, settings));
    });
}

/// A test on the window the crate registers, after `prepare` has run in its process.
pub(crate) fn hosted_after(
    module: &str,
    name: &str,
    prepare: impl FnOnce(),
    run: impl FnOnce(Offscreen),
) {
    hosted_on((module, name, &[]), prepare, Offscreen::new, run);
}

pub(crate) fn hosted(module: &str, name: &str, run: impl FnOnce(Offscreen)) {
    hosted_after(module, name, || {}, run);
}

pub(crate) fn hosted_with_env(
    (module, name): (&str, &str),
    env: &[(&str, &OsStr)],
    run: impl FnOnce(Offscreen),
) {
    hosted_on((module, name, env), || {}, Offscreen::new, run);
}

pub(crate) fn hosted_over(
    module: &str,
    name: &str,
    parts: fn() -> Vec<Entry<App>>,
    run: impl FnOnce(Offscreen),
) {
    hosted_over_with((module, name, &[]), || {}, parts, run);
}

/// [`hosted_over`], with `env` set and `prepare` run before settings are read.
pub(crate) fn hosted_over_with(
    target: (&str, &str, &[(&str, &OsStr)]),
    prepare: impl FnOnce(),
    parts: fn() -> Vec<Entry<App>>,
    run: impl FnOnce(Offscreen),
) {
    let host = |ui, settings| Offscreen::with_parts(parts().leak(), ui, settings);
    hosted_on(target, prepare, host, run);
}

pub(crate) fn idle_view<S>(
    name: &'static str,
    build: Build<S>,
    shows: fn(&str) -> bool,
) -> View<S> {
    View {
        build,
        shows,
        focus: name,
        supports_type_grouping: false,
        rechecks: VisibleDepths::All,
        apply: |_, _, _| false,
        returns: None,
        pointer_at: None,
        end_gestures: |_| {},
        rows: Rows {
            position: |_, _| None,
            neighbour: |_, _, _| None,
            activate: |_, _| {},
            select_only: |_, _, _| {},
            select_all: |_, _| {},
            reveal: |_, _, _| {},
        },
        editing: Editing {
            begin: |_, _, _| None,
            tick: |_| Polled::default(),
            hold: None,
            follow_now: None,
        },
        #[cfg(feature = "qa")]
        probe: Probe {
            name: "Made up",
            roles: ("list-box", "list-box-option"),
            listed: |_| true,
            lists: |_| Vec::new(),
            presented: |_| crate::handover::Presented::default(),
        },
    }
}

pub(crate) fn send_key(host: &mut Offscreen, event: KeyEvent) {
    host.modifiers(event.mods);
    host.runtime().key(event);
    host.modifiers(Mods::default());
}

pub(crate) fn folder_of(names: &[&str]) -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("folder");
    for name in names {
        std::fs::write(folder.path().join(name), name).expect("file");
    }
    folder
}

fn write_tree(folder: &Path, files: &[&str]) {
    for file in files {
        let path = folder.join(file);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("folder");
        std::fs::write(path, file).expect("file");
    }
}

pub(crate) fn fixture() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("folder");
    std::fs::create_dir(folder.path().join("archive")).expect("folder");
    write_tree(
        folder.path(),
        &[
            "documents/notes.txt",
            "documents/report.md",
            "documents/spreadsheet.csv",
            "pictures/diagram.txt",
            "pictures/photo.txt",
            "readme.md",
            "todo.txt",
            ".hidden.txt",
        ],
    );
    folder
}
