// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    future::Future,
    io,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    app::Browser,
    ui::{
        browser::{BrowserView, show_error_dialog},
        controls::{message_dialog_description, modal_layout},
        modal::{ModalHost, dismiss_modal_layer, modal_layer},
    },
};

use super::navigate_home_if_within;

const RELEASE_PROGRESS_DELAY: Duration = Duration::from_millis(350);

pub(super) const RELEASE_BODY: &str =
    "Writing data to the drive. Do not unplug it. You can hide this and keep working.";

pub(super) const RELEASE_ROW_TOOLTIP: &str = "Writing data to the drive. Do not unplug it.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReleaseKind {
    Unmount,
    Eject,
    Lock,
}

impl ReleaseKind {
    fn title(self) -> &'static str {
        match self {
            Self::Unmount => "Unmounting",
            Self::Eject => "Ejecting",
            Self::Lock => "Locking volume",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Lock => crate::assets::icons::LOCK,
            Self::Unmount | Self::Eject => crate::assets::icons::EJECT,
        }
    }

    pub(super) fn error_title(self) -> &'static str {
        match self {
            Self::Unmount => super::media_release_error_title(super::MediaRelease::UnmountMount),
            Self::Eject => super::media_release_error_title(super::MediaRelease::EjectMount),
            Self::Lock => "Unable to lock device",
        }
    }

    fn safe_to_remove(self) -> bool {
        matches!(self, Self::Unmount | Self::Eject)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OverlayMode {
    Hide,
    Dismiss,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReleasePhase {
    Flushing,
    Releasing,
    FinishedOk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DeviceIds {
    pub unix_device: Option<String>,
    pub uuid: Option<String>,
    pub mount_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReleaseKey {
    pub unix_device: Option<String>,
    pub uuid: Option<String>,
    pub mount_root: Option<PathBuf>,
    pub display_name: String,
}

impl ReleaseKey {
    fn ids(&self) -> DeviceIds {
        DeviceIds {
            unix_device: self.unix_device.clone(),
            uuid: self.uuid.clone(),
            mount_root: self.mount_root.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReleaseDecision {
    pub call_gio: bool,
    pub clear_pending: bool,
    pub dialog_title: Option<String>,
    pub dialog_message: Option<String>,
    pub safe_to_remove: bool,
}

struct ReleaseOverlay {
    layer: gtk::Box,
    host_overlay: gtk::Overlay,
    blurred_root: Option<crate::ui::blur::BlurBin>,
    title: gtk::Label,
    body: gtk::Label,
    confirm: gtk::Button,
    loading: gtk::Spinner,
    mode: Rc<Cell<OverlayMode>>,
}

struct PendingRelease {
    key: ReleaseKey,
    kind: ReleaseKind,
    phase: ReleasePhase,
    dismissed: bool,
    progress_message: Option<String>,
    error_count: usize,
    /// Success may say the drive can be unplugged only when it is removable or ejectable.
    unplug: bool,
    parent: Option<gtk::Widget>,
    overlay: Option<ReleaseOverlay>,
    present_source: Option<glib::SourceId>,
    #[expect(
        dead_code,
        reason = "held so Drop clears the header spinner when the release finishes"
    )]
    activity: Option<crate::ui::browser::GlobalActivity>,
}

thread_local! {
    static RELEASES: RefCell<Vec<PendingRelease>> = const { RefCell::new(Vec::new()) };
    static REBUILDERS: RefCell<Vec<(u64, std::rc::Weak<RebuildHook>)>> =
        const { RefCell::new(Vec::new()) };
    static NEXT_WATCH: Cell<u64> = const { Cell::new(1) };
}

struct RebuildHook {
    rebuild: Box<dyn Fn()>,
}

pub(super) struct SidebarWatch {
    id: u64,
    _hook: Rc<RebuildHook>,
}

impl Drop for SidebarWatch {
    fn drop(&mut self) {
        REBUILDERS.with(|hooks| hooks.borrow_mut().retain(|(id, _)| *id != self.id));
    }
}

pub(super) fn watch_sidebars(rebuild: impl Fn() + 'static) -> SidebarWatch {
    let id = NEXT_WATCH.with(|next| {
        let id = next.get();
        next.set(id.saturating_add(1));
        id
    });
    let hook = Rc::new(RebuildHook {
        rebuild: Box::new(rebuild),
    });
    REBUILDERS.with(|hooks| hooks.borrow_mut().push((id, Rc::downgrade(&hook))));
    SidebarWatch { id, _hook: hook }
}

fn refresh_sidebars() {
    let hooks: Vec<Rc<RebuildHook>> = REBUILDERS.with(|hooks| {
        hooks
            .borrow()
            .iter()
            .filter_map(|(_, hook)| hook.upgrade())
            .collect()
    });
    for hook in hooks {
        (hook.rebuild)();
    }
}

pub(super) fn device_matches(key: &ReleaseKey, device: &DeviceIds) -> bool {
    key.unix_device
        .as_ref()
        .is_some_and(|id| device.unix_device.as_ref() == Some(id))
        || key
            .uuid
            .as_ref()
            .is_some_and(|id| device.uuid.as_ref() == Some(id))
        || key
            .mount_root
            .as_ref()
            .is_some_and(|root| device.mount_root.as_ref() == Some(root))
}

fn keys_match(left: &ReleaseKey, right: &ReleaseKey) -> bool {
    device_matches(left, &right.ids())
}

fn with_releases<T>(update: impl FnOnce(&mut Vec<PendingRelease>) -> T) -> T {
    RELEASES.with(|releases| update(&mut releases.borrow_mut()))
}

pub(super) fn any_pending() -> bool {
    RELEASES.with(|releases| !releases.borrow().is_empty())
}

#[cfg(test)]
pub(super) fn reset_releases() {
    RELEASES.with(|releases| releases.borrow_mut().clear());
}

pub(super) fn is_pending(device: &DeviceIds) -> bool {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .any(|entry| device_matches(&entry.key, device))
    })
}

pub(super) fn unmatched_pending(live: &[DeviceIds]) -> Vec<ReleaseKey> {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .filter(|entry| !live.iter().any(|device| device_matches(&entry.key, device)))
            .map(|entry| entry.key.clone())
            .collect()
    })
}

pub(super) fn pending_device_shell(row: &gtk::Button) -> gtk::Box {
    row.set_sensitive(false);
    row.set_tooltip_text(Some(RELEASE_ROW_TOOLTIP));
    row.update_property(&[gtk::accessible::Property::Label(RELEASE_ROW_TOOLTIP)]);
    let spinner = gtk::Spinner::new();
    spinner.add_css_class("sidebar-device-action");
    spinner.set_tooltip_text(Some(RELEASE_ROW_TOOLTIP));
    spinner.update_property(&[gtk::accessible::Property::Label(RELEASE_ROW_TOOLTIP)]);
    spinner.set_hexpand(false);
    spinner.set_halign(gtk::Align::Center);
    spinner.set_valign(gtk::Align::Center);
    spinner.set_width_request(24);
    spinner.start();
    let shell = super::sidebar_device_row(row, None, None);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    actions.add_css_class("sidebar-device-actions");
    actions.set_hexpand(false);
    actions.set_halign(gtk::Align::End);
    actions.set_valign(gtk::Align::Center);
    actions.append(&spinner);
    shell.append(&actions);
    shell
}

pub(super) fn key_for_volume(volume: &gio::Volume) -> ReleaseKey {
    let mount = volume.get_mount();
    ReleaseKey {
        unix_device: super::gio_volume_unix_device(volume)
            .map(|value| value.to_string())
            .or_else(|| {
                volume
                    .drive()
                    .and_then(|drive| unix_device_identifier(&drive))
            }),
        uuid: volume.uuid().map(|value| value.to_string()),
        mount_root: mount.as_ref().and_then(mount_root_path),
        display_name: volume.name().to_string(),
    }
}

pub(super) fn ids_for_volume(volume: &gio::Volume) -> DeviceIds {
    key_for_volume(volume).ids()
}

pub(super) fn key_for_mount(mount: &gio::Mount) -> ReleaseKey {
    let volume = mount.volume();
    ReleaseKey {
        unix_device: mount
            .drive()
            .and_then(|drive| unix_device_identifier(&drive))
            .or_else(|| {
                volume
                    .as_ref()
                    .and_then(|volume| ids_for_volume(volume).unix_device)
            }),
        uuid: volume
            .as_ref()
            .and_then(|volume| volume.uuid())
            .map(|value| value.to_string())
            .or_else(|| {
                mount
                    .drive()
                    .and_then(|drive| drive.identifier(gio::VOLUME_IDENTIFIER_KIND_UUID.as_str()))
                    .map(|value| value.to_string())
            }),
        mount_root: mount_root_path(mount),
        display_name: mount.name().to_string(),
    }
}

pub(super) fn ids_for_mount(mount: &gio::Mount) -> DeviceIds {
    key_for_mount(mount).ids()
}

pub(super) fn key_for_drive(drive: &gio::Drive) -> ReleaseKey {
    ReleaseKey {
        unix_device: unix_device_identifier(drive),
        uuid: drive
            .identifier(gio::VOLUME_IDENTIFIER_KIND_UUID.as_str())
            .map(|value| value.to_string()),
        mount_root: None,
        display_name: drive.name().to_string(),
    }
}

pub(super) fn ids_for_drive(drive: &gio::Drive) -> DeviceIds {
    key_for_drive(drive).ids()
}

fn unix_device_identifier(drive: &gio::Drive) -> Option<String> {
    drive
        .identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str())
        .map(|value| value.to_string())
}

