// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    fs,
    os::unix::fs::symlink,
    path::Path,
    sync::{Arc, Barrier},
    thread,
};

use flate2::{Compression, write::GzEncoder};
use serde_json::json;

use super::{
    ExpectedBundle, acquire_lock, build_target, expected_bundle_from_url, extract_and_verify,
    install_archive, sha256_file, verify_elf,
};

#[test]
#[ignore = "requires locally built native release archives; scripts/release-gate.sh runs it"]
fn native_release_archives_migrate_rollback_and_restore_the_matching_pair() {
    let current =
        std::path::PathBuf::from(std::env::var_os("STRATA_LEGACY_UI").expect("finalized UI"));
    let previous =
        std::path::PathBuf::from(std::env::var_os("STRATA_LEGACY_PREVIOUS").expect("previous UI"));
    let bin = std::path::PathBuf::from(
        std::env::var_os("STRATA_RUST_OUTPUT").expect("private installation directory"),
    );
    fs::create_dir_all(&bin).expect("create installation directory");
    let launcher = bin.join("strata");
    fs::copy(&previous, &launcher).expect("install legacy executable");
    let mut running = launcher.clone();
    let mut pinned_versions = Vec::new();
    for executable in [&current, &previous, &current] {
        let package = executable.parent().expect("package directory");
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(package.join("bundle.json")).expect("read release manifest"),
        )
        .expect("parse release manifest");
        let tag = manifest["release_tag"].as_str().expect("release tag");
        let package_name = package
            .file_name()
            .expect("package name")
            .to_str()
            .expect("UTF-8 package name");
        let archive_name = format!("{package_name}.tar.gz");
        let archive = package.with_file_name(&archive_name);
        let expected = expected_bundle_from_url(&format!(
            "https://github.com/LGSE/strata/releases/download/{tag}/{archive_name}",
        ))
        .expect("trusted release identity");
        let installed = install_archive(
            &archive,
            &sha256_file(&archive).expect("archive digest"),
            &expected,
            &bin,
            &running,
        )
        .expect("activate release");
        for name in ["strata", "strata-media-helper"] {
            assert_eq!(
                sha256_file(&installed.join(name)).expect("installed digest"),
                sha256_file(&package.join(name)).expect("packaged digest")
            );
        }
        running = installed.join("strata");
        assert_eq!(
            fs::canonicalize(&launcher).expect("active launcher"),
            running
        );
        pinned_versions.push(running.clone());
        assert!(pinned_versions.iter().all(|path| path.is_file()));
    }
}

fn expected() -> ExpectedBundle {
    let target = build_target().to_owned();
    ExpectedBundle {
        release_tag: "v1.2.3".to_owned(),
        version: "1.2.3".to_owned(),
        top_directory: format!("strata-1.2.3-{target}"),
        target,
    }
}

fn elf(machine: u16) -> Vec<u8> {
    let mut bytes = vec![0_u8; 120];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[16..18].copy_from_slice(&2_u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&machine.to_le_bytes());
    bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
    bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
    bytes[52..54].copy_from_slice(&64_u16.to_le_bytes());
    bytes[54..56].copy_from_slice(&56_u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&1_u16.to_le_bytes());
    bytes[64..68].copy_from_slice(&3_u32.to_le_bytes());
    bytes[72..80].copy_from_slice(&120_u64.to_le_bytes());
    bytes[96..104].copy_from_slice(&0_u64.to_le_bytes());
    bytes[104..112].copy_from_slice(&0_u64.to_le_bytes());
    bytes
}

fn host_machine() -> u16 {
    if build_target().starts_with("x86_64-") {
        62
    } else {
        183
    }
}

fn hash_bytes(bytes: &[u8], dir: &Path, name: &str) -> String {
    let path = dir.join(name);
    fs::write(&path, bytes).expect("write hash input");
    sha256_file(&path).expect("hash input")
}

fn archive(
    files: &[(&str, Vec<u8>)],
    expected: &ExpectedBundle,
    manifest_files: Option<HashMap<String, String>>,
) -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (name, contents) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                format!("{}/{name}", expected.top_directory),
                &contents[..],
            )
            .expect("append file");
    }
    if let Some(hashes) = manifest_files {
        let manifest = serde_json::to_vec(&json!({
            "format": 1,
            "release_tag": expected.release_tag,
            "target": expected.target,
            "source_commit": "0123456789abcdef0123456789abcdef01234567",
            "media_protocol": 1,
            "files": hashes,
        }))
        .expect("manifest json");
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                format!("{}/bundle.json", expected.top_directory),
                &manifest[..],
            )
            .expect("append manifest");
    }
    builder.finish().expect("finish archive");
    builder
        .into_inner()
        .expect("take encoder")
        .finish()
        .expect("compress archive")
}

