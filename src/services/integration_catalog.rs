// SPDX-License-Identifier: MIT
//! Opt-in integrations bundled with Strata: the catalog under `integrations/`,
//! their installation as external file providers and update detection.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

use super::{file_providers, package_manager::PackageManager};

/// One embedded file: its plain name and contents.
pub(crate) type BundledFile = (&'static str, &'static [u8]);
/// One embedded integration directory: its id and files.
pub(crate) type BundledEntry = (&'static str, &'static [BundledFile]);

mod bundle {
    include!(concat!(env!("OUT_DIR"), "/integrations.rs"));
}

#[cfg(test)]
mod tests;

pub(crate) const CATALOG_FILE: &str = "catalog.toml";
/// Written beside an installed copy; its absence marks a manual installation.
pub(crate) const RECORD_FILE: &str = "catalog-install.json";
/// An `integrations/`-layout directory that replaces the bundled catalog while
/// developing an integration.
pub(crate) const CATALOG_OVERRIDE: &str = "STRATA_INTEGRATION_CATALOG";
const MANIFEST_FILE: &str = "provider.json";
const FILE_LIMIT: usize = 32;
const FILE_SIZE_LIMIT: usize = 1024 * 1024;
const REQUIREMENT_LIMIT: usize = 8;
const ALTERNATIVE_LIMIT: usize = 4;
const MODULE_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Category {
    Cloud,
    Devices,
    VersionControl,
    Other,
}

impl Category {
    pub(crate) const ALL: [Self; 4] = [
        Self::Cloud,
        Self::Devices,
        Self::VersionControl,
        Self::Other,
    ];

    /// The manifest spelling, also written to installed registrations.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Cloud => "cloud",
            Self::Devices => "devices",
            Self::VersionControl => "version-control",
            Self::Other => "other",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|category| category.id() == id)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Requirement {
    /// Executable expected in a trusted system directory.
    #[serde(default)]
    pub command: Option<String>,
    /// Other executable names that satisfy a command requirement, for packages
    /// that install the same program under different names.
    #[serde(default)]
    pub alternatives: Vec<String>,
    /// Module importable with the provider's interpreter.
    #[serde(default)]
    pub module: Option<String>,
    /// Package providing it, keyed by package manager id.
    #[serde(default)]
    pub packages: BTreeMap<String, String>,
}

impl Requirement {
    pub(crate) fn label(&self) -> &str {
        self.command
            .as_deref()
            .or(self.module.as_deref())
            .unwrap_or_default()
    }

    pub(crate) fn install_command(&self, manager: Option<PackageManager>) -> Option<String> {
        let manager = manager?;
        manager.install_command(&[self.packages.get(manager.id())?])
    }

