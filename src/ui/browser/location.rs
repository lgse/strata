// SPDX-License-Identifier: MIT

use crate::adapters::gio_file_for_location;
use crate::model::Location;
use crate::services::{
    LocationValidationError, UriCredentials, backend_unavailable_message, sanitize_uri_credentials,
};
use crate::ui::blur::BlurBin;
use crate::ui::browser::clipboard::copy_path_text;
use crate::ui::browser::{BrowserView, ViewState};
use crate::ui::controls::{
    form_entry, form_label, form_password_entry, message_dialog_description, modal_layout,
    segmented_control, wrap_dialog_text,
};
use crate::ui::modal::{
    ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog, submit_on_enter,
};
use crate::ui::window::{crypto_password_uuid_for_volume, gio_volume_is_encrypted};
use futures_channel::oneshot;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

const UNLOCK_PROGRESS_DELAY: Duration = Duration::from_millis(350);

// Long crumbs middle-elide past this cap; the scroller handles deeper paths.
const BREADCRUMB_LABEL_MAX_CHARS: i32 = 32;

fn ellipsize_crumb_label(label: &gtk::Label) {
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(BREADCRUMB_LABEL_MAX_CHARS);
}

pub(super) struct UnlockProgressView {
    layer: gtk::Box,
    overlay: gtk::Overlay,
    blurred_root: Option<BlurBin>,
}

pub(super) struct UnlockProgressSlot {
    keys: DeviceKeys,
    view: Option<UnlockProgressView>,
    pending: Option<glib::SourceId>,
    dismissed: bool,
    in_flight: bool,
}

pub(super) mod completion;

pub(super) fn is_breadcrumb_button_target(mut target: gtk::Widget) -> bool {
    loop {
        if target.is::<gtk::Button>() {
            return true;
        }
        let Some(parent) = target.parent() else {
            return false;
        };
        if parent.has_css_class("breadcrumbs") {
            return false;
        }
        target = parent;
    }
}

type MountCredentialsHandler = Rc<dyn Fn(MountCredentials)>;

type MountCancelledHandler = Rc<dyn Fn()>;

struct MountDialogHandlers {
    submitted: Option<MountCredentialsHandler>,
    cancelled: Option<MountCancelledHandler>,
}

#[derive(Clone)]
struct MountPromptDetails {
    message: String,
    default_user: String,
    default_domain: String,
    flags: gio::AskPasswordFlags,
}

impl MountPromptDetails {
    fn fallback(location: &Location) -> Self {
        Self {
            message: format!("Enter user and password for “{}”.", location.display_path()),
            default_user: String::new(),
            default_domain: String::new(),
            flags: gio::AskPasswordFlags::NEED_USERNAME
                | gio::AskPasswordFlags::NEED_DOMAIN
                | gio::AskPasswordFlags::NEED_PASSWORD
                | gio::AskPasswordFlags::SAVING_SUPPORTED
                | gio::AskPasswordFlags::ANONYMOUS_SUPPORTED,
        }
    }
}

const AUTHENTICATION_TEXT_WIDTH_CHARS: i32 = 64;

fn show_authentication_dialog(
    browser_overlay: &gtk::Overlay,
    operation: Option<&gio::MountOperation>,
    message: &str,
    defaults: (&str, &str),
    flags: gio::AskPasswordFlags,
    authentication_failed: bool,
    handlers: MountDialogHandlers,
) -> Option<gtk::Box> {
    let MountDialogHandlers {
        submitted,
        cancelled,
    } = handlers;
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(browser_overlay)
    else {
        if let Some(operation) = operation {
            operation.reply(gio::MountOperationResult::Unhandled);
        }
        return None;
    };

    let layout = modal_layout(
        crate::assets::icons::KEY,
        "Authentication required",
        "Authenticate to access this volume or location",
        "Connect",
    );
    layout.content.add_css_class("wide");
    layout.body.add_css_class("authentication-body");
    let explanation_text =
        wrap_dialog_text(message.trim(), AUTHENTICATION_TEXT_WIDTH_CHARS as usize);
    let explanation = gtk::Label::new(Some(&explanation_text));
    explanation.add_css_class("authentication-explanation");
    explanation.set_max_width_chars(AUTHENTICATION_TEXT_WIDTH_CHARS);
    explanation.set_wrap(true);
    explanation.set_xalign(0.0);
    layout.body.append(&explanation);
    if authentication_failed {
        let error_text = wrap_dialog_text(
            "Those credentials weren’t accepted. Check the username, domain, and password, then try again.",
            AUTHENTICATION_TEXT_WIDTH_CHARS as usize,
        );
        let error = gtk::Label::new(Some(&error_text));
        error.add_css_class("authentication-error");
        error.set_max_width_chars(AUTHENTICATION_TEXT_WIDTH_CHARS);
        error.set_wrap(true);
        error.set_xalign(0.0);
        layout.body.append(&error);
    }

    let credentials = gtk::Box::new(gtk::Orientation::Vertical, 10);
    credentials.add_css_class("authentication-fields");

    let username = form_entry();
    username.set_text(defaults.0);
    if flags.contains(gio::AskPasswordFlags::NEED_USERNAME) {
        append_authentication_field(&credentials, "Username", &username);
    }

    let domain = form_entry();
    domain.set_text(defaults.1);
    if flags.contains(gio::AskPasswordFlags::NEED_DOMAIN) {
        append_authentication_field(&credentials, "Domain", &domain);
    }

    let password = form_password_entry();
    password.set_show_peek_icon(true);
    if flags.contains(gio::AskPasswordFlags::NEED_PASSWORD) {
        append_authentication_field(&credentials, "Password", &password);
    }

    let (connect_as_control, connect_as_buttons) =
        segmented_control(&["Registered user", "Anonymous"], 0);
    let anonymous = connect_as_buttons[1].clone();
    if flags.contains(gio::AskPasswordFlags::ANONYMOUS_SUPPORTED) {
        let connect_as = gtk::Box::new(gtk::Orientation::Vertical, 7);
        connect_as.append(&form_label("Connect as"));
        connect_as.append(&connect_as_control);
        layout.body.append(&connect_as);
    }
    layout.body.append(&credentials);

    let (remember, remember_buttons) =
        segmented_control(&["Don't remember", "Until logout", "Forever"], 0);
    if flags.contains(gio::AskPasswordFlags::SAVING_SUPPORTED) {
        let remember_field = gtk::Box::new(gtk::Orientation::Vertical, 5);
        remember_field.append(&form_label("Password storage"));
        remember_field.append(&remember);
        layout.body.append(&remember_field);
    }
    let content = layout.content;
    let close = layout.close;
    let cancel = layout.cancel;
    let connect = layout.confirm;

    let credential_widgets = [
        username.clone().upcast::<gtk::Widget>(),
        domain.clone().upcast(),
        password.clone().upcast(),
        remember.clone().upcast(),
    ];
    anonymous.connect_toggled(move |anonymous| {
        for widget in &credential_widgets {
            widget.set_sensitive(!anonymous.is_active());
        }
    });

    let auth_user = username.clone();
    let auth_domain = domain.clone();
    let auth_password = password.clone();
    let layer = modal_layer(
        &content,
        &window_overlay,
        blurred_root.clone(),
        Some(Rc::new(move || {
            !auth_user.text().is_empty()
                || !auth_domain.text().is_empty()
                || !auth_password.text().is_empty()
        })),
    );
    window_overlay.add_overlay(&layer);

    let cancel_operation = operation.cloned();
    let cancel_handler = cancelled.clone();
    let cancel_layer = layer.clone();
    let cancel_overlay = window_overlay.clone();
    let cancel_root = blurred_root.clone();
    cancel.connect_clicked(move |_| {
        dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
        if let Some(operation) = cancel_operation.as_ref() {
            operation.reply(gio::MountOperationResult::Aborted);
        } else if let Some(cancelled) = cancel_handler.as_ref() {
            cancelled();
        }
    });

    let close_operation = operation.cloned();
    let close_handler = cancelled.clone();
    let close_layer = layer.clone();
    let close_overlay = window_overlay.clone();
    let close_root = blurred_root.clone();
    close.connect_clicked(move |_| {
        dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
        if let Some(operation) = close_operation.as_ref() {
            operation.reply(gio::MountOperationResult::Aborted);
        } else if let Some(cancelled) = close_handler.as_ref() {
            cancelled();
        }
    });

    let connect_operation = operation.cloned();
    let connect_layer = layer.clone();
    let connect_overlay = window_overlay.clone();
    let connect_root = blurred_root.clone();
    let connect_username = username.clone();
    let connect_domain = domain.clone();
    let connect_password = password.clone();
    let connect_anonymous = anonymous.clone();
    let connect_remember = remember_buttons;
    connect.connect_clicked(move |_| {
        let selected = connect_remember
            .iter()
            .position(gtk::ToggleButton::is_active)
            .unwrap_or_default() as u32;
        let credentials = MountCredentials {
            anonymous: connect_anonymous.is_active(),
            username: connect_username.text().to_string(),
            domain: connect_domain.text().to_string(),
            password: connect_password.text().to_string(),
            save: password_save_for_selection(selected),
        };
        if let Some(operation) = connect_operation.as_ref() {
            apply_mount_credentials(operation, &credentials);
        }
        dismiss_modal_layer(&connect_layer, &connect_overlay, connect_root.as_ref());
        if let Some(operation) = connect_operation.as_ref() {
            operation.reply(gio::MountOperationResult::Handled);
        }
        if let Some(submitted) = submitted.as_ref() {
            submitted(credentials);
        }
    });

    submit_on_enter(&layout.body, &connect);

    let escape = gtk::EventControllerKey::new();
    let escape_operation = operation.cloned();
    let escape_handler = cancelled;
    let escape_layer = layer.clone();
    let escape_overlay = window_overlay;
    let escape_root = blurred_root;
    escape.connect_key_pressed(move |_, key, _, _| {
        if key != gtk::gdk::Key::Escape {
            return glib::Propagation::Proceed;
        }
        dismiss_modal_layer(&escape_layer, &escape_overlay, escape_root.as_ref());
        if let Some(operation) = escape_operation.as_ref() {
            operation.reply(gio::MountOperationResult::Aborted);
        } else if let Some(cancelled) = escape_handler.as_ref() {
            cancelled();
        }
        glib::Propagation::Stop
    });
    layer.add_controller(escape);

    if flags.contains(gio::AskPasswordFlags::NEED_USERNAME) && defaults.0.is_empty() {
        username.grab_focus();
    } else if flags.contains(gio::AskPasswordFlags::NEED_PASSWORD) {
        password.grab_focus();
    } else {
        connect.grab_focus();
    }
    Some(layer)
}

