// SPDX-License-Identifier: MIT

use std::{
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo::rerun-if-changed=../../integrations");
    write_integration_bundle();

    println!("cargo::rustc-check-cfg=cfg(strata_updater_disabled)");
    println!("cargo::rerun-if-env-changed=STRATA_UPDATER");
    configure_updater();

    println!("cargo::rustc-check-cfg=cfg(strata_integrations_package)");
    println!("cargo::rerun-if-env-changed=STRATA_INTEGRATIONS");
    configure_integrations();

    println!("cargo::rerun-if-changed=../../Cargo.toml");
    let version = application_version();
    println!("cargo::rustc-env=STRATA_APP_VERSION={version}");

    println!("cargo::rerun-if-env-changed=STRATA_BUILD_COMMIT");
    track_git_metadata();

    let commit = env::var("STRATA_BUILD_COMMIT")
        .ok()
        .and_then(|value| value.lines().next().map(str::trim).map(str::to_owned))
        .filter(|value| !value.is_empty())
        .or_else(git_commit)
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo::rustc-env=STRATA_BUILD_COMMIT={commit}");

    println!("cargo::rerun-if-env-changed=STRATA_RELEASE_TAG");
    println!("cargo::rerun-if-env-changed=STRATA_BUILD_KIND");

    let release_tag = env::var("STRATA_RELEASE_TAG")
        .ok()
        .and_then(|value| value.lines().next().map(str::trim).map(str::to_owned))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("v{version}"));
    println!("cargo::rustc-env=STRATA_RELEASE_TAG={release_tag}");

    let build_kind = env::var("STRATA_BUILD_KIND")
        .ok()
        .and_then(|value| value.lines().next().map(str::trim).map(str::to_owned))
        .filter(|value| {
            matches!(
                value.as_str(),
                "stable" | "alpha" | "beta" | "rc" | "nightly"
            )
        })
        .unwrap_or_else(|| "stable".to_owned());
    println!("cargo::rustc-env=STRATA_BUILD_KIND={build_kind}");
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The application's version is the root package's, which the release workflow rewrites;
/// this crate's own version says nothing about a release.
fn application_version() -> String {
    let manifest =
        fs::read_to_string(workspace_root().join("Cargo.toml")).expect("read the root manifest");
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[package]")
        .skip(1)
        .take_while(|line| !line.starts_with('['))
        .find_map(|line| {
            let value = line
                .strip_prefix("version")?
                .trim_start()
                .strip_prefix('=')?;
            Some(value.trim().trim_matches('"').to_owned())
        })
        .expect("the root package declares its version")
}

/// Distribution packages set `STRATA_UPDATER=disabled` so their package manager
/// owns updates. Any other value is rejected rather than shipping an updater by accident.
fn configure_updater() {
    let Some(value) = env::var_os("STRATA_UPDATER") else {
        return;
    };
    match value.to_str().map(str::trim) {
        Some("" | "enabled") => {}
        Some("disabled") => println!("cargo::rustc-cfg=strata_updater_disabled"),
        _ => panic!("STRATA_UPDATER must be \"enabled\" or \"disabled\", not {value:?}"),
    }
}

/// Store-based packages (Nix, Guix) set `STRATA_INTEGRATIONS=package` because every
/// path the runtime installers would record changes on update. The package ships the
/// portal, D-Bus service, and udiskie files instead.
fn configure_integrations() {
    let Some(value) = env::var_os("STRATA_INTEGRATIONS") else {
        return;
    };
    match value.to_str().map(str::trim) {
        Some("" | "runtime") => {}
        Some("package") => println!("cargo::rustc-cfg=strata_integrations_package"),
        _ => panic!("STRATA_INTEGRATIONS must be \"runtime\" or \"package\", not {value:?}"),
    }
}

/// Embeds every integration under `integrations/` so releases carry the catalog
/// without packaging changes. Directories starting with `.` or `_` and
/// documentation files are left out.
fn write_integration_bundle() {
    let root = workspace_root().join("integrations");
    let mut directories: Vec<PathBuf> = fs::read_dir(&root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default();
    directories.sort();
    let mut source = String::from("pub(crate) static BUNDLED: &[super::BundledEntry] = &[\n");
    for directory in directories {
        let id = directory
            .file_name()
            .and_then(|name| name.to_str())
            .expect("integration directory name");
        if id.starts_with(['.', '_']) {
            continue;
        }
        let mut files: Vec<PathBuf> = fs::read_dir(&directory)
            .expect("read integration directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect();
        files.sort();
        writeln!(source, "({id:?}, &[").expect("format bundle");
        for file in files {
            let name = file
                .file_name()
                .and_then(|name| name.to_str())
                .expect("integration file name");
            if name.starts_with('.') || name == "README.md" {
                continue;
            }
            writeln!(
                source,
                "({name:?}, include_bytes!({:?})),",
                file.display().to_string()
            )
            .expect("format bundle");
        }
        source.push_str("]),\n");
    }
    source.push_str("];\n");
    let output = Path::new(&env::var_os("OUT_DIR").expect("OUT_DIR")).join("integrations.rs");
    fs::write(output, source).expect("write integration bundle");
}

fn git_commit() -> Option<String> {
    if !is_project_checkout() {
        return None;
    }

    let output = git_command()
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let commit = String::from_utf8(output.stdout).ok()?;
    let commit = commit.trim();
    (!commit.is_empty()).then(|| commit.to_owned())
}

fn track_git_metadata() {
    if !is_project_checkout() {
        return;
    }

    track_git_path("HEAD");
    track_git_path("packed-refs");

    if let Some(reference) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        track_git_path(reference.trim());
    }
}

fn track_git_path(path: &str) {
    if let Some(path) = git_output(&["rev-parse", "--git-path", path]) {
        let path = PathBuf::from(path.trim());
        if path.exists() {
            println!("cargo::rerun-if-changed={}", path.display());
        }
    }
}

fn is_project_checkout() -> bool {
    let Some(root) = git_output(&["rev-parse", "--show-toplevel"]) else {
        return false;
    };
    let Ok(root) = PathBuf::from(root.trim()).canonicalize() else {
        return false;
    };
    let Ok(workspace) = workspace_root().canonicalize() else {
        return false;
    };
    root == workspace
}

fn git_output(arguments: &[&str]) -> Option<String> {
    let output = git_command().args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}

fn git_command() -> Command {
    let mut command = Command::new("git");
    command.current_dir(workspace_root());
    command
}
