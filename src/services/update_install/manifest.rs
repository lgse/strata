// SPDX-License-Identifier: GPL-3.0-or-later

use std::{collections::HashSet, fs::File, io::Read, path::Path};

use ring::{digest, signature};
use serde::Deserialize;

use super::{InstallCancel, InstallRequest, InstallStop, MAX_UPDATE_ARCHIVE_BYTES, REPOSITORY};

pub(super) const MANIFEST_NAME: &str = "strata-update-manifest.json";
pub(super) const SIGNATURES_NAME: &str = "strata-update-manifest.signatures.json";
pub(super) const MAX_MANIFEST_BYTES: u64 = 32 * 1024;
pub(super) const MAX_SIGNATURES_BYTES: u64 = 8 * 1024;
const DOMAIN: &[u8] = b"strata-update-manifest-v1\0";
const TRUSTED_KEYS_JSON: &str = include_str!("../../../data/update-keys.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    repository: String,
    tag: String,
    source_commit: String,
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    name: String,
    target: String,
    size: u64,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Signatures {
    schema: u32,
    signatures: Vec<Signature>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Signature {
    key_id: String,
    signature: String,
}

pub(super) struct VerifiedRelease {
    artifact: Artifact,
    source_commit: String,
}

impl VerifiedRelease {
    pub(super) fn archive_size(&self) -> u64 {
        self.artifact.size
    }

    pub(super) fn verify_archive(
        &self,
        path: &Path,
        cancel: &InstallCancel,
    ) -> Result<(), InstallStop> {
        let mut file = File::open(path).map_err(|error| error.to_string())?;
        let mut context = digest::Context::new(&digest::SHA256);
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            cancel.check()?;
            let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            size = size.saturating_add(count as u64);
            if size > self.artifact.size {
                return Err("The update exceeds its signed size".to_owned().into());
            }
            context.update(&buffer[..count]);
        }
        if size != self.artifact.size || hex::encode(context.finish()) != self.artifact.sha256 {
            return Err("The update does not match its signed size and checksum"
                .to_owned()
                .into());
        }
        Ok(())
    }

    pub(super) fn verify_source_commit(&self, package: &Path) -> Result<(), String> {
        let mut contents = String::new();
        File::open(package.join("SOURCE_COMMIT"))
            .and_then(|file| file.take(128).read_to_string(&mut contents))
            .map_err(|error| format!("Could not read the update's source commit: {error}"))?;
        if contents.len() >= 128 || contents.trim() != self.source_commit {
            return Err("The packaged source commit does not match the signed manifest".to_owned());
        }
        Ok(())
    }
}

pub(super) fn authenticate(
    bytes: &[u8],
    signatures: &[u8],
    request: &InstallRequest,
) -> Result<VerifiedRelease, String> {
    let keys: Vec<String> = serde_json::from_str(TRUSTED_KEYS_JSON)
        .map_err(|error| format!("Invalid built-in release keys: {error}"))?;
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    authenticate_with_keys(bytes, signatures, request, &keys)
}

fn authenticate_with_keys(
    bytes: &[u8],
    signatures: &[u8],
    request: &InstallRequest,
    keys: &[&str],
) -> Result<VerifiedRelease, String> {
    super::verified_download_url(request)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES || signatures.len() as u64 > MAX_SIGNATURES_BYTES {
        return Err("The signed update metadata is too large".to_owned());
    }
    let signatures: Signatures = serde_json::from_slice(signatures)
        .map_err(|error| format!("Invalid update signatures: {error}"))?;
    if signatures.schema != 1 || signatures.signatures.is_empty() || signatures.signatures.len() > 8
    {
        return Err("Unsupported update signature format".to_owned());
    }
    let message = [DOMAIN, bytes].concat();
    let verified = keys.iter().any(|key| {
        let Ok(public_key) = hex::decode(key.trim()) else {
            return false;
        };
        if public_key.len() != 32 {
            return false;
        }
        let key_id = hex::encode(digest::digest(&digest::SHA256, &public_key));
        signatures.signatures.iter().any(|entry| {
            if entry.key_id != key_id {
                return false;
            }
            let Ok(signature) = hex::decode(&entry.signature) else {
                return false;
            };
            signature::UnparsedPublicKey::new(&signature::ED25519, &public_key)
                .verify(&message, &signature)
                .is_ok()
        })
    });
    if !verified {
        return Err("No trusted release key signed this update manifest".to_owned());
    }
    // Verify the original bytes, not a reserialized JSON representation.
    let manifest: Manifest = serde_json::from_slice(bytes)
        .map_err(|error| format!("Invalid signed update manifest: {error}"))?;
    if manifest.schema != 1
        || manifest.repository != REPOSITORY
        || manifest.tag != request.tag
        || !lower_hex(&manifest.source_commit, 40)
        || manifest.artifacts.is_empty()
        || manifest.artifacts.len() > 8
    {
        return Err("The signed manifest does not identify the selected Strata release".to_owned());
    }
    let version = request
        .tag
        .strip_prefix('v')
        .ok_or_else(|| "Invalid release tag".to_owned())?;
    let mut names = HashSet::new();
    let mut targets = HashSet::new();
    for artifact in &manifest.artifacts {
        if !matches!(
            artifact.target.as_str(),
            "x86_64-unknown-linux-gnu" | "aarch64-unknown-linux-gnu"
        ) || artifact.name != format!("strata-{version}-{}.tar.gz", artifact.target)
            || artifact.size == 0
            || artifact.size > MAX_UPDATE_ARCHIVE_BYTES
            || !lower_hex(&artifact.sha256, 64)
            || !names.insert(&artifact.name)
            || !targets.insert(&artifact.target)
        {
            return Err("The signed manifest contains invalid or duplicate artifacts".to_owned());
        }
    }
    let artifact = manifest
        .artifacts
        .into_iter()
        .find(|artifact| {
            artifact.name == request.asset_name
                && artifact.target == format!("{}-unknown-linux-gnu", std::env::consts::ARCH)
        })
        .ok_or_else(|| {
            "The signed manifest does not contain this architecture's update".to_owned()
        })?;
    Ok(VerifiedRelease {
        artifact,
        source_commit: manifest.source_commit,
    })
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests;
