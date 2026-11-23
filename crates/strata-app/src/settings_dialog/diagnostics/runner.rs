// SPDX-License-Identifier: MIT

//! Background diagnostics checks, sample-file generators, and system-fact collection.

use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::Sender,
    },
};

use gio::prelude::*;
use strata_core::{i18n::tr, trusted_command};
use strata_desktop::{
    build_info::{COMMIT, displayed_version, updater_enabled},
    services::{InstallSource, update_check},
};
use strata_fs::services::{gio_error_message, io_error_message};
use strata_preview::sandbox::{self, Cancellation, MediaPreviewBackend, ParseOperation};

use super::checks::{
    self, Check, CheckId, VfsState, appimage_icons, audio_playback, drives, file_types_for,
    folder_monitoring_for, gvfs, media_previews, mtp, preview_sandbox, rar_archives, raw_photos,
    remote_backend, sample_check, session_service, update_server, video_thumbnails, workers,
};

type Worker = fn(&Sender<Check>);

const RAW_TOOLS: [&str; 4] = ["magick", "convert", "dcraw", "simple_dcraw"];

pub(super) fn spawn(sender: Sender<Check>) {
    let workers: [(&str, Worker); 3] = [
        ("strata-diagnostics-previews", preview_checks),
        ("strata-diagnostics-system", system_checks),
        ("strata-diagnostics-network", network_checks),
    ];
    for (name, run) in workers {
        let sender = sender.clone();
        if let Err(error) = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || run(&sender))
        {
            tracing::warn!(%error, "could not start a diagnostics worker");
        }
    }
}

/// Collects system facts that require filesystem reads off the UI thread.
pub(super) fn detect_environment() -> Vec<(String, String)> {
    strata_core::util::ui_thread::assert_off();
    let installation = match InstallSource::detect().managed() {
        Some(managed) => managed.ownership_summary(),
        None if !updater_enabled() => tr("In-app updates are disabled in this build."),
        None => tr("Self-managed"),
    };
    let kernel = ["ostype", "osrelease"]
        .map(|name| {
            std::fs::read_to_string(format!("/proc/sys/kernel/{name}"))
                .map(|value| value.trim().to_owned())
                .unwrap_or_default()
        })
        .join(" ");
    environment_rows(Some(installation), Some(kernel.trim().to_owned()))
}

/// Fast UI-thread environment snapshot before background facts resolve.
pub(super) fn initial_environment() -> Vec<(String, String)> {
    environment_rows(None, None)
}

fn environment_rows(installation: Option<String>, kernel: Option<String>) -> Vec<(String, String)> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .ok()
        .filter(|desktop| !desktop.is_empty())
        .unwrap_or_else(|| tr("Unknown"));
    let mut rows = vec![
        (tr("Version"), displayed_version()),
        (tr("Commit"), COMMIT.to_owned()),
    ];
    if let Some(installation) = installation {
        rows.push((tr("Installation"), installation));
    }
    rows.push((tr("Toolkit"), "Rune".to_owned()));
    if let Some(kernel) = kernel.filter(|k| !k.is_empty()) {
        rows.push((tr("Kernel"), kernel));
    }
    rows.extend([
        (tr("Desktop environment"), desktop),
        (tr("Display server"), display_server()),
        (tr("Interface renderer"), "Rune".to_owned()),
        (tr("Language"), rust_i18n::locale().to_string()),
    ]);
    rows
}

fn display_server() -> String {
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
        return "Wayland".to_owned();
    }
    if std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) {
        return "X11".to_owned();
    }
    match std::env::var("XDG_SESSION_TYPE").ok().as_deref() {
        Some("wayland") => "Wayland".to_owned(),
        Some("x11") => "X11".to_owned(),
        Some(other) if !other.is_empty() => other.to_owned(),
        _ => tr("Unknown"),
    }
}

/// Reads the inotify watch limit off the UI thread.
pub(super) fn read_inotify_watch_limit() -> Option<u64> {
    strata_core::util::ui_thread::assert_off();
    std::fs::read_to_string("/proc/sys/fs/inotify/max_user_watches")
        .ok()
        .and_then(|text| text.trim().parse().ok())
}

/// GLib main-context check: verifies that a directory monitor attaches cleanly.
pub(super) fn folder_monitoring(limit: Option<u64>) -> Check {
    let monitored = TempDir::create("strata-diagnostics-")
        .map_err(|error| io_error_message(&error))
        .and_then(|directory| {
            let monitor = gio::File::for_path(directory.path())
                .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
                .map_err(|error| gio_error_message(&error))?;
            monitor.cancel();
            Ok(())
        });
    folder_monitoring_for(monitored, limit)
}

