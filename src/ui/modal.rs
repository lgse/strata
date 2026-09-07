// SPDX-License-Identifier: GPL-3.0-or-later

use crate::ui::blur::BlurBin;
use crate::ui::controls::{ModalTone, message_dialog_description, message_dialog_layout};
use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

#[cfg(test)]
mod tests;

pub(super) struct ModalHost {
    pub(super) overlay: gtk::Overlay,
    pub(super) blurred_root: Option<BlurBin>,
}

impl ModalHost {
    pub(super) fn blurred_for(parent: &impl IsA<gtk::Widget>) -> Option<Self> {
        let overlay = window_overlay(parent)?;
        let blurred_root = overlay.child().and_downcast::<BlurBin>();
        if let Some(root) = blurred_root.as_ref() {
            root.set_blurred(true);
        }
        Some(Self {
            overlay,
            blurred_root,
        })
    }
}

pub(super) fn window_overlay(parent: &impl IsA<gtk::Widget>) -> Option<gtk::Overlay> {
    parent
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.child())
        .and_downcast::<gtk::Overlay>()
}

#[expect(
    deprecated,
    reason = "GTK 4.12 deprecated translate_coordinates and allocation without a replacement for click-in-bounds checks"
)]
pub(super) fn modal_layer(
    content: &impl IsA<gtk::Widget>,
    overlay: &gtk::Overlay,
    root: Option<BlurBin>,
    block_dismiss: Option<Rc<dyn Fn() -> bool>>,
) -> gtk::Box {
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("app-modal-layer");
    layer.add_css_class("modal-backdrop");
    layer.set_halign(gtk::Align::Fill);
    layer.set_valign(gtk::Align::Fill);
    layer.set_hexpand(true);
    layer.set_vexpand(true);
    layer.set_focusable(true);
    let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
    top.set_vexpand(true);
    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bottom.set_vexpand(true);
    let left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    left.set_hexpand(true);
    let right = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    right.set_hexpand(true);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.append(&left);
    row.append(content);
    row.append(&right);
    layer.append(&top);
    layer.append(&row);
    layer.append(&bottom);

    let click = gtk::GestureClick::new();
    let weak_layer = layer.downgrade();
    let weak_content = content.downgrade();
    let overlay = overlay.clone();
    let root = root.clone();
    let block = block_dismiss.clone();
    click.connect_pressed(move |_, _, x, y| {
        if let Some(block) = block.as_ref()
            && block()
        {
            return;
        }
        let Some(layer) = weak_layer.upgrade() else {
            return;
        };
        let Some(content) = weak_content.upgrade() else {
            return;
        };
        let on_dialog = content
            .translate_coordinates(&layer, 0.0, 0.0)
            .is_some_and(|(cx, cy)| {
                let alloc = content.allocation();
                x >= cx
                    && x < cx + alloc.width() as f64
                    && y >= cy
                    && y < cy + alloc.height() as f64
            });
        if !on_dialog {
            dismiss_modal_layer(&layer, &overlay, root.as_ref());
        }
    });
    layer.add_controller(click);
    crate::ui::focus_navigation::install(&layer);
    animate_in(&layer);
    layer
}

pub(super) fn submit_on_enter(fields: &impl IsA<gtk::Widget>, confirm: &gtk::Button) {
    let mut child = fields.as_ref().first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(entry) = widget.downcast_ref::<gtk::Entry>() {
            let confirm = confirm.clone();
            entry.connect_activate(move |_| activate_primary(&confirm));
        } else if let Some(entry) = widget.downcast_ref::<gtk::PasswordEntry>() {
            let confirm = confirm.clone();
            entry.connect_activate(move |_| activate_primary(&confirm));
        } else {
            submit_on_enter(&widget, confirm);
        }
    }
}

fn activate_primary(confirm: &gtk::Button) {
    if confirm.is_sensitive() {
        confirm.emit_clicked();
    }
}

