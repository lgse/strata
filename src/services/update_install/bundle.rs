// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    fs::{self, DirBuilder, File, Metadata, OpenOptions},
    io::{BufReader, Read, Seek, SeekFrom},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt, symlink},
    path::{Component, Path, PathBuf},
    process::Command,
};

use flate2::bufread::GzDecoder;
use rustix::fs::{FlockOperation, Mode, OFlags, flock, open};
use serde::{Deserialize, Deserializer, de::{MapAccess, Visitor}};

use super::Version;

const BUNDLE_FORMAT: u32 = 1;
const MEDIA_PROTOCOL: u32 = 1;
const MAX_MANIFEST_SIZE: u64 = 1024 * 1024;
const MAX_MANIFEST_ENTRIES: usize = 128;
const MAX_ARCHIVE_ENTRIES: usize = MAX_MANIFEST_ENTRIES + 64;
const MAX_FILE_SIZE: u64 = 256 * 1024 * 1024;
const MAX_EXTRACTED_SIZE: u64 = 512 * 1024 * 1024;
const MAX_STORED_VERSIONS: usize = 8;
const REQUIRED_FILES: [&str; 2] = ["strata", "strata-media-helper"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleManifest {
    format: u32,
    release_tag: String,
    target: String,
    source_commit: String,
    media_protocol: u32,
    #[serde(deserialize_with = "deserialize_manifest_files")]
    files: HashMap<String, String>,
}

fn deserialize_manifest_files<'de, D>(deserializer: D) -> Result<HashMap<String, String>, D::Error>
where D: Deserializer<'de> {
    struct FilesVisitor;
    impl<'de> Visitor<'de> for FilesVisitor {
        type Value = HashMap<String, String>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a map with unique bundle paths")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut files = HashMap::new();
            while let Some((path, hash)) = map.next_entry::<String, String>()? {
                if files.len() >= MAX_MANIFEST_ENTRIES {
                    return Err(serde::de::Error::custom("too many bundle manifest entries"));
                }
                if files.insert(path.clone(), hash).is_some() {
                    return Err(serde::de::Error::custom(format!("duplicate bundle manifest path {path}")));
                }
            }
            Ok(files)
        }
    }
    deserializer.deserialize_map(FilesVisitor)
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct ExpectedBundle {
    pub release_tag: String,
    pub version: String,
    pub target: String,
    pub top_directory: String,
}

pub(super) fn expected_bundle_from_url(url: &str) -> Result<ExpectedBundle, String> {
    let invalid = || "The update URL is not a trusted GitHub release asset URL".to_owned();
    if !url.is_ascii() || url.contains(['?', '#', '\\']) {
        return Err(invalid());
    }
    let prefix = "https://github.com/";
    let remainder = url.get(..prefix.len()).filter(|value| value.eq_ignore_ascii_case(prefix))
        .and_then(|_| url.get(prefix.len()..)).ok_or_else(invalid)?;
    let components: Vec<_> = remainder.split('/').collect();
    if components.len() != 6 || !components[0].eq_ignore_ascii_case("lgse")
        || !components[1].eq_ignore_ascii_case("strata") || components[2] != "releases"
        || components[3] != "download" {
        return Err(invalid());
    }
    let tag = components[4];
    let version = tag.strip_prefix('v').ok_or_else(|| "The update URL contains an invalid release tag".to_owned())?;
    if Version::parse(tag).is_none() || !safe_release_component(tag) {
        return Err("The update URL contains an invalid release tag".to_owned());
    }
    let asset = components[5];
    let stem = asset.strip_prefix("strata-").and_then(|value| value.strip_suffix(".tar.gz"))
        .ok_or_else(|| "The update URL names an unexpected release asset".to_owned())?;
    let target = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"].into_iter()
        .find(|target| stem.strip_suffix(target).is_some_and(|value| value.ends_with('-')))
        .ok_or_else(|| "The update URL names an unsupported target".to_owned())?;
    let asset_version = stem.strip_suffix(target).and_then(|value| value.strip_suffix('-'))
        .ok_or_else(|| "The update URL names an invalid release asset".to_owned())?;
    if version != asset_version || !safe_release_component(asset_version) {
        return Err("The release tag and asset version do not match".to_owned());
    }
    if target != build_target() {
        return Err(format!("The update targets {target}, not {}", build_target()));
    }
    Ok(ExpectedBundle {
        release_tag: tag.to_owned(), version: version.to_owned(), target: target.to_owned(),
        top_directory: format!("strata-{version}-{target}"),
    })
}

