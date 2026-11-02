// SPDX-License-Identifier: MIT

//! The Updates page: the check-for-updates row with its install flow, the
//! release channel and automatic check preferences, and release notes.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{glib, prelude::*};

use super::{
    super::preferences::PreferenceManager,
    append_heading,
    channel::{append_channel_option, automatic_updates_option, managed_install_row},
    install::{
        FINALIZING_STATUS, InstallCallbacks, InstallGuard, InstallLauncher, InstallProgress,
        apply_install_progress, default_install_launcher, start_install,
    },
    managed_update::launch_managed_update,
    page_content,
    release_notes::{
        ReleaseNotesCard, ReleaseNotesKind, load_current_release_notes, release_notes_card,
    },
    release_picker,
    restart::restart_application,
    scrollable_page, search, settings_group,
    update_check::{
        UpdateNoticeHandler, abandon_row_check, complete_row_check, effective_update_channel,
        is_stale_check, is_superseded, next_check_generation, offer_still_eligible,
        publish_update_notice,
    },
    update_messages::installed_version_status,
    update_status,
};
use crate::{
    assets::icons,
    services::{
        self, BuildKind, Channel, InstallCancel, InstallRequest, InstallSource, UpdateCheck,
        UpdateMethod,
    },
};

pub(super) fn updates_page(
    manager: Rc<PreferenceManager>,
    update_notice: UpdateNoticeHandler,
    install_guard: InstallGuard,
    update_method: UpdateMethod,
) -> (gtk::Widget, Vec<(gtk::Box, gtk::Button)>) {
    let preferences = page_content();
    preferences.add_css_class("settings-updates-page");
    append_heading(&preferences, "STATUS");
    let managed = InstallSource::detect().managed();
    if let Some(managed) = managed {
        preferences.append(&managed_install_row(managed));
    }

    let available_notes = release_notes_card(
        ReleaseNotesKind::Available,
        &crate::i18n::tr("Available release"),
        &crate::i18n::tr("Check for updates to see the latest release notes."),
    );
    let UpdateCheckRow {
        row: update_row,
        run_check,
        responsive_action,
        install_underway,
        ..
    } = update_check_row(
        manager.clone(),
        available_notes.clone(),
        install_guard.clone(),
        update_method,
    );

    preferences.append(&update_row);
    let options = settings_group(&preferences, "PREFERENCES");
    options.append(&automatic_updates_option(&manager, update_method));
    let channel_row = append_channel_option(&options, manager.clone(), managed, update_method);
    if update_method == UpdateMethod::InPlace {
        options.append(
            &release_picker::release_picker(
                &manager,
                install_guard,
                release_picker::ReleaseSource::default(),
            )
            .row,
        );
    }
    append_current_release_notes(&preferences);
    bind_channel_row_to_auto_check(&manager, &channel_row);
    bind_auto_check_toggle(&manager, &preferences, run_check.clone(), update_notice);

    let page = scrollable_page(&preferences, None);
    wire_channel_change_check(&manager, &page, run_check, install_underway);
    (page, vec![responsive_action])
}

fn append_current_release_notes(preferences: &gtk::Box) {
    append_heading(preferences, "RELEASE NOTES");
    let current_notes = release_notes_card(
        ReleaseNotesKind::Current,
        &rust_i18n::t!(
            "What's new in v%{value1}",
            value1 = crate::build_info::installed_version()
        ),
        &crate::i18n::tr("Loading release notes…"),
    );
    current_notes.container.remove(&current_notes.title);
    current_notes.container.remove(&current_notes.summary);
    let header = gtk::Box::new(gtk::Orientation::Vertical, 6);
    header.append(&current_notes.title);
    header.append(&current_notes.summary);
    header.set_hexpand(true);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    heading.append(&header);
    heading.append(&crate::assets::primary_icon(icons::CHEVRON_RIGHT, 16));
    let toggle = gtk::Button::builder().child(&heading).build();
    toggle.add_css_class("release-notes-toggle");
    let details = gtk::Revealer::builder()
        .child(&current_notes.container)
        .transition_duration(0)
        .reveal_child(false)
        .build();
    let expander = gtk::Box::new(gtk::Orientation::Vertical, 0);
    expander.add_css_class("settings-release-expander");
    search::tag(&expander, "Release notes");
    expander.append(&toggle);
    expander.append(&details);
    toggle.update_state(&[gtk::accessible::State::Expanded(Some(false))]);
    toggle.connect_clicked(move |button| {
        let expanded = !details.reveals_child();
        details.set_reveal_child(expanded);
        button.update_state(&[gtk::accessible::State::Expanded(Some(expanded))]);
        if expanded {
            button.add_css_class("expanded");
        } else {
            button.remove_css_class("expanded");
        }
    });
    preferences.append(&expander);
    load_current_release_notes(&current_notes);
}

