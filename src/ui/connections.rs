// SPDX-License-Identifier: MIT

//! Saved remote connections: the add/edit form, sidebar actions, the
//! post-connect save offer, and capability-gated disconnects.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::remote_mount::{
        MountPrompter, MountSession, PasswordReply, PasswordRequest, QuestionReply,
    },
    model::Location,
    services::{
        connections::{
            ConnectionForm, ConnectionFormField, ConnectionStore, SaveError, SavedConnection,
            load_connection_store, update_connections,
        },
        remote::{
            MountQuestion, MountResolution, RemoteDestination, RemoteErrorContext, RemoteFailure,
            RemoteProtocol,
        },
    },
};

use super::{
    controls::{
        ModalTone, form_entry, form_error_label, form_label, message_dialog_description,
        message_dialog_layout, modal_layout, segmented_control, set_form_field_error,
    },
    modal::{ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog, submit_on_enter},
};

thread_local! {
    static LISTENERS: RefCell<Vec<Weak<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
    static SAVE_OFFER: RefCell<Option<(gtk::Overlay, gtk::Box)>> = const { RefCell::new(None) };
}

const SAVE_OFFER_TIMEOUT: Duration = Duration::from_secs(15);

/// Registers a callback for saved-connection changes in any window. The
/// callback stays registered while the returned handle is alive.
pub(in crate::ui) fn watch_connections(listener: impl Fn() + 'static) -> Rc<dyn Fn()> {
    let listener: Rc<dyn Fn()> = Rc::new(listener);
    LISTENERS.with(|listeners| {
        let mut listeners = listeners.borrow_mut();
        listeners.retain(|listener| listener.strong_count() > 0);
        listeners.push(Rc::downgrade(&listener));
    });
    listener
}

fn notify_connections_changed() {
    let listeners: Vec<_> = LISTENERS.with(|listeners| {
        listeners
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .collect()
    });
    for listener in listeners {
        listener();
    }
}

pub(in crate::ui) fn saved_connections() -> ConnectionStore {
    load_connection_store()
}

fn mount_destination(mount: &gio::Mount) -> Option<RemoteDestination> {
    RemoteDestination::parse(&mount.root().uri())
}

/// The live GVfs mount that already serves `connection`, if any.
pub(in crate::ui) fn matching_mount(
    connection: &SavedConnection,
) -> Option<(gio::Mount, RemoteDestination)> {
    gio::VolumeMonitor::get()
        .mounts()
        .into_iter()
        .filter(|mount| !mount.is_shadowed())
        .find_map(|mount| {
            let root = mount_destination(&mount)?;
            connection
                .destination()
                .is_served_by(&root)
                .then_some((mount, root))
        })
}

/// Whether a saved connection already represents this transient mount.
pub(in crate::ui) fn mount_is_saved(connections: &[SavedConnection], mount: &gio::Mount) -> bool {
    mount_destination(mount).is_some_and(|root| {
        connections
            .iter()
            .any(|connection| connection.destination().is_served_by(&root))
    })
}

/// Where selecting a connection should go: its destination on a matching
/// mount when one exists, so the existing session is reused.
pub(in crate::ui) fn connection_target(connection: &SavedConnection) -> Location {
    match matching_mount(connection) {
        Some((_, root)) => connection.destination().on_mount(&root).location(),
        None => connection.location(),
    }
}

pub(in crate::ui) fn connection_tooltip(connection: &SavedConnection, connected: bool) -> String {
    format!(
        "{} · {} · {}",
        connection.name,
        connection.protocol().label(),
        if connected {
            "Connected"
        } else {
            "Not connected"
        }
    )
}

fn save_error_text(error: &SaveError) -> String {
    error.to_string()
}

#[derive(Clone)]
pub(in crate::ui) enum ConnectionEditor {
    Add(Option<ConnectionForm>),
    Edit(SavedConnection),
}

struct FormField {
    group: gtk::Box,
    label: gtk::Label,
    entry: gtk::Entry,
    error: gtk::Label,
}

impl FormField {
    fn new(label: &str, text: &str) -> Self {
        let group = gtk::Box::new(gtk::Orientation::Vertical, 5);
        let label = form_label(label);
        let entry = form_entry();
        entry.set_text(text);
        super::accessibility::set_label(&entry, &label.text());
        let error = form_error_label();
        group.append(&label);
        group.append(&entry);
        group.append(&error);
        Self {
            group,
            label,
            entry,
            error,
        }
    }

