// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use gtk::{glib, prelude::*};

use super::{
    CandidateKeys, PromptResetListeners, ShortcutFooter, candidates::Candidates,
    reference::ReferenceSession,
};
use crate::ui::tenxer_mode::Prompt;

/// The open footer prompt. Its bar covers the whole footer so typing never
/// shares the row with status marks.
#[derive(Clone)]
pub(super) struct PromptBar {
    pub(super) bar: gtk::Box,
    pub(super) label: gtk::Label,
    pub(super) entry: gtk::Entry,
    pub(super) hint: gtk::Label,
    pub(super) kind: Rc<Cell<Option<Prompt>>>,
    pub(super) resets: PromptResetListeners,
    /// Set while a [`PromptSink`] replaces the text, which is not an edit.
    pub(super) replacing: Rc<Cell<bool>>,
    pub(super) candidates: Rc<Candidates>,
}

#[derive(Clone)]
pub(super) struct WeakPromptBar {
    bar: glib::WeakRef<gtk::Box>,
    label: glib::WeakRef<gtk::Label>,
    entry: glib::WeakRef<gtk::Entry>,
    hint: glib::WeakRef<gtk::Label>,
    kind: Rc<Cell<Option<Prompt>>>,
    resets: PromptResetListeners,
    replacing: Rc<Cell<bool>>,
    candidates: Rc<Candidates>,
}

impl PromptBar {
    pub(super) fn new() -> Self {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.add_css_class("shortcut-footer-prompt-bar");
        bar.set_visible(false);
        let label = gtk::Label::new(None);
        label.add_css_class("shortcut-footer-prompt-label");
        let entry = gtk::Entry::new();
        entry.add_css_class("form-control");
        entry.add_css_class("shortcut-footer-prompt");
        entry.set_hexpand(true);
        crate::ui::accessibility::set_labelled_by(&entry, &[label.upcast_ref()]);
        let hint = gtk::Label::new(None);
        hint.add_css_class("shortcut-footer-chord-hint");
        hint.set_ellipsize(gtk::pango::EllipsizeMode::End);
        hint.set_visible(false);
        bar.append(&label);
        bar.append(&entry);
        bar.append(&hint);
        let candidates = Candidates::attach(&entry);
        Self {
            bar,
            label,
            entry,
            hint,
            kind: Rc::new(Cell::new(None)),
            resets: Rc::default(),
            replacing: Rc::default(),
            candidates,
        }
    }

    pub(super) fn downgrade(&self) -> WeakPromptBar {
        WeakPromptBar {
            bar: self.bar.downgrade(),
            label: self.label.downgrade(),
            entry: self.entry.downgrade(),
            hint: self.hint.downgrade(),
            kind: self.kind.clone(),
            resets: self.resets.clone(),
            replacing: self.replacing.clone(),
            candidates: self.candidates.clone(),
        }
    }

    fn set_hint(&self, text: Option<&str>) {
        self.hint.set_text(text.unwrap_or_default());
        self.hint.set_visible(text.is_some());
        self.entry
            .update_property(&[gtk::accessible::Property::Description(
                text.unwrap_or_default(),
            )]);
    }

    fn reset(&self) {
        self.set_hint(None);
        self.candidates.clear();
        for listener in self.resets.borrow().iter() {
            listener();
        }
    }

    pub(super) fn has_focus(&self) -> bool {
        self.bar.is_visible()
            && self
                .bar
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focus| {
                    focus == *self.entry.upcast_ref::<gtk::Widget>()
                        || focus.is_ancestor(&self.entry)
                })
    }

    /// Shown, but focus moved to something other than the prompt or the reference.
    fn abandoned(&self, session: &ReferenceSession) -> bool {
        if !self.bar.is_visible() || self.has_focus() {
            return false;
        }
        !session.is_open()
    }

    fn open(&self, stack: &gtk::Stack, kind: Prompt, text: &str) -> bool {
        // Replacing an open prompt must not hand its text to the new kind.
        self.kind.set(None);
        self.entry.set_text("");
        self.reset();
        // Undo history could otherwise bring back typed credentials.
        self.entry.set_enable_undo(kind != Prompt::Go);
        self.kind.set(Some(kind));
        self.label.set_text(&kind.label());
        crate::ui::accessibility::set_label(&self.entry, &crate::i18n::tr(kind.name()));
        self.entry.set_text(text);
        self.bar.set_visible(true);
        stack.set_visible_child(&self.bar);
        // A pre-filled query stays unselected so typing extends it.
        let focused = self.entry.grab_focus_without_selecting();
        self.entry.set_position(-1);
        focused
    }

    pub(super) fn close(&self) {
        self.kind.set(None);
        self.entry.set_text("");
        self.bar.set_visible(false);
        self.reset();
    }
}

