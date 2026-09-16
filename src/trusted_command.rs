// SPDX-License-Identifier: MIT

use std::{
    path::{Component, Path, PathBuf},
    process::Command,
};

const TRUSTED_DIRECTORIES: [&str; 4] = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"];

#[cfg(test)]
mod tests;

pub(crate) fn resolve(name: &str) -> Result<PathBuf, String> {
    let dirs: [&Path; 4] = TRUSTED_DIRECTORIES.map(Path::new);
    resolve_in(name, &dirs)
}

pub(crate) fn command(name: &str) -> Result<Command, String> {
    Ok(Command::new(resolve(name)?))
}

pub(crate) fn resolve_in(name: &str, dirs: &[&Path]) -> Result<PathBuf, String> {
    if !is_single_basename(name) {
        return Err("helper name must be a single basename".to_owned());
    }

    let roots = canonical_directories(dirs);
    for dir in dirs {
        let candidate = dir.join(name);
        if !candidate.is_file() {
            continue;
        }
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        if roots.iter().any(|root| canonical.starts_with(root)) {
            return Ok(canonical);
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

fn canonical_directories(dirs: &[&Path]) -> Vec<PathBuf> {
    dirs.iter()
        .filter_map(|dir| dir.canonicalize().ok())
        .collect()
}
