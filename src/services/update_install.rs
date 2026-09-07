// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

use gtk::glib;
use serde::Deserialize;

use crate::services::{InstallSource, ensure_self_managed};

use super::release_channel::Version;

mod archive;
mod command;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const RELEASE_DOWNLOAD_ROOT: &str = "https://github.com/lgse/strata/releases/download";
const REPOSITORY: &str = "lgse/strata";
const MAX_UPDATE_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CHECKSUM_BYTES: u64 = 8 * 1024;
const DESKTOP_ENTRY: &str = "io.github.lgse.Strata.desktop";
const APPLICATION_ICON: &str = "io.github.lgse.Strata.svg";
const AUR_RPC: &str = "https://aur.archlinux.org/rpc/v5/info";
const AUR_RESPONSE_LIMIT: u64 = 1024 * 1024;
const PACMAN: &str = "/usr/bin/pacman";
const PACMAN_CONF: &str = "/usr/bin/pacman-conf";
const OS_RELEASE: &str = "/etc/os-release";
const PACKAGE_NAME: &str = "strata";
const REPOSITORY_DATABASE_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateMethod {
    InPlace,
    Aur,
    Omarchy,
    Pacman,
}

impl UpdateMethod {
    pub fn is_package_managed(self) -> bool {
        self != Self::InPlace
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateInstall {
    Downloading { downloaded: u64, total: Option<u64> },
    Verifying,
    Installing,
    Installed,
    Cancelled,
    Failed(String),
}

#[derive(Clone, Debug, Default)]
pub struct InstallCancel(Arc<AtomicBool>);

impl InstallCancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn check(&self) -> Result<(), InstallStop> {
        if self.is_cancelled() {
            return Err(InstallStop::Cancelled);
        }
        Ok(())
    }
}

#[derive(Debug)]
enum InstallStop {
    Cancelled,
    Failed(String),
}

impl From<String> for InstallStop {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallRequest {
    pub tag: String,
    pub asset_name: String,
    pub advertised_url: String,
}

fn verified_download_url(request: &InstallRequest) -> Result<String, String> {
    let version = request.tag.strip_prefix('v').and_then(Version::parse);
    if !is_release_token(&request.tag)
        || version.as_ref().is_none_or(|version| {
            request.asset_name != super::update_check::archive_name(&version.to_string())
        })
    {
        return Err("The release names an unexpected update artifact.".to_owned());
    }
    let expected = format!(
        "{RELEASE_DOWNLOAD_ROOT}/{}/{}",
        request.tag, request.asset_name
    );
    if request.advertised_url != expected {
        return Err("The release points at an unexpected download location.".to_owned());
    }
    Ok(expected)
}

fn is_release_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-+".contains(character))
}

/// Determines whether the running executable may be updated in place or is
/// owned by pacman and must be updated through the operating system.
pub fn update_method() -> UpdateMethod {
    if InstallSource::detect().is_managed() {
        return UpdateMethod::Aur;
    }
    let Ok(executable) = std::env::current_exe() else {
        return UpdateMethod::InPlace;
    };
    update_method_for(&executable, Path::new(PACMAN), Path::new(OS_RELEASE))
}

fn update_method_for(executable: &Path, pacman: &Path, os_release: &Path) -> UpdateMethod {
    let package_owned = match Command::new(pacman)
        .args(["--query", "--owns", "--quiet", "--"])
        .arg(executable)
        .output()
    {
        Ok(output) => output.status.success(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            tracing::warn!(%error, "could not query pacman ownership; disabling in-place updates");
            true
        }
    };
    if !package_owned {
        return UpdateMethod::InPlace;
    }

    if fs::read_to_string(os_release).is_ok_and(|contents| os_release_has_id(&contents, "omarchy"))
    {
        UpdateMethod::Omarchy
    } else {
        UpdateMethod::Pacman
    }
}

fn os_release_has_id(contents: &str, expected: &str) -> bool {
    contents.lines().any(|line| {
        line.strip_prefix("ID=")
            .map(|value| value.trim_matches(|character| character == '\'' || character == '"'))
            == Some(expected)
    })
}

/// Returns the Strata version currently offered by pacman's configured sync
/// databases. This is deliberately a local query: package-managed installs
/// must not advertise a GitHub release until their package manager can
/// actually install it.
pub(super) fn package_repository_version() -> Result<Version, String> {
    package_repository_version_for(Path::new(PACMAN), PACKAGE_NAME)
}

#[derive(Deserialize)]
struct AurResponse {
    #[serde(default)]
    results: Vec<AurPackage>,
}

#[derive(Deserialize)]
struct AurPackage {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Version")]
    version: String,
}

pub(super) fn aur_repository_version() -> Result<Version, String> {
    let package = InstallSource::detect()
        .managed()
        .and_then(|managed| managed.package())
        .ok_or_else(|| "the AUR packaging marker does not name a package".to_owned())?;
    if !package
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"@._+-".contains(&byte))
    {
        return Err("the AUR packaging marker contains an invalid package name".to_owned());
    }

