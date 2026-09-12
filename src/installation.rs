// SPDX-License-Identifier: MIT
use std::{
    fs,
    path::{Path, PathBuf},
};

fn versioned_bin_dir(executable: &Path) -> Option<&Path> {
    let version = executable.parent()?;
    let versions = version.parent()?;
    let root = versions.parent()?;
    let id = version.file_name()?.to_str()?;
    let digest = id.strip_prefix("legacy-").unwrap_or(id);
    (executable.file_name()? == "strata"
        && versions.file_name()? == "versions"
        && root.file_name()? == ".strata-bundles"
        && digest.len() == 64
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then(|| root.parent())
    .flatten()
}

pub(crate) fn bin_dir(executable: &Path) -> Option<&Path> {
    versioned_bin_dir(executable).or_else(|| executable.parent())
}

/// Integration entry points track the active version; helper discovery deliberately does not.
pub(crate) fn launch_path(executable: &Path) -> Result<PathBuf, String> {
    let executable = if !executable.exists() {
        executable
            .to_str()
            .and_then(|s| s.strip_suffix(" (deleted)"))
            .map(Path::new)
            .filter(|p| p.is_file())
            .unwrap_or(executable)
    } else {
        executable
    };
    let Some(bin) = versioned_bin_dir(executable) else {
        return Ok(executable.to_owned());
    };
    let bin = crate::media_helper::trusted_directory(bin)?;
    let launcher = bin.join("strata");
    if fs::read_link(&launcher).ok().as_deref() != Some(Path::new(".strata-bundles/current/strata"))
    {
        return Err(
            "The Strata bundle launcher is missing or changed; reinstall the complete bundle."
                .into(),
        );
    }
    Ok(launcher)
}

#[cfg(test)]
mod tests;
