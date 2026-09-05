// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use rustix::process::{Pid, Signal, kill_process_group};

const WALL_TIME_LIMIT: Duration = Duration::from_secs(12);
const MEDIA_WALL_TIME_LIMIT: Duration = Duration::from_secs(30);
const ADDRESS_SPACE_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const FILE_SIZE_LIMIT_BYTES: u64 = 512 * 1024 * 1024;
const TEMPORARY_STORAGE_LIMIT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RASTER_INPUT_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MediaPreviewBackend {
    Automatic,
    VaApi,
    Vulkan,
    Software,
}

impl MediaPreviewBackend {
    pub(crate) fn argument(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::VaApi => "vaapi",
            Self::Vulkan => "vulkan",
            Self::Software => "software",
        }
    }

    pub(crate) fn from_argument(value: &str) -> Option<Self> {
        match value {
            "automatic" => Some(Self::Automatic),
            "vaapi" => Some(Self::VaApi),
            "vulkan" => Some(Self::Vulkan),
            "software" => Some(Self::Software),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParseOperation {
    ThumbnailImage,
    ThumbnailRaw,
    ThumbnailPdf,
    ThumbnailVideo,
    PreviewImage,
    PreviewPdf,
    PreviewMedia,
    PreviewDatabase,
}

impl ParseOperation {
    fn argument(self) -> &'static str {
        match self {
            Self::ThumbnailImage => "thumbnail-image",
            Self::ThumbnailRaw => "thumbnail-raw",
            Self::ThumbnailPdf => "thumbnail-pdf",
            Self::ThumbnailVideo => "thumbnail-video",
            Self::PreviewImage => "preview-image",
            Self::PreviewPdf => "preview-pdf",
            Self::PreviewMedia => "preview-media",
            Self::PreviewDatabase => "preview-database",
        }
    }

    fn output_name(self) -> &'static str {
        match self {
            Self::PreviewMedia => "result.media",
            Self::PreviewDatabase => "result.db",
            _ => "result.png",
        }
    }

    fn wall_time_limit(self) -> Duration {
        if self == Self::PreviewMedia {
            MEDIA_WALL_TIME_LIMIT
        } else {
            WALL_TIME_LIMIT
        }
    }

    fn image_limits(self) -> Option<(u32, u32, u64)> {
        match self {
            Self::ThumbnailImage
            | Self::ThumbnailRaw
            | Self::ThumbnailPdf
            | Self::ThumbnailVideo => Some((256, 256, 256 * 256)),
            Self::PreviewImage => Some((1_400, 1_400, 1_400 * 1_400)),
            Self::PreviewPdf => Some((1_400, 1_800, 2_500_000)),
            Self::PreviewMedia | Self::PreviewDatabase => None,
        }
    }

    fn input_size_limit(self) -> Option<u64> {
        match self {
            Self::ThumbnailImage
            | Self::ThumbnailRaw
            | Self::ThumbnailPdf
            | Self::PreviewImage
            | Self::PreviewPdf => Some(MAX_RASTER_INPUT_BYTES),
            Self::ThumbnailVideo | Self::PreviewMedia | Self::PreviewDatabase => None,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub(crate) struct ParseOutput {
    pub(crate) data: Vec<u8>,
    pub(crate) page: i32,
    pub(crate) pages: i32,
}

pub(crate) fn parse(
    input: &Path,
    operation: ParseOperation,
    value: i32,
    media_backend: MediaPreviewBackend,
    cancellation: &Cancellation,
) -> Result<ParseOutput, String> {
    if cancellation.is_cancelled() {
        return Err("Preview cancelled".to_owned());
    }
    let input = input
        .canonicalize()
        .map_err(|error| format!("Unable to open preview input: {error}"))?;
    let input_metadata = fs::metadata(&input)
        .map_err(|error| format!("Unable to inspect preview input: {error}"))?;
    if !input_metadata.is_file() {
        return Err("Preview input is not a regular file".to_owned());
    }
    if operation
        .input_size_limit()
        .is_some_and(|limit| input_metadata.len() > limit)
    {
        return Err("Preview input exceeds the supported size limit".to_owned());
    }

    let output = PrivateOutput::create().map_err(|error| error.to_string())?;
    let executable = std::env::current_exe()
        .map_err(|error| format!("Unable to locate the Strata executable: {error}"))?;
    let devices = if operation == ParseOperation::PreviewMedia {
        gpu_devices(Path::new("/dev"), media_backend)
    } else {
        Vec::new()
    };
    let database_companions = if operation == ParseOperation::PreviewDatabase {
        open_database_companions(&input)
    } else {
        Vec::new()
    };
    let request = SandboxRequest {
        executable: &executable,
        input: &input,
        output: output.path(),
        operation,
        value,
        media_backend,
        devices: &devices,
        database_companions: &database_companions,
    };
    let mut command = sandbox_command(&request);
    command.stderr(Stdio::null());
    if operation == ParseOperation::PreviewMedia {
        command.stdout(Stdio::piped());
    } else {
        command.stdout(Stdio::null());
    }
    let mut child = spawn_renderer(&mut command)
        .map_err(|error| format!("Unable to start the preview sandbox: {error}"))?;
    if operation == ParseOperation::PreviewMedia {
        let (status, data) = wait_for_renderer_output(
            &mut child,
            cancellation,
            operation.wall_time_limit(),
            MAX_OUTPUT_BYTES,
        )?;
        if !status.success() {
            return Err("The sandboxed preview renderer failed".to_owned());
        }
        if data.is_empty() {
            return Err("The preview renderer produced no output".to_owned());
        }
        if !valid_output(operation, &data) {
            return Err("The preview renderer produced invalid media data".to_owned());
        }
        return Ok(ParseOutput {
            data,
            page: 0,
            pages: 0,
        });
    }

    let status = wait_for_renderer(&mut child, cancellation, operation.wall_time_limit())?;
    if !status.success() {
        return Err("The sandboxed preview renderer failed".to_owned());
    }

    let result_path = output.path().join(operation.output_name());
    let metadata = fs::metadata(&result_path)
        .map_err(|_| "The preview renderer produced no output".to_owned())?;
    if (metadata.len() == 0 && operation != ParseOperation::PreviewDatabase)
        || metadata.len() > MAX_OUTPUT_BYTES
    {
        return Err("The preview renderer produced an invalid output size".to_owned());
    }
    let data = fs::read(result_path).map_err(|error| error.to_string())?;
    if !valid_output(operation, &data) {
        return Err("The preview renderer produced invalid image data".to_owned());
    }
    let (page, pages) = read_metadata(&output.path().join("result.meta"));
    Ok(ParseOutput { data, page, pages })
}

fn spawn_renderer(command: &mut Command) -> io::Result<Child> {
    use std::os::unix::process::CommandExt;

    command.process_group(0).spawn()
}

/// `None` on kernels without pidfd support; wait loops fall back to interval polling.
fn child_pidfd(child: &Child) -> Option<rustix::fd::OwnedFd> {
    let pid = rustix::process::Pid::from_raw(i32::try_from(child.id()).ok()?)?;
    rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).ok()
}

/// A pidfd wakes on child exit; poll errors degrade to a short sleep, and the
/// deadline bounds every path.
fn wait_step(pidfd: Option<&rustix::fd::OwnedFd>, deadline: Instant) {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let remaining = deadline.saturating_duration_since(Instant::now());
    let quantum = remaining.min(Duration::from_millis(20));
    let Some(pidfd) = pidfd else {
        thread::sleep(quantum);
        return;
    };
    let timespec = Timespec {
        tv_sec: quantum.as_secs() as i64,
        tv_nsec: i64::from(quantum.subsec_nanos()),
    };
    let mut fds = [PollFd::new(pidfd, PollFlags::IN)];
    if poll(&mut fds, Some(&timespec)).is_err() {
        thread::sleep(Duration::from_millis(1));
    }
}

fn wait_for_renderer(
    child: &mut Child,
    cancellation: &Cancellation,
    wall_time_limit: Duration,
) -> Result<ExitStatus, String> {
    let started = Instant::now();
    let deadline = started + wall_time_limit;
    let pidfd = child_pidfd(child);
    loop {
        if cancellation.is_cancelled() {
            terminate(child);
            return Err("Preview cancelled".to_owned());
        }
        if Instant::now() >= deadline {
            terminate(child);
            return Err("The preview renderer timed out".to_owned());
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => wait_step(pidfd.as_ref(), deadline),
            Err(error) => {
                terminate(child);
                return Err(format!("Unable to monitor the preview renderer: {error}"));
            }
        }
    }
}

fn wait_for_renderer_output(
    child: &mut Child,
    cancellation: &Cancellation,
    wall_time_limit: Duration,
    max_bytes: u64,
) -> Result<(ExitStatus, Vec<u8>), String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Unable to capture preview renderer output".to_owned())?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut data = Vec::new();
        let result = stdout
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut data)
            .map(|_| data);
        let _sent = sender.send(result);
    });
    let started = Instant::now();
    let deadline = started + wall_time_limit;
    let pidfd = child_pidfd(child);
    let mut status = None;
    let mut output = None;
    loop {
        if cancellation.is_cancelled() {
            terminate(child);
            let _joined = reader.join();
            return Err("Preview cancelled".to_owned());
        }
        if Instant::now() >= deadline {
            terminate(child);
            let _joined = reader.join();
            return Err("The preview renderer timed out".to_owned());
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(current) => status = current,
                Err(error) => {
                    terminate(child);
                    let _joined = reader.join();
                    return Err(format!("Unable to monitor the preview renderer: {error}"));
                }
            }
        }
        if output.is_none() {
            match receiver.try_recv() {
                Ok(Ok(data)) if data.len() as u64 > max_bytes => {
                    terminate(child);
                    let _joined = reader.join();
                    return Err("Preview provider output exceeded its limit".to_owned());
                }
                Ok(Ok(data)) => output = Some(data),
                Ok(Err(error)) => {
                    terminate(child);
                    let _joined = reader.join();
                    return Err(format!("Unable to read preview renderer output: {error}"));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    terminate(child);
                    let _joined = reader.join();
                    return Err("Unable to read preview renderer output".to_owned());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(status) = status
            && let Some(output) = output.take()
        {
            let _joined = reader.join();
            return Ok((status, output));
        }
        wait_step(pidfd.as_ref(), deadline);
    }
}

fn open_database_companions(input: &Path) -> Vec<(rustix::fd::OwnedFd, &'static str)> {
    let mut companions = Vec::new();
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut companion_path = input.as_os_str().to_os_string();
        companion_path.push(suffix);
        let Ok(fd) = rustix::fs::open(
            Path::new(&companion_path),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
        ) else {
            continue;
        };
        let Ok(stat) = rustix::fs::fstat(&fd) else {
            continue;
        };
        if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile {
            continue;
        }
        companions.push((fd, suffix));
    }
    companions
}