    let url = format!("{AUR_RPC}?arg[]={package}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|error| format!("could not query the AUR: {error}"))?;
    let mut contents = String::new();
    response
        .body_mut()
        .as_reader()
        .take(AUR_RESPONSE_LIMIT + 1)
        .read_to_string(&mut contents)
        .map_err(|error| format!("could not read the AUR response: {error}"))?;
    if contents.len() as u64 > AUR_RESPONSE_LIMIT {
        return Err("the AUR response exceeded the size limit".to_owned());
    }
    aur_repository_version_from_response(&contents, package)
}

fn aur_repository_version_from_response(contents: &str, package: &str) -> Result<Version, String> {
    let response: AurResponse = serde_json::from_str(contents)
        .map_err(|error| format!("the AUR returned an invalid response: {error}"))?;
    response
        .results
        .into_iter()
        .find(|result| result.name == package)
        .and_then(|result| parse_aur_package_version(&result.version))
        .ok_or_else(|| format!("{package} is not available in the AUR"))
}

fn parse_aur_package_version(value: &str) -> Option<Version> {
    let value = value.lines().find(|line| !line.trim().is_empty())?.trim();
    let value = value.split_once(':').map_or(value, |(_, version)| version);
    let (upstream, package_release) = value.rsplit_once('-')?;
    if package_release.is_empty() {
        return None;
    }
    if let Some(version) = Version::parse(upstream) {
        return Some(version);
    }
    for kind in ["alpha", "beta", "rc", "nightly"] {
        if let Some(index) = upstream.find(kind) {
            let mut release = upstream.to_owned();
            release.insert(index, '-');
            return Version::parse(&release);
        }
    }
    None
}

/// Reads the live Omarchy repository database selected by this installation,
/// rather than pacman's cached copy. The cache is normally refreshed by the
/// same `omarchy update` that installs Strata, so consulting it would make the
/// notification arrive only after the update had already been installed.
pub(super) fn omarchy_repository_version() -> Result<Version, String> {
    let server = omarchy_repository_server(Path::new(PACMAN_CONF))?;
    let database_url = format!("{}/omarchy.db", server.trim_end_matches('/'));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut response = agent
        .get(&database_url)
        .call()
        .map_err(|error| format!("could not query the Omarchy repository: {error}"))?;
    let mut database = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(REPOSITORY_DATABASE_LIMIT + 1)
        .read_to_end(&mut database)
        .map_err(|error| format!("could not read the Omarchy repository: {error}"))?;
    if database.len() as u64 > REPOSITORY_DATABASE_LIMIT {
        return Err("Omarchy repository database exceeded the size limit".to_owned());
    }
    repository_database_version(&database, PACKAGE_NAME)
}

fn omarchy_repository_server(pacman_conf: &Path) -> Result<String, String> {
    let output = Command::new(pacman_conf)
        .args(["--repo", "omarchy", "Server"])
        .output()
        .map_err(|error| format!("could not read the Omarchy repository configuration: {error}"))?;
    if !output.status.success() {
        return Err("Omarchy repository is not configured".to_owned());
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|_| "Omarchy repository configuration is invalid".to_owned())?;
    output
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("https://") || line.starts_with("http://"))
        .map(str::to_owned)
        .ok_or_else(|| "Omarchy repository configuration has no package server".to_owned())
}