impl WeakPromptBar {
    pub(super) fn upgrade(&self) -> Option<PromptBar> {
        Some(PromptBar {
            bar: self.bar.upgrade()?,
            label: self.label.upgrade()?,
            entry: self.entry.upgrade()?,
            hint: self.hint.upgrade()?,
            kind: self.kind.clone(),
            resets: self.resets.clone(),
            replacing: self.replacing.clone(),
            candidates: self.candidates.clone(),
        })
    }
}

pub(in crate::ui) struct PromptSink {
    prompt: WeakPromptBar,
    kind: Prompt,
}

impl PromptSink {
    /// Replaces the text (caret at the end) and the hint, but only while the
    /// same kind of prompt is still open.
    pub(in crate::ui) fn show(&self, text: Option<&str>, hint: Option<&str>) {
        let Some(prompt) = self.prompt.upgrade() else {
            return;
        };
        if prompt.kind.get() != Some(self.kind) || !prompt.bar.is_visible() {
            return;
        }
        if let Some(text) = text {
            // GTK reports a replacement as two changes: cleared, then filled.
            prompt.replacing.set(true);
            prompt.entry.set_text(text);
            prompt.replacing.set(false);
            prompt.entry.set_position(-1);
        }
        prompt.set_hint(hint);
    }
}

impl ShortcutFooter {
    /// Focus moving anywhere but the shortcut reference (a clicked row, another
    /// pane) discards the prompt, so its text never reaches a browsing command.
    pub(super) fn close_prompt_on_focus_loss(&self) {
        let focus = gtk::EventControllerFocus::new();
        let prompt = self.prompt.downgrade();
        let session = self.session.clone();
        focus.connect_leave(move |_| {
            let prompt = prompt.clone();
            let session = session.clone();
            // Focus has not settled on its new owner while ::leave runs.
            glib::idle_add_local_once(move || {
                if let Some(prompt) = prompt.upgrade()
                    && prompt.abandoned(&session)
                {
                    prompt.close();
                }
            });
        });
        self.prompt.entry.add_controller(focus);
    }

    pub(in crate::ui) fn open_prompt(&self, kind: Prompt) -> bool {
        self.open_prompt_with(kind, "")
    }

    pub(in crate::ui) fn open_prompt_with(&self, kind: Prompt, text: &str) -> bool {
        // An armed chord must stay visible, and the prompt covers its mark.
        self.chords.clear();
        self.prompt.open(&self.root, kind, text)
    }

    pub(in crate::ui) fn select_prompt_region(&self, start: i32, end: i32) {
        self.prompt.entry.select_region(start, end);
    }

