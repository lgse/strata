// SPDX-License-Identifier: MIT

use crate::ui::blur::BlurBin;
use crate::ui::keys::{self, bindings::ESCAPE_ANY};
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;
use std::time::Duration;

mod focus;
pub(super) mod layout;
mod messages;

#[cfg(test)]
mod tests;

use focus::track_removed_layer;
pub(in crate::ui) use focus::{focus_modal_layer, is_closing_layer};
pub(super) use focus::{
    remember_modal_focus, remember_modal_focus_for_listing, set_modal_focus_restore,
};
pub(crate) use focus::{remember_persistent_modal_focus, set_modal_focus_fallback};
pub(super) use messages::{
    show_delete_error_dialog, show_error_dialog, show_error_dialog_after_close,
    show_error_dialog_with_action, show_error_dialog_with_summary, show_information_dialog,
    show_information_dialog_with_summary, show_partial_failure_dialog,
};

pub(super) struct ModalHost {
    pub(super) overlay: gtk::Overlay,
    pub(super) blurred_root: Option<BlurBin>,
}

impl ModalHost {
    pub(super) fn for_widget(parent: &impl IsA<gtk::Widget>) -> Option<Self> {
        let overlay = window_overlay(parent)?;
        let blurred_root = overlay.child().and_downcast::<BlurBin>();
        Some(Self {
            overlay,
            blurred_root,
        })
    }

    pub(super) fn blurred_for(parent: &impl IsA<gtk::Widget>) -> Option<Self> {
        let host = Self::for_widget(parent)?;
        if let Some(root) = host.blurred_root.as_ref() {
            root.set_blurred(true);
        }
        Some(host)
    }
}

pub(super) fn window_overlay(parent: &impl IsA<gtk::Widget>) -> Option<gtk::Overlay> {
    parent
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.child())
        .and_downcast::<gtk::Overlay>()
}

pub(super) fn modal_layer(
    content: &impl IsA<gtk::Widget>,
    overlay: &gtk::Overlay,
    root: Option<BlurBin>,
    block_dismiss: Option<Rc<dyn Fn() -> bool>>,
) -> gtk::Box {
    let overlay = overlay.clone();
    modal_layer_with_backdrop(
        content,
        Rc::new(move |layer: &gtk::Box| {
            if block_dismiss.as_ref().is_some_and(|block| block()) {
                return;
            }
            dismiss_modal_layer(layer, &overlay, root.as_ref());
        }),
    )
}

#[expect(
    deprecated,
    reason = "GTK 4.12 deprecated translate_coordinates and allocation without a replacement for click-in-bounds checks"
)]
pub(super) fn modal_layer_with_backdrop(
    content: &impl IsA<gtk::Widget>,
    on_backdrop: Rc<dyn Fn(&gtk::Box)>,
) -> gtk::Box {
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("app-modal-layer");
    layer.add_css_class("modal-backdrop");
    layer.set_halign(gtk::Align::Fill);
    layer.set_valign(gtk::Align::Fill);
    layer.set_hexpand(true);
    layer.set_vexpand(true);
    layer.set_focusable(true);
    let viewport = layout::install(&layer, content);

    let click = gtk::GestureClick::new();
    let weak_layer = layer.downgrade();
    let weak_viewport = viewport.downgrade();
    let weak_content = content.as_ref().downgrade();
    click.connect_pressed(move |_, _, x, y| {
        let Some(layer) = weak_layer.upgrade() else {
            return;
        };
        let Some(content) = weak_content.upgrade() else {
            return;
        };
        let Some(viewport) = weak_viewport.upgrade() else {
            return;
        };
        let on_dialog = [content, viewport.upcast()].iter().all(|widget| {
            widget
                .translate_coordinates(&layer, 0.0, 0.0)
                .is_some_and(|(cx, cy)| {
                    let alloc = widget.allocation();
                    x >= cx
                        && x < cx + alloc.width() as f64
                        && y >= cy
                        && y < cy + alloc.height() as f64
                })
        });
        if !on_dialog {
            on_backdrop(&layer);
        }
    });
    layer.add_controller(click);
    crate::ui::focus_navigation::install(&layer);
    track_removed_layer(layer.upcast_ref());
    animate_in(&layer);
    layer
}

pub(super) fn close_on_escape(
    widget: &impl IsA<gtk::Widget>,
    close: impl Fn() + 'static,
) -> gtk::EventControllerKey {
    let escape = gtk::EventControllerKey::new();
    keys::on_press(&escape, move |_, press| {
        if !press.is(ESCAPE_ANY) {
            return glib::Propagation::Proceed;
        }
        close();
        glib::Propagation::Stop
    });
    widget.add_controller(escape.clone());
    escape
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
    flash_css_class(widget.as_ref(), "slide-out", 240);
}

pub(super) fn slide_in_down(widget: &impl IsA<gtk::Widget>) {
    flash_css_class(widget.as_ref(), "just-dropped", 200);
}

/// Restarts a CSS animation class and removes it once the animation ends.
fn flash_css_class(widget: &gtk::Widget, class: &'static str, millis: u64) {
    widget.remove_css_class(class);
    widget.add_css_class(class);
    let weak = widget.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(millis), move || {
        if let Some(widget) = weak.upgrade() {
            widget.remove_css_class(class);
        }
    });
}

pub(super) fn dismiss_modal_layer(
    layer: &gtk::Box,
    overlay: &gtk::Overlay,
    root: Option<&BlurBin>,
) {
    dismiss_modal_layer_then(layer, overlay, root, || {});
}

pub(super) fn dismiss_modal_layer_then(
    layer: &gtk::Box,
    overlay: &gtk::Overlay,
    root: Option<&BlurBin>,
    on_done: impl FnOnce() + 'static,
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
        on_done();
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