fn repository_database_version(database: &[u8], package: &str) -> Result<Version, String> {
    let decoder = zstd::stream::read::Decoder::new(database)
        .map_err(|error| format!("could not decode the Omarchy repository: {error}"))?;
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| format!("could not read the Omarchy repository: {error}"))?;
    for entry in entries {
        let mut entry =
            entry.map_err(|error| format!("could not read the Omarchy repository: {error}"))?;
        let is_description = entry
            .path()
            .is_ok_and(|path| path.to_string_lossy().ends_with("/desc"));
        if !is_description {
            continue;
        }
        let mut description = String::new();
        if entry.read_to_string(&mut description).is_err() {
            continue;
        }
        if repository_description_field(&description, "NAME") == Some(package) {
            return repository_description_field(&description, "VERSION")
                .and_then(parse_package_version)
                .ok_or_else(|| "Omarchy repository returned an invalid version".to_owned());
        }
    }
    Err("Strata is not available in the Omarchy repository".to_owned())
}

fn repository_description_field<'a>(description: &'a str, field: &str) -> Option<&'a str> {
    let marker = format!("%{field}%");
    let mut lines = description.lines();
    while let Some(line) = lines.next() {
        if line == marker {
            return lines
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty());
        }
    }
    None
}

fn package_repository_version_for(pacman: &Path, package: &str) -> Result<Version, String> {
    let output = Command::new(pacman)
        .args([
            "--sync",
            "--print",
            "--print-format",
            "%n %v",
            "--",
            package,
        ])
        .output()
        .map_err(|error| format!("could not query the package repository: {error}"))?;
    if !output.status.success() {
        return Err("package is not available in the configured repositories".to_owned());
    }

    let output = String::from_utf8(output.stdout)
        .map_err(|_| "package repository returned an invalid version".to_owned())?;
    output
        .lines()
        .filter_map(|line| line.trim().split_once(char::is_whitespace))
        .find(|(name, _version)| *name == package)
        .and_then(|(_name, version)| parse_package_version(version.trim()))
        .ok_or_else(|| "package repository returned an invalid version".to_owned())
}

fn parse_package_version(value: &str) -> Option<Version> {
    let value = value.lines().find(|line| !line.trim().is_empty())?.trim();
    let value = value.split_once(':').map_or(value, |(_, version)| version);
    let (version, package_release) = value.rsplit_once('-')?;
    if package_release.is_empty() {
        return None;
    }
    Version::parse(version)
}

/// Downloads, verifies, and installs `request`'s archive in place of the running
/// executable. Package-owned executables are rejected before download.
pub fn install_update(request: InstallRequest, cancel: InstallCancel) -> Receiver<UpdateInstall> {
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("strata-update-install".into())
        .spawn(move || {
            let outcome = match perform_install(&request, &cancel, &sender) {
                Ok(()) => UpdateInstall::Installed,
                Err(InstallStop::Cancelled) => UpdateInstall::Cancelled,
                Err(InstallStop::Failed(message)) => UpdateInstall::Failed(message),
            };
            let _sent = sender.send(outcome);
        });
    drop(spawned);
    receiver
}

fn perform_install(
    request: &InstallRequest,
    cancel: &InstallCancel,
    progress: &Sender<UpdateInstall>,
) -> Result<(), InstallStop> {
    ensure_self_managed(InstallSource::detect())?;
    match update_method() {
        UpdateMethod::InPlace => {}
        UpdateMethod::Aur => {
            return Err(InstallStop::Failed(
                "This installation is managed by its package manager.".to_owned(),
            ));
        }
        UpdateMethod::Omarchy => {
            return Err(InstallStop::Failed(
                "This installation is managed by Omarchy; install updates with `omarchy update`."
                    .to_owned(),
            ));
        }
        UpdateMethod::Pacman => {
            return Err(InstallStop::Failed(
                "This installation is managed by pacman; install updates through a full system update."
                    .to_owned(),
            ));
        }
    }

    let current_exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| "Could not determine the install directory".to_owned())?;

    // A unique-per-install directory, not the old process-scoped
    // `.strata-update-{pid}`: with three independent install drivers (the
    // update row, rollback, and the update dialog) that can all be reachable
    // at once on a preview build, a shared path was how two installs racing
    // over the same archive and staged binary corrupted each other. The
    // shared `InstallGuard` in `ui::settings` now also prevents a second
    // install from starting at all, but a unique path is kept as its own
    // layer of defense -- e.g. against a leftover directory from a
    // hard-killed previous process.
    let download_url = verified_download_url(request)?;
    let workdir = stage_workdir(exe_dir)?;
    try_install(
        request,
        &download_url,
        cancel,
        workdir.path(),
        exe_dir,
        &current_exe,
        progress,
    )
    // `workdir` removes its directory on drop here, on both the success and
    // error paths, replacing the old manual `remove_dir_all` cleanup.
}

