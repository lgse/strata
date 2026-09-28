// SPDX-License-Identifier: MIT

//! Strata-styled answers to remote backend prompts: trust and busy questions,
//! and the plaintext transport warning shown before a first connection.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::{
    adapters::remote_mount::QuestionReply,
    services::remote::{
        MountQuestion, MountQuestionKind, RemoteDestination, plaintext_secure_alternative,
        plaintext_warning_text,
    },
};

use super::{
    controls::{ModalTone, message_dialog_description, message_dialog_layout},
    modal::{ModalHost, dismiss_modal_layer, modal_layer},
};

thread_local! {
    /// Plaintext servers the user agreed to use during this session.
    static ACKNOWLEDGED_PLAINTEXT: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn question_icon(kind: MountQuestionKind) -> &'static str {
    match kind {
        MountQuestionKind::HostIdentity => crate::assets::icons::KEY,
        MountQuestionKind::Certificate => crate::assets::icons::SHIELD_ALERT,
        MountQuestionKind::Busy => crate::assets::icons::UNPLUG,
        MountQuestionKind::Other => crate::assets::icons::INFO,
    }
}

/// Shows a backend question with every backend choice. Dismissing the
/// dialog in any way other than a choice aborts the operation.
pub(in crate::ui) fn show_mount_question(
    parent: &impl IsA<gtk::Widget>,
    question: MountQuestion,
    reply: QuestionReply,
) -> Option<gtk::Box> {
    let ModalHost {
        overlay,
        blurred_root,
    } = ModalHost::blurred_for(parent)?;
    let risky = question.kind != MountQuestionKind::Other;
    let layout = message_dialog_layout(
        question_icon(question.kind),
        question.title(),
        question.subtitle(),
        "",
        if risky {
            ModalTone::Danger
        } else {
            ModalTone::Accent
        },
    );
    layout.content.add_css_class("mount-question");
    layout.cancel.set_visible(false);
    layout.confirm.set_visible(false);
    let detail = message_dialog_description(&question.detail);
    detail.set_selectable(true);
    detail.add_css_class("mount-question-detail");
    layout.body.append(&detail);

    let layer = modal_layer(&layout.content, &overlay, blurred_root.clone(), None);
    overlay.add_overlay(&layer);
    let reply = Rc::new(RefCell::new(Some(reply)));
    let finish = {
        let layer = layer.clone();
        let overlay = overlay.clone();
        let reply = reply.clone();
        Rc::new(move |choice: Option<usize>| {
            let Some(reply) = reply.borrow_mut().take() else {
                return;
            };
            dismiss_modal_layer(&layer, &overlay, blurred_root.as_ref());
            match choice {
                Some(index) => reply.choose(index),
                None => reply.cancel(),
            }
        })
    };

    let mut safe_choice = None;
    for (index, label) in question.choices.iter().enumerate() {
        let button = gtk::Button::with_label(label);
        if question.is_risky_choice(index) {
            button.add_css_class("action-dialog-confirm");
            button.add_css_class("danger");
        } else {
            button.add_css_class("action-dialog-cancel");
            safe_choice.get_or_insert_with(|| button.clone());
        }
        let finish = finish.clone();
        button.connect_clicked(move |_| finish(Some(index)));
        layout.actions.append(&button);
    }
    if question.choices.is_empty() {
        let close = gtk::Button::with_label("Cancel");
        close.add_css_class("action-dialog-cancel");
        let finish = finish.clone();
        close.connect_clicked(move |_| finish(None));
        layout.actions.append(&close);
        safe_choice = Some(close);
    }
    let close_finish = finish.clone();
    layout.close.connect_clicked(move |_| close_finish(None));
    // A backdrop click dismisses the layer without a choice.
    let dismissed = finish.clone();
    layer.connect_parent_notify(move |layer| {
        if layer.parent().is_none() {
            dismissed(None);
        }
    });
    install_escape(&layer, {
        let finish = finish.clone();
        move || finish(None)
    });
    if risky {
        if let Some(button) = safe_choice {
            button.grab_focus();
        }
    } else if let Some(first) = layout.actions.last_child() {
        first.grab_focus();
    }
    Some(layer)
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

pub(in crate::ui) fn plaintext_acknowledged(destination: &RemoteDestination) -> bool {
    ACKNOWLEDGED_PLAINTEXT
        .with(|acknowledged| acknowledged.borrow().contains(&destination.server_key()))
}

fn acknowledge_plaintext(destination: &RemoteDestination) {
    ACKNOWLEDGED_PLAINTEXT.with(|acknowledged| {
        acknowledged.borrow_mut().insert(destination.server_key());
    });
}

/// Asks before the first connection to an unencrypted server in this session.
/// `on_decision` receives whether the user chose to connect anyway.
pub(in crate::ui) fn confirm_plaintext_connection(
    parent: &impl IsA<gtk::Widget>,
    destination: &RemoteDestination,
    on_decision: impl FnOnce(bool) + 'static,
) {
    if plaintext_acknowledged(destination) {
        on_decision(true);
        return;
    }
    let Some(ModalHost {
        overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        on_decision(false);
        return;
    };
    let protocol = destination.protocol();
    let layout = message_dialog_layout(
        crate::assets::icons::TRIANGLE_ALERT,
        "Connect without encryption?",
        &format!("{} sends everything in plain text", protocol.label()),
        "Connect Anyway",
        ModalTone::Danger,
    );
    layout.content.add_css_class("plaintext-warning");
    layout
        .body
        .append(&message_dialog_description(&plaintext_warning_text(
            protocol,
        )));
    if let Some(secure) = plaintext_secure_alternative(protocol) {
        layout.body.append(&message_dialog_description(&format!(
            "If the server supports it, use {}:// instead.",
            secure.scheme()
        )));
    }
    let layer = modal_layer(&layout.content, &overlay, blurred_root.clone(), None);
    overlay.add_overlay(&layer);
    let decision = Rc::new(RefCell::new(Some(on_decision)));
    let decided = Rc::new(Cell::new(false));
    let destination = destination.clone();
    let finish = {
        let layer = layer.clone();
        Rc::new(move |connect: bool| {
            if decided.replace(true) {
                return;
            }
            if connect {
                acknowledge_plaintext(&destination);
            }
            dismiss_modal_layer(&layer, &overlay, blurred_root.as_ref());
            if let Some(on_decision) = decision.borrow_mut().take() {
                on_decision(connect);
            }
        })
    };
    let confirm = finish.clone();
    layout.confirm.connect_clicked(move |_| confirm(true));
    for button in [&layout.cancel, &layout.close] {
        let cancel = finish.clone();
        button.connect_clicked(move |_| cancel(false));
    }
    let dismissed = finish.clone();
    layer.connect_parent_notify(move |layer| {
        if layer.parent().is_none() {
            dismissed(false);
        }
    });
    install_escape(&layer, move || finish(false));
    layout.cancel.grab_focus();
}
