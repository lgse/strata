// SPDX-License-Identifier: MIT

//! Diagnostics check model and pure evaluation logic, matching `src/services/diagnostics.rs`.

use std::{collections::HashSet, path::Path};

use rune::AudioBackend;
use strata_desktop::services::package_manager::PackageManager;
use strata_fs::services::network_error::NetworkError;
use strata_preview::sandbox::browser::WorkerSupport;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(super) enum CheckId {
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

impl CheckId {
    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Workers => "workers",
            Self::PdfThumbnails => "pdf-thumbnails",
            Self::VideoThumbnails => "video-thumbnails",
            Self::MediaPreviews => "media-previews",
            Self::AudioPlayback => "audio-playback",
            Self::RawPhotos => "raw-photos",
            Self::AppImageIcons => "appimage-icons",
            Self::SyntaxHighlighting => "syntax-highlighting",
            Self::FileTypes => "file-types",
            Self::RarArchives => "rar-archives",
            Self::Gvfs => "gvfs",
            Self::Sftp => "sftp",
            Self::Smb => "smb",
            Self::Ftp => "ftp",
            Self::WebDav => "webdav",
            Self::Mtp => "mtp",
            Self::Afc => "afc",
            Self::Cameras => "cameras",
            Self::Drives => "drives",
            Self::FolderMonitoring => "folder-monitoring",
            Self::UpdateServer => "update-server",
            Self::Terminal => "terminal",
            Self::Portal => "portal",
            Self::SecretService => "secret-service",
            Self::CustomActions => "custom-actions",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Group {
    Sandbox,
    Previews,
    Locations,
    Desktop,
}

/// Display order. Every [`CheckId`] appears exactly once.
pub(super) const GROUPS: [(Group, &[CheckId]); 4] = [
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
pub(super) enum Status {
    Passed,
    Warning,
    Problem,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Finding {
    SandboxWorks,
    BubblewrapMissing,
    PrlimitMissing {
        path: String,
    },
    SandboxFailed,
    WorkersWork,
    LandlockUnavailable,
    NamespacesRestricted,
    WorkersFailed,
    SampleProcessed,
    SampleFailed,
    SlowVideoThumbnails,
    Found(Vec<String>),
    Missing(Vec<String>),
    AudioOutput {
        sink: String,
    },
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
    KdeMtpConflict,
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

pub(super) type Packages = &'static [(PackageManager, &'static str)];

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Check {
    pub(super) id: CheckId,
    pub(super) status: Status,
    pub(super) finding: Finding,
    pub(super) packages: Packages,
    pub(super) detail: Option<String>,
}

impl Check {
    pub(super) fn new(id: CheckId, status: Status, finding: Finding) -> Self {
        Self {
            id,
            status,
            finding,
            packages: &[],
            detail: None,
        }
    }

    pub(super) fn passed(id: CheckId, finding: Finding) -> Self {
        Self::new(id, Status::Passed, finding)
    }

    pub(super) fn warning(id: CheckId, finding: Finding) -> Self {
        Self::new(id, Status::Warning, finding)
    }

    pub(super) fn problem(id: CheckId, finding: Finding) -> Self {
        Self::new(id, Status::Problem, finding)
    }

    pub(super) fn fixed_by(mut self, packages: Packages) -> Self {
        self.packages = packages;
        self
    }

    pub(super) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub(super) fn not_run(id: CheckId) -> Self {
        Self::warning(id, Finding::NotRun)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum InstallHint {
    Command(String),
    Package(&'static str),
}

pub(super) fn install_hint(
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

pub(super) mod packages {
    use super::{
        PackageManager::{Apt, Dnf, Pacman, Zypper},
        Packages,
    };

    const fn everywhere(name: &'static str) -> [(super::PackageManager, &'static str); 4] {
        [(Pacman, name), (Apt, name), (Dnf, name), (Zypper, name)]
    }

    pub(in super::super) const BUBBLEWRAP: Packages = &everywhere("bubblewrap");
    pub(in super::super) const UTIL_LINUX: Packages = &everywhere("util-linux");
    pub(in super::super) const FFMPEG: Packages = &[
        (Pacman, "ffmpeg"),
        (Apt, "ffmpeg"),
        (Dnf, "ffmpeg-free"),
        (Zypper, "ffmpeg"),
    ];
    pub(in super::super) const FFMPEGTHUMBNAILER: Packages = &everywhere("ffmpegthumbnailer");
    pub(in super::super) const IMAGEMAGICK: Packages = &[
        (Pacman, "imagemagick"),
        (Apt, "imagemagick"),
        (Dnf, "ImageMagick"),
        (Zypper, "ImageMagick"),
    ];
    pub(in super::super) const SQUASHFS: Packages = &everywhere("squashfs-tools");
    pub(in super::super) const SHARED_MIME_INFO: Packages = &everywhere("shared-mime-info");
    pub(in super::super) const GVFS: Packages = &everywhere("gvfs");
    pub(in super::super) const GVFS_DAEMONS: Packages = &[
        (Pacman, "gvfs"),
        (Apt, "gvfs-daemons"),
        (Dnf, "gvfs"),
        (Zypper, "gvfs"),
    ];
    pub(in super::super) const GVFS_NETWORK: Packages = &[
        (Pacman, "gvfs"),
        (Apt, "gvfs-backends"),
        (Dnf, "gvfs"),
        (Zypper, "gvfs-backends"),
    ];
    pub(in super::super) const GVFS_WEBDAV: Packages = &[
        (Pacman, "gvfs-dnssd"),
        (Apt, "gvfs-backends"),
        (Dnf, "gvfs"),
        (Zypper, "gvfs-backends"),
    ];
    pub(in super::super) const GVFS_SMB: Packages = &[
        (Pacman, "gvfs-smb"),
        (Apt, "gvfs-backends"),
        (Dnf, "gvfs-smb"),
        (Zypper, "gvfs-backend-samba"),
    ];
    pub(in super::super) const GVFS_MTP: Packages = &[
        (Pacman, "gvfs-mtp"),
        (Apt, "gvfs-backends"),
        (Dnf, "gvfs-mtp"),
        (Zypper, "gvfs-backends"),
    ];
    pub(in super::super) const GVFS_AFC: Packages = &[
        (Pacman, "gvfs-afc"),
        (Apt, "gvfs-backends"),
        (Dnf, "gvfs-afc"),
        (Zypper, "gvfs-backend-afc"),
    ];
    pub(in super::super) const GVFS_GPHOTO2: Packages = &[
        (Pacman, "gvfs-gphoto2"),
        (Apt, "gvfs-backends"),
        (Dnf, "gvfs-gphoto2"),
        (Zypper, "gvfs-backends"),
    ];
    pub(in super::super) const UDISKS: Packages = &everywhere("udisks2");
    pub(in super::super) const PORTAL: Packages = &everywhere("xdg-desktop-portal");
    pub(in super::super) const KEYRING: Packages = &everywhere("gnome-keyring");
}

#[derive(Debug)]
pub(super) enum VfsState {
    Bypassed,
    Missing,
    Running(HashSet<String>),
}

const KDE_MTP_NAME: &str = "org.kde.kmtpd5";
const UDISKS_NAME: &str = "org.freedesktop.UDisks2";
const VOLUME_MONITOR_NAME: &str = "org.gtk.vfs.UDisks2VolumeMonitor";
const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const SECRETS_NAME: &str = "org.freedesktop.secrets";
const LOW_WATCH_LIMIT: u64 = 8192;

pub(super) fn preview_sandbox(
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

pub(super) fn workers(
    support: impl Fn() -> WorkerSupport,
    render: impl FnOnce() -> Result<(), String>,
) -> Check {
    let id = CheckId::Workers;
    if let Some(finding) = one_shot_reason(support()) {
        return Check::warning(id, finding);
    }
    match render() {
        Ok(()) => match one_shot_reason(support()) {
            Some(finding) => Check::warning(id, finding),
            None => Check::passed(id, Finding::WorkersWork),
        },
        Err(error) => Check::problem(id, Finding::WorkersFailed).with_detail(error),
    }
}

fn one_shot_reason(support: WorkerSupport) -> Option<Finding> {
    match support {
        WorkerSupport::Available => None,
        WorkerSupport::LandlockUnavailable => Some(Finding::LandlockUnavailable),
        WorkerSupport::NamespacesRestricted => Some(Finding::NamespacesRestricted),
    }
}

pub(super) fn sample_check(id: CheckId, render: impl FnOnce() -> Result<(), String>) -> Check {
    match render() {
        Ok(()) => Check::passed(id, Finding::SampleProcessed),
        Err(error) => Check::problem(id, Finding::SampleFailed).with_detail(error),
    }
}

pub(super) fn video_thumbnails(
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

pub(super) fn media_previews(
    ffmpeg: bool,
    ffprobe: bool,
    read: impl FnOnce() -> Result<(), String>,
) -> Check {
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

pub(super) fn audio_playback(backend: AudioBackend) -> Check {
    match backend {
        AudioBackend::PulseAudio => Check::passed(
            CheckId::AudioPlayback,
            Finding::AudioOutput {
                sink: "PulseAudio".to_owned(),
            },
        ),
        AudioBackend::Alsa => Check::passed(
            CheckId::AudioPlayback,
            Finding::AudioOutput {
                sink: "ALSA".to_owned(),
            },
        ),
        _ => Check::not_run(CheckId::AudioPlayback),
    }
}

pub(super) fn raw_photos(found: &[&str]) -> Check {
    if found.is_empty() {
        Check::warning(CheckId::RawPhotos, Finding::NoRawConverter).fixed_by(packages::IMAGEMAGICK)
    } else {
        Check::passed(
            CheckId::RawPhotos,
            Finding::Found(found.iter().map(|name| (*name).into()).collect()),
        )
    }
}

pub(super) fn appimage_icons(found: bool) -> Check {
    let tool = vec!["unsquashfs".to_owned()];
    if found {
        Check::passed(CheckId::AppImageIcons, Finding::Found(tool))
    } else {
        Check::warning(CheckId::AppImageIcons, Finding::Missing(tool)).fixed_by(packages::SQUASHFS)
    }
}

pub(super) fn syntax_highlighting_for(languages: usize) -> Check {
    if languages == 0 {
        Check::warning(CheckId::SyntaxHighlighting, Finding::LanguagesMissing)
    } else {
        Check::passed(CheckId::SyntaxHighlighting, Finding::LanguagesInstalled)
    }
}

pub(super) fn file_types_for(png_type: &str) -> Check {
    if png_type == "image/png" {
        Check::passed(CheckId::FileTypes, Finding::MimeDatabaseWorks)
    } else {
        Check::problem(CheckId::FileTypes, Finding::MimeDatabaseMissing)
            .fixed_by(packages::SHARED_MIME_INFO)
    }
}

pub(super) fn rar_archives(included: bool) -> Check {
    if included {
        Check::passed(CheckId::RarArchives, Finding::RarIncluded)
    } else {
        Check::warning(CheckId::RarArchives, Finding::RarExcluded)
    }
}

pub(super) fn gvfs(state: &VfsState) -> Check {
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

pub(super) fn remote_backend(id: CheckId, state: &VfsState) -> Check {
    let (scheme, packages) = backend(id);
    match state {
        VfsState::Running(schemes) if schemes.contains(scheme) => {
            Check::passed(id, Finding::BackendInstalled)
        }
        VfsState::Running(_) => Check::warning(id, Finding::BackendMissing).fixed_by(packages),
        VfsState::Bypassed | VfsState::Missing => Check::warning(id, Finding::RequiresGvfs),
    }
}

pub(super) fn mtp(state: &VfsState, session: Option<&HashSet<String>>) -> Check {
    let check = remote_backend(CheckId::Mtp, state);
    let kde_mtp = session.is_some_and(|names| names.contains(KDE_MTP_NAME));
    if check.status == Status::Passed && kde_mtp {
        return Check::warning(CheckId::Mtp, Finding::KdeMtpConflict);
    }
    check
}

pub(super) fn drives(session: Option<&HashSet<String>>, system: Option<&HashSet<String>>) -> Check {
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

pub(super) fn session_service(id: CheckId, session: Option<&HashSet<String>>) -> Check {
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

pub(super) fn folder_monitoring_for(monitored: Result<(), String>, limit: Option<u64>) -> Check {
    let id = CheckId::FolderMonitoring;
    match (monitored, limit) {
        (Err(error), _) => Check::problem(id, Finding::MonitoringFailed).with_detail(error),
        (Ok(()), Some(limit)) if limit < LOW_WATCH_LIMIT => {
            Check::warning(id, Finding::WatchLimitLow { limit })
        }
        (Ok(()), _) => Check::passed(id, Finding::MonitoringWorks),
    }
}

pub(super) fn update_server(result: Result<(), NetworkError>) -> Check {
    match result {
        Ok(()) => Check::passed(CheckId::UpdateServer, Finding::Connected),
        Err(error) => Check::problem(CheckId::UpdateServer, Finding::ConnectionFailed(error)),
    }
}

pub(super) fn terminal(program: Option<String>) -> Check {
    match program {
        Some(program) => Check::passed(CheckId::Terminal, Finding::Found(vec![program])),
        None => Check::problem(CheckId::Terminal, Finding::TerminalMissing),
    }
}

pub(super) fn custom_actions(
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