fn mount_root_path(mount: &gio::Mount) -> Option<PathBuf> {
    mount.root().path().map(|path| path.to_path_buf())
}

#[cfg(test)]
pub(super) fn begin_pending(key: ReleaseKey, kind: ReleaseKind) -> bool {
    with_releases(|releases| {
        if releases.iter().any(|entry| keys_match(&entry.key, &key)) {
            return false;
        }
        releases.push(PendingRelease {
            key,
            kind,
            phase: ReleasePhase::Flushing,
            dismissed: false,
            progress_message: None,
            error_count: 0,
            unplug: true,
            parent: None,
            overlay: None,
            present_source: None,
            activity: None,
        });
        true
    })
}

pub(super) fn release_is_pending(key: &ReleaseKey) -> bool {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .any(|entry| keys_match(&entry.key, key))
    })
}

#[cfg(test)]
pub(super) fn release_is_dismissed(key: &ReleaseKey) -> bool {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .is_some_and(|entry| entry.dismissed)
    })
}

#[cfg(test)]
pub(super) fn release_shows_spinner(key: &ReleaseKey) -> bool {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .any(|entry| keys_match(&entry.key, key) && entry.phase != ReleasePhase::FinishedOk)
    })
}

#[cfg(test)]
pub(super) fn release_phase(key: &ReleaseKey) -> Option<ReleasePhase> {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .map(|entry| entry.phase)
    })
}

