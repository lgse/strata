// SPDX-License-Identifier: MIT

pub(super) use crate::adapters::remote_mount::{MountCredentials, MountStrategy};
use crate::adapters::remote_mount::{
    MountPrompter, MountSession, PasswordReply, PasswordRequest, QuestionReply,
};
use crate::model::Location;
use crate::services::remote::{
    MountQuestion, MountResolution, RemoteDestination, RemoteErrorContext, RemoteFailure,
    RemoteProtocol, plaintext_destination, redact_endpoints,
};
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

    let passphrase = crate::adapters::remote_mount::requests_passphrase(message);
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
            if passphrase {
                "That passphrase wasn’t accepted. Check it, then try again."
            } else {
                "Those credentials weren’t accepted. Check the username, domain, and password, then try again."
            },
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
        append_authentication_field(
            &credentials,
            if passphrase { "Passphrase" } else { "Password" },
            &password,
        );
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
        remember_field.append(&form_label(if passphrase {
            "Passphrase storage"
        } else {
            "Password storage"
        }));
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
            credentials.apply_to(operation);
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

pub(super) enum TypedLocation {
    Navigating,
    /// `sanitized` is the input without its URI credentials.
    Mounting {
        sanitized: String,
    },
}

fn default_prompt_credentials() -> MountCredentials {
    MountCredentials {
        anonymous: false,
        username: glib::user_name().to_string_lossy().into_owned(),
        domain: "WORKGROUP".to_owned(),
        password: String::new(),
        save: gio::PasswordSave::Never,
    }
}

fn mount_result_is_ok(result: &Result<(), glib::Error>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => error.matches(gio::IOErrorEnum::AlreadyMounted),
    }
}

/// SMB and some other backends reject a password by failing the operation
/// instead of prompting again; Strata then asks once more itself.
fn mount_failure_needs_credentials(
    location: &Location,
    failure: RemoteFailure,
    result: &Result<(), glib::Error>,
) -> bool {
    location.uri_value().is_some()
        && failure == RemoteFailure::AuthenticationFailed
        && result.is_err()
}

/// The sanitized explanation for a failed connection, or `None` when the
/// user cancelled.
fn mount_failure_message(
    location: &Location,
    failure: RemoteFailure,
    result: &Result<(), glib::Error>,
) -> Option<String> {
    if failure == RemoteFailure::Cancelled {
        return None;
    }
    let Some(uri) = location.uri_value() else {
        return result.as_ref().err().map(ToString::to_string);
    };
    let protocol = RemoteProtocol::for_location(location);
    match failure {
        RemoteFailure::BackendMissing => Some(backend_unavailable_message(uri)),
        RemoteFailure::Other => Some(match result {
            Err(error) if protocol.is_some() => format!(
                "{}\n{}",
                RemoteFailure::Other.guidance(protocol),
                redact_endpoints(error.message())
            ),
            Err(error) => error.to_string(),
            Ok(()) => RemoteFailure::Other.guidance(protocol),
        }),
        failure => Some(failure.guidance(protocol)),
    }
}

fn mount_error_is_cancelled(error: &glib::Error) -> bool {
    error.matches(gio::IOErrorEnum::Cancelled) || error.matches(gio::IOErrorEnum::FailedHandled)
}

enum MountTarget {
    Location(Location, MountStrategy),
    Volume(gio::Volume),
    Drive(gio::Drive),
}

struct MountOutcome {
    result: Result<(), glib::Error>,
    resolution: MountResolution,
    credentials: Option<MountCredentials>,
    details: Option<MountPromptDetails>,
}

struct ViewPrompter {
    overlay: gtk::Overlay,
    active_prompt: Rc<RefCell<Option<gtk::Box>>>,
    state: std::rc::Weak<ViewState>,
    progress_name: Option<String>,
    progress_keys: DeviceKeys,
    progress_encrypted: bool,
}

