// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::mpsc::{self, Sender, TryRecvError},
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use super::{
    install_command::install_command, page_content, scrollable_page, search, settings_group,
};
use crate::{
    assets::{self, icons},
    services::{
        InstallSource,
        diagnostics::{self, Check, CheckId, Finding, GROUPS, Group, InstallHint, Status},
        package_manager::PackageManager,
    },
    ui::{accessibility, terminal::Terminal},
};

#[cfg(test)]
mod tests;

const RUN_TITLE: &str = "Run diagnostics";
const COPY_FEEDBACK: Duration = Duration::from_millis(1500);

type Spawner = Rc<dyn Fn(Sender<Check>)>;

pub(super) fn diagnostics_page() -> gtk::Widget {
    diagnostics_page_with(Rc::new(diagnostics::spawn))
}

fn diagnostics_page_with(spawn: Spawner) -> gtk::Widget {
    let content = page_content();
    content.add_css_class("settings-diagnostics-page");
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.add_css_class("settings-option");
    search::tag(&row, RUN_TITLE);
    let copy_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy_box.set_hexpand(true);
    let title = gtk::Label::new(Some(&crate::i18n::tr(RUN_TITLE)));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(&crate::i18n::tr(
        "Checks the tools and services that previews, locations, devices and desktop integration rely on. Sample files are rendered in the preview sandbox and GitHub is contacted to test the connection. Nothing is changed or installed.",
    )));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("settings-option-description");
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("settings-integration-actions");
    actions.add_css_class("diagnostics-actions");
    actions.set_halign(gtk::Align::End);
    let copy = gtk::Button::with_label(&crate::i18n::tr("Copy report"));
    copy.add_css_class("action-dialog-cancel");
    copy.set_widget_name("diagnostics-copy");
    copy.set_sensitive(false);
    let run = gtk::Button::with_label(&crate::i18n::tr("Run"));
    run.add_css_class("action-dialog-confirm");
    run.set_widget_name("diagnostics-run");
    actions.append(&copy);
    actions.append(&run);
    copy_box.append(&title);
    copy_box.append(&description);
    copy_box.append(&actions);
    row.append(&copy_box);
    settings_group(&content, "").append(&row);

    // Search shows or hides this container, so it stays visible and starts empty.
    let output = gtk::Box::new(gtk::Orientation::Vertical, 0);
    search::tag(&output, RUN_TITLE);
    let summary = gtk::Label::new(None);
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    summary.add_css_class("diagnostics-summary");
    summary.set_visible(false);
    output.append(&summary);
    let groups = gtk::Box::new(gtk::Orientation::Vertical, 0);
    output.append(&groups);
    content.append(&output);

    let page = Rc::new(Page {
        run: run.downgrade(),
        copy: copy.clone(),
        summary,
        groups,
        rows: RefCell::default(),
        results: Rc::default(),
        environment: Rc::default(),
        running: Cell::new(false),
        pending_parts: Cell::new(0),
        manager: Cell::new(None),
        spawn,
    });
    let weak = Rc::downgrade(&page);
    copy.connect_clicked(move |button| {
        if let Some(page) = weak.upgrade() {
            copy_report(
                button,
                &page.environment.borrow(),
                &page.results.borrow(),
                page.manager.get(),
            );
        }
    });
    run.connect_clicked(move |_| page.start());
    scrollable_page(&content, None)
}

struct Row {
    icon: gtk::Stack,
    message: gtk::Label,
    detail: gtk::Label,
    fix: gtk::Box,
}

struct Page {
    run: glib::WeakRef<gtk::Button>,
    copy: gtk::Button,
    summary: gtk::Label,
    groups: gtk::Box,
    rows: RefCell<HashMap<CheckId, Row>>,
    results: Rc<RefCell<Vec<Check>>>,
    environment: Rc<RefCell<Vec<(String, String)>>>,
    running: Cell<bool>,
    pending_parts: Cell<u8>,
    manager: Cell<Option<PackageManager>>,
    spawn: Spawner,
}

