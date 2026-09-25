// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};

use super::browser_modes::BrowserMode;

type Shortcut = (&'static str, &'static str);

#[derive(Clone)]
pub(super) struct ShortcutFooter {
    root: gtk::Box,
    paste: gtk::Label,
    count: gtk::Label,
    show_hints: Rc<Cell<bool>>,
    pending_popup: Rc<Cell<bool>>,
    more: gtk::MenuButton,
    popover: gtk::Popover,
    reference: gtk::Box,
    focus_before: Rc<RefCell<Option<glib::WeakRef<gtk::Widget>>>>,
    status_widgets: Rc<RefCell<Vec<gtk::Widget>>>,
    tag: gtk::Label,
    tag_note: gtk::Label,
    experimental: gtk::Label,
    feedback: gtk::Label,
    prompt: gtk::Entry,
    chord: gtk::Label,
    view_mode: Rc<Cell<BrowserMode>>,
}

impl ShortcutFooter {
    pub fn new(mode: BrowserMode) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.add_css_class("shortcut-footer");
        let count = gtk::Label::new(None);
        count.add_css_class("shortcut-footer-count");
        count.set_visible(false);
        let paste = gtk::Label::new(Some("Files on clipboard"));
        paste.add_css_class("shortcut-footer-paste");
        paste.set_tooltip_text(Some("Press Ctrl+V to paste into a supported directory."));
        paste.set_visible(false);
        let tag = gtk::Label::new(Some(crate::ui::omastrata_mode::TAG_TEXT));
        tag.add_css_class("omastrata-tag");
        tag.set_tooltip_text(Some(crate::ui::omastrata_mode::TAG_NAME));
        super::accessibility::set_label(&tag, crate::ui::omastrata_mode::TAG_NAME);
        tag.set_visible(false);
        let tag_note = gtk::Label::new(None);
        tag_note.add_css_class("omastrata-experimental");
        tag_note.set_ellipsize(gtk::pango::EllipsizeMode::End);
        tag_note.set_max_width_chars(28);
        tag_note.set_visible(false);
        let chord = gtk::Label::new(None);
        chord.add_css_class("shortcut-footer-chord");
        chord.set_visible(false);
        let prompt = gtk::Entry::new();
        prompt.add_css_class("form-control");
        prompt.add_css_class("shortcut-footer-prompt");
        prompt.set_width_chars(12);
        prompt.set_hexpand(false);
        prompt.set_visible(false);
        super::accessibility::set_label(&prompt, "Command prompt");
        let feedback = gtk::Label::new(None);
        feedback.add_css_class("shortcut-footer-feedback");
        feedback.set_visible(false);
        root.append(&paste);
        root.append(&tag);
        root.append(&tag_note);
        root.append(&chord);
        root.append(&prompt);
        root.append(&feedback);
        root.append(&count);
        let show_hints = Rc::new(Cell::new(true));
        let pending_popup = Rc::new(Cell::new(false));

