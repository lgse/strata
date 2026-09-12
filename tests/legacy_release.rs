// SPDX-License-Identifier: MIT
//! Native release gate for the two published single-file installer families.
//! Only network transport and desktop/portal callbacks are substituted. The
//! extraction, selection, staging, permission and replacement routines are frozen.
#![allow(
    clippy::unwrap_used,
    reason = "release fixture failures must abort the gate"
)]
use sha2::{Digest, Sha256};
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
        try_install(archive, work, bin, &bin.join("strata"), &sender).unwrap();
    }
}

mod v016 {
    use super::*;
    include!("fixtures/legacy_update/v0_16_0.rs");
    fn refresh_desktop_metadata(_package: &Path, _executable: &Path, _data: &Path) {}
    pub fn install(archive: &str, work: &Path, bin: &Path) {
        let (sender, _receiver) = mpsc::channel();
        try_install(archive, work, bin, &bin.join("strata"), &sender).unwrap();
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
