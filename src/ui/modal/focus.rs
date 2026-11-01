// SPDX-License-Identifier: MIT

use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct ModalFocusOrigin {
    widget: Option<glib::WeakRef<gtk::Widget>>,
    restore: RefCell<Option<Rc<dyn Fn()>>>,
    /// False when the first dialog of a chain changes the listing; dialogs chained on
    /// it inherit that and return focus through the window fallback too.
    allow_widget: bool,
}

#[derive(Default)]
struct LayerFocus {
    /// The focused widget when the layer was added. A layer closed while another
    /// modal stays open returns focus here if it lies inside that modal.
    opener: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    /// Where the layer was added; a removed layer no longer knows its window.
    host: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    /// Whether the window showed focus rings when the dismissal began.
    focus_visible: Cell<bool>,
}

type FocusOrigins = Vec<(glib::WeakRef<gtk::Widget>, Rc<ModalFocusOrigin>)>;
type FocusFallbacks = Vec<(glib::WeakRef<gtk::Window>, Rc<dyn Fn()>)>;
type LayerFocusEntries = Vec<(glib::WeakRef<gtk::Widget>, Rc<LayerFocus>)>;

thread_local! {
    static MODAL_FOCUS_ORIGINS: RefCell<FocusOrigins> = const { RefCell::new(Vec::new()) };
    static MODAL_FOCUS_FALLBACKS: RefCell<FocusFallbacks> = const { RefCell::new(Vec::new()) };
    static LAYER_FOCUS: RefCell<LayerFocusEntries> = const { RefCell::new(Vec::new()) };
}

fn layer_focus(layer: &gtk::Widget) -> Rc<LayerFocus> {
    let existing = LAYER_FOCUS.with(|entries| {
        let mut entries = entries.borrow_mut();
        entries.retain(|(candidate, _)| candidate.upgrade().is_some());
        entries
            .iter()
            .find(|(candidate, _)| {
                candidate
                    .upgrade()
                    .is_some_and(|candidate| candidate == *layer)
            })
            .map(|(_, focus)| focus.clone())
    });
    if let Some(focus) = existing {
        return focus;
    }
    let focus = Rc::new(LayerFocus::default());
    let tracked = focus.clone();
    layer.connect_sensitive_notify(move |layer| {
        // A dismissal makes the layer insensitive while its control has focus. GTK
        // then drops focus and, on the key release, hides focus rings, so the
        // window's ring state is captured before that.
        if layer.is_sensitive() {
            tracked.focus_visible.set(false);
        } else if layer.has_css_class("dismissing")
            && let Some(window) = layer.root().and_downcast::<gtk::Window>()
        {
            tracked.focus_visible.set(window.gets_focus_visible());
        }
    });
    LAYER_FOCUS.with(|entries| {
        entries
            .borrow_mut()
            .push((layer.downgrade(), focus.clone()));
    });
    focus
}

/// Shows focus rings again after a keyboard dismissal hid them, once focus is back.
fn rearm_focus_ring(window: &gtk::Window, layer: &LayerFocus) {
    if layer.focus_visible.get() && gtk::prelude::RootExt::focus(window).is_some() {
        window.set_focus_visible(true);
    }
}

/// Returns focus to the opener of a layer closed over `under`, a modal that stays
/// open, unless something inside `under` already took focus. Without a usable opener,
/// such as a row the dialog's action re-rendered, `under`'s first focusable control
/// takes it. The window's restore order applies only once no modal is left.
fn restore_focus_under(window: &gtk::Window, under: &gtk::Widget, layer: &LayerFocus) {
    let live = gtk::prelude::RootExt::focus(window)
        .is_some_and(|focus| focus.is_mapped() && focus != *under);
    if live {
        return;
    }
    let opener = layer
        .opener
        .borrow()
        .as_ref()
        .and_then(glib::WeakRef::upgrade)
        .filter(|opener| {
            (opener == under || opener.is_ancestor(under))
                && shown_or_revealing(opener)
                && opener.is_sensitive()
        });
    let moved = match opener {
        Some(opener) => opener.grab_focus(),
        None => focus_first_control(under),
    };
    if moved {
        rearm_focus_ring(window, layer);
    }
}

fn focus_first_control(root: &gtk::Widget) -> bool {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if widget.is_visible() && widget.is_sensitive() {
            if grab_mapped_focus(&widget) {
                return true;
            }
            if focus_first_control(&widget) {
                return true;
            }
        }
        child = widget.next_sibling();
    }
    false
}

