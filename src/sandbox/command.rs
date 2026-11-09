// SPDX-License-Identifier: MIT
//! The bubblewrap command lines that run the preview helper, and the GPU devices they expose.

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use super::{
    ADDRESS_SPACE_LIMIT_BYTES, FILE_SIZE_LIMIT_BYTES, MAX_OUTPUT_BYTES, MediaPreviewBackend,
    ParseOperation, PdfRenderSize, TEMPORARY_STORAGE_LIMIT_BYTES, runtime,
};
use crate::services::{MediaPreviewSize, ModelRender, ModelRenderPurpose};

const ARCHITECTURES: [&str; 2] = ["x86_64-linux-gnu", "aarch64-linux-gnu"];

const SYSTEM_BINDS: [&str; 21] = [
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
    "/var/cache/fontconfig",
    "/var/cache/fontconfig",
    "--ro-bind-try",
    "/etc/ld.so.cache",
    "/etc/ld.so.cache",
    "--ro-bind-try",
    "/etc/ImageMagick-7",
    "/etc/ImageMagick-7",
    "--ro-bind-try",
    "/etc/ImageMagick-6",
    "/etc/ImageMagick-6",
];

const GL_LIBRARIES: [&str; 9] = [
    "libGL.so.1",
    "libEGL.so.1",
    "libGLESv2.so.2",
    "libGLESv1_CM.so.1",
    "libGL.so",
    "libEGL.so",
    "libGLESv2.so",
    "libGLESv1_CM.so",
    "libGLX_indirect.so.0",
];

/// Resolves a helper tool the way the sandbox's `PATH` does. The sandbox root
/// is mounted at its host path, so the host file is the one helpers run.
pub(crate) fn sandbox_tool(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    runtime::current()
        .tool_directories()
        .map(|directory| directory.join(name))
        .find(|candidate| {
            fs::metadata(candidate).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

fn bind_try(command: &mut Command, path: &str) {
    command.arg("--ro-bind-try").arg(path).arg(path);
}

fn bind_alternatives(command: &mut Command, libraries: &[&str], path: fn(&str, &str) -> String) {
    for architecture in ARCHITECTURES {
        for library in libraries {
            bind_try(command, &path(library, architecture));
        }
    }
}

pub(super) fn runtime_command(bwrap: &Path, needs_media_libraries: bool) -> Command {
    let runtime = runtime::current();
    let mut command = Command::new(bwrap);
    command.args([
        "--info-fd",
        "2",
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--clearenv",
        "--setenv",
        "PATH",
        runtime.path.as_str(),
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
        runtime.root.as_str(),
        runtime.root.as_str(),
    ]);
    command.args(SYSTEM_BINDS);
    if let Some(loaders) = &runtime.gdk_pixbuf_module_file {
        command.args(["--setenv", "GDK_PIXBUF_MODULE_FILE", loaders]);
    }
    if needs_media_libraries {
        // Debian-family FFmpeg libraries resolve BLAS/LAPACK through these links.
        // Expose only the runtime files, not the system alternatives directory.
        bind_alternatives(
            &mut command,
            &["libblas.so.3", "liblapack.so.3"],
            |library, architecture| format!("/etc/alternatives/{library}-{architecture}"),
        );
    }
    // Debian-family ImageMagick commands resolve through these links.
    for tool in ["magick", "convert"] {
        bind_try(&mut command, &format!("/etc/alternatives/{tool}"));
    }
    bind_alternatives(&mut command, &GL_LIBRARIES, |library, architecture| {
        format!("/etc/alternatives/glx--{library}-{architecture}")
    });
    command
}

#[expect(
    clippy::too_many_arguments,
    reason = "explicit executable path permits testing without installed bubblewrap"
)]
pub(super) fn sandbox_command(
    bwrap: &Path,
    executable: &Path,
    input: &Path,
    output: &Path,
    operation: ParseOperation,
    value: i32,
    media_backend: MediaPreviewBackend,
    devices: &[PathBuf],
) -> Command {
    SandboxCommand {
        bwrap,
        executable,
        input,
        output,
        operation,
        value,
        media_backend,
        devices,
    }
    .build()
}

/// Everything one sandboxed helper invocation is built from.
pub(super) struct SandboxCommand<'a> {
    pub(super) bwrap: &'a Path,
    pub(super) executable: &'a Path,
    pub(super) input: &'a Path,
    pub(super) output: &'a Path,
    pub(super) operation: ParseOperation,
    pub(super) value: i32,
    pub(super) media_backend: MediaPreviewBackend,
    pub(super) devices: &'a [PathBuf],
}

fn model_argument(render: &ModelRender) -> String {
    let surface = match render.purpose {
        ModelRenderPurpose::Preview => format!("{:06x}", render.palette.surface),
        ModelRenderPurpose::Thumbnail => "transparent".to_owned(),
    };
    format!(
        "{}:{}x{}:{:06x}:{}:{}",
        render.format.argument(),
        render.size.width,
        render.size.height,
        render.palette.accent,
        surface,
        render.view.argument(),
    )
}

