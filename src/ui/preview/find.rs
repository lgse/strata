// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone)]
enum Target {
    Source(sourceview5::View),
    Document(Rc<super::super::virtual_preview::VirtualPreviewState>),
}

#[derive(Clone, Copy)]
enum Match {
    Source(usize, usize),
    Document(super::super::virtual_preview::TextMatch),
}

impl Target {
    fn matches(&self, query: &str) -> Vec<Match> {
        match self {
            Self::Source(view) => {
                let buffer = view.buffer();
                let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
                super::super::virtual_preview::match_ranges(&text, query)
                    .into_iter()
                    .map(|(start, end)| Match::Source(start, end))
                    .collect()
            }
            Self::Document(state) => state
                .find_matches(query)
                .into_iter()
                .map(Match::Document)
                .collect(),
        }
    }

    fn select(&self, found: Match) {
        match (self, found) {
            (Self::Source(view), Match::Source(start, end)) => {
                let buffer = view.buffer();
                let mut start = buffer.iter_at_offset(start as i32);
                buffer.select_range(&buffer.iter_at_offset(end as i32), &start);
                view.scroll_to_iter(&mut start, 0.15, false, 0.0, 0.0);
            }
            (Self::Document(state), Match::Document(found)) => state.select_match(found),
            _ => {}
        }
    }

    fn clear_selection(&self) {
        match self {
            Self::Source(view) => {
                let buffer = view.buffer();
                buffer.place_cursor(&buffer.iter_at_offset(buffer.cursor_position()));
            }
            Self::Document(state) => state.clear_selection(),
        }
    }
}

pub(super) struct PreviewFind {
    pub widget: gtk::Box,
    pub entry: gtk::Entry,
    status: gtk::Label,
    previous: gtk::Button,
    next: gtk::Button,
    close: gtk::Button,
    target: RefCell<Option<Target>>,
    source_subscription: RefCell<Option<(gtk::TextBuffer, glib::SignalHandlerId)>>,
    matches: RefCell<Vec<Match>>,
    current: Cell<usize>,
}

pub(super) fn icon_button(label: &str, icon: &str) -> gtk::Button {
    let label = crate::i18n::tr(label);
    let button = gtk::Button::new();
    button.add_css_class("preview-header-action");
    button.set_valign(gtk::Align::Center);
    button.set_child(Some(&crate::assets::primary_icon(icon, 16)));
    button.set_tooltip_text(Some(&label));
    super::super::accessibility::set_label(&button, &label);
    button
}

impl PreviewFind {
    pub(super) fn new() -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        widget.add_css_class("preview-find");
        widget.set_visible(false);
        let entry = super::super::controls::form_entry();
        entry.set_hexpand(true);
        entry.set_max_length(256);
        entry.set_width_chars(8);
        entry.set_placeholder_text(Some(&crate::i18n::tr("Find in preview…")));
        super::super::accessibility::set_label(&entry, &crate::i18n::tr("Find in preview"));
        let status = gtk::Label::new(None);
        status.add_css_class("preview-find-status");
        status.set_ellipsize(gtk::pango::EllipsizeMode::End);
        status.set_accessible_role(gtk::AccessibleRole::Status);
        let previous = icon_button("Previous match", crate::assets::icons::ARROW_UP);
        let next = icon_button("Next match", crate::assets::icons::ARROW_DOWN);
        let close = icon_button("Close find", crate::assets::icons::X);
        widget.append(&entry);
        widget.append(&status);
        widget.append(&previous);
        widget.append(&next);
        widget.append(&close);
        Self {
            widget,
            entry,
            status,
            previous,
            next,
            close,
            target: RefCell::new(None),
            source_subscription: RefCell::new(None),
            matches: RefCell::new(Vec::new()),
            current: Cell::new(0),
        }
    }

    fn recompute(&self) {
        let query = self.entry.text();
        let target = self.target.borrow();
        if let Some(target) = target.as_ref() {
            target.clear_selection();
        }
        let matches = target
            .as_ref()
            .map_or_else(Vec::new, |target| target.matches(&query));
        self.matches.replace(matches);
        self.current.set(0);
        drop(target);
        self.select_current();
    }

    fn select_current(&self) {
        let matches = self.matches.borrow();
        let count = matches.len();
        self.previous.set_sensitive(count > 0);
        self.next.set_sensitive(count > 0);
        self.status.set_visible(!self.entry.text().is_empty());
        let message = if count == 0 {
            crate::i18n::tr("No matches")
        } else {
            rust_i18n::t!(
                "Match %{current} of %{total}",
                current = crate::i18n::integer((self.current.get() + 1) as u64),
                total = crate::i18n::integer(count as u64)
            )
            .into_owned()
        };
        self.status.set_text(&message);
        if let Some(found) = matches.get(self.current.get())
            && let Some(target) = self.target.borrow().as_ref()
        {
            target.select(*found);
        }
    }

    fn step(&self, backward: bool) {
        let count = self.matches.borrow().len();
        if count == 0 {
            return;
        }
        self.current.set(if backward {
            (self.current.get() + count - 1) % count
        } else {
            (self.current.get() + 1) % count
        });
        self.select_current();
    }

    fn disconnect_source(&self) {
        if let Some((buffer, handler)) = self.source_subscription.borrow_mut().take() {
            buffer.disconnect(handler);
        }
    }

    fn reset(&self) {
        self.disconnect_source();
        self.target.borrow_mut().take();
        self.matches.borrow_mut().clear();
        self.widget.set_visible(false);
        self.entry.set_text("");
    }
}