pub(super) fn animate_in(layer: &gtk::Box) {
    layer.remove_css_class("dismissing");
    layer.set_sensitive(true);
    layer.add_css_class("modal-hidden");
    let weak = layer.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(16), move || {
        if let Some(layer) = weak.upgrade() {
            layer.remove_css_class("modal-hidden");
        }
    });
}

pub(super) fn animate_out(layer: &gtk::Box, on_done: impl FnOnce() + 'static) {
    layer.add_css_class("modal-hidden");
    glib::timeout_add_local_once(Duration::from_millis(200), on_done);
}

pub(super) fn slide_out(widget: &impl IsA<gtk::Widget>) {
    let w = widget.as_ref();
    w.remove_css_class("slide-out");
    w.add_css_class("slide-out");
    let weak = w.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(240), move || {
        if let Some(w) = weak.upgrade() {
            w.remove_css_class("slide-out");
        }
    });
}

pub(super) fn slide_in_down(widget: &impl IsA<gtk::Widget>) {
    let w = widget.as_ref();
    w.remove_css_class("just-dropped");
    w.add_css_class("just-dropped");
    let weak = w.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(200), move || {
        if let Some(w) = weak.upgrade() {
            w.remove_css_class("just-dropped");
        }
    });
}

pub(super) fn dismiss_modal_layer(
    layer: &gtk::Box,
    overlay: &gtk::Overlay,
    root: Option<&BlurBin>,
) {
    if layer.has_css_class("dismissing") {
        return;
    }
    layer.add_css_class("dismissing");
    layer.set_sensitive(false);
    let overlay = overlay.clone();
    let layer_for_anim = layer.clone();
    let layer = layer.clone();
    let root = root.cloned();
    animate_out(&layer_for_anim, move || {
        overlay.remove_overlay(&layer);
        if let Some(root) = root
            && !overlay_has_modal_layer(&overlay)
        {
            root.set_blurred(false);
        }
    });
}

fn overlay_has_modal_layer(overlay: &gtk::Overlay) -> bool {
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        if widget.is_visible() && widget.has_css_class("app-modal-layer") {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

pub(super) fn show_error_dialog(parent: &impl IsA<gtk::Widget>, message: &str, detail: &str) {
    show_error_dialog_after_close(parent, message, detail, Rc::new(|| {}));
}

pub(super) fn show_error_dialog_after_close(
    parent: &impl IsA<gtk::Widget>,
    message: &str,
    detail: &str,
    on_close: Rc<dyn Fn()>,
) {
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        on_close();
        return;
    };

    let layout = message_dialog_layout(
        crate::assets::icons::X,
        message,
        if message == "Completed with errors" {
            "Some items could not be processed"
        } else {
            "The operation could not be completed"
        },
        "Close",
        ModalTone::Danger,
    );
    layout.cancel.set_visible(false);
    let explanation = message_dialog_description(detail);
    explanation.set_selectable(true);
    layout.body.append(&explanation);
    let content = layout.content;
    let close_icon = layout.close;
    let close = layout.confirm;

    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
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
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            dismiss();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);
    close.grab_focus();
}

/// Like [`show_error_dialog`], but for a `Completed with errors` delete
/// result where every failure was caused by the destination not supporting
/// Trash (issue #179): rather than a dead-end "Done" button, this offers an
/// actionable "Delete Permanently" button that invokes `on_retry` -- the
/// caller's job is to re-run the delete for just the retryable entries,
/// e.g. via `show_delete_confirmation(retryable_entries)`.
pub(super) fn show_delete_error_dialog(
    parent: &impl IsA<gtk::Widget>,
    detail: &str,
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
        "Completed with errors",
        "Some items could not be processed",
        "Delete Permanently",
        ModalTone::Danger,
    );
    layout.cancel.set_label("Done");
    let explanation = message_dialog_description(detail);
    explanation.set_selectable(true);
    layout.body.append(&explanation);
    let content = layout.content;
    let close_icon = layout.close;
    let cancel = layout.cancel;
    let confirm = layout.confirm;

    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
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
    let escape = gtk::EventControllerKey::new();
    let escape_dismiss = dismiss.clone();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            escape_dismiss();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);
    cancel.grab_focus();
}
