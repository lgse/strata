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
pub(crate) enum ParseOperation {
    ThumbnailImage,
    ThumbnailRaw,
    ThumbnailPdf,
    ThumbnailVideo,
    PreviewImage,
    PreviewPdf,
    PreviewMedia,
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
        }
    }

    fn output_name(self) -> &'static str {
        if self == Self::PreviewMedia {
            "result.media"
        } else {
            "result.png"
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
            Self::PreviewMedia => None,
        }
    }

    fn input_size_limit(self) -> Option<u64> {
        match self {
            Self::ThumbnailImage
            | Self::ThumbnailRaw
            | Self::ThumbnailPdf
            | Self::PreviewImage
            | Self::PreviewPdf => Some(MAX_RASTER_INPUT_BYTES),
            Self::ThumbnailVideo | Self::PreviewMedia => None,
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
        gpu_devices(Path::new("/dev"))
    } else {
        Vec::new()
    };
    let mut command = sandbox_command(
        &executable,
        &input,
        output.path(),
        operation,
        value,
        &devices,
    );
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
    if metadata.len() == 0 || metadata.len() > MAX_OUTPUT_BYTES {
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

fn wait_for_renderer(
    child: &mut Child,
    cancellation: &Cancellation,
    wall_time_limit: Duration,
) -> Result<ExitStatus, String> {
    let started = Instant::now();
    loop {
        if cancellation.is_cancelled() {
            terminate(child);
            return Err("Preview cancelled".to_owned());
        }
        if started.elapsed() >= wall_time_limit {
            terminate(child);
            return Err("The preview renderer timed out".to_owned());
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(20)),
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
    let mut status = None;
    let mut output = None;
    loop {
        if cancellation.is_cancelled() {
            terminate(child);
            let _joined = reader.join();
            return Err("Preview cancelled".to_owned());
        }
        if started.elapsed() >= wall_time_limit {
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
        thread::sleep(Duration::from_millis(20));
    }
}

fn sandbox_command(
    executable: &Path,
    input: &Path,
    output: &Path,
    operation: ParseOperation,
    value: i32,
    devices: &[PathBuf],
) -> Command {
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
        "--ro-bind",
    ]);
    let sandbox_input = sandbox_input_path(input);
    command.arg(executable).arg("/app/strata");
    command.arg("--ro-bind").arg(input).arg(&sandbox_input);
    if operation != ParseOperation::PreviewMedia {
        command.arg("--bind").arg(output).arg("/output");
    }
    if operation == ParseOperation::PreviewMedia {
        // Hardware media drivers need selected render nodes plus read-only sysfs discovery data.
        for device in devices {
            command.arg("--dev-bind-try").arg(device).arg(device);
        }
        command.args(["--ro-bind", "/sys", "/sys"]);
    }
    if operation != ParseOperation::PreviewMedia {
        // Keep CPU-scaled glibc arenas within the helper's address-space limit.
        command.args(["--setenv", "MALLOC_ARENA_MAX", "1"]);
    }
    command.arg("--");
    if operation != ParseOperation::PreviewMedia {
        command
            .arg("/usr/bin/prlimit")
            .arg(format!("--as={ADDRESS_SPACE_LIMIT_BYTES}"))
            .arg("--cpu=10")
            .arg(format!("--fsize={FILE_SIZE_LIMIT_BYTES}"))
            .arg("--");
    }
    command.args([
        "/app/strata",
        "--preview-helper",
        operation.argument(),
        &sandbox_input,
    ]);
    if operation == ParseOperation::PreviewMedia {
        command.arg("/dev/stdout");
    } else {
        command.arg(format!("/output/{}", operation.output_name()));
    }
    command.arg(value.to_string());
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

pub(crate) fn gpu_devices(dev: &Path) -> Vec<PathBuf> {
    let mut devices = Vec::new();
    if let Ok(entries) = fs::read_dir(dev.join("dri")) {
        for entry in entries.flatten() {
            if numbered_name(&entry.file_name(), "renderD") {
                devices.push(entry.path());
            }
        }
    }
    if let Ok(entries) = fs::read_dir(dev) {
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
