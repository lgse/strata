// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_ENTRIES: usize = 512;
const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

// Allow extra regular files so future package additions do not strand older updaters.
const REQUIRED_ENTRIES: &[&str] = &["strata", "SOURCE_COMMIT"];

pub(super) fn extract_release_archive(
    archive: &Path,
    destination: &Path,
) -> Result<PathBuf, String> {
    let file = fs::File::open(archive)
        .map_err(|error| format!("Could not open the downloaded update: {error}"))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    let entries = tar
        .entries()
        .map(|entries| entries.raw(true))
        .map_err(|error| format!("Could not read the downloaded update: {error}"))?;

    let mut package_dir: Option<String> = None;
    let mut extracted = 0_u64;
    let mut count = 0_usize;

    for entry in entries {
        let mut entry = entry.map_err(|error| format!("Could not read the update: {error}"))?;

        count += 1;
        if count > MAX_ENTRIES {
            return Err(format!(
                "The update contains more than {MAX_ENTRIES} files and was rejected"
            ));
        }

        let path = entry
            .path()
            .map_err(|error| format!("The update contains an unreadable path: {error}"))?
            .into_owned();
        let relative = safe_relative_path(&path)?;
        let package = relative
            .components()
            .next()
            .and_then(|component| component.as_os_str().to_str())
            .ok_or_else(|| "The update contains a file outside its package directory".to_owned())?
            .to_owned();
        match &package_dir {
            Some(existing) if *existing != package => {
                return Err("The update contains more than one package directory".to_owned());
            }
            Some(_existing) => {}
            None => package_dir = Some(package),
        }

        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            return Err(format!(
                "The update contains an unsupported entry ({}) and was rejected",
                describe_entry_type(kind)
            ));
        }

        let target = destination.join(&relative);
        if kind.is_dir() {
            if entry.size() != 0 {
                return Err("The update contains a directory with file data".to_owned());
            }
            fs::create_dir_all(&target)
                .map_err(|error| format!("Could not extract the update: {error}"))?;
            continue;
        }

        let size = entry.header().size().unwrap_or(u64::MAX);
        if size > MAX_ENTRY_BYTES {
            return Err("The update contains an oversized file and was rejected".to_owned());
        }
        extracted = extracted.saturating_add(size);
        if extracted > MAX_TOTAL_BYTES {
            return Err("The update expands beyond its permitted size and was rejected".to_owned());
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not extract the update: {error}"))?;
        }
        write_entry(&mut entry, &target, size)?;
    }

    let package_dir = package_dir
        .map(|package| destination.join(package))
        .ok_or_else(|| "The update archive is empty".to_owned())?;
    for required in REQUIRED_ENTRIES {
        if !package_dir.join(required).is_file() {
            return Err(format!("The update archive contains no {required}"));
        }
    }
    Ok(package_dir)
}

fn write_entry<R: Read>(entry: &mut R, target: &Path, size: u64) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|error| format!("Could not extract the update: {error}"))?;
    let copied = std::io::copy(&mut entry.take(size), &mut file)
        .map_err(|error| format!("Could not extract the update: {error}"))?;
    if copied != size {
        return Err("The update contains a truncated file and was rejected".to_owned());
    }
    Ok(())
}

fn safe_relative_path(path: &Path) -> Result<PathBuf, String> {
    let mut relative = PathBuf::new();
    let mut depth = 0_usize;
    for component in path.components() {
        match component {
            Component::Normal(name) => {
                relative.push(name);
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("The update contains a path outside its directory".to_owned());
            }
        }
    }
    if depth == 0 {
        return Err("The update contains an empty path".to_owned());
    }
    Ok(relative)
}

fn describe_entry_type(kind: tar::EntryType) -> &'static str {
    match kind {
        kind if kind.is_symlink() => "symbolic link",
        kind if kind.is_hard_link() => "hard link",
        kind if kind.is_fifo() => "named pipe",
        kind if kind.is_block_special() || kind.is_character_special() => "device",
        _ => "unknown type",
    }
}

#[cfg(test)]
mod tests;
