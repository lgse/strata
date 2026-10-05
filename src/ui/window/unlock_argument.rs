// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use super::{
    BrowserView,
    open_argument::{CONNECTING_DELAY, clear_status, show_connecting_overlay},
    present_target,
};

const INVALID_UNLOCK_VOLUME_OPERAND: &str = "invalid --unlock-volume operand";
const CONNECTING_MESSAGE: &str = "Waiting for encrypted volume…";
const NOT_ENCRYPTED_MESSAGE: &str = "This is not an encrypted volume";
const NOT_FOUND_MESSAGE: &str = "The encrypted volume was not found";

const UNLOCK_APPEARANCE_WAIT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnlockTarget {
    pub unix_device: Option<PathBuf>,
    pub uuid: Option<String>,
}

impl UnlockTarget {
    pub fn parse(operand: &str) -> Result<Self, &'static str> {
        if operand.is_empty() {
            return Err(INVALID_UNLOCK_VOLUME_OPERAND);
        }
        if operand.starts_with('/') {
            return Ok(Self {
                unix_device: Some(PathBuf::from(operand)),
                uuid: None,
            });
        }
        match super::devices::normalize_luks_uuid(operand) {
            Some(uuid) => Ok(Self {
                unix_device: None,
                uuid: Some(uuid),
            }),
            None => Err(INVALID_UNLOCK_VOLUME_OPERAND),
        }
    }
}

pub fn present_unlock(application: &gtk::Application, target: UnlockTarget) -> BrowserView {
    let browser = present_target(application, None, Vec::new(), false, true);
    start_unlock(browser.clone(), target);
    browser
}

fn start_unlock(browser: BrowserView, target: UnlockTarget) {
    start_unlock_wait(browser, target, CONNECTING_DELAY, UNLOCK_APPEARANCE_WAIT);
}

// Subscribe before scanning so a newly appearing volume cannot be missed.
fn start_unlock_wait(
    browser: BrowserView,
    target: UnlockTarget,
    connecting_delay: Duration,
    appearance_wait: Duration,
) {
    let request = UnlockRequest::new(gio::VolumeMonitor::get());
    if let Some(window) = browser.overlay().root().and_downcast::<gtk::Window>() {
        let close_request = request.clone();
        window.connect_destroy(move |_| close_request.finish());
    }

    let added_browser = browser.downgrade();
    let added_target = target.clone();
    let added_request = request.clone();
    let volume_added = request.monitor.connect_volume_added(move |_, _| {
        let Some(browser) = added_browser.upgrade() else {
            added_request.finish();
            return;
        };
        try_complete_unlock(&browser, &added_target, &added_request);
    });
    request.volume_added.replace(Some(volume_added));

    let connected_browser = browser.downgrade();
    let connected_target = target.clone();
    let connected_request = request.clone();
    let drive_connected = request.monitor.connect_drive_connected(move |_, _| {
        let Some(browser) = connected_browser.upgrade() else {
            connected_request.finish();
            return;
        };
        try_complete_unlock(&browser, &connected_target, &connected_request);
    });
    request.drive_connected.replace(Some(drive_connected));

    if try_complete_unlock(&browser, &target, &request) {
        return;
    }

    let connecting_browser = browser.downgrade();
    let connecting_request = request.clone();
    let connecting_timer = glib::timeout_add_local_once(connecting_delay, move || {
        connecting_request.connecting_timer.take();
        if !connecting_request.active.get() {
            return;
        }
        let Some(browser) = connecting_browser.upgrade() else {
            connecting_request.finish();
            return;
        };
        let cancel_request = connecting_request.clone();
        let cancel_browser = browser.downgrade();
        show_connecting_overlay(&browser, CONNECTING_MESSAGE, move || {
            cancel_request.finish();
            if let Some(browser) = cancel_browser.upgrade() {
                clear_status(&browser);
            }
        });
    });
    request.connecting_timer.replace(Some(connecting_timer));

    let timeout_browser = browser.downgrade();
    let timeout_request = request.clone();
    let timeout_target = target.clone();
    let timeout_timer = glib::timeout_add_local_once(appearance_wait, move || {
        timeout_request.timeout_timer.take();
        if !timeout_request.active.get() {
            return;
        }
        timeout_request.finish();
        tracing::warn!(
            ?timeout_target,
            "encrypted volume was not found within the appearance wait"
        );
        if let Some(browser) = timeout_browser.upgrade() {
            show_not_found(&browser);
        }
    });
    request.timeout_timer.replace(Some(timeout_timer));
}