        let more = gtk::MenuButton::new();
        more.set_child(Some(&gtk::Label::new(Some("F1  Shortcuts"))));
        more.add_css_class("shortcut-footer-button");
        more.set_tooltip_text(Some("Show all file-view shortcuts (F1)"));
        root.prepend(&more);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        root.insert_child_after(&spacer, Some(&more));
        let popover = gtk::Popover::builder()
            .position(gtk::PositionType::Top)
            .halign(gtk::Align::Start)
            .has_arrow(false)
            .build();
        popover.add_css_class("shortcut-popover");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let title = gtk::Label::builder()
            .label("Keyboard shortcuts")
            .xalign(0.0)
            .hexpand(true)
            .build();
        title.add_css_class("shortcut-reference-title");
        let close = gtk::Button::with_label("Close");
        close.add_css_class("shortcut-reference-close");
        let weak = popover.downgrade();
        close.connect_clicked(move |_| {
            if let Some(popover) = weak.upgrade() {
                popover.popdown();
            }
        });
        header.append(&title);
        header.append(&close);
        content.append(&header);
        let note = gtk::Label::builder()
            .label("Media controls use Ctrl+Alt. Plain keys keep browsing; text fields and dialogs keep native controls.")
            .xalign(0.0).wrap(true).build();
        note.add_css_class("shortcut-reference-note");
        content.append(&note);
        let experimental = gtk::Label::new(None);
        experimental.add_css_class("shortcut-reference-note");
        experimental.add_css_class("omastrata-experimental");
        experimental.set_xalign(0.0);
        experimental.set_wrap(true);
        experimental.set_visible(false);
        content.append(&experimental);
        let reference = gtk::Box::new(gtk::Orientation::Vertical, 16);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&reference)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .max_content_height(440)
            .width_request(420)
            .focusable(true)
            .build();
        content.append(&scroll);
        popover.set_child(Some(&content));
        let weak_scroll = scroll.downgrade();
        popover.connect_show(move |popover| {
            if let Some(scroll) = weak_scroll.upgrade()
                && let Some(window) = popover.root().and_downcast::<gtk::Window>()
            {
                scroll.vadjustment().set_value(scroll.vadjustment().lower());
                scroll.set_max_content_height((window.height() - 150).clamp(100, 440));
                scroll.set_width_request((window.width() - 60).clamp(260, 420));
            }
        });
        more.set_popover(Some(&popover));
        let focus_before: Rc<RefCell<Option<glib::WeakRef<gtk::Widget>>>> =
            Rc::new(RefCell::new(None));
        let restored_focus = focus_before.clone();
        let weak_more = more.downgrade();
        let closed_hints = show_hints.clone();
        let closed_pending = pending_popup.clone();
        let weak_popover = popover.downgrade();
        popover.connect_closed(move |_| {
            let restored_focus = restored_focus.clone();
            let closed_pending = closed_pending.clone();
            let weak_more = weak_more.clone();
            let closed_hints = closed_hints.clone();
            let weak_popover = weak_popover.clone();
            // MenuButton restores its own focus after ::closed; wait without overriding a newer focus move.
            glib::idle_add_local_once(move || {
                if closed_pending.get()
                    || weak_popover
                        .upgrade()
                        .is_some_and(|popover| popover.is_visible())
                {
                    return;
                }
                let previous = restored_focus.borrow_mut().take();
                let Some(more) = weak_more.upgrade() else {
                    return;
                };
                let still_on_button =
                    more.root()
                        .and_then(|root| root.focus())
                        .is_some_and(|focused| {
                            focused == *more.upcast_ref::<gtk::Widget>()
                                || focused.is_ancestor(&more)
                        });
                if still_on_button
                    && let Some(previous) = previous.and_then(|previous| previous.upgrade())
                    && previous.is_mapped()
                {
                    previous.grab_focus();
                }
                more.set_visible(closed_hints.get());
            });
        });
        let status_widgets: Rc<RefCell<Vec<gtk::Widget>>> = Rc::new(RefCell::new(vec![
            paste.clone().upcast::<gtk::Widget>(),
            count.clone().upcast(),
            more.clone().upcast(),
            tag.clone().upcast(),
            tag_note.clone().upcast(),
            chord.clone().upcast(),
            prompt.clone().upcast(),
            feedback.clone().upcast(),
        ]));
        for widget in status_widgets.borrow().iter() {
            watch_status_widget(widget, &status_widgets, &root);
        }
        let footer = Self {
            root,
            paste,
            count,
            show_hints,
            pending_popup,
            more,
            popover,
            reference,
            focus_before,
            status_widgets,
            tag,
            tag_note,
            experimental,
            feedback,
            prompt,
            chord,
            view_mode: Rc::new(Cell::new(mode)),
        };
        footer.set_mode(mode);
        footer
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn set_activity(&self, widget: &impl IsA<gtk::Widget>) {
        let widget = widget.as_ref().clone();
        self.root.insert_child_after(&widget, Some(&self.count));
        self.status_widgets.borrow_mut().push(widget.clone());
        watch_status_widget(&widget, &self.status_widgets, &self.root);
    }

    pub fn bind_preferences(&self, manager: &super::preferences::PreferenceManager) {
        let tag = self.tag.clone();
        let tag_note = self.tag_note.clone();
        let experimental = self.experimental.clone();
        let reference = self.reference.clone();
        let view_mode = self.view_mode.clone();
        let feedback = self.feedback.clone();
        let prompt = self.prompt.clone();
        let chord = self.chord.clone();
        let primed = Rc::new(Cell::new(false));
        manager.bind_preference(
            &self.root,
            super::preferences::PreferenceManager::omastrata_mode,
            move |_, enabled| {
                let starting = !primed.replace(true);
                apply_experimental_label(&tag, &tag_note, &experimental, enabled);
                if !starting && !enabled {
                    clear_transient(&feedback, &prompt, &chord);
                }
                rebuild_reference(&reference, view_mode.get());
            },
        );
        let show_hints = self.show_hints.clone();
        let pending = self.pending_popup.clone();
        let weak_popover = self.popover.downgrade();
        let more = self.more.downgrade();
        manager.on_keybinding_hints_changed(&self.root, move |_, enabled| {
            show_hints.set(enabled);
            if !enabled {
                pending.set(false);
            }
            if let Some(more) = more.upgrade() {
                more.set_visible(
                    enabled
                        || weak_popover
                            .upgrade()
                            .is_some_and(|popover| popover.is_visible()),
                );
            }
        });
    }

    pub fn connect_clipboard(&self, clipboard: &gdk::Clipboard) -> glib::SignalHandlerId {
        let label = self.paste.downgrade();
        let generation = Rc::new(Cell::new(0));
        refresh_paste_availability(clipboard, &label, &generation);
        clipboard.connect_changed(move |clipboard| {
            refresh_paste_availability(clipboard, &label, &generation);
        })
    }

    pub fn observe_browser(&self, browser: &Rc<crate::app::Browser>) {
        update_item_count(&self.count, browser);
        let label = self.count.downgrade();
        let weak_browser = Rc::downgrade(browser);
        browser.observe(move |_| {
            if let Some(label) = label.upgrade()
                && let Some(browser) = weak_browser.upgrade()
            {
                update_item_count(&label, &browser);
            }
        });
    }

    #[cfg(test)]
    pub(in crate::ui) fn tag_visible(&self) -> bool {
        self.tag.is_visible()
    }

    pub fn set_mode(&self, mode: BrowserMode) {
        self.view_mode.set(mode);
        rebuild_reference(&self.reference, mode);
    }

    #[cfg(test)]
    pub(in crate::ui) fn show_feedback(&self, text: &str) {
        self.feedback.set_text(text);
        self.feedback.set_visible(!text.is_empty());
    }

    #[cfg(test)]
    pub(in crate::ui) fn dismiss_feedback(&self) {
        self.feedback.set_text("");
        self.feedback.set_visible(false);
    }

    #[cfg(test)]
    pub(in crate::ui) fn show_prompt(&self) {
        self.prompt.set_visible(true);
        self.prompt.set_sensitive(true);
    }

    #[cfg(test)]
    pub(in crate::ui) fn prompt(&self) -> &gtk::Entry {
        &self.prompt
    }

    pub(in crate::ui) fn dismiss_prompt(&self) {
        self.prompt.set_text("");
        self.prompt.set_visible(false);
    }

    #[cfg(test)]
    pub(in crate::ui) fn arm_chord(&self, mark: &str) {
        self.chord.set_text(mark);
        self.chord.set_visible(!mark.is_empty());
    }

    #[cfg(test)]
    pub(in crate::ui) fn chord(&self) -> &gtk::Label {
        &self.chord
    }

    pub(in crate::ui) fn prompt_has_focus(&self) -> bool {
        gtk::prelude::WidgetExt::is_visible(&self.prompt)
            && self
                .root
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focus| {
                    focus == *self.prompt.upcast_ref::<gtk::Widget>()
                        || focus.is_ancestor(&self.prompt)
                })
    }

    pub fn handle_key(
        &self,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> Option<glib::Propagation> {
        let command_modifiers = modifiers.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK,
        );
        let reference_open = self.popover.is_visible() || self.pending_popup.get();
        let f1 = key == gdk::Key::F1
            && !command_modifiers
            && !modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let tilde = self.tilde_toggles(key, modifiers, reference_open);
        if self.prompt_has_focus() && !f1 && !tilde && !reference_open {
            if key == gdk::Key::Escape && !command_modifiers {
                self.dismiss_prompt();
                return Some(glib::Propagation::Stop);
            }
            return None;
        }
        if f1 || tilde {
            if self.popover.is_visible() || self.pending_popup.replace(false) {
                if self.popover.is_visible() {
                    self.more.popdown();
                } else {
                    self.focus_before.take();
                    self.more.set_visible(self.show_hints.get());
                }
            } else {
                if self.focus_before.borrow().is_none() {
                    self.focus_before.replace(
                        self.root
                            .root()
                            .and_then(|root| root.focus())
                            .map(|widget| widget.downgrade()),
                    );
                }
                if self.more.is_mapped() && self.more.width() > 0 {
                    self.more.popup();
                } else {
                    self.pending_popup.set(true);
                    self.more.set_visible(true);
                    let pending = self.pending_popup.clone();
                    let weak_more = self.more.downgrade();
                    // A hidden shortcut button needs an allocation before positioning the popover.
                    self.root.add_tick_callback(move |_, _| {
                        let Some(more) = weak_more.upgrade() else {
                            return glib::ControlFlow::Break;
                        };
                        if !pending.get() {
                            return glib::ControlFlow::Break;
                        }
                        if !more.is_mapped() || more.width() == 0 {
                            return glib::ControlFlow::Continue;
                        }
                        pending.set(false);
                        more.popup();
                        glib::ControlFlow::Break
                    });
                }
            }
            return Some(glib::Propagation::Stop);
        }
        if self.pending_popup.get() {
            if key == gdk::Key::Escape {
                self.pending_popup.set(false);
                self.focus_before.take();
                self.more.set_visible(self.show_hints.get());
            }
            return Some(glib::Propagation::Stop);
        }
        if !self.popover.is_visible() {
            return None;
        }
        if key == gdk::Key::Escape {
            self.more.popdown();
            return Some(glib::Propagation::Stop);
        }
        // The reference is read-only: never let a shortcut operate on files behind it.
        Some(
            if !command_modifiers
                && matches!(
                    key,
                    gdk::Key::Tab
                        | gdk::Key::ISO_Left_Tab
                        | gdk::Key::Up
                        | gdk::Key::Down
                        | gdk::Key::Left
                        | gdk::Key::Right
                        | gdk::Key::Page_Up
                        | gdk::Key::Page_Down
                        | gdk::Key::Home
                        | gdk::Key::End
                        | gdk::Key::Return
                        | gdk::Key::KP_Enter
                        | gdk::Key::space
                )
            {
                glib::Propagation::Proceed
            } else {
                glib::Propagation::Stop
            },
        )
    }

    fn tilde_toggles(
        &self,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
        reference_open: bool,
    ) -> bool {
        if !super::preferences::PreferenceManager::shared().omastrata_mode() {
            return false;
        }
        if modifiers.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK,
        ) {
            return false;
        }
        let tilde = key == gdk::Key::asciitilde
            || (key == gdk::Key::grave && modifiers.contains(gdk::ModifierType::SHIFT_MASK));
        tilde && (reference_open || !self.prompt_has_focus())
    }
}