fn safe_release_component(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

fn build_target() -> &'static str {
    #[cfg(target_arch = "x86_64")] { "x86_64-unknown-linux-gnu" }
    #[cfg(target_arch = "aarch64")] { "aarch64-unknown-linux-gnu" }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))] { "unsupported" }
}

pub(super) fn installation_bin_dir(current_exe: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(current_exe).map_err(|error| format!("Could not resolve the running executable: {error}"))?;
    let parent = canonical.parent().ok_or_else(|| "Could not determine the install directory".to_owned())?;
    if parent.parent().and_then(Path::file_name).and_then(|v| v.to_str()) == Some("versions")
        && parent.parent().and_then(Path::parent).and_then(Path::file_name).and_then(|v| v.to_str()) == Some(".strata-bundles") {
        return parent.parent().and_then(Path::parent).and_then(Path::parent).map(Path::to_owned)
            .ok_or_else(|| "The bundle installation layout is invalid".to_owned());
    }
    Ok(parent.to_owned())
}

#[derive(Debug)]
struct InstallLock(#[allow(dead_code)] File);

fn secure_metadata(path: &Path, kind: &str, directory: bool) -> Result<Metadata, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| format!("Could not inspect {kind}: {error}"))?;
    let valid_type = if directory { metadata.file_type().is_dir() } else { metadata.file_type().is_file() };
    if !valid_type || metadata.file_type().is_symlink() || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o022 != 0 {
        return Err(format!("The {kind} is not a private, owner-controlled {}", if directory { "directory" } else { "regular file" }));
    }
    Ok(metadata)
}

fn create_private_dir(path: &Path, kind: &str) -> Result<(), String> {
    match DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => sync_parent(path),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => secure_metadata(path, kind, true).map(|_| ()),
        Err(error) => Err(format!("Could not create {kind}: {error}")),
    }
}

fn open_nofollow(path: &Path, flags: OFlags, mode: Mode) -> Result<File, String> {
    open(path, flags | OFlags::NOFOLLOW | OFlags::CLOEXEC, mode).map(File::from)
        .map_err(|error| format!("Could not securely open {}: {error}", path.display()))
}

fn acquire_lock(bundle_root: &Path) -> Result<InstallLock, String> {
    create_private_dir(bundle_root, "bundle storage")?;
    let lock_path = bundle_root.join("install.lock");
    let file = open_nofollow(&lock_path, OFlags::RDWR | OFlags::CREATE, Mode::RUSR | Mode::WUSR)?;
    secure_metadata(&lock_path, "update lock", false)?;
    file.sync_all().map_err(|error| format!("Could not sync the update lock: {error}"))?;
    sync_directory(bundle_root)?;
    flock(&file, FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| "Another Strata update is already installing; try again when it finishes".to_owned())?;
    Ok(InstallLock(file))
}

