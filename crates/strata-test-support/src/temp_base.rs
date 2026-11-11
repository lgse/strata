// SPDX-License-Identifier: MIT

//! What the base under the tests' temporary folders can do. CI runs them on
//! a tmpfs that can do everything; a packager's overlay `/tmp` cannot, and
//! the tests that need what it lacks skip there.

use std::{io, os::fd::AsRawFd, path::Path, sync::OnceLock};

use rustix::{
    fs::{AtFlags, Mode, OFlags},
    io::Errno,
};

/// Something a test needs of the base, and the probe that tells whether it is there.
pub(crate) struct BaseNeed {
    pub(crate) what: &'static str,
    pub(crate) probe: fn() -> io::Result<bool>,
}

/// Unnamed files, opened with `O_TMPFILE` and linked into place: overlayfs
/// has them only since Linux 6.10.
pub(crate) const UNNAMED_FILES: BaseNeed = BaseNeed {
    what: "unnamed files linked into place",
    probe: links_unnamed_files,
};

/// Whether the base offers what `need` names. A test that needs it returns
/// early on a base without it, unless `STRATA_REQUIRE_DEVICE_TESTS` is set,
/// as CI sets it on its tmpfs, which fails the test instead.
pub(crate) fn base_offers(need: &BaseNeed) -> io::Result<bool> {
    let offered = (need.probe)()?;
    if !offered {
        assert!(
            std::env::var_os("STRATA_REQUIRE_DEVICE_TESTS").is_none(),
            "the base of the temporary folders offers no {}; point TMPDIR at a tmpfs",
            need.what
        );
        eprintln!(
            "Skipping: the base of the temporary folders offers no {}",
            need.what
        );
    }
    Ok(offered)
}

/// Runs `probe` once per process, in a folder of its own under the temporary directory.
pub(crate) fn probed_once(
    cell: &'static OnceLock<Result<bool, String>>,
    probe: fn(&Path) -> io::Result<bool>,
) -> io::Result<bool> {
    cell.get_or_init(|| {
        let folder = tempfile::tempdir().map_err(|error| error.to_string())?;
        probe(folder.path()).map_err(|error| error.to_string())
    })
    .clone()
    .map_err(io::Error::other)
}

fn links_unnamed_files() -> io::Result<bool> {
    static PROBED: OnceLock<Result<bool, String>> = OnceLock::new();
    probed_once(&PROBED, |folder| {
        let dir = std::fs::File::open(folder)?;
        let flags = OFlags::TMPFILE | OFlags::RDWR | OFlags::CLOEXEC;
        let file = match rustix::fs::openat(&dir, ".", flags, Mode::from_raw_mode(0o600)) {
            Ok(file) => file,
            Err(Errno::OPNOTSUPP | Errno::ISDIR | Errno::INVAL) => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        // The `/proc` form links on every kernel that links unnamed files at all.
        let through_proc = format!("/proc/self/fd/{}", file.as_raw_fd());
        match rustix::fs::linkat(
            rustix::fs::CWD,
            through_proc,
            &dir,
            "linked",
            AtFlags::SYMLINK_FOLLOW,
        ) {
            Ok(()) => Ok(true),
            Err(Errno::OPNOTSUPP | Errno::PERM | Errno::NOENT | Errno::XDEV) => Ok(false),
            Err(error) => Err(error.into()),
        }
    })
}
