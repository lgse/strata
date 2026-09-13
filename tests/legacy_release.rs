// SPDX-License-Identifier: MIT
//! Native release gate for the two published single-file installer families.
//! Only network transport and desktop/portal callbacks are substituted. The
//! extraction, selection, staging, permission and replacement routines are frozen.
#![allow(
    clippy::unwrap_used,
    reason = "release fixture failures must abort the gate"
)]
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Sender};

#[derive(Debug)]
enum UpdateInstall {
    Verifying,
    Installing,
}

fn download_to_file(
    url: &str,
    destination: &Path,
    _progress: &Sender<UpdateInstall>,
) -> Result<(), String> {
    fs::copy(url, destination)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn verify_checksum(url: &str, archive: &Path) -> Result<(), String> {
    let checksum =
        fs::read_to_string(format!("{url}.sha256")).map_err(|error| error.to_string())?;
    let bytes = fs::read(archive).map_err(|error| error.to_string())?;
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if checksum.split_whitespace().next() == Some(actual.as_str()) {
        Ok(())
    } else {
        Err("archive checksum mismatch".into())
    }
}

mod portal_setup {
    pub fn refresh_after_in_place_update() -> Result<(), String> {
        Ok(())
    }
}

mod v04 {
    use super::*;
    include!("fixtures/legacy_update/v0_4_0.rs");
    pub fn install(archive: &str, work: &Path, bin: &Path) {
        let (sender, _receiver) = mpsc::channel();
        let executable = fs::canonicalize(bin.join("strata")).unwrap();
        try_install(
            archive,
            work,
            executable.parent().unwrap(),
            &executable,
            &sender,
        )
        .unwrap();
    }
}

mod v016 {
    use super::*;
    include!("fixtures/legacy_update/v0_16_0.rs");
    fn refresh_desktop_metadata(_package: &Path, _executable: &Path, _data: &Path) {}
    pub fn install(archive: &str, work: &Path, bin: &Path) {
        let (sender, _receiver) = mpsc::channel();
        let executable = fs::canonicalize(bin.join("strata")).unwrap();
        try_install(
            archive,
            work,
            executable.parent().unwrap(),
            &executable,
            &sender,
        )
        .unwrap();
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn fixture_archive(directory: &Path, version: &str, bundled: bool) -> (PathBuf, Vec<u8>) {
    // Use a real ELF without executing it: only the frozen updater routines run here.
    let mut executable = fs::read(std::env::current_exe().unwrap()).unwrap();
    executable.extend_from_slice(version.as_bytes());
    let mut files = BTreeMap::from([("strata", executable.clone())]);
    if bundled {
        files.insert("strata-media-helper", executable.clone());
        files.insert("bundle.json", serde_json::to_vec(&serde_json::json!({
            "format": 1, "release_tag": format!("v{version}"), "target": fixture_target(),
            "source_commit": "a".repeat(40), "media_protocol": 1,
            "files": files.iter().map(|(name, bytes)| (*name, digest(bytes))).collect::<BTreeMap<_, _>>(),
        })).unwrap());
    }
    let path = directory.join(format!("{version}.tar.gz"));
    let encoder = flate2::write::GzEncoder::new(
        fs::File::create(&path).unwrap(),
        flate2::Compression::fast(),
    );
    let mut archive = tar::Builder::new(encoder);
    for (name, bytes) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(
                &mut header,
                format!("strata-{version}-{}/{name}", fixture_target()),
                bytes.as_slice(),
            )
            .unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();
    fs::write(
        format!("{}.sha256", path.display()),
        digest(&fs::read(&path).unwrap()),
    )
    .unwrap();
    (path, executable)
}

fn fixture_target() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

fn install_bundle(archive: &Path, version: &str, launcher: &Path) -> PathBuf {
    let result = Command::new("/bin/bash")
        .args([
            "-c",
            "source \"$1\"; install_bundle \"$2\" \"$3\" \"$4\" \"$5\"",
            "fixture",
        ])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh"))
        .arg(archive)
        .arg(version)
        .arg(fixture_target())
        .arg(launcher)
        .env("STRATA_INSTALLER_TESTING", "1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    PathBuf::from(String::from_utf8(result.stdout).unwrap().trim())
}

#[test]
fn legacy_updaters_round_trip_without_mutating_cached_releases() {
    for (version, install) in [
        ("0.4.0", v04::install as fn(&str, &Path, &Path)),
        ("0.16.0", v016::install),
    ] {
        let fixture = tempfile::tempdir().unwrap();
        let (modern, modern_bytes) = fixture_archive(fixture.path(), "1.2.3", true);
        let (legacy, legacy_bytes) = fixture_archive(fixture.path(), version, false);
        let bin = fixture.path().join("bin");
        let launcher = bin.join("strata");
        let modern_cache = install_bundle(&modern, "1.2.3", &launcher);
        let legacy_cache = install_bundle(&legacy, version, &launcher);
        assert!(
            !launcher.is_symlink(),
            "legacy updaters require a flat launcher"
        );
        assert_eq!(fs::read(&launcher).unwrap(), legacy_bytes);
        let work = tempfile::tempdir_in(&bin).unwrap();
        install(modern.to_str().unwrap(), work.path(), &bin);
        drop(work);
        assert_eq!(fs::read(&launcher).unwrap(), modern_bytes);
        assert!(!bin.join("strata-media-helper").exists());
        assert_eq!(fs::read(legacy_cache.join("strata")).unwrap(), legacy_bytes);
        assert_eq!(fs::read(modern_cache.join("strata")).unwrap(), modern_bytes);
        assert_eq!(install_bundle(&modern, "1.2.3", &launcher), modern_cache);
        assert!(launcher.is_symlink());
        assert_eq!(install_bundle(&legacy, version, &launcher), legacy_cache);
        assert!(!launcher.is_symlink());
        assert_eq!(fs::read(&launcher).unwrap(), legacy_bytes);
    }
}

#[test]
#[ignore = "requires locally built native release archives; scripts/release-gate.sh runs it"]
fn published_installers_select_only_the_ui_from_a_real_two_binary_release() {
    let archive = std::env::var("STRATA_LEGACY_ARCHIVE").expect("native release archive");
    let previous =
        PathBuf::from(std::env::var_os("STRATA_LEGACY_PREVIOUS").expect("previous executable"));
    let output = PathBuf::from(
        std::env::var_os("STRATA_LEGACY_OUTPUT").expect("private evidence directory"),
    );
    let expected = fs::read(std::env::var_os("STRATA_LEGACY_UI").expect("finalized UI")).unwrap();
    for (name, install) in [
        ("v0.4.0", v04::install as fn(&str, &Path, &Path)),
        ("v0.16.0", v016::install),
    ] {
        let bin = output.join(name);
        fs::create_dir_all(&bin).unwrap();
        fs::copy(&previous, bin.join("strata")).unwrap();
        let work = tempfile::tempdir_in(&bin).unwrap();
        install(&archive, work.path(), &bin);
        drop(work);
        assert!(
            fs::read(bin.join("strata")).unwrap() == expected,
            "installed bytes must match the finalized UI"
        );
        assert!(!bin.join("strata-media-helper").exists());
        assert_eq!(
            fs::read_dir(&bin).unwrap().count(),
            1,
            "no retained extraction sidecar"
        );
    }
}