pub(super) fn install_archive(archive_path: &Path, archive_hash: &str, expected: &ExpectedBundle,
    bin_dir: &Path, current_exe: &Path) -> Result<PathBuf, String> {
    if !valid_hash(archive_hash) || sha256_file(archive_path)? != archive_hash {
        return Err("The archive checksum is invalid".to_owned());
    }
    secure_metadata(bin_dir, "installation directory", true)?;
    let root = bin_dir.join(".strata-bundles");
    let _lock = acquire_lock(&root)?;
    let versions = root.join("versions");
    create_private_dir(&versions, "bundle version storage")?;
    let destination = versions.join(archive_hash);
    if fs::symlink_metadata(&destination).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) {
        enforce_version_limit(&versions)?;
        let staging = tempfile::Builder::new().prefix(".staging-").tempdir_in(&versions)
            .map_err(|error| format!("Could not stage the bundle: {error}"))?;
        extract_and_verify(archive_path, staging.path(), expected)?;
        set_executable(&staging.path().join("strata"))?;
        set_executable(&staging.path().join("strata-media-helper"))?;
        sync_tree(staging.path())?;
        fs::rename(staging.keep(), &destination).map_err(|error| format!("Could not publish the immutable bundle: {error}"))?;
        sync_directory(&versions)?;
    } else {
        secure_metadata(&destination, "existing bundle version", true)?;
        verify_existing_version(&destination, expected)?;
    }

    let migrate_launcher = validate_stable_launcher(bin_dir, current_exe)?;
    let current_missing = fs::symlink_metadata(root.join("current"))
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
    if migrate_launcher && current_missing {
        let legacy = preserve_legacy(&versions, current_exe)?;
        activate(&root, &legacy)?;
    }
    activate(&root, archive_hash)?;
    if migrate_launcher { install_stable_launcher(bin_dir)?; }
    Ok(destination)
}

fn enforce_version_limit(versions: &Path) -> Result<(), String> {
    let mut count = 0;
    for entry in fs::read_dir(versions).map_err(|error| format!("Could not inspect bundle storage: {error}"))? {
        let entry = entry.map_err(|error| format!("Could not inspect bundle storage: {error}"))?;
        if entry.file_name().to_string_lossy().starts_with(".staging-") { continue; }
        secure_metadata(&entry.path(), "stored bundle version", true)?;
        count += 1;
    }
    if count >= MAX_STORED_VERSIONS {
        return Err(format!("Strata has {count} stored update versions. Remove an unused directory from {} and try again; do not remove the current or previous target.", versions.display()));
    }
    Ok(())
}

fn preserve_legacy(versions: &Path, current_exe: &Path) -> Result<String, String> {
    let hash = sha256_file(current_exe)?;
    let id = format!("legacy-{hash}");
    let destination = versions.join(&id);
    if fs::symlink_metadata(&destination).is_ok() {
        secure_metadata(&destination, "legacy bundle version", true)?;
        if sha256_file(&destination.join("strata"))? != hash { return Err("The preserved legacy executable was modified".to_owned()); }
        return Ok(id);
    }
    enforce_version_limit(versions)?;
    let staging = tempfile::Builder::new().prefix(".legacy-").tempdir_in(versions)
        .map_err(|error| format!("Could not stage the legacy executable: {error}"))?;
    let mut source = open_nofollow(current_exe, OFlags::RDONLY, Mode::empty())?;
    let path = staging.path().join("strata");
    let mut output = OpenOptions::new().write(true).create_new(true).mode(0o700)
        .custom_flags((OFlags::NOFOLLOW | OFlags::CLOEXEC).bits() as i32)
        .open(&path).map_err(|error| format!("Could not preserve the legacy executable: {error}"))?;
    std::io::copy(&mut source, &mut output).map_err(|error| format!("Could not preserve the legacy executable: {error}"))?;
    output.set_permissions(fs::Permissions::from_mode(0o755)).map_err(|error| format!("Could not preserve legacy permissions: {error}"))?;
    output.sync_all().map_err(|error| format!("Could not sync the legacy executable: {error}"))?;
    sync_directory(staging.path())?;
    fs::rename(staging.keep(), &destination).map_err(|error| format!("Could not publish the legacy executable: {error}"))?;
    sync_directory(versions)?;
    Ok(id)
}