fn preview_checks(sender: &Sender<Check>) {
    let send = |check| {
        let _ = sender.send(check);
    };
    let has = |tool: &str| sandbox::sandbox_tool(tool).is_some();
    match Samples::create() {
        Ok(samples) => run_sample_checks(&samples, &has, &send),
        Err(error) => send_sample_creation_failure(&error, &send),
    }
    let raw_tools = RAW_TOOLS
        .into_iter()
        .filter(|tool| has(tool))
        .collect::<Vec<_>>();
    send(raw_photos(&raw_tools));
    send(appimage_icons(has("unsquashfs")));
    send(file_types());
    send(rar_archives(cfg!(feature = "rar")));
}

fn run_sample_checks(samples: &Samples, has: &impl Fn(&str) -> bool, send: &impl Fn(Check)) {
    send(preview_sandbox(
        trusted_command::resolve_unprivileged("bwrap").is_ok(),
        Path::new(&sandbox::runtime::current().prlimit),
        || render_one_shot(&samples.image),
    ));
    send(workers(sandbox::browser::worker_support, || {
        render_pooled(&samples.image, ParseOperation::ThumbnailImage)
    }));
    send(sample_check(CheckId::PdfThumbnails, || {
        render_pooled(&samples.pdf, ParseOperation::ThumbnailPdf)
    }));
    send(video_thumbnails(
        has("ffmpeg"),
        has("ffmpegthumbnailer"),
        || render_pooled(&samples.video, ParseOperation::ThumbnailVideo),
    ));
    send(media_previews(has("ffmpeg"), has("ffprobe"), || {
        sandbox::browser::metadata(&samples.audio, false, &Cancellation::default()).map(drop)
    }));
}

fn send_sample_creation_failure(error: &std::io::Error, send: &impl Fn(Check)) {
    let error = io_error_message(error);
    for id in [
        CheckId::Sandbox,
        CheckId::Workers,
        CheckId::PdfThumbnails,
        CheckId::VideoThumbnails,
        CheckId::MediaPreviews,
    ] {
        send(Check::not_run(id).with_detail(error.clone()));
    }
}

fn system_checks(sender: &Sender<Check>) {
    let send = |check| {
        let _ = sender.send(check);
    };
    send(audio_playback(strata_platform::media::probe_backend(
        "Strata",
        "Diagnostics",
    )));
    let vfs = detect_vfs();
    send(gvfs(&vfs));
    let session = bus_names(gio::BusType::Session);
    let system = bus_names(gio::BusType::System);
    for id in [
        CheckId::Sftp,
        CheckId::Smb,
        CheckId::Ftp,
        CheckId::WebDav,
        CheckId::Afc,
        CheckId::Cameras,
    ] {
        send(remote_backend(id, &vfs));
    }
    send(mtp(&vfs, session.as_ref()));
    send(drives(session.as_ref(), system.as_ref()));
    send(session_service(CheckId::Portal, session.as_ref()));
    send(session_service(CheckId::SecretService, session.as_ref()));
}

fn network_checks(sender: &Sender<Check>) {
    let _ = sender.send(update_server(update_check::test_connection()));
}

fn render_one_shot(path: &Path) -> Result<(), String> {
    sandbox::parse(
        path,
        ParseOperation::ThumbnailImage,
        256,
        MediaPreviewBackend::Software,
        &Cancellation::default(),
    )
    .map(drop)
}

fn render_pooled(path: &Path, operation: ParseOperation) -> Result<(), String> {
    sandbox::browser::thumbnail(path, operation, &Cancellation::default()).map(drop)
}

fn file_types() -> Check {
    let (content_type, _) = gio::content_type_guess(Some("diagnostics.png"), None);
    file_types_for(&gio::content_type_get_mime_type(&content_type).unwrap_or(content_type))
}

fn detect_vfs() -> VfsState {
    if std::env::var_os("GIO_USE_VFS").is_some_and(|value| value == "local") {
        return VfsState::Bypassed;
    }
    let vfs = gio::Vfs::default();
    if vfs.type_().name() != "GDaemonVfs" {
        return VfsState::Missing;
    }
    VfsState::Running(
        vfs.supported_uri_schemes()
            .iter()
            .map(ToString::to_string)
            .collect(),
    )
}

fn bus_names(bus: gio::BusType) -> Option<HashSet<String>> {
    let connection = gio::bus_get_sync(bus, gio::Cancellable::NONE).ok()?;
    let mut names = HashSet::new();
    for method in ["ListNames", "ListActivatableNames"] {
        let reply = connection
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                method,
                None,
                Some(glib::VariantTy::new("(as)").expect("valid D-Bus reply type")),
                gio::DBusCallFlags::NONE,
                2000,
                gio::Cancellable::NONE,
            )
            .ok()?;
        let (list,) = reply.get::<(Vec<String>,)>()?;
        names.extend(list);
    }
    Some(names)
}