fn dismiss_authentication_prompt(browser_overlay: &gtk::Overlay, layer: &gtk::Box) {
    if layer.parent().is_none() {
        return;
    }
    let Some(window_overlay) = crate::ui::modal::window_overlay(browser_overlay) else {
        return;
    };
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    dismiss_modal_layer(layer, &window_overlay, blurred_root.as_ref());
}

fn append_authentication_field(fields: &gtk::Box, label_text: &str, field: &impl IsA<gtk::Widget>) {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 5);
    group.append(&form_label(label_text));
    group.append(field);
    fields.append(&group);
}

fn password_save_for_selection(selected: u32) -> gio::PasswordSave {
    match selected {
        1 => gio::PasswordSave::ForSession,
        2 => gio::PasswordSave::Permanently,
        _ => gio::PasswordSave::Never,
    }
}

fn credentials_from_location_input(
    input: &str,
) -> Result<(String, Option<MountCredentials>), LocationValidationError> {
    if !input.contains("://") {
        return Ok((input.to_owned(), None));
    }
    let (sanitized, credentials) = sanitize_uri_credentials(input)?;
    let credentials = credentials.map(|credentials: UriCredentials| MountCredentials {
        anonymous: false,
        username: credentials.username,
        domain: String::new(),
        password: credentials.password,
        save: gio::PasswordSave::Never,
    });
    Ok((sanitized, credentials))
}

#[derive(Clone)]
pub(super) struct MountCredentials {
    anonymous: bool,
    username: String,
    domain: String,
    password: String,
    save: gio::PasswordSave,
}

impl MountCredentials {
    fn default_for_prompt() -> Self {
        Self {
            anonymous: false,
            username: glib::user_name().to_string_lossy().into_owned(),
            domain: "WORKGROUP".to_owned(),
            password: String::new(),
            save: gio::PasswordSave::Never,
        }
    }
}

fn apply_mount_credentials(operation: &gio::MountOperation, credentials: &MountCredentials) {
    operation.set_anonymous(credentials.anonymous);
    if credentials.anonymous {
        return;
    }
    operation.set_username(Some(&credentials.username));
    if !credentials.domain.is_empty() {
        operation.set_domain(Some(&credentials.domain));
    }
    operation.set_password(Some(&credentials.password));
    operation.set_password_save(credentials.save);
}

#[derive(Clone, Copy)]
pub(super) enum MountStrategy {
    /// The location itself is accessible but sits on an unmounted volume.
    EnclosingVolume,
    /// The location is itself the mountable target (an SMB share, a
    /// "Connect to Server" bookmark, ...).
    Mountable,
}

fn mount_result_is_ok(result: &Result<(), glib::Error>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => error.matches(gio::IOErrorEnum::AlreadyMounted),
    }
}

fn mount_error_is_authentication_failure(location: &Location, error: &glib::Error) -> bool {
    if location.uri_value().is_none() {
        return false;
    }
    if error.matches(gio::IOErrorEnum::PermissionDenied) {
        return true;
    }

    // GVfs' SMB backend reports rejected credentials as G_IO_ERROR_FAILED on
    // some versions, preserving the useful distinction only in its message.
    let message = error.message().to_ascii_lowercase();
    [
        "permission denied",
        "authentication failed",
        "logon failure",
        "invalid credentials",
    ]
    .iter()
    .any(|reason| message.contains(reason))
}

/// Decides what, if anything, to tell the user about a failed mount attempt.
/// A user-initiated cancel (the GTK credential dialog's Cancel button, or a
/// backend that already reported the failure to the operation itself) should
/// quietly return to the prior state rather than surface an alarming error,
/// per lgse/strata#20's "cancelling authentication returns to the prior
/// committed location" requirement.
fn mount_failure_message(location: &Location, error: &glib::Error) -> Option<String> {
    if mount_error_is_cancelled(error) {
        return None;
    }
    if error.matches(gio::IOErrorEnum::NotSupported)
        && let Some(uri) = location.uri_value()
    {
        return Some(backend_unavailable_message(uri));
    }
    Some(error.to_string())
}

fn mount_error_is_cancelled(error: &glib::Error) -> bool {
    error.matches(gio::IOErrorEnum::Cancelled) || error.matches(gio::IOErrorEnum::FailedHandled)
}

enum MountTarget {
    Location(Location, MountStrategy),
    Volume(gio::Volume),
    Drive(gio::Drive),
}

fn volume_error_is_authentication_failure(error: &glib::Error) -> bool {
    let message = error.message().to_ascii_lowercase();
    [
        "incorrect passphrase",
        "invalid passphrase",
        "no key available with this passphrase",
        "authentication failed",
    ]
    .iter()
    .any(|reason| message.contains(reason))
}

fn device_volume_mount_is_ready(result: &Result<(), glib::Error>, mount_present: bool) -> bool {
    mount_present || mount_result_is_ok(result)
}