fn archive_with_manifest(
    files: &[(&str, Vec<u8>)],
    expected: &ExpectedBundle,
    manifest: &[u8],
) -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (name, contents) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                format!("{}/{name}", expected.top_directory),
                &contents[..],
            )
            .expect("append file");
    }
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(
            &mut header,
            format!("{}/bundle.json", expected.top_directory),
            manifest,
        )
        .expect("append manifest");
    builder.finish().expect("finish archive");
    builder
        .into_inner()
        .expect("take encoder")
        .finish()
        .expect("compress archive")
}

fn valid_archive(dir: &Path, expected: &ExpectedBundle) -> Vec<u8> {
    let binary = elf(host_machine());
    let helper = elf(host_machine());
    let hashes = HashMap::from([
        ("strata".to_owned(), hash_bytes(&binary, dir, "hash-strata")),
        (
            "strata-media-helper".to_owned(),
            hash_bytes(&helper, dir, "hash-helper"),
        ),
    ]);
    archive(
        &[("strata", binary), ("strata-media-helper", helper)],
        expected,
        Some(hashes),
    )
}

#[test]
fn rollback_to_audited_binary_only_releases_preserves_bundles_and_rechecks_cached_bytes() {
    let dir = tempfile::tempdir().expect("fixture");
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).expect("bin");
    let launcher = bin.join("strata");
    fs::write(&launcher, b"initial executable").expect("initial");
    let modern = expected();
    let archive_path = dir.path().join("modern.tar.gz");
    fs::write(&archive_path, valid_archive(dir.path(), &modern)).expect("modern archive");
    let modern_hash = sha256_file(&archive_path).expect("modern digest");
    let installed = install_archive(&archive_path, &modern_hash, &modern, &bin, &launcher)
        .expect("modern install");
    let legacy = expected_bundle_from_url(&format!(
        "https://github.com/lgse/strata/releases/download/v0.16.0/strata-0.16.0-{}.tar.gz",
        build_target()
    ))
    .expect("audited URL");
    let legacy_archive = dir.path().join("legacy.tar.gz");
    fs::write(
        &legacy_archive,
        archive(&[("strata", elf(host_machine()))], &legacy, None),
    )
    .expect("legacy archive");
    let legacy_hash = sha256_file(&legacy_archive).expect("legacy digest");
    let rolled_back = install_archive(
        &legacy_archive,
        &legacy_hash,
        &legacy,
        &bin,
        &installed.join("strata"),
    )
    .expect("legacy rollback");
    assert_eq!(
        fs::canonicalize(&launcher).expect("old activated"),
        rolled_back.join("strata")
    );
    assert!(installed.join("strata-media-helper").is_file());
    assert!(!rolled_back.join("strata-media-helper").exists());
    install_archive(
        &archive_path,
        &modern_hash,
        &modern,
        &bin,
        &rolled_back.join("strata"),
    )
    .expect("restore pair");
    let mut damaged = elf(host_machine());
    damaged[119] = 1;
    fs::write(rolled_back.join("strata"), damaged).expect("corrupt retained release");
    assert!(
        install_archive(
            &legacy_archive,
            &legacy_hash,
            &legacy,
            &bin,
            &installed.join("strata")
        )
        .is_err()
    );
    assert_eq!(
        fs::canonicalize(&launcher).expect("pair still active"),
        installed.join("strata")
    );
}

#[test]
fn only_audited_published_tags_accept_a_manifestless_archive() {
    let dir = tempfile::tempdir().expect("fixture");
    for tag in include_str!("legacy-releases.txt")
        .lines()
        .chain(["v1.2.3", "v0.16.0-rc.99"])
    {
        let version = tag.trim_start_matches('v');
        let expected = expected_bundle_from_url(&format!(
            "https://github.com/lgse/strata/releases/download/{tag}/strata-{version}-{}.tar.gz",
            build_target()
        ))
        .expect("valid URL");
        let path = dir.path().join(format!("{tag}.tar.gz"));
        fs::write(
            &path,
            archive(&[("strata", elf(host_machine()))], &expected, None),
        )
        .expect("archive");
        assert_eq!(
            extract_and_verify(&path, &dir.path().join(tag), &expected).is_ok(),
            tag != "v1.2.3" && tag != "v0.16.0-rc.99",
            "{tag}"
        );
    }
}

