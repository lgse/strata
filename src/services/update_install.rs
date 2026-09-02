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

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const DESKTOP_ENTRY: &str = "io.github.lgse.Strata.desktop";
const APPLICATION_ICON: &str = "io.github.lgse.Strata.svg";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateInstall {
    Downloading { downloaded: u64, total: Option<u64> },
    Installing,
    Installed,
    Failed(String),
}

/// Downloads, verifies, and installs `download_url` in place of the running executable.
/// Runs off the GTK thread and reports the outcome once. Mirrors the manual install
/// steps in the README: fetch the release archive, check its published `sha256`, and
/// extract the `strata` binary over the current install.
pub fn install_update(download_url: String) -> Receiver<UpdateInstall> {
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("strata-update-install".into())
        .spawn(move || {
            let outcome = match perform_install(&download_url, &sender) {
                Ok(()) => UpdateInstall::Installed,
                Err(message) => UpdateInstall::Failed(message),
            };
            let _sent = sender.send(outcome);
        });
    drop(spawned);
    receiver
}

fn perform_install(download_url: &str, progress: &Sender<UpdateInstall>) -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| "Could not determine the install directory".to_owned())?;

    let workdir = exe_dir.join(format!(".strata-update-{}", std::process::id()));
    fs::create_dir_all(&workdir).map_err(|error| format!("Could not stage the update: {error}"))?;
    let cleanup = || {
        let _ = fs::remove_dir_all(&workdir);
    };

    let result = try_install(download_url, &workdir, exe_dir, &current_exe, progress);
    cleanup();
    result
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
    let _sent = progress.send(UpdateInstall::Installing);
    verify_checksum(download_url, &archive_path)?;

    let extract_dir = workdir.join("extracted");
    fs::create_dir_all(&extract_dir).map_err(|error| error.to_string())?;
    run(Command::new("tar")
        .arg("-xzf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&extract_dir))?;

    let binary_path = find_binary(&extract_dir)?;
    let staged = exe_dir.join(format!(".strata-update-{}.tmp", std::process::id()));
    fs::copy(&binary_path, &staged)
        .map_err(|error| format!("Could not stage the new binary: {error}"))?;
    set_executable(&staged)?;
    fs::rename(&staged, current_exe)
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
    if let Err(error) = write_application_icon(package_dir, data_home) {
        tracing::warn!("could not refresh the application icon: {error}");
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

fn find_binary(extract_dir: &Path) -> Result<PathBuf, String> {
    let entries = fs::read_dir(extract_dir).map_err(|error| error.to_string())?;
    for entry in entries.flatten() {
        let candidate = entry.path().join("strata");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("Could not find the strata binary in the downloaded archive".to_owned())
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
