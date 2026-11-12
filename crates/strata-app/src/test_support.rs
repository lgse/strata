// SPDX-License-Identifier: MIT

//! What the crate's tests share: the offscreen host in a process of its own, and folders to
//! open in it.

use std::{path::Path, time::Duration};

use crate::{
    App,
    host::offscreen::Offscreen,
    slots::Entry,
    startup::{self, UiThread},
};

/// How long a test waits for the browser before it fails.
pub(crate) const PATIENCE: Duration = Duration::from_secs(30);

/// Runs the test `name` of the test module `module` on a host in a process of its own: the
/// default GLib context, the start-up sequence and the saved settings belong to one process.
/// `prepare` runs there first, before anything is read.
pub(crate) fn hosted_after(
    module: &str,
    name: &str,
    prepare: impl FnOnce(),
    run: impl FnOnce(Offscreen),
) {
    strata_test_support::isolated_test(
        &format!("{module}::{name}"),
        std::iter::empty::<(&str, &str)>(),
        || true,
        || {
            prepare();
            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            run(Offscreen::new(ui, settings));
        },
    );
}

pub(crate) fn hosted(module: &str, name: &str, run: impl FnOnce(Offscreen)) {
    hosted_after(module, name, || {}, run);
}

/// As [`hosted`], on a host whose window is made of the table `parts`: a test's made-up
/// parts in the place of the registered ones.
pub(crate) fn hosted_over(
    module: &str,
    name: &str,
    parts: fn() -> Vec<Entry<App>>,
    run: impl FnOnce(Offscreen),
) {
    strata_test_support::isolated_test(
        &format!("{module}::{name}"),
        std::iter::empty::<(&str, &str)>(),
        || true,
        || {
            let settings = startup::load_settings();
            let ui = UiThread::claim().expect("the UI thread");
            run(Offscreen::with_parts(parts().leak(), ui, settings));
        },
    );
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
