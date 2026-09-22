// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};

use crate::services::SearchItem;

use super::{browser_modes::BrowserMode, minimal_mode::MinimalChord};

type Shortcut = (&'static str, &'static str);

#[derive(Clone)]
pub(super) struct ShortcutFooter(Rc<FooterState>);

#[derive(Clone)]
pub(super) struct WeakShortcutFooter(std::rc::Weak<FooterState>);

impl WeakShortcutFooter {
    pub(super) fn upgrade(&self) -> Option<ShortcutFooter> {
        self.0.upgrade().map(ShortcutFooter)
    }
}

impl std::ops::Deref for ShortcutFooter {
    type Target = FooterState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(super) struct FooterState {
    root: gtk::Box,
    stack: gtk::Stack,
    paste: gtk::Label,
    count: gtk::Label,
    min_tag: gtk::Label,
    filter_mark: gtk::Label,
    chord_label: gtk::Label,
    flash_label: gtk::Label,
    prompt_box: gtk::Box,
    prompt_prefix: gtk::Label,
    prompt_entry: gtk::Entry,
    prompt_spinner: gtk::Spinner,
    prompt_hint: gtk::Label,
    history_popover: gtk::Popover,
    chord_hints: gtk::Popover,
    chord_hint_list: gtk::Box,
    history_list: gtk::ListBox,
    history_scroller: gtk::ScrolledWindow,
    history_stack: gtk::Stack,
    history_empty: gtk::Label,
    history_items: Rc<RefCell<Vec<SearchItem>>>,
    prompt_changed: Rc<RefCell<Option<glib::SignalHandlerId>>>,
    flash_timeout: Rc<RefCell<Option<glib::SourceId>>>,
    show_hints: Rc<Cell<bool>>,
    pending_popup: Rc<Cell<bool>>,
    more: gtk::MenuButton,
    popover: gtk::Popover,
    reference: gtk::Box,
    focus_before: Rc<RefCell<Option<glib::WeakRef<gtk::Widget>>>>,
    mode: Cell<BrowserMode>,
    minimal: Rc<Cell<bool>>,
    status_widgets: Rc<RefCell<Vec<gtk::Widget>>>,
}

impl ShortcutFooter {
    pub fn new(mode: BrowserMode) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.add_css_class("shortcut-footer");
        let stack = gtk::Stack::builder()
            .hexpand(true)
            .transition_type(gtk::StackTransitionType::None)
            .build();
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        status.set_hexpand(true);
        let count = gtk::Label::new(None);
        count.add_css_class("shortcut-footer-count");
        count.set_visible(false);
        let paste = gtk::Label::new(Some("Files on clipboard"));
        paste.add_css_class("shortcut-footer-paste");
        paste.set_tooltip_text(Some("Press Ctrl+V to paste into a supported directory."));
        paste.set_visible(false);
        let min_tag = gtk::Label::new(Some("MIN"));
        min_tag.add_css_class("minimal-mode-tag");
        min_tag.set_tooltip_text(Some(&format!(
            "Minimal mode {}: Yazi-style keys. Press q to leave.",
            super::minimal_mode::EXPERIMENTAL_NOTE
        )));
        min_tag.set_visible(false);
        let filter_mark = gtk::Label::new(None);
        filter_mark.add_css_class("minimal-filter-mark");
        filter_mark.set_visible(false);
        let chord_label = gtk::Label::new(Some("g-"));
        chord_label.add_css_class("minimal-chord-label");
        chord_label.set_visible(false);
        let chord_hint_list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        chord_hint_list.add_css_class("minimal-chord-hints-list");
        let chord_hints = gtk::Popover::builder()
            .position(gtk::PositionType::Top)
            .autohide(false)
            .has_arrow(true)
            .build();
        chord_hints.set_can_focus(false);
        chord_hints.add_css_class("minimal-chord-hints");
        chord_hints.set_child(Some(&chord_hint_list));
        chord_hints.set_parent(&chord_label);
        super::accessibility::set_label(&chord_hints, "Chord options");
        let chord_hints_unparent = chord_hints.downgrade();
        chord_label.connect_destroy(move |_| {
            if let Some(popover) = chord_hints_unparent.upgrade() {
                popover.unparent();
            }
        });
        let flash_label = gtk::Label::new(None);
        flash_label.add_css_class("minimal-flash");
        flash_label.set_visible(false);
        status.append(&min_tag);
        status.append(&filter_mark);
        status.append(&chord_label);
        status.append(&flash_label);
        status.append(&paste);
        status.append(&count);
        let show_hints = Rc::new(Cell::new(true));
        let pending_popup = Rc::new(Cell::new(false));
        let minimal = Rc::new(Cell::new(false));