fn rebuild_reference(reference: &gtk::Box, mode: BrowserMode) {
    while let Some(child) = reference.first_child() {
        reference.remove(&child);
    }
    for section in super::shortcut_reference::reference_sections(mode) {
        append_section(reference, section.title, &section.rows);
    }
}

fn apply_experimental_label(
    tag: &gtk::Label,
    tag_note: &gtk::Label,
    reference_note: &gtk::Label,
    enabled: bool,
) {
    tag.set_text(crate::ui::omastrata_mode::TAG_TEXT);
    tag.set_visible(enabled);
    let phrase = super::shortcut_reference::EXPERIMENTAL_LABEL;
    tag_note.set_text(if enabled { phrase } else { "" });
    tag_note.set_visible(enabled);
    reference_note.set_text(if enabled { phrase } else { "" });
    reference_note.set_visible(enabled);
    let announced = if enabled {
        format!("{} {phrase}", crate::ui::omastrata_mode::TAG_NAME)
    } else {
        crate::ui::omastrata_mode::TAG_NAME.to_owned()
    };
    tag.set_tooltip_text(Some(&announced));
    tag.update_property(&[
        gtk::accessible::Property::Label(&announced),
        gtk::accessible::Property::Description(if enabled { phrase } else { "" }),
    ]);
}