/// Private RAII temporary directory (`0o700`), cleaned up on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn create(prefix: &str) -> std::io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir();
        let pid = std::process::id();
        for _ in 0..64 {
            let seq = NEXT.fetch_add(1, Ordering::Relaxed);
            let candidate = root.join(format!("{prefix}{pid}-{seq}"));
            match std::fs::DirBuilder::new().mode(0o700).create(&candidate) {
                Ok(()) => return Ok(Self { path: candidate }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "unable to allocate diagnostics temporary directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Tiny files in each format a check renders, removed when dropped.
pub(super) struct Samples {
    _directory: TempDir,
    pub(super) image: PathBuf,
    pub(super) pdf: PathBuf,
    video: PathBuf,
    audio: PathBuf,
}

impl Samples {
    pub(super) fn create() -> std::io::Result<Self> {
        let directory = TempDir::create("strata-diagnostics-")?;
        let path = |name: &str| directory.path().join(name);
        let (image, pdf, video, audio) = (
            path("sample.png"),
            path("sample.pdf"),
            path("sample.y4m"),
            path("sample.wav"),
        );
        std::fs::write(&image, sample_png()?)?;
        std::fs::write(&pdf, sample_pdf())?;
        std::fs::write(&video, sample_y4m())?;
        std::fs::write(&audio, sample_wav())?;
        Ok(Self {
            _directory: directory,
            image,
            pdf,
            video,
            audio,
        })
    }
}

fn sample_png() -> std::io::Result<Vec<u8>> {
    let image = image::RgbImage::from_pixel(32, 32, image::Rgb([64, 128, 192]));
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(std::io::Error::other)?;
    Ok(png)
}

/// Minimal single-page 72x72 PDF 1.4 with computed xref offsets, avoiding any Cairo dependency.
pub(super) fn sample_pdf() -> Vec<u8> {
    let stream = "0.25 0.5 0.75 rg\n0 0 72 72 re f\n";
    let objects = [
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_owned(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_owned(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Contents 4 0 R >>\nendobj\n"
            .to_owned(),
        format!(
            "4 0 obj\n<< /Length {} >>\nstream\n{stream}endstream\nendobj\n",
            stream.len()
        ),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for obj in &objects {
        offsets.push(pdf.len());
        pdf.extend_from_slice(obj.as_bytes());
    }
    let xref_offset = pdf.len();
    let _ = writeln!(pdf, "xref\n0 {}", objects.len() + 1);
    let _ = writeln!(pdf, "0000000000 65535 f ");
    for offset in offsets {
        let _ = writeln!(pdf, "{offset:010} 00000 n ");
    }
    let _ = write!(
        pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
        objects.len() + 1
    );
    pdf
}

/// One second of uncompressed 4:2:0 video, which every FFmpeg build decodes.
fn sample_y4m() -> Vec<u8> {
    const EDGE: usize = 32;
    const FRAMES: u8 = 10;
    let mut video =
        format!("YUV4MPEG2 W{EDGE} H{EDGE} F{FRAMES}:1 Ip A1:1 C420jpeg\n").into_bytes();
    for frame in 0..FRAMES {
        video.extend_from_slice(b"FRAME\n");
        video.extend(std::iter::repeat_n(32 + frame * 16, EDGE * EDGE));
        video.extend(std::iter::repeat_n(128, EDGE * EDGE / 2));
    }
    video
}

/// A quarter second of 8 kHz mono silence.
fn sample_wav() -> Vec<u8> {
    const RATE: u32 = 8000;
    let samples = RATE / 4;
    let data_bytes = samples * 2;
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    let mut put = |bytes: &[u8]| wav.write_all(bytes).expect("writing to a vector");
    put(b"RIFF");
    put(&(36 + data_bytes).to_le_bytes());
    put(b"WAVEfmt ");
    put(&16u32.to_le_bytes());
    put(&1u16.to_le_bytes());
    put(&1u16.to_le_bytes());
    put(&RATE.to_le_bytes());
    put(&(RATE * 2).to_le_bytes());
    put(&2u16.to_le_bytes());
    put(&16u16.to_le_bytes());
    put(b"data");
    put(&data_bytes.to_le_bytes());
    wav.resize(44 + data_bytes as usize, 0);
    wav
}

pub(super) fn syntax_highlighting(languages: usize) -> Check {
    checks::syntax_highlighting_for(languages)
}