#[cfg(test)]
pub(super) fn release_error_count(key: &ReleaseKey) -> usize {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .map(|entry| entry.error_count)
            .unwrap_or(0)
    })
}

#[cfg(test)]
pub(super) fn release_body_text(key: &ReleaseKey) -> Option<String> {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .map(|entry| body_for_progress(entry.progress_message.as_deref()))
    })
}

#[cfg(test)]
pub(super) fn release_overlay_presented(key: &ReleaseKey) -> bool {
    RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .is_some_and(|entry| entry.overlay.is_some())
    })
}

pub(super) fn enter_releasing(key: &ReleaseKey) {
    with_releases(|releases| {
        if let Some(entry) = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))
        {
            entry.phase = ReleasePhase::Releasing;
        }
    });
}

#[cfg(test)]
pub(super) fn clear_release_dismissed(key: &ReleaseKey) {
    with_releases(|releases| {
        if let Some(entry) = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))
        {
            entry.dismissed = false;
        }
    });
}

pub(super) fn note_unmount_progress(
    key: &ReleaseKey,
    message: &str,
    _time_left: i64,
    _bytes_left: i64,
) {
    let trimmed = message.trim();
    with_releases(|releases| {
        let Some(entry) = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))
        else {
            return;
        };
        if trimmed.is_empty() {
            return;
        }
        entry.progress_message = Some(trimmed.to_owned());
        if let Some(overlay) = entry.overlay.as_ref() {
            overlay
                .body
                .set_text(&body_for_progress(entry.progress_message.as_deref()));
        }
    });
}

