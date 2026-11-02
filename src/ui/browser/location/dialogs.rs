// SPDX-License-Identifier: MIT

use crate::model::Location;
use crate::services::sanitize_failure_message;
use crate::ui::blur::BlurBin;
use crate::ui::controls::{
    ModalLayout, ModalTone, dialog_text, focus_button, form_entry, form_label, form_password_entry,
    keep_words_whole, message_dialog_description, message_dialog_layout, modal_layout,
    segmented_control,
};
use crate::ui::modal::{
    ModalHost, close_on_escape, dismiss_modal_layer, modal_layer, submit_on_enter,
};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::Cell;
use std::rc::Rc;

/// A presented location dialog's layer and the overlay it was added to.
pub(super) struct LocationModal {
    pub(super) layer: gtk::Box,
    pub(super) overlay: gtk::Overlay,
    pub(super) blurred_root: Option<BlurBin>,
}

impl LocationModal {
    /// Adds `content` to the window overlay; `block_dismiss` keeps backdrop clicks from closing it.
    pub(super) fn present(
        content: &gtk::Box,
        host: ModalHost,
        block_dismiss: Rc<dyn Fn() -> bool>,
    ) -> Self {
        let layer = modal_layer(
            content,
            &host.overlay,
            host.blurred_root.clone(),
            Some(block_dismiss),
        );
        host.overlay.add_overlay(&layer);
        Self {
            layer,
            overlay: host.overlay,
            blurred_root: host.blurred_root,
        }
    }

    pub(super) fn dismiss(&self) {
        dismiss_modal_layer(&self.layer, &self.overlay, self.blurred_root.as_ref());
    }

    fn share(&self) -> Self {
        Self {
            layer: self.layer.clone(),
            overlay: self.overlay.clone(),
            blurred_root: self.blurred_root.clone(),
        }
    }
}

type MountCredentialsHandler = Rc<dyn Fn(MountCredentials)>;

type MountCancelledHandler = Rc<dyn Fn()>;

pub(super) struct MountDialogHandlers {
    pub(super) submitted: Option<MountCredentialsHandler>,
    pub(super) cancelled: Option<MountCancelledHandler>,
}

#[derive(Clone)]
pub(super) struct MountPromptDetails {
    pub(super) message: String,
    pub(super) default_user: String,
    pub(super) default_domain: String,
    pub(super) flags: gio::AskPasswordFlags,
}

impl MountPromptDetails {
    pub(super) fn fallback(location: &Location) -> Self {
        Self {
            message: rust_i18n::t!(
                "Enter your credentials for “%{value1}”.",
                value1 = location.display_path()
            )
            .into_owned(),
            default_user: String::new(),
            default_domain: String::new(),
            flags: fallback_ask_password_flags(location),
        }
    }
}

fn fallback_ask_password_flags(location: &Location) -> gio::AskPasswordFlags {
    let flags = gio::AskPasswordFlags::NEED_USERNAME
        | gio::AskPasswordFlags::NEED_PASSWORD
        | gio::AskPasswordFlags::SAVING_SUPPORTED;
    if location
        .uri_value()
        .is_some_and(|uri| uri.starts_with("smb://"))
    {
        flags | gio::AskPasswordFlags::NEED_DOMAIN | gio::AskPasswordFlags::ANONYMOUS_SUPPORTED
    } else {
        flags
    }
}

fn password_field_label(message: &str) -> &'static str {
    if message.to_ascii_lowercase().contains("passphrase") {
        "Passphrase"
    } else {
        "Password"
    }
}

pub(super) fn authentication_retry_message(flags: gio::AskPasswordFlags, message: &str) -> String {
    let mut fields = Vec::new();
    if flags.contains(gio::AskPasswordFlags::NEED_USERNAME) {
        fields.push("username");
    }
    if flags.contains(gio::AskPasswordFlags::NEED_DOMAIN) {
        fields.push("domain");
    }
    if flags.contains(gio::AskPasswordFlags::NEED_PASSWORD) {
        fields.push(if password_field_label(message) == "Passphrase" {
            "passphrase"
        } else {
            "password"
        });
    }
    if fields.is_empty() {
        crate::i18n::tr("That attempt wasn’t accepted. Try again.")
    } else {
        rust_i18n::t!(
            "Those credentials weren’t accepted. Check %{value1} and try again.",
            value1 = crate::i18n::list(fields.into_iter().map(crate::i18n::tr))
        )
        .into_owned()
    }
}

