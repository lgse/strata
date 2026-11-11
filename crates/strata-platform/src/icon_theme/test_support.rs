//! Fixtures shared by the module's tests.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(super) fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/icons")
}

/// A writable icon root under the build's target directory, removed on drop.
pub(super) struct Scratch(PathBuf);

impl Scratch {
    pub(super) fn new(name: &str) -> Self {
        // `CARGO_TARGET_TMPDIR` is set for integration tests only. A unit test binary sits in
        // `<target>/<profile>/deps/`, so the profile directory is two levels up.
        let exe = std::env::current_exe().unwrap_or_else(|e| panic!("test binary: {e}"));
        let profile = exe
            .ancestors()
            .nth(2)
            .unwrap_or_else(|| panic!("{}", exe.display()));
        let dir = profile.join(format!("icon-theme-scratch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        Scratch(dir)
    }

    pub(super) fn root(&self) -> &Path {
        &self.0
    }

    /// Writes `text` to `path`, then moves the directory's modification time a second on, so
    /// a check sees the change even where the kernel's timestamps are coarser than the test.
    pub(super) fn put(&self, path: &str, text: impl AsRef<[u8]>) {
        let path = self.0.join(path);
        let dir = path.parent().unwrap_or(&self.0);
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let handle = File::open(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let was = handle.metadata().and_then(|m| m.modified());
        let was = was.unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        handle
            .set_modified(was + Duration::from_secs(1))
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