pub(crate) struct SandboxRequest<'a> {
    pub(crate) executable: &'a Path,
    pub(crate) input: &'a Path,
    pub(crate) output: &'a Path,
    pub(crate) operation: ParseOperation,
    pub(crate) value: i32,
    pub(crate) media_backend: MediaPreviewBackend,
    pub(crate) devices: &'a [PathBuf],
    pub(crate) database_companions: &'a [(rustix::fd::OwnedFd, &'static str)],
}

fn sandbox_command(request: &SandboxRequest<'_>) -> Command {
    let mut command = Command::new("bwrap");
    command.args([
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin",
        "--setenv",
        "HOME",
        "/nonexistent",
        "--setenv",
        "XDG_CACHE_HOME",
        "/tmp/cache",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--size",
        &TEMPORARY_STORAGE_LIMIT_BYTES.to_string(),
        "--tmpfs",
        "/tmp",
        "--dir",
        "/app",
        "--dir",
        "/etc",
        "--ro-bind",
        "/usr",
        "/usr",
        "--ro-bind-try",
        "/lib",
        "/lib",
        "--ro-bind-try",
        "/lib64",
        "/lib64",
        "--ro-bind-try",
        "/etc/fonts",
        "/etc/fonts",
        "--ro-bind-try",
        "/etc/ld.so.cache",
        "/etc/ld.so.cache",
        "--ro-bind-try",
        "/etc/ImageMagick-7",
        "/etc/ImageMagick-7",
        "--ro-bind-try",
        "/etc/ImageMagick-6",
        "/etc/ImageMagick-6",
    ]);
    let sandbox_input = sandbox_input_path(request.input);
    if request.operation != ParseOperation::ThumbnailVideo {
        command
            .arg("--ro-bind")
            .arg(request.executable)
            .arg("/app/strata");
    }
    command
        .arg("--ro-bind")
        .arg(request.input)
        .arg(&sandbox_input);
    for (fd, suffix) in request.database_companions {
        use std::os::fd::AsRawFd;
        command
            .arg("--ro-bind-fd")
            .arg(fd.as_raw_fd().to_string())
            .arg(format!("{sandbox_input}{suffix}"));
    }
    if request.operation != ParseOperation::PreviewMedia {
        command.arg("--bind").arg(request.output).arg("/output");
    }
    if request.operation == ParseOperation::PreviewMedia
        && request.media_backend != MediaPreviewBackend::Software
    {
        // Hardware media drivers need selected render nodes plus read-only sysfs discovery data.
        for device in request.devices {
            command.arg("--dev-bind-try").arg(device).arg(device);
        }
        command.args(["--ro-bind", "/sys", "/sys"]);
    }
    if request.operation != ParseOperation::PreviewMedia {
        // Keep CPU-scaled glibc arenas within the helper's address-space limit.
        command.args(["--setenv", "MALLOC_ARENA_MAX", "1"]);
    }
    command.arg("--");
    if request.operation != ParseOperation::PreviewMedia {
        command
            .arg("/usr/bin/prlimit")
            .arg(format!("--as={ADDRESS_SPACE_LIMIT_BYTES}"))
            .arg("--cpu=10")
            .arg(format!(
                "--fsize={}",
                if request.operation == ParseOperation::ThumbnailVideo {
                    MAX_OUTPUT_BYTES
                } else {
                    FILE_SIZE_LIMIT_BYTES
                }
            ))
            .arg("--");
    }
    if request.operation == ParseOperation::ThumbnailVideo {
        command
            .args(["/usr/bin/ffmpegthumbnailer", "-i", &sandbox_input, "-o"])
            .arg(format!("/output/{}", request.operation.output_name()))
            .arg("-s")
            .arg(request.value.to_string())
            .args(["-q", "8"]);
        return command;
    }
    command.args([
        "/app/strata",
        "--preview-helper",
        request.operation.argument(),
        &sandbox_input,
    ]);
    if request.operation == ParseOperation::PreviewMedia {
        command.arg("/dev/stdout");
    } else {
        command.arg(format!("/output/{}", request.operation.output_name()));
    }
    command.arg(request.value.to_string());
    command.arg(request.media_backend.argument());
    command
}

