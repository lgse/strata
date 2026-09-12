// SPDX-License-Identifier: MIT
// Frozen published installer functions; transport and metadata callbacks are supplied by the test.

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
    if let Err(error) = crate::portal_setup::refresh_after_in_place_update() {
        tracing::warn!(%error, "could not refresh the configured Strata portal after updating");
    }

    Ok(())
}

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