fn bind_channel_row_to_auto_check(manager: &Rc<PreferenceManager>, channel_row: &gtk::Box) {
    manager.bind_preference(
        channel_row,
        PreferenceManager::checks_for_updates,
        |widget, enabled| widget.set_sensitive(enabled),
    );
}

/// Toggling automatic checks on runs a check at once; off withdraws the sidebar notice.
fn bind_auto_check_toggle(
    manager: &Rc<PreferenceManager>,
    preferences: &gtk::Box,
    run_check: Rc<dyn Fn(bool)>,
    update_notice: UpdateNoticeHandler,
) {
    let initial = Cell::new(true);
    manager.bind_preference(
        preferences,
        PreferenceManager::checks_for_updates,
        move |_, enabled| {
            if initial.replace(false) {
                return;
            }
            if enabled {
                run_check(false);
            } else {
                update_notice(None);
            }
        },
    );
}

fn wire_channel_change_check(
    manager: &Rc<PreferenceManager>,
    page: &impl IsA<gtk::Widget>,
    run_check: Rc<dyn Fn(bool)>,
    install_underway: Rc<dyn Fn() -> bool>,
) {
    // No automatic check here: the due scheduler owns background checks process-wide.
    manager.on_release_channel_changed(
        page,
        Rc::new(move || {
            if !install_underway() {
                run_check(false);
            }
        }),
    );
}

pub(super) struct PendingInstall {
    pub(super) kind: BuildKind,
    pub(super) returns_to_stable: bool,
    pub(super) request: InstallRequest,
}

pub(super) struct UpdateCheckRow {
    pub(super) row: gtk::Box,
    pub(super) run_check: Rc<dyn Fn(bool)>,
    pub(super) responsive_action: (gtk::Box, gtk::Button),
    pub(super) install_underway: Rc<dyn Fn() -> bool>,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "tests read the row's status text")
    )]
    pub(super) status: gtk::Label,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "tests seed and inspect the cached install offer")
    )]
    pub(super) pending_download: Rc<RefCell<Option<PendingInstall>>>,
}

/// The widgets a check or install flow updates on the row.
#[derive(Clone)]
struct UpdateRowView {
    title: gtk::Label,
    status: gtk::Label,
    progress: gtk::ProgressBar,
    button: gtk::Button,
    notes: ReleaseNotesCard,
}

#[derive(Clone)]
struct UpdateRowState {
    checking: Rc<Cell<bool>>,
    row_generation: Rc<Cell<u64>>,
    // Set once a check finds an update this platform can install; consumed by the
    // button's next click instead of re-running a check.
    pending_download: Rc<RefCell<Option<PendingInstall>>>,
    // Set once an install finishes, so the next click restarts instead of re-checking.
    installed: Rc<Cell<bool>>,
    installing: Rc<Cell<bool>>,
    cancel_handle: Rc<RefCell<Option<InstallCancel>>>,
    managed_update_available: Rc<Cell<bool>>,
}

impl UpdateRowState {
    fn new() -> Self {
        Self {
            checking: Rc::new(Cell::new(false)),
            row_generation: Rc::new(Cell::new(0)),
            pending_download: Rc::new(RefCell::new(None)),
            installed: Rc::new(Cell::new(false)),
            installing: Rc::new(Cell::new(false)),
            cancel_handle: Rc::new(RefCell::new(None)),
            managed_update_available: Rc::new(Cell::new(false)),
        }
    }

    fn install_underway(&self) -> Rc<dyn Fn() -> bool> {
        let installed = self.installed.clone();
        let installing = self.installing.clone();
        Rc::new(move || installed.get() || installing.get())
    }
}