impl Page {
    fn start(self: &Rc<Self>) {
        if self.running.replace(true) {
            return;
        }
        if let Some(run) = self.run.upgrade() {
            run.set_sensitive(false);
        }
        self.copy.set_sensitive(false);
        self.results.borrow_mut().clear();
        self.summary.set_text(&crate::i18n::tr("Running checks…"));
        self.summary.set_visible(true);
        self.build_rows();

        let (sender, receiver) = mpsc::channel();
        (self.spawn)(sender);
        self.pending_parts.set(2);
        let page = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let detected = gio::spawn_blocking(PackageManager::detect).await;
            let Some(page) = page.upgrade() else {
                return;
            };
            let manager = detected.ok().flatten();
            page.manager.set(manager);
            let poll = Rc::downgrade(&page);
            glib::timeout_add_local(Duration::from_millis(50), move || {
                let Some(page) = poll.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                loop {
                    match receiver.try_recv() {
                        Ok(check) => page.apply(check, manager),
                        Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        Err(TryRecvError::Disconnected) => {
                            page.finish_part(manager);
                            return glib::ControlFlow::Break;
                        }
                    }
                }
            });
            for check in main_thread_checks().await {
                page.apply(check, manager);
            }
            page.finish_part(manager);
        });
    }

    fn finish_part(&self, manager: Option<PackageManager>) {
        let remaining = self.pending_parts.get().saturating_sub(1);
        self.pending_parts.set(remaining);
        if remaining == 0 {
            self.finish(manager);
        }
    }

    fn build_rows(&self) {
        while let Some(child) = self.groups.first_child() {
            self.groups.remove(&child);
        }
        let environment = environment(&self.groups);
        let system = settings_group(&self.groups, "SYSTEM");
        system.add_css_class("diagnostics-system");
        for (label, value) in &environment {
            super::about::append_detail(&system, label, value);
        }
        self.environment.replace(environment);
        let mut rows = self.rows.borrow_mut();
        rows.clear();
        for (group, ids) in GROUPS {
            let list = settings_group(&self.groups, heading(group));
            for id in ids {
                let (widget, row) = pending_row(*id);
                list.append(&widget);
                rows.insert(*id, row);
            }
        }
    }

    fn apply(&self, check: Check, manager: Option<PackageManager>) {
        if let Some(row) = self.rows.borrow().get(&check.id) {
            show(row, &check, manager);
        }
        let mut results = self.results.borrow_mut();
        results.retain(|result| result.id != check.id);
        results.push(check);
    }

    fn finish(&self, manager: Option<PackageManager>) {
        let missing = GROUPS
            .iter()
            .flat_map(|(_, ids)| ids.iter().copied())
            .filter(|id| !self.results.borrow().iter().any(|check| check.id == *id))
            .collect::<Vec<_>>();
        for id in missing {
            self.apply(Check::not_run(id), manager);
        }
        let count = |status| {
            self.results
                .borrow()
                .iter()
                .filter(|check| check.status == status)
                .count()
        };
        self.summary.set_text(&rust_i18n::t!(
            "Passed: %{passed} · Warnings: %{warnings} · Problems: %{problems}",
            passed = crate::i18n::integer(count(Status::Passed) as u64),
            warnings = crate::i18n::integer(count(Status::Warning) as u64),
            problems = crate::i18n::integer(count(Status::Problem) as u64)
        ));
        self.running.set(false);
        if let Some(run) = self.run.upgrade() {
            run.set_sensitive(true);
        }
        self.copy.set_sensitive(true);
    }
}

async fn main_thread_checks() -> [Check; 4] {
    let terminal = gio::spawn_blocking(|| {
        Terminal::resolve().map(|terminal| terminal.program().to_string_lossy().into_owned())
    });
    let catalog = crate::ui::actions::shared().reload().await;
    let unavailable = catalog
        .actions()
        .iter()
        .filter(|action| action.definition.enabled && !action.is_available())
        .map(|action| action.name().to_owned())
        .collect();
    let failed = catalog
        .failures()
        .iter()
        .map(|failure| failure.directory.clone())
        .collect();
    [
        diagnostics::syntax_highlighting(),
        diagnostics::folder_monitoring(),
        diagnostics::terminal(terminal.await.unwrap_or_default()),
        diagnostics::custom_actions(catalog.actions().len(), unavailable, failed),
    ]
}

