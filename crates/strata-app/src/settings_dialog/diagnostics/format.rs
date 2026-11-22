// SPDX-License-Identifier: MIT

//! Localization and plain-text report formatting for Settings -> Diagnostics,
//! matching `src/ui/settings/diagnostics.rs`.

use std::collections::HashMap;

use strata_core::i18n::{integer, list, tr};
use strata_desktop::services::package_manager::PackageManager;

use super::checks::{Check, CheckId, Finding, GROUPS, Group, InstallHint, Status, install_hint};

pub(super) fn heading(group: Group) -> &'static str {
    match group {
        Group::Sandbox => "PREVIEW SANDBOX",
        Group::Previews => "PREVIEWS",
        Group::Locations => "LOCATIONS AND DEVICES",
        Group::Desktop => "DESKTOP AND NETWORK",
    }
}

pub(super) fn title(id: CheckId) -> &'static str {
    match id {
        CheckId::Sandbox => "Preview sandbox",
        CheckId::Workers => "Sandboxed workers",
        CheckId::PdfThumbnails => "PDF thumbnails",
        CheckId::VideoThumbnails => "Video thumbnails",
        CheckId::MediaPreviews => "Video and audio previews",
        CheckId::AudioPlayback => "Audio playback",
        CheckId::RawPhotos => "RAW photos",
        CheckId::AppImageIcons => "AppImage icons",
        CheckId::SyntaxHighlighting => "Syntax highlighting",
        CheckId::FileTypes => "File type detection",
        CheckId::RarArchives => "RAR archives",
        CheckId::Gvfs => "GVfs",
        CheckId::Sftp => "SFTP",
        CheckId::Smb => "Windows shares (SMB)",
        CheckId::Ftp => "FTP",
        CheckId::WebDav => "WebDAV",
        CheckId::Mtp => "Android devices (MTP)",
        CheckId::Afc => "iPhone and iPad",
        CheckId::Cameras => "Cameras",
        CheckId::Drives => "Removable drives",
        CheckId::FolderMonitoring => "Folder change monitoring",
        CheckId::UpdateServer => "Update server",
        CheckId::Terminal => "Terminal",
        CheckId::Portal => "Desktop portal",
        CheckId::SecretService => "Password storage",
        CheckId::CustomActions => "Custom actions",
    }
}

pub(super) fn status_label(status: Status) -> &'static str {
    match status {
        Status::Passed => "Passed",
        Status::Warning => "Warning",
        Status::Problem => "Problem",
    }
}

pub(super) fn message(check: &Check) -> String {
    let items = |names: &[String]| list(names.iter().cloned());
    match &check.finding {
        Finding::PrlimitMissing { path } => rust_i18n::t!(
            "prlimit wasn't found at %{path}, so no thumbnails or previews can be shown.",
            path = path
        )
        .into_owned(),
        Finding::Found(names) => {
            rust_i18n::t!("Found: %{items}", items = items(names)).into_owned()
        }
        Finding::Missing(names) => {
            rust_i18n::t!("Not installed: %{items}", items = items(names)).into_owned()
        }
        Finding::WatchLimitLow { limit } => rust_i18n::t!(
            "The inotify watch limit is %{limit}, so some folders may not refresh automatically.",
            limit = integer(*limit)
        )
        .into_owned(),
        Finding::TerminalMissing => crate::terminal::no_terminal_message(),
        Finding::ActionsBroken {
            unavailable,
            failed,
        } => actions_broken_message(unavailable, failed),
        finding => tr(sentence(finding)),
    }
}

fn actions_broken_message(unavailable: &[String], failed: &[String]) -> String {
    let mut lines = Vec::new();
    if !unavailable.is_empty() {
        lines.push(rust_i18n::t!(
            "Can't run: %{items}",
            items = list(unavailable.iter().cloned())
        ));
    }
    if !failed.is_empty() {
        lines.push(rust_i18n::t!(
            "Couldn't load: %{items}",
            items = list(failed.iter().cloned())
        ));
    }
    lines.join("\n")
}