impl PreviewState {
    pub(super) fn install_find(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.find_button.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.open_find();
            }
        });
        let weak = Rc::downgrade(self);
        self.find.entry.connect_changed(move |_| {
            if let Some(state) = weak.upgrade() {
                state.find.recompute();
            }
        });
        for (button, backward) in [(&self.find.previous, true), (&self.find.next, false)] {
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.find.step(backward);
                }
            });
        }
        let weak = Rc::downgrade(self);
        self.find.close.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.close_find();
            }
        });
    }

    fn find_target(&self) -> Option<Target> {
        if let Some(preview) = self.document_preview.borrow().as_ref()
            && self.document_view.get() == DocumentView::Rendered
        {
            return preview
                .rendered_state
                .borrow()
                .as_ref()
                .filter(|state| state.can_find())
                .cloned()
                .map(Target::Document);
        }
        if let Some(state) = self.rendered_text.borrow().as_ref() {
            return state.can_find().then(|| Target::Document(state.clone()));
        }
        if let Some(state) = self.source_preview.virtual_state.borrow().as_ref() {
            return Some(Target::Document(state.clone()));
        }
        self.source_preview
            .scroll
            .borrow()
            .as_ref()
            .map(|_| Target::Source(self.source_preview.view.clone()))
    }

    pub(super) fn refresh_find(self: &Rc<Self>) {
        let target = self.find_target();
        let buffer = match &target {
            Some(Target::Source(view)) => Some(view.buffer()),
            _ => None,
        };
        let same_buffer = self
            .find
            .source_subscription
            .borrow()
            .as_ref()
            .is_some_and(|(subscribed, _)| Some(subscribed) == buffer.as_ref());
        if !same_buffer {
            self.find.disconnect_source();
            if let Some(buffer) = buffer {
                let weak = Rc::downgrade(self);
                let handler = buffer.connect_changed(move |_| {
                    if let Some(state) = weak.upgrade()
                        && state.find.widget.is_visible()
                    {
                        state.find.recompute();
                    }
                });
                self.find
                    .source_subscription
                    .replace(Some((buffer, handler)));
            }
        }
        self.find_button.set_visible(target.is_some());
        self.find.target.replace(target);
        if self.find.widget.is_visible() {
            self.find.recompute();
        }
    }

    fn open_find(self: &Rc<Self>) -> bool {
        if self.find_target().is_none() {
            return false;
        }
        self.find.widget.set_visible(true);
        self.refresh_find();
        self.find.entry.grab_focus();
        self.find.entry.select_region(0, -1);
        true
    }

    fn close_find(self: &Rc<Self>) {
        self.find.widget.set_visible(false);
        PreviewDrawer {
            state: self.clone(),
        }
        .take_keyboard();
    }

    pub(super) fn reset_find(&self) {
        self.find.reset();
    }

    pub(super) fn detach_find(&self) {
        self.find.disconnect_source();
        self.find.target.borrow_mut().take();
        self.find.matches.borrow_mut().clear();
        self.find.status.set_visible(false);
        self.find.previous.set_sensitive(false);
        self.find.next.set_sensitive(false);
        self.find_button.set_visible(false);
    }
}

impl PreviewDrawer {
    pub(in crate::ui) fn bind_find_shortcut(
        &self,
        preferences: &super::super::preferences::PreferenceManager,
    ) {
        let button = self.state.find_button.downgrade();
        preferences.bind_preference(
            &self.state.find_button,
            super::super::preferences::PreferenceManager::tenxer_mode,
            move |_, tenxer| {
                let Some(button) = button.upgrade() else {
                    return;
                };
                let label = rust_i18n::t!(
                    "Find in preview (%{shortcut})",
                    shortcut = if tenxer { "/" } else { "Ctrl+F" }
                );
                button.set_tooltip_text(Some(&label));
                super::super::accessibility::set_label(&button, &label);
            },
        );
    }

    /// 10xer opens find with `/` so Ctrl+F keeps paging the document.
    pub(in crate::ui) fn handle_find_key(
        &self,
        key: gtk::gdk::Key,
        modifiers: gtk::gdk::ModifierType,
        tenxer: bool,
    ) -> bool {
        use gtk::gdk::{Key, ModifierType as Modifiers};
        let focused = self.state.pane.root().and_then(|root| root.focus());
        if !self.owns_focus(focused.as_ref())
            || modifiers.intersects(Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
        {
            return false;
        }
        let control = modifiers.contains(Modifiers::CONTROL_MASK);
        let shift = modifiers.contains(Modifiers::SHIFT_MASK);
        let in_find = focused
            .as_ref()
            .is_some_and(|focus| focus.is_ancestor(&self.state.find.widget));
        let opens = if tenxer {
            !control && !in_find && matches!(key, Key::slash | Key::KP_Divide)
        } else {
            control && !shift && matches!(key, Key::f | Key::F)
        };
        if opens {
            return self.state.open_find();
        }
        if !self.state.find.widget.is_visible() {
            return false;
        }
        if key == Key::Escape && !control && !shift {
            self.state.close_find();
            return true;
        }
        if in_find && !control && matches!(key, Key::Return | Key::KP_Enter) {
            self.state.find.step(shift);
            return true;
        }
        false
    }
}
