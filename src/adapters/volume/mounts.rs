// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

/// Mount points and filesystem types from `/proc/self/mountinfo`. Reading it
/// never touches the mounted filesystems, so it is safe to consult for a path
/// on a network mount that may be unresponsive.
pub(super) struct MountTable {
    entries: Vec<(PathBuf, String)>,
}

impl MountTable {
    pub(super) fn current() -> Self {
        std::fs::read_to_string("/proc/self/mountinfo")
            .map(|mountinfo| Self::parse(&mountinfo))
            .unwrap_or(Self {
                entries: Vec::new(),
            })
    }

    pub(super) fn parse(mountinfo: &str) -> Self {
        let entries = mountinfo
            .lines()
            .filter_map(|line| {
                let (front, back) = line.split_once(" - ")?;
                let mount_point = front.split(' ').nth(4)?;
                let fs_type = back.split(' ').next()?;
                Some((PathBuf::from(unescape(mount_point)), fs_type.to_owned()))
            })
            .collect();
        Self { entries }
    }

    /// Filesystem type of the innermost mount containing `path`.
    pub(super) fn fs_type_for(&self, path: &Path) -> Option<&str> {
        self.entries
            .iter()
            .filter(|(mount_point, _)| path.starts_with(mount_point))
            .max_by_key(|(mount_point, _)| mount_point.as_os_str().len())
            .map(|(_, fs_type)| fs_type.as_str())
    }

    pub(super) fn is_remote_path(&self, path: &Path) -> bool {
        self.fs_type_for(path).is_some_and(is_remote_fs_type)
    }
}

/// Network filesystems and anything served by a userspace daemon (FUSE, which
/// includes gvfs, sshfs, rclone, and the document portal): a request against
/// these can block indefinitely if the peer is gone.
pub(super) fn is_remote_fs_type(fs_type: &str) -> bool {
    matches!(
        fs_type,
        "nfs"
            | "nfs4"
            | "cifs"
            | "smb"
            | "smb2"
            | "smb3"
            | "smbfs"
            | "ncpfs"
            | "afs"
            | "afp"
            | "9p"
            | "ceph"
            | "coda"
            | "davfs"
            | "glusterfs"
            | "gfs"
            | "gfs2"
            | "lustre"
            | "ocfs2"
            | "webdav"
            | "vboxsf"
            | "virtiofs"
    ) || fs_type.starts_with("fuse")
}

/// mountinfo escapes space, tab, newline, and backslash as octal `\ooo`.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let octal = (bytes[index] == b'\\' && index + 3 < bytes.len())
            .then(|| &bytes[index + 1..index + 4])
            .filter(|digits| digits.iter().all(|digit| (b'0'..=b'7').contains(digit)))
            .map(|digits| {
                digits
                    .iter()
                    .fold(0u32, |code, digit| code * 8 + u32::from(digit - b'0'))
            });
        match octal.and_then(|code| u8::try_from(code).ok()) {
            Some(byte) => {
                out.push(byte);
                index += 4;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
