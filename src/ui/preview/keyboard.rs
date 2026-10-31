// SPDX-License-Identifier: MIT

//! The focused pane owns the keys, including password prompts that take focus
//! by themselves. Each preview surface decides what its keys do. The content box
//! is focusable only while it holds the keys for a surface that has no focusable
//! widget yet, so default Tab order is
//! unchanged.

use super::*;
use crate::ui::{
    browser::{BrowserView, WeakBrowserView},
    keys::{
        Match,
        bindings::{
            DOWN, END_KEY, HOME_KEY, LEFT, MEDIA_SEEK_BACK_PAGE, MEDIA_SEEK_END,
            MEDIA_SEEK_FORWARD_PAGE, MEDIA_SEEK_START, MEDIA_VOLUME_STEP_DOWN,
            MEDIA_VOLUME_STEP_UP, PAGE_DOWN_KEY, PAGE_UP_KEY, PDF_ZOOM_RESET, RIGHT, SPACE,
            TENXER_MEDIA_MUTE, TENXER_MEDIA_NEXT, TENXER_MEDIA_PREVIOUS, UP,
        },
    },
};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::ui) enum ArchiveMove {
    Previous,
    Next,
    Parent,
    Open,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::ui) enum MediaCommand {
    PlayPause,
    Seek(MediaSeek),
    VolumeUp,
    VolumeDown,
    Mute,
    Previous,
    Next,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::ui) enum MediaSeek {
    By(i64),
    Start,
    End,
}

impl MediaSeek {
    pub(super) fn target(self, timestamp: i64, duration: i64) -> i64 {
        let last = media_last_frame(duration);
        match self {
            Self::By(delta) => (timestamp + delta).clamp(0, last),
            Self::Start => 0,
            Self::End => last,
        }
    }
}

impl MediaCommand {
    pub(super) fn volume_delta(self) -> Option<f64> {
        match self {
            Self::VolumeUp => Some(MEDIA_VOLUME_STEP),
            Self::VolumeDown => Some(-MEDIA_VOLUME_STEP),
            _ => None,
        }
    }
}

const SEEK_AND_VOLUME: [(&dyn Match, MediaCommand); 12] = [
    (
        &LEFT,
        MediaCommand::Seek(MediaSeek::By(-MEDIA_SEEK_STEP_US)),
    ),
    (
        &RIGHT,
        MediaCommand::Seek(MediaSeek::By(MEDIA_SEEK_STEP_US)),
    ),
    (
        &MEDIA_SEEK_BACK_PAGE,
        MediaCommand::Seek(MediaSeek::By(-MEDIA_SEEK_PAGE_US)),
    ),
    (
        &MEDIA_SEEK_FORWARD_PAGE,
        MediaCommand::Seek(MediaSeek::By(MEDIA_SEEK_PAGE_US)),
    ),
    (
        &PAGE_DOWN_KEY,
        MediaCommand::Seek(MediaSeek::By(-MEDIA_SEEK_PAGE_US)),
    ),
    (
        &PAGE_UP_KEY,
        MediaCommand::Seek(MediaSeek::By(MEDIA_SEEK_PAGE_US)),
    ),
    (&MEDIA_SEEK_START, MediaCommand::Seek(MediaSeek::Start)),
    (&HOME_KEY, MediaCommand::Seek(MediaSeek::Start)),
    (&MEDIA_SEEK_END, MediaCommand::Seek(MediaSeek::End)),
    (&END_KEY, MediaCommand::Seek(MediaSeek::End)),
    (&MEDIA_VOLUME_STEP_UP, MediaCommand::VolumeUp),
    (&MEDIA_VOLUME_STEP_DOWN, MediaCommand::VolumeDown),
];

const PLAYBACK: [(&dyn Match, MediaCommand); 6] = [
    (&SPACE, MediaCommand::PlayPause),
    (&UP, MediaCommand::VolumeUp),
    (&DOWN, MediaCommand::VolumeDown),
    (&TENXER_MEDIA_MUTE, MediaCommand::Mute),
    (&TENXER_MEDIA_PREVIOUS, MediaCommand::Previous),
    (&TENXER_MEDIA_NEXT, MediaCommand::Next),
];

pub(super) fn seek_or_volume(press: &KeyPress) -> Option<MediaCommand> {
    command_for(&SEEK_AND_VOLUME, press)
}

/// Keys a keyboard-owned media preview claims, even when the stream cannot
/// act on them.
pub(in crate::ui) fn media_key_command(press: &KeyPress) -> Option<MediaCommand> {
    command_for(&PLAYBACK, press).or_else(|| seek_or_volume(press))
}

fn command_for(commands: &[(&dyn Match, MediaCommand)], press: &KeyPress) -> Option<MediaCommand> {
    commands
        .iter()
        .find(|(key, _)| press.is(key))
        .map(|(_, command)| *command)
}

/// Steps the shared preview volume; reaching zero mutes and keeps the volume
/// to restore.
pub(super) fn step_preview_volume(delta: f64) {
    let preferences = crate::ui::preferences::PreferenceManager::shared();
    let current = if preferences.preview_muted() {
        0.0
    } else {
        preferences.preview_volume()
    };
    let volume = snapped_volume(current + delta);
    preferences.set_preview_audio(volume, volume == 0.0);
}

impl PreviewDrawer {
    pub(in crate::ui) fn reset_pdf_zoom(&self, press: &KeyPress) -> bool {
        press.is(PDF_ZOOM_RESET)
            && self
                .state
                .pane
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focused| {
                    self.owns_focus(Some(&focused))
                        && focused.activate_action("pdf.reset-zoom", None).is_ok()
                })
    }

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

    /// Moves the keys from the placeholder content box to a message's action button.
    pub(in crate::ui) fn focus_feedback_action(&self) -> bool {
        self.state.focus_feedback_action()
    }

    pub(in crate::ui) fn feedback_action_focused(&self, focused: &gtk::Widget) -> bool {
        self.state
            .feedback_action()
            .is_some_and(|action| *focused == action)
    }

    /// Plain media keys for a keyboard-owned media preview, plus `<` / `>` from
    /// the 10xer listing while an audio preview is open.
    pub(in crate::ui) fn media_key(&self, command: MediaCommand) -> bool {
        self.has_video() && self.state.media_command(command)
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

    /// Keyboard-driven focus the window's own key handling never reports to GTK,
    /// so the focus ring is requested explicitly.
    fn focus_feedback_action(&self) -> bool {
        let Some(action) = self.feedback_action() else {
            return false;
        };
        if !action.grab_focus() {
            return false;
        }
        if let Some(window) = action.root().and_downcast::<gtk::Window>() {
            window.set_focus_visible(true);
        }
        true
    }

    fn feedback_action(&self) -> Option<gtk::Button> {
        let mut feedback = self.content.first_child();
        while let Some(widget) = feedback {
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if widget.has_css_class("preview-feedback-action") {
                    return widget.downcast().ok();
                }
                child = widget.next_sibling();
            }
            feedback = widget.next_sibling();
        }
        None
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
        } else if self.focus_feedback_action() {
            self.set_keyboard_owner(true);
            return true;
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
        if crate::ui::focus_navigation::accepts_typing(focused) {
            return PreviewSurface::Text;
        }
        let media_view = self.has_media_view()
            && (focused == self.content.upcast_ref::<gtk::Widget>()
                || focused.is::<gtk::Overlay>()
                || focused.is::<super::waveform::Waveform>());
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