fn activate(root: &Path, id: &str) -> Result<(), String> {
    if !safe_storage_id(id) { return Err("The bundle activation identifier is invalid".to_owned()); }
    let target = Path::new("versions").join(id);
    let current = root.join("current");
    if current.symlink_metadata().is_ok() {
        if !current.symlink_metadata().is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err("The current bundle pointer is not a symlink".to_owned());
        }
        let old_target = fs::read_link(&current).map_err(|error| format!("Could not inspect bundle activation: {error}"))?;
        if old_target == target { return Ok(()); }
        validate_pointer_target(&old_target)?;
        replace_symlink(root, "previous", &old_target)?;
    }
    sync_directory(root)?;
    replace_symlink(root, "current", &target)?;
    if let Err(error) = sync_directory(root) {
        tracing::warn!(%error, "bundle activation committed but its parent directory could not be synced");
    }
    Ok(())
}

fn replace_symlink(directory: &Path, name: &str, target: &Path) -> Result<(), String> {
    let staging = tempfile::Builder::new().prefix(".activation-").tempdir_in(directory)
        .map_err(|error| format!("Could not stage bundle activation: {error}"))?;
    let temporary = staging.path().join(name);
    symlink(target, &temporary).map_err(|error| format!("Could not stage bundle activation: {error}"))?;
    sync_directory(staging.path())?;
    fs::rename(&temporary, directory.join(name)).map_err(|error| format!("Could not activate the bundle: {error}"))
}

fn validate_pointer_target(target: &Path) -> Result<(), String> {
    let mut parts = target.components();
    if parts.next() != Some(Component::Normal("versions".as_ref()))
        || !parts.next().is_some_and(|part| matches!(part, Component::Normal(value) if value.to_str().is_some_and(safe_storage_id)))
        || parts.next().is_some() {
        return Err("The installed bundle pointer has an unsafe target".to_owned());
    }
    Ok(())
}

fn safe_storage_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn validate_stable_launcher(bin_dir: &Path, current_exe: &Path) -> Result<bool, String> {
    let launcher = bin_dir.join("strata");
    let canonical_current = fs::canonicalize(current_exe).map_err(|error| format!("Could not resolve the running executable: {error}"))?;
    if launcher.symlink_metadata().is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        let target = fs::read_link(&launcher).map_err(|error| format!("Could not inspect the Strata launcher: {error}"))?;
        if target == Path::new(".strata-bundles/current/strata") { return Ok(false); }
        return Err("The Strata launcher is an unexpected symlink; it was not modified".to_owned());
    }
    secure_metadata(&launcher, "installed Strata launcher", false)?;
    if fs::canonicalize(&launcher).ok().as_deref() != Some(canonical_current.as_path()) {
        return Err("The installed Strata launcher changed during the update; it was not modified".to_owned());
    }
    Ok(true)
}

fn install_stable_launcher(bin_dir: &Path) -> Result<(), String> {
    let staging = tempfile::Builder::new().prefix(".launcher-").tempdir_in(bin_dir)
        .map_err(|error| format!("Could not stage the Strata launcher: {error}"))?;
    let temporary = staging.path().join("strata");
    symlink(".strata-bundles/current/strata", &temporary).map_err(|error| format!("Could not stage the Strata launcher: {error}"))?;
    sync_directory(staging.path())?;
    if let Err(error) = sync_directory(bin_dir) {
        tracing::warn!(%error, "could not pre-sync the stable launcher's parent directory");
    }
    fs::rename(&temporary, bin_dir.join("strata")).map_err(|error| format!("Could not activate the Strata launcher: {error}"))?;
    if let Err(error) = sync_directory(bin_dir) {
        tracing::warn!(%error, "stable launcher committed but its parent directory could not be synced");
    }
    Ok(())
}

