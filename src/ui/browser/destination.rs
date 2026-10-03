// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

pub(super) fn resolve_destination_path(input: &str, base: &Path, home: &Path) -> PathBuf {
    let input = input.trim();
    if input == "~" {
        home.to_path_buf()
    } else if let Some(relative) = input.strip_prefix("~/") {
        home.join(relative)
    } else {
        let path = Path::new(input);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        }
    }
}

pub(super) fn canonical_existing_directory(path: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(path).ok()?;
    canonical.is_dir().then_some(canonical)
}

pub(super) fn canonical_directory_within(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let root = canonical_existing_directory(root)?;
    let candidate = canonical_existing_directory(candidate)?;
    candidate.strip_prefix(root).ok()?;
    Some(candidate)
}

pub(super) fn rebind_directory_within_root(
    opened_root: &Path,
    selected: &Path,
    current_root: &Path,
) -> Option<PathBuf> {
    let relative = selected.strip_prefix(opened_root).ok()?;
    let current_root = canonical_existing_directory(current_root)?;
    canonical_directory_within(&current_root, &current_root.join(relative))
}
