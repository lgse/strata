// SPDX-License-Identifier: MIT

use std::{
    fs::{self, Metadata},
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    process::Command,
};

// NixOS setuid wrappers must take precedence over store symlinks.
pub(crate) const SEARCH_ROOTS: &[&str] = &[
    "/run/wrappers/bin",
    "/usr/bin",
    "/usr/sbin",
    "/bin",
    "/sbin",
    "/run/current-system/sw/bin",
    "/run/current-system/profile/bin",
];

// NixOS wrappers are regular files, not store symlinks.
pub(crate) const TRUST_ROOTS: &[&str] = &[
    "/usr/bin",
    "/usr/sbin",
    "/bin",
    "/sbin",
    "/run/wrappers/bin",
    "/nix/store",
    "/gnu/store",
];

/// Colon-separated helper directories a package embeds at build time, such as
/// the `bin` directories of its Nix runtime closure.
const CONFIGURED_SEARCH_PATH: &str = match option_env!("STRATA_TRUSTED_COMMAND_PATH") {
    Some(path) => path,
    None => "",
};

#[cfg(test)]
mod tests;

pub(crate) fn resolve(name: &str) -> Result<PathBuf, String> {
    let trust: Vec<&Path> = TRUST_ROOTS.iter().copied().map(Path::new).collect();
    let search = search_roots(SEARCH_ROOTS, CONFIGURED_SEARCH_PATH, &trust);
    let search: Vec<&Path> = search.iter().map(PathBuf::as_path).collect();
    resolve_in(name, &search, &trust)
}

/// Like [`resolve`], but also accepts a search-root entry whose target lies
/// outside the trust roots when root owns the target and every directory above
/// it, none writable by group or others, as with packages installed under
/// `/opt` or `/usr/lib`. Such a match returns the target itself, because the
/// links leading to it are not checked. Only integration requirements use this.
pub(crate) fn resolve_installed(name: &str) -> Result<PathBuf, String> {
    let trust: Vec<&Path> = TRUST_ROOTS.iter().copied().map(Path::new).collect();
    let search = search_roots(SEARCH_ROOTS, CONFIGURED_SEARCH_PATH, &trust);
    let search: Vec<&Path> = search.iter().map(PathBuf::as_path).collect();
    resolve_in(name, &search, &trust).or_else(|error| {
        resolve_owned_in(name, &search, Path::new("/"), |metadata| {
            metadata.uid() == 0
        })
        .ok_or(error)
    })
}

fn resolve_owned_in(
    name: &str,
    search_roots: &[&Path],
    top: &Path,
    trusted_owner: impl Fn(&Metadata) -> bool,
) -> Option<PathBuf> {
    if !is_single_basename(name) {
        return None;
    }
    search_roots
        .iter()
        .map(|dir| dir.join(name))
        .filter(|candidate| candidate.is_file())
        .filter_map(|candidate| candidate.canonicalize().ok())
        .find(|target| owned_up_to(target, top, &trusted_owner))
}

fn owned_up_to(path: &Path, top: &Path, trusted_owner: impl Fn(&Metadata) -> bool) -> bool {
    path.starts_with(top)
        && path
            .ancestors()
            .take_while(|ancestor| ancestor.starts_with(top))
            .all(|ancestor| {
                fs::metadata(ancestor)
                    .is_ok_and(|metadata| trusted_owner(&metadata) && metadata.mode() & 0o022 == 0)
            })
}

/// Fixed roots keep priority so NixOS wrappers still win. A configured directory
/// is searched only when it exists and canonicalizes under a trust root, which
/// keeps user-writable locations out even if a build embeds one.
fn search_roots(fixed: &[&str], configured: &str, trust_roots: &[&Path]) -> Vec<PathBuf> {
    fixed
        .iter()
        .map(PathBuf::from)
        .chain(
            configured
                .split(':')
                .map(Path::new)
                .filter(|directory| directory.is_absolute())
                .filter(|directory| {
                    directory
                        .canonicalize()
                        .is_ok_and(|canonical| sits_under(&canonical, trust_roots))
                })
                .map(Path::to_path_buf),
        )
        .collect()
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
            // Preserve argv0 for multicall binaries and profile wrappers.
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