const AUTHENTICATION_TEXT_WIDTH_CHARS: i32 = 64;

fn is_rejection_choice(choice: &str) -> bool {
    let choice = choice.to_ascii_lowercase();
    [
        "cancel",
        "reject",
        "deny",
        "disconnect",
        "abort",
        "do not",
        "don't",
        "no",
    ]
    .iter()
    .any(|word| choice == *word || choice.starts_with(&format!("{word} ")))
}

fn trust_question_layout(message: &str) -> ModalLayout {
    let layout = message_dialog_layout(
        crate::assets::icons::TRIANGLE_ALERT,
        &crate::i18n::tr("Verify server identity"),
        &crate::i18n::tr(
            "Only continue after verifying the SSH host key through a trusted source.",
        ),
        &crate::i18n::tr("Continue anyway"),
        ModalTone::Danger,
    );
    let backend_message = message_dialog_description(&sanitize_failure_message(message));
    layout.body.append(&backend_message);
    let warning = message_dialog_description(&crate::i18n::tr(
        "If the server did not provide a fingerprint, verify it outside Strata before continuing.",
    ));
    layout.body.append(&warning);
    layout.actions.remove(&layout.cancel);
    layout.actions.remove(&layout.confirm);
    layout
}

/// One answer to a trust question; rejections come first and record the refusal.
fn trust_choice_button(
    (index, choice): (usize, &String),
    modal: &LocationModal,
    operation: &gio::MountOperation,
    declined: &Rc<Cell<bool>>,
) -> gtk::Button {
    let button = gtk::Button::with_label(choice);
    let rejects_trust = is_rejection_choice(choice);
    if rejects_trust {
        button.add_css_class("action-dialog-cancel");
    } else {
        button.add_css_class("action-dialog-confirm");
        button.add_css_class("danger");
    }
    let modal = modal.share();
    let operation = operation.clone();
    let declined = declined.clone();
    button.connect_clicked(move |_| {
        if rejects_trust {
            declined.set(true);
        }
        modal.dismiss();
        operation.set_choice(index as i32);
        operation.reply(gio::MountOperationResult::Handled);
    });
    button
}

fn decline_trust_question(
    modal: &LocationModal,
    operation: &gio::MountOperation,
    declined: &Rc<Cell<bool>>,
) -> impl Fn() + 'static {
    let modal = modal.share();
    let operation = operation.clone();
    let declined = declined.clone();
    move || {
        declined.set(true);
        modal.dismiss();
        operation.reply(gio::MountOperationResult::Aborted);
    }
}

pub(super) fn show_trust_question_dialog(
    browser_overlay: &gtk::Overlay,
    operation: &gio::MountOperation,
    message: &str,
    choices: &[String],
    declined: Rc<Cell<bool>>,
) -> Option<gtk::Box> {
    if choices.is_empty() {
        operation.reply(gio::MountOperationResult::Unhandled);
        return None;
    }
    let Some(host) = ModalHost::blurred_for(browser_overlay) else {
        operation.reply(gio::MountOperationResult::Unhandled);
        return None;
    };

    let layout = trust_question_layout(message);
    let modal = LocationModal::present(&layout.content, host, Rc::new(|| true));
    let close = decline_trust_question(&modal, operation, &declined);
    layout.close.connect_clicked(move |_| close());

    let mut rejection = None;
    let mut ordered_choices: Vec<_> = choices.iter().enumerate().collect();
    ordered_choices.sort_by_key(|(_, choice)| !is_rejection_choice(choice));
    for (index, choice) in ordered_choices {
        let button = trust_choice_button((index, choice), &modal, operation, &declined);
        if is_rejection_choice(choice) {
            rejection.get_or_insert_with(|| button.clone());
        }
        layout.actions.append(&button);
    }
    focus_button(rejection.as_ref().unwrap_or(&layout.close));

    close_on_escape(
        &modal.layer,
        decline_trust_question(&modal, operation, &declined),
    );
    Some(modal.layer)
}

fn authentication_note(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(&dialog_text(text)));
    label.add_css_class(class);
    label.set_max_width_chars(AUTHENTICATION_TEXT_WIDTH_CHARS);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    keep_words_whole(&label);
    label.set_xalign(0.0);
    label
}