/// Everything the row's button needs to run a check or an install.
#[derive(Clone)]
struct UpdateRow {
    view: UpdateRowView,
    state: UpdateRowState,
    manager: Rc<PreferenceManager>,
    update_method: UpdateMethod,
    install_guard: InstallGuard,
    launcher: InstallLauncher,
}

fn update_check_row(
    manager: Rc<PreferenceManager>,
    available_notes: ReleaseNotesCard,
    install_guard: InstallGuard,
    update_method: UpdateMethod,
) -> UpdateCheckRow {
    update_check_row_with(
        manager,
        available_notes,
        install_guard,
        update_method,
        default_install_launcher(),
    )
}

pub(super) fn update_check_row_with(
    manager: Rc<PreferenceManager>,
    available_notes: ReleaseNotesCard,
    install_guard: InstallGuard,
    update_method: UpdateMethod,
    launcher: InstallLauncher,
) -> UpdateCheckRow {
    let (row, summary, status_icon, view) = build_update_row(available_notes, update_method);
    let state = UpdateRowState::new();
    let install_underway = state.install_underway();
    update_status::View {
        title: view.title.clone(),
        icon: status_icon,
        status: view.status.clone(),
        button: view.button.clone(),
        notes: view.notes.clone(),
        pending: state.pending_download.clone(),
        managed: state.managed_update_available.clone(),
        checking: state.checking.clone(),
        install_underway: install_underway.clone(),
        manager: manager.clone(),
        method: update_method,
    }
    .bind(&row);
    let update_row = UpdateRow {
        view: view.clone(),
        state: state.clone(),
        manager,
        update_method,
        install_guard,
        launcher,
    };
    let run_check: Rc<dyn Fn(bool)> = Rc::new({
        let update_row = update_row.clone();
        move |force: bool| update_row.start_check(force)
    });
    view.button
        .connect_clicked(move |button| update_row.on_button_clicked(button));
    UpdateCheckRow {
        row,
        run_check,
        responsive_action: (summary, view.button),
        install_underway,
        status: view.status,
        pending_download: state.pending_download,
    }
}

fn build_update_row(
    available_notes: ReleaseNotesCard,
    update_method: UpdateMethod,
) -> (gtk::Box, gtk::Box, gtk::Image, UpdateRowView) {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    row.add_css_class("settings-option");
    row.add_css_class("settings-update-status");
    search::tag(&row, "Check for updates");
    let summary = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    summary.add_css_class("settings-update-summary");
    summary.set_vexpand(true);
    row.set_vexpand(false);
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(&crate::i18n::tr("Check for updates")));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let status = gtk::Label::new(Some(&installed_version_status(
        &crate::build_info::installed_version(),
        crate::build_info::build_kind(),
        update_method,
    )));
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.set_use_markup(true);
    status.add_css_class("settings-option-description");
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let status_icon = crate::assets::primary_icon(icons::INFO, 18);
    status_icon.set_valign(gtk::Align::Center);
    title.set_valign(gtk::Align::Center);
    heading.append(&status_icon);
    heading.append(&title);
    copy.append(&heading);
    copy.append(&status);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("settings-update-progress");
    progress.set_hexpand(true);
    progress.set_visible(false);
    copy.append(&progress);
    let button = gtk::Button::with_label(&crate::i18n::tr("Check now"));
    button.add_css_class("settings-update-check");
    button.set_valign(gtk::Align::Center);
    summary.append(&copy);
    summary.append(&button);
    row.append(&summary);
    available_notes.container.add_css_class("inline");
    available_notes.container.set_visible(false);
    row.append(&available_notes.container);
    let view = UpdateRowView {
        title,
        status,
        progress,
        button,
        notes: available_notes,
    };
    (row, summary, status_icon, view)
}