fn pending_row(id: CheckId) -> (gtk::Box, Row) {
    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    widget.add_css_class("diagnostics-row");
    let icon = gtk::Stack::new();
    icon.set_valign(gtk::Align::Start);
    icon.add_css_class("diagnostics-status");
    let spinner = gtk::Spinner::new();
    spinner.start();
    icon.add_named(&spinner, Some("pending"));
    for (name, image) in [
        ("passed", assets::primary_icon(icons::CIRCLE_CHECK, 18)),
        ("warning", assets::text_icon(icons::TRIANGLE_ALERT, 18)),
        ("problem", assets::danger_icon(icons::CIRCLE_X, 18)),
    ] {
        icon.add_named(&image, Some(name));
    }
    icon.set_visible_child_name("pending");
    widget.append(&icon);

    let copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
    copy.set_hexpand(true);
    let title = gtk::Label::new(Some(&crate::i18n::tr(title(id))));
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.add_css_class("settings-option-title");
    copy.append(&title);
    let message = gtk::Label::new(Some(&crate::i18n::tr("Checking…")));
    message.set_xalign(0.0);
    message.set_wrap(true);
    message.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    message.set_selectable(true);
    message.add_css_class("settings-option-description");
    message.add_css_class("diagnostics-message");
    crate::ui::controls::keep_words_whole(&message);
    copy.append(&message);
    let detail = gtk::Label::new(None);
    detail.set_xalign(0.0);
    detail.set_wrap(true);
    detail.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    detail.set_selectable(true);
    detail.add_css_class("diagnostics-detail");
    detail.set_visible(false);
    copy.append(&detail);
    let fix = gtk::Box::new(gtk::Orientation::Vertical, 6);
    fix.add_css_class("diagnostics-fix");
    fix.set_visible(false);
    copy.append(&fix);
    widget.append(&copy);
    (
        widget,
        Row {
            icon,
            message,
            detail,
            fix,
        },
    )
}

fn show(row: &Row, check: &Check, manager: Option<PackageManager>) {
    let state = match check.status {
        Status::Passed => "passed",
        Status::Warning => "warning",
        Status::Problem => "problem",
    };
    row.icon.set_visible_child_name(state);
    accessibility::set_label(&row.icon, &crate::i18n::tr(status_label(check.status)));
    for class in ["passed", "warning", "problem"] {
        row.message.remove_css_class(class);
    }
    row.message.add_css_class(state);
    row.message.set_text(&message(check));
    let detail = detail(check);
    row.detail.set_text(detail.as_deref().unwrap_or_default());
    row.detail.set_visible(detail.is_some());
    while let Some(child) = row.fix.first_child() {
        row.fix.remove(&child);
    }
    let hint = (check.status != Status::Passed)
        .then(|| diagnostics::install_hint(check.packages, manager))
        .flatten();
    row.fix.set_visible(hint.is_some());
    match hint {
        Some(InstallHint::Command(command)) => {
            row.fix
                .append(&fix_label(&crate::i18n::tr("Install with:")));
            row.fix.append(&install_command(
                &command,
                &crate::i18n::tr(
                    "The command opened in your terminal. Run diagnostics again when it finishes.",
                ),
            ));
        }
        Some(InstallHint::Package(package)) => {
            row.fix.append(&fix_label(&rust_i18n::t!(
                "Install %{package} or your distribution's equivalent.",
                package = package
            )));
        }
        None => {}
    }
}

fn fix_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.add_css_class("settings-option-description");
    label
}

fn heading(group: Group) -> &'static str {
    match group {
        Group::Sandbox => "PREVIEW SANDBOX",
        Group::Previews => "PREVIEWS",
        Group::Locations => "LOCATIONS AND DEVICES",
        Group::Desktop => "DESKTOP AND NETWORK",
    }
}

fn title(id: CheckId) -> &'static str {
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

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Passed => "Passed",
        Status::Warning => "Warning",
        Status::Problem => "Problem",
    }
}