fn try_complete_unlock(
    browser: &BrowserView,
    target: &UnlockTarget,
    request: &UnlockRequest,
) -> bool {
    if !request.active.get() {
        return false;
    }
    match resolve_unlock_target(target, &request.monitor) {
        ResolvedUnlock::Unmatched => false,
        resolved => {
            complete_matched_unlock(browser, request, resolved);
            true
        }
    }
}

fn complete_matched_unlock(
    browser: &BrowserView,
    request: &UnlockRequest,
    resolved: ResolvedUnlock,
) {
    request.finish();
    clear_status(browser);
    apply_resolved_unlock(browser, resolved);
}

fn apply_resolved_unlock(browser: &BrowserView, resolved: ResolvedUnlock) {
    match resolved {
        ResolvedUnlock::Volume(volume) => browser.unlock_volume(volume),
        ResolvedUnlock::PasswordDrive(drive) => browser.start_password_drive(drive, true),
        ResolvedUnlock::NotEncrypted => show_not_encrypted(browser),
        ResolvedUnlock::Unmatched => {}
    }
}

enum ResolvedUnlock {
    Volume(gio::Volume),
    PasswordDrive(gio::Drive),
    NotEncrypted,
    Unmatched,
}

fn resolve_unlock_target(target: &UnlockTarget, monitor: &gio::VolumeMonitor) -> ResolvedUnlock {
    let volumes = monitor.volumes();
    let volume_identities: Vec<_> = volumes.iter().map(volume_unlock_identity).collect();
    let drives = super::orphaned_password_drives(&volumes, &monitor.connected_drives());
    let drive_identities: Vec<_> = drives.iter().map(password_drive_identity).collect();
    match classify_unlock_identities(target, &volume_identities, &drive_identities) {
        UnlockResolution::EncryptedVolume(index) => ResolvedUnlock::Volume(volumes[index].clone()),
        UnlockResolution::PasswordDrive(index) => {
            ResolvedUnlock::PasswordDrive(drives[index].clone())
        }
        UnlockResolution::NotEncrypted => ResolvedUnlock::NotEncrypted,
        UnlockResolution::Unmatched => ResolvedUnlock::Unmatched,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VolumeUnlockIdentity {
    unix_device: Option<String>,
    uuid: Option<String>,
    crypto_uuid: Option<String>,
    encrypted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PasswordDriveIdentity {
    unix_device: Option<String>,
    uuid: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnlockResolution {
    EncryptedVolume(usize),
    PasswordDrive(usize),
    NotEncrypted,
    Unmatched,
}

fn classify_unlock_identities(
    target: &UnlockTarget,
    volumes: &[VolumeUnlockIdentity],
    password_drives: &[PasswordDriveIdentity],
) -> UnlockResolution {
    if let Some(device) = target.unix_device.as_ref()
        && let Some((index, volume)) = volumes.iter().enumerate().find(|(_, volume)| {
            volume
                .unix_device
                .as_ref()
                .is_some_and(|unix| std::path::Path::new(unix) == device)
        })
    {
        return if volume.encrypted {
            UnlockResolution::EncryptedVolume(index)
        } else {
            UnlockResolution::NotEncrypted
        };
    }

    if let Some(target_uuid) = target.uuid.as_deref()
        && let Some((index, volume)) = volumes.iter().enumerate().find(|(_, volume)| {
            uuid_matches(target_uuid, volume.uuid.as_deref())
                || uuid_matches(target_uuid, volume.crypto_uuid.as_deref())
        })
    {
        return if volume.encrypted {
            UnlockResolution::EncryptedVolume(index)
        } else {
            UnlockResolution::NotEncrypted
        };
    }

    if let Some((index, _)) = password_drives
        .iter()
        .enumerate()
        .find(|(_, drive)| password_drive_matches(target, drive))
    {
        return UnlockResolution::PasswordDrive(index);
    }

    UnlockResolution::Unmatched
}

fn uuid_matches(target_uuid: &str, candidate: Option<&str>) -> bool {
    let Some(candidate) = candidate else {
        return false;
    };
    let Some(normalized_target) = super::devices::normalize_luks_uuid(target_uuid) else {
        return false;
    };
    super::devices::normalize_luks_uuid(candidate).as_deref() == Some(normalized_target.as_str())
}

fn password_drive_matches(target: &UnlockTarget, drive: &PasswordDriveIdentity) -> bool {
    if let Some(device) = target.unix_device.as_ref() {
        return drive
            .unix_device
            .as_ref()
            .is_some_and(|unix| std::path::Path::new(unix) == device);
    }
    if let Some(uuid) = target.uuid.as_deref() {
        return uuid_matches(uuid, drive.uuid.as_deref());
    }
    false
}

fn volume_unlock_identity(volume: &gio::Volume) -> VolumeUnlockIdentity {
    VolumeUnlockIdentity {
        unix_device: super::gio_volume_unix_device(volume).map(|device| device.to_string()),
        uuid: volume.uuid().map(|uuid| uuid.to_string()),
        crypto_uuid: super::crypto_password_uuid_for_volume(volume),
        encrypted: super::gio_volume_is_encrypted(volume),
    }
}

fn password_drive_identity(drive: &gio::Drive) -> PasswordDriveIdentity {
    PasswordDriveIdentity {
        unix_device: drive
            .identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str())
            .map(|device| device.to_string()),
        uuid: drive
            .identifier(gio::VOLUME_IDENTIFIER_KIND_UUID.as_str())
            .map(|uuid| uuid.to_string()),
    }
}

fn show_not_encrypted(browser: &BrowserView) {
    show_unlock_error(browser, NOT_ENCRYPTED_MESSAGE);
}

fn show_not_found(browser: &BrowserView) {
    show_unlock_error(browser, NOT_FOUND_MESSAGE);
}

fn show_unlock_error(browser: &BrowserView, message: &str) {
    clear_status(browser);
    let overlay = browser.overlay();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.add_css_class("open-argument-status");
    content.add_css_class("directory-feedback");
    content.set_halign(gtk::Align::Center);
    content.set_valign(gtk::Align::Center);

    let label = gtk::Label::new(Some(message));
    label.add_css_class("status-message");
    label.add_css_class("error");
    label.set_justify(gtk::Justification::Center);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    content.append(&label);

    overlay.add_overlay(&content);
}

struct UnlockRequest {
    active: Cell<bool>,
    connecting_timer: RefCell<Option<glib::SourceId>>,
    timeout_timer: RefCell<Option<glib::SourceId>>,
    volume_added: RefCell<Option<glib::SignalHandlerId>>,
    drive_connected: RefCell<Option<glib::SignalHandlerId>>,
    monitor: gio::VolumeMonitor,
}

impl UnlockRequest {
    fn new(monitor: gio::VolumeMonitor) -> Rc<Self> {
        Rc::new(Self {
            active: Cell::new(true),
            connecting_timer: RefCell::new(None),
            timeout_timer: RefCell::new(None),
            volume_added: RefCell::new(None),
            drive_connected: RefCell::new(None),
            monitor,
        })
    }

    fn finish(&self) {
        if !self.active.replace(false) {
            return;
        }
        if let Some(timer) = self.connecting_timer.take() {
            timer.remove();
        }
        if let Some(timer) = self.timeout_timer.take() {
            timer.remove();
        }
        if let Some(handler) = self.volume_added.take() {
            self.monitor.disconnect(handler);
        }
        if let Some(handler) = self.drive_connected.take() {
            self.monitor.disconnect(handler);
        }
    }
}

#[cfg(test)]
mod tests;
