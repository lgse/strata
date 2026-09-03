// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

use gtk::glib;

use crate::services::{InstallSource, ensure_self_managed};

use super::release_channel::Version;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const DESKTOP_ENTRY: &str = "io.github.lgse.Strata.desktop";
const APPLICATION_ICON: &str = "io.github.lgse.Strata.svg";
const PACMAN: &str = "/usr/bin/pacman";
const PACMAN_CONF: &str = "/usr/bin/pacman-conf";
const OS_RELEASE: &str = "/etc/os-release";
const PACKAGE_NAME: &str = "strata";
const REPOSITORY_DATABASE_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateMethod {
    InPlace,
    MarkedPackage,
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
    Failed(String),
}

/// What to install: the archive to download.
///
/// Earlier versions of this type also carried the expected `version`
/// string, but nothing ever read it: `install_update` only uses
/// `download_url`, and the rollback caller (which was documented as
/// needing it to flip the persisted channel afterwards) sets
/// `Channel::Stable` unconditionally instead. Removed rather than wired up
/// -- verifying the extracted binary against an expected version would
/// mean executing an untrusted downloaded binary before replacing the
/// installed one, which is a bigger change than this field's one dead
/// reader justified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallRequest {
    pub download_url: String,
}

/// Determines whether the running executable may be updated in place or is
/// owned by pacman and must be updated through the operating system.
pub fn update_method() -> UpdateMethod {
    if InstallSource::detect().is_managed() {
        return UpdateMethod::MarkedPackage;
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
pub fn install_update(request: InstallRequest) -> Receiver<UpdateInstall> {
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("strata-update-install".into())
        .spawn(move || {
            let outcome = match perform_install(&request.download_url, &sender) {
                Ok(()) => UpdateInstall::Installed,
                Err(message) => UpdateInstall::Failed(message),
            };
            let _sent = sender.send(outcome);
        });
    drop(spawned);
    receiver
}

fn perform_install(download_url: &str, progress: &Sender<UpdateInstall>) -> Result<(), String> {
    ensure_self_managed(InstallSource::detect())?;
    match update_method() {
        UpdateMethod::InPlace => {}
        UpdateMethod::MarkedPackage => {
            return Err("This installation is managed by its package manager.".to_owned());
        }
        UpdateMethod::Omarchy => {
            return Err(
                "This installation is managed by Omarchy; install updates with `omarchy update`."
                    .to_owned(),
            );
        }
        UpdateMethod::Pacman => {
            return Err(
                "This installation is managed by pacman; install updates through a full system update."
                    .to_owned(),
            );
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
    let workdir = stage_workdir(exe_dir)?;
    try_install(
        download_url,
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
    download_url: &str,
    workdir: &Path,
    exe_dir: &Path,
    current_exe: &Path,
    progress: &Sender<UpdateInstall>,
) -> Result<(), String> {
    let archive_path = workdir.join("strata.tar.gz");
    download_to_file(download_url, &archive_path, progress)?;
    let _sent = progress.send(UpdateInstall::Verifying);
    verify_checksum(download_url, &archive_path)?;
    let _sent = progress.send(UpdateInstall::Installing);

    let extract_dir = workdir.join("extracted");
    fs::create_dir_all(&extract_dir).map_err(|error| error.to_string())?;
    run(Command::new("tar")
        .arg("-xzf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&extract_dir))?;

    let binary_paths = find_binaries(&extract_dir, &["strata"])?;
    let binary_path = binary_paths
        .first()
        .ok_or_else(|| "Could not find the strata binary in the downloaded archive".to_owned())?;
    let staged = stage_binary_path(exe_dir)?;
    fs::copy(binary_path, staged.path())
        .map_err(|error| format!("Could not stage the new binary: {error}"))?;
    set_executable(staged.path())?;
    staged
        .persist(current_exe)
        .map_err(|error| format!("Could not replace the installed binary: {error}"))?;

    if let Some(package_dir) = binary_path.parent() {
        refresh_desktop_metadata(package_dir, current_exe, &glib::user_data_dir());
    }

    Ok(())
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
    progress: &Sender<UpdateInstall>,
) -> Result<(), String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent
        .get(url)
        .header("User-Agent", "strata-file-manager")
        .call()
        .map_err(|error| format!("Could not download the update: {error}"))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    let mut reader = response.body_mut().as_reader();
    let mut file = fs::File::create(destination)
        .map_err(|error| format!("Could not save the update: {error}"))?;
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    let _sent = progress.send(UpdateInstall::Downloading { downloaded, total });
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("Could not download the update: {error}"))?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count])
            .map_err(|error| format!("Could not save the update: {error}"))?;
        downloaded = downloaded.saturating_add(count as u64);
        let _sent = progress.send(UpdateInstall::Downloading { downloaded, total });
    }
    Ok(())
}

fn verify_checksum(download_url: &str, archive_path: &Path) -> Result<(), String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build();
    let agent: ureq::Agent = config.into();
    let checksum_url = format!("{download_url}.sha256");
    let expected = agent
        .get(&checksum_url)
        .header("User-Agent", "strata-file-manager")
        .call()
        .and_then(|mut response| response.body_mut().read_to_string())
        .map_err(|error| format!("Could not verify the update: {error}"))?;
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

fn first_hash_token(text: &str) -> Option<String> {
    text.split_whitespace().next().map(str::to_ascii_lowercase)
}

/// Locates each of `names` as a nested file within `extract_dir` (searching one
/// level down, matching the layout of the release archives). Returns their paths
/// in the same order as `names`, or an error naming the first one not found.
///
/// This is the seam issue #59 would need if it ever ships a second executable:
/// today it is always called with a single name, and no caller performs a
/// multi-file transactional install.
fn find_binaries(extract_dir: &Path, names: &[&str]) -> Result<Vec<PathBuf>, String> {
    let entries: Vec<_> = fs::read_dir(extract_dir)
        .map_err(|error| error.to_string())?
        .flatten()
        .collect();
    names
        .iter()
        .map(|name| {
            entries
                .iter()
                .map(|entry| entry.path().join(name))
                .find(|candidate| candidate.is_file())
                .ok_or_else(|| {
                    format!("Could not find the {name} binary in the downloaded archive")
                })
        })
        .collect()
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
mod tests;
