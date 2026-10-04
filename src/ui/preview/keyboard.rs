// SPDX-License-Identifier: MIT

//! The focused pane owns the keys, including password prompts that take focus
//! by themselves. Each preview surface decides what its keys do. The content box
//! is focusable only while it holds the keys for a surface that has no focusable
//! widget yet, so default Tab order is
//! unchanged.

use super::*;
use crate::ui::browser::{BrowserView, WeakBrowserView};

const OWNER_CLASS: &str = "preview-keyboard-owner";
const MAX_CLAIM_FRAMES: u32 = 30;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::ui) enum PreviewSurface {
    Document,
    Archive,
    Media,
    Text,
    Control,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::ui) enum DocumentScroll {
    Line(i32),
    HalfPage(i32),
    Page(i32),
    Start,
    End,
}

impl PreviewDrawer {
    /// The browser whose Miller columns yield their keyboard-destination bar
    /// while the drawer owns the keys.
    pub(in crate::ui) fn bind_keyboard_view(&self, view: &BrowserView) {
        self.state.keyboard_view.replace(Some(view.downgrade()));
        let weak = Rc::downgrade(&self.state);
        view.set_playback_handoff(Rc::new(move |location| {
            weak.upgrade()
                .map(|state| PreviewDrawer { state })
                .and_then(|drawer| drawer.prepare_handoff(location))
        }));
    }

    /// Whether keyboard focus is anywhere inside the drawer.
    pub(in crate::ui) fn owns_focus(&self, focused: Option<&gtk::Widget>) -> bool {
        let pane = self.state.pane.upcast_ref::<gtk::Widget>();
        focused.is_some_and(|focused| focused == pane || focused.is_ancestor(pane))
    }

    pub(in crate::ui) fn surface(&self, focused: &gtk::Widget) -> PreviewSurface {
        self.state.surface(focused)
    }

    /// Moves keys into the open drawer. A drawer suspended for lack of room
    /// takes them when it is shown again for the same file.
    pub(in crate::ui) fn take_keyboard(&self) -> bool {
        self.state.take_keyboard()
    }

    pub(in crate::ui) fn scroll_document(&self, motion: DocumentScroll) -> bool {
        self.state.scroll_document(motion)
    }

    /// Plain media keys for a keyboard-owned media preview, plus `<` / `>` from
    /// the 10xer listing while an audio preview is open.
    pub(in crate::ui) fn media_key(&self, key: gtk::gdk::Key) -> bool {
        self.has_video() && self.state.media_command(key)
    }

    pub(in crate::ui) fn archive_at_root(&self) -> bool {
        self.state
            .archive_browser
            .borrow()
            .as_ref()
            .is_none_or(archive::ArchiveBrowser::at_root)
    }

    pub(in crate::ui) fn archive_edge(&self, last: bool) -> bool {
        self.state
            .archive_browser
            .borrow()
            .as_ref()
            .is_some_and(|browser| {
                browser.move_cursor(if last {
                    isize::MAX / 2
                } else {
                    -isize::MAX / 2
                })
            })
    }

    #[cfg(test)]
    pub(in crate::ui) fn document_scroll_value(&self) -> Option<f64> {
        self.state
            .primary_scroll()
            .map(|scroll| scroll.vadjustment().value())
    }
}

impl PreviewState {
    pub(super) fn install_keyboard_ownership(self: &Rc<Self>) {
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        // Rebuilt archive rows and self-focusing password prompts re-enter here.
        focus.connect_enter(move |_| {
            if let Some(state) = weak.upgrade() {
                state.set_keyboard_owner(true);
            }
        });
        let weak = Rc::downgrade(self);
        focus.connect_leave(move |_| {
            if let Some(state) = weak.upgrade() {
                state.content.set_focusable(false);
                state.set_keyboard_owner(false);
            }
        });
        self.pane.add_controller(focus);
    }

    /// The owner bar replaces the Miller column's destination bar while the
    /// drawer holds the keys.
    pub(super) fn set_keyboard_owner(&self, owned: bool) {
        if owned {
            self.pane.add_css_class(OWNER_CLASS);
        } else {
            self.pane.remove_css_class(OWNER_CLASS);
        }
        if let Some(view) = self
            .keyboard_view
            .borrow()
            .as_ref()
            .and_then(WeakBrowserView::upgrade)
        {
            view.set_preview_owns_keys(owned);
        }
    }