fn clear_transient(feedback: &gtk::Label, prompt: &gtk::Entry, chord: &gtk::Label) {
    feedback.set_text("");
    feedback.set_visible(false);
    prompt.set_text("");
    prompt.set_visible(false);
    chord.set_text("");
    chord.set_visible(false);
}

fn update_item_count(label: &gtk::Label, browser: &Rc<crate::app::Browser>) {
    let Some(depth) = browser.active_depth() else {
        label.set_visible(false);
        return;
    };
    let counts = browser.column_entry_counts(depth).unwrap_or_default();
    let selected = browser.selected_entries();
    for position in browser.selected_positions(depth) {
        if let Some(entry) = browser.entry_at(depth, position)
            && !entry.is_directory()
            && entry.size == crate::model::MetadataValue::Unknown
        {
            browser.request_metadata_fill(depth, position, entry.location, false);
        }
    }
    let noun = if counts.total == 1 { "item" } else { "items" };
    if !selected.is_empty() {
        label.set_label(&selection_details(&selected));
        label.set_tooltip_text(Some(&format!(
            "{} of {} {noun} selected. Size includes selected files only; folder contents are not counted.",
            selected.len(), counts.total
        )));
    } else {
        label.set_label(&format!("{} {noun}", counts.total));
        let files = if counts.files == 1 { "file" } else { "files" };
        let folders = if counts.folders == 1 {
            "folder"
        } else {
            "folders"
        };
        label.set_tooltip_text(Some(&format!(
            "{} {files}, {} {folders}",
            counts.files, counts.folders
        )));
    }
    label.set_visible(true);
}

