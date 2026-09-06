// SPDX-License-Identifier: GPL-3.0-or-later

use crate::adapters::gio_file_for_location;
use crate::model::Location;
use crate::services::{
    LocationValidationError, UriCredentials, backend_unavailable_message, sanitize_uri_credentials,
};
use crate::ui::blur::BlurBin;
use crate::ui::browser::clipboard::copy_path_text;
use crate::ui::browser::{BrowserView, ViewState};
use crate::ui::controls::{
    form_entry, form_label, form_password_entry, modal_layout, segmented_control, wrap_dialog_text,
};
use crate::ui::modal::{
    ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog, submit_on_enter,
};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

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
        "Sign in to access this network location",
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
    if error.matches(gio::IOErrorEnum::Cancelled) || error.matches(gio::IOErrorEnum::FailedHandled)
    {
        return None;
    }
    if error.matches(gio::IOErrorEnum::NotSupported) {
        return Some(backend_unavailable_message(
            location.uri_value().unwrap_or_default(),
        ));
    }
    Some(error.to_string())
}

impl ViewState {
    pub(super) fn begin_location_edit(&self) {
        self.location_stack.set_visible_child_name("entry");
        self.location_entry.grab_focus();
        self.location_entry.select_region(0, -1);
    }

    pub(super) fn cancel_location_edit(&self) {
        self.restore_location_text();
        self.location_stack.set_visible_child_name("breadcrumbs");
        self.browser.focus_active();
    }

    pub(super) fn submit_location(self: &Rc<Self>) {
        let input = self.location_entry.text();
        let (input, credentials) = match credentials_from_location_input(input.as_str()) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.restore_location_text();
                self.location_stack.set_visible_child_name("breadcrumbs");
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
                self.restore_location_text();
                self.location_stack.set_visible_child_name("breadcrumbs");
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
                        state.restore_location_text();
                        state.location_stack.set_visible_child_name("breadcrumbs");
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
            &prompt_location,
            previous_credentials,
            prompt_details,
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
                    state.restore_location_text();
                    state.location_stack.set_visible_child_name("breadcrumbs");
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
            &prompt_location,
            previous_credentials,
            prompt_details,
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

    fn show_mount_retry_prompt(
        &self,
        location: &Location,
        previous_credentials: Option<MountCredentials>,
        prompt_details: Option<MountPromptDetails>,
        retry: impl Fn(MountCredentials) + 'static,
        cancelled: impl Fn() + 'static,
    ) {
        let authentication_failed = previous_credentials.is_some();
        let details = prompt_details.unwrap_or_else(|| MountPromptDetails::fallback(location));
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
        let Some(window) = self.overlay.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let activity = BrowserView {
            state: self.clone(),
        }
        .begin_global_activity("Connecting…");
        let file = gio_file_for_location(&location);
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
                            move |credentials| {
                                attempts_for_signal.replace(Some(credentials));
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
            let result = match strategy {
                MountStrategy::EnclosingVolume => {
                    file.mount_enclosing_volume_future(gio::MountMountFlags::NONE, Some(&operation))
                        .await
                }
                MountStrategy::Mountable => file
                    .mount_mountable_future(gio::MountMountFlags::NONE, Some(&operation))
                    .await
                    .map(|_| ()),
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
        self.location_entry.set_text(&location.display_path());
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
    }
}

#[cfg(test)]
mod tests;
