// SPDX-License-Identifier: MIT
// Frozen published installer functions; transport and metadata callbacks are supplied by the test.

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

    Ok(())
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