    fn validate(&self) -> Result<(), String> {
        match (&self.command, &self.module) {
            (Some(command), None) if single_basename(command) => {}
            (None, Some(module))
                if !module.is_empty()
                    && module.len() <= 64
                    && module.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                    && module
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_') => {}
            (None, Some(_)) => {
                return Err("module must be a top-level module name".into());
            }
            _ => return Err("each requirement names exactly one command or module".into()),
        }
        if !self.alternatives.is_empty() && self.command.is_none() {
            return Err("alternatives belong to a command requirement".into());
        }
        if self.alternatives.len() > ALTERNATIVE_LIMIT
            || !self.alternatives.iter().all(|name| single_basename(name))
        {
            return Err(format!(
                "at most {ALTERNATIVE_LIMIT} alternatives, each an executable basename"
            ));
        }
        for (manager, package) in &self.packages {
            if !["pacman", "apt", "dnf", "zypper"].contains(&manager.as_str()) {
                return Err(format!("unknown package manager {manager:?}"));
            }
            if PackageManager::Pacman.install_command(&[package]).is_none() {
                return Err(format!("invalid package name {package:?}"));
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    id: String,
    name: String,
    description: String,
    category: Category,
    maintainer: String,
    runtime: String,
    entry: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    icons: BTreeMap<String, String>,
    #[serde(default)]
    requirements: Vec<Requirement>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: Category,
    pub maintainer: String,
    /// Interpreter basename, resolved in trusted system directories on install.
    pub runtime: String,
    pub entry: String,
    pub icon: Option<String>,
    pub icons: BTreeMap<String, String>,
    pub requirements: Vec<Requirement>,
    /// Every installed file keyed by plain filename, including the catalog manifest.
    pub files: BTreeMap<String, Vec<u8>>,
    /// Content hash recorded at install time; a different bundled hash is an update.
    pub hash: String,
}

impl Entry {
    pub(crate) fn icon_bytes(&self) -> Option<&[u8]> {
        self.files
            .get(self.icons.get(self.icon.as_ref()?)?)
            .map(Vec::as_slice)
    }

    /// The runtime first, then the declared requirements.
    pub(crate) fn requirements(&self) -> Vec<Requirement> {
        let mut requirements = self.requirements.clone();
        if !requirements
            .iter()
            .any(|requirement| requirement.command.as_deref() == Some(&self.runtime))
        {
            requirements.insert(
                0,
                Requirement {
                    command: Some(self.runtime.clone()),
                    alternatives: Vec::new(),
                    module: None,
                    packages: BTreeMap::new(),
                },
            );
        }
        requirements
    }
}

fn single_basename(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

fn plain_text(text: &str, limit: usize) -> bool {
    !text.trim().is_empty() && text.len() <= limit && !text.chars().any(char::is_control)
}

pub(crate) fn parse(id: &str, files: BTreeMap<String, Vec<u8>>) -> Result<Entry, String> {
    if !file_providers::slug(id) {
        return Err("directory name must be a slug".into());
    }
    if files.len() > FILE_LIMIT {
        return Err(format!("at most {FILE_LIMIT} files"));
    }
    for (name, bytes) in &files {
        if !single_basename(name) {
            return Err(format!("invalid filename {name:?}"));
        }
        if [MANIFEST_FILE, RECORD_FILE, file_providers::DISABLED_FILE].contains(&name.as_str()) {
            return Err(format!("{name} is written by Strata at install time"));
        }
        if bytes.len() > FILE_SIZE_LIMIT {
            return Err(format!("{name} exceeds {FILE_SIZE_LIMIT} bytes"));
        }
    }
    let manifest = files
        .get(CATALOG_FILE)
        .ok_or_else(|| format!("{CATALOG_FILE} is missing"))?;
    let manifest: Manifest = toml::from_str(
        std::str::from_utf8(manifest).map_err(|_| format!("{CATALOG_FILE} is not UTF-8"))?,
    )
    .map_err(|error| format!("{CATALOG_FILE}: {error}"))?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported schema_version {}",
            manifest.schema_version
        ));
    }
    if manifest.id != id {
        return Err(format!("id {:?} does not match the directory", manifest.id));
    }
    if !plain_text(&manifest.name, 64) {
        return Err("name must be plain text of at most 64 bytes".into());
    }
    if !plain_text(&manifest.description, 400) {
        return Err("description must be plain text of at most 400 bytes".into());
    }
    if !(1..=39).contains(&manifest.maintainer.len())
        || !manifest
            .maintainer
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("maintainer must be a GitHub username".into());
    }
    if !single_basename(&manifest.runtime) {
        return Err("runtime must be an interpreter basename".into());
    }
    if !files.contains_key(&manifest.entry) {
        return Err(format!("entry {:?} is not among the files", manifest.entry));
    }
    if manifest.icons.len() > 16 {
        return Err("at most 16 icons".into());
    }
    for (icon, filename) in &manifest.icons {
        let bytes = files.get(filename);
        if !file_providers::slug(icon)
            || !filename.ends_with(".png")
            || !bytes.is_some_and(|bytes| file_providers::valid_icon(bytes))
        {
            return Err(format!(
                "icon {icon:?} must name a bundled PNG of at most 256×256"
            ));
        }
    }
    if let Some(icon) = &manifest.icon
        && !manifest.icons.contains_key(icon)
    {
        return Err(format!("icon {icon:?} is not declared"));
    }
    if manifest.requirements.len() > REQUIREMENT_LIMIT {
        return Err(format!("at most {REQUIREMENT_LIMIT} requirements"));
    }
    for requirement in &manifest.requirements {
        requirement.validate()?;
    }
    let hash = content_hash(&files);
    Ok(Entry {
        id: manifest.id,
        name: manifest.name,
        description: manifest.description,
        category: manifest.category,
        maintainer: manifest.maintainer,
        runtime: manifest.runtime,
        entry: manifest.entry,
        icon: manifest.icon,
        icons: manifest.icons,
        requirements: manifest.requirements,
        files,
        hash,
    })
}

fn content_hash(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut hasher = Sha256::new();
    for (name, bytes) in files {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    hex(&hasher.finalize())
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn read_directory(directory: &Path) -> Result<Entry, String> {
    let id = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("directory name is not UTF-8")?;
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with('.') || name == "README.md" || !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path).map_err(|error| format!("{name}: {error}"))?;
        files.insert(name.to_owned(), bytes);
    }
    parse(id, files)
}