impl UpdateRow {
    /// Always starts a fresh check rather than dropping it: a channel toggle must
    /// never be silently ignored just because a previous check (for the old
    /// channel) is still in flight. The stale check's own result is discarded by
    /// [`RowCheck::poll`] instead, once its generation no longer matches.
    fn start_check(&self, force: bool) {
        let (view, state) = (&self.view, &self.state);
        let update_method = self.update_method;
        let generation = next_check_generation();
        let row_generation = state.row_generation.get().saturating_add(1);
        state.row_generation.set(row_generation);
        state.checking.set(true);
        *state.pending_download.borrow_mut() = None;
        state.installed.set(false);
        state.managed_update_available.set(false);
        view.button.set_label(&crate::i18n::tr("Check now"));
        view.progress.set_fraction(0.0);
        view.progress.set_visible(false);
        view.progress.remove_css_class("error");
        view.title
            .set_text(&crate::i18n::tr("Checking for updates…"));
        view.status
            .set_text(&crate::i18n::tr("Checking for updates…"));
        view.notes.container.set_visible(false);
        view.notes.fallback.set_visible(false);
        // Clear any previously offered release immediately, not only once
        // this check's own result lands: otherwise the sidebar keeps
        // showing a (possibly prerelease) offer from before the channel
        // was switched for the whole duration of this check. Other
        // Settings rows keep their offer until that result arrives.
        update_status::forget();
        publish_update_notice(None);
        view.button.set_sensitive(false);
        // Read the channel now, not once when the row was built: a
        // mid-session channel toggle must be reflected by the very next
        // check, including this one if it was triggered by that toggle.
        let channel = effective_update_channel(self.manager.release_channel(), update_method);
        let receiver = services::check_for_updates(
            channel,
            crate::build_info::installed_version(),
            update_method,
            force,
        );
        let check = RowCheck {
            generation,
            row_generation,
            state: state.clone(),
            button: view.button.clone(),
            channel,
            update_method,
        };
        glib::timeout_add_local(Duration::from_millis(100), move || check.poll(&receiver));
    }
}

/// A check this row started, polled until its result lands or a newer check
/// supersedes it.
struct RowCheck {
    generation: u64,
    row_generation: u64,
    state: UpdateRowState,
    button: gtk::Button,
    channel: Channel,
    update_method: UpdateMethod,
}

impl RowCheck {
    fn poll(&self, receiver: &std::sync::mpsc::Receiver<UpdateCheck>) -> glib::ControlFlow {
        if is_superseded(self.generation) {
            // Only a newer check on this row owns its disabled button.
            if !is_stale_check(self.row_generation, self.state.row_generation.get()) {
                self.button.set_sensitive(true);
                self.state.checking.set(false);
            }
            return glib::ControlFlow::Break;
        }
        match receiver.try_recv() {
            Ok(result) => complete_row_check(self.channel, self.update_method, result),
            Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                abandon_row_check(self.channel, self.update_method);
            }
        }
        glib::ControlFlow::Break
    }
}

impl UpdateRow {
    fn on_button_clicked(&self, button: &gtk::Button) {
        if self.managed_update_offered() {
            self.open_managed_update();
            return;
        }
        if self.state.installed.get() {
            restart_application(button);
            return;
        }
        if self.cancel_running_install() {
            return;
        }
        // Both branches can reborrow this cell; an `if let` scrutinee would retain the RefMut.
        let pending = self.state.pending_download.take();
        match pending {
            Some(pending) => self.install_offer(pending),
            None => self.start_check(true),
        }
    }

    fn managed_update_offered(&self) -> bool {
        matches!(
            self.update_method,
            UpdateMethod::Aur | UpdateMethod::Omarchy
        ) && self.state.managed_update_available.get()
    }

    fn open_managed_update(&self) {
        let status = self.view.status.clone();
        let update_method = self.update_method;
        glib::MainContext::default().spawn_local(async move {
            status.set_text(
                &launch_managed_update(update_method)
                    .await
                    .unwrap_or_else(|error| error),
            );
        });
    }

    /// Cancels the install in flight, if any, reporting whether there was one.
    fn cancel_running_install(&self) -> bool {
        let Some(cancel) = self.state.cancel_handle.borrow_mut().take() else {
            return false;
        };
        self.view
            .status
            .set_text(&crate::i18n::tr(if cancel.cancel() {
                "Cancelling…"
            } else {
                FINALIZING_STATUS
            }));
        self.view.button.set_sensitive(false);
        true
    }