fn body_for_progress(message: Option<&str>) -> String {
    match message.map(str::trim).filter(|message| !message.is_empty()) {
        Some(message) => format!("{message}\nDo not unplug it."),
        None => RELEASE_BODY.to_owned(),
    }
}

pub(super) fn decide_flush(kind: ReleaseKind, flush: Result<(), impl ToString>) -> ReleaseDecision {
    match flush {
        Ok(()) => ReleaseDecision {
            call_gio: true,
            clear_pending: false,
            dialog_title: None,
            dialog_message: None,
            safe_to_remove: false,
        },
        Err(error) => ReleaseDecision {
            call_gio: false,
            clear_pending: true,
            dialog_title: Some(kind.error_title().to_owned()),
            dialog_message: Some(error.to_string()),
            safe_to_remove: false,
        },
    }
}

pub(super) fn decide_gio(
    kind: ReleaseKind,
    progress: Option<&str>,
    result: Result<(), glib::Error>,
    unplug: bool,
) -> ReleaseDecision {
    match result {
        Ok(()) => ReleaseDecision {
            call_gio: false,
            clear_pending: true,
            dialog_title: None,
            dialog_message: None,
            safe_to_remove: kind.safe_to_remove() && unplug,
        },
        Err(error) if release_error_is_quiet(&error) => ReleaseDecision {
            call_gio: false,
            clear_pending: true,
            dialog_title: None,
            dialog_message: None,
            safe_to_remove: false,
        },
        Err(error) => {
            let mut message = error.to_string();
            if let Some(progress) = progress.map(str::trim).filter(|line| !line.is_empty()) {
                message.push('\n');
                message.push_str(progress);
            }
            ReleaseDecision {
                call_gio: false,
                clear_pending: true,
                dialog_title: Some(kind.error_title().to_owned()),
                dialog_message: Some(message),
                safe_to_remove: false,
            }
        }
    }
}

fn release_error_is_quiet(error: &glib::Error) -> bool {
    error.matches(gio::IOErrorEnum::Cancelled)
        || error.matches(gio::IOErrorEnum::FailedHandled)
        || error.message().to_ascii_lowercase().contains("aborted")
}

#[cfg(test)]
pub(super) fn run_release_attempt(
    kind: ReleaseKind,
    progress: Option<&str>,
    flush: Result<(), String>,
    unplug: bool,
    gio_calls: &Cell<u32>,
    gio: impl FnOnce() -> Result<(), glib::Error>,
) -> ReleaseDecision {
    let decision = decide_flush(kind, flush);
    if !decision.call_gio {
        return decision;
    }
    gio_calls.set(gio_calls.get().saturating_add(1));
    decide_gio(kind, progress, gio(), unplug)
}

#[cfg(test)]
pub(super) fn settle_pending_attempt(
    key: &ReleaseKey,
    flush: Result<(), String>,
    gio_calls: &Cell<u32>,
    gio: impl FnOnce() -> Result<(), glib::Error>,
) -> ReleaseDecision {
    let (kind, progress, unplug) = RELEASES.with(|releases| {
        let releases = releases.borrow();
        let entry = releases.iter().find(|entry| keys_match(&entry.key, key));
        (
            entry
                .map(|entry| entry.kind)
                .unwrap_or(ReleaseKind::Unmount),
            entry.and_then(|entry| entry.progress_message.clone()),
            entry.is_some_and(|entry| entry.unplug),
        )
    });
    let decision = run_release_attempt(kind, progress.as_deref(), flush, unplug, gio_calls, gio);
    if decision.clear_pending {
        remove_release(key);
    }
    decision
}