    fn set_label(&self, text: &str) {
        self.label.set_text(text);
        super::accessibility::set_label(&self.entry, text);
    }

    fn text(&self) -> String {
        self.entry.text().to_string()
    }

    fn set_error(&self, message: Option<&str>) {
        set_form_field_error(&self.entry, &self.error, message);
    }
}

/// Opens the Add or Edit connection form. The form never asks for a password;
/// GVfs prompts for one when connecting.
pub(in crate::ui) fn show_connection_editor(
    parent: &impl IsA<gtk::Widget>,
    editor: ConnectionEditor,
) {
    let Some(ModalHost {
        overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        return;
    };
    let (title, confirm_label, initial, editing) = match &editor {
        ConnectionEditor::Add(form) => (
            "Add connection",
            "Save",
            form.clone().unwrap_or(ConnectionForm {
                protocol: Some(RemoteProtocol::Smb),
                ..ConnectionForm::default()
            }),
            None,
        ),
        ConnectionEditor::Edit(connection) => (
            "Edit connection",
            "Save",
            ConnectionForm::from_connection(connection),
            Some(connection.id.clone()),
        ),
    };
    let layout = modal_layout(
        crate::assets::icons::SERVER,
        title,
        "Reconnect to a server from the sidebar",
        confirm_label,
    );
    layout.content.add_css_class("wide");
    layout.content.add_css_class("connection-editor");

    let protocols = RemoteProtocol::ALL;
    let labels: Vec<&str> = protocols.iter().map(|protocol| protocol.label()).collect();
    let selected = initial
        .protocol
        .and_then(|protocol| {
            protocols
                .iter()
                .position(|candidate| *candidate == protocol)
        })
        .unwrap_or_default();
    let (protocol_control, protocol_buttons) = segmented_control(&labels, selected);
    for (button, protocol) in protocol_buttons.iter().zip(protocols) {
        button.set_tooltip_text(Some(protocol.description()));
        super::accessibility::set_label(
            button,
            &format!("{} ({})", protocol.label(), protocol.description()),
        );
    }
    let protocol_group = gtk::Box::new(gtk::Orientation::Vertical, 7);
    protocol_group.append(&form_label("Protocol"));
    protocol_group.append(&protocol_control);
    let transport_note = gtk::Label::new(None);
    transport_note.add_css_class("connection-transport-note");
    transport_note.set_wrap(true);
    transport_note.set_xalign(0.0);
    protocol_group.append(&transport_note);
    layout.body.append(&protocol_group);

    let server = FormField::new("Server", &initial.server);
    let port = FormField::new("Port (optional)", &initial.port);
    port.entry.set_input_purpose(gtk::InputPurpose::Digits);
    port.entry.set_max_width_chars(7);
    port.entry.set_width_chars(7);
    let address = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    server.group.set_hexpand(true);
    address.append(&server.group);
    address.append(&port.group);
    layout.body.append(&address);

    let username = FormField::new("Username (optional)", &initial.username);
    let share = FormField::new("Share", &initial.share);
    let path = FormField::new("Folder (optional)", &initial.path);
    let name = FormField::new("Name (optional)", &initial.name);
    for field in [&username, &share, &path, &name] {
        layout.body.append(&field.group);
    }
    let general_error = form_error_label();
    layout.body.append(&general_error);
    if let Some(reason) = saved_connections().read_only_reason() {
        general_error.set_text(&reason.to_string());
        general_error.set_visible(true);
        layout.confirm.set_sensitive(false);
    }

    let fields = Rc::new([server, port, username, share, path, name]);
    let [server, _, _, share, _, name] = &*fields;
    let current_protocol = {
        let buttons = protocol_buttons.clone();
        move || {
            buttons
                .iter()
                .position(gtk::ToggleButton::is_active)
                .map(|index| protocols[index])
        }
    };
    let read_form = {
        let fields = fields.clone();
        let current_protocol = current_protocol.clone();
        move || ConnectionForm {
            protocol: current_protocol(),
            server: fields[0].text(),
            port: fields[1].text(),
            username: fields[2].text(),
            share: fields[3].text(),
            path: fields[4].text(),
            name: fields[5].text(),
        }
    };

    let sync = {
        let fields = fields.clone();
        let transport_note = transport_note.clone();
        let read_form = read_form.clone();
        let current_protocol = current_protocol.clone();
        Rc::new(move || {
            let protocol = current_protocol().unwrap_or(RemoteProtocol::Smb);
            let smb = protocol == RemoteProtocol::Smb;
            fields[3].group.set_visible(smb);
            fields[4].set_label(match protocol {
                RemoteProtocol::Smb => "Folder (optional)",
                RemoteProtocol::Dav | RemoteProtocol::Davs => "Path (optional)",
                _ => "Remote path (optional)",
            });
            fields[1]
                .entry
                .set_placeholder_text(Some(&protocol.default_port().to_string()));
            fields[0].entry.set_placeholder_text(Some(match protocol {
                RemoteProtocol::Dav | RemoteProtocol::Davs => "server or https://server/path",
                _ => "server.example.com",
            }));
            transport_note.set_text(&if protocol.is_plaintext() {
                format!(
                    "{} isn't encrypted. Passwords and files can be read on the network.",
                    protocol.label()
                )
            } else {
                protocol.description().to_owned()
            });
            if protocol.is_plaintext() {
                transport_note.add_css_class("warning");
            } else {
                transport_note.remove_css_class("warning");
            }
            let placeholder = read_form()
                .validate()
                .map(|draft| draft.destination.default_name())
                .unwrap_or_default();
            fields[5].entry.set_placeholder_text(Some(&placeholder));
        })
    };
    for button in &protocol_buttons {
        let sync = sync.clone();
        button.connect_toggled(move |button| {
            if button.is_active() {
                sync();
            }
        });
    }
    for field in fields.iter() {
        let sync = sync.clone();
        let error = field.error.clone();
        let entry = field.entry.clone();
        field.entry.connect_changed(move |_| {
            set_form_field_error(&entry, &error, None);
            sync();
        });
    }
    sync();

    let layer = modal_layer(&layout.content, &overlay, blurred_root.clone(), None);
    overlay.add_overlay(&layer);
    let dismiss = {
        let layer = layer.clone();
        let overlay = overlay.clone();
        let blurred_root = blurred_root.clone();
        Rc::new(move || dismiss_modal_layer(&layer, &overlay, blurred_root.as_ref()))
    };
    for button in [&layout.cancel, &layout.close] {
        let dismiss = dismiss.clone();
        button.connect_clicked(move |_| dismiss());
    }
    let save_fields = fields.clone();
    let save_dismiss = dismiss.clone();
    layout.confirm.connect_clicked(move |_| {
        general_error.set_visible(false);
        let draft = match read_form().validate() {
            Ok(draft) => draft,
            Err(error) => {
                let field = match error.field {
                    ConnectionFormField::Server => &save_fields[0],
                    ConnectionFormField::Port => &save_fields[1],
                    ConnectionFormField::Username => &save_fields[2],
                    ConnectionFormField::Share => &save_fields[3],
                    ConnectionFormField::Path => &save_fields[4],
                    ConnectionFormField::Name => &save_fields[5],
                };
                field.set_error(Some(&error.message));
                field.entry.grab_focus();
                return;
            }
        };
        let result = update_connections(|store| match editing.as_deref() {
            Some(id) => store.update(id, draft),
            None => store.add(draft),
        });
        match result {
            Ok(_) => {
                save_dismiss();
                notify_connections_changed();
            }
            Err(error) => {
                general_error.set_text(&save_error_text(&error));
                general_error.set_visible(true);
            }
        }
    });
    submit_on_enter(&layout.body, &layout.confirm);
    install_escape(&layer, move || dismiss());
    if server.text().is_empty() {
        server.entry.grab_focus();
    } else if share.group.is_visible() && share.text().is_empty() {
        share.entry.grab_focus();
    } else {
        name.entry.grab_focus();
    }
}

fn install_escape(layer: &gtk::Box, on_escape: impl Fn() + 'static) {
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            on_escape();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);
}

