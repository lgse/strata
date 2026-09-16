// SPDX-License-Identifier: MIT

use std::{fs, path::Path};

use super::{command, resolve, resolve_in};

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("scratch directory")
}

fn write_helper(dir: &Path, name: &str) {
    fs::create_dir_all(dir).expect("create helper directory");
    fs::write(dir.join(name), b"").expect("write helper");
}

#[test]
fn resolve_uses_only_allowlisted_directories() {
    let root = scratch();
    let first = root.path().join("first");
    let second = root.path().join("second");
    let path_dir = root.path().join("path");
    write_helper(&first, "bwrap");
    write_helper(&path_dir, "bwrap");

    let resolved = resolve_in("bwrap", &[first.as_path(), second.as_path()]).expect("found bwrap");

    assert_eq!(
        resolved,
        first.join("bwrap").canonicalize().expect("canonical bwrap")
    );
    assert!(resolved.is_absolute());
    assert_ne!(
        resolved,
        path_dir
            .join("bwrap")
            .canonicalize()
            .expect("canonical PATH helper")
    );
}

#[test]
fn resolve_rejects_bad_names_and_misses() {
    let root = scratch();
    let empty = root.path().join("empty");
    fs::create_dir_all(&empty).expect("create empty directory");

    for name in ["", ".", "..", "usr/bin/bwrap", "bwrap"] {
        assert!(
            resolve_in(name, &[empty.as_path()]).is_err(),
            "{name:?} must fail closed"
        );
    }
    assert!(resolve_in("bwrap", &[]).is_err());
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

    let accepted = resolve_in("tar", &[trusted.as_path()]).expect("in-allowlist file");
    assert_eq!(
        accepted,
        trusted.join("tar").canonicalize().expect("canonical tar")
    );
    assert!(resolve_in("bwrap", &[trusted.as_path()]).is_err());
}

#[test]
fn host_helpers_use_absolute_allowlisted_paths() {
    for name in ["sh", "tar"] {
        if let Ok(path) = resolve(name) {
            assert!(path.is_absolute());
            let roots: Vec<_> = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"]
                .into_iter()
                .filter_map(|dir| Path::new(dir).canonicalize().ok())
                .collect();
            assert!(
                roots.iter().any(|root| path.starts_with(root)),
                "{name} resolved to {}",
                path.display()
            );
            let program = command(name).expect("command").get_program().to_owned();
            assert_eq!(program, path.as_os_str());
        }
    }
    assert!(command("strata-missing-trusted-helper").is_err());
}
