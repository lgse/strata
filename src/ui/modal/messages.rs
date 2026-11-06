// SPDX-License-Identifier: MIT

use super::{ModalHost, close_on_escape, dismiss_modal_layer, modal_layer, remember_modal_focus};
use crate::ui::controls::{
    ModalLayout, ModalTone, focus_button, message_dialog_description, message_dialog_layout,
    stack_actions_when_constrained,
};
use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageKind {
    Error,
    PartialFailure,
    Information,
}

impl MessageKind {
    fn is_error(self) -> bool {
        self != Self::Information
    }

    fn icon(self) -> &'static str {
        if self.is_error() {
            crate::assets::icons::X
        } else {
            crate::assets::icons::INFO
        }
    }

    fn tone(self) -> ModalTone {
        if self.is_error() {
            ModalTone::Danger
        } else {
            ModalTone::Accent
        }
    }

    fn default_summary(self) -> String {
        crate::i18n::tr(match self {
            Self::Information => "Reported by the file provider",
            Self::PartialFailure => "Some items could not be processed",
            Self::Error => "The operation could not be completed",
        })
    }
}

/// The text of a one-button message dialog.
struct Message<'a> {
    title: &'a str,
    summary: Option<&'a str>,
    detail: &'a str,
    kind: MessageKind,
}

pub(in crate::ui) fn show_error_dialog(
    parent: &impl IsA<gtk::Widget>,
    message: &str,
    detail: &str,
) {
    show_error_dialog_after_close(parent, message, detail, Rc::new(|| {}));
}

/// Reports an operation that finished while some items failed.
pub(in crate::ui) fn show_partial_failure_dialog(parent: &impl IsA<gtk::Widget>, detail: &str) {
    let title = crate::i18n::tr("Completed with errors");
    Message {
        title: &title,
        summary: None,
        detail,
        kind: MessageKind::PartialFailure,
    }
    .show(parent, Rc::new(|| {}));
}

pub(in crate::ui) fn show_information_dialog(
    parent: &impl IsA<gtk::Widget>,
    message: &str,
    detail: &str,
) {
    Message::information(message, None, detail).show(parent, Rc::new(|| {}));
}

pub(in crate::ui) fn show_information_dialog_with_summary(
    parent: &impl IsA<gtk::Widget>,
    message: &str,
    summary: &str,
    detail: &str,
) {
    Message::information(message, Some(summary), detail).show(parent, Rc::new(|| {}));
}

pub(in crate::ui) fn show_error_dialog_after_close(
    parent: &impl IsA<gtk::Widget>,
    message: &str,
    detail: &str,
    on_close: Rc<dyn Fn()>,
) {
    Message::error(message, None, detail).show(parent, on_close);
}

/// [`show_error_dialog_after_close`] with `summary` under the title in place of
/// the generic "The operation could not be completed".
pub(in crate::ui) fn show_error_dialog_with_summary(
    parent: &impl IsA<gtk::Widget>,
    message: &str,
    summary: &str,
    detail: &str,
    on_close: Rc<dyn Fn()>,
) {
    Message::error(message, Some(summary), detail).show(parent, on_close);
}

impl<'a> Message<'a> {
    fn information(title: &'a str, summary: Option<&'a str>, detail: &'a str) -> Self {
        Self {
            title,
            summary,
            detail,
            kind: MessageKind::Information,
        }
    }

    fn error(title: &'a str, summary: Option<&'a str>, detail: &'a str) -> Self {
        Self {
            title,
            summary,
            detail,
            kind: MessageKind::Error,
        }
    }

    fn show(&self, parent: &impl IsA<gtk::Widget>, on_close: Rc<dyn Fn()>) {
        let Some(host) = message_host(parent) else {
            on_close();
            return;
        };
        present_message(host, self.layout(), on_close);
    }

    fn layout(&self) -> ModalLayout {
        let summary = self
            .summary
            .map_or_else(|| self.kind.default_summary(), str::to_owned);
        let layout = message_dialog_layout(
            self.kind.icon(),
            self.title,
            &summary,
            &crate::i18n::tr("Close"),
            self.kind.tone(),
        );
        layout.cancel.set_visible(false);
        append_detail(&layout, self.detail);
        layout
    }
}

fn message_host(parent: &impl IsA<gtk::Widget>) -> Option<ModalHost> {
    ModalHost::blurred_for(parent).or_else(|| {
        tracing::warn!(
            "No window modal host is available; reporting the error on the requesting overlay"
        );
        parent
            .as_ref()
            .clone()
            .downcast::<gtk::Overlay>()
            .ok()
            .map(|overlay| ModalHost {
                overlay,
                blurred_root: None,
            })
    })
}

fn append_detail(layout: &ModalLayout, detail: &str) {
    let explanation = message_dialog_description(detail);
    explanation.set_selectable(true);
    layout.body.append(&explanation);
}