impl ViewPrompter {
    fn replace_prompt(&self, prompt: Option<gtk::Box>) {
        if let Some(state) = self.state.upgrade() {
            state.dismiss_unlock_progress(&self.progress_keys);
        }
        if let Some(previous) = self.active_prompt.replace(prompt) {
            dismiss_authentication_prompt(&self.overlay, &previous);
        }
    }
}

impl MountPrompter for ViewPrompter {
    fn ask_password(&self, request: PasswordRequest, reply: PasswordReply) {
        self.replace_prompt(None);
        let reply = Rc::new(RefCell::new(Some(reply)));
        let cancel_reply = reply.clone();
        let progress_state = self.state.clone();
        let progress_name = self.progress_name.clone();
        let progress_keys = self.progress_keys.clone();
        let progress_encrypted = self.progress_encrypted;
        let prompt = show_authentication_dialog(
            &self.overlay,
            None,
            &request.message,
            (&request.default_user, &request.default_domain),
            request.flags,
            request.retry,
            MountDialogHandlers {
                submitted: Some(Rc::new(move |credentials| {
                    if let Some(reply) = reply.borrow_mut().take() {
                        reply.submit(credentials);
                    }
                    if let (Some(state), Some(name)) =
                        (progress_state.upgrade(), progress_name.as_ref())
                        && progress_encrypted
                    {
                        state.present_unlock_progress(&progress_keys, name);
                    }
                })),
                cancelled: Some(Rc::new(move || {
                    if let Some(reply) = cancel_reply.borrow_mut().take() {
                        reply.cancel();
                    }
                })),
            },
        );
        self.active_prompt.replace(prompt);
    }