/// Creates a fresh, uniquely-named staging directory for one install inside
/// `exe_dir`. See `perform_install` for why uniqueness matters.
fn stage_workdir(exe_dir: &Path) -> Result<tempfile::TempDir, String> {
    tempfile::Builder::new()
        .prefix(".strata-update-")
        .tempdir_in(exe_dir)
        .map_err(|error| format!("Could not stage the update: {error}"))
}

/// Creates a fresh, uniquely-named path for the staged replacement binary
/// inside `exe_dir`, for the same reason as `stage_workdir`.
fn stage_binary_path(exe_dir: &Path) -> Result<tempfile::NamedTempFile, String> {
    tempfile::Builder::new()
        .prefix(".strata-update-")
        .suffix(".tmp")
        .tempfile_in(exe_dir)
        .map_err(|error| format!("Could not stage the new binary: {error}"))
}

fn try_install(
    request: &InstallRequest,
    download_url: &str,
    cancel: &InstallCancel,
    workdir: &Path,
    exe_dir: &Path,
    current_exe: &Path,
    progress: &Sender<UpdateInstall>,
) -> Result<(), InstallStop> {
    let archive_path = workdir.join("strata.tar.gz");
    download_to_file(download_url, &archive_path, cancel, progress)?;

    let _sent = progress.send(UpdateInstall::Verifying);
    cancel.check()?;
    verify_checksum(download_url, &archive_path)?;
    cancel.check()?;
    verify_provenance(&archive_path, cancel)?;
    cancel.check()?;

    let _sent = progress.send(UpdateInstall::Installing);
    let extract_dir = workdir.join("extracted");
    fs::create_dir_all(&extract_dir).map_err(|error| error.to_string())?;
    let package_dir = archive::extract_release_archive(&archive_path, &extract_dir)?;
    verify_package_name(&package_dir, request)?;
    verify_source_commit(&package_dir, &request.tag)?;
    cancel.check()?;

    let binary_path = package_dir.join("strata");
    let staged = stage_verified_binary(&binary_path, exe_dir)?;

    cancel.check()?;
    let rollback = stage_rollback(current_exe, exe_dir)?;
    if let Err(stop) = cancel.check() {
        let _removed = fs::remove_file(&rollback);
        return Err(stop);
    }
    if let Err(error) = staged.persist(current_exe) {
        let _removed = fs::remove_file(&rollback);
        return Err(InstallStop::Failed(format!(
            "Could not replace the installed binary: {error}"
        )));
    }

    if let Err(error) = sync_directory(exe_dir).and_then(|()| confirm_replacement(current_exe)) {
        restore_rollback(&rollback, current_exe)?;
        return Err(InstallStop::Failed(error));
    }

    refresh_desktop_metadata(&package_dir, current_exe, &glib::user_data_dir());
    if let Err(error) = crate::portal_setup::refresh_after_in_place_update() {
        tracing::warn!(%error, "could not refresh the configured Strata portal after updating");
    }

    Ok(())
}

pub(crate) fn rollback_path(exe_dir: &Path) -> PathBuf {
    exe_dir.join(".strata-update-rollback")
}

