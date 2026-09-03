use std::{env, path::PathBuf, process::Command};

fn main() {
    glib_build_tools::compile_resources(&["data"], "data/strata.gresource.xml", "strata.gresource");

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
        .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
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
    let Ok(manifest) = PathBuf::from(env!("CARGO_MANIFEST_DIR")).canonicalize() else {
        return false;
    };
    root == manifest
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
    command.current_dir(env!("CARGO_MANIFEST_DIR"));
    command
}
