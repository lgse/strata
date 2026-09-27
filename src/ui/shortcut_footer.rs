// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

use super::{browser_modes::BrowserMode, tenxer_mode::Chord};

type Shortcut = (&'static str, &'static str);
type ChordListener = Box<dyn Fn(Option<Chord>)>;

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
    categories: gtk::Box,
    reference_count: gtk::Label,
    search: gtk::SearchEntry,
    selected_category: Rc<RefCell<String>>,
    scroll: gtk::ScrolledWindow,
    focus_before: Rc<RefCell<Option<glib::WeakRef<gtk::Widget>>>>,
    status_widgets: Rc<RefCell<Vec<gtk::Widget>>>,
    tag: gtk::Label,
    experimental: gtk::Label,
    feedback: gtk::Label,
    feedback_epoch: Rc<Cell<u64>>,
    prompt: gtk::Entry,
    chords: ChordIndicator,
    visual: gtk::Label,
    view_mode: Rc<Cell<BrowserMode>>,
}

/// The armed chord and its footer mark. The chord is armed exactly while the
/// mark is showing.
#[derive(Clone)]
struct ChordIndicator {
    armed: Rc<Cell<Option<Chord>>>,
    mark: glib::WeakRef<gtk::Label>,
    hint: glib::WeakRef<gtk::Label>,
    listeners: Rc<RefCell<Vec<ChordListener>>>,
}

