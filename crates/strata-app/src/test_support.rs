// SPDX-License-Identifier: MIT

//! What the crate's tests share: the offscreen host in a process of its own, and folders to
//! open in it.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    time::Duration,
};

use rune::{KeyEvent, Mods, PhysicalInput};
use strata_components::rename::Polled;

#[cfg(feature = "qa")]
use crate::slots::Probe;
use crate::{
    App,
    host::offscreen::Offscreen,
    settings::Saved,
    slots::{Build, Editing, Entry, Rows, View},
    startup::{self, UiThread},
};

#[cfg(test)]
mod tests;
pub(crate) mod view;

/// How long a test waits for the browser before it fails.
pub(crate) const PATIENCE: Duration = Duration::from_secs(30);

/// Where a test run in a process of its own says that it ran.
const RAN: &str = "STRATA_TEST_RAN";

/// Runs `body` as the test `test` in a process of its own, with `env` set there, and fails
/// when no test ran there.
///
/// The child is this executable started for the one test whose path is `test` exactly
/// (`strata_test_support::isolated_test`). A path that is no test's starts nothing, and the
/// child still exits well, so without this check a test moved to another module, or renamed,
/// passes without running (finding lgse/strata#2193). Every test of this crate that runs in a
/// process of its own goes through here.
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

/// Runs the test `name` of the test module `module` on the host `host` makes, in a process of
/// its own: the default GLib context, the start-up sequence and the saved settings belong to
/// one process. `prepare` runs there first, before anything is read.
fn hosted_on(
    (module, name): (&str, &str),
    prepare: impl FnOnce(),
    host: impl FnOnce(UiThread, Saved) -> Offscreen,
    run: impl FnOnce(Offscreen),
) {
    in_own_process(&format!("{module}::{name}"), &[], || {
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
    hosted_on((module, name), prepare, Offscreen::new, run);
}

pub(crate) fn hosted(module: &str, name: &str, run: impl FnOnce(Offscreen)) {
    hosted_after(module, name, || {}, run);
}

/// A test on a window made of the table `parts`: a test's made-up parts in the place of the
/// registered ones.
pub(crate) fn hosted_over(
    module: &str,
    name: &str,
    parts: fn() -> Vec<Entry<App>>,
    run: impl FnOnce(Offscreen),
) {
    let host = |ui, settings| Offscreen::with_parts(parts().leak(), ui, settings);
    hosted_on((module, name), || {}, host, run);
}

/// A view for a made-up table: `build` draws it, it answers to `name` as the saved mode and
/// takes the keyboard at the key `name`, and whatever else is asked of it does nothing.
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
        pointer_at: None,
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
        },
    }
}

/// Feeds the host one key with its modifiers held around it, as a keyboard sends one: the
/// parts read the held modifiers to tell a Shift range from a move. The caller lets the
/// window settle as its test needs.
pub(crate) fn send_key(host: &mut Offscreen, event: KeyEvent) {
    let runtime = host.runtime();
    runtime.physical_input(PhysicalInput::Modifiers(event.mods));
    runtime.key(event);
    runtime.physical_input(PhysicalInput::Modifiers(Mods::default()));
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

/// The QA harness's canonical tree (`tests/fixtures/harness.json`), which the G2 scenarios
/// run in: three folders, two files and a hidden file.
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