/// Changes only the label Strata shows; the destination is untouched.
pub(in crate::ui) fn show_rename_connection(
    parent: &impl IsA<gtk::Widget>,
    connection: SavedConnection,
) {
    let Some(ModalHost {
        overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        return;
    };
    let layout = modal_layout(
        crate::assets::icons::PENCIL,
        "Rename connection",
        "Only the name in Strata's sidebar changes",
        "Rename",
    );
    let field = FormField::new("Name", &connection.name);
    layout.body.append(&field.group);
    let layer = modal_layer(&layout.content, &overlay, blurred_root.clone(), None);
    overlay.add_overlay(&layer);
    let dismiss = {
        let layer = layer.clone();
        Rc::new(move || dismiss_modal_layer(&layer, &overlay, blurred_root.as_ref()))
    };
    for button in [&layout.cancel, &layout.close] {
        let dismiss = dismiss.clone();
        button.connect_clicked(move |_| dismiss());
    }
    let entry = field.entry.clone();
    let error = field.error.clone();
    let rename_dismiss = dismiss.clone();
    layout.confirm.connect_clicked(move |_| {
        let name = entry.text().to_string();
        match update_connections(|store| store.rename(&connection.id, &name)) {
            Ok(_) => {
                rename_dismiss();
                notify_connections_changed();
            }
            Err(failure) => set_form_field_error(&entry, &error, Some(&save_error_text(&failure))),
        }
    });
    submit_on_enter(&layout.body, &layout.confirm);
    install_escape(&layer, move || dismiss());
    field.entry.grab_focus();
    field.entry.select_region(0, -1);
}

/// Removes only Strata's record. Files on the server and any password the
/// desktop keyring saved are left alone; a live mount is offered separately.
pub(in crate::ui) fn confirm_remove_connection(
    parent: &impl IsA<gtk::Widget>,
    connection: SavedConnection,
) {
    let parent = parent.as_ref().clone();
    let removal_parent = parent.clone();
    confirm(
        &parent,
        crate::assets::icons::TRASH,
        &format!("Remove “{}”?", connection.name),
        "Only the sidebar connection is removed",
        "Files on the server and passwords saved in your keyring aren't changed.",
        "Remove",
        move || match update_connections(|store| store.remove(&connection.id)) {
            Ok(removed) => {
                notify_connections_changed();
                if let Some((mount, _)) =
                    matching_mount(&removed).filter(|(mount, _)| mount.can_unmount())
                {
                    offer_disconnect_after_remove(&removal_parent, removed, mount);
                }
            }
            Err(error) => {
                show_error_dialog(
                    &removal_parent,
                    "Unable to remove connection",
                    &save_error_text(&error),
                );
            }
        },
    );
}

fn offer_disconnect_after_remove(
    parent: &gtk::Widget,
    removed: SavedConnection,
    mount: gio::Mount,
) {
    let disconnect_parent = parent.clone();
    confirm(
        parent,
        crate::assets::icons::UNPLUG,
        &format!("Disconnect from “{}”?", removed.name),
        "The connection was removed but is still open",
        "Disconnecting closes it now. Leave it open to keep browsing until you disconnect later.",
        "Disconnect",
        move || disconnect_mount(&disconnect_parent, mount.clone(), || {}),
    );
}

fn confirm(
    parent: &gtk::Widget,
    icon: &str,
    title: &str,
    subtitle: &str,
    detail: &str,
    confirm_label: &str,
    on_confirm: impl Fn() + 'static,
) {
    let Some(ModalHost {
        overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        return;
    };
    let layout = message_dialog_layout(icon, title, subtitle, confirm_label, ModalTone::Danger);
    layout.body.append(&message_dialog_description(detail));
    let layer = modal_layer(&layout.content, &overlay, blurred_root.clone(), None);
    overlay.add_overlay(&layer);
    let dismiss = {
        let layer = layer.clone();
        Rc::new(move || dismiss_modal_layer(&layer, &overlay, blurred_root.as_ref()))
    };
    for button in [&layout.cancel, &layout.close] {
        let dismiss = dismiss.clone();
        button.connect_clicked(move |_| dismiss());
    }
    let confirm_dismiss = dismiss.clone();
    layout.confirm.connect_clicked(move |_| {
        confirm_dismiss();
        on_confirm();
    });
    install_escape(&layer, move || dismiss());
    layout.cancel.grab_focus();
}

struct DisconnectPrompter {
    parent: gtk::Widget,
}

impl MountPrompter for DisconnectPrompter {
    fn ask_password(&self, _request: PasswordRequest, reply: PasswordReply) {
        reply.cancel();
    }

    fn ask_question(&self, question: MountQuestion, reply: QuestionReply) {
        super::remote_prompts::show_mount_question(&self.parent, question, reply);
    }
}

/// Unmounts a remote mount, asking before closing files other applications
/// still use, and reports busy or failed disconnects.
pub(in crate::ui) fn disconnect_mount(
    parent: &impl IsA<gtk::Widget>,
    mount: gio::Mount,
    on_finished: impl FnOnce() + 'static,
) {
    if !mount.can_unmount() {
        return;
    }
    let parent = parent.as_ref().clone();
    let session = MountSession::new(
        gio::MountOperation::new(),
        None,
        Rc::new(DisconnectPrompter {
            parent: parent.clone(),
        }),
    );
    let protocol = mount_destination(&mount).map(|root| root.protocol());
    glib::MainContext::default().spawn_local(async move {
        let result = session.unmount(&mount).await;
        match session.resolve(&result, RemoteErrorContext::Unmount) {
            MountResolution::Succeeded | MountResolution::Cancelled => {}
            MountResolution::Failed(RemoteFailure::Other) => {
                let detail = result
                    .err()
                    .map(|error| crate::services::remote::redact_endpoints(&error.to_string()))
                    .unwrap_or_default();
                show_error_dialog(&parent, "Unable to disconnect", &detail);
            }
            MountResolution::Failed(failure) => {
                show_error_dialog(&parent, "Unable to disconnect", &failure.guidance(protocol));
            }
        }
        on_finished();
    });
}

/// Offers to save a destination the user just connected to directly. It is
/// never saved without the user choosing to.
pub(in crate::ui) fn offer_to_save(overlay: &gtk::Overlay, location: &Location) {
    let Some(form) = ConnectionForm::from_location(location) else {
        return;
    };
    if saved_connections().containing(location).is_some() {
        return;
    }
    dismiss_save_offer();
    let banner = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    banner.add_css_class("connection-save-offer");
    banner.set_halign(gtk::Align::Center);
    banner.set_valign(gtk::Align::End);
    banner.set_margin_bottom(18);
    banner.append(&crate::assets::primary_icon(
        crate::assets::icons::SERVER,
        16,
    ));
    let text = gtk::Label::new(Some(
        "Connected. Save this server to reopen it from the sidebar.",
    ));
    text.set_wrap(true);
    banner.append(&text);
    let save = gtk::Button::with_label("Save Connection…");
    save.add_css_class("action-dialog-confirm");
    let dismiss = gtk::Button::new();
    dismiss.add_css_class("action-dialog-close");
    dismiss.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::X,
        14,
    )));
    dismiss.set_tooltip_text(Some("Dismiss"));
    super::accessibility::set_label(&dismiss, "Dismiss");
    banner.append(&save);
    banner.append(&dismiss);
    overlay.add_overlay(&banner);
    SAVE_OFFER.with(|offer| offer.replace(Some((overlay.clone(), banner.clone()))));

    let parent = overlay.clone();
    save.connect_clicked(move |_| {
        dismiss_save_offer();
        show_connection_editor(&parent, ConnectionEditor::Add(Some(form.clone())));
    });
    dismiss.connect_clicked(|_| dismiss_save_offer());
    let shown = banner.downgrade();
    let hovered = Rc::new(Cell::new(false));
    let motion = gtk::EventControllerMotion::new();
    {
        let hovered = hovered.clone();
        motion.connect_enter(move |_, _, _| hovered.set(true));
    }
    {
        let hovered = hovered.clone();
        motion.connect_leave(move |_| hovered.set(false));
    }
    banner.add_controller(motion);
    glib::timeout_add_local_once(SAVE_OFFER_TIMEOUT, move || {
        let Some(banner) = shown.upgrade() else {
            return;
        };
        if !hovered.get() && banner.focus_child().is_none() {
            SAVE_OFFER.with(|offer| {
                let mut offer = offer.borrow_mut();
                if offer.as_ref().is_some_and(|(_, shown)| shown == &banner)
                    && let Some((overlay, banner)) = offer.take()
                {
                    overlay.remove_overlay(&banner);
                }
            });
        }
    });
}

pub(in crate::ui) fn dismiss_save_offer() {
    if let Some((overlay, banner)) = SAVE_OFFER.with(|offer| offer.borrow_mut().take())
        && banner.parent().is_some()
    {
        overlay.remove_overlay(&banner);
    }
}

#[cfg(test)]
mod tests;
