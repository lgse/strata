// SPDX-License-Identifier: MIT
use std::{
    fs,
    os::unix::fs::{FileExt, MetadataExt},
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn bind_aliases(command: &mut Command) {
    for (source, destination) in aliases_at(Path::new("/"), 0) {
        command.arg("--ro-bind").arg(source).arg(destination);
    }
}

fn protected_ancestors(path: &Path, root: &Path, owner: u32) -> bool {
    path.ancestors()
        .take_while(|path| path.starts_with(root))
        .all(|directory| {
            fs::symlink_metadata(directory).is_ok_and(|metadata| {
                metadata.is_dir() && metadata.uid() == owner && metadata.mode() & 0o022 == 0
            })
        })
}

fn aliases_at(root: &Path, owner: u32) -> Vec<(PathBuf, PathBuf)> {
    let abi = if cfg!(target_arch = "aarch64") {
        "aarch64-linux-gnu"
    } else {
        "x86_64-linux-gnu"
    };
    let machine: u16 = if cfg!(target_arch = "aarch64") {
        183
    } else {
        62
    };
    let alternatives = root.join("etc/alternatives");
    if !protected_ancestors(&alternatives, root, owner) {
        return Vec::new();
    }
    ["libblas.so.3", "liblapack.so.3"]
        .into_iter()
        .filter_map(|library| {
            let name = format!("{library}-{abi}");
            let alias = alternatives.join(&name);
            if fs::symlink_metadata(&alias).ok()?.uid() != owner {
                return None;
            }
            let source = alias.canonicalize().ok()?;
            if !source.starts_with(root.join("usr/lib"))
                || !protected_ancestors(source.parent()?, root, owner)
            {
                return None;
            }
            let file = fs::File::from(
                rustix::fs::open(
                    &source,
                    rustix::fs::OFlags::RDONLY
                        | rustix::fs::OFlags::NOFOLLOW
                        | rustix::fs::OFlags::NONBLOCK
                        | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )
                .ok()?,
            );
            let metadata = file.metadata().ok()?;
            if !metadata.is_file() || metadata.uid() != owner || metadata.mode() & 0o6022 != 0 {
                return None;
            }
            let mut header = [0; 20];
            file.read_exact_at(&mut header, 0).ok()?;
            if &header[..7] != b"\x7fELF\x02\x01\x01" || header[18..20] != machine.to_le_bytes() {
                return None;
            }
            // Only privileged package changes can replace this canonical source: every
            // ancestor and the file are root-owned and deny group/other writes.
            Some((source, Path::new("/etc/alternatives").join(name)))
        })
        .collect()
}

#[cfg(test)]
mod tests;