fn message(check: &Check) -> String {
    let tr = crate::i18n::tr;
    let items = |names: &[String]| crate::i18n::list(names.iter().cloned());
    match &check.finding {
        Finding::SandboxWorks => tr("A sample image rendered in the sandbox."),
        Finding::BubblewrapMissing => {
            tr("Bubblewrap (bwrap) isn't installed, so no thumbnails or previews can be shown.")
        }
        Finding::PrlimitMissing { path } => rust_i18n::t!(
            "prlimit wasn't found at %{path}, so no thumbnails or previews can be shown.",
            path = path
        )
        .into_owned(),
        Finding::SandboxFailed => tr(
            "The sandbox couldn't start. Unprivileged user namespaces may be disabled or restricted, for example by AppArmor.",
        ),
        Finding::WorkersWork => tr("Thumbnails reuse sandboxed worker processes."),
        Finding::LandlockUnavailable => tr(
            "The kernel doesn't support Landlock ABI 3, so each thumbnail starts its own sandbox and loads more slowly.",
        ),
        Finding::WorkersFailed => tr("A sandboxed worker couldn't render a sample image."),
        Finding::SampleProcessed => tr("A sample file was processed correctly."),
        Finding::SampleFailed => tr("A sample file couldn't be processed."),
        Finding::SlowVideoThumbnails => tr(
            "ffmpegthumbnailer isn't installed, so video thumbnails use FFmpeg and load more slowly.",
        ),
        Finding::Found(names) => {
            rust_i18n::t!("Found: %{items}", items = items(names)).into_owned()
        }
        Finding::Missing(names) => {
            rust_i18n::t!("Not installed: %{items}", items = items(names)).into_owned()
        }
        Finding::AudioOutput { sink } => {
            rust_i18n::t!("Audio plays through %{sink}.", sink = sink).into_owned()
        }
        Finding::NoAudioSink => tr("No GStreamer audio output plugin was found."),
        Finding::GstreamerFailed => tr("GStreamer couldn't start."),
        Finding::NoRawConverter => {
            tr("No RAW converter was found, so RAW photos show only their embedded preview.")
        }
        Finding::LanguagesInstalled => tr("Language definitions are installed."),
        Finding::LanguagesMissing => {
            tr("No language definitions were found, so code previews show plain text.")
        }
        Finding::MimeDatabaseWorks => tr("File types are recognized."),
        Finding::MimeDatabaseMissing => {
            tr("The shared MIME database is missing, so files can't be recognized by type.")
        }
        Finding::RarIncluded => tr("This build includes RAR support."),
        Finding::RarExcluded => tr("This build doesn't include RAR support."),
        Finding::GvfsRunning => tr("GVfs is running."),
        Finding::GvfsBypassed => tr(
            "This session uses local files only because GVfs didn't respond when Strata started. Trash, network locations and devices are unavailable until Strata restarts with GVfs working.",
        ),
        Finding::GvfsMissing => {
            tr("GVfs isn't running, so trash, network locations and devices are unavailable.")
        }
        Finding::BackendInstalled => tr("The GVfs backend is installed."),
        Finding::BackendMissing => tr("The GVfs backend isn't installed."),
        Finding::RequiresGvfs => tr("Unavailable while GVfs isn't running."),
        Finding::DrivesReady => tr("UDisks2 and the GVfs volume monitor are available."),
        Finding::UdisksMissing => {
            tr("UDisks2 isn't available, so drives can't be mounted, formatted or unlocked.")
        }
        Finding::VolumeMonitorMissing => tr(
            "The GVfs UDisks2 volume monitor isn't installed, so drives may not appear in the sidebar.",
        ),
        Finding::SessionBusUnavailable => tr("The D-Bus session bus isn't available."),
        Finding::MonitoringWorks => tr("Folder changes are detected automatically."),
        Finding::WatchLimitLow { limit } => rust_i18n::t!(
            "The inotify watch limit is %{limit}, so some folders may not refresh automatically.",
            limit = crate::i18n::integer(*limit)
        )
        .into_owned(),
        Finding::MonitoringFailed => {
            tr("Folder changes can't be detected, so folders won't refresh automatically.")
        }
        Finding::Connected => tr("Connected to GitHub."),
        Finding::ConnectionFailed(_) => {
            tr("Couldn't connect to GitHub, so update checks and release notes are unavailable.")
        }
        Finding::TerminalMissing => crate::ui::terminal::no_terminal_message(),
        Finding::ServiceAvailable => tr("The service is available."),
        Finding::PortalMissing => tr(
            "xdg-desktop-portal isn't available, so Strata can't provide file dialogs to other apps.",
        ),
        Finding::SecretServiceMissing => tr(
            "No Secret Service provider, such as GNOME Keyring or KWallet, is available, so passwords for encrypted drives can't be remembered.",
        ),
        Finding::NoActions => tr("No custom actions are installed."),
        Finding::ActionsReady => tr("Every custom action can run."),
        Finding::ActionsBroken {
            unavailable,
            failed,
        } => {
            let mut lines = Vec::new();
            if !unavailable.is_empty() {
                lines.push(rust_i18n::t!(
                    "Can't run: %{items}",
                    items = items(unavailable)
                ));
            }
            if !failed.is_empty() {
                lines.push(rust_i18n::t!(
                    "Couldn't load: %{items}",
                    items = items(failed)
                ));
            }
            lines.join("\n")
        }
        Finding::NotRun => tr("This check couldn't run."),
    }
}