fn authentication_layout(
    message: &str,
    flags: gio::AskPasswordFlags,
    authentication_failed: bool,
) -> ModalLayout {
    let layout = modal_layout(
        crate::assets::icons::KEY,
        &crate::i18n::tr(if password_field_label(message) == "Passphrase" {
            "Passphrase required"
        } else {
            "Authentication required"
        }),
        &crate::i18n::tr("Authenticate to access this volume or location"),
        &crate::i18n::tr("Connect"),
    );
    layout.content.add_css_class("wide");
    layout.body.add_css_class("authentication-body");
    layout
        .body
        .append(&authentication_note(message, "authentication-explanation"));
    if authentication_failed {
        layout.body.append(&authentication_note(
            &authentication_retry_message(flags, message),
            "authentication-error",
        ));
    }
    layout
}

/// The credential inputs of an authentication prompt.
#[derive(Clone)]
struct AuthenticationFields {
    username: gtk::Entry,
    domain: gtk::Entry,
    password: gtk::PasswordEntry,
    anonymous: gtk::ToggleButton,
    remember: gtk::Widget,
    remember_buttons: Vec<gtk::ToggleButton>,
}

fn labelled_group(spacing: i32, label: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Vertical, spacing);
    group.append(&form_label(&crate::i18n::tr(label)));
    group.append(control);
    group
}

impl AuthenticationFields {
    fn build(
        body: &gtk::Box,
        message: &str,
        defaults: (&str, &str),
        flags: gio::AskPasswordFlags,
    ) -> Self {
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
            append_authentication_field(&credentials, password_field_label(message), &password);
        }
        let (connect_as_control, connect_as_buttons) =
            segmented_control(&["Registered user", "Anonymous"], 0);
        if flags.contains(gio::AskPasswordFlags::ANONYMOUS_SUPPORTED) {
            body.append(&labelled_group(7, "Connect as", &connect_as_control));
        }
        body.append(&credentials);
        let (remember, remember_buttons) =
            segmented_control(&["Don't remember", "Until logout", "Forever"], 0);
        if flags.contains(gio::AskPasswordFlags::SAVING_SUPPORTED) {
            body.append(&labelled_group(5, "Password storage", &remember));
        }
        Self {
            username,
            domain,
            password,
            anonymous: connect_as_buttons[1].clone(),
            remember: remember.upcast(),
            remember_buttons,
        }
    }

    fn disable_when_anonymous(&self) {
        let credential_widgets = [
            self.username.clone().upcast::<gtk::Widget>(),
            self.domain.clone().upcast(),
            self.password.clone().upcast(),
            self.remember.clone(),
        ];
        self.anonymous.connect_toggled(move |anonymous| {
            for widget in &credential_widgets {
                widget.set_sensitive(!anonymous.is_active());
            }
        });
    }

    /// Backdrop clicks must not discard anything typed.
    fn typed_anything(&self) -> Rc<dyn Fn() -> bool> {
        let (user, domain, password) = (
            self.username.clone(),
            self.domain.clone(),
            self.password.clone(),
        );
        Rc::new(move || {
            !user.text().is_empty() || !domain.text().is_empty() || !password.text().is_empty()
        })
    }

    fn credentials(&self) -> MountCredentials {
        let selected = self
            .remember_buttons
            .iter()
            .position(gtk::ToggleButton::is_active)
            .unwrap_or_default() as u32;
        MountCredentials {
            anonymous: self.anonymous.is_active(),
            username: self.username.text().to_string(),
            domain: self.domain.text().to_string(),
            password: self.password.text().to_string(),
            save: password_save_for_selection(selected),
        }
    }

    fn focus_first(&self, flags: gio::AskPasswordFlags, default_user: &str, connect: &gtk::Button) {
        if flags.contains(gio::AskPasswordFlags::NEED_USERNAME) && default_user.is_empty() {
            self.username.grab_focus();
        } else if flags.contains(gio::AskPasswordFlags::NEED_PASSWORD) {
            self.password.grab_focus();
        } else {
            connect.grab_focus();
        }
    }
}

/// Who hears that the prompt was abandoned: the mount operation, or else the caller.
#[derive(Clone)]
struct AuthenticationAbort {
    operation: Option<gio::MountOperation>,
    cancelled: Option<MountCancelledHandler>,
}