fn grab_mapped_focus(widget: &gtk::Widget) -> bool {
    if !widget.is_focusable() || !widget.is_mapped() {
        return false;
    }
    widget.grab_focus()
}

pub(in crate::ui) fn is_closing_layer(widget: &gtk::Widget) -> bool {
    widget.has_css_class("app-modal-layer") && widget.has_css_class("dismissing")
}

/// The focused widget when a layer opens. A layer opened while another closes, such
/// as an error after a confirmation, finds focus gone or inside the closing layer,
/// so it takes over the closing layer's opener instead.
fn layer_opener(host: &gtk::Widget, window: &gtk::Window) -> Option<glib::WeakRef<gtk::Widget>> {
    let focus = gtk::prelude::RootExt::focus(window);
    let closing = match &focus {
        Some(focus) => {
            let mut current = Some(focus.clone());
            while let Some(widget) = current.as_ref().filter(|widget| !is_closing_layer(widget)) {
                current = widget.parent();
            }
            current
        }
        None => {
            let mut child = host.first_child();
            while let Some(widget) = child.as_ref().filter(|widget| !is_closing_layer(widget)) {
                child = widget.next_sibling();
            }
            child
        }
    };
    match closing {
        Some(closing) => layer_focus(&closing).opener.borrow().clone(),
        None => focus.map(|focus| focus.downgrade()),
    }
}

/// Tracks the opener of a layer that is added and removed, and returns focus to it
/// when the layer closes over another modal.
pub(super) fn track_removed_layer(layer: &gtk::Widget) {
    let focus = layer_focus(layer);
    layer.connect_parent_notify(move |layer| {
        if let Some(parent) = layer.parent() {
            let opener = parent
                .root()
                .and_downcast::<gtk::Window>()
                .and_then(|window| layer_opener(&parent, &window));
            focus.opener.replace(opener);
            focus.host.replace(Some(parent.downgrade()));
            return;
        }
        if !layer.has_css_class("dismissing") {
            return;
        }
        let Some(window) = focus
            .host
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade)
            .and_then(|host| host.root())
            .and_downcast::<gtk::Window>()
        else {
            return;
        };
        if let Some(under) = crate::ui::window::visible_modal_layer(&window) {
            restore_focus_under(&window, &under, &focus);
        }
    });
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
    if allow_origin && origin_takes_focus(origin, window) {
        return;
    }
    if let Some(fallback) = modal_focus_fallback(window) {
        fallback();
    }
}

fn origin_takes_focus(origin: &ModalFocusOrigin, window: &gtk::Window) -> bool {
    if !origin.allow_widget {
        return false;
    }
    let Some(widget) = origin.widget.as_ref().and_then(glib::WeakRef::upgrade) else {
        return false;
    };
    if !shown_or_revealing(&widget) || !widget.is_sensitive() {
        return false;
    }
    widget.root().as_ref() == Some(window.upcast_ref()) && widget.grab_focus()
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
        let shown_in_parent = current.is_child_visible() || revealing;
        if !current.is_visible() || !shown_in_parent {
            return false;
        }
        current = parent;
    }
    true
}

pub(in crate::ui) fn remember_modal_focus(layer: &gtk::Box, overlay: &gtk::Overlay) -> Rc<Cell<bool>> {
    remember_removed_modal_focus(layer, overlay, true)
}

/// [`remember_modal_focus`] for dialogs whose work changes the listing, such as
/// file-operation progress: closing it hands focus to the browser cursor through the
/// window fallback, never to the row widget that was focused when it opened.
pub(in crate::ui) fn remember_modal_focus_for_listing(layer: &gtk::Box, overlay: &gtk::Overlay) {
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
        rearm_focus_ring(&window, &layer_focus(layer.upcast_ref()));
    });
    restore
}

/// [`remember_modal_focus`] for layers hidden with `set_visible(false)` rather than
/// removed, such as Settings and the search palettes. The origin is captured each
/// time the layer becomes visible and restored when a `dismissing` hide completes.
/// The returned flag is re-armed on every show; clear it to skip the origin and
/// fall back to the browser (an activation that already moved focus keeps it).
pub(crate) fn remember_persistent_modal_focus(layer: &gtk::Widget) -> Rc<Cell<bool>> {
    let focus = layer_focus(layer);
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
        rearm_focus_ring(&window, &focus);
    });
    restore
}

pub(in crate::ui) fn set_modal_focus_restore(layer: &gtk::Widget, restore: Rc<dyn Fn()>) {
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
