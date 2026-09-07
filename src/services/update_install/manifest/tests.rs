// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use ring::signature::KeyPair;
use serde_json::{Value, json};

const FIXTURE: &[u8] =
    include_bytes!("../../../../tests/fixtures/signed-update/strata-update-manifest.json");
const SIGNATURES: &[u8] = include_bytes!(
    "../../../../tests/fixtures/signed-update/strata-update-manifest.signatures.json"
);
const PUBLIC: &str = include_str!("../../../../tests/fixtures/signed-update/public-key.hex");
const CONTENT: &[u8] = b"strata update fixture\n";

fn request() -> InstallRequest {
    let name = crate::services::update_check::archive_name("0.12.0");
    InstallRequest {
        tag: "v0.12.0".to_owned(),
        advertised_url: format!("{}/v0.12.0/{name}", super::super::RELEASE_DOWNLOAD_ROOT),
        asset_name: name,
    }
}

fn signed(bytes: &[u8], seeds: &[u8]) -> Vec<u8> {
    let signatures: Vec<_> = seeds.iter().map(|seed| {
        let key = signature::Ed25519KeyPair::from_seed_unchecked(&[*seed; 32]).expect("test key");
        json!({"key_id": hex::encode(digest::digest(&digest::SHA256, key.public_key().as_ref())),
            "signature": hex::encode(key.sign(&[DOMAIN, bytes].concat()).as_ref())})
    }).collect();
    serde_json::to_vec(&json!({"schema": 1, "signatures": signatures})).expect("signature JSON")
}

fn checked(bytes: &[u8], signatures: &[u8]) -> Result<VerifiedRelease, String> {
    authenticate_with_keys(bytes, signatures, &request(), &[PUBLIC.trim()])
}

#[test]
fn openssl_signed_manifest_is_verified_natively() {
    let release = checked(FIXTURE, SIGNATURES).expect("OpenSSL interoperability");
    assert_eq!(release.archive_size(), CONTENT.len() as u64);
    let directory = tempfile::tempdir().expect("fixture directory");
    let archive = directory.path().join("archive");
    std::fs::write(&archive, CONTENT).expect("archive");
    release
        .verify_archive(&archive, &InstallCancel::new())
        .expect("authenticated hash");
    std::fs::write(
        directory.path().join("SOURCE_COMMIT"),
        format!("{}\n", "a".repeat(40)),
    )
    .expect("source commit");
    release
        .verify_source_commit(directory.path())
        .expect("authenticated source");
}

#[test]
fn modified_bytes_and_wrong_keys_or_signatures_fail_closed() {
    let mut bytes = FIXTURE.to_vec();
    bytes.push(b' ');
    assert!(checked(&bytes, SIGNATURES).is_err());
    assert!(checked(FIXTURE, &signed(FIXTURE, &[8])).is_err());
    assert!(checked(FIXTURE, b"{\"schema\":1,\"signatures\":[]}").is_err());
    let mut envelope: Value = serde_json::from_slice(SIGNATURES).expect("signature fixture");
    envelope["signatures"][0]["signature"] = json!("00".repeat(64));
    assert!(
        checked(
            FIXTURE,
            &serde_json::to_vec(&envelope).expect("corrupt signature")
        )
        .is_err()
    );
    assert!(checked(FIXTURE, b"not JSON").is_err());
    assert!(checked(&vec![0; MAX_MANIFEST_BYTES as usize + 1], SIGNATURES).is_err());
    assert!(checked(FIXTURE, &vec![0; MAX_SIGNATURES_BYTES as usize + 1]).is_err());
}

#[test]
fn rotation_accepts_either_pinned_key_but_never_trusts_an_unsigned_key() {
    let new_key = signature::Ed25519KeyPair::from_seed_unchecked(&[8; 32]).expect("new test key");
    let new_public = hex::encode(new_key.public_key());
    let signatures = signed(FIXTURE, &[7, 8]);
    assert!(checked(FIXTURE, &signatures).is_ok());
    assert!(authenticate_with_keys(FIXTURE, &signatures, &request(), &[&new_public]).is_ok());
    assert!(authenticate_with_keys(FIXTURE, &signatures, &request(), &[]).is_err());
    let mut envelope: Value = serde_json::from_slice(&signed(FIXTURE, &[8])).expect("signature");
    envelope["public_key"] = json!(new_public);
    assert!(
        checked(
            FIXTURE,
            &serde_json::to_vec(&envelope).expect("untrusted key")
        )
        .is_err()
    );
}

