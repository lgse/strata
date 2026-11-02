// SPDX-License-Identifier: MIT

//! The dialog that offers a release and installs it in place, or hands a
//! package-managed install to its package manager.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use super::{
    super::{
        blur::BlurBin,
        browser::dismiss_modal_layer,
        controls::{ModalLayout, modal_layout},
        modal::{close_on_escape, modal_layer_with_backdrop},
        preferences::PreferenceManager,
    },
    install::{
        FINALIZING_STATUS, InstallCallbacks, InstallGuard, InstallLauncher, InstallProgress,
        default_install_launcher, start_install,
    },
    managed_update::{aur_update_action_label, launch_managed_update},
    release_notes::{release_date, set_release_note_blocks, set_release_notes_message},
    restart::{restart, restart_application},
    update_check::offer_still_eligible,
};
use crate::{
    assets::icons,
    services::{
        BuildKind, InstallCancel, InstallRequest, InstallSource, ManagedInstall, ReleaseMetadata,
        UpdateMethod,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateDialogPhase {
    Ready,
    Downloading,
    Finalizing,
    Cancelled,
    Failed,
    Installed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UpdateOffer {
    /// The newest release on the channel, found by an update check.
    Available,
    /// A release picked from the release list, possibly older than the installed one.
    Chosen,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "tests drive the dialog through these handles")
)]
pub(super) struct UpdateDialog {
    pub(super) layer: gtk::Box,
    pub(super) cancel: gtk::Button,
    pub(super) close: gtk::Button,
    pub(super) action: gtk::Button,
    pub(super) status: gtk::Label,
    pub(super) escape: gtk::EventControllerKey,
}

pub(crate) fn show_update_dialog(
    parent: &gtk::Window,
    release: &ReleaseMetadata,
    install: InstallRequest,
    install_guard: InstallGuard,
    update_method: UpdateMethod,
) {
    let _dialog = build_update_dialog(
        parent,
        release,
        install,
        install_guard,
        update_method,
        UpdateOffer::Available,
        default_install_launcher(),
    );
}

/// Where the dialog is shown: the window's overlay and the root it blurs.
#[derive(Clone)]
struct DialogHost {
    overlay: gtk::Overlay,
    root: Option<BlurBin>,
}

impl DialogHost {
    fn dismiss(&self, layer: &gtk::Box) {
        dismiss_modal_layer(layer, &self.overlay, self.root.as_ref());
    }
}

/// The dialog widgets an install updates as it progresses.
#[derive(Clone)]
struct DialogView {
    status: gtk::Label,
    progress: gtk::ProgressBar,
    action: gtk::Button,
    cancel: gtk::Button,
    close: gtk::Button,
}

impl DialogView {
    fn set_dismissible(&self, enabled: bool) {
        self.cancel.set_sensitive(enabled);
        self.close.set_sensitive(enabled);
    }
}

/// The dialog's phase and the handles that can change it.
#[derive(Clone)]
struct DialogCore {
    view: DialogView,
    host: DialogHost,
    phase: Rc<Cell<UpdateDialogPhase>>,
    cancel_handle: Rc<RefCell<Option<InstallCancel>>>,
    // Set when the offer this dialog was opened with is no longer eligible on
    // the current channel, which turns the action button into a plain Close.
    withdrawn: Rc<Cell<bool>>,
}

impl DialogCore {
    fn enter_finalizing(&self) {
        self.phase.set(UpdateDialogPhase::Finalizing);
        self.view.progress.set_fraction(1.0);
        self.view
            .status
            .set_text(&crate::i18n::tr(FINALIZING_STATUS));
        self.view.set_dismissible(false);
    }

    fn close_dialog(&self, layer: &gtk::Box) {
        match self.phase.get() {
            UpdateDialogPhase::Finalizing => return,
            UpdateDialogPhase::Downloading => {
                let committed = self
                    .cancel_handle
                    .borrow()
                    .as_ref()
                    .is_some_and(|handle| !handle.cancel());
                if committed {
                    // The installer can commit before its Finalizing event reaches the UI.
                    self.enter_finalizing();
                    return;
                }
                self.cancel_handle.take();
                self.phase.set(UpdateDialogPhase::Cancelled);
            }
            UpdateDialogPhase::Ready
            | UpdateDialogPhase::Cancelled
            | UpdateDialogPhase::Failed
            | UpdateDialogPhase::Installed => {}
        }
        self.host.dismiss(layer);
    }

    fn withdraw(&self) {
        self.withdrawn.set(true);
        self.view.status.set_text(&crate::i18n::tr(
            "This build is no longer offered on your update channel — check for updates again.",
        ));
        self.view.action.set_label(&crate::i18n::tr("Close"));
    }
}

/// The offer the dialog was opened with and what it takes to install it.
#[derive(Clone)]
struct DialogOffer {
    offer: UpdateOffer,
    offered_kind: BuildKind,
    update_method: UpdateMethod,
    install: InstallRequest,
    install_guard: InstallGuard,
    launcher: InstallLauncher,
}

/// The action button's flow, from the offer through the install to restart.
#[derive(Clone)]
struct DialogFlow {
    core: DialogCore,
    layer: gtk::Box,
    offer: DialogOffer,
    aur_action: &'static str,
    application: Option<gtk::Application>,
}

pub(super) fn build_update_dialog(
    parent: &gtk::Window,
    release: &ReleaseMetadata,
    install: InstallRequest,
    install_guard: InstallGuard,
    update_method: UpdateMethod,
    offer: UpdateOffer,
    launcher: InstallLauncher,
) -> Option<UpdateDialog> {
    let window_overlay = parent.child().and_downcast::<gtk::Overlay>()?;
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    if let Some(root) = blurred_root.as_ref() {
        root.set_blurred(true);
    }
    let aur_action = aur_update_action_label();
    let layout = dialog_layout(release, update_method, offer, aur_action);
    let (status, progress) = append_dialog_body(&layout.body, release, update_method, offer);
    let core = DialogCore {
        view: DialogView {
            status,
            progress,
            action: layout.confirm,
            cancel: layout.cancel,
            close: layout.close,
        },
        host: DialogHost {
            overlay: window_overlay.clone(),
            root: blurred_root,
        },
        phase: Rc::new(Cell::new(UpdateDialogPhase::Ready)),
        cancel_handle: Rc::new(RefCell::new(None)),
        withdrawn: Rc::new(Cell::new(false)),
    };
    let layer = modal_layer_with_backdrop(&layout.content, {
        let core = core.clone();
        Rc::new(move |layer: &gtk::Box| core.close_dialog(layer))
    });
    window_overlay.add_overlay(&layer);
    core.view.action.grab_focus();
    let escape = wire_dismissal(&core, &layer);
    let flow = DialogFlow {
        core: core.clone(),
        layer: layer.clone(),
        offer: DialogOffer {
            offer,
            offered_kind: release.kind,
            update_method,
            install,
            install_guard,
            launcher,
        },
        aur_action,
        application: parent.application(),
    };
    flow.withdraw_on_channel_change();
    core.view
        .action
        .connect_clicked(move |button| flow.on_action_clicked(button));
    Some(UpdateDialog {
        layer,
        cancel: core.view.cancel,
        close: core.view.close,
        action: core.view.action,
        status: core.view.status,
        escape,
    })
}

fn dialog_layout(
    release: &ReleaseMetadata,
    update_method: UpdateMethod,
    offer: UpdateOffer,
    aur_action: &str,
) -> ModalLayout {
    let installed = crate::build_info::installed_version();
    let (title, subtitle) = match offer {
        UpdateOffer::Available => (
            rust_i18n::t!("Strata v%{value1} is available", value1 = release.version),
            rust_i18n::t!(
                "Installed v%{value1}  →  Available v%{value2}",
                value1 = installed,
                value2 = release.version
            ),
        ),
        UpdateOffer::Chosen => (
            rust_i18n::t!("Install Strata v%{value1}", value1 = release.version),
            rust_i18n::t!(
                "Installed v%{value1}  →  Selected v%{value2}",
                value1 = installed,
                value2 = release.version
            ),
        ),
    };
    let layout = modal_layout(
        icons::DOWNLOADS,
        &title,
        &subtitle,
        &crate::i18n::tr(match (update_method, offer) {
            (UpdateMethod::InPlace, UpdateOffer::Available) => "Download update",
            (UpdateMethod::InPlace, UpdateOffer::Chosen) => "Install release",
            (UpdateMethod::Aur, _) => aur_action,
            (UpdateMethod::Omarchy, _) => "Open Omarchy Update",
            (UpdateMethod::Pacman, _) => "Close",
        }),
    );
    layout.content.add_css_class("update-dialog");
    layout.content.set_size_request(560, -1);
    layout
}

/// Fills the dialog body and returns its status label and progress bar.
fn append_dialog_body(
    body: &gtk::Box,
    release: &ReleaseMetadata,
    update_method: UpdateMethod,
    offer: UpdateOffer,
) -> (gtk::Label, gtk::ProgressBar) {
    // A prerelease offer must be visibly labelled, and must let the user
    // confirm exactly what they are about to install before doing so: which
    // channel it is, its precise tag, the source commit, and when it was
    // published. A picked release shows its details too, since it may be a rollback.
    if release.kind != BuildKind::Stable {
        let badge = gtk::Label::new(Some(&release.kind.localized_label()));
        badge.add_css_class("prerelease-badge");
        badge.set_xalign(0.0);
        badge.set_halign(gtk::Align::Start);
        body.append(&badge);
    }
    if release.kind != BuildKind::Stable || offer == UpdateOffer::Chosen {
        body.append(&update_dialog_details(release));
    }
    let notes_heading = gtk::Label::new(Some(&crate::i18n::tr("What's new")));
    notes_heading.add_css_class("release-notes-title");
    notes_heading.set_xalign(0.0);
    let notes_scroll = release_notes_scroller(release);
    let fallback =
        gtk::LinkButton::with_label(&release.url, &crate::i18n::tr("View release on GitHub"));
    fallback.set_has_tooltip(false);
    fallback.add_css_class("release-notes-fallback");
    fallback.set_halign(gtk::Align::Start);
    let status = gtk::Label::new(Some(&initial_status_message(update_method, offer)));
    status.add_css_class("update-dialog-status");
    status.set_xalign(0.0);
    status.set_wrap(true);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("update-dialog-progress");
    progress.set_fraction(0.0);
    progress.set_visible(false);
    body.append(&notes_heading);
    body.append(&notes_scroll);
    body.append(&fallback);
    body.append(&status);
    body.append(&progress);
    (status, progress)
}

fn release_notes_scroller(release: &ReleaseMetadata) -> gtk::ScrolledWindow {
    let notes = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if release.note_blocks.is_empty() {
        set_release_notes_message(
            &notes,
            &crate::i18n::tr(
                "No release notes were provided. Review this release on GitHub before continuing.",
            ),
        );
    } else {
        set_release_note_blocks(&notes, &release.note_blocks);
    }
    let notes_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(120)
        .max_content_height(300)
        .propagate_natural_height(true)
        .child(&notes)
        .build();
    notes_scroll.add_css_class("update-dialog-notes");
    notes_scroll
}

fn initial_status_message(update_method: UpdateMethod, offer: UpdateOffer) -> String {
    match update_method {
        UpdateMethod::InPlace => crate::i18n::tr(match offer {
            UpdateOffer::Available => "Review the release notes before downloading the update.",
            UpdateOffer::Chosen => "Review the release notes before installing this release.",
        }),
        UpdateMethod::Aur => InstallSource::detect()
            .managed()
            .map(update_dialog_status)
            .unwrap_or_else(|| {
                crate::i18n::tr("This installation is managed by its package manager.")
            }),
        UpdateMethod::Omarchy => crate::i18n::tr(
            "This installation is managed by Omarchy. Run “omarchy update” to install it.",
        ),
        UpdateMethod::Pacman => crate::i18n::tr(
            "This installation is managed by pacman. Install it through a full system update.",
        ),
    }
}

/// Cancel, the close button, and Escape all run the phase-aware close.
fn wire_dismissal(core: &DialogCore, layer: &gtk::Box) -> gtk::EventControllerKey {
    for button in [&core.view.cancel, &core.view.close] {
        let core = core.clone();
        let layer = layer.clone();
        button.connect_clicked(move |_| core.close_dialog(&layer));
    }
    let core = core.clone();
    let escape_layer = layer.clone();
    close_on_escape(layer, move || core.close_dialog(&escape_layer))
}

impl DialogFlow {
    fn withdraw_on_channel_change(&self) {
        let core = self.core.clone();
        let offered_kind = self.offer.offered_kind;
        PreferenceManager::shared().on_release_channel_changed(
            &self.layer,
            Rc::new(move || {
                if core.phase.get() != UpdateDialogPhase::Ready || core.withdrawn.get() {
                    return;
                }
                if !offer_still_eligible(
                    PreferenceManager::shared().release_channel(),
                    offered_kind,
                ) {
                    core.withdraw();
                }
            }),
        );
    }

    fn dismiss(&self, button: &gtk::Button) {
        self.core.host.dismiss(&self.layer);
        button.set_sensitive(false);
    }

    fn on_action_clicked(&self, button: &gtk::Button) {
        let update_method = self.offer.update_method;
        if update_method == UpdateMethod::Aur && self.aur_action == "Close" {
            self.dismiss(button);
            return;
        }
        if matches!(update_method, UpdateMethod::Aur | UpdateMethod::Omarchy) {
            self.open_managed_update(button);
            return;
        }
        if update_method == UpdateMethod::Pacman {
            self.dismiss(button);
            return;
        }
        match self.core.phase.get() {
            UpdateDialogPhase::Installed => {
                restart(self.application.as_ref());
                button.set_sensitive(false);
                return;
            }
            UpdateDialogPhase::Failed | UpdateDialogPhase::Cancelled => {
                self.dismiss(button);
                return;
            }
            UpdateDialogPhase::Downloading | UpdateDialogPhase::Finalizing => return,
            UpdateDialogPhase::Ready => {}
        }
        if self.core.withdrawn.get() {
            self.dismiss(button);
            return;
        }
        // Read the channel at the click, not when the dialog was opened: this
        // dialog is driven by the sidebar notice, whose cached offer survives
        // a channel switch made anywhere in the process -- including in
        // another window. `withdrawn` rather than a phase change so Cancel and
        // Escape keep dismissing normally.
        if !offer_still_eligible(
            PreferenceManager::shared().release_channel(),
            self.offer.offered_kind,
        ) {
            self.core.withdraw();
            return;
        }
        self.start_download(button);
    }

    fn open_managed_update(&self, button: &gtk::Button) {
        button.set_sensitive(false);
        let (flow, button) = (self.clone(), button.clone());
        glib::MainContext::default().spawn_local(async move {
            match launch_managed_update(flow.offer.update_method).await {
                Ok(_) => flow.core.host.dismiss(&flow.layer),
                Err(error) => {
                    flow.core.view.status.set_text(&error);
                    button.set_sensitive(true);
                }
            }
        });
    }

    fn start_download(&self, button: &gtk::Button) {
        let view = &self.core.view;
        self.core.phase.set(UpdateDialogPhase::Downloading);
        button.set_sensitive(false);
        view.progress.set_visible(true);
        view.status.set_text(&crate::i18n::tr("Starting download…"));
        let outcome = start_install(
            &self.offer.install_guard,
            self.offer.install.clone(),
            &self.offer.launcher,
            self.install_callbacks(button),
        );
        match outcome {
            Ok(handle) => *self.core.cancel_handle.borrow_mut() = Some(handle),
            Err(_request) => {
                self.core.phase.set(UpdateDialogPhase::Ready);
                view.status.set_text(&crate::i18n::tr(
                    "Another install is already running — try again shortly.",
                ));
                view.progress.set_visible(false);
                button.set_sensitive(true);
            }
        }
    }

    fn install_callbacks(&self, button: &gtk::Button) -> InstallCallbacks {
        let offer = self.offer.offer;
        InstallCallbacks {
            on_progress: Box::new({
                let core = self.core.clone();
                move |event| core.apply_progress(event)
            }),
            on_installed: Box::new({
                let (core, action) = (self.core.clone(), button.clone());
                move || {
                    let _taken = core.cancel_handle.borrow_mut().take();
                    core.phase.set(UpdateDialogPhase::Installed);
                    core.view.progress.set_fraction(1.0);
                    core.view.status.set_text(&crate::i18n::tr(match offer {
                        UpdateOffer::Available => "Update installed — restart to apply",
                        UpdateOffer::Chosen => "Release installed — restart to apply",
                    }));
                    action.set_label(&crate::i18n::tr("Restart now"));
                    action.add_css_class("suggested-action");
                    action.set_sensitive(true);
                    // Restart can fail; leave a way to close the dialog.
                    core.view.set_dismissible(true);
                    restart_application(&action);
                }
            }),
            on_cancelled: Box::new({
                let (core, layer) = (self.core.clone(), self.layer.clone());
                move || {
                    let _taken = core.cancel_handle.borrow_mut().take();
                    core.phase.set(UpdateDialogPhase::Cancelled);
                    core.host.dismiss(&layer);
                }
            }),
            on_failed: Box::new({
                let (core, action) = (self.core.clone(), button.clone());
                move |message| {
                    let _taken = core.cancel_handle.borrow_mut().take();
                    core.phase.set(UpdateDialogPhase::Failed);
                    core.view.set_dismissible(true);
                    core.show_install_failure(message);
                    action.set_label(&crate::i18n::tr("Close"));
                    action.set_sensitive(true);
                }
            }),
        }
    }
}

impl DialogCore {
    /// The dialog's progress wording, fuller than the update row's.
    fn apply_progress(&self, event: InstallProgress) {
        let view = &self.view;
        match event {
            InstallProgress::Downloading { downloaded, total } => {
                view.status
                    .set_text(&download_status(&view.progress, downloaded, total));
            }
            InstallProgress::Verifying => {
                view.status.set_text(&crate::i18n::tr("Verifying update…"));
            }
            InstallProgress::Installing => {
                view.progress.set_fraction(1.0);
                view.status.set_text(&crate::i18n::tr("Installing update…"));
            }
            InstallProgress::Finalizing => {
                if self.phase.get() == UpdateDialogPhase::Downloading {
                    self.enter_finalizing();
                }
            }
        }
    }

    fn show_install_failure(&self, message: Option<String>) {
        match message {
            Some(message) => {
                self.view.status.set_text(&rust_i18n::t!(
                    "Couldn't install update: %{message}",
                    message = crate::services::error_detail(message)
                ));
                self.view.progress.add_css_class("error");
            }
            None => self
                .view
                .status
                .set_text(&crate::i18n::tr("Couldn't install update")),
        }
    }
}

/// Advances `progress` for a download event and returns its status text.
fn download_status(progress: &gtk::ProgressBar, downloaded: u64, total: Option<u64>) -> String {
    if let Some(total) = total.filter(|total| *total > 0) {
        let fraction = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
        progress.set_fraction(fraction);
        rust_i18n::t!(
            "Downloading… %{value1}%  (%{value2} of %{value3} MB)",
            value1 = format!("{:.0}", fraction * 100.0),
            value2 = crate::i18n::decimal(downloaded as f64 / 1_048_576.0, 1),
            value3 = crate::i18n::decimal(total as f64 / 1_048_576.0, 1)
        )
        .into_owned()
    } else {
        progress.pulse();
        rust_i18n::t!(
            "Downloading… %{value1} MB",
            value1 = crate::i18n::decimal(downloaded as f64 / 1_048_576.0, 1)
        )
        .into_owned()
    }
}

fn or_unknown(value: Option<String>) -> String {
    value.unwrap_or_else(|| crate::i18n::tr("Unknown"))
}

fn release_detail_rows(release: &ReleaseMetadata) -> [(&'static str, String); 4] {
    [
        ("Channel", release.kind.localized_label()),
        ("Tag", release.tag.clone()),
        ("Commit", or_unknown(release.commit.clone())),
        (
            "Published",
            or_unknown(release.published_at.as_deref().map(release_date)),
        ),
    ]
}

/// Renders `release`'s channel, tag, source commit, and publication date as
/// a small identity block, for the dialog to show above the notes whenever
/// it is offering a prerelease -- the issue requires the user be able to
/// confirm exactly what they are about to install before doing so.
fn update_dialog_details(release: &ReleaseMetadata) -> gtk::Box {
    let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
    details.add_css_class("update-dialog-details");
    for (label, value) in release_detail_rows(release) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("update-dialog-detail-row");
        let label_widget = gtk::Label::new(Some(&crate::i18n::tr(label)));
        label_widget.add_css_class("update-dialog-detail-label");
        label_widget.set_xalign(0.0);
        label_widget.set_hexpand(true);
        let value_widget = gtk::Label::new(Some(&value));
        value_widget.add_css_class("update-dialog-detail-value");
        value_widget.set_xalign(0.0);
        value_widget.set_selectable(true);
        value_widget.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        if label == "Commit" {
            value_widget.add_css_class("monospace");
        }
        row.append(&label_widget);
        row.append(&value_widget);
        details.append(&row);
    }
    details
}

pub(super) fn update_dialog_status(managed: &ManagedInstall) -> String {
    crate::i18n::join_sentences(&managed.ownership_summary(), &managed.update_instruction())
}