fn sandbox_input_path(input: &Path) -> String {
    match input.extension().and_then(|extension| extension.to_str()) {
        Some(extension)
            if (1..=8).contains(&extension.len())
                && extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) =>
        {
            format!("/input.{extension}")
        }
        _ => "/input".to_owned(),
    }
}

pub(crate) fn gpu_devices(dev: &Path, media_backend: MediaPreviewBackend) -> Vec<PathBuf> {
    if media_backend == MediaPreviewBackend::Software {
        return Vec::new();
    }
    let mut devices = Vec::new();
    if let Ok(entries) = fs::read_dir(dev.join("dri")) {
        for entry in entries.flatten() {
            if numbered_name(&entry.file_name(), "renderD") {
                devices.push(entry.path());
            }
        }
    }
    if media_backend != MediaPreviewBackend::VaApi
        && let Ok(entries) = fs::read_dir(dev)
    {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name == "nvidiactl" || numbered_name(&name, "nvidia") {
                devices.push(entry.path());
            }
        }
    }
    devices.sort();
    devices
}

pub(crate) fn polaris_gpu_available() -> bool {
    polaris_gpu_available_at(Path::new("/dev"), Path::new("/sys/class/drm"))
}

fn polaris_gpu_available_at(dev: &Path, drm: &Path) -> bool {
    fs::read_dir(dev.join("dri")).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            numbered_name(&entry.file_name(), "renderD") && polaris_render_node(&entry.path(), drm)
        })
    })
}