#[test]
fn release_url_is_the_only_source_of_expected_identity() {
    let target = build_target();
    let parsed = expected_bundle_from_url(&format!(
        "https://github.com/LGSE/strata/releases/download/v1.2.3/strata-1.2.3-{target}.tar.gz"
    ))
    .expect("trusted URL");
    assert_eq!(parsed, expected());
    for url in [
        format!("https://example.test/releases/download/v1.2.3/strata-1.2.3-{target}.tar.gz"),
        format!(
            "https://github.com/lgse/strata/releases/download/v1.2.3/../strata-1.2.3-{target}.tar.gz"
        ),
        format!(
            "https://github.com/lgse/strata/releases/download/v9.9.9/strata-1.2.3-{target}.tar.gz"
        ),
        format!(
            "https://github.com/lgse/strata/releases/download/v１.2.3/strata-１.2.3-{target}.tar.gz"
        ),
        format!(
            "https://github.com/lgse/strata/releases/download/not-semver/strata-not-semver-{target}.tar.gz"
        ),
    ] {
        assert!(expected_bundle_from_url(&url).is_err(), "accepted {url}");
    }
}

#[test]
fn archive_rejects_traversal_symlink_and_duplicate_entries() {
    let expected = expected();
    let dir = tempfile::tempdir().expect("tempdir");
    let archive_path = dir.path().join("bad.tar.gz");

    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let mut traversal = tar::Header::new_gnu();
    let unsafe_name = format!("{}/../escaped", expected.top_directory);
    traversal.as_mut_bytes()[..unsafe_name.len()].copy_from_slice(unsafe_name.as_bytes());
    traversal.set_size(1);
    traversal.set_mode(0o644);
    traversal.set_cksum();
    builder
        .append(&traversal, &b"x"[..])
        .expect("append traversal");
    builder.finish().expect("finish");
    fs::write(
        &archive_path,
        builder
            .into_inner()
            .expect("encoder")
            .finish()
            .expect("gzip"),
    )
    .expect("archive");
    assert!(
        extract_and_verify(&archive_path, &dir.path().join("out-traversal"), &expected)
            .expect_err("traversal rejected")
            .contains("unsafe path")
    );

    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let mut link = tar::Header::new_gnu();
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_size(0);
    link.set_mode(0o777);
    link.set_link_name("elsewhere").expect("link name");
    link.set_cksum();
    builder
        .append_data(
            &mut link,
            format!("{}/strata", expected.top_directory),
            &[][..],
        )
        .expect("append link");
    builder.finish().expect("finish");
    fs::write(
        &archive_path,
        builder
            .into_inner()
            .expect("encoder")
            .finish()
            .expect("gzip"),
    )
    .expect("archive");
    assert!(
        extract_and_verify(&archive_path, &dir.path().join("out-link"), &expected)
            .expect_err("symlink rejected")
            .contains("link or non-regular")
    );

    let duplicate = archive(&[("strata", vec![1]), ("strata", vec![2])], &expected, None);
    fs::write(&archive_path, duplicate).expect("duplicate archive");
    assert!(
        extract_and_verify(&archive_path, &dir.path().join("out-duplicate"), &expected)
            .expect_err("duplicate rejected")
            .contains("duplicate")
    );
}

#[test]
fn accepts_the_real_dynamically_linked_test_executable() {
    verify_elf(
        &std::env::current_exe().expect("test executable"),
        build_target(),
    )
    .expect("a dynamically linked host ELF is valid");
}

#[test]
fn archive_rejects_missing_helper_and_wrong_elf_architecture() {
    let expected = expected();
    let dir = tempfile::tempdir().expect("tempdir");
    let binary = elf(host_machine());
    let hashes = HashMap::from([("strata".to_owned(), hash_bytes(&binary, dir.path(), "one"))]);
    let path = dir.path().join("archive.tar.gz");
    fs::write(
        &path,
        archive(&[("strata", binary)], &expected, Some(hashes)),
    )
    .expect("archive");
    assert!(
        extract_and_verify(&path, &dir.path().join("missing"), &expected)
            .expect_err("missing helper rejected")
            .contains("required binary")
    );

    let wrong = elf(if host_machine() == 62 { 183 } else { 62 });
    let helper = elf(host_machine());
    let hashes = HashMap::from([
        ("strata".to_owned(), hash_bytes(&wrong, dir.path(), "wrong")),
        (
            "strata-media-helper".to_owned(),
            hash_bytes(&helper, dir.path(), "helper"),
        ),
    ]);
    fs::write(
        &path,
        archive(
            &[("strata", wrong), ("strata-media-helper", helper)],
            &expected,
            Some(hashes),
        ),
    )
    .expect("archive");
    assert!(
        extract_and_verify(&path, &dir.path().join("wrong-arch"), &expected)
            .expect_err("wrong architecture rejected")
            .contains("wrong ELF architecture")
    );
}

