// SPDX-License-Identifier: MIT

//! On-demand checks of the runtime components Strata does not link against.
//! Results are language-neutral; Settings → Diagnostics localizes them.

use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc::Sender,
};

use gio::prelude::*;

use super::{network_error::NetworkError, package_manager::PackageManager};
use crate::sandbox::{self, Cancellation, MediaPreviewBackend, ParseOperation};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum CheckId {
    Sandbox,
    Workers,
    PdfThumbnails,
    VideoThumbnails,
    MediaPreviews,
    AudioPlayback,
    RawPhotos,
    AppImageIcons,
    SyntaxHighlighting,
    FileTypes,
    RarArchives,
    Gvfs,
    Sftp,
    Smb,
    Ftp,
    WebDav,
    Mtp,
    Afc,
    Cameras,
    Drives,
    FolderMonitoring,
    UpdateServer,
    Terminal,
    Portal,
    SecretService,
    CustomActions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Group {
    Sandbox,
    Previews,
    Locations,
    Desktop,
}

/// Display order. Every [`CheckId`] appears exactly once.
pub(crate) const GROUPS: [(Group, &[CheckId]); 4] = [
    (Group::Sandbox, &[CheckId::Sandbox, CheckId::Workers]),
    (
        Group::Previews,
        &[
            CheckId::PdfThumbnails,
            CheckId::VideoThumbnails,
            CheckId::MediaPreviews,
            CheckId::AudioPlayback,
            CheckId::RawPhotos,
            CheckId::AppImageIcons,
            CheckId::SyntaxHighlighting,
            CheckId::FileTypes,
            CheckId::RarArchives,
        ],
    ),
    (
        Group::Locations,
        &[
            CheckId::Gvfs,
            CheckId::Sftp,
            CheckId::Smb,
            CheckId::Ftp,
            CheckId::WebDav,
            CheckId::Mtp,
            CheckId::Afc,
            CheckId::Cameras,
            CheckId::Drives,
            CheckId::FolderMonitoring,
        ],
    ),
    (
        Group::Desktop,
        &[
            CheckId::UpdateServer,
            CheckId::Terminal,
            CheckId::Portal,
            CheckId::SecretService,
            CheckId::CustomActions,
        ],
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum Status {
    Passed,
    Warning,
    Problem,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Finding {
    SandboxWorks,
    BubblewrapMissing,
    PrlimitMissing {
        path: String,
    },
    SandboxFailed,
    WorkersWork,
    LandlockUnavailable,
    WorkersFailed,
    SampleProcessed,
    SampleFailed,
    SlowVideoThumbnails,
    Found(Vec<String>),
    Missing(Vec<String>),
    AudioOutput {
        sink: String,
    },
    NoAudioSink,
    GstreamerFailed,
    NoRawConverter,
    LanguagesInstalled,
    LanguagesMissing,
    MimeDatabaseWorks,
    MimeDatabaseMissing,
    RarIncluded,
    RarExcluded,
    GvfsRunning,
    GvfsBypassed,
    GvfsMissing,
    BackendInstalled,
    BackendMissing,
    RequiresGvfs,
    DrivesReady,
    UdisksMissing,
    VolumeMonitorMissing,
    SessionBusUnavailable,
    MonitoringWorks,
    WatchLimitLow {
        limit: u64,
    },
    MonitoringFailed,
    Connected,
    ConnectionFailed(NetworkError),
    TerminalMissing,
    ServiceAvailable,
    PortalMissing,
    SecretServiceMissing,
    NoActions,
    ActionsReady,
    ActionsBroken {
        unavailable: Vec<String>,
        failed: Vec<String>,
    },
    NotRun,
}

pub(crate) type Packages = &'static [(PackageManager, &'static str)];

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Check {
    pub(crate) id: CheckId,
    pub(crate) status: Status,
    pub(crate) finding: Finding,
    /// Packages that fix a failed check, per distribution.
    pub(crate) packages: Packages,
    /// Untranslated library or system text.
    pub(crate) detail: Option<String>,
}

impl Check {
    fn new(id: CheckId, status: Status, finding: Finding) -> Self {
        Self {
            id,
            status,
            finding,
            packages: &[],
            detail: None,
        }
    }

    fn passed(id: CheckId, finding: Finding) -> Self {
        Self::new(id, Status::Passed, finding)
    }

    fn warning(id: CheckId, finding: Finding) -> Self {
        Self::new(id, Status::Warning, finding)
    }

    fn problem(id: CheckId, finding: Finding) -> Self {
        Self::new(id, Status::Problem, finding)
    }

    fn fixed_by(mut self, packages: Packages) -> Self {
        self.packages = packages;
        self
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub(crate) fn not_run(id: CheckId) -> Self {
        Self::warning(id, Finding::NotRun)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum InstallHint {
    Command(String),
    /// No command for this distribution; names the Arch package as a reference.
    Package(&'static str),
}

pub(crate) fn install_hint(
    packages: Packages,
    manager: Option<PackageManager>,
) -> Option<InstallHint> {
    let (_, reference) = packages.first()?;
    manager
        .and_then(|manager| {
            packages
                .iter()
                .find(|(candidate, _)| *candidate == manager)
                .and_then(|(_, package)| manager.install_command(&[package]))
        })
        .map(InstallHint::Command)
        .or(Some(InstallHint::Package(reference)))
}

mod packages {
    use super::{
        PackageManager::{Apt, Dnf, Pacman, Zypper},
        Packages,
    };

    const fn everywhere(name: &'static str) -> [(super::PackageManager, &'static str); 4] {
        [(Pacman, name), (Apt, name), (Dnf, name), (Zypper, name)]
    }

    pub(super) const BUBBLEWRAP: Packages = &everywhere("bubblewrap");
    pub(super) const UTIL_LINUX: Packages = &everywhere("util-linux");
    pub(super) const FFMPEG: Packages = &[
        (Pacman, "ffmpeg"),
        (Apt, "ffmpeg"),
        (Dnf, "ffmpeg-free"),
        (Zypper, "ffmpeg"),
    ];
    pub(super) const FFMPEGTHUMBNAILER: Packages = &everywhere("ffmpegthumbnailer");
    pub(super) const IMAGEMAGICK: Packages = &[
        (Pacman, "imagemagick"),
        (Apt, "imagemagick"),
        (Dnf, "ImageMagick"),
        (Zypper, "ImageMagick"),
    ];
    pub(super) const SQUASHFS: Packages = &everywhere("squashfs-tools");
    pub(super) const GTKSOURCEVIEW: Packages = &[
        (Pacman, "gtksourceview5"),
        (Apt, "libgtksourceview-5-common"),
        (Dnf, "gtksourceview5"),
    ];
    pub(super) const SHARED_MIME_INFO: Packages = &everywhere("shared-mime-info");
    pub(super) const GST_BASE: Packages = &[
        (Pacman, "gst-plugins-base"),
        (Apt, "gstreamer1.0-plugins-base"),
        (Dnf, "gstreamer1-plugins-base"),
        (Zypper, "gstreamer-plugins-base"),
    ];
    pub(super) const GST_GOOD: Packages = &[
        (Pacman, "gst-plugins-good"),
        (Apt, "gstreamer1.0-plugins-good"),
        (Dnf, "gstreamer1-plugins-good"),
        (Zypper, "gstreamer-plugins-good"),
    ];
    pub(super) const GVFS: Packages = &everywhere("gvfs");
    pub(super) const GVFS_DAEMONS: Packages = &[
        (Pacman, "gvfs"),
        (Apt, "gvfs-daemons"),
        (Dnf, "gvfs"),
        (Zypper, "gvfs"),
    ];
    pub(super) const GVFS_NETWORK: Packages = &[
        (Pacman, "gvfs"),
        (Apt, "gvfs-backends"),
        (Dnf, "gvfs"),
        (Zypper, "gvfs-backends"),
    ];
    pub(super) const GVFS_WEBDAV: Packages = &[
        (Pacman, "gvfs-dnssd"),
        (Apt, "gvfs-backends"),
        (Dnf, "gvfs"),
        (Zypper, "gvfs-backends"),
    ];
    pub(super) const GVFS_SMB: Packages = &[
        (Pacman, "gvfs-smb"),
        (Apt, "gvfs-backends"),
        (Dnf, "gvfs-smb"),
        (Zypper, "gvfs-backend-samba"),
    ];
    pub(super) const GVFS_MTP: Packages = &[
        (Pacman, "gvfs-mtp"),
        (Apt, "gvfs-backends"),
        (Dnf, "gvfs-mtp"),
        (Zypper, "gvfs-backends"),
    ];
    pub(super) const GVFS_AFC: Packages = &[
        (Pacman, "gvfs-afc"),
        (Apt, "gvfs-backends"),
        (Dnf, "gvfs-afc"),
        (Zypper, "gvfs-backend-afc"),
    ];
    pub(super) const GVFS_GPHOTO2: Packages = &[
        (Pacman, "gvfs-gphoto2"),
        (Apt, "gvfs-backends"),
        (Dnf, "gvfs-gphoto2"),
        (Zypper, "gvfs-backends"),
    ];
    pub(super) const UDISKS: Packages = &everywhere("udisks2");
    pub(super) const PORTAL: Packages = &everywhere("xdg-desktop-portal");
    pub(super) const KEYRING: Packages = &everywhere("gnome-keyring");
}

type Worker = fn(&Sender<Check>);

/// Runs the background checks on worker threads, sending each result as it
/// completes. The receiver disconnects once every worker has finished.
pub(crate) fn spawn(sender: Sender<Check>) {
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

fn preview_checks(sender: &Sender<Check>) {
    let send = |check| {
        let _ = sender.send(check);
    };
    let has = |tool| sandbox::sandbox_tool(tool).is_some();
    match Samples::create() {
        Ok(samples) => {
            send(preview_sandbox(
                crate::trusted_command::resolve("bwrap").is_ok(),
                Path::new(&sandbox::runtime::current().prlimit),
                || render_one_shot(&samples.image),
            ));
            send(workers(sandbox::browser::workers_supported(), || {
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
                sandbox::browser::metadata(&samples.audio, false, &Cancellation::default())
                    .map(drop)
            }));
        }
        Err(error) => {
            let error = crate::services::io_error_message(&error);
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

fn system_checks(sender: &Sender<Check>) {
    let send = |check| {
        let _ = sender.send(check);
    };
    send(audio_playback(probe_gstreamer()));
    let vfs = VfsState::detect();
    send(gvfs(&vfs));
    for id in [
        CheckId::Sftp,
        CheckId::Smb,
        CheckId::Ftp,
        CheckId::WebDav,
        CheckId::Mtp,
        CheckId::Afc,
        CheckId::Cameras,
    ] {
        send(remote_backend(id, &vfs));
    }
    let session = bus_names(gio::BusType::Session);
    let system = bus_names(gio::BusType::System);
    send(drives(session.as_ref(), system.as_ref()));
    send(session_service(CheckId::Portal, session.as_ref()));
    send(session_service(CheckId::SecretService, session.as_ref()));
}

fn network_checks(sender: &Sender<Check>) {
    let _ = sender.send(update_server(super::update_check::test_connection()));
}

const RAW_TOOLS: [&str; 4] = ["magick", "convert", "dcraw", "simple_dcraw"];

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

fn preview_sandbox(
    bubblewrap: bool,
    prlimit: &Path,
    render: impl FnOnce() -> Result<(), String>,
) -> Check {
    let id = CheckId::Sandbox;
    if !bubblewrap {
        return Check::problem(id, Finding::BubblewrapMissing).fixed_by(packages::BUBBLEWRAP);
    }
    if !prlimit.is_file() {
        return Check::problem(
            id,
            Finding::PrlimitMissing {
                path: prlimit.display().to_string(),
            },
        )
        .fixed_by(packages::UTIL_LINUX);
    }
    match render() {
        Ok(()) => Check::passed(id, Finding::SandboxWorks),
        Err(error) => Check::problem(id, Finding::SandboxFailed).with_detail(error),
    }
}

fn workers(supported: bool, render: impl FnOnce() -> Result<(), String>) -> Check {
    let id = CheckId::Workers;
    if !supported {
        return Check::warning(id, Finding::LandlockUnavailable);
    }
    match render() {
        Ok(()) => Check::passed(id, Finding::WorkersWork),
        Err(error) => Check::problem(id, Finding::WorkersFailed).with_detail(error),
    }
}

fn sample_check(id: CheckId, render: impl FnOnce() -> Result<(), String>) -> Check {
    match render() {
        Ok(()) => Check::passed(id, Finding::SampleProcessed),
        Err(error) => Check::problem(id, Finding::SampleFailed).with_detail(error),
    }
}

fn video_thumbnails(
    ffmpeg: bool,
    thumbnailer: bool,
    render: impl FnOnce() -> Result<(), String>,
) -> Check {
    let id = CheckId::VideoThumbnails;
    if !ffmpeg && !thumbnailer {
        return Check::problem(id, Finding::Missing(vec!["ffmpegthumbnailer".into()]))
            .fixed_by(packages::FFMPEGTHUMBNAILER);
    }
    let check = sample_check(id, render);
    match (check.status, thumbnailer) {
        (_, true) => check,
        (Status::Passed, false) => {
            Check::warning(id, Finding::SlowVideoThumbnails).fixed_by(packages::FFMPEGTHUMBNAILER)
        }
        (_, false) => check.fixed_by(packages::FFMPEGTHUMBNAILER),
    }
}

fn media_previews(ffmpeg: bool, ffprobe: bool, read: impl FnOnce() -> Result<(), String>) -> Check {
    let id = CheckId::MediaPreviews;
    let missing = [("ffmpeg", ffmpeg), ("ffprobe", ffprobe)]
        .into_iter()
        .filter(|(_, found)| !found)
        .map(|(name, _)| name.to_owned())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Check::problem(id, Finding::Missing(missing)).fixed_by(packages::FFMPEG);
    }
    sample_check(id, read)
}

/// GStreamer elements the preview audio pipeline links, and the package of each.
const AUDIO_ELEMENTS: [(&str, Packages); 5] = [
    ("appsrc", packages::GST_BASE),
    ("audioconvert", packages::GST_BASE),
    ("audioresample", packages::GST_BASE),
    ("volume", packages::GST_BASE),
    ("autoaudiosink", packages::GST_GOOD),
];

struct GstreamerProbe {
    missing: Vec<&'static str>,
    sink: Option<String>,
}

fn probe_gstreamer() -> Result<GstreamerProbe, String> {
    use gstreamer as gst;
    use gstreamer::prelude::*;

    gst::init().map_err(|error| error.to_string())?;
    let missing = AUDIO_ELEMENTS
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| gst::ElementFactory::find(name).is_none())
        .collect();
    let mut sinks: Vec<_> = gst::ElementFactory::factories_with_type(
        gst::ElementFactoryType::SINK | gst::ElementFactoryType::MEDIA_AUDIO,
        gst::Rank::MARGINAL,
    )
    .into_iter()
    .collect();
    sinks.sort_by_key(|factory| std::cmp::Reverse(factory.rank()));
    Ok(GstreamerProbe {
        missing,
        sink: sinks.first().map(|factory| factory.longname().to_owned()),
    })
}

fn audio_playback(probe: Result<GstreamerProbe, String>) -> Check {
    let id = CheckId::AudioPlayback;
    let probe = match probe {
        Ok(probe) => probe,
        Err(error) => {
            return Check::problem(id, Finding::GstreamerFailed)
                .fixed_by(packages::GST_BASE)
                .with_detail(error);
        }
    };
    if let Some(first) = probe.missing.first() {
        let packages = AUDIO_ELEMENTS
            .iter()
            .find(|(name, _)| name == first)
            .map_or(packages::GST_BASE, |(_, packages)| *packages);
        return Check::problem(
            id,
            Finding::Missing(probe.missing.iter().map(|name| (*name).into()).collect()),
        )
        .fixed_by(packages);
    }
    match probe.sink {
        Some(sink) => Check::passed(id, Finding::AudioOutput { sink }),
        None => Check::problem(id, Finding::NoAudioSink).fixed_by(packages::GST_GOOD),
    }
}

fn raw_photos(found: &[&str]) -> Check {
    if found.is_empty() {
        Check::warning(CheckId::RawPhotos, Finding::NoRawConverter).fixed_by(packages::IMAGEMAGICK)
    } else {
        Check::passed(
            CheckId::RawPhotos,
            Finding::Found(found.iter().map(|name| (*name).into()).collect()),
        )
    }
}

fn appimage_icons(found: bool) -> Check {
    let tool = vec!["unsquashfs".to_owned()];
    if found {
        Check::passed(CheckId::AppImageIcons, Finding::Found(tool))
    } else {
        Check::warning(CheckId::AppImageIcons, Finding::Missing(tool)).fixed_by(packages::SQUASHFS)
    }
}

/// GTK thread only: the language manager is a GObject singleton.
pub(crate) fn syntax_highlighting() -> Check {
    let languages = sourceview5::LanguageManager::default().language_ids().len();
    syntax_highlighting_for(languages)
}

fn syntax_highlighting_for(languages: usize) -> Check {
    if languages == 0 {
        Check::warning(CheckId::SyntaxHighlighting, Finding::LanguagesMissing)
            .fixed_by(packages::GTKSOURCEVIEW)
    } else {
        Check::passed(CheckId::SyntaxHighlighting, Finding::LanguagesInstalled)
    }
}

fn file_types() -> Check {
    let (content_type, _) = gio::content_type_guess(Some("diagnostics.png"), None);
    file_types_for(&gio::content_type_get_mime_type(&content_type).unwrap_or(content_type))
}

fn file_types_for(png_type: &str) -> Check {
    if png_type == "image/png" {
        Check::passed(CheckId::FileTypes, Finding::MimeDatabaseWorks)
    } else {
        Check::problem(CheckId::FileTypes, Finding::MimeDatabaseMissing)
            .fixed_by(packages::SHARED_MIME_INFO)
    }
}

fn rar_archives(included: bool) -> Check {
    if included {
        Check::passed(CheckId::RarArchives, Finding::RarIncluded)
    } else {
        Check::warning(CheckId::RarArchives, Finding::RarExcluded)
    }
}

#[derive(Debug)]
enum VfsState {
    /// `GIO_USE_VFS=local`, set by the startup fallback when GVfs hangs.
    Bypassed,
    Missing,
    Running(HashSet<String>),
}

impl VfsState {
    fn detect() -> Self {
        if std::env::var_os("GIO_USE_VFS").is_some_and(|value| value == "local") {
            return Self::Bypassed;
        }
        let vfs = gio::Vfs::default();
        if vfs.type_().name() != "GDaemonVfs" {
            return Self::Missing;
        }
        Self::Running(
            vfs.supported_uri_schemes()
                .iter()
                .map(ToString::to_string)
                .collect(),
        )
    }
}

fn gvfs(state: &VfsState) -> Check {
    let id = CheckId::Gvfs;
    match state {
        VfsState::Running(_) => Check::passed(id, Finding::GvfsRunning),
        VfsState::Bypassed => Check::problem(id, Finding::GvfsBypassed),
        VfsState::Missing => Check::problem(id, Finding::GvfsMissing).fixed_by(packages::GVFS),
    }
}

fn backend(id: CheckId) -> (&'static str, Packages) {
    match id {
        CheckId::Sftp => ("sftp", packages::GVFS_NETWORK),
        CheckId::Smb => ("smb", packages::GVFS_SMB),
        CheckId::Ftp => ("ftp", packages::GVFS_NETWORK),
        CheckId::WebDav => ("dav", packages::GVFS_WEBDAV),
        CheckId::Mtp => ("mtp", packages::GVFS_MTP),
        CheckId::Afc => ("afc", packages::GVFS_AFC),
        CheckId::Cameras => ("gphoto2", packages::GVFS_GPHOTO2),
        _ => unreachable!("{id:?} is not a GVfs backend"),
    }
}

fn remote_backend(id: CheckId, state: &VfsState) -> Check {
    let (scheme, packages) = backend(id);
    match state {
        VfsState::Running(schemes) if schemes.contains(scheme) => {
            Check::passed(id, Finding::BackendInstalled)
        }
        VfsState::Running(_) => Check::warning(id, Finding::BackendMissing).fixed_by(packages),
        VfsState::Bypassed | VfsState::Missing => Check::warning(id, Finding::RequiresGvfs),
    }
}

/// Owned or activatable names, or `None` when the bus is unreachable.
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

const UDISKS_NAME: &str = "org.freedesktop.UDisks2";
const VOLUME_MONITOR_NAME: &str = "org.gtk.vfs.UDisks2VolumeMonitor";
const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const SECRETS_NAME: &str = "org.freedesktop.secrets";

fn drives(session: Option<&HashSet<String>>, system: Option<&HashSet<String>>) -> Check {
    let id = CheckId::Drives;
    if !system.is_some_and(|names| names.contains(UDISKS_NAME)) {
        return Check::warning(id, Finding::UdisksMissing).fixed_by(packages::UDISKS);
    }
    let Some(session) = session else {
        return Check::warning(id, Finding::SessionBusUnavailable);
    };
    if !session.contains(VOLUME_MONITOR_NAME) {
        return Check::warning(id, Finding::VolumeMonitorMissing).fixed_by(packages::GVFS_DAEMONS);
    }
    Check::passed(id, Finding::DrivesReady)
}

fn session_service(id: CheckId, session: Option<&HashSet<String>>) -> Check {
    let (name, missing, packages) = match id {
        CheckId::Portal => (PORTAL_NAME, Finding::PortalMissing, packages::PORTAL),
        CheckId::SecretService => (
            SECRETS_NAME,
            Finding::SecretServiceMissing,
            packages::KEYRING,
        ),
        _ => unreachable!("{id:?} is not a session service"),
    };
    match session {
        None => Check::warning(id, Finding::SessionBusUnavailable),
        Some(names) if names.contains(name) => Check::passed(id, Finding::ServiceAvailable),
        Some(_) => Check::warning(id, missing).fixed_by(packages),
    }
}

/// Below the kernel's minimum default, other applications can exhaust it.
const LOW_WATCH_LIMIT: u64 = 8192;

/// GTK thread only: the monitor attaches to the thread's main context.
pub(crate) fn folder_monitoring() -> Check {
    let limit = std::fs::read_to_string("/proc/sys/fs/inotify/max_user_watches")
        .ok()
        .and_then(|text| text.trim().parse().ok());
    let monitored = tempfile::Builder::new()
        .prefix("strata-diagnostics-")
        .tempdir()
        .map_err(|error| crate::services::io_error_message(&error))
        .and_then(|directory| {
            let monitor = gio::File::for_path(directory.path())
                .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
                .map_err(|error| crate::services::gio_error_message(&error))?;
            monitor.cancel();
            Ok(())
        });
    folder_monitoring_for(monitored, limit)
}

fn folder_monitoring_for(monitored: Result<(), String>, limit: Option<u64>) -> Check {
    let id = CheckId::FolderMonitoring;
    match (monitored, limit) {
        (Err(error), _) => Check::problem(id, Finding::MonitoringFailed).with_detail(error),
        (Ok(()), Some(limit)) if limit < LOW_WATCH_LIMIT => {
            Check::warning(id, Finding::WatchLimitLow { limit })
        }
        (Ok(()), _) => Check::passed(id, Finding::MonitoringWorks),
    }
}

fn update_server(result: Result<(), NetworkError>) -> Check {
    match result {
        Ok(()) => Check::passed(CheckId::UpdateServer, Finding::Connected),
        Err(error) => Check::problem(CheckId::UpdateServer, Finding::ConnectionFailed(error)),
    }
}

pub(crate) fn terminal(program: Option<String>) -> Check {
    match program {
        Some(program) => Check::passed(CheckId::Terminal, Finding::Found(vec![program])),
        None => Check::problem(CheckId::Terminal, Finding::TerminalMissing),
    }
}

pub(crate) fn custom_actions(
    installed: usize,
    unavailable: Vec<String>,
    failed: Vec<String>,
) -> Check {
    let id = CheckId::CustomActions;
    if !unavailable.is_empty() || !failed.is_empty() {
        return Check::warning(
            id,
            Finding::ActionsBroken {
                unavailable,
                failed,
            },
        );
    }
    if installed == 0 {
        Check::passed(id, Finding::NoActions)
    } else {
        Check::passed(id, Finding::ActionsReady)
    }
}

/// Tiny files in each format a check renders, removed when dropped.
struct Samples {
    _directory: tempfile::TempDir,
    image: PathBuf,
    pdf: PathBuf,
    video: PathBuf,
    audio: PathBuf,
}

impl Samples {
    fn create() -> std::io::Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("strata-diagnostics-")
            .tempdir()?;
        let path = |name: &str| directory.path().join(name);
        let (image, pdf, video, audio) = (
            path("sample.png"),
            path("sample.pdf"),
            path("sample.y4m"),
            path("sample.wav"),
        );
        std::fs::write(&image, sample_png()?)?;
        write_sample_pdf(&pdf)?;
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

fn write_sample_pdf(path: &Path) -> std::io::Result<()> {
    let surface = cairo::PdfSurface::new(72.0, 72.0, path).map_err(std::io::Error::other)?;
    {
        let context = cairo::Context::new(&surface).map_err(std::io::Error::other)?;
        context.set_source_rgb(0.25, 0.5, 0.75);
        context.paint().map_err(std::io::Error::other)?;
        context.show_page().map_err(std::io::Error::other)?;
    }
    surface.finish();
    Ok(())
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
