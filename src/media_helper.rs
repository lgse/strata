// SPDX-License-Identifier: MIT

use crate::media::{Cancellation, TimedReader};
use std::{
    fs::{self, File},
    io::{self, Read},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
    process::Child,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

include!(concat!(env!("OUT_DIR"), "/media-helper.rs"));

const NAME: &str = "strata-media-helper";
static HELPER: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

pub(crate) fn job() -> u64 {
    NEXT_JOB.fetch_add(1, Ordering::Relaxed)
}

/// Pin the installed inode before a package manager can rename or unlink it.
/// Missing installations remain retryable; there is no PATH/cwd search.
pub(crate) fn initialize() {
    let _ = with_helper(|_| Ok(()));
}

fn with_helper<T>(use_file: impl FnOnce(&File) -> Result<T, String>) -> Result<T, String> {
    let mut helper = HELPER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "Media helper discovery failed")?;
    if helper.is_none() {
        let current =
            std::env::current_exe().map_err(|_| "Cannot locate the running Strata installation")?;
        let directory = current
            .parent()
            .ok_or("Cannot locate the running Strata installation")?;
        #[cfg(test)]
        let directory = if directory.file_name().is_some_and(|name| name == "deps") {
            directory.parent().ok_or("Missing test build directory")?
        } else {
            directory
        };
        *helper = Some(open_helper(&directory.join(NAME))?);
    }
    use_file(helper.as_ref().ok_or("Media helper is unavailable")?)
}

fn open_helper(path: &Path) -> Result<File, String> {
    use rustix::fs::{Mode, OFlags, open};
    let parent = trusted_directory(path.parent().ok_or("Invalid helper installation")?)?;
    let path = parent.join(path.file_name().ok_or("Invalid helper filename")?);
    let fd = open(&path, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK, Mode::empty())
        .map_err(|error| match error {
            rustix::io::Errno::NOENT => "The bundled strata-media-helper is missing. Reinstall this exact Strata release; installing GStreamer alone cannot repair it.",
            _ => "The bundled strata-media-helper cannot be opened. Repair or reinstall the Strata bundle.",
        })?;
    let file = File::from(fd);
    let metadata = file
        .metadata()
        .map_err(|_| "Cannot inspect the media helper")?;
    if !metadata.is_file()
        || metadata.mode() & 0o111 == 0
        || metadata.mode() & 0o6022 != 0
        || (metadata.uid() != 0 && metadata.uid() != rustix::process::geteuid().as_raw())
    {
        return Err("The bundled strata-media-helper is not a safe executable. Repair or reinstall the Strata bundle.".into());
    }
    let mut header = [0; 20];
    use std::os::unix::fs::FileExt;
    file.read_exact_at(&mut header, 0)
        .map_err(|_| "The bundled strata-media-helper is corrupt; reinstall Strata.")?;
    let machine: u16 = if cfg!(target_arch = "aarch64") {
        183
    } else {
        62
    };
    if &header[..7] != b"\x7fELF\x02\x01\x01" || header[18..20] != machine.to_le_bytes() {
        return Err("The bundled strata-media-helper is corrupt or built for another architecture. Reinstall Strata for this system.".into());
    }
    Ok(file)
}

pub(crate) fn trusted_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Media and installation storage must use an absolute trusted directory".into());
    }
    let path = path
        .canonicalize()
        .map_err(|_| "Cannot locate trusted media or installation storage")?;
    for ancestor in path.ancestors() {
        let metadata =
            fs::symlink_metadata(ancestor).map_err(|_| "Cannot inspect trusted storage")?;
        if !metadata.is_dir()
            || (metadata.uid() != 0 && metadata.uid() != rustix::process::geteuid().as_raw())
            || (metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0)
        {
            return Err("Unsafe writable media or installation directory. Use private user-owned storage or a root-owned sticky temporary directory.".into());
        }
    }
    Ok(path)
}

