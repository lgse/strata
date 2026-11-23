// SPDX-License-Identifier: MIT

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

    pub(super) fn put(&self, path: &str, text: impl AsRef<[u8]>) {
        let path = self.0.join(path);
        let dir = path.parent().unwrap_or(&self.0);
        // Record mtimes before writing: under coarse filesystem timestamp ticks, a write
        // can reset a previously bumped directory back to the current tick, so advancing
        // only `after + 1s` would repeat the cached stamp.
        let mut to_bump = Vec::new();
        let mut cursor = Some(dir);
        while let Some(current) = cursor {
            let before = std::fs::metadata(current).and_then(|m| m.modified()).ok();
            let existed = before.is_some();
            to_bump.push((current.to_path_buf(), before));
            if existed || current == self.0 {
                break;
            }
            cursor = current.parent();
        }
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for (target, before) in to_bump {
            let handle =
                File::open(&target).unwrap_or_else(|e| panic!("{}: {e}", target.display()));
            let after = handle
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or_else(|e| panic!("{}: {e}", target.display()));
            let base = before.map_or(after, |was| after.max(was));
            handle
                .set_modified(base + Duration::from_secs(1))
                .unwrap_or_else(|e| panic!("{}: {e}", target.display()));
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