impl AuthenticationAbort {
    fn abort(&self) {
        if let Some(operation) = self.operation.as_ref() {
            operation.reply(gio::MountOperationResult::Aborted);
        } else if let Some(cancelled) = self.cancelled.as_ref() {
            cancelled();
        }
    }

    fn dismissing(&self, modal: &LocationModal) -> impl Fn() + 'static {
        let abort = self.clone();
        let modal = modal.share();
        move || {
            modal.dismiss();
            abort.abort();
        }
    }
}

fn connect_authentication_submit(
    connect: &gtk::Button,
    fields: AuthenticationFields,
    modal: LocationModal,
    (operation, submitted): (Option<gio::MountOperation>, Option<MountCredentialsHandler>),
) {
    connect.connect_clicked(move |_| {
        let credentials = fields.credentials();
        if let Some(operation) = operation.as_ref() {
            apply_mount_credentials(operation, &credentials);
        }
        modal.dismiss();
        if let Some(operation) = operation.as_ref() {
            operation.reply(gio::MountOperationResult::Handled);
        }
        if let Some(submitted) = submitted.as_ref() {
            submitted(credentials);
        }
    });
}

pub(super) fn show_authentication_dialog(
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
    let Some(host) = ModalHost::blurred_for(browser_overlay) else {
        if let Some(operation) = operation {
            operation.reply(gio::MountOperationResult::Unhandled);
        }
        return None;
    };

    let layout = authentication_layout(message, flags, authentication_failed);
    let fields = AuthenticationFields::build(&layout.body, message, defaults, flags);
    fields.disable_when_anonymous();
    let modal = LocationModal::present(&layout.content, host, fields.typed_anything());
    let abort = AuthenticationAbort {
        operation: operation.cloned(),
        cancelled,
    };
    for button in [&layout.cancel, &layout.close] {
        let dismiss = abort.dismissing(&modal);
        button.connect_clicked(move |_| dismiss());
    }
    let connect = layout.confirm;
    let layer = modal.layer.clone();
    let escape = abort.dismissing(&modal);
    connect_authentication_submit(
        &connect,
        fields.clone(),
        modal,
        (operation.cloned(), submitted),
    );
    submit_on_enter(&layout.body, &connect);
    close_on_escape(&layer, escape);
    fields.focus_first(flags, defaults.0, &connect);
    Some(layer)
}

pub(super) fn dismiss_authentication_prompt(browser_overlay: &gtk::Overlay, layer: &gtk::Box) {
    if layer.parent().is_none() {
        return;
    }
    let Some(window_overlay) = crate::ui::modal::window_overlay(browser_overlay) else {
        return;
    };
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    dismiss_modal_layer(layer, &window_overlay, blurred_root.as_ref());
}

fn append_authentication_field(
    fields: &gtk::Box,
    label_text: &str,
    field: &(impl IsA<gtk::Widget> + IsA<gtk::Accessible>),
) {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 5);
    let label = form_label(&crate::i18n::tr(label_text));
    crate::ui::accessibility::set_labelled_by(field, &[label.upcast_ref()]);
    group.append(&label);
    group.append(field);
    fields.append(&group);
}

pub(super) fn password_save_for_selection(selected: u32) -> gio::PasswordSave {
    match selected {
        1 => gio::PasswordSave::ForSession,
        2 => gio::PasswordSave::Permanently,
        _ => gio::PasswordSave::Never,
    }
}

#[derive(Clone)]
pub(in crate::ui::browser) struct MountCredentials {
    pub(super) anonymous: bool,
    pub(super) username: String,
    pub(super) domain: String,
    pub(super) password: String,
    pub(super) save: gio::PasswordSave,
}

impl MountCredentials {
    pub(super) fn default_for_prompt(flags: gio::AskPasswordFlags) -> Self {
        Self {
            anonymous: false,
            username: glib::user_name().to_string_lossy().into_owned(),
            domain: if flags.contains(gio::AskPasswordFlags::NEED_DOMAIN) {
                "WORKGROUP".to_owned()
            } else {
                String::new()
            },
            password: String::new(),
            save: gio::PasswordSave::Never,
        }
    }
}

pub(super) fn apply_mount_credentials(
    operation: &gio::MountOperation,
    credentials: &MountCredentials,
) {
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
