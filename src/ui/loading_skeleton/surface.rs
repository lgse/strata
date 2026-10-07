// SPDX-License-Identifier: MIT

//! A directory pane's page stack doubles as its keyboard focus target while it shows a
//! status (empty or unreadable) or loading page, because the hidden collection view
//! would accept focus that nothing can see or move.

use std::{
    cell::Cell,
    rc::{Rc, Weak},
};

use gtk::{glib, prelude::*};

use super::{CONTENT_PAGE, LOADING_PAGE, PENDING_PAGE};
use crate::app::Browser;

pub(crate) struct DirectorySurface {
    stack: glib::WeakRef<gtk::Stack>,
    message: glib::WeakRef<gtk::Label>,
    view: glib::WeakRef<gtk::Widget>,
    /// Filter results keep their own focus inside the content page.
    keeps_focus: Option<glib::WeakRef<gtk::Widget>>,
    browser: Weak<Browser>,
    directory: String,
    settling: Cell<bool>,
}

impl DirectorySurface {
    /// `message` is the status page's text; `view` is the collection the content page
    /// shows. Holds only weak references, since the stack owns the handlers.
    pub(crate) fn install(
        stack: &gtk::Stack,
        message: &gtk::Label,
        view: &impl IsA<gtk::Widget>,
        keeps_focus: Option<&gtk::Widget>,
        browser: &Rc<Browser>,
        directory: &str,
    ) {
        stack.add_css_class("directory-surface");
        let surface = Rc::new(Self {
            stack: stack.downgrade(),
            message: message.downgrade(),
            view: view.as_ref().downgrade(),
            keeps_focus: keeps_focus.map(ObjectExt::downgrade),
            browser: Rc::downgrade(browser),
            directory: directory.to_owned(),
            settling: Cell::new(false),
        });
        let on_page = surface.clone();
        stack.connect_visible_child_notify(move |_| on_page.sync());
        let on_message = surface.clone();
        message.connect_label_notify(move |_| on_message.sync());
        // A reload or re-sort that removes the focused row leaves focus on a detached
        // widget, which GTK resolves at the next paint by focusing the next Tab stop.
        if let Some(rows) = view.property::<Option<gtk::SelectionModel>>("model") {
            let on_rows = Rc::downgrade(&surface);
            rows.connect_items_changed(move |_, _, removed, _| {
                if removed > 0
                    && let Some(surface) = on_rows.upgrade()
                    && let Some(stack) = surface.stack.upgrade()
                    && focus_removed_from_content(&stack)
                {
                    surface.settle_before_paint(&stack);
                }
            });
        }
        surface.sync();
    }

    fn sync(self: &Rc<Self>) {
        let (Some(stack), Some(message)) = (self.stack.upgrade(), self.message.upgrade()) else {
            return;
        };
        let page = stack.visible_child_name();
        match page.as_deref() {
            // Every reload detaches the rows and shows the grace page, so the surface
            // holds the focus they had until they return. It stays named but says
            // nothing more: "Loading" is announced only once the loading page shows,
            // so a reload that finishes within the grace period stays quiet.
            Some(PENDING_PAGE) => {
                if self.content_holds_focus(&stack) {
                    if !stack.is_focusable() {
                        self.describe(&stack, "");
                        stack.set_focusable(true);
                    }
                    self.settle_before_paint(&stack);
                }
            }
            Some(CONTENT_PAGE) => {
                stack.reset_property(gtk::AccessibleProperty::Label);
                stack.reset_property(gtk::AccessibleProperty::Description);
                stack.set_focusable(false);
                if surface_has_focus(&stack) {
                    self.hand_back(&stack);
                } else if self.content_holds_focus(&stack) {
                    self.settle_before_paint(&stack);
                }
            }
            Some(page) => {
                let status = if page == LOADING_PAGE {
                    crate::i18n::tr(crate::ui::accessibility::LOADING_SURFACE_DESCRIPTION)
                } else {
                    message.label().to_string()
                };
                self.describe(&stack, &status);
                stack.set_focusable(true);
                if self.content_holds_focus(&stack) {
                    stack.grab_focus();
                }
            }
            None => {}
        }
    }

    fn describe(&self, stack: &gtk::Stack, status: &str) {
        crate::ui::accessibility::describe_pane_surface(stack, &self.directory, status);
    }

