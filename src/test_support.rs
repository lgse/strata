// SPDX-License-Identifier: GPL-3.0-or-later

#![cfg(test)]

use std::sync::{LockResult, Mutex, MutexGuard};

pub(crate) struct TestMutex(Mutex<()>);

impl TestMutex {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(()))
    }

    pub(crate) fn lock(&self) -> LockResult<MutexGuard<'_, ()>> {
        match self.0.lock() {
            Ok(guard) => Ok(guard),
            Err(error) => {
                self.0.clear_poison();
                Ok(error.into_inner())
            }
        }
    }
}

/// Serializes tests that drive `glib::MainContext::default()` directly, since it is a
/// process-wide singleton and concurrent access from the test harness's per-test threads panics
/// with a GLib thread-affinity error. A single shared lock, not one static per module: two
/// separate locks each covering only their own module's tests do not prevent a test in one
/// module from racing a test in another, since neither knows about the other's lock. Poisoning is
/// cleared because the mutex protects no state and should not turn one failure into a cascade.
pub(crate) static ASYNC_MAIN_CONTEXT_DEFAULT: TestMutex = TestMutex::new();

/// GTK initialization is thread-affine; each UI test gets a process and disposable preferences.
pub(crate) fn gtk_test(name: &str, run: impl FnOnce()) {
    const CHILD: &str = "STRATA_ISOLATED_GTK_TEST";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        if let Err(error) = gtk::init() {
            assert!(
                std::env::var_os("STRATA_REQUIRE_GTK_TESTS").is_none(),
                "GTK display required: {error}"
            );
            eprintln!("Skipping {name}: {error}");
            return;
        }
        crate::assets::prepare().expect("bundled assets");
        crate::assets::register_icon_theme();
        run();
        return;
    }
    let sandbox = tempfile::tempdir().expect("isolated preferences");
    let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", name, "--nocapture"])
        .env(CHILD, name)
        .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
        .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
        .env("XDG_DATA_HOME", sandbox.path().join("data"))
        .status()
        .expect("isolated GTK test starts");
    assert!(status.success(), "{name} failed");
}