    fn take_keyboard(self: &Rc<Self>) -> bool {
        if !self.is_enabled() {
            return false;
        }
        if self.sizing.is_suspended() {
            self.claim_on_resume.set(true);
            return true;
        }
        if self.claim_keyboard() {
            return true;
        }
        let weak = Rc::downgrade(self);
        let frames = Cell::new(0);
        self.pane.add_tick_callback(move |_, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            frames.set(frames.get() + 1);
            if !state.is_enabled() || state.claim_keyboard() || frames.get() >= MAX_CLAIM_FRAMES {
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        true
    }

    /// Called when a suspended drawer is shown again.
    pub(super) fn resume_keyboard_claim(self: &Rc<Self>) {
        if self.claim_on_resume.replace(false) {
            self.take_keyboard();
        }
    }

    /// The document's own widget, so its select-all and copy handlers receive
    /// **Ctrl+A** / **Ctrl+C**.
    fn document_key_target(&self) -> Option<gtk::Widget> {
        fn visit(widget: &gtk::Widget) -> Option<gtk::Widget> {
            if !widget.is_visible() {
                return None;
            }
            if widget.is_focusable()
                && (widget.has_css_class("preview-virtual-list")
                    || widget.has_css_class("preview-pdf-scroll")
                    || widget.has_css_class("preview-text"))
            {
                return Some(widget.clone());
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(found) = visit(&widget) {
                    return Some(found);
                }
                child = widget.next_sibling();
            }
            None
        }
        visit(self.content.upcast_ref())
    }

    fn claim_keyboard(&self) -> bool {
        if !self.pane.is_mapped() {
            return false;
        }
        let target: gtk::Widget = if let Some(browser) = self.archive_browser.borrow().as_ref() {
            browser.list().clone().upcast()
        } else if let Some(entry) = self.password_entry.borrow().as_ref() {
            entry.clone().upcast()
        } else if let Some(document) = self.document_key_target() {
            document
        } else {
            self.content.set_focusable(true);
            self.content.clone().upcast()
        };
        if !target.grab_focus() {
            return false;
        }
        self.set_keyboard_owner(true);
        true
    }

    /// Replacing content destroys its focused child; keep the keys in the drawer
    /// instead of letting them fall to an unrelated widget.
    pub(super) fn content_owns_keys(&self) -> bool {
        self.pane.has_css_class(OWNER_CLASS)
            && self
                .content
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focused| {
                    focused == *self.content.upcast_ref::<gtk::Widget>()
                        || focused.is_ancestor(&self.content)
                })
    }

    pub(super) fn keep_keys_in_content(&self, owned: bool) {
        if !owned || !self.is_enabled() {
            return;
        }
        let focus_lost = self
            .content
            .root()
            .and_then(|root| root.focus())
            .is_none_or(|focused| {
                focused != *self.content.upcast_ref::<gtk::Widget>()
                    && !focused.is_ancestor(&self.content)
            });
        if focus_lost {
            self.content.set_focusable(true);
            self.content.grab_focus();
            self.set_keyboard_owner(true);
        }
    }

    /// Self-focusing password prompts and rebuilt archive rows own the keys too.
    pub(super) fn reassert_keyboard_owner(&self) {
        let inside = self
            .pane
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focused| {
                focused == *self.pane.upcast_ref::<gtk::Widget>() || focused.is_ancestor(&self.pane)
            });
        if inside {
            self.set_keyboard_owner(true);
        }
    }

    /// A newly rendered interactive surface inherits keys owned by the content box.
    pub(super) fn hand_keys_to(&self, widget: &impl IsA<gtk::Widget>) {
        if self.content.has_focus() {
            widget.grab_focus();
        }
    }

    /// A document that finished rendering after **l** takes the keys from the
    /// placeholder content box.
    pub(super) fn hand_keys_to_document(&self) {
        if self.content.has_focus()
            && let Some(document) = self.document_key_target()
        {
            document.grab_focus();
        }
    }

    fn surface(&self, focused: &gtk::Widget) -> PreviewSurface {
        if self.archive_list_has_focus(Some(focused)) {
            return PreviewSurface::Archive;
        }
        if accepts_typing(focused) {
            return PreviewSurface::Text;
        }
        let media_view = self.has_media_view()
            && (focused == self.content.upcast_ref::<gtk::Widget>()
                || focused.is::<gtk::Overlay>()
                || focused.is::<super::audio::Scrubber>()
                || focused.is::<super::video::Timeline>());
        if media_view {
            return PreviewSurface::Media;
        }
        let control = focused.is::<gtk::Button>()
            || focused.is::<gtk::Range>()
            || focused.is::<gtk::Switch>()
            || focused.is::<gtk::DropDown>()
            || focused.ancestor(gtk::Button::static_type()).is_some()
            || focused.ancestor(gtk::Range::static_type()).is_some();
        if control {
            PreviewSurface::Control
        } else {
            PreviewSurface::Document
        }
    }

    /// The tallest scrollable view in the content, which is the document body
    /// rather than a breadcrumb strip or metadata scroller.
    fn primary_scroll(&self) -> Option<gtk::ScrolledWindow> {
        fn visit(widget: &gtk::Widget, best: &mut Option<gtk::ScrolledWindow>) {
            if !widget.is_visible() {
                return;
            }
            if let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>()
                && scroll.vscrollbar_policy() != gtk::PolicyType::Never
                && best
                    .as_ref()
                    .is_none_or(|best| scroll.height() > best.height())
            {
                best.replace(scroll.clone());
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                visit(&widget, best);
                child = widget.next_sibling();
            }
        }
        let mut best = None;
        visit(self.content.upcast_ref(), &mut best);
        best
    }

    fn scroll_document(&self, motion: DocumentScroll) -> bool {
        let Some(scroll) = self.primary_scroll() else {
            return false;
        };
        let adjustment = scroll.vadjustment();
        let page = adjustment.page_size().max(1.0);
        let lower = adjustment.lower();
        let limit = (adjustment.upper() - adjustment.page_size()).max(lower);
        let target = match motion {
            DocumentScroll::Line(direction) => {
                let step = if adjustment.step_increment() >= 1.0 {
                    adjustment.step_increment()
                } else {
                    page / 10.0
                };
                adjustment.value() + f64::from(direction) * step
            }
            DocumentScroll::HalfPage(direction) => {
                adjustment.value() + f64::from(direction) * page / 2.0
            }
            DocumentScroll::Page(direction) => adjustment.value() + f64::from(direction) * page,
            DocumentScroll::Start => lower,
            DocumentScroll::End => limit,
        };
        adjustment.set_value(target.clamp(lower, limit));
        true
    }
}

/// Read-only source text is a document, not a text field.
fn accepts_typing(focused: &gtk::Widget) -> bool {
    let view = focused
        .downcast_ref::<gtk::TextView>()
        .cloned()
        .or_else(|| {
            focused
                .ancestor(gtk::TextView::static_type())
                .and_downcast::<gtk::TextView>()
        });
    match view {
        Some(view) => view.is_editable(),
        None => crate::ui::focus_navigation::editable(focused),
    }
}
