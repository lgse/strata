use std::{env, fs, path::Path, process::Command};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    println!("cargo::rerun-if-changed={}", root.join("Cargo.toml").display());
    for name in ["STRATA_RELEASE_TAG", "STRATA_BUILD_COMMIT"] {
        println!("cargo::rerun-if-env-changed={name}");
    }
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("workspace manifest");
    let version = manifest.lines().find_map(|line| line.strip_prefix("version = \"").and_then(|value| value.strip_suffix('"'))).expect("Strata version");
    let release = env::var("STRATA_RELEASE_TAG").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| format!("v{version}"));
    let git = |args: &[&str]| Command::new("git").current_dir(&root).args(args).output().ok().filter(|o| o.status.success()).and_then(|o| String::from_utf8(o.stdout).ok()).map(|s| s.trim().to_owned());
    let commit = env::var("STRATA_BUILD_COMMIT").ok().filter(|v| !v.is_empty()).or_else(|| git(&["rev-parse", "--short=12", "HEAD"])).unwrap_or_else(|| "unknown".into());
    for entry in [Some("HEAD".to_owned()), Some("packed-refs".to_owned()), git(&["symbolic-ref", "-q", "HEAD"])] .into_iter().flatten() {
        if let Some(path) = git(&["rev-parse", "--git-path", &entry]) {
            let path = root.join(path);
            if path.exists() { println!("cargo::rerun-if-changed={}", path.display()); }
        }
    }
    println!("cargo::rustc-env=STRATA_RELEASE_TAG={release}");
    println!("cargo::rustc-env=STRATA_BUILD_COMMIT={commit}");
}