fn stage_rollback(current_exe: &Path, exe_dir: &Path) -> Result<PathBuf, String> {
    let rollback = rollback_path(exe_dir);
    let staged = stage_binary_path(exe_dir)?;
    fs::copy(current_exe, staged.path())
        .map_err(|error| format!("Could not preserve the current version: {error}"))?;
    set_executable(staged.path())?;
    staged
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    staged
        .persist(&rollback)
        .map_err(|error| error.to_string())?;
    sync_directory(exe_dir)?;
    Ok(rollback)
}

fn sync_directory(directory: &Path) -> Result<(), String> {
    fs::File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("Could not synchronize the install directory: {error}"))
}

fn restore_rollback(rollback: &Path, current_exe: &Path) -> Result<(), InstallStop> {
    fs::rename(rollback, current_exe).map_err(|error| {
        InstallStop::Failed(format!(
            "The update failed and the previous version could not be restored: {error}. \
             Reinstall Strata from the release page."
        ))
    })?;
    if let Some(directory) = current_exe.parent() {
        sync_directory(directory)?;
    }
    Ok(())
}

fn stage_verified_binary(binary: &Path, directory: &Path) -> Result<tempfile::TempPath, String> {
    let staged = stage_binary_path(directory)?;
    fs::copy(binary, staged.path())
        .map_err(|error| format!("Could not stage the new binary: {error}"))?;
    set_executable(staged.path())?;
    staged
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    // Linux refuses exec while any writable descriptor remains open (ETXTBSY).
    let staged = staged.into_temp_path();
    verify_staged_binary(&staged)?;
    Ok(staged)
}

fn binary_probe_output(staged: &Path) -> Result<std::process::Output, String> {
    // Published releases lack --version; this existing headless probe loads their runtime libraries.
    command::run(
        Command::new(staged)
            .arg("--gvfs-probe")
            .env("GIO_USE_VFS", "local")
            .env("GIO_USE_VOLUME_MONITOR", "unix"),
        &InstallCancel::new(),
        Duration::from_secs(2),
    )
    .map_err(|stop| match stop {
        InstallStop::Failed(message) => message,
        InstallStop::Cancelled => "Update cancelled".to_owned(),
    })
}

fn verify_staged_binary(staged: &Path) -> Result<(), String> {
    if binary_probe_output(staged)?.status.success() {
        Ok(())
    } else {
        Err("The downloaded update does not run on this system".to_owned())
    }
}

fn verify_package_name(package: &Path, request: &InstallRequest) -> Result<(), String> {
    if package.file_name().and_then(|name| name.to_str())
        == request.asset_name.strip_suffix(".tar.gz")
    {
        Ok(())
    } else {
        Err("The authenticated archive does not contain the selected release package".to_owned())
    }
}

fn confirm_replacement(current_exe: &Path) -> Result<(), String> {
    if !current_exe.is_file() {
        return Err("The updated executable is missing after installation".to_owned());
    }
    verify_staged_binary(current_exe)
}

fn verify_source_commit(package_dir: &Path, tag: &str) -> Result<(), String> {
    let packaged = fs::read_to_string(package_dir.join("SOURCE_COMMIT"))
        .map_err(|error| format!("Could not read the update's source commit: {error}"))?;
    let source_ref = release_source_ref(tag)?;
    let expected = super::update_check::fetch_commit(&source_ref)
        .ok_or_else(|| "Could not confirm which commit this release was built from".to_owned())?;
    commits_match(&packaged, &expected)
}

fn release_source_ref(tag: &str) -> Result<String, String> {
    let version = tag
        .strip_prefix('v')
        .and_then(Version::parse)
        .ok_or_else(|| "The release tag is invalid".to_owned())?;
    // Stable tags mark the version-bump commit; release.yml builds its parent.
    Ok(if version.to_string().contains('-') {
        tag.to_owned()
    } else {
        format!("{tag}~1")
    })
}

fn commits_match(packaged: &str, expected: &str) -> Result<(), String> {
    let packaged = packaged.trim().to_ascii_lowercase();
    let expected = expected.trim().to_ascii_lowercase();
    if packaged.is_empty() || expected.is_empty() {
        return Err("This release does not identify the commit it was built from".to_owned());
    }
    if packaged == expected {
        return Ok(());
    }
    Err("The update was not built from this release's commit".to_owned())
}

