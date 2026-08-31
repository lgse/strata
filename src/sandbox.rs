// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const WALL_TIME_LIMIT: Duration = Duration::from_secs(12);
const MEDIA_WALL_TIME_LIMIT: Duration = Duration::from_secs(30);
const ADDRESS_SPACE_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
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
}

#[derive(Clone, Default)]
pub(crate) struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
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
    if !input.is_file() {
        return Err("Preview input is not a regular file".to_owned());
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
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Unable to start the preview sandbox: {error}"))?;
    let started = Instant::now();

    let status = loop {
        if cancellation.is_cancelled() {
            terminate(&mut child);
            return Err("Preview cancelled".to_owned());
        }
        if started.elapsed() >= operation.wall_time_limit() {
            terminate(&mut child);
            return Err("The preview renderer timed out".to_owned());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                terminate(&mut child);
                return Err(format!("Unable to monitor the preview renderer: {error}"));
            }
        }
    };
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
        return Err(if operation == ParseOperation::PreviewMedia {
            "The preview renderer produced invalid media data".to_owned()
        } else {
            "The preview renderer produced invalid image data".to_owned()
        });
    }
    let (page, pages) = read_metadata(&output.path().join("result.meta"));
    Ok(ParseOutput { data, page, pages })
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
    command.arg(executable).arg("/app/strata");
    command.arg("--ro-bind").arg(input).arg("/input");
    command.arg("--bind").arg(output).arg("/output");
    if operation == ParseOperation::PreviewMedia {
        // Hardware media drivers need selected render nodes plus read-only sysfs discovery data.
        for device in devices {
            command.arg("--dev-bind-try").arg(device).arg(device);
        }
        command.args(["--ro-bind", "/sys", "/sys"]);
    }
    command.args([
        "--",
        "/usr/bin/prlimit",
        &format!("--as={ADDRESS_SPACE_LIMIT_BYTES}"),
    ]);
    if operation != ParseOperation::PreviewMedia {
        command.arg("--cpu=10");
    }
    command.args([
        "--fsize=33554432",
        "--",
        "/app/strata",
        "--preview-helper",
        operation.argument(),
        "/input",
    ]);
    command.arg(format!("/output/{}", operation.output_name()));
    command.arg(value.to_string());
    command
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
        data.starts_with(b"\x89PNG\r\n\x1a\n")
    }
}

fn terminate(child: &mut std::process::Child) {
    // bwrap is the PID-namespace init process. Killing it tears down every process in
    // the sandbox, including descendants started by ImageMagick or thumbnail tools.
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