    fn ask_question(&self, question: MountQuestion, reply: QuestionReply) {
        self.replace_prompt(None);
        let prompt = crate::ui::remote_prompts::show_mount_question(&self.overlay, question, reply);
        self.active_prompt.replace(prompt);
    }
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
    /// A mount session whose prompts use this view's Strata dialogs.
    pub(in crate::ui) fn mount_session(&self) -> MountSession {
        MountSession::new(
            gio::MountOperation::new(),
            None,
            Rc::new(ViewPrompter {
                overlay: self.state.overlay.clone(),
                active_prompt: Rc::default(),
                state: Rc::downgrade(&self.state),
                progress_name: None,
                progress_keys: DeviceKeys::new([], []),
                progress_encrypted: false,
            }),
        )
    }

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
            move |state,
                  MountOutcome {
                      result,
                      credentials: attempted,
                      details,
                      ..
                  }| {
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
            move |state,
                  MountOutcome {
                      result,
                      credentials: attempted,
                      details,
                      ..
                  }| {
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
        // Resetting a visible entry emits changed and reopens its completion popover.
        self.location_stack.set_visible_child_name("breadcrumbs");
        self.restore_location_text();
        self.browser.focus_active();
    }

    pub(super) fn submit_location(self: &Rc<Self>) {
        self.path_completion.dismiss();
        let input = self.location_entry.text();
        match self.open_typed_location(input.as_str(), None) {
            Ok(TypedLocation::Navigating) => {
                self.location_stack.set_visible_child_name("breadcrumbs");
                self.browser.focus_active();
            }
            Ok(TypedLocation::Mounting { sanitized }) => {
                if sanitized != input.as_str() {
                    self.location_entry.set_text(&sanitized);
                }
            }
            Err(error) => {
                self.location_stack.set_visible_child_name("breadcrumbs");
                self.restore_location_text();
                show_error_dialog(&self.overlay, "Unable to open location", &error.to_string());
            }
        }
    }

    /// Navigates to typed `input`, mounting first when the location needs it.
    /// Credentials embedded in a URI move into the mount operation and are
    /// never kept with the text. A relative path resolves against `base`.
    pub(super) fn open_typed_location(
        self: &Rc<Self>,
        input: &str,
        base: Option<&Path>,
    ) -> Result<TypedLocation, LocationValidationError> {
        let (input, credentials) = credentials_from_location_input(input)?;
        self.pending_location_credentials.replace(credentials);
        match self.browser.navigate_input_from(&input, base) {
            Ok(()) => Ok(TypedLocation::Navigating),
            Err(LocationValidationError::NotMounted(location)) => {
                let credentials = self.pending_location_credentials.take();
                self.mount_then_navigate_with_credentials(
                    location,
                    MountStrategy::EnclosingVolume,
                    credentials,
                );
                Ok(TypedLocation::Mounting { sanitized: input })
            }
            Err(LocationValidationError::Mountable(location)) => {
                let credentials = self.pending_location_credentials.take();
                self.mount_then_navigate_with_credentials(
                    location,
                    MountStrategy::Mountable,
                    credentials,
                );
                Ok(TypedLocation::Mounting { sanitized: input })
            }
            Err(error) => {
                self.pending_location_credentials.take();
                Err(error)
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
        let weak = Rc::downgrade(self);
        self.confirm_transport(&location.clone(), move |connect| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if !connect {
                state.restore_after_cancelled_connection();
                return;
            }
            state.mount_location(
                location.clone(),
                strategy,
                credentials.clone(),
                move |state, outcome| match outcome.resolution {
                    MountResolution::Succeeded => {
                        state.browser.navigate(location.clone());
                        state.location_stack.set_visible_child_name("breadcrumbs");
                        state.browser.focus_active();
                        crate::ui::connections::offer_to_save(&state.overlay, &location);
                    }
                    MountResolution::Cancelled => state.restore_after_cancelled_connection(),
                    MountResolution::Failed(failure) => {
                        if mount_failure_needs_credentials(&location, failure, &outcome.result) {
                            state.prompt_to_retry_navigation(
                                location.clone(),
                                strategy,
                                outcome.credentials,
                                outcome.details,
                            );
                        } else {
                            state.location_stack.set_visible_child_name("breadcrumbs");
                            state.restore_location_text();
                            state.report_connection_failure(&location, failure, &outcome.result);
                        }
                    }
                },
            );
        });
    }

    /// Cancelling a sign-in or trust decision returns to the prior committed
    /// location without adding history.
    fn restore_after_cancelled_connection(&self) {
        self.location_stack.set_visible_child_name("breadcrumbs");
        self.restore_location_text();
        self.browser.focus_active();
    }

    /// Plaintext protocols are confirmed once per server and session before a
    /// new connection is attempted; other locations continue immediately.
    fn confirm_transport(&self, location: &Location, on_decision: impl FnOnce(bool) + 'static) {
        match plaintext_destination(location) {
            Some(destination) => crate::ui::remote_prompts::confirm_plaintext_connection(
                &self.overlay,
                &destination,
                on_decision,
            ),
            None => on_decision(true),
        }
    }

    fn report_connection_failure(
        &self,
        location: &Location,
        failure: RemoteFailure,
        result: &Result<(), glib::Error>,
    ) {
        if let Some(message) = mount_failure_message(location, failure, result) {
            show_error_dialog(&self.overlay, failure.title(), &message);
        }
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
        let weak = Rc::downgrade(self);
        self.confirm_transport(&location.clone(), move |connect| {
            let Some(state) = weak.upgrade().filter(|_| connect) else {
                return;
            };
            state.mount_location(
                location.clone(),
                strategy,
                credentials.clone(),
                move |state, outcome| match outcome.resolution {
                    MountResolution::Succeeded => {
                        state.browser.descend(parent_depth, location.clone());
                        crate::ui::connections::offer_to_save(&state.overlay, &location);
                    }
                    MountResolution::Cancelled => {}
                    MountResolution::Failed(failure) => {
                        if mount_failure_needs_credentials(&location, failure, &outcome.result) {
                            state.prompt_to_retry_descend(
                                parent_depth,
                                location.clone(),
                                strategy,
                                outcome.credentials,
                                outcome.details,
                            );
                        } else {
                            state.report_connection_failure(&location, failure, &outcome.result);
                        }
                    }
                },
            );
        });
    }

    /// Reconnects a column whose remote mount went away, then reloads it.
    pub(super) fn reconnect_column(self: &Rc<Self>, depth: usize) {
        let Some(location) = self
            .browser
            .location_at(depth)
            .filter(|location| RemoteProtocol::for_location(location).is_some())
        else {
            self.browser.retry_column(depth);
            return;
        };
        let weak = Rc::downgrade(self);
        self.confirm_transport(&location.clone(), move |connect| {
            let Some(state) = weak.upgrade().filter(|_| connect) else {
                return;
            };
            state.mount_location(
                location.clone(),
                MountStrategy::EnclosingVolume,
                None,
                move |state, outcome| match outcome.resolution {
                    MountResolution::Succeeded => {
                        if state.browser.location_at(depth).as_ref() == Some(&location) {
                            state.browser.retry_column(depth);
                        }
                    }
                    MountResolution::Cancelled => {}
                    MountResolution::Failed(failure) => {
                        state.report_connection_failure(&location, failure, &outcome.result);
                    }
                },
            );
        });
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
            let mut defaults = default_prompt_credentials();
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
        on_result: impl Fn(&Rc<Self>, MountOutcome) + 'static,
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
        on_result: impl Fn(&Rc<Self>, MountOutcome) + 'static,
    ) {
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
        let active_prompt = Rc::new(RefCell::new(None::<gtk::Box>));
        // A plain GIO operation: every prompt, including backend trust
        // questions, is answered by Strata's own dialogs rather than GTK's.
        let session = MountSession::new(
            gio::MountOperation::new(),
            credentials,
            Rc::new(ViewPrompter {
                overlay: self.overlay.clone(),
                active_prompt: active_prompt.clone(),
                state: Rc::downgrade(self),
                progress_name: unlock_name,
                progress_keys: unlock_keys,
                progress_encrypted: encrypted,
            }),
        );
        let weak = Rc::downgrade(self);
        let result_overlay = self.overlay.clone();
        glib::MainContext::default().spawn_local(async move {
            let _activity = activity;
            let result = match &target {
                MountTarget::Volume(volume) => {
                    volume
                        .mount_future(gio::MountMountFlags::NONE, Some(session.operation()))
                        .await
                }
                MountTarget::Drive(drive) => {
                    drive
                        .start_future(gio::DriveStartFlags::NONE, Some(session.operation()))
                        .await
                }
                MountTarget::Location(location, strategy) => {
                    session.mount(location, *strategy).await
                }
            };
            if let Some(prompt) = active_prompt.borrow_mut().take() {
                dismiss_authentication_prompt(&result_overlay, &prompt);
            }
            let resolution = session.resolve(&result, RemoteErrorContext::Mount);
            let details = session
                .last_password_request()
                .map(|request| MountPromptDetails {
                    message: request.message,
                    default_user: request.default_user,
                    default_domain: request.default_domain,
                    flags: request.flags,
                });
            if let Some(state) = weak.upgrade() {
                on_result(
                    &state,
                    MountOutcome {
                        result,
                        resolution,
                        credentials: session.attempted_credentials(),
                        details,
                    },
                );
            }
        });
    }

    /// Keeps browsing columns in place when their remote mount disappears,
    /// leaving a retryable unavailable state instead of navigating away.
    pub(super) fn install_remote_disconnect_watch(self: &Rc<Self>) {
        let monitor = gio::VolumeMonitor::get();
        let weak = Rc::downgrade(self);
        let handler = monitor.connect_mount_removed(move |_, mount| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let Some(root) = RemoteDestination::parse(&mount.root().uri()) else {
                return;
            };
            state.browser.mark_unavailable(
                |location| {
                    RemoteDestination::for_location(location)
                        .is_some_and(|destination| destination.is_served_by(&root))
                },
                &RemoteFailure::Disconnected.guidance(None),
            );
        });
        let handler = RefCell::new(Some(handler));
        self.overlay.connect_destroy(move |_| {
            if let Some(handler) = handler.take() {
                monitor.disconnect(handler);
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