        let more = gtk::MenuButton::new();
        more.set_child(Some(&gtk::Label::new(Some("F1  Shortcuts"))));
        more.add_css_class("shortcut-footer-button");
        more.set_tooltip_text(Some("Show all file-view shortcuts (F1)"));
        status.prepend(&more);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        status.insert_child_after(&spacer, Some(&more));
        status.reorder_child_after(&min_tag, Some(&spacer));
        status.reorder_child_after(&filter_mark, Some(&min_tag));
        status.reorder_child_after(&chord_label, Some(&filter_mark));
        status.reorder_child_after(&flash_label, Some(&chord_label));

        let prompt_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        prompt_box.set_hexpand(true);
        prompt_box.add_css_class("minimal-prompt");
        let prompt_prefix = gtk::Label::new(None);
        prompt_prefix.add_css_class("minimal-prompt-prefix");
        prompt_prefix.set_xalign(0.0);
        let prompt_entry = gtk::Entry::new();
        prompt_entry.add_css_class("minimal-prompt-entry");
        prompt_entry.set_hexpand(true);
        crate::ui::accessibility::set_label(&prompt_entry, "Minimal mode command");
        let prompt_spinner = gtk::Spinner::new();
        prompt_spinner.set_visible(false);
        prompt_box.append(&prompt_prefix);
        prompt_box.append(&prompt_entry);
        prompt_box.append(&prompt_spinner);
        let prompt_hint = gtk::Label::new(None);
        prompt_hint.add_css_class("minimal-flash");
        prompt_hint.set_visible(false);
        prompt_box.append(&prompt_hint);
        let weak_hint = prompt_hint.downgrade();
        prompt_entry.connect_changed(move |_| {
            if let Some(hint) = weak_hint.upgrade() {
                hint.set_label("");
                hint.set_visible(false);
            }
        });

        let history_empty = gtk::Label::new(Some("No folder history yet"));
        history_empty.add_css_class("minimal-history-empty");
        history_empty.set_wrap(true);
        history_empty.set_xalign(0.0);
        let history_list = gtk::ListBox::new();
        history_list.add_css_class("search-results");
        history_list.set_selection_mode(gtk::SelectionMode::Single);
        history_list.set_activate_on_single_click(false);
        history_list.set_can_focus(false);
        let history_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .max_content_height(240)
            .min_content_width(360)
            .child(&history_list)
            .build();
        history_scroller.add_css_class("minimal-history-scroll");
        history_scroller.set_can_focus(false);
        let history_stack = gtk::Stack::new();
        history_stack.add_named(&history_empty, Some("empty"));
        history_stack.add_named(&history_scroller, Some("results"));
        history_stack.set_visible_child_name("empty");
        let history_popover = gtk::Popover::builder()
            .position(gtk::PositionType::Top)
            .autohide(false)
            .has_arrow(true)
            .build();
        history_popover.set_can_focus(false);
        history_popover.add_css_class("minimal-history-popover");
        history_popover.set_child(Some(&history_stack));
        history_popover.set_parent(&prompt_entry);
        super::accessibility::set_label(&history_popover, "Visited folders");
        let history_unparent = history_popover.downgrade();
        prompt_entry.connect_destroy(move |_| {
            if let Some(popover) = history_unparent.upgrade() {
                popover.unparent();
            }
        });
        let history_focus_entry = prompt_entry.downgrade();
        history_list.connect_selected_rows_changed(move |_| {
            if let Some(entry) = history_focus_entry.upgrade() {
                entry.grab_focus_without_selecting();
            }
        });