fn selection_details(entries: &[crate::model::FileEntry]) -> String {
    let folders = entries.iter().filter(|entry| entry.is_directory()).count();
    let files = entries.len() - folders;
    let mut parts = Vec::new();
    if folders > 0 {
        let noun = if folders == 1 { "folder" } else { "folders" };
        parts.push(format!("{folders} {noun}"));
    }
    if files > 0 {
        let noun = if files == 1 { "file" } else { "files" };
        parts.push(format!("{files} {noun}"));
    }
    let mut text = format!("{} selected", parts.join(", "));
    if files > 0 {
        let mut bytes = 0u64;
        let mut known = 0;
        for entry in entries.iter().filter(|entry| !entry.is_directory()) {
            if let crate::model::MetadataValue::Known(size) = entry.size {
                bytes = bytes.saturating_add(size);
                known += 1;
            }
        }
        let size = super::browser::format_file_size(bytes);
        if known == files {
            text.push_str(&format!(" ({size})"));
        } else if known > 0 {
            text.push_str(&format!(" ({size} known; size incomplete)"));
        } else {
            text.push_str(" (size unavailable)");
        }
    }
    text
}

fn watch_status_widget(
    widget: &gtk::Widget,
    status_widgets: &Rc<RefCell<Vec<gtk::Widget>>>,
    root: &gtk::Box,
) {
    let root = root.downgrade();
    let statuses = status_widgets.clone();
    widget.connect_visible_notify(move |_| {
        // Ignore ancestor visibility so a hidden footer can reveal itself.
        if let Some(root) = root.upgrade() {
            root.set_visible(
                statuses
                    .borrow()
                    .iter()
                    .any(gtk::prelude::WidgetExt::get_visible),
            );
        }
    });
}

fn refresh_paste_availability(
    clipboard: &gdk::Clipboard,
    label: &glib::WeakRef<gtk::Label>,
    generation: &Rc<Cell<u64>>,
) {
    let revision = generation.get().wrapping_add(1);
    generation.set(revision);
    let Some(paste) = label.upgrade() else {
        return;
    };
    paste.set_visible(false);
    let formats = clipboard.formats();
    if !formats.contains_type(gdk::FileList::static_type())
        && !formats.contain_mime_type("text/uri-list")
    {
        return;
    }
    let clipboard = clipboard.clone();
    let label = label.clone();
    let generation = generation.clone();
    glib::MainContext::default().spawn_local(async move {
        let available = clipboard
            .read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT)
            .await
            .ok()
            .and_then(|value| value.get::<gdk::FileList>().ok())
            .is_some_and(|files| !files.files().is_empty());
        if revision == generation.get()
            && let Some(label) = label.upgrade()
        {
            label.set_visible(available);
        }
    });
}

fn append_section(parent: &gtk::Box, title: &str, shortcuts: &[Shortcut]) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 7);
    let heading = gtk::Label::builder().label(title).xalign(0.0).build();
    heading.add_css_class("shortcut-reference-heading");
    section.append(&heading);
    for (key, action) in shortcuts {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let key = gtk::Label::builder()
            .label(*key)
            .xalign(0.0)
            .width_chars(17)
            .build();
        key.add_css_class("shortcut-reference-key");
        let action = gtk::Label::builder()
            .label(*action)
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .build();
        action.add_css_class("shortcut-reference-description");
        row.append(&key);
        row.append(&action);
        section.append(&row);
    }
    parent.append(&section);
}

#[cfg(test)]
mod tests;