    /// Whether the window's focus belongs to the content page: on a widget in it, or on
    /// a row that a reload just removed, while GTK's focus chain still runs through it.
    fn content_holds_focus(&self, stack: &gtk::Stack) -> bool {
        let (Some(content), Some(focused)) = (
            stack.child_by_name(CONTENT_PAGE),
            stack.root().and_then(|root| root.focus()),
        ) else {
            return false;
        };
        if focused.root().is_none() {
            return focus_removed_from_content(stack);
        }
        focused.is_ancestor(&content)
            && !crate::ui::focus_navigation::editable(&focused)
            && !crate::ui::focus_navigation::in_popover(&focused)
            && !self
                .keeps_focus
                .as_ref()
                .and_then(glib::WeakRef::upgrade)
                .is_some_and(|owner| focused == owner || focused.is_ancestor(&owner))
    }

    /// GTK moves focus off a hidden or removed widget after the next paint, to its
    /// nearest focusable ancestor or, failing that, to the next control in Tab order,
    /// which is outside the listing. Settle it first: on the surface while the rows are
    /// hidden, on the cursor row once they are back. A fast reload that restores the
    /// cursor before the frame leaves nothing to do.
    fn settle_before_paint(self: &Rc<Self>, stack: &gtk::Stack) {
        if self.settling.replace(true) {
            return;
        }
        let surface = Rc::downgrade(self);
        stack.add_tick_callback(move |stack, _| {
            let Some(surface) = surface.upgrade() else {
                return glib::ControlFlow::Break;
            };
            surface.settling.set(false);
            let Some(root) = stack.root() else {
                return glib::ControlFlow::Break;
            };
            let removed = focus_removed_from_content(stack);
            if stack.visible_child_name().as_deref() == Some(CONTENT_PAGE) {
                if removed {
                    surface.restore_cursor(stack);
                } else if let Some(focused) = root.focus().filter(gtk::Widget::is_visible) {
                    // Setting the focus again cancels GTK's pending move.
                    root.set_focus(Some(&focused));
                }
            } else if (removed || surface.content_holds_focus(stack))
                && super::surface_takes_focus(stack)
            {
                stack.grab_focus();
            }
            glib::ControlFlow::Break
        });
    }

    /// The rows are back: the keyboard cursor takes focus from the surface.
    fn hand_back(self: &Rc<Self>, stack: &gtk::Stack) {
        let stack = stack.downgrade();
        let surface = Rc::downgrade(self);
        // After the page switch settles; the cursor row is bound by then.
        glib::idle_add_local_once(move || {
            if let (Some(stack), Some(surface)) = (stack.upgrade(), surface.upgrade())
                && surface_has_focus(&stack)
            {
                surface.restore_cursor(&stack);
            }
        });
    }

    fn restore_cursor(&self, stack: &gtk::Stack) {
        if let Some(browser) = self.browser.upgrade() {
            browser.focus_active();
        }
        let lost = stack
            .root()
            .and_then(|root| root.focus())
            .is_none_or(|focused| {
                focused == *stack.upcast_ref::<gtk::Widget>() || focused.root().is_none()
            });
        if lost && let Some(view) = self.view.upgrade() {
            view.grab_focus();
        }
    }
}

fn focus_removed_from_content(stack: &gtk::Stack) -> bool {
    let (Some(content), Some(focused)) = (
        stack.child_by_name(CONTENT_PAGE),
        stack.root().and_then(|root| root.focus()),
    ) else {
        return false;
    };
    crate::ui::focus_navigation::focus_removed_from(&content, &focused)
}

fn surface_has_focus(stack: &gtk::Stack) -> bool {
    stack
        .root()
        .and_then(|root| root.focus())
        .is_some_and(|focused| focused == *stack.upcast_ref::<gtk::Widget>())
}

/// Focuses the surface while it stands in for `view`, otherwise `view` itself.
pub(crate) fn focus_surface_or(stack: &gtk::Stack, view: &impl IsA<gtk::Widget>) -> bool {
    if surface_takes_focus(stack) {
        stack.grab_focus()
    } else {
        view.as_ref().grab_focus()
    }
}

/// Whether focus requests for this pane go to the surface rather than its view. On the
/// grace page the surface keeps whatever role the previous page gave it.
pub(crate) fn surface_takes_focus(stack: &gtk::Stack) -> bool {
    stack.is_focusable() && stack.visible_child_name().as_deref() != Some(CONTENT_PAGE)
}