#[test]
fn signed_but_invalid_release_identity_is_rejected() {
    for (field, value) in [
        ("schema", json!(2)),
        ("repository", json!("attacker/strata")),
        ("tag", json!("v0.11.2")),
        ("source_commit", json!("main")),
        ("source_commit", json!("A".repeat(40))),
        ("artifacts", json!([])),
    ] {
        let mut manifest: Value = serde_json::from_slice(FIXTURE).expect("manifest fixture");
        manifest[field] = value;
        let bytes = serde_json::to_vec(&manifest).expect("manifest JSON");
        assert!(
            checked(&bytes, &signed(&bytes, &[7])).is_err(),
            "field {field}"
        );
    }
}

#[test]
fn signed_artifacts_must_have_unambiguous_bounded_identity() {
    for (field, value) in [
        ("name", json!("../strata.tar.gz")),
        ("target", json!("wrong-architecture")),
        ("size", json!(0)),
        ("size", json!(MAX_UPDATE_ARCHIVE_BYTES + 1)),
        ("size", json!(-1)),
        ("sha256", json!("abcdef")),
        ("sha256", json!("A".repeat(64))),
    ] {
        let mut manifest: Value = serde_json::from_slice(FIXTURE).expect("manifest fixture");
        manifest["artifacts"][0][field] = value;
        let bytes = serde_json::to_vec(&manifest).expect("manifest JSON");
        assert!(
            checked(&bytes, &signed(&bytes, &[7])).is_err(),
            "field {field}"
        );
    }
    let mut manifest: Value = serde_json::from_slice(FIXTURE).expect("manifest fixture");
    let duplicate = manifest["artifacts"][0].clone();
    manifest["artifacts"]
        .as_array_mut()
        .expect("artifacts")
        .push(duplicate);
    let bytes = serde_json::to_vec(&manifest).expect("manifest JSON");
    assert!(checked(&bytes, &signed(&bytes, &[7])).is_err());
    let mut manifest: Value = serde_json::from_slice(FIXTURE).expect("manifest fixture");
    manifest["artifacts"]
        .as_array_mut()
        .expect("artifacts")
        .retain(|artifact| artifact["name"] != request().asset_name);
    let bytes = serde_json::to_vec(&manifest).expect("manifest JSON");
    assert!(checked(&bytes, &signed(&bytes, &[7])).is_err());
}

#[test]
fn duplicate_json_fields_and_unsigned_format_extensions_are_rejected() {
    let original = std::str::from_utf8(FIXTURE).expect("fixture UTF-8");
    let duplicated = original.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1);
    assert!(checked(duplicated.as_bytes(), &signed(duplicated.as_bytes(), &[7])).is_err());
    let mut manifest: Value = serde_json::from_slice(FIXTURE).expect("manifest fixture");
    manifest["unknown"] = json!(true);
    let bytes = serde_json::to_vec(&manifest).expect("manifest JSON");
    assert!(checked(&bytes, &signed(&bytes, &[7])).is_err());
}

#[test]
fn hash_size_source_mismatches_and_cancellation_preserve_the_fixture() {
    let release = checked(FIXTURE, SIGNATURES).expect("signed manifest");
    let directory = tempfile::tempdir().expect("directory");
    let archive = directory.path().join("archive");
    for bytes in [
        b"different bytes here!\n".as_slice(),
        &CONTENT[..3],
        b"strata update fixture\nextra",
    ] {
        std::fs::write(&archive, bytes).expect("corrupt fixture");
        assert!(
            release
                .verify_archive(&archive, &InstallCancel::new())
                .is_err()
        );
        assert_eq!(std::fs::read(&archive).expect("unchanged archive"), bytes);
    }
    let cancel = InstallCancel::new();
    cancel.cancel();
    assert!(matches!(
        release.verify_archive(&archive, &cancel),
        Err(InstallStop::Cancelled)
    ));
    for contents in [
        "b".repeat(40),
        "a".repeat(40) + &" ".repeat(200),
        String::new(),
    ] {
        std::fs::write(directory.path().join("SOURCE_COMMIT"), contents).expect("source fixture");
        assert!(release.verify_source_commit(directory.path()).is_err());
    }
}