        stack.add_named(&status, Some("status"));
        stack.add_named(&prompt_box, Some("prompt"));
        stack.set_visible_child_name("status");
        root.append(&stack);
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
        let reference = gtk::Box::new(gtk::Orientation::Vertical, 16);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&reference)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .overlay_scrolling(false)
            .propagate_natural_height(true)
            .max_content_height(440)
            .width_request(420)
            .focusable(true)
            .build();
        scroll.add_css_class("fixed-scrollbar");
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
            min_tag.clone().upcast(),
            filter_mark.clone().upcast(),
            chord_label.clone().upcast(),
            flash_label.clone().upcast(),
        ]));
        for widget in status_widgets.borrow().iter() {
            watch_status_widget(widget, &status_widgets, &root);
        }
        let footer = Self(Rc::new(FooterState {
            root,
            stack,
            paste,
            count,
            min_tag,
            filter_mark,
            chord_label,
            flash_label,
            prompt_box,
            prompt_prefix,
            prompt_entry,
            prompt_spinner,
            prompt_hint,
            history_popover,
            chord_hints,
            chord_hint_list,
            history_list,
            history_scroller,
            history_stack,
            history_empty,
            history_items: Rc::new(RefCell::new(Vec::new())),
            prompt_changed: Rc::new(RefCell::new(None)),
            flash_timeout: Rc::new(RefCell::new(None)),
            show_hints,
            pending_popup,
            more,
            popover,
            reference,
            focus_before,
            mode: Cell::new(mode),
            minimal,
            status_widgets,
        }));
        footer.rebuild_reference();
        let popover_keys = gtk::EventControllerKey::new();
        popover_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let dispatch = footer.downgrade();
        popover_keys.connect_key_pressed(move |_, key, _, modifiers| {
            dispatch
                .upgrade()
                .and_then(|footer| footer.handle_key(key, modifiers))
                .unwrap_or(glib::Propagation::Proceed)
        });
        footer.popover.add_controller(popover_keys);
        let chord_keys = gtk::EventControllerKey::new();
        chord_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let chord_host = footer.chord_label.downgrade();
        chord_keys.connect_key_pressed(move |controller, _, _, _| {
            let Some(root) = chord_host.upgrade().and_then(|host| host.root()) else {
                return glib::Propagation::Proceed;
            };
            if controller.forward(&root) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        footer.chord_hints.add_controller(chord_keys);
        footer
    }

    pub(super) fn downgrade(&self) -> WeakShortcutFooter {
        WeakShortcutFooter(Rc::downgrade(&self.0))
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn set_activity(&self, widget: &impl IsA<gtk::Widget>) {
        let widget = widget.as_ref().clone();
        if let Some(parent) = self.count.parent().and_downcast::<gtk::Box>() {
            parent.insert_child_after(&widget, Some(&self.count));
        }
        self.status_widgets.borrow_mut().push(widget.clone());
        watch_status_widget(&widget, &self.status_widgets, &self.root);
    }

    pub fn bind_preferences(&self, manager: &super::preferences::PreferenceManager) {
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

    pub fn observe_browser(&self, view: &super::browser::BrowserView) {
        update_item_count(&self.count, view);
        self.set_query_mark(&view.hidden_filter_query(), view.force_recursive_search());
        let label = self.count.downgrade();
        let weak_view = view.downgrade();
        view.observe_visible_listing(move || {
            if let Some(label) = label.upgrade()
                && let Some(view) = weak_view.upgrade()
            {
                update_item_count(&label, &view);
            }
        });
        let label = self.count.downgrade();
        let weak_view = view.downgrade();
        view.browser().observe(move |_| {
            if let Some(label) = label.upgrade()
                && let Some(view) = weak_view.upgrade()
            {
                update_item_count(&label, &view);
            }
        });
        let footer = self.downgrade();
        let view_for_mode = view.downgrade();
        view.connect_view_mode_changed(move |_| {
            let footer = footer.clone();
            let view_for_mode = view_for_mode.clone();
            glib::idle_add_local_once(move || {
                if let Some(footer) = footer.upgrade()
                    && let Some(view) = view_for_mode.upgrade()
                {
                    footer
                        .set_query_mark(&view.hidden_filter_query(), view.force_recursive_search());
                    update_item_count(&footer.count, &view);
                }
            });
        });
    }

    pub fn set_mode(&self, mode: BrowserMode) {
        self.mode.set(mode);
        self.rebuild_reference();
    }

    /// Swaps the shortcut reference between the default map and the minimal
    /// map. Bound on the same preference as the minimal chrome.
    pub fn set_minimal(&self, minimal: bool) {
        self.minimal.set(minimal);
        self.min_tag.set_visible(minimal);
        if !minimal {
            self.clear_chord_mark();
            self.set_filter_mark("");
            self.hide_prompt();
        }
        self.rebuild_reference();
    }

    pub fn bind_minimal_mode(&self, manager: &super::preferences::PreferenceManager) {
        let footer = self.downgrade();
        manager.bind_preference(
            &self.root,
            super::preferences::PreferenceManager::minimal_mode,
            move |_, minimal| {
                if let Some(footer) = footer.upgrade() {
                    footer.set_minimal(minimal);
                }
            },
        );
        let hints = self.chord_hints.downgrade();
        manager.bind_preference(
            &self.root,
            super::preferences::PreferenceManager::reduce_motion,
            move |_, reduced| {
                let Some(hints) = hints.upgrade() else {
                    return;
                };
                if reduced {
                    hints.add_css_class("instant");
                } else {
                    hints.remove_css_class("instant");
                }
            },
        );
    }

    pub fn prompt_entry_widget(&self) -> gtk::Entry {
        self.prompt_entry.clone()
    }

    pub fn prompt_text(&self) -> String {
        self.prompt_entry.text().to_string()
    }

    pub fn set_prompt_hint(&self, message: &str) {
        self.prompt_hint.set_label(message);
        self.prompt_hint.set_visible(!message.is_empty());
    }

    /// Whether `focused` is the minimal footer prompt entry.
    pub fn is_prompt_entry(&self, focused: &Option<gtk::Widget>) -> bool {
        focused.as_ref().is_some_and(|widget| {
            let entry = self.prompt_entry.upcast_ref::<gtk::Widget>();
            widget == entry
                || widget.is_ancestor(entry)
                || self
                    .prompt_entry
                    .delegate()
                    .is_some_and(|delegate| delegate == *widget || widget.is_ancestor(&delegate))
        })
    }

    /// Shows the footer prompt with a prefix label and placeholder. The
    /// caller owns submit/cancel via the dispatcher; `on_change` runs
    /// incrementally for find/filter/search kinds.
    pub fn show_prompt(
        &self,
        prefix: &str,
        placeholder: &str,
        initial: &str,
        on_change: Option<Rc<dyn Fn(String)>>,
    ) {
        self.disconnect_prompt_changed();
        self.set_prompt_hint("");
        self.hide_history_candidates();
        self.prompt_prefix.set_label(prefix);
        self.prompt_entry.set_placeholder_text(Some(placeholder));
        crate::ui::accessibility::set_label(&self.prompt_entry, &format!("Minimal mode {prefix}"));
        self.prompt_entry.set_text(initial);
        if let Some(callback) = on_change {
            let id = self.prompt_entry.connect_changed(move |entry| {
                callback(entry.text().to_string());
            });
            self.prompt_changed.replace(Some(id));
        }
        // GtkStack ignores set_visible_child for a hidden page.
        self.prompt_box.set_visible(true);
        self.stack.set_visible_child_name("prompt");
        self.root.set_visible(true);
        self.prompt_entry.grab_focus();
        // A just-revealed entry may not be mapped yet; retry focus once it is.
        let weak = self.prompt_entry.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(entry) = weak.upgrade()
                && entry.is_mapped()
                && !entry.has_focus()
            {
                entry.grab_focus();
            }
        });
    }

    /// Selects the stem of a rename initial value (full name for folders).
    pub fn show_rename_prompt(&self, name: &str, is_directory: bool) {
        let stem_end = if is_directory {
            -1
        } else {
            super::collection_edit::rename_stem_end(name)
        };
        self.show_prompt("rename", "rename focused item", name, None);
        self.prompt_entry.select_region(0, stem_end);
    }

    pub fn hide_prompt(&self) {
        self.set_prompt_hint("");
        self.disconnect_prompt_changed();
        self.hide_history_candidates();
        // Clear credentials such as `smb://user:pass@host` before hiding.
        self.prompt_entry.set_text("");
        self.prompt_spinner.set_visible(false);
        self.prompt_spinner.stop();
        self.stack.set_visible_child_name("status");
        self.prompt_box.set_visible(false);
    }

    /// Compact candidate list for `z` / `Z`. The footer entry stays the input.
    pub fn show_history_candidates(&self, items: Vec<SearchItem>, query: &str) {
        while let Some(child) = self.history_list.first_child() {
            self.history_list.remove(&child);
        }
        let empty = if query.trim().is_empty() {
            "No folder history yet"
        } else {
            "No matching folders"
        };
        self.history_empty.set_label(empty);
        self.history_items.replace(items);
        if self.history_items.borrow().is_empty() {
            self.history_stack.set_visible_child_name("empty");
        } else {
            for item in self.history_items.borrow().iter() {
                self.history_list.append(&history_candidate_row(item));
            }
            self.history_stack.set_visible_child_name("results");
            self.history_list
                .select_row(self.history_list.row_at_index(0).as_ref());
        }
        if !self.history_popover.is_visible() {
            self.history_popover.popup();
        }
        self.prompt_entry.grab_focus_without_selecting();
    }

    pub fn hide_history_candidates(&self) {
        if self.history_popover.is_visible() {
            self.history_popover.popdown();
        }
        while let Some(child) = self.history_list.first_child() {
            self.history_list.remove(&child);
        }
        self.history_items.borrow_mut().clear();
        self.history_empty.set_label("No folder history yet");
        self.history_stack.set_visible_child_name("empty");
    }

    pub fn move_history_selection(&self, delta: i32) {
        let count = i32::try_from(self.history_items.borrow().len()).unwrap_or(0);
        if count == 0 {
            return;
        }
        let current = self
            .history_list
            .selected_row()
            .map(|row| row.index())
            .unwrap_or(0);
        let next = (current + delta).rem_euclid(count);
        let Some(row) = self.history_list.row_at_index(next) else {
            return;
        };
        self.history_list.select_row(Some(&row));
        scroll_history_row_into_view(&self.history_scroller, &self.history_list, &row);
        self.prompt_entry.grab_focus_without_selecting();
    }

    pub fn selected_history_path(&self) -> Option<PathBuf> {
        let index = self.history_list.selected_row()?.index();
        usize::try_from(index).ok().and_then(|index| {
            self.history_items
                .borrow()
                .get(index)
                .map(|item| item.path.clone())
        })
    }

    #[cfg(test)]
    pub fn history_candidates_visible(&self) -> bool {
        self.history_popover.is_visible()
    }

    #[cfg(test)]
    pub fn history_candidate_names(&self) -> Vec<String> {
        self.history_items
            .borrow()
            .iter()
            .map(|item| item.name.clone())
            .collect()
    }

    #[cfg(test)]
    pub fn chord_hints_visible(&self) -> bool {
        self.chord_hints.is_visible()
    }

    #[cfg(test)]
    pub fn chord_hint_labels(&self) -> Vec<String> {
        let mut labels = Vec::new();
        let mut child = self.chord_hint_list.first_child();
        while let Some(widget) = child {
            if widget.has_css_class("minimal-chord-hints-row") {
                let keys = widget
                    .first_child()
                    .map(|keys| chord_hint_key_text(&keys))
                    .unwrap_or_default();
                let action = widget
                    .first_child()
                    .and_then(|first| first.next_sibling())
                    .and_downcast::<gtk::Label>()
                    .map(|label| label.label().to_string())
                    .unwrap_or_default();
                labels.push(format!("{keys} {action}"));
            } else if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                labels.push(label.label().to_string());
            }
            child = widget.next_sibling();
        }
        labels
    }

    #[cfg(test)]
    pub fn chord_hint_keycaps(&self) -> Vec<Vec<String>> {
        let mut rows = Vec::new();
        let mut child = self.chord_hint_list.first_child();
        while let Some(widget) = child {
            if widget.has_css_class("minimal-chord-hints-row")
                && let Some(keys) = widget.first_child()
            {
                let mut keycaps = Vec::new();
                let mut key = keys.first_child();
                while let Some(label) = key {
                    if label.has_css_class("minimal-chord-hints-key")
                        && let Some(text) = label.downcast_ref::<gtk::Label>()
                    {
                        keycaps.push(text.label().to_string());
                    }
                    key = label.next_sibling();
                }
                rows.push(keycaps);
            }
            child = widget.next_sibling();
        }
        rows
    }

    fn disconnect_prompt_changed(&self) {
        if let Some(id) = self.prompt_changed.borrow_mut().take() {
            self.prompt_entry.disconnect(id);
        }
    }

    /// Hides the pending chord mark (`g-`, `c-`, `,-`, `;-`) and its hint list.
    pub fn clear_chord_mark(&self) {
        self.hide_chord_hints();
        self.chord_label.set_visible(false);
    }

    pub fn set_chord_mark(&self, kind: MinimalChord) {
        self.present_chord_mark(
            kind,
            kind.hints().iter().map(|(keys, action)| {
                (
                    keys.iter().map(|key| (*key).to_owned()).collect(),
                    (*action).to_owned(),
                )
            }),
        );
    }

    pub fn set_chord_mark_named(&self, kind: MinimalChord, rows: &[(char, String)]) {
        self.present_chord_mark(
            kind,
            rows.iter()
                .map(|(key, name)| (vec![key.to_string()], name.clone())),
        );
    }

    fn present_chord_mark(
        &self,
        kind: MinimalChord,
        rows: impl IntoIterator<Item = (Vec<String>, String)>,
    ) {
        self.chord_label.set_label(kind.mark());
        self.chord_label.set_visible(true);
        self.root.set_visible(true);
        self.show_chord_hint_rows(kind.hint_title(), rows, kind.hint_note());
    }

    fn show_chord_hint_rows(
        &self,
        title: &str,
        rows: impl IntoIterator<Item = (Vec<String>, String)>,
        note: Option<&str>,
    ) {
        while let Some(child) = self.chord_hint_list.first_child() {
            self.chord_hint_list.remove(&child);
        }
        for (keys, action) in rows {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("minimal-chord-hints-row");
            let key_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            key_box.set_valign(gtk::Align::Center);
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    let sep = gtk::Label::new(Some("/"));
                    sep.add_css_class("minimal-chord-hints-sep");
                    key_box.append(&sep);
                }
                let key_label = gtk::Label::new(Some(key));
                key_label.add_css_class("minimal-chord-hints-key");
                key_label.set_xalign(0.0);
                key_box.append(&key_label);
            }
            let action_label = gtk::Label::new(Some(&action));
            action_label.add_css_class("minimal-chord-hints-action");
            action_label.set_xalign(0.0);
            action_label.set_hexpand(true);
            row.append(&key_box);
            row.append(&action_label);
            self.chord_hint_list.append(&row);
        }
        if let Some(note) = note {
            let note_label = gtk::Label::new(Some(note));
            note_label.add_css_class("minimal-chord-hints-note");
            note_label.set_xalign(0.0);
            self.chord_hint_list.append(&note_label);
        }
        super::accessibility::set_label(&self.chord_hints, title);
        self.popup_chord_hints();
    }

    fn popup_chord_hints(&self) {
        if super::motion::animations_enabled() {
            self.chord_hints.remove_css_class("instant");
        } else {
            self.chord_hints.add_css_class("instant");
        }
        self.chord_hints.popup();
    }

    fn hide_chord_hints(&self) {
        if self.chord_hints.is_visible() {
            self.chord_hints.popdown();
        }
    }

    /// Shows `filter: <query>` while a hidden pane filter is active.
    pub fn set_filter_mark(&self, query: &str) {
        self.set_query_mark(query, false);
    }

    /// Shows `search: <query>` or `filter: <query>` for the hidden pane query.
    /// Recursive `s` keeps the search prefix until that search is dismissed.
    pub fn set_query_mark(&self, query: &str, search: bool) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            self.filter_mark.set_visible(false);
            self.filter_mark.set_label("");
        } else {
            let prefix = if search { "search" } else { "filter" };
            self.filter_mark.set_label(&format!("{prefix}: {trimmed}"));
            self.filter_mark.set_visible(true);
            self.root.set_visible(true);
        }
    }

    /// Flashes a transient footer hint (unknown chord, missing pin).
    /// Clears automatically; never logs secrets.
    pub fn flash(&self, message: &str) {
        if let Some(id) = self.flash_timeout.borrow_mut().take() {
            id.remove();
        }
        self.flash_label.set_label(message);
        self.flash_label.set_visible(true);
        self.root.set_visible(true);
        let weak = self.flash_label.downgrade();
        let slot = self.flash_timeout.clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
            slot.borrow_mut().take();
            if let Some(label) = weak.upgrade() {
                label.set_visible(false);
                label.set_label("");
            }
        });
        self.flash_timeout.replace(Some(id));
        tracing::debug!(message, "minimal footer hint");
    }

    /// Toggles the shortcut reference popover.
    pub fn toggle_reference(&self) -> glib::Propagation {
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
        glib::Propagation::Stop
    }

    fn rebuild_reference(&self) {
        while let Some(child) = self.reference.first_child() {
            self.reference.remove(&child);
        }
        let minimal = self.minimal.get();
        if !minimal {
            let mode = self.mode.get();
            append_section(
                &self.reference,
                match mode {
                    BrowserMode::Columns => "Columns navigation",
                    BrowserMode::Icons => "Icons navigation",
                    BrowserMode::List => "List navigation",
                },
                &navigation_shortcuts(mode),
            );
        }
        for &category in super::shortcut_reference::categories(minimal) {
            if !minimal && category == "Navigation" {
                continue;
            }
            let rows: Vec<_> = super::shortcut_reference::shortcuts(minimal)
                .filter(|(group, _, _, _)| *group == category)
                .map(|(_, action, note, keys)| {
                    let description = if note.is_empty() {
                        (*action).to_owned()
                    } else {
                        format!("{action} ({note})")
                    };
                    (keys.replace(" + ", "+"), description)
                })
                .collect();
            let shortcuts: Vec<_> = rows
                .iter()
                .map(|(keys, action)| (keys.as_str(), action.as_str()))
                .collect();
            append_section(&self.reference, category, &shortcuts);
        }
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
        if key == gdk::Key::F1
            && !command_modifiers
            && !modifiers.contains(gdk::ModifierType::SHIFT_MASK)
        {
            return Some(self.toggle_reference());
        }
        // Open-state only; closed-state `~` stays in MinimalDispatcher.
        if self.minimal.get()
            && key == gdk::Key::asciitilde
            && !command_modifiers
            && (self.popover.is_visible() || self.pending_popup.get())
        {
            return Some(self.toggle_reference());
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
}

fn update_item_count(label: &gtk::Label, view: &super::browser::BrowserView) {
    let browser = view.browser();
    if let Some(results) = view.search_result_listing() {
        apply_unselected_count(
            label,
            results.len(),
            results.iter().filter(|entry| !entry.is_directory()).count(),
            results.iter().filter(|entry| entry.is_directory()).count(),
        );
        return;
    }
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
    if !selected.is_empty() {
        label.set_label(&selection_details(&selected));
        let noun = if counts.total == 1 { "item" } else { "items" };
        label.set_tooltip_text(Some(&format!(
            "{} of {} {noun} selected. Size includes selected files only; folder contents are not counted.",
            selected.len(), counts.total
        )));
        label.set_visible(true);
        return;
    }
    apply_unselected_count(label, counts.total, counts.files, counts.folders);
}

fn apply_unselected_count(label: &gtk::Label, total: usize, files: usize, folders: usize) {
    let noun = if total == 1 { "item" } else { "items" };
    label.set_label(&format!("{total} {noun}"));
    let file_noun = if files == 1 { "file" } else { "files" };
    let folder_noun = if folders == 1 { "folder" } else { "folders" };
    label.set_tooltip_text(Some(&format!(
        "{files} {file_noun}, {folders} {folder_noun}"
    )));
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

fn navigation_shortcuts(mode: BrowserMode) -> Vec<Shortcut> {
    let mut shortcuts = match mode {
        BrowserMode::Columns => vec![
            ("↑ / ↓", "Move between items"),
            ("← / →", "Parent pane / enter folder"),
            ("Space", "Open folder column"),
            ("← at first pane", "Focus the visible sidebar"),
            (
                "Backspace",
                "Close the current pane or go to the parent folder",
            ),
            (
                "h / j / k / l",
                "Move between items; l opens the item (type-to-search off)",
            ),
        ],
        BrowserMode::Icons => vec![
            ("↑ ↓ ← →", "Move spatially between tiles"),
            ("← at left edge", "Focus the visible sidebar"),
            ("Backspace", "Go to the parent folder"),
            ("h / l", "Parent folder / open item (type-to-search off)"),
            ("j / k", "Next / previous item (type-to-search off)"),
        ],
        BrowserMode::List => vec![
            ("↑ / ↓", "Move between file rows"),
            ("←", "Focus the visible sidebar"),
            ("Backspace", "Go to the parent folder"),
            ("h / l", "Parent folder / open item (type-to-search off)"),
            ("j / k", "Next / previous item (type-to-search off)"),
        ],
    };
    shortcuts.extend_from_slice(&[
        ("↑ at top", "Focus the navigation header"),
        ("← / → in header", "Move between header controls"),
        ("↓ in header", "Return to the files"),
        ("→ in sidebar", "Return to the browser"),
        ("↑ at sidebar top", "Focus the top navigation bar"),
        ("← / → in top bar", "Move between top-bar controls"),
        (
            "↓ in top bar",
            "Return to the sidebar, or files when hidden",
        ),
        ("Alt+← / Alt+→", "Back / forward in history"),
        ("Alt+↑", "Go to the parent folder"),
        ("Alt+Home", "Go to Home"),
        ("Home / End", "First / last item"),
        ("Ctrl+↑ / Ctrl+↓", "First / last item"),
        ("PgUp / PgDn", "Move one page"),
        ("Tab / Shift+Tab", "Next / previous interface control"),
    ]);
    shortcuts
}

fn history_candidate_row(item: &SearchItem) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("search-result");
    row.set_can_focus(false);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&crate::assets::primary_icon(
        crate::assets::icons::FOLDER,
        16,
    ));
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let name = gtk::Label::new(Some(&item.name));
    name.add_css_class("search-result-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let full_path = item.path.to_string_lossy();
    row.set_tooltip_text(Some(&full_path));
    let path = gtk::Label::new(Some(&full_path));
    path.add_css_class("search-result-path");
    path.set_xalign(0.0);
    path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    labels.append(&name);
    labels.append(&path);
    content.append(&labels);
    row.set_child(Some(&content));
    row
}

fn scroll_history_row_into_view(
    scroller: &gtk::ScrolledWindow,
    list: &gtk::ListBox,
    row: &gtk::ListBoxRow,
) {
    let Some(bounds) = row.compute_bounds(list) else {
        return;
    };
    let adjustment = scroller.vadjustment();
    let viewport_top = adjustment.value();
    let viewport_bottom = viewport_top + adjustment.page_size();
    let row_top = f64::from(bounds.y());
    let row_bottom = row_top + f64::from(bounds.height());
    if row_top < viewport_top {
        adjustment.set_value(row_top);
    } else if row_bottom > viewport_bottom {
        adjustment.set_value(row_bottom - adjustment.page_size());
    }
}

fn append_section(parent: &gtk::Box, title: &str, shortcuts: &[(&str, &str)]) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 7);
    let heading = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build();
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
fn chord_hint_key_text(keys: &gtk::Widget) -> String {
    let mut parts = Vec::new();
    let mut child = keys.first_child();
    while let Some(widget) = child {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            parts.push(label.label().to_string());
        }
        child = widget.next_sibling();
    }
    parts.join("")
}

#[cfg(test)]
mod tests;
