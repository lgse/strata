// SPDX-License-Identifier: MIT

use crate::ui::blur::BlurBin;
use crate::ui::controls::{
    ModalTone, focus_button, message_dialog_description, message_dialog_layout,
};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

pub(super) mod layout;

#[cfg(test)]
mod tests;

struct ModalFocusOrigin {
    widget: Option<glib::WeakRef<gtk::Widget>>,
    restore: RefCell<Option<Rc<dyn Fn()>>>,
    /// False when the first dialog of a chain changes the listing; dialogs chained on
    /// it inherit that and return focus through the window fallback too.
    allow_widget: bool,
}

type FocusOrigins = Vec<(glib::WeakRef<gtk::Widget>, Rc<ModalFocusOrigin>)>;
type FocusFallbacks = Vec<(glib::WeakRef<gtk::Window>, Rc<dyn Fn()>)>;

thread_local! {
    static MODAL_FOCUS_ORIGINS: RefCell<FocusOrigins> = const { RefCell::new(Vec::new()) };
    static MODAL_FOCUS_FALLBACKS: RefCell<FocusFallbacks> = const { RefCell::new(Vec::new()) };
}

/// Sets where focus goes when a dismissed overlay leaves `window` without a
/// focused widget, normally the browser's file view.
pub(crate) fn set_modal_focus_fallback(window: &gtk::Window, fallback: Rc<dyn Fn()>) {
    MODAL_FOCUS_FALLBACKS.with(|fallbacks| {
        let mut fallbacks = fallbacks.borrow_mut();
        fallbacks.retain(|(candidate, _)| {
            candidate
                .upgrade()
                .is_some_and(|candidate| candidate != *window)
        });
        fallbacks.push((window.downgrade(), fallback));
    });
}

fn modal_focus_fallback(window: &gtk::Window) -> Option<Rc<dyn Fn()>> {
    MODAL_FOCUS_FALLBACKS.with(|fallbacks| {
        let mut fallbacks = fallbacks.borrow_mut();
        fallbacks.retain(|(candidate, _)| candidate.upgrade().is_some());
        fallbacks
            .iter()
            .find(|(candidate, _)| {
                candidate
                    .upgrade()
                    .is_some_and(|candidate| candidate == *window)
            })
            .map(|(_, fallback)| fallback.clone())
    })
}

/// Records the window focus for `layer`, or the browser origin of the modal it is chained on.
fn register_focus_origin(
    layer: &gtk::Widget,
    window: Option<&gtk::Window>,
    allow_widget: bool,
) -> Rc<ModalFocusOrigin> {
    let previous = window.and_then(crate::ui::window::visible_modal_layer);
    MODAL_FOCUS_ORIGINS.with(|origins| {
        let mut origins = origins.borrow_mut();
        origins.retain(|(layer, _)| layer.upgrade().is_some());
        // Chained dialogs inherit the browser origin, not the preceding modal's focus.
        let origin = previous
            .and_then(|previous| {
                origins.iter().find_map(|(layer, origin)| {
                    layer
                        .upgrade()
                        .filter(|layer| *layer == previous)
                        .map(|_| origin.clone())
                })
            })
            .unwrap_or_else(|| {
                Rc::new(ModalFocusOrigin {
                    widget: window
                        .and_then(gtk::prelude::RootExt::focus)
                        .map(|focus| focus.downgrade()),
                    restore: RefCell::new(None),
                    allow_widget,
                })
            });
        origins.push((layer.downgrade(), origin.clone()));
        origin
    })
}

fn forget_focus_origin(layer: &gtk::Widget) {
    MODAL_FOCUS_ORIGINS.with(|origins| {
        origins.borrow_mut().retain(|(candidate, _)| {
            candidate
                .upgrade()
                .is_some_and(|candidate| candidate != *layer)
        });
    });
}

/// Priority: focus taken meanwhile, explicit restore, on-screen origin, window fallback.
fn restore_modal_focus(window: &gtk::Window, origin: &ModalFocusOrigin, allow_origin: bool) {
    if gtk::prelude::RootExt::focus(window).is_some_and(|focus| focus.is_mapped()) {
        return;
    }
    let restore = origin.restore.borrow().clone();
    if let Some(restore) = restore {
        restore();
        return;
    }
    if allow_origin
        && origin.allow_widget
        && let Some(widget) = origin.widget.as_ref().and_then(glib::WeakRef::upgrade)
        && shown_or_revealing(&widget)
        && widget.is_sensitive()
        && widget.root().as_ref() == Some(window.upcast_ref())
        && widget.grab_focus()
    {
        return;
    }
    if let Some(fallback) = modal_focus_fallback(window) {
        fallback();
    }
}

/// Whether `widget` is on screen or is mapped by the next frame: an opening revealer,
/// such as a just-shown filter field's, maps its child only on its first animation
/// tick, which a slow paint can delay until after an overlay opened over it closes.
fn shown_or_revealing(widget: &gtk::Widget) -> bool {
    let mut current = widget.clone();
    while !current.is_mapped() {
        let Some(parent) = current.parent() else {
            return false;
        };
        let revealing = parent
            .downcast_ref::<gtk::Revealer>()
            .is_some_and(gtk::Revealer::reveals_child);
        if !current.is_visible() || !(current.is_child_visible() || revealing) {
            return false;
        }
        current = parent;
    }
    true
}

pub(super) fn remember_modal_focus(layer: &gtk::Box, overlay: &gtk::Overlay) -> Rc<Cell<bool>> {
    remember_removed_modal_focus(layer, overlay, true)
}

/// [`remember_modal_focus`] for dialogs whose work changes the listing, such as
/// file-operation progress: closing it hands focus to the browser cursor through the
/// window fallback, never to the row widget that was focused when it opened.
pub(super) fn remember_modal_focus_for_listing(layer: &gtk::Box, overlay: &gtk::Overlay) {
    remember_removed_modal_focus(layer, overlay, false);
}

