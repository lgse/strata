// SPDX-License-Identifier: MIT

//! Volumes GVfs lists although the system holds them, so they cannot be mounted.

use super::probe_cache::{self, ProbeCache};

thread_local! {
    static HELD: ProbeCache<bool> = ProbeCache::default();
}

/// GVfs marks a loop for automount only when this user attached it. Any other
/// loop is system plumbing, such as the root image of a live system.
pub(in crate::ui::window) fn is_system_loop(class: Option<&str>, should_automount: bool) -> bool {
    class == Some("loop") && !should_automount
}

pub(in crate::ui::window) fn is_exclusively_held(unix_device: &str) -> bool {
    probe_cache::cached(&HELD, unix_device, exclusive_open_is_refused)
}

/// The kernel refuses `O_EXCL` on a block device that a file system or another
/// holder has claimed, including one mounted where the volume monitor cannot see.
pub(super) fn exclusive_open_is_refused(unix_device: &str) -> bool {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_EXCL | libc::O_NONBLOCK)
        .open(unix_device)
        .err()
        .is_some_and(|error| error.raw_os_error() == Some(libc::EBUSY))
}