#[test]
fn archive_rejects_duplicate_manifest_keys_and_bad_gzip_endings() {
    let expected = expected();
    let dir = tempfile::tempdir().expect("tempdir");
    let binary = elf(host_machine());
    let hash = hash_bytes(&binary, dir.path(), "binary-hash");
    let manifest = format!(
        r#"{{"format":1,"release_tag":"v1.2.3","target":"{}","source_commit":"0123456789abcdef0123456789abcdef01234567","media_protocol":1,"files":{{"strata":"{hash}","strata":"{hash}"}}}}"#,
        expected.target
    );
    let path = dir.path().join("duplicate-manifest.tar.gz");
    fs::write(
        &path,
        archive_with_manifest(&[("strata", binary)], &expected, manifest.as_bytes()),
    )
    .expect("archive");
    assert!(
        extract_and_verify(&path, &dir.path().join("duplicate-manifest"), &expected)
            .expect_err("duplicate manifest rejected")
            .contains("duplicate bundle manifest path")
    );

    let mut trailing = valid_archive(dir.path(), &expected);
    trailing.extend_from_slice(b"untrusted trailing bytes");
    fs::write(&path, trailing).expect("trailing archive");
    assert!(
        extract_and_verify(&path, &dir.path().join("trailing"), &expected)
            .expect_err("trailing bytes rejected")
            .contains("trailing data")
    );

    let mut truncated = valid_archive(dir.path(), &expected);
    truncated.truncate(truncated.len() - 4);
    fs::write(&path, truncated).expect("truncated archive");
    assert!(extract_and_verify(&path, &dir.path().join("truncated"), &expected).is_err());
}

#[test]
fn finished_transactions_release_the_lock_despite_a_fork_style_descriptor_duplicate() {
    let dir = tempfile::tempdir().expect("fixture");
    let lock = acquire_lock(dir.path()).expect("installer lock");
    let _inherited = lock.file.try_clone().expect("fork-style duplicate");
    assert!(acquire_lock(dir.path()).is_err());
    drop(lock);
    assert!(acquire_lock(dir.path()).is_ok());
}

#[test]
fn process_lock_rejects_a_concurrent_installer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _first = acquire_lock(dir.path()).expect("first lock");
    assert!(
        acquire_lock(dir.path())
            .expect_err("competing lock rejected")
            .contains("already installing")
    );
}

#[test]
fn planted_storage_and_lock_symlinks_are_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let attacker = dir.path().join("attacker");
    fs::create_dir(&attacker).expect("attacker dir");
    symlink(&attacker, dir.path().join("root-link")).expect("root symlink");
    assert!(acquire_lock(&dir.path().join("root-link")).is_err());

    let root = dir.path().join("root");
    fs::create_dir(&root).expect("root");
    fs::write(attacker.join("lock"), b"").expect("attacker lock");
    symlink(attacker.join("lock"), root.join("install.lock")).expect("lock symlink");
    assert!(acquire_lock(&root).is_err());
}

#[test]
fn concurrent_install_archive_is_rejected_by_the_process_lock() {
    let expected = expected();
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).expect("bin");
    let launcher = bin.join("strata");
    fs::write(&launcher, b"old executable").expect("old executable");
    let archive_path = dir.path().join("valid.tar.gz");
    fs::write(&archive_path, valid_archive(dir.path(), &expected)).expect("archive");
    let hash = sha256_file(&archive_path).expect("hash");
    let root = bin.join(".strata-bundles");
    let barrier = Arc::new(Barrier::new(2));
    let child_barrier = Arc::clone(&barrier);
    let holder = thread::spawn(move || {
        let _lock = acquire_lock(&root).expect("holder lock");
        child_barrier.wait();
        child_barrier.wait();
    });
    barrier.wait();
    let error = install_archive(&archive_path, &hash, &expected, &bin, &launcher)
        .expect_err("concurrent install rejected");
    barrier.wait();
    holder.join().expect("holder");
    assert!(error.contains("already installing"));
    assert_eq!(fs::read(launcher).expect("old launcher"), b"old executable");
}

