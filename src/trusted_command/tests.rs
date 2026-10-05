// SPDX-License-Identifier: MIT

use std::{fs, path::Path};

use super::{SEARCH_ROOTS, TRUST_ROOTS, command, resolve, resolve_in};

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("scratch directory")
}

fn write_helper(dir: &Path, name: &str) {
    fs::create_dir_all(dir).expect("create helper directory");
    fs::write(dir.join(name), b"").expect("write helper");
}

fn resolve_same(name: &str, dirs: &[&Path]) -> Result<std::path::PathBuf, String> {
    resolve_in(name, dirs, dirs)
}

#[test]
fn resolve_uses_only_allowlisted_directories() {
    let root = scratch();
    let first = root.path().join("first");
    let second = root.path().join("second");
    let path_dir = root.path().join("path");
    write_helper(&first, "bwrap");
    write_helper(&path_dir, "bwrap");

    let resolved =
        resolve_same("bwrap", &[first.as_path(), second.as_path()]).expect("found bwrap");

    assert_eq!(resolved, first.join("bwrap"));
    assert!(resolved.is_absolute());
    assert_ne!(resolved, path_dir.join("bwrap"));
}

#[test]
fn resolve_rejects_bad_names_and_misses() {
    let root = scratch();
    let empty = root.path().join("empty");
    fs::create_dir_all(&empty).expect("create empty directory");

    for name in ["", ".", "..", "usr/bin/bwrap", "bwrap"] {
        assert!(
            resolve_same(name, &[empty.as_path()]).is_err(),
            "{name:?} must fail closed"
        );
    }
    assert!(resolve_same("bwrap", &[]).is_err());
}

#[test]
fn resolve_requires_final_path_under_a_trusted_directory() {
    let root = scratch();
    let trusted = root.path().join("trusted");
    let outside = root.path().join("outside");
    write_helper(&trusted, "tar");
    write_helper(&outside, "bwrap");
    std::os::unix::fs::symlink(outside.join("bwrap"), trusted.join("bwrap"))
        .expect("helper symlink");

    let accepted = resolve_same("tar", &[trusted.as_path()]).expect("in-allowlist file");
    assert_eq!(accepted, trusted.join("tar"));
    assert!(resolve_same("bwrap", &[trusted.as_path()]).is_err());
}

#[test]
fn resolve_execs_search_path_when_canonical_target_is_in_store() {
    let root = scratch();
    let profile = root.path().join("sw/bin");
    let store = root.path().join("nix/store/hash-bwrap/bin");
    write_helper(&store, "bwrap");
    fs::create_dir_all(&profile).expect("create profile bin");
    std::os::unix::fs::symlink(store.join("bwrap"), profile.join("bwrap"))
        .expect("profile symlink into store");

    let store_root = root.path().join("nix/store");
    let resolved = resolve_in("bwrap", &[profile.as_path()], &[store_root.as_path()])
        .expect("nix profile helper");

    assert_eq!(resolved, profile.join("bwrap"));
    assert_eq!(
        resolved.file_name().and_then(|name| name.to_str()),
        Some("bwrap")
    );
    assert_ne!(
        resolved,
        store.join("bwrap").canonicalize().expect("store target")
    );
}

#[test]
fn host_helpers_use_absolute_allowlisted_paths() {
    for name in ["sh", "tar"] {
        if let Ok(path) = resolve(name) {
            assert!(path.is_absolute());
            assert_eq!(path.file_name().and_then(|part| part.to_str()), Some(name));
            assert!(
                SEARCH_ROOTS.iter().any(|root| path.starts_with(root)),
                "{name} resolved to {}",
                path.display()
            );
            let canonical = path.canonicalize().expect("canonical helper");
            assert!(
                TRUST_ROOTS.iter().any(|root| {
                    let root = Path::new(root);
                    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
                    canonical.starts_with(root)
                }),
                "{name} canonical {} not under a trust root",
                canonical.display()
            );
            let program = command(name).expect("command").get_program().to_owned();
            assert_eq!(program, path.as_os_str());
        }
    }
    assert!(command("strata-missing-trusted-helper").is_err());
}