fn packaged_release(directory: &Path, source: &str) -> (std::path::PathBuf, VerifiedRelease) {
    let path = directory.join("release.tar.gz");
    let encoder = flate2::write::GzEncoder::new(
        File::create(&path).expect("archive file"),
        flate2::Compression::fast(),
    );
    let mut builder = tar::Builder::new(encoder);
    let binary = std::fs::read("/bin/true").expect("ELF fixture");
    let package = request().asset_name.trim_end_matches(".tar.gz").to_owned();
    for (name, contents) in [
        ("strata", binary.as_slice()),
        ("SOURCE_COMMIT", source.as_bytes()),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        builder
            .append_data(&mut header, format!("{package}/{name}"), contents)
            .expect("archive entry");
    }
    builder
        .into_inner()
        .expect("tar finish")
        .finish()
        .expect("gzip finish");
    let bytes = std::fs::read(&path).expect("archive bytes");
    let mut manifest: Value = serde_json::from_slice(FIXTURE).expect("manifest fixture");
    for artifact in manifest["artifacts"].as_array_mut().expect("artifacts") {
        if artifact["name"] == request().asset_name {
            artifact["size"] = json!(bytes.len());
            artifact["sha256"] = json!(hex::encode(digest::digest(&digest::SHA256, &bytes)));
        }
    }
    let manifest = serde_json::to_vec(&manifest).expect("manifest JSON");
    let release = checked(&manifest, &signed(&manifest, &[7])).expect("authenticated package");
    (path, release)
}

#[test]
fn signed_package_is_verified_extracted_and_staged_before_replacement() {
    let directory = tempfile::tempdir().expect("directory");
    let (archive, release) = packaged_release(directory.path(), &"a".repeat(40));
    let installed = directory.path().join("installed");
    std::fs::write(&installed, b"previous version").expect("installed fixture");
    let (package, staged) = super::super::prepare_release_binary(
        &request(),
        &release,
        &archive,
        directory.path(),
        &InstallCancel::new(),
    )
    .expect("prepare authenticated binary");
    assert!(package.join("strata").is_file());
    assert_eq!(
        std::fs::read(&installed).expect("old executable"),
        b"previous version"
    );
    staged.persist(&installed).expect("atomic replacement");
    super::super::confirm_replacement(&installed).expect("replacement runs");
}

#[test]
fn damaged_download_is_rejected_before_extraction_or_replacement() {
    use std::io::Write;
    let directory = tempfile::tempdir().expect("directory");
    let (archive, release) = packaged_release(directory.path(), &"a".repeat(40));
    std::fs::OpenOptions::new()
        .write(true)
        .open(&archive)
        .expect("archive")
        .write_all(b"bad")
        .expect("tamper");
    assert!(
        super::super::prepare_release_binary(
            &request(),
            &release,
            &archive,
            directory.path(),
            &InstallCancel::new()
        )
        .is_err()
    );
    assert!(!directory.path().join("extracted").exists());
}

#[test]
fn signed_archive_with_wrong_packaged_source_never_stages_an_executable() {
    let directory = tempfile::tempdir().expect("directory");
    let (archive, release) = packaged_release(directory.path(), &"b".repeat(40));
    assert!(
        super::super::prepare_release_binary(
            &request(),
            &release,
            &archive,
            directory.path(),
            &InstallCancel::new()
        )
        .is_err()
    );
    assert!(
        !std::fs::read_dir(directory.path())
            .expect("workdir")
            .any(|entry| entry
                .expect("entry")
                .path()
                .extension()
                .is_some_and(|extension| extension == "tmp"))
    );
}

#[test]
fn production_trust_store_excludes_test_keys() {
    let keys: Vec<String> =
        serde_json::from_str(TRUSTED_KEYS_JSON).expect("production trust store");
    assert!(!keys.is_empty());
    assert!(keys.iter().all(|key| lower_hex(key, 64)));
    assert!(!keys.iter().any(|key| key == PUBLIC.trim()));
    assert!(authenticate(FIXTURE, SIGNATURES, &request()).is_err());
}