fn detail(check: &Check) -> Option<String> {
    match (&check.finding, &check.detail) {
        (Finding::ConnectionFailed(error), _) => Some(error.message()),
        (_, Some(detail)) => Some(crate::i18n::tr(detail)),
        (_, None) => None,
    }
}

fn copy_report(
    button: &gtk::Button,
    environment: &[(String, String)],
    results: &[Check],
    manager: Option<PackageManager>,
) {
    button
        .clipboard()
        .set_text(&report(environment, results, manager));
    button.set_label(&crate::i18n::tr("Copied"));
    let button = button.downgrade();
    glib::timeout_add_local_once(COPY_FEEDBACK, move || {
        if let Some(button) = button.upgrade() {
            button.set_label(&crate::i18n::tr("Copy report"));
        }
    });
}

fn environment(anchor: &impl IsA<gtk::Widget>) -> Vec<(String, String)> {
    let tr = crate::i18n::tr;
    let installation = match InstallSource::detect().managed() {
        Some(managed) => managed.ownership_summary(),
        None if !crate::build_info::updater_enabled() => {
            tr("In-app updates are disabled in this build.")
        }
        None => tr("Self-managed"),
    };
    let kernel = ["ostype", "osrelease"]
        .map(|name| {
            std::fs::read_to_string(format!("/proc/sys/kernel/{name}"))
                .map(|value| value.trim().to_owned())
                .unwrap_or_default()
        })
        .join(" ");
    let display = anchor.display();
    let display_server = if display.is::<gdk4_wayland::WaylandDisplay>() {
        "Wayland".to_owned()
    } else if display.is::<gdk4_x11::X11Display>() {
        "X11".to_owned()
    } else {
        display.type_().name().to_owned()
    };
    let renderer = anchor
        .native()
        .and_then(|native| native.renderer())
        .map_or_else(
            || tr("Unknown"),
            |renderer| renderer.type_().name().to_owned(),
        );
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .ok()
        .filter(|desktop| !desktop.is_empty())
        .unwrap_or_else(|| tr("Unknown"));
    vec![
        (tr("Version"), crate::build_info::displayed_version()),
        (tr("Commit"), crate::build_info::COMMIT.to_owned()),
        (tr("Installation"), installation),
        (tr("Toolkit"), super::about::toolkit_version()),
        (
            "GStreamer".to_owned(),
            gstreamer::version_string().to_string(),
        ),
        (tr("Kernel"), kernel.trim().to_owned()),
        (tr("Desktop environment"), desktop),
        (tr("Display server"), display_server),
        (tr("Interface renderer"), renderer),
        (tr("Language"), rust_i18n::locale().to_string()),
    ]
}

fn field(label: &str, value: &str) -> String {
    rust_i18n::t!("%{label}: %{value}", label = label, value = value).into_owned()
}

fn report(
    environment: &[(String, String)],
    results: &[Check],
    manager: Option<PackageManager>,
) -> String {
    let tr = crate::i18n::tr;
    let mut lines = vec![tr("Strata diagnostics")];
    lines.extend(environment.iter().map(|(label, value)| field(label, value)));
    for (group, ids) in GROUPS {
        lines.push(String::new());
        lines.push(tr(heading(group)));
        for check in ids
            .iter()
            .filter_map(|id| results.iter().find(|check| check.id == *id))
        {
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
                && let Some(InstallHint::Command(command)) =
                    diagnostics::install_hint(check.packages, manager)
            {
                lines.push(format!("    {command}"));
            }
        }
    }
    lines.join("\n")
}