    fn install_offer(&self, pending: PendingInstall) {
        if !offer_still_eligible(self.manager.release_channel(), pending.kind) {
            // The channel was switched back to Stable -- possibly from
            // another window, which this row never hears about -- after
            // this offer was cached. Drop it and re-check rather than
            // installing a prerelease the user has since opted out of;
            // `run_check` clears the sidebar notice and relabels the
            // button on its way.
            self.start_check(true);
            return;
        }
        let PendingInstall {
            kind: offered_kind,
            returns_to_stable,
            request,
        } = pending;
        if self.state.checking.replace(true) {
            return;
        }
        self.state.installing.set(true);
        self.view
            .status
            .set_text(&crate::i18n::tr(if returns_to_stable {
                "Downloading stable release…"
            } else {
                "Downloading update…"
            }));
        self.view.progress.set_fraction(0.0);
        self.view.progress.set_visible(true);
        self.view.progress.remove_css_class("error");
        self.view.button.set_label(&crate::i18n::tr("Cancel"));
        let started = start_install(
            &self.install_guard,
            request,
            &self.launcher,
            self.install_callbacks(returns_to_stable),
        );
        match started {
            Ok(cancel) => *self.state.cancel_handle.borrow_mut() = Some(cancel),
            Err(request) => self.keep_offer_after_rejection(PendingInstall {
                kind: offered_kind,
                returns_to_stable,
                request,
            }),
        }
    }

    /// An install from an update dialog or another window is already running.
    /// Leave this row re-triable rather than stuck mid-"downloading" with
    /// nothing actually happening.
    fn keep_offer_after_rejection(&self, pending: PendingInstall) {
        self.view.status.set_text(&crate::i18n::tr(
            "Another install is already running — try again shortly.",
        ));
        self.view.progress.set_visible(false);
        self.view
            .button
            .set_label(&crate::i18n::tr(if pending.returns_to_stable {
                "Return to stable"
            } else {
                "Install update"
            }));
        self.view.button.set_sensitive(true);
        self.state.checking.set(false);
        self.state.installing.set(false);
        *self.state.pending_download.borrow_mut() = Some(pending);
    }

    fn install_callbacks(&self, returns_to_stable: bool) -> InstallCallbacks {
        InstallCallbacks {
            on_progress: Box::new({
                let view = self.view.clone();
                move |event| {
                    if matches!(event, InstallProgress::Finalizing) {
                        view.button.set_sensitive(false);
                    }
                    apply_install_progress(&view.status, &view.progress, event);
                }
            }),
            on_installed: Box::new({
                let (view, state) = (self.view.clone(), self.state.clone());
                move || {
                    let _taken = state.cancel_handle.borrow_mut().take();
                    view.status.set_text(&crate::i18n::tr(if returns_to_stable {
                        "Stable release installed — restart to apply"
                    } else {
                        "Update installed — restart to apply"
                    }));
                    view.button.set_label(&crate::i18n::tr("Restart now"));
                    view.button.set_sensitive(true);
                    state.installed.set(true);
                    state.checking.set(false);
                    restart_application(&view.button);
                }
            }),
            on_cancelled: Box::new({
                let (view, state) = (self.view.clone(), self.state.clone());
                move || {
                    let _taken = state.cancel_handle.borrow_mut().take();
                    view.status.set_text(&crate::i18n::tr("Update cancelled"));
                    view.progress.set_visible(false);
                    view.button.set_label(&crate::i18n::tr("Check now"));
                    view.button.set_sensitive(true);
                    state.checking.set(false);
                    state.installing.set(false);
                }
            }),
            on_failed: Box::new({
                let (view, state) = (self.view.clone(), self.state.clone());
                move |message| {
                    let _taken = state.cancel_handle.borrow_mut().take();
                    view.status.set_text(&install_failure_message(message));
                    view.progress.add_css_class("error");
                    view.button.set_label(&crate::i18n::tr("Check now"));
                    view.button.set_sensitive(true);
                    state.checking.set(false);
                    state.installing.set(false);
                }
            }),
        }
    }
}

fn install_failure_message(message: Option<String>) -> String {
    match message {
        Some(message) => rust_i18n::t!(
            "Couldn't install update: %{message}",
            message = crate::services::error_detail(message)
        )
        .into_owned(),
        None => crate::i18n::tr("Couldn't install update"),
    }
}