fn remove_release(key: &ReleaseKey) -> Option<PendingRelease> {
    with_releases(|releases| {
        let index = releases
            .iter()
            .position(|entry| keys_match(&entry.key, key))?;
        let mut entry = releases.remove(index);
        if let Some(source) = entry.present_source.take() {
            source.remove();
        }
        Some(entry)
    })
}

pub(super) fn hide_release(key: &ReleaseKey) {
    let overlay = with_releases(|releases| {
        let entry = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))?;
        entry.dismissed = true;
        if let Some(source) = entry.present_source.take() {
            source.remove();
        }
        entry.overlay.take()
    });
    if let Some(overlay) = overlay {
        dismiss_modal_layer(
            &overlay.layer,
            &overlay.host_overlay,
            overlay.blurred_root.as_ref(),
        );
    }
}

pub(super) fn present_release_overlay(parent: &gtk::Widget, key: &ReleaseKey) {
    let snapshot = with_releases(|releases| {
        let entry = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))?;
        entry.present_source = None;
        if entry.dismissed || entry.overlay.is_some() || entry.phase == ReleasePhase::FinishedOk {
            return None;
        }
        entry.parent = Some(parent.clone());
        Some((
            entry.kind,
            entry.key.display_name.clone(),
            entry.progress_message.clone(),
        ))
    });
    let Some((kind, display_name, progress)) = snapshot else {
        return;
    };
    let Some(host) = ModalHost::blurred_for(parent) else {
        return;
    };
    let layout = modal_layout(kind.icon(), kind.title(), &display_name, "Hide");
    layout.content.add_css_class("compact");
    layout.set_loading(true, Some(kind.title()));
    layout.cancel.set_visible(false);
    let body = message_dialog_description(&body_for_progress(progress.as_deref()));
    layout.body.append(&body);
    let layer = modal_layer(
        &layout.content,
        &host.overlay,
        host.blurred_root.clone(),
        Some(Rc::new(|| true)),
    );
    host.overlay.add_overlay(&layer);

    let mode = Rc::new(Cell::new(OverlayMode::Hide));
    let hide_key = key.clone();
    let layer_for_hide = layer.clone();
    let overlay_for_hide = host.overlay.clone();
    let root_for_hide = host.blurred_root.clone();
    let hide: Rc<dyn Fn()> = Rc::new({
        let mode = mode.clone();
        move || {
            if mode.get() == OverlayMode::Hide {
                hide_release(&hide_key);
            } else {
                dismiss_modal_layer(&layer_for_hide, &overlay_for_hide, root_for_hide.as_ref());
            }
        }
    });
    let hide_clicked = hide.clone();
    layout.confirm.connect_clicked(move |_| hide_clicked());
    let hide_closed = hide.clone();
    layout.close.connect_clicked(move |_| hide_closed());
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            hide();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);
    layout.confirm.grab_focus();

    let stored = with_releases(|releases| {
        let Some(entry) = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))
        else {
            return false;
        };
        if entry.dismissed {
            return false;
        }
        entry.overlay = Some(ReleaseOverlay {
            layer: layer.clone(),
            host_overlay: host.overlay.clone(),
            blurred_root: host.blurred_root.clone(),
            title: layout.title,
            body,
            confirm: layout.confirm,
            loading: layout.loading,
            mode,
        });
        true
    });
    if !stored {
        dismiss_modal_layer(&layer, &host.overlay, host.blurred_root.as_ref());
    }
}

fn schedule_present(parent: gtk::Widget, key: ReleaseKey) {
    let present_key = key.clone();
    let source = glib::timeout_add_local_once(RELEASE_PROGRESS_DELAY, move || {
        present_release_overlay(&parent, &present_key);
    });
    with_releases(|releases| {
        if let Some(entry) = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, &key))
        {
            if let Some(previous) = entry.present_source.replace(source) {
                previous.remove();
            }
        } else {
            source.remove();
        }
    });
}