fn volume_error_is_in_flight_mount(error: &glib::Error) -> bool {
    if error.matches(gio::IOErrorEnum::Pending) || error.matches(gio::IOErrorEnum::Busy) {
        return true;
    }
    let message = error.message().to_ascii_lowercase();
    ["already unlocking", "already in progress"]
        .iter()
        .any(|reason| message.contains(reason))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForeignVolumeWaitOutcome {
    Mounted,
    StillLocked,
    Gone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VolumeSuccessorKind {
    Mounted,
    Locked,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForeignVolumeWaitFollowUp {
    Navigate,
    StartOwnedMount,
    Quiet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnlockViewFollowUp {
    Reload,
    Navigate,
    None,
}

fn unlock_view_follow_up(
    active: Option<&Location>,
    mount_location: &Location,
    user_asked_to_open: bool,
    progress_dismissed: bool,
) -> UnlockViewFollowUp {
    if active.is_some_and(|active| active == mount_location || active.is_within(mount_location)) {
        UnlockViewFollowUp::Reload
    } else if user_asked_to_open && !progress_dismissed {
        UnlockViewFollowUp::Navigate
    } else {
        UnlockViewFollowUp::None
    }
}

fn foreign_volume_wait_follow_up(
    outcome: ForeignVolumeWaitOutcome,
    already_waited: bool,
    successor: VolumeSuccessorKind,
) -> ForeignVolumeWaitFollowUp {
    match outcome {
        ForeignVolumeWaitOutcome::Mounted => ForeignVolumeWaitFollowUp::Navigate,
        ForeignVolumeWaitOutcome::Gone => match successor {
            VolumeSuccessorKind::Mounted => ForeignVolumeWaitFollowUp::Navigate,
            VolumeSuccessorKind::Locked => ForeignVolumeWaitFollowUp::StartOwnedMount,
            VolumeSuccessorKind::Absent => ForeignVolumeWaitFollowUp::Quiet,
        },
        ForeignVolumeWaitOutcome::StillLocked if already_waited => ForeignVolumeWaitFollowUp::Quiet,
        ForeignVolumeWaitOutcome::StillLocked => ForeignVolumeWaitFollowUp::StartOwnedMount,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DeviceKeys {
    volume_tokens: Vec<String>,
    drive_tokens: Vec<String>,
    volume_object: Option<glib::Object>,
    drive_object: Option<glib::Object>,
}

impl DeviceKeys {
    fn new(
        volume: impl IntoIterator<Item = Option<String>>,
        drive: impl IntoIterator<Item = Option<String>>,
    ) -> Self {
        Self {
            volume_tokens: collect_device_tokens(volume),
            drive_tokens: collect_device_tokens(drive),
            volume_object: None,
            drive_object: None,
        }
    }

    fn has_volume_identity(&self) -> bool {
        !self.volume_tokens.is_empty() || self.volume_object.is_some()
    }

    fn is_empty(&self) -> bool {
        !self.has_volume_identity() && self.drive_tokens.is_empty() && self.drive_object.is_none()
    }
}

fn collect_device_tokens(parts: impl IntoIterator<Item = Option<String>>) -> Vec<String> {
    parts
        .into_iter()
        .filter_map(|value| {
            let value = value?.trim().to_owned();
            (!value.is_empty()).then_some(value)
        })
        .collect()
}

fn tokens_overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|token| right.contains(token))
}

fn same_volume(left: &DeviceKeys, right: &DeviceKeys) -> bool {
    tokens_overlap(&left.volume_tokens, &right.volume_tokens)
        || left
            .volume_object
            .as_ref()
            .is_some_and(|object| Some(object) == right.volume_object.as_ref())
}

fn same_drive(left: &DeviceKeys, right: &DeviceKeys) -> bool {
    tokens_overlap(&left.drive_tokens, &right.drive_tokens)
        || left
            .drive_object
            .as_ref()
            .is_some_and(|object| Some(object) == right.drive_object.as_ref())
}

fn unlock_target_matches(left: &DeviceKeys, right: &DeviceKeys) -> bool {
    if left.has_volume_identity() && right.has_volume_identity() {
        same_volume(left, right)
    } else {
        same_drive(left, right)
    }
}

fn unix_device_is_partition_of(volume_unix: &str, drive_unix: &str) -> bool {
    let Some(rest) = volume_unix.strip_prefix(drive_unix) else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    rest.starts_with(|ch: char| ch.is_ascii_digit())
        || rest
            .strip_prefix('p')
            .is_some_and(|digits| digits.starts_with(|ch: char| ch.is_ascii_digit()))
}

fn unix_device_is_sibling_partition(waited: &DeviceKeys, candidate: &DeviceKeys) -> bool {
    candidate.volume_tokens.iter().any(|volume_unix| {
        waited
            .drive_tokens
            .iter()
            .any(|drive_unix| unix_device_is_partition_of(volume_unix, drive_unix))
    })
}

fn identity_matches_volume(waited: &DeviceKeys, candidate: &DeviceKeys) -> bool {
    same_volume(waited, candidate)
        || (!waited.has_volume_identity()
            && same_drive(waited, candidate)
            && !unix_device_is_sibling_partition(waited, candidate))
}

fn begin_unlock_slot(slots: &mut Vec<UnlockProgressSlot>, keys: &DeviceKeys) -> bool {
    if let Some(slot) = slots
        .iter_mut()
        .find(|slot| unlock_target_matches(&slot.keys, keys))
    {
        if slot.in_flight {
            return false;
        }
        slot.in_flight = true;
        slot.dismissed = false;
        return true;
    }
    slots.push(UnlockProgressSlot {
        keys: keys.clone(),
        view: None,
        pending: None,
        dismissed: false,
        in_flight: true,
    });
    true
}

fn unlock_progress_dismissed_for(slots: &[UnlockProgressSlot], keys: &DeviceKeys) -> bool {
    slots
        .iter()
        .find(|slot| unlock_target_matches(&slot.keys, keys))
        .is_some_and(|slot| slot.dismissed)
}

fn gio_identifier(value: Option<glib::GString>) -> Option<String> {
    value.map(|value| value.to_string())
}

fn device_keys(volume: Option<&gio::Volume>, drive: Option<&gio::Drive>) -> DeviceKeys {
    let mut keys = DeviceKeys::new(
        [
            volume.and_then(|volume| {
                gio_identifier(volume.identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str()))
            }),
            volume.and_then(|volume| gio_identifier(volume.uuid())),
            volume.and_then(crypto_password_uuid_for_volume),
        ],
        [
            drive.and_then(|drive| {
                gio_identifier(drive.identifier(gio::VOLUME_IDENTIFIER_KIND_UNIX_DEVICE.as_str()))
            }),
            drive.and_then(|drive| {
                gio_identifier(drive.identifier(gio::VOLUME_IDENTIFIER_KIND_UUID.as_str()))
            }),
        ],
    );
    keys.volume_object = volume.map(|volume| volume.clone().upcast());
    keys.drive_object = drive.map(|drive| drive.clone().upcast());
    keys
}

#[derive(Clone)]
struct DeviceMatch {
    keys: DeviceKeys,
}

impl DeviceMatch {
    fn from_volume(volume: &gio::Volume) -> Self {
        let drive = volume.drive();
        Self {
            keys: device_keys(Some(volume), drive.as_ref()),
        }
    }

    fn from_drive(drive: &gio::Drive) -> Self {
        Self {
            keys: device_keys(None, Some(drive)),
        }
    }

    fn is_absent(&self) -> bool {
        self.keys.is_empty()
    }

    fn matches_mount(&self, mount: &gio::Mount) -> bool {
        identity_matches_volume(
            &self.keys,
            &device_keys(mount.volume().as_ref(), mount.drive().as_ref()),
        )
    }

    fn matches_volume(&self, volume: &gio::Volume) -> bool {
        identity_matches_volume(
            &self.keys,
            &device_keys(Some(volume), volume.drive().as_ref()),
        )
    }

    fn matches_password_drive(&self, drive: &gio::Drive) -> bool {
        drive.start_stop_type() == gio::DriveStartStopType::Password
            && same_drive(&self.keys, &device_keys(None, Some(drive)))
    }
}

fn successor_kind(waited: &DeviceMatch) -> VolumeSuccessorKind {
    if waited.is_absent() {
        return VolumeSuccessorKind::Absent;
    }
    let monitor = gio::VolumeMonitor::get();
    let volumes = monitor.volumes();
    if monitor
        .mounts()
        .iter()
        .any(|mount| waited.matches_mount(mount))
    {
        return VolumeSuccessorKind::Mounted;
    }
    if volumes.iter().any(|volume| waited.matches_volume(volume)) {
        return VolumeSuccessorKind::Locked;
    }
    if monitor
        .connected_drives()
        .iter()
        .any(|drive| waited.matches_password_drive(drive))
    {
        return VolumeSuccessorKind::Locked;
    }
    VolumeSuccessorKind::Absent
}

fn successor_mount_location(waited: &DeviceMatch) -> Option<Location> {
    let monitor = gio::VolumeMonitor::get();
    monitor
        .mounts()
        .into_iter()
        .find(|mount| waited.matches_mount(mount))
        .and_then(|mount| crate::adapters::location_for_file(&mount.root()))
}

fn successor_volume(waited: &DeviceMatch) -> Option<gio::Volume> {
    gio::VolumeMonitor::get()
        .volumes()
        .into_iter()
        .find(|volume| waited.matches_volume(volume))
}

fn successor_password_drive(waited: &DeviceMatch) -> Option<gio::Drive> {
    gio::VolumeMonitor::get()
        .connected_drives()
        .into_iter()
        .find(|drive| waited.matches_password_drive(drive))
}

const FOREIGN_VOLUME_MOUNT_WAIT: Duration = Duration::from_secs(8);

async fn wait_for_mount_change(
    signal: oneshot::Receiver<()>,
    mounted: impl Fn() -> bool,
    timeout: Duration,
) {
    futures_lite::future::race(
        async {
            let _ = signal.await;
        },
        async {
            let deadline = Instant::now() + timeout;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() || mounted() {
                    break;
                }
                glib::timeout_future(remaining.min(Duration::from_millis(200))).await;
            }
        },
    )
    .await;
}

async fn wait_for_foreign_volume_mount(volume: &gio::Volume) -> ForeignVolumeWaitOutcome {
    if volume.get_mount().is_some() {
        return ForeignVolumeWaitOutcome::Mounted;
    }

    let (tx, rx) = oneshot::channel();
    let tx = Rc::new(RefCell::new(Some(tx)));
    let complete = Rc::new({
        let tx = tx.clone();
        move || {
            if let Some(tx) = tx.borrow_mut().take() {
                let _ = tx.send(());
            }
        }
    });
    let removed = Rc::new(Cell::new(false));

    let changed_complete = complete.clone();
    let changed_volume = volume.clone();
    let changed_id = volume.connect_changed(move |_| {
        if changed_volume.get_mount().is_some() {
            changed_complete();
        }
    });
    let removed_flag = removed.clone();
    let removed_complete = complete.clone();
    let removed_id = volume.connect_removed(move |_| {
        removed_flag.set(true);
        removed_complete();
    });
    wait_for_mount_change(
        rx,
        || volume.get_mount().is_some(),
        FOREIGN_VOLUME_MOUNT_WAIT,
    )
    .await;
    volume.disconnect(changed_id);
    volume.disconnect(removed_id);

    if volume.get_mount().is_some() {
        ForeignVolumeWaitOutcome::Mounted
    } else if removed.get() {
        ForeignVolumeWaitOutcome::Gone
    } else {
        ForeignVolumeWaitOutcome::StillLocked
    }
}

async fn wait_for_foreign_drive_start(
    drive: &gio::Drive,
    waited: &DeviceMatch,
) -> ForeignVolumeWaitOutcome {
    if successor_kind(waited) == VolumeSuccessorKind::Mounted {
        return ForeignVolumeWaitOutcome::Mounted;
    }

    let (tx, rx) = oneshot::channel();
    let tx = Rc::new(RefCell::new(Some(tx)));
    let complete = Rc::new({
        let tx = tx.clone();
        move || {
            if let Some(tx) = tx.borrow_mut().take() {
                let _ = tx.send(());
            }
        }
    });
    let removed = Rc::new(Cell::new(false));

    let changed_complete = complete.clone();
    let changed_waited = waited.clone();
    let changed_id = drive.connect_changed(move |_| {
        if successor_kind(&changed_waited) == VolumeSuccessorKind::Mounted {
            changed_complete();
        }
    });
    let removed_flag = removed.clone();
    let removed_complete = complete.clone();
    let disconnected_id = drive.connect_disconnected(move |_| {
        removed_flag.set(true);
        removed_complete();
    });
    wait_for_mount_change(
        rx,
        || successor_kind(waited) == VolumeSuccessorKind::Mounted,
        FOREIGN_VOLUME_MOUNT_WAIT,
    )
    .await;
    drive.disconnect(changed_id);
    drive.disconnect(disconnected_id);

    if successor_kind(waited) == VolumeSuccessorKind::Mounted {
        ForeignVolumeWaitOutcome::Mounted
    } else if removed.get() {
        ForeignVolumeWaitOutcome::Gone
    } else {
        ForeignVolumeWaitOutcome::StillLocked
    }
}

impl BrowserView {
    pub(crate) fn mount_volume(&self, volume: gio::Volume) {
        self.state.mount_device_volume(volume, None, false, true);
    }

    pub(crate) fn unlock_volume(&self, volume: gio::Volume) {
        self.state.mount_device_volume(volume, None, false, true);
    }

    pub(crate) fn start_password_drive(&self, drive: gio::Drive, user_asked_to_open: bool) {
        self.state
            .start_password_drive(drive, None, false, user_asked_to_open);
    }
}

impl ViewState {
    fn apply_unlock_view_follow_up(
        &self,
        keys: &DeviceKeys,
        mount_location: &Location,
        user_asked_to_open: bool,
    ) {
        match unlock_view_follow_up(
            self.browser.active_location().as_ref(),
            mount_location,
            user_asked_to_open,
            unlock_progress_dismissed_for(&self.unlock_slots.borrow(), keys),
        ) {
            UnlockViewFollowUp::Reload => self.browser.reload_active(),
            UnlockViewFollowUp::Navigate => self.browser.navigate(mount_location.clone()),
            UnlockViewFollowUp::None => {}
        }
    }

    fn open_unlocked_identity(
        &self,
        volume: Option<&gio::Volume>,
        waited: &DeviceMatch,
        user_asked_to_open: bool,
    ) {
        let location = volume
            .and_then(|volume| volume.get_mount())
            .and_then(|mount| crate::adapters::location_for_file(&mount.root()))
            .or_else(|| successor_mount_location(waited));
        if let Some(location) = location {
            self.apply_unlock_view_follow_up(&waited.keys, &location, user_asked_to_open);
        }
        self.finish_unlock_slot(&waited.keys);
    }

    fn start_owned_successor(self: &Rc<Self>, waited: &DeviceMatch, user_asked_to_open: bool) {
        let user_asked_to_open = user_asked_to_open
            && !unlock_progress_dismissed_for(&self.unlock_slots.borrow(), &waited.keys);
        self.finish_unlock_slot(&waited.keys);
        if waited.is_absent() {
            return;
        }
        if let Some(volume) = successor_volume(waited) {
            self.mount_device_volume(volume, None, false, user_asked_to_open);
            return;
        }
        if let Some(drive) = successor_password_drive(waited) {
            self.start_password_drive(drive, None, false, user_asked_to_open);
        }
    }

    fn mount_device_volume(
        self: &Rc<Self>,
        volume: gio::Volume,
        credentials: Option<MountCredentials>,
        already_waited: bool,
        user_asked_to_open: bool,
    ) {
        let waited = DeviceMatch::from_volume(&volume);
        if credentials.is_none() && !already_waited && !self.begin_unlock_progress(&waited.keys) {
            return;
        }
        self.mount_target(
            MountTarget::Volume(volume.clone()),
            credentials,
            move |state, result, attempted, details| {
                if !result
                    .as_ref()
                    .err()
                    .is_some_and(volume_error_is_in_flight_mount)
                {
                    state.dismiss_unlock_progress(&waited.keys);
                }
                if device_volume_mount_is_ready(&result, volume.get_mount().is_some()) {
                    state.open_unlocked_identity(Some(&volume), &waited, user_asked_to_open);
                } else if let Err(error) = result {
                    if volume_error_is_authentication_failure(&error)
                        && let Some(details) = details
                    {
                        let weak = Rc::downgrade(state);
                        let retry_volume = volume.clone();
                        state.show_unlock_retry_prompt(
                            waited.keys.clone(),
                            attempted,
                            details,
                            move |credentials| {
                                if let Some(state) = weak.upgrade() {
                                    state.mount_device_volume(
                                        retry_volume.clone(),
                                        Some(credentials),
                                        false,
                                        user_asked_to_open,
                                    );
                                }
                            },
                        );
                    } else if volume_error_is_in_flight_mount(&error) {
                        // A competing automounter may own this job; do not cancel it.
                        tracing::debug!(
                            volume = %volume.name(),
                            already_waited,
                            "waiting for in-flight volume mount"
                        );
                        let weak = Rc::downgrade(state);
                        let wait_volume = volume.clone();
                        let wait_match = waited.clone();
                        glib::MainContext::default().spawn_local(async move {
                            let Some(state) = weak.upgrade() else {
                                return;
                            };
                            let _activity = BrowserView {
                                state: state.clone(),
                            }
                            .begin_global_activity("Connecting…");
                            let outcome = wait_for_foreign_volume_mount(&wait_volume).await;
                            drop(_activity);
                            let successor = match outcome {
                                ForeignVolumeWaitOutcome::Mounted => VolumeSuccessorKind::Mounted,
                                ForeignVolumeWaitOutcome::StillLocked => {
                                    VolumeSuccessorKind::Locked
                                }
                                ForeignVolumeWaitOutcome::Gone => successor_kind(&wait_match),
                            };
                            match foreign_volume_wait_follow_up(outcome, already_waited, successor)
                            {
                                ForeignVolumeWaitFollowUp::Navigate => {
                                    state.open_unlocked_identity(
                                        Some(&wait_volume),
                                        &wait_match,
                                        user_asked_to_open,
                                    );
                                }
                                ForeignVolumeWaitFollowUp::StartOwnedMount
                                    if outcome == ForeignVolumeWaitOutcome::Gone =>
                                {
                                    state.start_owned_successor(&wait_match, user_asked_to_open);
                                }
                                ForeignVolumeWaitFollowUp::StartOwnedMount => {
                                    state.mount_device_volume(
                                        wait_volume,
                                        None,
                                        true,
                                        user_asked_to_open,
                                    );
                                }
                                ForeignVolumeWaitFollowUp::Quiet => {
                                    state.finish_unlock_slot(&wait_match.keys);
                                }
                            }
                        });
                    } else {
                        state.finish_unlock_slot(&waited.keys);
                        if !mount_error_is_cancelled(&error) {
                            show_error_dialog(
                                &state.overlay,
                                "Unable to mount volume",
                                &error.to_string(),
                            );
                        }
                    }
                }
            },
        );
    }

    fn start_password_drive(
        self: &Rc<Self>,
        drive: gio::Drive,
        credentials: Option<MountCredentials>,
        already_waited: bool,
        user_asked_to_open: bool,
    ) {
        let waited = DeviceMatch::from_drive(&drive);
        if credentials.is_none() && !already_waited && !self.begin_unlock_progress(&waited.keys) {
            return;
        }
        self.mount_target(
            MountTarget::Drive(drive.clone()),
            credentials,
            move |state, result, attempted, details| {
                if !result
                    .as_ref()
                    .err()
                    .is_some_and(volume_error_is_in_flight_mount)
                {
                    state.dismiss_unlock_progress(&waited.keys);
                }
                let this_mounted = successor_kind(&waited) == VolumeSuccessorKind::Mounted;
                if this_mounted {
                    state.open_unlocked_identity(None, &waited, user_asked_to_open);
                } else if mount_result_is_ok(&result) {
                    let weak = Rc::downgrade(state);
                    let waited = waited.clone();
                    glib::MainContext::default().spawn_local(async move {
                        let (_send, receive) = oneshot::channel();
                        wait_for_mount_change(
                            receive,
                            || {
                                successor_volume(&waited).is_some()
                                    || successor_mount_location(&waited).is_some()
                            },
                            FOREIGN_VOLUME_MOUNT_WAIT,
                        )
                        .await;
                        let Some(state) = weak.upgrade() else {
                            return;
                        };
                        if successor_mount_location(&waited).is_some() {
                            state.open_unlocked_identity(None, &waited, user_asked_to_open);
                        } else if successor_volume(&waited).is_some() {
                            state.start_owned_successor(&waited, user_asked_to_open);
                        } else {
                            state.finish_unlock_slot(&waited.keys);
                            show_error_dialog(
                                &state.overlay,
                                "Unable to mount volume",
                                "The device started, but no mountable volume appeared.",
                            );
                        }
                    });
                } else if let Err(error) = result {
                    if volume_error_is_authentication_failure(&error)
                        && let Some(details) = details
                    {
                        let weak = Rc::downgrade(state);
                        let retry_drive = drive.clone();
                        state.show_unlock_retry_prompt(
                            waited.keys.clone(),
                            attempted,
                            details,
                            move |credentials| {
                                if let Some(state) = weak.upgrade() {
                                    state.start_password_drive(
                                        retry_drive.clone(),
                                        Some(credentials),
                                        false,
                                        user_asked_to_open,
                                    );
                                }
                            },
                        );
                    } else if volume_error_is_in_flight_mount(&error) {
                        tracing::debug!(
                            drive = %drive.name(),
                            already_waited,
                            "waiting for in-flight drive start"
                        );
                        let weak = Rc::downgrade(state);
                        let wait_drive = drive.clone();
                        let wait_match = waited.clone();
                        glib::MainContext::default().spawn_local(async move {
                            let Some(state) = weak.upgrade() else {
                                return;
                            };
                            let _activity = BrowserView {
                                state: state.clone(),
                            }
                            .begin_global_activity("Connecting…");
                            let outcome =
                                wait_for_foreign_drive_start(&wait_drive, &wait_match).await;
                            drop(_activity);
                            let successor = match outcome {
                                ForeignVolumeWaitOutcome::Mounted => VolumeSuccessorKind::Mounted,
                                ForeignVolumeWaitOutcome::StillLocked => {
                                    VolumeSuccessorKind::Locked
                                }
                                ForeignVolumeWaitOutcome::Gone => successor_kind(&wait_match),
                            };
                            match foreign_volume_wait_follow_up(outcome, already_waited, successor)
                            {
                                ForeignVolumeWaitFollowUp::Navigate => {
                                    state.open_unlocked_identity(
                                        None,
                                        &wait_match,
                                        user_asked_to_open,
                                    );
                                }
                                ForeignVolumeWaitFollowUp::StartOwnedMount
                                    if outcome == ForeignVolumeWaitOutcome::Gone =>
                                {
                                    state.start_owned_successor(&wait_match, user_asked_to_open);
                                }
                                ForeignVolumeWaitFollowUp::StartOwnedMount => {
                                    state.start_password_drive(
                                        wait_drive,
                                        None,
                                        true,
                                        user_asked_to_open,
                                    );
                                }
                                ForeignVolumeWaitFollowUp::Quiet => {
                                    state.finish_unlock_slot(&wait_match.keys);
                                }
                            }
                        });
                    } else {
                        state.finish_unlock_slot(&waited.keys);
                        if !mount_error_is_cancelled(&error) {
                            show_error_dialog(
                                &state.overlay,
                                "Unable to mount volume",
                                &error.to_string(),
                            );
                        }
                    }
                }
            },
        );
    }

    pub(super) fn begin_location_edit(&self) {
        self.location_stack.set_visible_child_name("entry");
        self.location_entry.grab_focus();
        self.location_entry.select_region(0, -1);
        self.path_completion
            .refresh(&self.location_entry, &self.browser);
    }

    pub(super) fn cancel_location_edit(&self) {
        self.path_completion.dismiss();
        // Switch away from the entry before resetting its text: the change
        // signal re-opens the completion popover while it is still active.
        self.location_stack.set_visible_child_name("breadcrumbs");
        self.restore_location_text();
        self.browser.focus_active();
    }

    pub(super) fn submit_location(self: &Rc<Self>) {
        self.path_completion.dismiss();
        let input = self.location_entry.text();
        let (input, credentials) = match credentials_from_location_input(input.as_str()) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.location_stack.set_visible_child_name("breadcrumbs");
                self.restore_location_text();
                show_error_dialog(&self.overlay, "Unable to open location", &error.to_string());
                return;
            }
        };
        if credentials.is_some() {
            self.location_entry.set_text(&input);
        }
        self.pending_location_credentials.replace(credentials);
        match self.browser.navigate_input(&input) {
            Ok(()) => {
                self.location_stack.set_visible_child_name("breadcrumbs");
                self.browser.focus_active();
            }
            Err(LocationValidationError::NotMounted(location)) => {
                let credentials = self.pending_location_credentials.take();
                self.mount_then_navigate_with_credentials(
                    location,
                    MountStrategy::EnclosingVolume,
                    credentials,
                );
            }
            Err(LocationValidationError::Mountable(location)) => {
                let credentials = self.pending_location_credentials.take();
                self.mount_then_navigate_with_credentials(
                    location,
                    MountStrategy::Mountable,
                    credentials,
                );
            }
            Err(error) => {
                self.pending_location_credentials.take();
                self.location_stack.set_visible_child_name("breadcrumbs");
                self.restore_location_text();
                show_error_dialog(&self.overlay, "Unable to open location", &error.to_string());
            }
        }
    }

    pub(super) fn handle_navigation_rejected(
        self: &Rc<Self>,
        parent_depth: usize,
        error: LocationValidationError,
    ) {
        match error {
            LocationValidationError::NotMounted(location) => {
                self.mount_then_descend(parent_depth, location, MountStrategy::EnclosingVolume);
            }
            LocationValidationError::Mountable(location) => {
                self.mount_then_descend(parent_depth, location, MountStrategy::Mountable);
            }
            error => {
                show_error_dialog(
                    &self.overlay,
                    "Unable to open directory",
                    &error.to_string(),
                );
            }
        }
    }

    pub(super) fn mount_then_navigate_with_credentials(
        self: &Rc<Self>,
        location: Location,
        strategy: MountStrategy,
        credentials: Option<MountCredentials>,
    ) {
        self.mount_location(
            location.clone(),
            strategy,
            credentials,
            move |state, result, attempted_credentials, prompt_details| {
                if mount_result_is_ok(&result) {
                    state.browser.navigate(location.clone());
                    state.location_stack.set_visible_child_name("breadcrumbs");
                    state.browser.focus_active();
                } else if let Err(error) = result {
                    if mount_error_is_authentication_failure(&location, &error) {
                        state.prompt_to_retry_navigation(
                            location.clone(),
                            strategy,
                            attempted_credentials,
                            prompt_details,
                        );
                    } else {
                        state.location_stack.set_visible_child_name("breadcrumbs");
                        state.restore_location_text();
                        if let Some(message) = mount_failure_message(&location, &error) {
                            show_error_dialog(&state.overlay, "Unable to connect", &message);
                        }
                    }
                }
            },
        );
    }

    fn mount_then_descend(
        self: &Rc<Self>,
        parent_depth: usize,
        location: Location,
        strategy: MountStrategy,
    ) {
        self.mount_then_descend_with_credentials(parent_depth, location, strategy, None);
    }

    fn mount_then_descend_with_credentials(
        self: &Rc<Self>,
        parent_depth: usize,
        location: Location,
        strategy: MountStrategy,
        credentials: Option<MountCredentials>,
    ) {
        self.mount_location(
            location.clone(),
            strategy,
            credentials,
            move |state, result, attempted_credentials, prompt_details| {
                if mount_result_is_ok(&result) {
                    state.browser.descend(parent_depth, location.clone());
                } else if let Err(error) = result {
                    if mount_error_is_authentication_failure(&location, &error) {
                        state.prompt_to_retry_descend(
                            parent_depth,
                            location.clone(),
                            strategy,
                            attempted_credentials,
                            prompt_details,
                        );
                    } else if let Some(message) = mount_failure_message(&location, &error) {
                        show_error_dialog(&state.overlay, "Unable to connect", &message);
                    }
                }
            },
        );
    }

    fn prompt_to_retry_navigation(
        self: &Rc<Self>,
        location: Location,
        strategy: MountStrategy,
        previous_credentials: Option<MountCredentials>,
        prompt_details: Option<MountPromptDetails>,
    ) {
        let weak = Rc::downgrade(self);
        let cancel_weak = weak.clone();
        let prompt_location = location.clone();
        self.show_mount_retry_prompt(
            previous_credentials,
            prompt_details.unwrap_or_else(|| MountPromptDetails::fallback(&prompt_location)),
            move |credentials| {
                if let Some(state) = weak.upgrade() {
                    state.mount_then_navigate_with_credentials(
                        location.clone(),
                        strategy,
                        Some(credentials),
                    );
                }
            },
            move || {
                if let Some(state) = cancel_weak.upgrade() {
                    state.location_stack.set_visible_child_name("breadcrumbs");
                    state.restore_location_text();
                    state.browser.focus_active();
                }
            },
        );
    }

    fn prompt_to_retry_descend(
        self: &Rc<Self>,
        parent_depth: usize,
        location: Location,
        strategy: MountStrategy,
        previous_credentials: Option<MountCredentials>,
        prompt_details: Option<MountPromptDetails>,
    ) {
        let weak = Rc::downgrade(self);
        let prompt_location = location.clone();
        self.show_mount_retry_prompt(
            previous_credentials,
            prompt_details.unwrap_or_else(|| MountPromptDetails::fallback(&prompt_location)),
            move |credentials| {
                if let Some(state) = weak.upgrade() {
                    state.mount_then_descend_with_credentials(
                        parent_depth,
                        location.clone(),
                        strategy,
                        Some(credentials),
                    );
                }
            },
            || {},
        );
    }

    fn show_unlock_retry_prompt(
        self: &Rc<Self>,
        keys: DeviceKeys,
        previous_credentials: Option<MountCredentials>,
        details: MountPromptDetails,
        retry: impl Fn(MountCredentials) + 'static,
    ) {
        let weak = Rc::downgrade(self);
        self.show_mount_retry_prompt(previous_credentials, details, retry, move || {
            if let Some(state) = weak.upgrade() {
                state.finish_unlock_slot(&keys);
            }
        });
    }

    fn show_mount_retry_prompt(
        &self,
        previous_credentials: Option<MountCredentials>,
        details: MountPromptDetails,
        retry: impl Fn(MountCredentials) + 'static,
        cancelled: impl Fn() + 'static,
    ) {
        let authentication_failed = previous_credentials.is_some();
        let defaults = previous_credentials.unwrap_or_else(|| {
            let mut defaults = MountCredentials::default_for_prompt();
            if !details.default_user.is_empty() {
                defaults.username.clone_from(&details.default_user);
            }
            if !details.default_domain.is_empty() {
                defaults.domain.clone_from(&details.default_domain);
            }
            defaults
        });
        let _prompt = show_authentication_dialog(
            &self.overlay,
            None,
            &details.message,
            (&defaults.username, &defaults.domain),
            details.flags,
            authentication_failed,
            MountDialogHandlers {
                submitted: Some(Rc::new(retry)),
                cancelled: Some(Rc::new(cancelled)),
            },
        );
    }

    fn mount_location(
        self: &Rc<Self>,
        location: Location,
        strategy: MountStrategy,
        credentials: Option<MountCredentials>,
        on_result: impl Fn(
            &Rc<Self>,
            Result<(), glib::Error>,
            Option<MountCredentials>,
            Option<MountPromptDetails>,
        ) + 'static,
    ) {
        self.mount_target(
            MountTarget::Location(location, strategy),
            credentials,
            on_result,
        );
    }

    fn begin_unlock_progress(&self, keys: &DeviceKeys) -> bool {
        begin_unlock_slot(&mut self.unlock_slots.borrow_mut(), keys)
    }

    fn schedule_device_mount_chrome(
        self: &Rc<Self>,
        keys: &DeviceKeys,
        volume_name: &str,
        encrypted: bool,
    ) {
        if !encrypted {
            return;
        }
        self.schedule_unlock_progress(keys, volume_name);
    }

    fn schedule_unlock_progress(self: &Rc<Self>, keys: &DeviceKeys, volume_name: &str) {
        self.dismiss_unlock_progress(keys);
        if unlock_progress_dismissed_for(&self.unlock_slots.borrow(), keys) {
            return;
        }
        let weak = Rc::downgrade(self);
        let volume_name = volume_name.to_owned();
        let keys = keys.clone();
        let source = glib::timeout_add_local_once(UNLOCK_PROGRESS_DELAY, {
            let keys = keys.clone();
            move || {
                if let Some(state) = weak.upgrade() {
                    if let Some(slot) = state
                        .unlock_slots
                        .borrow_mut()
                        .iter_mut()
                        .find(|slot| unlock_target_matches(&slot.keys, &keys))
                    {
                        slot.pending = None;
                    }
                    state.present_unlock_progress(&keys, &volume_name);
                }
            }
        });
        if let Some(slot) = self
            .unlock_slots
            .borrow_mut()
            .iter_mut()
            .find(|slot| unlock_target_matches(&slot.keys, &keys))
        {
            if let Some(previous) = slot.pending.take() {
                previous.remove();
            }
            slot.pending = Some(source);
        } else {
            source.remove();
        }
    }

    fn present_unlock_progress(self: &Rc<Self>, keys: &DeviceKeys, volume_name: &str) {
        self.dismiss_other_unlock_views(keys);
        self.dismiss_unlock_progress(keys);
        if unlock_progress_dismissed_for(&self.unlock_slots.borrow(), keys) {
            return;
        }
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            return;
        };

        let layout = modal_layout(
            crate::assets::icons::LOCK,
            "Unlocking volume",
            volume_name,
            "Hide",
        );
        layout.content.add_css_class("compact");
        layout.set_loading(true, Some("Unlocking volume"));
        layout.cancel.set_visible(false);
        layout.body.append(&message_dialog_description(
            "You can hide this and keep working. Unlocking will continue in the background.",
        ));
        let content = layout.content;
        let close = layout.close;
        let hide = layout.confirm;

        let layer = modal_layer(
            &content,
            &window_overlay,
            blurred_root.clone(),
            Some(Rc::new(|| true)),
        );
        window_overlay.add_overlay(&layer);
        let view = UnlockProgressView {
            layer,
            overlay: window_overlay,
            blurred_root,
        };

        let weak = Rc::downgrade(self);
        let hide_keys = keys.clone();
        hide.connect_clicked({
            let weak = weak.clone();
            let hide_keys = hide_keys.clone();
            move |_| {
                if let Some(state) = weak.upgrade() {
                    state.hide_unlock_progress(&hide_keys);
                }
            }
        });
        close.connect_clicked({
            let weak = weak.clone();
            let hide_keys = hide_keys.clone();
            move |_| {
                if let Some(state) = weak.upgrade() {
                    state.hide_unlock_progress(&hide_keys);
                }
            }
        });
        let escape = gtk::EventControllerKey::new();
        escape.connect_key_pressed({
            let hide_keys = hide_keys.clone();
            move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    if let Some(state) = weak.upgrade() {
                        state.hide_unlock_progress(&hide_keys);
                    }
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        });
        view.layer.add_controller(escape);
        hide.grab_focus();
        if let Some(slot) = self
            .unlock_slots
            .borrow_mut()
            .iter_mut()
            .find(|slot| unlock_target_matches(&slot.keys, keys))
        {
            slot.view = Some(view);
        } else {
            dismiss_modal_layer(&view.layer, &view.overlay, view.blurred_root.as_ref());
        }
    }

    fn hide_unlock_progress(&self, keys: &DeviceKeys) {
        if let Some(slot) = self
            .unlock_slots
            .borrow_mut()
            .iter_mut()
            .find(|slot| unlock_target_matches(&slot.keys, keys))
        {
            slot.dismissed = true;
        }
        self.dismiss_unlock_progress(keys);
    }

    fn dismiss_other_unlock_views(&self, keys: &DeviceKeys) {
        let others: Vec<DeviceKeys> = self
            .unlock_slots
            .borrow()
            .iter()
            .filter(|slot| !unlock_target_matches(&slot.keys, keys))
            .map(|slot| slot.keys.clone())
            .collect();
        for other in others {
            self.dismiss_unlock_progress(&other);
        }
    }

    fn dismiss_unlock_progress(&self, keys: &DeviceKeys) {
        let view = {
            let mut slots = self.unlock_slots.borrow_mut();
            let Some(slot) = slots
                .iter_mut()
                .find(|slot| unlock_target_matches(&slot.keys, keys))
            else {
                return;
            };
            if let Some(source) = slot.pending.take() {
                source.remove();
            }
            slot.view.take()
        };
        let Some(view) = view else {
            return;
        };
        dismiss_modal_layer(&view.layer, &view.overlay, view.blurred_root.as_ref());
    }

    fn finish_unlock_slot(&self, keys: &DeviceKeys) {
        self.dismiss_unlock_progress(keys);
        self.unlock_slots
            .borrow_mut()
            .retain(|slot| !unlock_target_matches(&slot.keys, keys));
    }

    fn mount_target(
        self: &Rc<Self>,
        target: MountTarget,
        credentials: Option<MountCredentials>,
        on_result: impl Fn(
            &Rc<Self>,
            Result<(), glib::Error>,
            Option<MountCredentials>,
            Option<MountPromptDetails>,
        ) + 'static,
    ) {
        let Some(window) = self.overlay.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let (unlock_name, unlock_keys, encrypted) = match &target {
            MountTarget::Volume(volume) => (
                Some(volume.name().to_string()),
                DeviceMatch::from_volume(volume).keys,
                gio_volume_is_encrypted(volume),
            ),
            MountTarget::Drive(drive) => (
                Some(drive.name().to_string()),
                DeviceMatch::from_drive(drive).keys,
                true,
            ),
            MountTarget::Location(_, _) => (None, DeviceKeys::new([], []), false),
        };
        if let Some(name) = unlock_name.as_deref() {
            self.schedule_device_mount_chrome(&unlock_keys, name, encrypted);
        }
        let activity = BrowserView {
            state: self.clone(),
        }
        .begin_global_activity("Connecting…");
        // A native gtk::MountOperation (rather than a bare gio::MountOperation)
        // is required so GTK's own "ask-question" dialog handles host-key and
        // certificate trust decisions for us; we only override "ask-password"
        // below with Strata's own dialog, stopping that one signal's default
        // handler so the two don't both try to reply.
        let operation = gtk::MountOperation::new(Some(&window));
        let prompt_overlay = self.overlay.clone();
        let active_prompt = Rc::new(RefCell::new(None::<gtk::Box>));
        let prompt_for_signal = active_prompt.clone();
        let prompt_details = Rc::new(RefCell::new(None::<MountPromptDetails>));
        let details_for_signal = prompt_details.clone();
        let attempted_credentials = Rc::new(RefCell::new(credentials.clone()));
        let attempts_for_signal = attempted_credentials.clone();
        let supplied_credentials = Rc::new(RefCell::new(credentials));
        let credentials_for_signal = supplied_credentials.clone();
        let already_prompted = Cell::new(credentials_for_signal.borrow().is_some());
        let progress_state = Rc::downgrade(self);
        let progress_name = unlock_name;
        let progress_keys = unlock_keys;
        let progress_encrypted = encrypted;
        operation.connect_ask_password(
            move |operation, message, default_user, default_domain, flags| {
                // Suppress GtkMountOperation's own native password dialog: we
                // reply ourselves (immediately or via our custom prompt)
                // below. "ask-question" is deliberately left unconnected so
                // its native default handler still runs for host-key/cert
                // trust prompts.
                operation.stop_signal_emission_by_name("ask-password");
                details_for_signal.replace(Some(MountPromptDetails {
                    message: message.to_owned(),
                    default_user: default_user.to_owned(),
                    default_domain: default_domain.to_owned(),
                    flags,
                }));
                if let Some(credentials) = credentials_for_signal.borrow_mut().take() {
                    apply_mount_credentials(operation.upcast_ref(), &credentials);
                    operation.reply(gio::MountOperationResult::Handled);
                    return;
                }
                if let Some(state) = progress_state.upgrade() {
                    state.dismiss_unlock_progress(&progress_keys);
                }
                if let Some(previous) = prompt_for_signal.borrow_mut().take() {
                    dismiss_authentication_prompt(&prompt_overlay, &previous);
                }
                let retry = already_prompted.replace(true);
                let prompt = show_authentication_dialog(
                    &prompt_overlay,
                    Some(operation.upcast_ref()),
                    message,
                    (default_user, default_domain),
                    flags,
                    retry,
                    MountDialogHandlers {
                        submitted: Some(Rc::new({
                            let attempts_for_signal = attempts_for_signal.clone();
                            let progress_state = progress_state.clone();
                            let progress_name = progress_name.clone();
                            let progress_keys = progress_keys.clone();
                            move |credentials| {
                                attempts_for_signal.replace(Some(credentials));
                                if let (Some(state), Some(name)) =
                                    (progress_state.upgrade(), progress_name.as_ref())
                                    && progress_encrypted
                                {
                                    state.present_unlock_progress(&progress_keys, name);
                                }
                            }
                        })),
                        cancelled: None,
                    },
                );
                prompt_for_signal.replace(prompt);
            },
        );
        let weak = Rc::downgrade(self);
        let result_overlay = self.overlay.clone();
        glib::MainContext::default().spawn_local(async move {
            let _activity = activity;
            let result = match target {
                MountTarget::Volume(volume) => {
                    volume
                        .mount_future(gio::MountMountFlags::NONE, Some(&operation))
                        .await
                }
                MountTarget::Drive(drive) => {
                    drive
                        .start_future(gio::DriveStartFlags::NONE, Some(&operation))
                        .await
                }
                MountTarget::Location(location, strategy) => {
                    let file = gio_file_for_location(&location);
                    match strategy {
                        MountStrategy::EnclosingVolume => {
                            file.mount_enclosing_volume_future(
                                gio::MountMountFlags::NONE,
                                Some(&operation),
                            )
                            .await
                        }
                        MountStrategy::Mountable => file
                            .mount_mountable_future(gio::MountMountFlags::NONE, Some(&operation))
                            .await
                            .map(|_| ()),
                    }
                }
            };
            if let Some(prompt) = active_prompt.borrow_mut().take() {
                dismiss_authentication_prompt(&result_overlay, &prompt);
            }
            if let Some(state) = weak.upgrade() {
                on_result(
                    &state,
                    result,
                    attempted_credentials.borrow().clone(),
                    prompt_details.borrow().clone(),
                );
            }
        });
    }

    fn restore_location_text(&self) {
        if let Some(location) = self.browser.active_location() {
            self.location_entry.set_text(&location.display_path());
        }
    }

    pub(super) fn sync_active_location(self: &Rc<Self>) {
        if let Some(location) = self.browser.active_location() {
            self.set_location(&location);
        }
    }

    pub(super) fn set_location(self: &Rc<Self>, location: &Location) {
        while let Some(child) = self.breadcrumbs.first_child() {
            self.breadcrumbs.remove(&child);
        }

        let home = Location::local(glib::home_dir());
        let mut locations = location.breadcrumbs();
        if let Some(home_index) = locations.iter().position(|crumb| crumb == &home) {
            locations.drain(..home_index);
        }
        let starts_at_root = locations
            .first()
            .and_then(Location::native_path)
            .is_some_and(|path| path == Path::new("/"));
        let last = locations.len().saturating_sub(1);
        for (index, crumb) in locations.into_iter().enumerate() {
            if index > 0 && !(starts_at_root && index == 1) {
                let separator = gtk::Label::new(Some("/"));
                separator.add_css_class("breadcrumb-separator");
                self.breadcrumbs.append(&separator);
            }

            let label = if crumb == home {
                "~".to_owned()
            } else {
                crumb.display_name()
            };
            if index == last {
                let current = gtk::Box::new(gtk::Orientation::Horizontal, 2);
                current.add_css_class("current-breadcrumb");
                let current_label = gtk::Label::new(Some(&label));
                current_label.add_css_class("breadcrumb");
                current_label.add_css_class("current");
                ellipsize_crumb_label(&current_label);
                current_label.set_tooltip_text(Some(&crumb.display_path()));
                let copy = gtk::Button::builder().tooltip_text("Copy path").build();
                let copy_icon = crate::assets::primary_icon(crate::assets::icons::COPY, 16);
                copy.set_child(Some(&copy_icon));
                copy.add_css_class("copy-path");
                copy.set_has_frame(false);
                copy.set_cursor_from_name(Some("pointer"));
                let copied_path = copy_path_text(location, true);
                let feedback_generation = Rc::new(Cell::new(0_u64));
                copy.connect_clicked(move |button| {
                    if let Some(display) = gtk::gdk::Display::default() {
                        display.clipboard().set_text(&copied_path);
                    }
                    let generation = feedback_generation.get().saturating_add(1);
                    feedback_generation.set(generation);
                    crate::assets::set_primary_icon(&copy_icon, crate::assets::icons::CHECK);
                    button.set_tooltip_text(Some("Path copied"));
                    let button = button.clone();
                    let copy_icon = copy_icon.clone();
                    let feedback_generation = feedback_generation.clone();
                    glib::timeout_add_local_once(Duration::from_secs(2), move || {
                        if feedback_generation.get() == generation {
                            crate::assets::set_primary_icon(&copy_icon, crate::assets::icons::COPY);
                            button.set_tooltip_text(Some("Copy path"));
                        }
                    });
                });
                current.append(&current_label);
                current.append(&copy);
                self.breadcrumbs.append(&current);
            } else {
                let button = gtk::Button::with_label(&label);
                if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                    ellipsize_crumb_label(&label);
                }
                button.add_css_class("breadcrumb");
                if crumb
                    .native_path()
                    .is_some_and(|path| path == Path::new("/"))
                {
                    button.add_css_class("breadcrumb-root");
                }
                button.set_has_frame(false);
                button.set_tooltip_text(Some(&crumb.display_path()));
                button.set_cursor_from_name(Some("pointer"));
                let weak = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(state) = weak.upgrade() {
                        state.browser.navigate(crumb.clone());
                    }
                });
                self.breadcrumbs.append(&button);
            }
        }
        self.location_stack.set_visible_child_name("breadcrumbs");
        // Set the entry text only after the stack hides the entry: the change
        // signal re-opens the completion popover while it is still active.
        self.location_entry.set_text(&location.display_path());
        let Some(last) = self.breadcrumbs.last_child() else {
            return;
        };
        let last = last.downgrade();
        let _tick = self
            .breadcrumb_scroller
            .add_tick_callback(move |scroller, _| {
                let Some(last) = last.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                // The adjustment's upper bound is stale until the new crumbs are allocated.
                if last.width() <= 0 {
                    return glib::ControlFlow::Continue;
                }
                let adjustment = scroller.hadjustment();
                adjustment.set_value(adjustment.upper() - adjustment.page_size());
                glib::ControlFlow::Break
            });
    }

    pub(super) fn show_breadcrumb_hierarchy_menu(
        self: &Rc<Self>,
        anchor: &gtk::Widget,
        x: f64,
        y: f64,
    ) {
        let Some(active_location) = self.browser.active_location() else {
            return;
        };
        let mut locations = active_location.breadcrumbs();
        if locations.is_empty() {
            return;
        }
        let home = Location::local(glib::home_dir());
        if let Some(home_index) = locations.iter().position(|crumb| crumb == &home) {
            locations.drain(..home_index);
        }

        locations.reverse();

        let menu_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        menu_box.add_css_class("breadcrumb-hierarchy-menu");

        let popover = gtk::Popover::builder()
            .child(&menu_box)
            .has_arrow(true)
            .position(gtk::PositionType::Bottom)
            .pointing_to(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1))
            .build();
        popover.add_css_class("breadcrumb-popover");
        popover.set_parent(anchor);

        for (i, crumb) in locations.iter().enumerate() {
            let item_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let icon_name = if *crumb == home {
                crate::assets::icons::HOME
            } else if crumb
                .native_path()
                .is_some_and(|path| path == Path::new("/"))
            {
                crate::assets::icons::HARD_DRIVE
            } else if crumb.uri_value().is_some_and(|u| u.starts_with("trash://")) {
                crate::assets::icons::TRASH
            } else if crumb.uri_value().is_some() {
                crate::assets::icons::NETWORK
            } else {
                crate::assets::icons::FOLDER
            };

            let icon = crate::assets::primary_icon(icon_name, 16);
            let display_name = if *crumb == home {
                "~".to_owned()
            } else {
                crumb.display_name()
            };

            let label = gtk::Label::new(Some(&display_name));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            ellipsize_crumb_label(&label);

            item_row.append(&icon);
            item_row.append(&label);

            let button = gtk::Button::builder()
                .child(&item_row)
                .has_frame(false)
                .tooltip_text(crumb.display_path())
                .build();
            button.set_cursor_from_name(Some("pointer"));
            button.add_css_class("breadcrumb-hierarchy-item");
            if i == 0 {
                button.add_css_class("current");
            }

            let weak_self = Rc::downgrade(self);
            let weak_popover = popover.downgrade();
            let target_crumb = crumb.clone();
            button.connect_clicked(move |_| {
                if let Some(popover) = weak_popover.upgrade() {
                    popover.popdown();
                }
                if let Some(state) = weak_self.upgrade() {
                    state.browser.navigate(target_crumb.clone());
                }
            });

            menu_box.append(&button);
        }

        popover.connect_closed(move |popover| {
            popover.unparent();
        });

        popover.popup();
    }
}

#[cfg(test)]
mod tests;