/// The catalog key of a finding whose message has no interpolated values.
fn sentence(finding: &Finding) -> &'static str {
    match finding {
        Finding::SandboxWorks => "A sample image rendered in the sandbox.",
        Finding::BubblewrapMissing => {
            "Bubblewrap (bwrap) isn't installed, so no thumbnails or previews can be shown."
        }
        Finding::SandboxFailed => {
            "The sandbox couldn't start. Unprivileged user namespaces may be disabled or restricted, for example by AppArmor."
        }
        Finding::WorkersWork => "Thumbnails reuse sandboxed worker processes.",
        Finding::LandlockUnavailable => {
            "The kernel doesn't support Landlock ABI 3, so each thumbnail starts its own sandbox and loads more slowly."
        }
        Finding::NamespacesRestricted => {
            "The system blocks nested user namespaces, for example through AppArmor, so each thumbnail starts its own sandbox and loads more slowly."
        }
        Finding::WorkersFailed => "A sandboxed worker couldn't render a sample image.",
        Finding::SampleProcessed => "A sample file was processed correctly.",
        Finding::SampleFailed => "A sample file couldn't be processed.",
        Finding::SlowVideoThumbnails => {
            "ffmpegthumbnailer isn't installed, so video thumbnails use FFmpeg and load more slowly."
        }
        Finding::NoRawConverter => {
            "No RAW converter was found, so RAW photos show only their embedded preview."
        }
        Finding::LanguagesInstalled => "Language definitions are installed.",
        Finding::LanguagesMissing => {
            "No language definitions were found, so code previews show plain text."
        }
        Finding::MimeDatabaseWorks => "File types are recognized.",
        Finding::MimeDatabaseMissing => {
            "The shared MIME database is missing, so files can't be recognized by type."
        }
        Finding::RarIncluded => "This build includes RAR support.",
        Finding::RarExcluded => "This build doesn't include RAR support.",
        Finding::GvfsRunning => "GVfs is running.",
        Finding::GvfsBypassed => {
            "This session uses local files only because GVfs didn't respond when Strata started. Trash, network locations and devices are unavailable until Strata restarts with GVfs working."
        }
        Finding::GvfsMissing => {
            "GVfs isn't running, so trash, network locations and devices are unavailable."
        }
        Finding::BackendInstalled => "The GVfs backend is installed.",
        Finding::BackendMissing => "The GVfs backend isn't installed.",
        Finding::KdeMtpConflict => {
            "KDE's MTP service is also installed, and whichever claims a phone first leaves the other with no storage. If a phone appears empty, unplug it, run pkill -x kiod6, then reconnect it unlocked."
        }
        Finding::RequiresGvfs => "Unavailable while GVfs isn't running.",
        Finding::DrivesReady => "UDisks2 and the GVfs volume monitor are available.",
        Finding::UdisksMissing => {
            "UDisks2 isn't available, so drives can't be mounted, formatted or unlocked."
        }
        Finding::VolumeMonitorMissing => {
            "The GVfs UDisks2 volume monitor isn't installed, so drives may not appear in the sidebar."
        }
        Finding::SessionBusUnavailable => "The D-Bus session bus isn't available.",
        Finding::MonitoringWorks => "Folder changes are detected automatically.",
        Finding::MonitoringFailed => {
            "Folder changes can't be detected, so folders won't refresh automatically."
        }
        Finding::Connected => "Connected to GitHub.",
        Finding::ConnectionFailed(_) => {
            "Couldn't connect to GitHub, so update checks and release notes are unavailable."
        }
        Finding::ServiceAvailable => "The service is available.",
        Finding::PortalMissing => {
            "xdg-desktop-portal isn't available, so Strata can't provide file dialogs to other apps."
        }
        Finding::SecretServiceMissing => {
            "No Secret Service provider, such as GNOME Keyring or KWallet, is available, so passwords for encrypted drives can't be remembered."
        }
        Finding::NoActions => "No custom actions are installed.",
        Finding::ActionsReady => "Every custom action can run.",
        Finding::NotRun => "This check couldn't run.",
        Finding::PrlimitMissing { .. }
        | Finding::Found(_)
        | Finding::Missing(_)
        | Finding::WatchLimitLow { .. }
        | Finding::TerminalMissing
        | Finding::ActionsBroken { .. } => {
            unreachable!("{finding:?} has a formatted message")
        }
    }
}

pub(super) fn detail(check: &Check) -> Option<String> {
    match (&check.finding, &check.detail) {
        (Finding::ConnectionFailed(error), _) => Some(error.message()),
        (_, Some(detail)) => Some(tr(detail)),
        (_, None) => None,
    }
}

pub(super) fn summary_text(results: &HashMap<CheckId, Check>) -> String {
    let count = |status| {
        results
            .values()
            .filter(|check| check.status == status)
            .count() as u64
    };
    rust_i18n::t!(
        "Passed: %{passed} · Warnings: %{warnings} · Problems: %{problems}",
        passed = integer(count(Status::Passed)),
        warnings = integer(count(Status::Warning)),
        problems = integer(count(Status::Problem))
    )
    .into_owned()
}

fn field(label: &str, value: &str) -> String {
    rust_i18n::t!("%{label}: %{value}", label = label, value = value).into_owned()
}

pub(super) fn report(
    environment: &[(String, String)],
    results: &HashMap<CheckId, Check>,
    manager: Option<PackageManager>,
) -> String {
    let mut lines = vec![tr("Strata diagnostics")];
    lines.extend(environment.iter().map(|(label, value)| field(label, value)));
    for (group, ids) in GROUPS {
        lines.push(String::new());
        lines.push(tr(heading(group)));
        for check in ids.iter().filter_map(|id| results.get(id)) {
            append_check_lines(&mut lines, check, manager);
        }
    }
    lines.join("\n")
}

fn append_check_lines(lines: &mut Vec<String>, check: &Check, manager: Option<PackageManager>) {
    let label = format!(
        "[{}] {}",
        tr(status_label(check.status)),
        tr(title(check.id))
    );
    lines.push(field(&label, &message(check)));
    if let Some(detail) = detail(check) {
        lines.push(format!("    {detail}"));
    }
    if check.status != Status::Passed
        && let Some(InstallHint::Command(command)) = install_hint(check.packages, manager)
    {
        lines.push(format!("    {command}"));
    }
}