fn extract_and_verify(archive_path: &Path, destination: &Path, expected: &ExpectedBundle) -> Result<(), String> {
    let file = open_nofollow(archive_path, OFlags::RDONLY, Mode::empty()).map_err(|error| format!("Could not open the bundle: {error}"))?;
    let mut decoder = GzDecoder::new(BufReader::new(file));
    let mut seen = HashSet::new();
    let mut regular = HashSet::new();
    let mut directories = HashSet::new();
    let mut total = 0_u64;
    {
        let mut archive = tar::Archive::new(&mut decoder);
        let entries = archive.entries().map_err(|error| format!("Could not read the bundle: {error}"))?;
        for (entry_index, entry) in entries.enumerate() {
            if entry_index >= MAX_ARCHIVE_ENTRIES {
                return Err("The bundle contains too many archive entries".to_owned());
            }
            let mut entry = entry.map_err(|error| format!("Could not read a bundle entry: {error}"))?;
            let path = entry.path().map_err(|error| format!("Invalid bundle path: {error}"))?.into_owned();
            validate_archive_path(&path, &expected.top_directory)?;
            if !seen.insert(path.clone()) { return Err(format!("The bundle contains duplicate path {}", path.display())); }
            let relative = path.strip_prefix(&expected.top_directory).expect("validated prefix");
            if relative.as_os_str().is_empty() {
                if !entry.header().entry_type().is_dir() { return Err("The bundle top-level entry is not a directory".to_owned()); }
                continue;
            }
            let output = destination.join(relative);
            if entry.header().entry_type().is_dir() {
                create_archive_directory(&output)?;
                directories.insert(path_string(relative)?);
            } else if entry.header().entry_type().is_file() {
                let size = entry.size();
                if size > MAX_FILE_SIZE || total.saturating_add(size) > MAX_EXTRACTED_SIZE { return Err("The bundle exceeds the extracted size limit".to_owned()); }
                total += size;
                if let Some(parent) = output.parent() { create_archive_directories(parent, destination)?; }
                let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600)
                    .custom_flags((OFlags::NOFOLLOW | OFlags::CLOEXEC).bits() as i32).open(&output)
                    .map_err(|error| format!("Could not extract {}: {error}", relative.display()))?;
                std::io::copy(&mut entry, &mut file).map_err(|error| format!("Could not extract {}: {error}", relative.display()))?;
                file.sync_all().map_err(|error| format!("Could not sync {}: {error}", relative.display()))?;
                regular.insert(path_string(relative)?);
            } else { return Err(format!("The bundle contains a link or non-regular entry: {}", path.display())); }
        }
    }
    let mut drain = Vec::new();
    decoder.read_to_end(&mut drain).map_err(|error| format!("The bundle gzip stream is corrupt or truncated: {error}"))?;
    if drain.iter().any(|byte| *byte != 0) {
        return Err("The bundle contains trailing archive data".to_owned());
    }
    let reader = decoder.into_inner();
    if !reader.buffer().is_empty() || reader.get_ref().stream_position().map_err(|error| error.to_string())? != reader.get_ref().metadata().map_err(|error| error.to_string())?.len() {
        return Err("The bundle contains trailing data after the gzip stream".to_owned());
    }
    verify_directory_entries(&directories, &regular)?;
    verify_extracted(destination, expected, &regular)
}

fn create_archive_directory(path: &Path) -> Result<(), String> {
    match DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => secure_metadata(path, "bundle directory", true).map(|_| ()),
        Err(error) => Err(format!("Could not create bundle directory: {error}")),
    }
}
fn create_archive_directories(path: &Path, root: &Path) -> Result<(), String> {
    if path == root { return Ok(()); }
    if let Some(parent) = path.parent() { create_archive_directories(parent, root)?; }
    create_archive_directory(path)
}
fn verify_directory_entries(directories: &HashSet<String>, files: &HashSet<String>) -> Result<(), String> {
    let mut required = HashSet::new();
    for file in files {
        let mut parent = Path::new(file).parent();
        while let Some(path) = parent.filter(|path| !path.as_os_str().is_empty()) {
            required.insert(path_string(path)?);
            parent = path.parent();
        }
    }
    if !directories.is_subset(&required) { return Err("The bundle contains an unneeded directory entry".to_owned()); }
    Ok(())
}