impl ChordIndicator {
    fn set(&self, chord: Option<Chord>) {
        if let Some(mark) = self.mark.upgrade() {
            mark.set_text(chord.map_or("", Chord::mark));
            mark.set_visible(chord.is_some());
        }
        if let Some(hint) = self.hint.upgrade() {
            hint.set_text(chord.map_or("", Chord::hint));
            hint.set_visible(chord.is_some());
        }
        if self.armed.replace(chord) != chord {
            for listener in self.listeners.borrow().iter() {
                listener(chord);
            }
        }
    }
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
        let tag = gtk::Label::new(Some(crate::ui::tenxer_mode::TAG_TEXT));
        tag.add_css_class("tenxer-tag");
        tag.set_tooltip_text(Some(crate::ui::tenxer_mode::TAG_NAME));
        super::accessibility::set_label(&tag, crate::ui::tenxer_mode::TAG_NAME);
        tag.set_visible(false);
        let chord = gtk::Label::new(None);
        chord.add_css_class("shortcut-footer-chord");
        chord.set_visible(false);
        let chord_hint = gtk::Label::new(None);
        chord_hint.add_css_class("shortcut-footer-chord-hint");
        chord_hint.set_ellipsize(gtk::pango::EllipsizeMode::End);
        chord_hint.set_visible(false);
        let chords = ChordIndicator {
            armed: Rc::new(Cell::new(None)),
            mark: chord.downgrade(),
            hint: chord_hint.downgrade(),
            listeners: Rc::new(RefCell::new(Vec::new())),
        };
        let visual = gtk::Label::new(None);
        visual.add_css_class("shortcut-footer-chord");
        visual.set_visible(false);
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
        root.append(&visual);
        root.append(&chord);
        root.append(&chord_hint);
        // Transient marks grow leftward so the pill stays put.
        root.append(&tag);
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
            .halign(gtk::Align::Center)
            .has_arrow(false)
            .build();
        popover.add_css_class("shortcut-popover");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("shortcut-reference-header");
        let title = gtk::Label::new(Some("Keyboard shortcuts"));
        title.add_css_class("shortcut-reference-title");
        header.append(&title);
        let dismiss_note = gtk::Label::new(Some("F1 to close"));
        dismiss_note.add_css_class("shortcut-reference-note");
        header.append(&dismiss_note);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search actions or keys…"));
        search.set_hexpand(true);
        search.add_css_class("shortcut-reference-search");
        header.append(&search);
        content.append(&header);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.add_css_class("shortcut-reference-body");
        let categories = gtk::Box::new(gtk::Orientation::Vertical, 0);
        categories.add_css_class("shortcut-reference-categories");
        body.append(&categories);
        let reference = gtk::Box::new(gtk::Orientation::Vertical, 24);
        reference.add_css_class("shortcut-reference-results");
        let selected_category = Rc::new(RefCell::new(String::from("All")));
        let scroll = gtk::ScrolledWindow::builder()
            .child(&reference)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .max_content_height(620)
            .focusable(true)
            .build();
        scroll.add_css_class("shortcut-reference-scroll");
        body.append(&scroll);
        content.append(&body);
        let footer_note = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer_note.add_css_class("shortcut-reference-footer");
        let count_note = gtk::Label::new(None);
        count_note.add_css_class("shortcut-reference-note");
        footer_note.append(&count_note);
        let note = gtk::Label::new(Some(
            "Media controls use Ctrl+Alt. Text fields and dialogs keep native controls.",
        ));
        note.add_css_class("shortcut-reference-note");
        note.set_hexpand(true);
        note.set_xalign(0.0);
        footer_note.append(&note);
        let experimental = gtk::Label::new(None);
        experimental.add_css_class("shortcut-reference-note");
        experimental.add_css_class("tenxer-experimental");
        experimental.set_visible(false);
        footer_note.append(&experimental);
        content.append(&footer_note);
        popover.set_child(Some(&content));
        let search_reference = reference.downgrade();
        let view_mode = Rc::new(Cell::new(mode));
        let search_mode = view_mode.clone();
        let search_category = selected_category.clone();
        search.connect_search_changed(move |entry| {
            if let Some(reference) = search_reference.upgrade() {
                render_reference(
                    &reference,
                    search_mode.get(),
                    &search_category.borrow(),
                    &entry.text(),
                );
            }
        });
        let weak_scroll = scroll.downgrade();
        let weak_popover = popover.downgrade();
        let weak_search = search.downgrade();
        popover.connect_show(move |popover| {
            let Some(scroll) = weak_scroll.upgrade() else {
                return;
            };
            if let Some(window) = popover.root().and_downcast::<gtk::Window>() {
                scroll.vadjustment().set_value(scroll.vadjustment().lower());
                scroll.set_max_content_height((window.height() - 160).clamp(140, 620));
                scroll.set_width_request((window.width() - 260).clamp(200, 1200));
            }
            let search = weak_search.clone();
            let popover = weak_popover.clone();
            // Show runs before the popover can take focus; grab it once mapped.
            glib::idle_add_local_once(move || {
                if popover
                    .upgrade()
                    .is_some_and(|popover| popover.is_visible())
                    && let Some(search) = search.upgrade()
                {
                    search.grab_focus();
                }
            });
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
            chord.clone().upcast(),
            chord_hint.clone().upcast(),
            visual.clone().upcast(),
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
            categories,
            reference_count: count_note,
            search,
            selected_category,
            scroll,
            focus_before,
            status_widgets,
            tag,
            experimental,
            feedback,
            feedback_epoch: Rc::new(Cell::new(0)),
            prompt,
            chords,
            visual,
            view_mode,
        };
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let shortcuts = footer.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            shortcuts
                .handle_key(key, modifiers)
                .unwrap_or(glib::Propagation::Proceed)
        });
        footer.popover.add_controller(keys);
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
        let tag = self.tag.downgrade();
        let experimental = self.experimental.downgrade();
        let reference = self.reference.downgrade();
        let categories = self.categories.downgrade();
        let reference_count = self.reference_count.downgrade();
        let search = self.search.downgrade();
        let selected = self.selected_category.clone();
        let scroll = self.scroll.downgrade();
        let view_mode = self.view_mode.clone();
        let feedback = self.feedback.downgrade();
        let prompt = self.prompt.downgrade();
        let chords = self.chords.clone();
        let primed = Rc::new(Cell::new(false));
        manager.bind_preference(
            &self.root,
            super::preferences::PreferenceManager::tenxer_mode,
            move |_, enabled| {
                let Some(tag) = tag.upgrade() else {
                    return;
                };
                let Some(experimental) = experimental.upgrade() else {
                    return;
                };
                let Some(reference) = reference.upgrade() else {
                    return;
                };
                let starting = !primed.replace(true);
                apply_experimental_label(&tag, &experimental, enabled);
                if !starting
                    && !enabled
                    && let Some(feedback) = feedback.upgrade()
                    && let Some(prompt) = prompt.upgrade()
                {
                    clear_transient(&feedback, &prompt);
                    chords.set(None);
                }
                if let (Some(categories), Some(search), Some(scroll), Some(count)) = (
                    categories.upgrade(),
                    search.upgrade(),
                    scroll.upgrade(),
                    reference_count.upgrade(),
                ) {
                    rebuild_reference(
                        &reference,
                        &categories,
                        &search,
                        &scroll,
                        &count,
                        &selected,
                        &view_mode,
                    );
                }
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
        update_visual_mode(&self.visual, browser);
        let label = self.count.downgrade();
        let visual = self.visual.downgrade();
        let weak_browser = Rc::downgrade(browser);
        let footer = self.clone();
        browser.observe(move |event| {
            if matches!(
                event,
                crate::app::BrowserEvent::NavigationStarting
                    | crate::app::BrowserEvent::SelectionSetChanged { .. }
            ) {
                footer.clear_feedback();
            }
            let Some(browser) = weak_browser.upgrade() else {
                return;
            };
            if let Some(label) = label.upgrade() {
                update_item_count(&label, &browser);
            }
            if let Some(visual) = visual.upgrade() {
                update_visual_mode(&visual, &browser);
            }
        });
    }

    #[cfg(test)]
    pub(in crate::ui) fn tag_visible(&self) -> bool {
        self.tag.is_visible()
    }

    pub fn set_mode(&self, mode: BrowserMode) {
        self.view_mode.set(mode);
        rebuild_reference(
            &self.reference,
            &self.categories,
            &self.search,
            &self.scroll,
            &self.reference_count,
            &self.selected_category,
            &self.view_mode,
        );
    }

    pub(in crate::ui) fn show_feedback(&self, text: &str) {
        let epoch = self.feedback_epoch.get().wrapping_add(1);
        self.feedback_epoch.set(epoch);
        self.feedback.set_text(text);
        let visible = !text.is_empty();
        self.feedback.set_visible(visible);
        if !visible {
            return;
        }
        let epochs = self.feedback_epoch.clone();
        let feedback = self.feedback.downgrade();
        glib::timeout_add_local_once(FEEDBACK_FLASH, move || {
            if epochs.get() != epoch {
                return;
            }
            if let Some(feedback) = feedback.upgrade() {
                feedback.set_text("");
                feedback.set_visible(false);
            }
        });
    }

    pub(in crate::ui) fn clear_feedback(&self) {
        self.feedback_epoch
            .set(self.feedback_epoch.get().wrapping_add(1));
        self.feedback.set_text("");
        self.feedback.set_visible(false);
    }

    #[cfg(test)]
    pub(in crate::ui) fn feedback_text(&self) -> String {
        self.feedback.text().to_string()
    }

    #[cfg(test)]
    pub(in crate::ui) fn dismiss_feedback(&self) {
        self.clear_feedback();
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

    pub(in crate::ui) fn arm_chord(&self, chord: Chord) {
        self.chords.set(Some(chord));
    }

    pub(in crate::ui) fn armed_chord(&self) -> Option<Chord> {
        self.chords.armed.get()
    }

    pub(in crate::ui) fn cancel_chord(&self) {
        self.chords.set(None);
    }

    pub(in crate::ui) fn connect_chord_changed(&self, listener: impl Fn(Option<Chord>) + 'static) {
        self.chords.listeners.borrow_mut().push(Box::new(listener));
    }

    #[cfg(test)]
    pub(in crate::ui) fn chord(&self) -> gtk::Label {
        self.chords.mark.upgrade().expect("chord mark")
    }

    #[cfg(test)]
    pub(in crate::ui) fn chord_hint(&self) -> Option<String> {
        let hint = self.chords.hint.upgrade()?;
        hint.is_visible().then(|| hint.text().to_string())
    }

    pub(in crate::ui) fn prompt_is_visible(&self) -> bool {
        gtk::prelude::WidgetExt::is_visible(&self.prompt)
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
        let search_focused = self
            .search
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focus| {
                focus == *self.search.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&self.search)
            });
        if search_focused
            && !command_modifiers
            && matches!(
                key,
                gdk::Key::Up | gdk::Key::Down | gdk::Key::Page_Up | gdk::Key::Page_Down
            )
            && self.scroll_reference(key)
        {
            return Some(glib::Propagation::Stop);
        }
        if search_focused
            && (!command_modifiers
                || (modifiers == gdk::ModifierType::CONTROL_MASK
                    && matches!(key, gdk::Key::a | gdk::Key::c | gdk::Key::v | gdk::Key::x)))
        {
            return Some(glib::Propagation::Proceed);
        }
        if !command_modifiers && self.scroll_reference(key) {
            return Some(glib::Propagation::Stop);
        }
        // The reference is read-only: never let a shortcut operate on files behind it.
        Some(
            if !command_modifiers
                && matches!(
                    key,
                    gdk::Key::Tab
                        | gdk::Key::ISO_Left_Tab
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

    fn scroll_reference(&self, key: gdk::Key) -> bool {
        let adjustment = self.scroll.vadjustment();
        let page = adjustment.page_size().max(1.0);
        let step = if adjustment.step_increment() >= 1.0 {
            adjustment.step_increment()
        } else {
            page / 10.0
        };
        let page_step = if adjustment.page_increment() >= 1.0 {
            adjustment.page_increment()
        } else {
            page
        };
        let delta = match key {
            gdk::Key::Up | gdk::Key::KP_Up | gdk::Key::Left | gdk::Key::KP_Left => -step,
            gdk::Key::Down | gdk::Key::KP_Down | gdk::Key::Right | gdk::Key::KP_Right => step,
            gdk::Key::Page_Up | gdk::Key::KP_Page_Up => -page_step,
            gdk::Key::Page_Down | gdk::Key::KP_Page_Down => page_step,
            _ => return false,
        };
        let limit = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        adjustment.set_value((adjustment.value() + delta).clamp(adjustment.lower(), limit));
        true
    }

    fn tilde_toggles(
        &self,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
        reference_open: bool,
    ) -> bool {
        if !super::preferences::PreferenceManager::shared().tenxer_mode() {
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

fn rebuild_reference(
    reference: &gtk::Box,
    categories: &gtk::Box,
    search: &gtk::SearchEntry,
    scroll: &gtk::ScrolledWindow,
    count_label: &gtk::Label,
    selected: &Rc<RefCell<String>>,
    mode: &Rc<Cell<BrowserMode>>,
) {
    while let Some(child) = categories.first_child() {
        categories.remove(&child);
    }
    let sections = super::shortcut_reference::reference_sections(mode.get());
    if !sections
        .iter()
        .any(|section| section.title == selected.borrow().as_str())
    {
        *selected.borrow_mut() = String::from("All");
    }
    let total: usize = sections.iter().map(|section| section.rows.len()).sum();
    count_label.set_text(&format!("{total} shortcuts"));
    for (name, count) in std::iter::once(("All", total)).chain(
        sections
            .iter()
            .map(|section| (section.title, section.rows.len())),
    ) {
        let button = gtk::Button::new();
        button.add_css_class("shortcut-reference-category");
        if *selected.borrow() == name {
            button.add_css_class("selected");
        }
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some(name));
        label.set_hexpand(true);
        label.set_xalign(0.0);
        line.append(&label);
        let amount = gtk::Label::new(Some(&count.to_string()));
        amount.add_css_class("shortcut-reference-count");
        line.append(&amount);
        button.set_child(Some(&line));
        let reference = reference.downgrade();
        let weak_categories = categories.downgrade();
        let search = search.downgrade();
        let scroll = scroll.downgrade();
        let count_label = count_label.downgrade();
        let selected = selected.clone();
        let mode = mode.clone();
        button.connect_clicked(move |_| {
            *selected.borrow_mut() = name.to_owned();
            if let (Some(reference), Some(categories), Some(search), Some(scroll), Some(count)) = (
                reference.upgrade(),
                weak_categories.upgrade(),
                search.upgrade(),
                scroll.upgrade(),
                count_label.upgrade(),
            ) {
                rebuild_reference(
                    &reference,
                    &categories,
                    &search,
                    &scroll,
                    &count,
                    &selected,
                    &mode,
                );
                scroll.vadjustment().set_value(0.0);
            }
        });
        categories.append(&button);
    }
    render_reference(reference, mode.get(), &selected.borrow(), &search.text());
}

fn render_reference(reference: &gtk::Box, mode: BrowserMode, selected: &str, query: &str) {
    while let Some(child) = reference.first_child() {
        reference.remove(&child);
    }
    let query = query.trim().to_lowercase();
    let mut found = false;
    for section in super::shortcut_reference::reference_sections(mode) {
        if selected != "All" && selected != section.title {
            continue;
        }
        let rows: Vec<_> = section
            .rows
            .into_iter()
            .filter(|(key, action)| {
                query.is_empty()
                    || key.to_lowercase().contains(&query)
                    || action.to_lowercase().contains(&query)
            })
            .collect();
        if !rows.is_empty() {
            found = true;
            append_section(reference, section.title, &rows);
        }
    }
    if !found {
        let empty = gtk::Label::new(Some("No matching shortcuts"));
        empty.add_css_class("shortcut-reference-note");
        reference.append(&empty);
    }
}

/// The footer shows only the pill; the experimental note lives in its tooltip,
/// accessible description, and the shortcut reference.
fn apply_experimental_label(tag: &gtk::Label, reference_note: &gtk::Label, enabled: bool) {
    tag.set_text(crate::ui::tenxer_mode::TAG_TEXT);
    tag.set_visible(enabled);
    let phrase = super::shortcut_reference::EXPERIMENTAL_LABEL;
    reference_note.set_text(if enabled { phrase } else { "" });
    reference_note.set_visible(enabled);
    let announced = if enabled {
        format!("{} {phrase}", crate::ui::tenxer_mode::TAG_NAME)
    } else {
        crate::ui::tenxer_mode::TAG_NAME.to_owned()
    };
    tag.set_tooltip_text(Some(&announced));
    tag.update_property(&[
        gtk::accessible::Property::Label(&announced),
        gtk::accessible::Property::Description(if enabled { phrase } else { "" }),
    ]);
}

const FEEDBACK_FLASH: Duration = Duration::from_millis(2_000);

fn clear_transient(feedback: &gtk::Label, prompt: &gtk::Entry) {
    feedback.set_text("");
    feedback.set_visible(false);
    prompt.set_text("");
    prompt.set_visible(false);
}

fn update_visual_mode(label: &gtk::Label, browser: &Rc<crate::app::Browser>) {
    let (text, name) = match browser.visual_kind() {
        Some(crate::app::VisualKind::Select) => ("VISUAL", "Visual select"),
        Some(crate::app::VisualKind::Unset) => ("UNSET", "Visual unset"),
        None => ("", ""),
    };
    if label.text() != text {
        label.set_text(text);
        super::accessibility::set_label(label, name);
    }
    label.set_visible(!text.is_empty());
}

#[cfg(test)]
impl ShortcutFooter {
    pub(in crate::ui) fn visual_text(&self) -> Option<String> {
        self.visual
            .is_visible()
            .then(|| self.visual.text().to_string())
    }
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
    let section = gtk::Box::new(gtk::Orientation::Vertical, 16);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(&title.to_uppercase()));
    label.add_css_class("shortcut-reference-heading");
    heading.append(&label);
    let count = gtk::Label::new(Some(&shortcuts.len().to_string()));
    count.add_css_class("shortcut-reference-count");
    heading.append(&count);
    let divider = gtk::Separator::new(gtk::Orientation::Horizontal);
    divider.set_hexpand(true);
    heading.append(&divider);
    section.append(&heading);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(48);
    grid.set_row_spacing(12);
    for (index, (key, action)) in shortcuts.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("shortcut-reference-row");
        let action = gtk::Label::builder()
            .label(*action)
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .build();
        action.add_css_class("shortcut-reference-description");
        row.append(&action);
        let keys = gtk::Label::new(Some(key));
        keys.add_css_class("shortcut-reference-key");
        row.append(&keys);
        grid.attach(&row, (index % 2) as i32, (index / 2) as i32, 1, 1);
    }
    section.append(&grid);
    parent.append(&section);
}

#[cfg(test)]
mod tests;