fn polaris_render_node(device: &Path, drm: &Path) -> bool {
    let Some(name) = device.file_name() else {
        return false;
    };
    let metadata = drm.join(name).join("device");
    let Some(vendor) = pci_id(&metadata.join("vendor")) else {
        return false;
    };
    let Some(device) = pci_id(&metadata.join("device")) else {
        return false;
    };
    vendor == 0x1002 && matches!(device, 0x67c0..=0x67df | 0x67e0..=0x67ff | 0x6980..=0x699f)
}

fn pci_id(path: &Path) -> Option<u16> {
    let value = fs::read_to_string(path).ok()?;
    u16::from_str_radix(value.trim().strip_prefix("0x").unwrap_or(value.trim()), 16).ok()
}

pub(crate) fn numbered_name(name: &std::ffi::OsStr, prefix: &str) -> bool {
    name.to_str()
        .and_then(|name| name.strip_prefix(prefix))
        .is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn valid_output(operation: ParseOperation, data: &[u8]) -> bool {
    if operation == ParseOperation::PreviewMedia {
        data.starts_with(b"\x1a\x45\xdf\xa3")
            || data.get(4..8).is_some_and(|signature| signature == b"ftyp")
    } else if operation == ParseOperation::PreviewDatabase {
        true
    } else {
        let Some((width, height)) = png_dimensions(data) else {
            return false;
        };
        let Some((max_width, max_height, max_pixels)) = operation.image_limits() else {
            return false;
        };
        width <= max_width
            && height <= max_height
            && u64::from(width)
                .checked_mul(u64::from(height))
                .is_some_and(|pixels| pixels <= max_pixels)
    }
}

fn png_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n")
        || data.get(8..12)? != 13u32.to_be_bytes()
        || data.get(12..16)? != b"IHDR"
    {
        return None;
    }
    let width = u32::from_be_bytes(data.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

fn terminate(child: &mut Child) {
    if let Ok(raw_pid) = i32::try_from(child.id())
        && let Some(process_group) = Pid::from_raw(raw_pid)
    {
        let _killed = kill_process_group(process_group, Signal::KILL);
    }
    // bwrap is also the PID-namespace init process, so descendants that create a new
    // process group still die with it.
    let _killed = child.kill();
    let _waited = child.wait();
}

fn read_metadata(path: &Path) -> (i32, i32) {
    let Ok(value) = fs::read_to_string(path) else {
        return (0, 0);
    };
    let mut values = value
        .split_whitespace()
        .filter_map(|part| part.parse().ok());
    (values.next().unwrap_or(0), values.next().unwrap_or(0))
}

struct PrivateOutput(PathBuf);

impl PrivateOutput {
    fn create() -> io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;

        let path = std::env::temp_dir().join(format!(
            "strata-preview-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PrivateOutput {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests;