#[test]
fn failed_verification_keeps_the_old_launcher_usable() {
    let expected = expected();
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).expect("bin");
    let launcher = bin.join("strata");
    fs::write(&launcher, b"old executable").expect("old executable");
    let archive_path = dir.path().join("invalid.tar.gz");
    fs::write(&archive_path, archive(&[], &expected, Some(HashMap::new()))).expect("archive");
    let hash = sha256_file(&archive_path).expect("archive hash");

    assert!(install_archive(&archive_path, &hash, &expected, &bin, &launcher).is_err());
    assert_eq!(
        fs::read(&launcher).expect("old executable remains"),
        b"old executable"
    );
    assert!(!bin.join(".strata-bundles/current").exists());
}

#[test]
fn activation_failure_keeps_the_old_launcher_and_does_not_claim_success() {
    let expected = expected();
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).expect("bin");
    let launcher = bin.join("strata");
    fs::write(&launcher, b"old executable").expect("old executable");
    let root = bin.join(".strata-bundles");
    fs::create_dir_all(root.join("current/blocker")).expect("planted current directory");
    let archive_path = dir.path().join("valid.tar.gz");
    fs::write(&archive_path, valid_archive(dir.path(), &expected)).expect("archive");
    let hash = sha256_file(&archive_path).expect("hash");

    let error = install_archive(&archive_path, &hash, &expected, &bin, &launcher)
        .expect_err("invalid pointer rejected");
    assert!(error.contains("current bundle pointer"));
    assert_eq!(
        fs::read(&launcher).expect("old launcher"),
        b"old executable"
    );
}

#[test]
fn first_bundle_install_atomically_migrates_the_stable_launcher() {
    let expected = expected();
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).expect("bin");
    let launcher = bin.join("strata");
    fs::write(&launcher, b"running legacy").expect("legacy executable");
    let archive_path = dir.path().join("valid.tar.gz");
    fs::write(&archive_path, valid_archive(dir.path(), &expected)).expect("archive");
    let hash = sha256_file(&archive_path).expect("archive hash");

    install_archive(&archive_path, &hash, &expected, &bin, &launcher).expect("install");

    assert_eq!(
        fs::read_link(&launcher).expect("stable launcher"),
        Path::new(".strata-bundles/current/strata")
    );
    assert_eq!(
        fs::read_link(bin.join(".strata-bundles/current")).expect("current"),
        Path::new("versions").join(hash)
    );
    let previous =
        fs::read_link(bin.join(".strata-bundles/previous")).expect("legacy rollback pointer");
    assert!(previous.to_string_lossy().starts_with("versions/legacy-"));
    assert_eq!(
        fs::read(bin.join(".strata-bundles").join(previous).join("strata"))
            .expect("preserved legacy"),
        b"running legacy"
    );
}

#[test]
fn updating_from_an_immutable_version_switches_the_pair_and_preserves_old() {
    let expected = expected();
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = dir.path().join("bin");
    let root = bin.join(".strata-bundles");
    let old = root.join("versions/old");
    fs::create_dir_all(&old).expect("old version");
    fs::write(old.join("strata"), b"running old").expect("old strata");
    fs::write(old.join("strata-media-helper"), b"old helper").expect("old helper");
    symlink("versions/old", root.join("current")).expect("current old");
    symlink(".strata-bundles/current/strata", bin.join("strata")).expect("launcher");
    let archive_path = dir.path().join("valid.tar.gz");
    fs::write(&archive_path, valid_archive(dir.path(), &expected)).expect("archive");
    let hash = sha256_file(&archive_path).expect("archive hash");

    let installed = install_archive(&archive_path, &hash, &expected, &bin, &old.join("strata"))
        .expect("install");

    assert_eq!(
        fs::read(old.join("strata")).expect("running old remains"),
        b"running old"
    );
    assert_eq!(
        fs::read_link(root.join("current")).expect("current target"),
        Path::new("versions").join(&hash)
    );
    assert_eq!(
        fs::read_link(root.join("previous")).expect("rollback target"),
        Path::new("versions/old")
    );
    assert!(installed.join("strata").is_file());
    assert!(installed.join("strata-media-helper").is_file());
}