#[cfg(test)]
pub(super) fn complete_release_success(key: &ReleaseKey) {
    let (kind, unplug) = RELEASES.with(|releases| {
        releases
            .borrow()
            .iter()
            .find(|entry| keys_match(&entry.key, key))
            .map(|entry| (entry.kind, entry.unplug))
            .unwrap_or((ReleaseKind::Unmount, false))
    });
    apply_decision(key, decide_gio(kind, None, Ok(()), unplug));
}

fn apply_decision(key: &ReleaseKey, decision: ReleaseDecision) {
    let taken = with_releases(|releases| {
        let entry = releases
            .iter_mut()
            .find(|entry| keys_match(&entry.key, key))?;
        if decision.safe_to_remove {
            entry.phase = ReleasePhase::FinishedOk;
        }
        Some((
            entry.dismissed,
            entry.parent.clone(),
            entry.overlay.take(),
            entry.key.display_name.clone(),
        ))
    });
    if let Some((dismissed, parent, overlay, display_name)) = taken {
        if decision.safe_to_remove {
            if !dismissed && let Some(overlay) = overlay {
                show_safe_to_remove(&overlay, &display_name);
            }
        } else if let Some(overlay) = overlay {
            dismiss_modal_layer(
                &overlay.layer,
                &overlay.host_overlay,
                overlay.blurred_root.as_ref(),
            );
        }
        if decision.clear_pending
            && let (Some(parent), Some(title), Some(message)) = (
                parent.as_ref(),
                decision.dialog_title.as_deref(),
                decision.dialog_message.as_deref(),
            )
        {
            with_releases(|releases| {
                if let Some(entry) = releases
                    .iter_mut()
                    .find(|entry| keys_match(&entry.key, key))
                {
                    entry.error_count = entry.error_count.saturating_add(1);
                }
            });
            show_error_dialog(parent, title, message);
        }
    }
    if decision.clear_pending {
        remove_release(key);
        refresh_sidebars();
    }
}

fn show_safe_to_remove(overlay: &ReleaseOverlay, display_name: &str) {
    overlay.mode.set(OverlayMode::Dismiss);
    overlay.title.set_text("Safe to remove");
    overlay.loading.stop();
    overlay.loading.set_visible(false);
    overlay.loading.set_tooltip_text(None);
    overlay
        .body
        .set_text(&format!("{display_name} can be unplugged."));
    overlay.confirm.set_label("Close");
}

#[expect(
    clippy::too_many_arguments,
    reason = "release start wires the window, flush root, and GIO future together"
)]
pub(super) fn start_release<F, Fut>(
    parent: &gtk::Widget,
    browser: Option<Rc<Browser>>,
    view: Option<&BrowserView>,
    key: ReleaseKey,
    kind: ReleaseKind,
    unplug: bool,
    sync_root: Option<PathBuf>,
    away_root: Option<gio::File>,
    on_settled: impl FnOnce() + 'static,
    run_gio: F,
) -> bool
where
    F: FnOnce(gtk::MountOperation) -> Fut + 'static,
    Fut: Future<Output = Result<(), glib::Error>> + 'static,
{
    if release_is_pending(&key) {
        return false;
    }
    let activity = view.map(|view| view.begin_global_activity("Writing to device…"));
    let inserted = with_releases(|releases| {
        if releases.iter().any(|entry| keys_match(&entry.key, &key)) {
            return false;
        }
        releases.push(PendingRelease {
            key: key.clone(),
            kind,
            phase: ReleasePhase::Flushing,
            dismissed: false,
            progress_message: None,
            error_count: 0,
            unplug,
            parent: Some(parent.clone()),
            overlay: None,
            present_source: None,
            activity,
        });
        true
    });
    if !inserted {
        return false;
    }
    let parent = parent.clone();
    glib::timeout_add_local_once(Duration::ZERO, move || {
        refresh_sidebars();
        schedule_present(parent.clone(), key.clone());
        glib::MainContext::default().spawn_local(async move {
            // Drop directory monitors before syncfs. Restore only when GIO
            // fails and the volume is still mounted.
            let previous = browser
                .as_ref()
                .and_then(|browser| browser.active_location());
            if let (Some(browser), Some(root)) = (browser.as_ref(), away_root.as_ref()) {
                navigate_home_if_within(browser, root);
            }
            #[cfg(test)]
            note_release_before_flush(browser.as_ref());
            let flush_root = sync_root.clone();
            let flush = flush_mount(sync_root).await;
            let decision = match flush {
                Err(error) => decide_flush(kind, Err(error)),
                Ok(()) => {
                    enter_releasing(&key);
                    let operation = mount_operation(&parent, &key);
                    let result = run_gio(operation).await;
                    if result.is_err()
                        && flush_root.as_deref().is_some_and(volume_still_mounted)
                        && let Some(browser) = browser.as_ref()
                        && let Some(previous) = previous.as_ref()
                        && browser.active_location().as_ref() != Some(previous)
                    {
                        browser.navigate(previous.clone());
                    }
                    let (progress, unplug) = RELEASES.with(|releases| {
                        let releases = releases.borrow();
                        let entry = releases.iter().find(|entry| keys_match(&entry.key, &key));
                        (
                            entry.and_then(|entry| entry.progress_message.clone()),
                            entry.is_some_and(|entry| entry.unplug),
                        )
                    });
                    decide_gio(kind, progress.as_deref(), result, unplug)
                }
            };
            apply_decision(&key, decision);
            on_settled();
        });
    });
    true
}