fn provenance_command(archive: &Path, config: &Path) -> Command {
    let mut command = Command::new("gh");
    command
        .args(["attestation", "verify"])
        .arg(archive)
        .args([
            "--repo",
            REPOSITORY,
            "--signer-workflow",
            "lgse/strata/.github/workflows/release.yml",
            "--hostname",
            "github.com",
        ])
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_ENTERPRISE_TOKEN")
        .env_remove("GITHUB_ENTERPRISE_TOKEN")
        .env("GH_CONFIG_DIR", config)
        .env("GH_HOST", "github.com")
        .env("GH_PROMPT_DISABLED", "1")
        // gh probes Secret Service even with empty config; public verification must not open a keyring.
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/dev/null");
    command
}

fn verify_provenance(archive: &Path, cancel: &InstallCancel) -> Result<(), InstallStop> {
    cancel.check()?;
    let directory = archive
        .parent()
        .ok_or_else(|| "Could not locate the update staging directory".to_owned())?;
    let config = tempfile::Builder::new()
        .prefix("gh-config-")
        .tempdir_in(directory)
        .map_err(|error| format!("Could not isolate update verification: {error}"))?;
    let output = command::run(
        &mut provenance_command(archive, config.path()),
        cancel,
        REQUEST_TIMEOUT,
    );
    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            tracing::warn!(
                stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                "update provenance verification failed"
            );
            Err(InstallStop::Failed(
                "The downloaded update failed build provenance verification".to_owned(),
            ))
        }
        Err(InstallStop::Cancelled) => Err(InstallStop::Cancelled),
        Err(InstallStop::Failed(message)) => Err(InstallStop::Failed(format!(
            "Could not verify the update with GitHub CLI (gh): {message}. Install gh or download the release manually."
        ))),
    }
}

/// Rewrites an already installed desktop entry and application icon from the
/// downloaded archive so an in-app update cannot leave desktop metadata stale.
/// Absent metadata is never created: a user who did not install a launcher does
/// not gain one from an update. Failures are reported but never fail the update,
/// which has already replaced the binary.
fn refresh_desktop_metadata(package_dir: &Path, executable: &Path, data_home: &Path) {
    let entry_path = data_home.join("applications").join(DESKTOP_ENTRY);
    if !entry_path.is_file() {
        return;
    }

    if let Err(error) = write_desktop_entry(package_dir, executable, &entry_path) {
        tracing::warn!("could not refresh the desktop entry: {error}");
    }
    match write_application_icon(package_dir, data_home) {
        Ok(()) => {
            if let Err(error) = run(Command::new("gtk-update-icon-cache")
                .arg("-qtf")
                .arg(data_home.join("icons/hicolor")))
            {
                tracing::warn!("could not refresh the application icon cache: {error}");
            }
        }
        Err(error) => tracing::warn!("could not refresh the application icon: {error}"),
    }

    let _refreshed =
        run(Command::new("update-desktop-database").arg(data_home.join("applications")));
}

fn write_desktop_entry(
    package_dir: &Path,
    executable: &Path,
    entry_path: &Path,
) -> Result<(), String> {
    let staged_entry = package_dir.join(DESKTOP_ENTRY);
    if !staged_entry.is_file() {
        return Err(format!("the archive contains no {DESKTOP_ENTRY}"));
    }
    let template = fs::read_to_string(&staged_entry).map_err(|error| error.to_string())?;
    fs::write(entry_path, desktop_entry_with_exec(&template, executable))
        .map_err(|error| error.to_string())
}

fn write_application_icon(package_dir: &Path, data_home: &Path) -> Result<(), String> {
    let staged_icon = package_dir.join(APPLICATION_ICON);
    if !staged_icon.is_file() {
        return Err(format!("the archive contains no {APPLICATION_ICON}"));
    }
    let icon_dir = data_home.join("icons/hicolor/scalable/apps");
    fs::create_dir_all(&icon_dir).map_err(|error| error.to_string())?;
    fs::copy(&staged_icon, icon_dir.join(APPLICATION_ICON))
        .map(|_copied| ())
        .map_err(|error| error.to_string())
}