    /// Runs `listener` as the open prompt's text is edited, not when a prompt
    /// opens empty or closes, a [`PromptSink`] replaces the text, or a change
    /// leaves the text as it was, such as a `/` that puts back a selection.
    pub(in crate::ui) fn connect_prompt_changed(
        &self,
        listener: impl Fn(Prompt, String) + 'static,
    ) {
        let kind = self.prompt.kind.clone();
        let replacing = self.prompt.replacing.clone();
        let last = RefCell::new(self.prompt.entry.text().to_string());
        self.prompt.entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if *last.borrow() == text {
                return;
            }
            last.replace(text.clone());
            if replacing.get() {
                return;
            }
            if let Some(kind) = kind.get() {
                listener(kind, text);
            }
        });
    }

    /// Runs `listener` whenever a prompt opens or closes, so work tied to the
    /// previous text can be dropped.
    pub(in crate::ui) fn connect_prompt_reset(&self, listener: impl Fn() + 'static) {
        self.prompt.resets.borrow_mut().push(Box::new(listener));
    }

    #[cfg(test)]
    pub(in crate::ui) fn prompt_hint(&self) -> Option<String> {
        self.prompt
            .hint
            .is_visible()
            .then(|| self.prompt.hint.text().to_string())
    }

    pub(in crate::ui) fn prompt_sink(&self, kind: Prompt) -> PromptSink {
        PromptSink {
            prompt: self.prompt.downgrade(),
            kind,
        }
    }

    #[cfg(test)]
    pub(in crate::ui) fn prompt(&self) -> &gtk::Entry {
        &self.prompt.entry
    }

    pub(in crate::ui) fn open_prompt_kind(&self) -> Option<Prompt> {
        self.prompt.kind.get()
    }

    pub(in crate::ui) fn prompt_text(&self) -> String {
        self.prompt.entry.text().to_string()
    }

    /// Unlike `PromptSink`, this notifies edit listeners.
    pub(in crate::ui) fn type_prompt_text(&self, text: &str) {
        self.prompt.entry.set_text(text);
        self.prompt.entry.set_position(-1);
    }

    #[cfg(test)]
    pub(in crate::ui) fn prompt_label(&self) -> Option<String> {
        (self.root.visible_child().as_ref() == Some(self.prompt.bar.upcast_ref()))
            .then(|| self.prompt.label.text().to_string())
    }

    pub(in crate::ui) fn dismiss_prompt(&self) {
        self.prompt.close();
    }

    pub(in crate::ui) fn show_candidates(&self, paths: Vec<PathBuf>, keys: CandidateKeys) {
        if self.prompt.bar.is_visible() {
            self.prompt.candidates.set(paths, keys);
        }
    }

    pub(in crate::ui) fn step_candidate(&self, delta: i32) {
        self.prompt.candidates.step(delta);
    }

    pub(in crate::ui) fn forget_candidate_step(&self) {
        self.prompt.candidates.forget_step();
    }

    pub(in crate::ui) fn candidate_stepped(&self) -> bool {
        self.prompt.candidates.stepped()
    }

    pub(in crate::ui) fn chosen_candidate(&self) -> Option<PathBuf> {
        self.prompt.candidates.chosen()
    }

    pub(in crate::ui) fn candidate_position(&self) -> Option<(usize, usize)> {
        self.prompt.candidates.position()
    }

    pub(in crate::ui) fn connect_prompt_activated(&self, listener: impl Fn() + 'static) {
        self.prompt.entry.connect_activate(move |_| listener());
    }

    /// See [`crate::ui::browser::connect_path_separator`].
    pub(in crate::ui) fn connect_prompt_path_separator(
        &self,
        accepts: impl Fn() -> bool + 'static,
        descend: impl Fn() + 'static,
    ) {
        crate::ui::browser::connect_path_separator(&self.prompt.entry, accepts, descend);
    }

    pub(in crate::ui) fn connect_candidate_activated(&self, listener: impl Fn(PathBuf) + 'static) {
        self.prompt.candidates.connect_activated(listener);
    }

    #[cfg(test)]
    pub(in crate::ui) fn candidates(&self) -> Vec<PathBuf> {
        self.prompt.candidates.paths()
    }

    #[cfg(test)]
    pub(in crate::ui) fn candidate_keys(&self) -> String {
        self.prompt.candidates.keys_text()
    }

    #[cfg(test)]
    pub(in crate::ui) fn candidates_shown(&self) -> bool {
        self.prompt.candidates.is_shown()
    }

    #[cfg(test)]
    pub(in crate::ui) fn click_candidate(&self, index: usize) {
        self.prompt.candidates.activate(index);
    }

    pub(in crate::ui) fn prompt_is_visible(&self) -> bool {
        self.prompt.bar.is_visible()
    }

    pub(in crate::ui) fn prompt_has_focus(&self) -> bool {
        self.prompt.has_focus()
    }
}
