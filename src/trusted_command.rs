// SPDX-License-Identifier: MIT

use std::{
    path::{Component, Path, PathBuf},
    process::Command,
};

/// Where a helper basename may appear. Admin-managed; never `$HOME` or `PATH`.
/// NixOS `security.wrappers` is first so setuid wrappers win over store
/// symlinks.
pub(crate) const SEARCH_ROOTS: &[&str] = &[
    "/run/wrappers/bin",
    "/usr/bin",
    "/usr/sbin",
    "/bin",
    "/sbin",
    "/run/current-system/sw/bin",
    "/run/current-system/profile/bin",
];

/// Where canonicalize of a search hit may land. Wrapper dirs stay here
/// because NixOS wrappers are regular files, not `/nix/store` symlinks.
pub(crate) const TRUST_ROOTS: &[&str] = &[
    "/usr/bin",
    "/usr/sbin",
    "/bin",
    "/sbin",
    "/run/wrappers/bin",
    "/nix/store",
    "/gnu/store",
];

#[cfg(test)]
mod tests;

pub(crate) fn resolve(name: &str) -> Result<PathBuf, String> {
    let search: Vec<&Path> = SEARCH_ROOTS.iter().copied().map(Path::new).collect();
    let trust: Vec<&Path> = TRUST_ROOTS.iter().copied().map(Path::new).collect();
    resolve_in(name, &search, &trust)
}

pub(crate) fn command(name: &str) -> Result<Command, String> {
    Ok(Command::new(resolve(name)?))
}

pub(crate) fn resolve_in(
    name: &str,
    search_roots: &[&Path],
    trust_roots: &[&Path],
) -> Result<PathBuf, String> {
    if !is_single_basename(name) {
        return Err("helper name must be a single basename".to_owned());
    }

    for dir in search_roots {
        let candidate = dir.join(name);
        if !candidate.is_file() {
            continue;
        }
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        if sits_under(&canonical, trust_roots) {
            // Exec the search hit, not the store target, so argv0 stays the
            // profile path for busybox/coreutils and Nix wrappers.
            return Ok(found_path(candidate));
        }
    }

    Err(format!(
        "{name} was not found in a trusted system directory"
    ))
}

fn is_single_basename(name: &str) -> bool {
    if name.is_empty() || name.contains('/') || name.contains('\0') {
        return false;
    }
    matches!(
        Path::new(name).components().collect::<Vec<_>>().as_slice(),
        [Component::Normal(part)] if *part == name
    )
}

fn sits_under(canonical: &Path, trust_roots: &[&Path]) -> bool {
    trust_roots.iter().any(|root| {
        let root = match root.canonicalize() {
            Ok(path) => path,
            Err(_) if root.is_absolute() => (*root).to_path_buf(),
            Err(_) => return false,
        };
        canonical.starts_with(root)
    })
}

fn found_path(candidate: PathBuf) -> PathBuf {
    if candidate.is_absolute() {
        candidate
    } else {
        std::path::absolute(&candidate).unwrap_or(candidate)
    }
}