fn verify_existing_version(destination: &Path, expected: &ExpectedBundle) -> Result<(), String> {
    fn collect(directory: &Path, root: &Path, files: &mut HashSet<String>) -> Result<(), String> {
        secure_metadata(directory, "existing bundle directory", true)?;
        for entry in fs::read_dir(directory).map_err(|error| format!("Could not inspect an existing bundle: {error}"))? {
            let entry = entry.map_err(|error| format!("Could not inspect an existing bundle: {error}"))?;
            let metadata = entry.file_type().map_err(|error| format!("Could not inspect an existing bundle: {error}"))?;
            if metadata.is_symlink() { return Err("An existing immutable bundle contains a symlink".to_owned()); }
            if metadata.is_dir() { collect(&entry.path(), root, files)?; }
            else if metadata.is_file() { secure_metadata(&entry.path(), "existing bundle file", false)?; files.insert(path_string(entry.path().strip_prefix(root).expect("inside root"))?); }
            else { return Err("An existing immutable bundle contains a non-regular file".to_owned()); }
        }
        Ok(())
    }
    let mut files = HashSet::new(); collect(destination, destination, &mut files)?; verify_extracted(destination, expected, &files)
}

fn validate_archive_path(path: &Path, top: &str) -> Result<(), String> {
    if path.is_absolute() || path.components().any(|part| !matches!(part, Component::Normal(_))) { return Err(format!("The bundle contains an unsafe path: {}", path.display())); }
    if path.components().next().and_then(|part| match part { Component::Normal(v) => v.to_str(), _ => None }) != Some(top) {
        return Err("Every bundle entry must be inside the expected top-level directory".to_owned());
    }
    Ok(())
}
fn path_string(path: &Path) -> Result<String, String> { path.to_str().map(str::to_owned).ok_or_else(|| "The bundle contains a non-UTF-8 path".to_owned()) }

fn verify_extracted(destination: &Path, expected: &ExpectedBundle, regular: &HashSet<String>) -> Result<(), String> {
    let manifest_path = destination.join("bundle.json");
    let metadata = fs::symlink_metadata(&manifest_path).map_err(|_| "The bundle contains no bundle.json manifest".to_owned())?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_MANIFEST_SIZE { return Err("The bundle manifest is not a regular file within the size limit".to_owned()); }
    let manifest: BundleManifest = serde_json::from_reader(open_nofollow(&manifest_path, OFlags::RDONLY, Mode::empty())?)
        .map_err(|error| format!("The bundle manifest is invalid: {error}"))?;
    if manifest.format != BUNDLE_FORMAT || manifest.media_protocol != MEDIA_PROTOCOL || manifest.release_tag != expected.release_tag
        || manifest.target != expected.target || manifest.source_commit.len() != 40 || !manifest.source_commit.bytes().all(|v| v.is_ascii_hexdigit()) {
        return Err("The bundle manifest does not match the requested release, target, or protocol".to_owned());
    }
    let archived_files: HashSet<_> = regular.iter().filter(|path| path.as_str() != "bundle.json").cloned().collect();
    let manifest_files: HashSet<_> = manifest.files.keys().cloned().collect();
    if archived_files != manifest_files || REQUIRED_FILES.iter().any(|name| !archived_files.contains(*name)) {
        return Err("The bundle files do not exactly match its manifest or a required binary is missing".to_owned());
    }
    for (path, expected_hash) in &manifest.files {
        if !valid_manifest_path(path) || !valid_hash(expected_hash) { return Err(format!("The manifest contains an invalid file entry: {path}")); }
        let actual = sha256_file(&destination.join(path))?;
        if !actual.eq_ignore_ascii_case(expected_hash) { return Err(format!("Bundle file {path} failed checksum verification")); }
    }
    for name in REQUIRED_FILES { verify_elf(&destination.join(name), &expected.target)?; }
    Ok(())
}

fn valid_manifest_path(value: &str) -> bool {
    let path = Path::new(value); !path.is_absolute() && !value.is_empty() && path.components().all(|part| matches!(part, Component::Normal(_)))
}
fn valid_hash(value: &str) -> bool { value.len() == 64 && value.bytes().all(|v| v.is_ascii_hexdigit()) }