fn blocking_join_message(error: Box<dyn std::any::Any + Send>) -> String {
    error
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            error
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
        })
        .unwrap_or_else(|| "filesystem sync panicked".to_owned())
}

fn volume_still_mounted(root: &Path) -> bool {
    #[cfg(test)]
    if let Some(forced) = MOUNTED_ROOTS.with(|slot| slot.borrow().clone()) {
        return forced
            .iter()
            .any(|mount| root == mount || root.starts_with(mount));
    }
    crate::adapters::MountTable::current().is_mount_point(root)
}

async fn flush_mount(root: Option<PathBuf>) -> Result<(), io::Error> {
    let Some(root) = root else {
        return Ok(());
    };
    // Open and syncfs stay on the blocking worker. Statting here would freeze
    // the UI, and a failed stat must not be reported as "not a directory".
    match gio::spawn_blocking(move || crate::adapters::flush_filesystem(&root)).await {
        Ok(result) => result,
        Err(error) => Err(io::Error::other(blocking_join_message(error))),
    }
}

#[cfg(test)]
thread_local! {
    static LOCATION_AT_FLUSH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static SYNCS_BEFORE_FLUSH: Cell<usize> = const { Cell::new(0) };
    static MOUNTED_ROOTS: RefCell<Option<Vec<PathBuf>>> = const { RefCell::new(None) };
}

#[cfg(test)]
fn note_release_before_flush(browser: Option<&Rc<Browser>>) {
    LOCATION_AT_FLUSH.with(|slot| {
        *slot.borrow_mut() = browser
            .and_then(|browser| browser.active_location())
            .and_then(|location| location.native_path().map(Path::to_path_buf));
    });
    SYNCS_BEFORE_FLUSH.with(|slot| slot.set(crate::adapters::sync_probe_len()));
}

#[cfg(test)]
pub(super) fn location_at_flush_for_test() -> Option<PathBuf> {
    LOCATION_AT_FLUSH.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
pub(super) fn syncs_before_flush_for_test() -> usize {
    SYNCS_BEFORE_FLUSH.with(|slot| slot.get())
}

#[cfg(test)]
pub(super) fn set_mounted_roots_for_test(roots: Option<Vec<PathBuf>>) {
    MOUNTED_ROOTS.with(|slot| *slot.borrow_mut() = roots);
}

pub(super) fn mount_operation(parent: &gtk::Widget, key: &ReleaseKey) -> gtk::MountOperation {
    let window = parent.root().and_downcast::<gtk::Window>();
    let operation = gtk::MountOperation::new(window.as_ref());
    let key = key.clone();
    operation.connect_show_unmount_progress(move |operation, message, time_left, bytes_left| {
        operation.stop_signal_emission_by_name("show-unmount-progress");
        note_unmount_progress(&key, message, time_left, bytes_left);
    });
    operation
}

#[cfg(test)]
mod tests;