/// The `value` argument of a file-producing operation.
fn helper_value(operation: &ParseOperation, value: i32) -> String {
    match operation {
        ParseOperation::PreviewPdf(size) => {
            let size = PdfRenderSize::new(size.width, size.height);
            format!("{value}:{}x{}", size.width, size.height)
        }
        ParseOperation::PreviewModel(render) => model_argument(render),
        ParseOperation::PreviewRoute(render) => format!(
            "{}:{:06x}:{:06x}",
            render.format.label(),
            render.palette.accent,
            render.palette.surface
        ),
        ParseOperation::ThumbnailModel(format) => format.argument().to_owned(),
        ParseOperation::ThumbnailCover(format) => format.argument().to_owned(),
        ParseOperation::PreviewCover(format) => format.argument().to_owned(),
        ParseOperation::PreviewDocument(format) => format.argument().to_owned(),
        ParseOperation::ArchiveList { format, .. } => format.extension().to_owned(),
        _ => value.to_string(),
    }
}

impl SandboxCommand<'_> {
    fn bind_media_devices(&self, command: &mut Command) {
        if self.media_backend == MediaPreviewBackend::Software {
            return;
        }
        // Hardware media drivers need selected render nodes plus read-only sysfs discovery data.
        for device in self.devices {
            command.arg("--dev-bind-try").arg(device).arg(device);
        }
        command.args(["--ro-bind", "/sys", "/sys"]);
    }

    fn file_size_limit(&self) -> u64 {
        if matches!(self.operation, ParseOperation::ThumbnailVideo) {
            MAX_OUTPUT_BYTES
        } else {
            FILE_SIZE_LIMIT_BYTES
        }
    }

    fn bind_output_with_limits(&self, command: &mut Command) {
        command.arg("--bind").arg(self.output).arg("/output");
        // Keep CPU-scaled glibc arenas within the helper's address-space limit.
        command.args(["--setenv", "MALLOC_ARENA_MAX", "1"]);
        command.arg("--");
        command
            .arg(&runtime::current().prlimit)
            .arg(format!("--as={ADDRESS_SPACE_LIMIT_BYTES}"))
            .arg("--cpu=10")
            .arg(format!("--fsize={}", self.file_size_limit()))
            .arg("--");
    }

    fn push_destination(&self, command: &mut Command) {
        match &self.operation {
            ParseOperation::PreviewMedia(size) | ParseOperation::PreviewAudio(size) => {
                let size = MediaPreviewSize::new(size.width, size.height);
                command.arg("/dev/stdout");
                command.arg(format!("{}x{}", size.width, size.height));
            }
            ParseOperation::AudioPeaks => {
                command.arg("/dev/stdout");
                command.arg("0");
            }
            ParseOperation::VideoStoryboard { cell_edge } => {
                command.arg("/dev/stdout");
                command.arg(cell_edge.to_string());
            }
            operation => {
                command.arg(format!("/output/{}", operation.output_name()));
                command.arg(helper_value(operation, self.value));
            }
        }
    }

    pub(super) fn build(self) -> Command {
        let mut command = runtime_command(self.bwrap, self.operation.needs_media_libraries());
        let sandbox_input = sandbox_input_path(self.input);
        command
            .arg("--ro-bind")
            .arg(self.executable)
            .arg("/app/strata");
        command.arg("--ro-bind").arg(self.input).arg(&sandbox_input);
        if self.operation.is_media() || matches!(self.operation, ParseOperation::AudioCover) {
            command.args([
                "--setenv",
                runtime::PRLIMIT_VARIABLE,
                &runtime::current().prlimit,
            ]);
        }
        if self.operation.is_media() {
            self.bind_media_devices(&mut command);
            command.arg("--");
        } else {
            self.bind_output_with_limits(&mut command);
        }
        command.args([
            "/app/strata",
            "--preview-helper",
            self.operation.argument(),
            &sandbox_input,
        ]);
        self.push_destination(&mut command);
        command.arg(self.media_backend.argument());
        command
    }
}

pub(super) fn sandbox_input_path(input: &Path) -> String {
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

fn matching_entries(directory: &Path, matches: impl Fn(&OsStr) -> bool) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| matches(&entry.file_name()))
        .map(|entry| entry.path())
        .collect()
}

pub(crate) fn gpu_devices(dev: &Path, media_backend: MediaPreviewBackend) -> Vec<PathBuf> {
    if media_backend == MediaPreviewBackend::Software {
        return Vec::new();
    }
    let mut devices = matching_entries(&dev.join("dri"), |name| numbered_name(name, "renderD"));
    if media_backend != MediaPreviewBackend::VaApi {
        devices.extend(matching_entries(dev, |name| {
            name == OsStr::new("nvidiactl") || numbered_name(name, "nvidia")
        }));
    }
    devices.sort();
    devices
}

pub(crate) fn polaris_gpu_available() -> bool {
    polaris_gpu_available_at(Path::new("/dev"), Path::new("/sys/class/drm"))
}

pub(super) fn polaris_gpu_available_at(dev: &Path, drm: &Path) -> bool {
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

pub(crate) fn numbered_name(name: &OsStr, prefix: &str) -> bool {
    name.to_str()
        .and_then(|name| name.strip_prefix(prefix))
        .is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        })
}