/// Points the packaged entry's `Exec` line at the running install path, keeping
/// the packaged field codes so the entry still receives directory arguments.
fn desktop_entry_with_exec(template: &str, executable: &Path) -> String {
    let program = executable.display().to_string();
    let program = if program.contains(char::is_whitespace) {
        format!("\"{program}\"")
    } else {
        program
    };

    let mut entry = String::with_capacity(template.len() + program.len());
    for line in template.lines() {
        match line.strip_prefix("Exec=") {
            Some(command) => {
                let field_codes = command.split_once(' ').map_or("", |(_program, rest)| rest);
                entry.push_str("Exec=");
                entry.push_str(&program);
                if !field_codes.is_empty() {
                    entry.push(' ');
                    entry.push_str(field_codes);
                }
            }
            None => entry.push_str(line),
        }
        entry.push('\n');
    }
    entry
}

fn download_to_file(
    url: &str,
    destination: &Path,
    cancel: &InstallCancel,
    progress: &Sender<UpdateInstall>,
) -> Result<(), InstallStop> {
    download_to_file_bounded(url, destination, MAX_UPDATE_ARCHIVE_BYTES, cancel, progress)
}

fn download_to_file_bounded(
    url: &str,
    destination: &Path,
    limit: u64,
    cancel: &InstallCancel,
    progress: &Sender<UpdateInstall>,
) -> Result<(), InstallStop> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent
        .get(url)
        .header("User-Agent", "strata-file-manager")
        .call()
        .map_err(|error| format!("Could not download the update: {error}"))?;
    let total: Option<u64> = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    if total.is_some_and(|total| total > limit) {
        return Err(oversized_update());
    }

    let mut reader = response.body_mut().as_reader();
    let mut file = fs::File::create(destination)
        .map_err(|error| format!("Could not save the update: {error}"))?;
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    let _sent = progress.send(UpdateInstall::Downloading { downloaded, total });
    loop {
        cancel.check()?;
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("Could not download the update: {error}"))?;
        if count == 0 {
            break;
        }
        downloaded = downloaded.saturating_add(count as u64);
        if downloaded > limit {
            return Err(oversized_update());
        }
        file.write_all(&buffer[..count])
            .map_err(|error| format!("Could not save the update: {error}"))?;
        let _sent = progress.send(UpdateInstall::Downloading { downloaded, total });
    }
    Ok(())
}

fn oversized_update() -> InstallStop {
    InstallStop::Failed("The update is larger than expected and was not installed".to_owned())
}

fn verify_checksum(download_url: &str, archive_path: &Path) -> Result<(), String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build();
    let agent: ureq::Agent = config.into();
    let checksum_url = format!("{download_url}.sha256");
    let mut response = agent
        .get(&checksum_url)
        .header("User-Agent", "strata-file-manager")
        .call()
        .map_err(|error| format!("Could not verify the update: {error}"))?;
    let expected = read_checksum_body(&mut response)?;
    let expected_hash =
        first_hash_token(&expected).ok_or_else(|| "The published checksum was empty".to_owned())?;

    let output = run(Command::new("sha256sum").arg(archive_path))?;
    let actual_hash =
        first_hash_token(&output).ok_or_else(|| "sha256sum produced no output".to_owned())?;

    if actual_hash == expected_hash {
        Ok(())
    } else {
        Err("Downloaded update failed checksum verification".to_owned())
    }
}

fn read_checksum_body(response: &mut ureq::http::Response<ureq::Body>) -> Result<String, String> {
    if response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_CHECKSUM_BYTES)
    {
        return Err("The published checksum is too large".to_owned());
    }
    let mut body = String::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_CHECKSUM_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|error| error.to_string())?;
    if body.len() as u64 > MAX_CHECKSUM_BYTES {
        return Err("The published checksum is too large".to_owned());
    }
    Ok(body)
}

fn first_hash_token(text: &str) -> Option<String> {
    text.split_whitespace().next().map(str::to_ascii_lowercase)
}

fn set_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .map_err(|error| format!("Could not mark the update executable: {error}"))
}

fn run(command: &mut Command) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|error| format!("Could not run {:?}: {error}", command.get_program()))?;
    if !output.status.success() {
        return Err(format!(
            "{:?} failed: {}",
            command.get_program(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod tests;