fn present_message(host: ModalHost, layout: ModalLayout, on_close: Rc<dyn Fn()>) {
    let ModalHost {
        overlay: window_overlay,
        blurred_root,
    } = host;
    let content = layout.content;
    let close_icon = layout.close;
    let close = layout.confirm;

    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
    remember_modal_focus(&layer, &window_overlay);
    window_overlay.add_overlay(&layer);
    let close_layer = layer.clone();
    let close_overlay = window_overlay.clone();
    let close_root = blurred_root.clone();
    let dismissed = Rc::new(Cell::new(false));
    let dismiss = move || {
        if dismissed.replace(true) {
            return;
        }
        dismiss_modal_layer(&close_layer, &close_overlay, close_root.as_ref());
        let on_close = on_close.clone();
        glib::timeout_add_local_once(Duration::from_millis(250), move || on_close());
    };
    let dismiss = Rc::new(dismiss);
    let clicked_dismiss = dismiss.clone();
    close.connect_clicked(move |_| clicked_dismiss());
    let icon_dismiss = dismiss.clone();
    close_icon.connect_clicked(move |_| icon_dismiss());
    close_on_escape(&layer, move || dismiss());
    close.grab_focus();
}

/// An error dialog that also offers one way past the error, such as opening the
/// location as administrator.
pub(in crate::ui) fn show_error_dialog_with_action(
    parent: &impl IsA<gtk::Widget>,
    message: &str,
    detail: &str,
    action: (&str, &str),
    on_action: Rc<dyn Fn()>,
) {
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        return;
    };
    let (icon, label) = action;
    let layout = message_dialog_layout(
        icon,
        message,
        &crate::i18n::tr("The operation could not be completed"),
        label,
        ModalTone::Accent,
    );
    layout.cancel.set_label(&crate::i18n::tr("Close"));
    stack_actions_when_constrained(&layout);
    append_detail(&layout, detail);

    let layer = modal_layer(&layout.content, &window_overlay, blurred_root.clone(), None);
    remember_modal_focus(&layer, &window_overlay);
    window_overlay.add_overlay(&layer);
    let dismissed = Rc::new(Cell::new(false));
    let dismiss_layer = layer.clone();
    let dismiss_overlay = window_overlay.clone();
    let dismiss = Rc::new(move || {
        if dismissed.replace(true) {
            return;
        }
        dismiss_modal_layer(&dismiss_layer, &dismiss_overlay, blurred_root.as_ref());
    });
    let cancel_dismiss = dismiss.clone();
    layout.cancel.connect_clicked(move |_| cancel_dismiss());
    let icon_dismiss = dismiss.clone();
    layout.close.connect_clicked(move |_| icon_dismiss());
    let confirm_dismiss = dismiss.clone();
    layout.confirm.connect_clicked(move |_| {
        confirm_dismiss();
        on_action();
    });
    close_on_escape(&layer, move || dismiss());
    focus_button(&layout.confirm);
}

/// For a delete that completed with errors, some of them Trash-unsupported: offers
/// Delete Permanently for those entries via `on_retry` (#179).
pub(in crate::ui) fn show_delete_error_dialog(
    parent: &impl IsA<gtk::Widget>,
    detail: &str,
    retry_label: &str,
    on_retry: Rc<dyn Fn()>,
) {
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        return;
    };

    let layout = message_dialog_layout(
        crate::assets::icons::X,
        &crate::i18n::tr("Completed with errors"),
        &crate::i18n::tr("Some items could not be processed"),
        retry_label,
        ModalTone::Danger,
    );
    layout.cancel.set_label(&crate::i18n::tr("Done"));
    append_detail(&layout, detail);
    let content = layout.content;
    let close_icon = layout.close;
    let cancel = layout.cancel;
    let confirm = layout.confirm;

    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
    remember_modal_focus(&layer, &window_overlay);
    window_overlay.add_overlay(&layer);
    let dismissed = Rc::new(Cell::new(false));

    let dismiss_layer = layer.clone();
    let dismiss_overlay = window_overlay.clone();
    let dismiss_root = blurred_root.clone();
    let dismissed_for_dismiss = dismissed.clone();
    let dismiss = Rc::new(move || {
        if dismissed_for_dismiss.replace(true) {
            return;
        }
        dismiss_modal_layer(&dismiss_layer, &dismiss_overlay, dismiss_root.as_ref());
    });

    let clicked_dismiss = dismiss.clone();
    cancel.connect_clicked(move |_| clicked_dismiss());
    let icon_dismiss = dismiss.clone();
    close_icon.connect_clicked(move |_| icon_dismiss());
    let confirm_dismiss = dismiss.clone();
    confirm.connect_clicked(move |_| {
        confirm_dismiss();
        on_retry();
    });
    close_on_escape(&layer, move || dismiss());
    focus_button(&confirm);
}
