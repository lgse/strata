use std::{
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo::rerun-if-changed=data/locales");
    write_translation_catalogs();
    println!("cargo::rerun-if-changed=integrations");
    write_integration_bundle();
    glib_build_tools::compile_resources(&["data"], "data/strata.gresource.xml", "strata.gresource");

    println!("cargo::rerun-if-env-changed=STRATA_TRUSTED_COMMAND_PATH");
    validate_trusted_command_path();

    println!("cargo::rustc-check-cfg=cfg(strata_updater_disabled)");
    println!("cargo::rerun-if-env-changed=STRATA_UPDATER");
    configure_updater();

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

/// Extra host helper directories are embedded verbatim, so a relative or empty
/// entry would be an unsearchable mistake; reject it where the packager sees it.
fn validate_trusted_command_path() {
    let Some(value) = env::var_os("STRATA_TRUSTED_COMMAND_PATH") else {
        return;
    };
    let value = value
        .to_str()
        .expect("STRATA_TRUSTED_COMMAND_PATH must be valid UTF-8");
    for directory in value.split(':') {
        assert!(
            Path::new(directory).is_absolute(),
            "STRATA_TRUSTED_COMMAND_PATH entry {directory:?} must be an absolute directory"
        );
    }
}

/// Embeds the catalogs as static tables. rust-i18n's own `i18n!` loader emits one
/// statement per message, and that unoptimized frame overflows 2 MiB thread stacks.
fn write_translation_catalogs() {
    let catalogs = rust_i18n_support::load_locales("data/locales", |_| false);
    // load_locales silently returns nothing when the catalog directory is absent.
    assert!(
        catalogs.contains_key("en"),
        "translation catalogs are missing from data/locales"
    );
    let mut source = String::from("pub(crate) static CATALOGS: &[(&str, &[(&str, &str)])] = &[\n");
    for (locale, messages) in &catalogs {
        writeln!(source, "({locale:?}, &[").expect("format catalog");
        for (key, text) in messages {
            writeln!(source, "({key:?}, {text:?}),").expect("format catalog");
        }
        source.push_str("]),\n");
    }
    source.push_str("];\n");
    let output = Path::new(&env::var_os("OUT_DIR").expect("OUT_DIR")).join("catalogs.rs");
    fs::write(output, source).expect("write translation catalogs");
}

/// Embeds every integration under `integrations/` so releases carry the catalog
/// without packaging changes. Directories starting with `.` or `_` and
/// documentation files are left out.
fn write_integration_bundle() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("integrations");
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