pub(super) fn sha256_file(path: &Path) -> Result<String, String> {
    let output = Command::new("sha256sum").arg("--").arg(path).output().map_err(|error| format!("Could not run sha256sum: {error}"))?;
    if !output.status.success() { return Err(format!("sha256sum failed: {}", String::from_utf8_lossy(&output.stderr).trim())); }
    let text = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    let hash = text.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    if valid_hash(&hash) { Ok(hash) } else { Err("sha256sum produced an invalid hash".to_owned()) }
}

fn verify_elf(path: &Path, target: &str) -> Result<(), String> {
    let mut file = open_nofollow(path, OFlags::RDONLY, Mode::empty())?;
    let length = file.metadata().map_err(|error| format!("Could not inspect {}: {error}", path.display()))?.len();
    let mut header = [0_u8; 64];
    file.read_exact(&mut header).map_err(|_| format!("{} is not a complete ELF executable", path.display()))?;
    if &header[..4] != b"\x7fELF" || header[4] != 2 || header[5] != 1 || header[6] != 1
        || u32::from_le_bytes(header[20..24].try_into().expect("ELF version")) != 1 {
        return Err(format!("{} is not a supported 64-bit little-endian ELF executable", path.display()));
    }
    if !matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3) { return Err(format!("{} is not an ELF executable", path.display())); }
    let machine = u16::from_le_bytes([header[18], header[19]]);
    let expected_machine = if target.starts_with("x86_64-") { 62 } else { 183 };
    if machine != expected_machine { return Err(format!("{} has the wrong ELF architecture", path.display())); }
    let phoff = u64::from_le_bytes(header[32..40].try_into().expect("ELF offset"));
    let ehsize = u16::from_le_bytes([header[52], header[53]]) as u64;
    let phentsize = u16::from_le_bytes([header[54], header[55]]) as u64;
    let phnum = u16::from_le_bytes([header[56], header[57]]) as u64;
    let table_size = phentsize.checked_mul(phnum).ok_or_else(|| format!("{} has invalid ELF program headers", path.display()))?;
    if ehsize < 64 || phnum == 0 || phnum > 1024 || phentsize < 56 || phoff < ehsize
        || phoff.checked_add(table_size).is_none_or(|end| end > length) {
        return Err(format!("{} has invalid ELF program headers", path.display()));
    }
    for index in 0..phnum {
        file.seek(SeekFrom::Start(phoff + index * phentsize)).map_err(|error| format!("Could not inspect ELF headers: {error}"))?;
        let mut program = [0_u8; 56];
        file.read_exact(&mut program).map_err(|_| "The ELF program headers are truncated".to_owned())?;
        let offset = u64::from_le_bytes(program[8..16].try_into().expect("segment offset"));
        let file_size = u64::from_le_bytes(program[32..40].try_into().expect("segment size"));
        let memory_size = u64::from_le_bytes(program[40..48].try_into().expect("memory size"));
        if file_size > memory_size || offset.checked_add(file_size).is_none_or(|end| end > length) {
            return Err(format!("{} has an invalid ELF segment extent", path.display()));
        }
    }
    Ok(())
}

fn set_executable(path: &Path) -> Result<(), String> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|error| format!("Could not mark {} executable: {error}", path.display()))?;
    open_nofollow(path, OFlags::RDONLY, Mode::empty())?.sync_all().map_err(|error| format!("Could not sync {}: {error}", path.display()))
}
fn sync_tree(path: &Path) -> Result<(), String> {
    for entry in fs::read_dir(path).map_err(|error| format!("Could not inspect staged bundle: {error}"))? {
        let entry = entry.map_err(|error| format!("Could not inspect staged bundle: {error}"))?;
        if entry.file_type().map_err(|error| error.to_string())?.is_dir() { sync_tree(&entry.path())?; }
    }
    sync_directory(path)
}
fn sync_directory(path: &Path) -> Result<(), String> {
    open_nofollow(path, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty())?.sync_all()
        .map_err(|error| format!("Could not sync directory {}: {error}", path.display()))
}
fn sync_parent(path: &Path) -> Result<(), String> {
    path.parent().map_or(Ok(()), sync_directory)
}

#[cfg(test)]
mod tests;