fn remember_removed_modal_focus(
    layer: &gtk::Box,
    overlay: &gtk::Overlay,
    allow_origin: bool,
) -> Rc<Cell<bool>> {
    let window = overlay.root().and_downcast::<gtk::Window>();
    let origin = register_focus_origin(layer.upcast_ref(), window.as_ref(), allow_origin);
    let overlay = overlay.downgrade();
    let restore = Rc::new(Cell::new(true));
    let restore_on_close = restore.clone();
    layer.connect_parent_notify(move |layer| {
        if layer.parent().is_some() {
            return;
        }
        forget_focus_origin(layer.upcast_ref());
        if !layer.has_css_class("dismissing") || !restore_on_close.get() {
            return;
        }
        // The focus trap releases the browser only after the final modal is removed.
        let Some(window) = overlay
            .upgrade()
            .and_then(|overlay| overlay.root())
            .and_downcast::<gtk::Window>()
        else {
            return;
        };
        if crate::ui::window::visible_modal_layer(&window).is_some() {
            return;
        }
        restore_modal_focus(&window, &origin, allow_origin);
    });
    restore
}

/// [`remember_modal_focus`] for layers hidden with `set_visible(false)` rather than
/// removed, such as Settings and the search palettes. The origin is captured each
/// time the layer becomes visible and restored when a `dismissing` hide completes.
/// The returned flag is re-armed on every show; clear it to skip the origin and
/// fall back to the browser (an activation that already moved focus keeps it).
pub(crate) fn remember_persistent_modal_focus(layer: &gtk::Widget) -> Rc<Cell<bool>> {
    let restore = Rc::new(Cell::new(true));
    let shown: RefCell<Option<Rc<ModalFocusOrigin>>> = RefCell::new(None);
    let restore_on_hide = restore.clone();
    layer.connect_visible_notify(move |layer| {
        let window = layer.root().and_downcast::<gtk::Window>();
        if layer.is_visible() {
            restore_on_hide.set(true);
            forget_focus_origin(layer);
            shown.replace(Some(register_focus_origin(layer, window.as_ref(), true)));
            return;
        }
        let Some(origin) = shown.take() else {
            return;
        };
        forget_focus_origin(layer);
        let Some(window) = window else {
            return;
        };
        if !layer.has_css_class("dismissing")
            || crate::ui::window::visible_modal_layer(&window).is_some()
        {
            return;
        }
        restore_modal_focus(&window, &origin, restore_on_hide.get());
    });
    restore
}

pub(super) fn set_modal_focus_restore(layer: &gtk::Widget, restore: Rc<dyn Fn()>) {
    MODAL_FOCUS_ORIGINS.with(|origins| {
        if let Some((_, origin)) = origins.borrow().iter().find(|(candidate, _)| {
            candidate
                .upgrade()
                .is_some_and(|candidate| candidate == *layer)
        }) {
            origin.restore.replace(Some(restore));
        }
    });
}

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageKind {
    Error,
    PartialFailure,
    Information,
}

pub(super) fn show_error_dialog(parent: &impl IsA<gtk::Widget>, message: &str, detail: &str) {
    show_error_dialog_after_close(parent, message, detail, Rc::new(|| {}));
}

/// Reports an operation that finished while some items failed.
pub(super) fn show_partial_failure_dialog(parent: &impl IsA<gtk::Widget>, detail: &str) {
    show_message_dialog(
        parent,
        &crate::i18n::tr("Completed with errors"),
        detail,
        MessageKind::PartialFailure,
        Rc::new(|| {}),
    );
}

pub(super) fn show_information_dialog(parent: &impl IsA<gtk::Widget>, message: &str, detail: &str) {
    show_message_dialog(
        parent,
        message,
        detail,
        MessageKind::Information,
        Rc::new(|| {}),
    );
}

pub(super) fn show_error_dialog_after_close(
    parent: &impl IsA<gtk::Widget>,
    message: &str,
    detail: &str,
    on_close: Rc<dyn Fn()>,
) {
    show_message_dialog(parent, message, detail, MessageKind::Error, on_close);
}

fn show_message_dialog(
    parent: &impl IsA<gtk::Widget>,
    message: &str,
    detail: &str,
    kind: MessageKind,
    on_close: Rc<dyn Fn()>,
) {
    let error = kind != MessageKind::Information;
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent).or_else(|| {
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
    else {
        on_close();
        return;
    };

    let layout = message_dialog_layout(
        if error {
            crate::assets::icons::X
        } else {
            crate::assets::icons::INFO
        },
        message,
        &crate::i18n::tr(match kind {
            MessageKind::Information => "Reported by the file provider",
            MessageKind::PartialFailure => "Some items could not be processed",
            MessageKind::Error => "The operation could not be completed",
        }),
        &crate::i18n::tr("Close"),
        if error {
            ModalTone::Danger
        } else {
            ModalTone::Accent
        },
    );
    layout.cancel.set_visible(false);
    let explanation = message_dialog_description(detail);
    explanation.set_selectable(true);
    layout.body.append(&explanation);
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

/// For a delete that completed with errors, some of them Trash-unsupported: offers
/// Delete Permanently for those entries via `on_retry` (#179).
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
        &crate::i18n::tr("Completed with errors"),
        &crate::i18n::tr("Some items could not be processed"),
        &crate::i18n::tr("Delete Permanently"),
        ModalTone::Danger,
    );
    layout.cancel.set_label(&crate::i18n::tr("Done"));
    let explanation = message_dialog_description(detail);
    explanation.set_selectable(true);
    layout.body.append(&explanation);
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
    focus_button(&confirm);
}