pub(crate) fn load_directory(root: &Path) -> Vec<Entry> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut directories: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| !name.starts_with(['.', '_']))
        })
        .collect();
    directories.sort();
    directories
        .iter()
        .filter_map(|directory| match read_directory(directory) {
            Ok(entry) => Some(entry),
            Err(error) => {
                eprintln!("Strata integration {}: {error}", directory.display());
                None
            }
        })
        .collect()
}

pub(crate) fn bundled() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        bundle::BUNDLED
            .iter()
            .filter_map(|(id, files)| {
                let files = files
                    .iter()
                    .map(|(name, bytes)| ((*name).to_owned(), bytes.to_vec()))
                    .collect();
                match parse(id, files) {
                    Ok(entry) => Some(entry),
                    Err(error) => {
                        eprintln!("Strata bundled integration {id}: {error}");
                        None
                    }
                }
            })
            .collect()
    })
}

/// The bundled catalog, or the directory named by `STRATA_INTEGRATION_CATALOG`.
pub(crate) fn catalog() -> Vec<Entry> {
    match std::env::var_os(CATALOG_OVERRIDE) {
        Some(directory) => load_directory(Path::new(&directory)),
        None => bundled().to_vec(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    NotInstalled,
    /// Installed from the catalog. `update` when the catalog copy changed since
    /// installation, `modified` when the installed files were edited.
    Installed {
        update: bool,
        modified: bool,
    },
    /// Registered by hand or by another tool; Strata only removes it.
    Manual,
}

#[derive(Deserialize, Serialize)]
struct Record {
    version: u32,
    catalog: String,
    files: BTreeMap<String, String>,
}

fn read_record(directory: &Path) -> Option<Record> {
    let path = directory.join(RECORD_FILE);
    if fs::symlink_metadata(&path).ok()?.file_type().is_symlink() {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn is_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

pub(crate) fn status(entry: &Entry, root: &Path) -> Status {
    let directory = root.join(&entry.id);
    if fs::symlink_metadata(&directory).is_err() {
        return Status::NotInstalled;
    }
    if !is_directory(&directory) {
        return Status::Manual;
    }
    match read_record(&directory) {
        Some(record) if record.version == 1 => Status::Installed {
            update: record.catalog != entry.hash,
            modified: record.files.iter().any(|(name, digest)| {
                fs::read(directory.join(name))
                    .ok()
                    .map(|bytes| sha256(&bytes))
                    .as_ref()
                    != Some(digest)
            }),
        },
        _ => Status::Manual,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Installation {
    pub id: String,
    pub name: String,
    /// The declared icon, when the registration loads as a trusted provider.
    pub icon: Option<Vec<u8>>,
    pub enabled: bool,
}

/// Whether the installed registration loads; a `disabled` marker turns it off.
pub(crate) fn enabled(root: &Path, id: &str) -> bool {
    !root.join(id).join(file_providers::DISABLED_FILE).exists()
}

/// Writes or removes the `disabled` marker of an installed registration.
pub(crate) fn set_enabled(root: &Path, id: &str, enabled: bool) -> Result<(), String> {
    if !file_providers::slug(id) {
        return Err(format!("{id}: not a provider id"));
    }
    let directory = root.join(id);
    if fs::symlink_metadata(&directory).is_ok_and(|metadata| metadata.file_type().is_symlink())
        || !directory.is_dir()
    {
        return Err(format!("{id}: not installed"));
    }
    let marker = directory.join(file_providers::DISABLED_FILE);
    if enabled {
        match fs::remove_file(&marker) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("{}: {error}", marker.display())),
        }
    } else {
        write_private(&marker, b"")
    }
}

/// Provider directories that no catalog entry owns.
pub(crate) fn manual_installations(root: &Path, catalog: &[Entry]) -> Vec<Installation> {
    #[derive(Deserialize)]
    struct Name {
        name: Option<String>,
    }
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut installations: Vec<Installation> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(|path| {
            let id = path.file_name()?.to_str()?.to_owned();
            if !file_providers::slug(&id) || catalog.iter().any(|entry| entry.id == id) {
                return None;
            }
            let name = fs::read(path.join(MANIFEST_FILE))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Name>(&bytes).ok())
                .and_then(|manifest| manifest.name)
                .filter(|name| plain_text(name, 64))
                .unwrap_or_else(|| id.clone());
            let icon = file_providers::load(&path).and_then(|registration| {
                let icon = registration.manifest.icon.as_ref()?;
                registration.icons.get(icon).cloned()
            });
            let enabled = !path.join(file_providers::DISABLED_FILE).exists();
            Some(Installation {
                id,
                name,
                icon,
                enabled,
            })
        })
        .collect();
    installations.sort_by(|a, b| a.id.cmp(&b.id));
    installations
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let describe = |error: std::io::Error| format!("{}: {error}", path.display());
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(format!("{} is a symbolic link", path.display()));
    }
    let parent = path.parent().ok_or("destination has no parent")?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("destination has no name")?;
    let temporary = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(describe)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    written
        .and_then(|()| fs::rename(&temporary, path))
        .map_err(|error| {
            let _ = fs::remove_file(&temporary);
            describe(error)
        })
}

fn private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("{} is a symbolic link", path.display()))
        }
        Ok(metadata) if !metadata.is_dir() => Err(format!("{} is not a folder", path.display())),
        Ok(metadata) if metadata.mode() & 0o022 != 0 => Err(format!(
            "{} is writable by other users, so Strata will not load providers from it",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(_) => {
            fs::create_dir(path).map_err(|error| format!("{}: {error}", path.display()))?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("{}: {error}", path.display()))
        }
    }
}