pub(crate) fn private_tempdir() -> Result<tempfile::TempDir, String> {
    use std::os::unix::fs::PermissionsExt;
    let root = trusted_directory(&std::env::temp_dir())?;
    tempfile::Builder::new()
        .prefix("strata-media-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(root)
        .map_err(|_| "Cannot create private media storage".into())
}

pub(crate) fn snapshot(directory: &Path) -> Result<PathBuf, String> {
    let directory = trusted_directory(directory)?;
    let directory = directory.as_path();
    let pinned = with_helper(|file| copy_matching_helper(file, directory));
    match (pinned, EMBEDDED) {
        (Ok(path), _) => Ok(path),
        (Err(error), None) => Err(error),
        (Err(_), Some((compressed, hash))) => {
            let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
                .filter(|p| p.is_absolute()).ok_or("Cannot recover the media helper: no absolute user cache directory. Reinstall the complete Strata bundle.")?;
            let recovered = recover_at(&cache, compressed, hash)?;
            let file = open_helper(&recovered)?;
            let path = copy_matching_helper(&file, directory)?;
            *HELPER
                .get_or_init(|| Mutex::new(None))
                .lock()
                .map_err(|_| "Media helper discovery failed")? = Some(file);
            Ok(path)
        }
    }
}

fn copy_matching_helper(file: &File, directory: &Path) -> Result<PathBuf, String> {
    if let Some((_, expected)) = EMBEDDED
        && hash_file(file)? != expected
    {
        return Err("The installed media helper does not match this release".into());
    }
    let path = directory.join(NAME);
    fs::copy(format!("/proc/self/fd/{}", file.as_raw_fd()), &path)
        .map_err(|_| "Cannot preserve the installed media helper for this job")?;
    Ok(path)
}

fn hash_file(file: &File) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::FileExt;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut offset = 0;
    loop {
        let n = file
            .read_at(&mut buffer, offset)
            .map_err(|_| "Cannot verify the media helper")?;
        if n == 0 {
            break;
        }
        offset += n as u64;
        if offset > 256 * 1024 * 1024 {
            return Err("Media helper exceeds the installation size limit".into());
        }
        digest.update(&buffer[..n]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn recover_at(cache: &Path, compressed: &[u8], expected: &str) -> Result<PathBuf, String> {
    use std::{
        io::Write,
        os::unix::fs::{DirBuilderExt, PermissionsExt},
    };
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid embedded helper identity".into());
    }
    let existing = cache
        .ancestors()
        .find(|path| path.is_dir())
        .ok_or("Cannot locate recovery storage")?;
    let base = trusted_directory(existing)?;
    let cache = base.join(
        cache
            .strip_prefix(existing)
            .map_err(|_| "Invalid recovery storage")?,
    );
    fs::create_dir_all(&cache).map_err(
        |_| "Cannot create the media recovery cache; reinstall Strata or free disk space.",
    )?;
    let cache = trusted_directory(&cache)?;
    let root = cache.join("strata-media-recovery");
    match fs::DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err("Cannot create the private media recovery cache".into()),
    }
    let meta = fs::symlink_metadata(&root).map_err(|_| "Cannot inspect recovery storage")?;
    if !meta.is_dir()
        || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.mode() & 0o077 != 0
    {
        return Err(
            "Unsafe media recovery storage; reinstall Strata in a private directory.".into(),
        );
    }
    let lock = rustix::fs::open(
        root.join("install.lock"),
        rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::RDWR
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .map_err(|_| "Cannot lock media recovery storage")?;
    let stat = rustix::fs::fstat(&lock).map_err(|_| "Cannot inspect recovery lock")?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile {
        return Err("Invalid recovery lock".into());
    }
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| "Media helper recovery is already in progress; retry the preview.")?;
    let destination = root.join(expected);
    let helper = destination.join(NAME);
    if destination.exists() {
        let meta =
            fs::symlink_metadata(&destination).map_err(|_| "Cannot inspect recovered helper")?;
        if !meta.is_dir() || meta.mode() & 0o077 != 0 {
            return Err("Unsafe recovered helper directory".into());
        }
        let file = open_helper(&helper)?;
        if hash_file(&file)? == expected {
            return Ok(helper);
        }
        return Err("Recovered media helper is damaged. Remove this release's recovery cache and retry, or reinstall Strata.".into());
    }
    if fs::read_dir(&root)
        .map_err(|_| "Cannot inspect recovery storage")?
        .count()
        > 8
    {
        return Err("Media recovery cache is full. Reinstall the complete bundle, or close all Strata instances and clear its media recovery cache.".into());
    }
    let staged = tempfile::Builder::new()
        .prefix(".recover-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(&root)
        .map_err(|_| "Cannot stage media recovery")?;
    let staged_helper = staged.path().join(NAME);
    let mut file = File::create(&staged_helper).map_err(|_| "Cannot stage media helper")?;
    let mut decoder = flate2::read::GzDecoder::new(compressed).take(256 * 1024 * 1024 + 1);
    let size = io::copy(&mut decoder, &mut file)
        .map_err(|_| "Embedded media helper is corrupt or recovery storage is full")?;
    file.flush().map_err(|_| "Cannot write media helper")?;
    if size > 256 * 1024 * 1024
        || hash_file(&File::open(&staged_helper).map_err(|_| "Cannot verify recovered helper")?)?
            != expected
    {
        return Err("Embedded media helper failed integrity verification".into());
    }
    fs::set_permissions(&staged_helper, fs::Permissions::from_mode(0o700))
        .map_err(|_| "Cannot set media helper permissions")?;
    file.sync_all()
        .map_err(|_| "Cannot persist recovered helper")?;
    File::open(staged.path())
        .and_then(|f| f.sync_all())
        .map_err(|_| "Cannot persist media recovery directory")?;
    fs::rename(staged.path(), &destination)
        .map_err(|_| "Cannot activate recovered media helper")?;
    File::open(&root)
        .and_then(|f| f.sync_all())
        .map_err(|_| "Cannot persist media recovery activation")?;
    Ok(helper)
}

pub(crate) fn failure(child: &mut Child, fallback: String) -> String {
    let Some(stderr) = child.stderr.take() else {
        return fallback;
    };
    let cancellation = Cancellation::default();
    let mut bytes = Vec::new();
    let _ = TimedReader {
        fd: &stderr,
        deadline: Instant::now() + Duration::from_millis(50),
        cancellation: &cancellation,
    }
    .take(4096)
    .read_to_end(&mut bytes);
    classify_stderr(&bytes).unwrap_or(fallback)
}

fn classify_stderr(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    let message = if text.contains("error while loading shared libraries") {
        if text.contains("libgst") {
            "Media helper runtime libraries are missing. Install GStreamer core/base (Arch/Omarchy: gstreamer gst-plugins-base-libs; Debian/Ubuntu: libgstreamer1.0-0 libgstreamer-plugins-base1.0-0), then retry the preview. All media-helper modes require these libraries."
        } else {
            "A media helper runtime library is missing. Reinstall Strata's media runtime dependencies for your distribution, then retry."
        }
    } else if text.contains("strata_media:version") {
        "Media helper version/protocol mismatch. Reinstall the complete matching Strata bundle."
    } else if text.contains("strata_media:ffmpeg") {
        "Sandboxed media tools are missing. Install FFmpeg (including ffprobe), then retry."
    } else if text.contains("strata_media:thumbnail-tool") {
        "Video thumbnail tools are missing. Install ffmpegthumbnailer, then retry."
    } else if text.contains("strata_media:audio-plugin") {
        "Audio output plugins are missing. Install gst-plugins-good (Arch/Omarchy) or gstreamer1.0-plugins-good (Debian/Ubuntu), then retry."
    } else if text.contains("strata_media:audio-server") {
        "Audio output is unavailable. Start PulseAudio or PipeWire with pipewire-pulse, check the audio service, then retry."
    } else if text.contains("bwrap:") {
        "The preview sandbox could not start. Check bubblewrap and user-namespace support. No unsandboxed fallback is used."
    } else {
        return None;
    };
    Some(message.into())
}

pub(crate) fn spawn_error(error: io::Error) -> String {
    if error.kind() == io::ErrorKind::NotFound {
        "The preview sandbox is unavailable. Install bubblewrap and retry; decoding cannot run without it.".into()
    } else {
        "The media worker could not start. Check the installation and sandbox permissions.".into()
    }
}

#[cfg(test)]
mod tests;