/// Copies the entry into `root/<id>`, registers it with the interpreter and
/// script paths resolved now, and records what was written. Re-installing
/// replaces the files and removes ones the previous installation recorded.
pub(crate) fn install(entry: &Entry, root: &Path) -> Result<PathBuf, String> {
    let interpreter = crate::trusted_command::resolve(&entry.runtime)
        .map_err(|_| format!("{} is not installed in a system directory", entry.runtime))?;
    if let Some(parent) = root.parent()
        && !is_directory(parent)
    {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    private_directory(root)?;
    let directory = root.join(&entry.id);
    private_directory(&directory)?;
    let previous = read_record(&directory).map(|record| record.files);
    let manifest = serde_json::json!({
        "version": 1,
        "id": entry.id,
        "name": entry.name,
        "command": [interpreter, directory.join(&entry.entry)],
        "icon": entry.icon,
        "icons": entry.icons,
        "category": entry.category.id(),
    });
    let mut files: Vec<(&str, Vec<u8>)> = entry
        .files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.clone()))
        .collect();
    files.push((
        MANIFEST_FILE,
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    ));
    let mut record = Record {
        version: 1,
        catalog: entry.hash.clone(),
        files: BTreeMap::new(),
    };
    for (name, bytes) in &files {
        write_private(&directory.join(name), bytes)?;
        record.files.insert((*name).to_owned(), sha256(bytes));
    }
    write_private(
        &directory.join(RECORD_FILE),
        &serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?,
    )?;
    for stale in previous.into_iter().flatten().map(|(name, _)| name) {
        if single_basename(&stale) && !record.files.contains_key(&stale) {
            let _ = fs::remove_file(directory.join(stale));
        }
    }
    Ok(directory)
}

/// Deletes `root/<id>`, whether Strata or the user installed it.
pub(crate) fn remove(root: &Path, id: &str) -> Result<(), String> {
    if !file_providers::slug(id) {
        return Err(format!("{id:?} is not a provider id"));
    }
    let directory = root.join(id);
    let metadata = fs::symlink_metadata(&directory)
        .map_err(|_| format!("{} is not installed", directory.display()))?;
    let removed = if metadata.is_dir() {
        fs::remove_dir_all(&directory)
    } else {
        fs::remove_file(&directory)
    };
    removed.map_err(|error| format!("{}: {error}", directory.display()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequirementResult {
    pub requirement: Requirement,
    pub satisfied: bool,
}

/// Looks up each requirement without running provider code. Module checks
/// ask the interpreter whether the module can be found, not to import it.
pub(crate) fn check_requirements(entry: &Entry) -> Vec<RequirementResult> {
    let interpreter = crate::trusted_command::resolve(&entry.runtime).ok();
    entry
        .requirements()
        .into_iter()
        .map(|requirement| {
            let satisfied = match (&requirement.command, &requirement.module) {
                (Some(command), _) => std::iter::once(command)
                    .chain(&requirement.alternatives)
                    .any(|name| crate::trusted_command::resolve_installed(name).is_ok()),
                (None, Some(module)) => interpreter
                    .as_ref()
                    .is_some_and(|interpreter| module_available(interpreter, module)),
                (None, None) => false,
            };
            RequirementResult {
                requirement,
                satisfied,
            }
        })
        .collect()
}

fn module_available(interpreter: &Path, module: &str) -> bool {
    let Ok(mut child) = Command::new(interpreter)
        .args([
            "-I",
            "-c",
            "import importlib.util, sys\nsys.exit(0 if importlib.util.find_spec(sys.argv[1]) else 1)",
            module,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + MODULE_CHECK_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}
