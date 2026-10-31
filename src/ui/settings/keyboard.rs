// SPDX-License-Identifier: MIT

//! Settings → Keyboard: each map's table of commands and their keys. The page
//! only edits keymap overrides and type-to-search; dispatch and labels follow
//! the preferences. See `docs/keyboard-shortcuts.md`.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

use gtk::{gdk::Key, glib, prelude::*};

use super::{page_content, scrollable_page, search};
use crate::{
    assets::{self, icons},
    services::{ActionCatalog, ActionRegistry},
    ui::{
        accessibility,
        controls::{
            ModalTone, keep_words_whole, message_dialog_description, message_dialog_layout,
            segmented_control,
        },
        keys::{
            self, Binding, Command, CommandKind, Group, Keys, Map,
            bindings::*,
            conflicts::{self, FieldLimit, Finding, Fixed, Kind, Outcome, Policy, Severity},
        },
        modal::{ModalHost, dismiss_modal_layer, modal_layer},
        preferences::PreferenceManager,
        shortcut_reference,
    },
};

#[cfg(test)]
mod tests;

pub(super) const PAGE: &str = "keyboard";

pub(super) fn keyboard_page(preferences: Rc<PreferenceManager>) -> gtk::Widget {
    let content = page_content();
    content.add_css_class("settings-keyboard-page");
    // The map comes first as the scope the search then narrows; the first
    // section heading follows at the page's usual section gap.
    let controls = gtk::Box::new(gtk::Orientation::Vertical, 12);
    controls.set_margin_bottom(24);
    search::tag(&controls, "Keyboard shortcuts");
    let (filter_field, filter, clear_filter) = super::search_field("Search keybinds");

    let shown = if preferences.tenxer_mode() {
        Map::Tenxer
    } else {
        Map::Default
    };
    let toolbar = super::wrap::WrapRow::new(8);
    toolbar.set_hexpand(true);
    toolbar.set_end_align(true);
    let (maps, map_buttons) = segmented_control(
        &["Default", "10xer mode"],
        usize::from(shown == Map::Tenxer),
    );
    maps.set_hexpand(false);
    maps.set_valign(gtk::Align::Center);
    let reset_all = gtk::Button::with_label(&crate::i18n::tr("Reset all"));
    reset_all.add_css_class("settings-action-button");
    reset_all.add_css_class("settings-actions-create-button");
    reset_all.set_valign(gtk::Align::Center);
    accessibility::set_description(
        &reset_all,
        Some(&crate::i18n::tr(
            "Restore the shipped keys of every action in the shown mode.",
        )),
    );
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    toolbar.append(&maps);
    toolbar.append(&spacer);
    toolbar.append(&reset_all);
    controls.append(&toolbar);
    controls.append(&filter_field);
    let no_match = gtk::Label::new(Some(&crate::i18n::tr("No matching shortcuts")));
    no_match.set_xalign(0.0);
    no_match.add_css_class("settings-option-description");
    no_match.set_visible(false);
    controls.append(&no_match);
    content.append(&controls);

    let page = Rc::new_cyclic(|weak: &Weak<Page>| {
        let mut rows = Vec::new();
        let mut sections = Vec::new();
        let tables = [Map::Default, Map::Tenxer].map(|map| {
            let table = table(map, weak, &mut rows, &mut sections);
            table.set_visible(map == shown);
            content.append(&table);
            (map, table)
        });
        Page {
            preferences: preferences.clone(),
            actions: crate::ui::actions::shared(),
            rows,
            tables,
            sections,
            no_match,
            query: RefCell::new(String::new()),
            reset_all: reset_all.clone(),
            shown: Cell::new(shown),
            capture: RefCell::new(None),
        }
    });

    let weak = Rc::downgrade(&page);
    filter.connect_changed(move |filter| {
        clear_filter.set_visible(!filter.text().is_empty());
        if let Some(page) = weak.upgrade() {
            page.query.replace(filter.text().to_lowercase());
            page.filter();
        }
    });

    for (button, map) in map_buttons.iter().zip([Map::Default, Map::Tenxer]) {
        let page = Rc::downgrade(&page);
        button.connect_toggled(move |button| {
            if button.is_active()
                && let Some(page) = page.upgrade()
            {
                page.show(map);
            }
        });
    }
    let pending = Rc::new(Cell::new(false));
    for (_, table) in &page.tables {
        for group in groups(table) {
            let (page, pending, buttons) = (
                Rc::downgrade(&page),
                pending.clone(),
                map_buttons
                    .iter()
                    .map(ObjectExt::downgrade)
                    .collect::<Vec<_>>(),
            );
            // Search hides groups one table at a time, so look once both are done.
            group.connect_visible_notify(move |_| {
                if pending.replace(true) {
                    return;
                }
                let (page, pending, buttons) = (page.clone(), pending.clone(), buttons.clone());
                glib::idle_add_local_once(move || {
                    pending.set(false);
                    if let Some(page) = page.upgrade() {
                        page.follow_search(&buttons);
                    }
                });
            });
        }
    }
    let weak = Rc::downgrade(&page);
    reset_all.connect_clicked(move |button| {
        if let Some(page) = weak.upgrade() {
            page.end_capture(false);
            page.confirm_reset_all(button);
        }
    });

    // Keys reach an armed capture through `keys::capture` before any other
    // handler; this reader stands in for a window dispatcher where none runs.
    let reader = gtk::EventControllerKey::new();
    reader.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys::on_press(&reader, |_, _| glib::Propagation::Proceed);
    content.add_controller(reader);

    let weak = Rc::downgrade(&page);
    let observer = RefCell::new(Some(page.actions.observe(Rc::new(move || {
        if let Some(page) = weak.upgrade() {
            page.name_slot_actions();
            if !page.query.borrow().trim().is_empty() {
                page.filter();
            }
        }
    }))));
    content.connect_destroy(move |_| {
        observer.borrow_mut().take();
    });
    preferences.bind_preference(
        &content,
        |preferences| (preferences.keymap_revision(), preferences.type_to_search()),
        move |_, _| page.render(),
    );
    scrollable_page(&content, Some("settings-keyboard-scroll"))
}

/// A searchable row: its action name and the keys both maps give it.
pub(super) struct SearchTarget {
    pub(super) id: String,
    /// The name in English and the interface language, and a slot's action.
    pub(super) names: String,
    pub(super) aliases: String,
}

pub(super) fn search_targets() -> Vec<SearchTarget> {
    let locale = rust_i18n::locale();
    let catalog = crate::ui::actions::loaded_catalog();
    keys::registry()
        .map(|command| {
            let mut aliases = Vec::new();
            for map in [Map::Default, Map::Tenxer] {
                for binding in command.bindings_in(map) {
                    for locale in [&*locale, "en"] {
                        aliases.push(keys::label_in(locale, &Keys::One(binding.shown())));
                    }
                }
            }
            SearchTarget {
                id: target_id(command),
                names: search_names(command, &catalog),
                aliases: aliases.join(" "),
            }
        })
        .collect()
}

pub(super) fn is_target(id: &str) -> bool {
    id.strip_prefix(TARGET_PREFIX)
        .is_some_and(|id| Command::parse(id).is_some())
}

const TARGET_PREFIX: &str = "keymap.";

fn target_id(command: Command) -> String {
    format!("{TARGET_PREFIX}{}", command.id())
}

struct Page {
    preferences: Rc<PreferenceManager>,
    actions: Rc<ActionRegistry>,
    rows: Vec<Rc<Row>>,
    tables: [(Map, gtk::Box); 2],
    sections: Vec<Section>,
    no_match: gtk::Label,
    query: RefCell<String>,
    reset_all: gtk::Button,
    shown: Cell<Map>,
    capture: RefCell<Option<Capturing>>,
}

struct Capturing {
    row: Rc<Row>,
    _keys: keys::Capture,
    focus: Option<(glib::WeakRef<gtk::Window>, glib::SignalHandlerId)>,
}

impl Drop for Capturing {
    fn drop(&mut self) {
        if let Some((window, handler)) = self.focus.take()
            && let Some(window) = window.upgrade()
        {
            window.disconnect(handler);
        }
    }
}

impl Page {
    fn policy(&self) -> Policy {
        Policy {
            type_to_search: self.preferences.type_to_search(),
        }
    }

    fn render(&self) {
        let keymap = self.preferences.keymap();
        let policy = self.policy();
        let mut findings: HashMap<(Map, Command), Vec<Finding>> = HashMap::new();
        for map in [Map::Default, Map::Tenxer] {
            for (command, finding) in conflicts::audit(&keymap, map, policy) {
                findings.entry((map, command)).or_default().push(finding);
            }
        }
        for row in &self.rows {
            let found = findings
                .get(&(row.map, row.command))
                .map_or(&[][..], Vec::as_slice);
            row.render(self, &keymap, found);
        }
        self.update_reset_all();
        self.name_slot_actions();
    }

    fn name_slot_actions(&self) {
        let catalog = self.actions.catalog();
        for row in &self.rows {
            if let Some(action) = slot_action(row.command, &catalog) {
                row.action.set_text(&action);
            }
        }
    }

    fn confirm_reset_all(&self, anchor: &gtk::Button) {
        let Some(host) = ModalHost::blurred_for(anchor) else {
            return;
        };
        let map = self.shown.get();
        let layout = message_dialog_layout(
            icons::UNDO_2,
            &crate::i18n::tr("Reset all keybinds?"),
            &crate::i18n::tr(match map {
                Map::Default => "Default",
                Map::Tenxer => "10xer mode",
            }),
            &crate::i18n::tr("Reset"),
            ModalTone::Danger,
        );
        layout
            .body
            .append(&message_dialog_description(&crate::i18n::tr(
                "Restore the shipped keys of every action in the shown mode.",
            )));
        let layer = modal_layer(
            &layout.content,
            &host.overlay,
            host.blurred_root.clone(),
            None,
        );
        let dismiss = {
            let layer = layer.downgrade();
            let overlay = host.overlay.clone();
            let root = host.blurred_root.clone();
            Rc::new(move || {
                if let Some(layer) = layer.upgrade() {
                    dismiss_modal_layer(&layer, &overlay, root.as_ref());
                }
            })
        };
        for button in [&layout.close, &layout.cancel] {
            let dismiss = dismiss.clone();
            button.connect_clicked(move |_| dismiss());
        }
        let preferences = Rc::downgrade(&self.preferences);
        layout.confirm.connect_clicked(move |_| {
            if let Some(preferences) = preferences.upgrade() {
                preferences.reset_map(map);
            }
            dismiss();
        });
        host.overlay.add_overlay(&layer);
        layout.confirm.grab_focus();
    }

    fn update_reset_all(&self) {
        let keymap = self.preferences.keymap();
        let map = self.shown.get();
        self.reset_all.set_sensitive(
            self.rows
                .iter()
                .any(|row| row.map == map && keymap.overrides(map, row.command)),
        );
    }

    fn show(&self, map: Map) {
        self.end_capture(false);
        self.shown.set(map);
        for (table_map, table) in &self.tables {
            table.set_visible(*table_map == map);
        }
        self.update_reset_all();
        self.filter();
    }

    fn filter(&self) {
        let query = self.query.borrow();
        let query = query.trim();
        let catalog = self.actions.catalog();
        let mut shown_any = false;
        for section in &self.sections {
            let mut any = false;
            for (slot, command) in &section.rows {
                let matches = search_names(*command, &catalog).contains(query);
                slot.set_visible(matches);
                any |= matches;
            }
            section.widget.set_visible(any);
            shown_any |= any && section.map == self.shown.get();
        }
        self.no_match.set_visible(!shown_any);
    }

    /// Shows the other table when a search leaves no row in the shown one
    /// but matches some there.
    fn follow_search(&self, buttons: &[glib::WeakRef<gtk::ToggleButton>]) {
        let has_rows = |map: Map| {
            self.tables
                .iter()
                .filter(|(table_map, _)| *table_map == map)
                .any(|(_, table)| groups(table).any(|group| group.get_visible()))
        };
        let shown = self.shown.get();
        let other = match shown {
            Map::Default => Map::Tenxer,
            Map::Tenxer => Map::Default,
        };
        if !has_rows(shown)
            && has_rows(other)
            && let Some(button) = buttons[usize::from(other == Map::Tenxer)].upgrade()
        {
            button.set_active(true);
        }
    }

    fn start_capture(self: &Rc<Self>, row: &Rc<Row>) {
        self.end_capture(false);
        set_button_text(&row.capture, &crate::i18n::tr("Press a key…"));
        accessibility::set_label(
            &row.capture,
            &rust_i18n::t!(
                "Press the new key for “%{action}”",
                action = title(row.command)
            ),
        );
        accessibility::set_description(&row.capture, Some(&capture_help()));
        row.capture.set_visible(true);
        row.cancel.set_visible(true);
        row.capture.grab_focus();
        row.add.set_visible(false);
        clear(&row.pending);
        let page = Rc::downgrade(self);
        let captured = row.clone();
        let armed = keys::capture(&row.capture, move |press| {
            if let Some(page) = page.upgrade() {
                page.captured(&captured, press);
            }
        });
        let focus = row
            .capture
            .root()
            .and_downcast::<gtk::Window>()
            .map(|window| {
                let page = Rc::downgrade(self);
                let (capture, cancel) = (row.capture.downgrade(), row.cancel.downgrade());
                let handler =
                    window.connect_notify_local(Some("focus-widget"), move |window, _| {
                        let focus = gtk::prelude::GtkWindowExt::focus(window);
                        let inside = [&capture, &cancel]
                            .iter()
                            .filter_map(|widget| widget.upgrade())
                            .any(|widget| focus.as_ref() == Some(widget.upcast_ref()));
                        if !inside && let Some(page) = page.upgrade() {
                            page.end_capture(false);
                        }
                    });
                (window.downgrade(), handler)
            });
        self.capture.replace(Some(Capturing {
            row: row.clone(),
            _keys: armed,
            focus,
        }));
    }

    fn end_capture(&self, refocus: bool) {
        let Some(capturing) = self.capture.take() else {
            return;
        };
        let row = capturing.row.clone();
        drop(capturing);
        row.add.set_visible(true);
        if refocus {
            row.add.grab_focus();
        }
        row.capture.set_visible(false);
        row.cancel.set_visible(false);
        clear(&row.pending);
    }

    fn captured(&self, row: &Rc<Row>, press: &keys::KeyPress) {
        let Some(binding) = press.binding() else {
            return;
        };
        if binding.key() == Key::Escape {
            self.end_capture(true);
            return;
        }
        if binding == BACKSPACE {
            self.end_capture(true);
            self.preferences
                .set_bindings(row.map, row.command, Vec::new());
            return;
        }
        let keymap = self.preferences.keymap();
        let mut entry = keymap.bindings(row.map, row.command).to_vec();
        // The press also matches its digit-row position where the layout
        // types a symbol there, so a digit that is taken refuses it too.
        let mut matched = vec![binding];
        matched.extend(
            press
                .digit_position()
                .filter(|position| *position != binding),
        );
        if matched.iter().any(|binding| entry.contains(binding)) {
            self.end_capture(true);
            return;
        }
        let refused: Vec<_> = matched
            .iter()
            .flat_map(|binding| {
                conflicts::check(&keymap, row.map, row.command, *binding, self.policy())
            })
            .filter(|finding| finding.severity == Severity::Error)
            .map(|finding| message(row.map, row.command, finding))
            .collect();
        if refused.is_empty() {
            self.end_capture(true);
            entry.push(binding);
            self.preferences.set_bindings(row.map, row.command, entry);
            return;
        }
        let shown = key_label(binding);
        set_button_text(&row.capture, &shown);
        clear(&row.pending);
        for refusal in &refused {
            row.pending.append(&finding_label(refusal, Severity::Error));
        }
        accessibility::set_description(
            &row.capture,
            Some(&format!("{} {}", refused.join(" "), capture_help())),
        );
    }
}

fn capture_help() -> String {
    rust_i18n::t!(
        "%{escape} cancels, and %{backspace} removes every key of this action.",
        escape = key_label(Binding::plain(Key::Escape)),
        backspace = key_label(BACKSPACE)
    )
    .into_owned()
}

struct Row {
    map: Map,
    command: Command,
    /// The action a custom action slot runs.
    action: gtk::Label,
    prefix: gtk::Label,
    chips: super::wrap::WrapRow,
    unbound: gtk::Label,
    findings: gtk::Box,
    pending: gtk::Box,
    capture: gtk::Button,
    cancel: gtk::Button,
    add: gtk::Button,
    reset: gtk::Button,
    page: Weak<Page>,
}

/// The least width a row's copy keeps before the key chips move below it.
const COPY_WIDTH_CHARS: i32 = 20;

impl Row {
    fn new(map: Map, command: Command, page: &Weak<Page>) -> (super::wrap::WrapRow, Rc<Self>) {
        // Not a box: its minimum height assumes the split between copy and
        // chips that wraps least, while it allocates the natural split, so a
        // scrolled table cut rows short. The wrap row measures the split it
        // allocates and drops the chips below the copy when both cannot fit.
        let widget = super::wrap::WrapRow::new(16);
        widget.add_css_class("settings-option");
        widget.add_css_class("keymap-row");
        search::tag_id(&widget, &target_id(command));
        let action = title(command);

        let copy = gtk::Box::new(gtk::Orientation::Vertical, 4);
        copy.set_hexpand(true);
        let title = gtk::Label::new(Some(&action));
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_width_chars(COPY_WIDTH_CHARS);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        title.add_css_class("settings-option-title");
        copy.append(&title);
        let runs = gtk::Label::new(None);
        runs.set_xalign(0.0);
        runs.set_wrap(true);
        runs.set_width_chars(COPY_WIDTH_CHARS);
        runs.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        runs.add_css_class("settings-option-description");
        runs.set_visible(keys::custom_action_slot(command).is_some());
        copy.append(&runs);
        let findings = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let pending = gtk::Box::new(gtk::Orientation::Vertical, 2);
        copy.append(&findings);
        copy.append(&pending);
        widget.append(&copy);

        let controls = super::wrap::WrapRow::new(6);
        controls.set_end_align(true);
        controls.set_valign(gtk::Align::Center);
        let prefix = gtk::Label::new(None);
        prefix.add_css_class("keymap-prefix");
        prefix.add_css_class("settings-nowrap");
        prefix.set_valign(gtk::Align::Center);
        let chips = super::wrap::WrapRow::new(6);
        chips.set_end_align(true);
        chips.set_valign(gtk::Align::Center);
        let unbound = gtk::Label::new(Some(&crate::i18n::tr("Not bound")));
        unbound.add_css_class("settings-option-description");
        unbound.add_css_class("settings-nowrap");
        unbound.set_valign(gtk::Align::Center);
        let button = |label: &str| {
            let button = gtk::Button::new();
            set_button_text(&button, &crate::i18n::tr(label));
            button.add_css_class("settings-action-button");
            button.set_valign(gtk::Align::Center);
            button
        };
        let capture = gtk::Button::new();
        capture.add_css_class("settings-action-button");
        capture.add_css_class("keymap-capture");
        capture.set_valign(gtk::Align::Center);
        capture.set_visible(false);
        let cancel = button("Cancel");
        cancel.set_visible(false);
        let add = button("Add");
        accessibility::set_label(
            &add,
            &rust_i18n::t!("Add a key for “%{action}”", action = action.as_str()),
        );
        let reset = button("Reset");
        accessibility::set_label(
            &reset,
            &rust_i18n::t!(
                "Restore the default keys of “%{action}”",
                action = action.as_str()
            ),
        );
        for child in [
            prefix.upcast_ref::<gtk::Widget>(),
            chips.upcast_ref(),
            unbound.upcast_ref(),
            capture.upcast_ref(),
            cancel.upcast_ref(),
            add.upcast_ref(),
            reset.upcast_ref(),
        ] {
            controls.append(child);
        }
        widget.append(&controls);

        let row = Rc::new(Self {
            map,
            command,
            action: runs,
            prefix,
            chips,
            unbound,
            findings,
            pending,
            capture,
            cancel,
            add,
            reset,
            page: page.clone(),
        });
        let weak = Rc::downgrade(&row);
        row.add.connect_clicked(move |_| {
            if let Some(row) = weak.upgrade()
                && let Some(page) = row.page.upgrade()
            {
                page.start_capture(&row);
            }
        });
        let page_for_cancel = page.clone();
        row.cancel.connect_clicked(move |_| {
            if let Some(page) = page_for_cancel.upgrade() {
                page.end_capture(true);
            }
        });
        let page_for_unmap = page.clone();
        row.capture.connect_unmap(move |_| {
            if let Some(page) = page_for_unmap.upgrade() {
                page.end_capture(false);
            }
        });
        let page_for_reset = page.clone();
        let add = row.add.downgrade();
        row.reset.connect_clicked(move |_| {
            if let Some(page) = page_for_reset.upgrade() {
                page.end_capture(false);
                page.preferences.reset_binding(map, command);
            }
            if let Some(add) = add.upgrade() {
                add.grab_focus();
            }
        });
        (widget, row)
    }

    fn render(&self, page: &Page, keymap: &keys::Keymap, findings: &[Finding]) {
        let entry = keymap.bindings(self.map, self.command);
        let action = title(self.command);
        let trigger = match (self.map, self.command.kind()) {
            (Map::Tenxer, CommandKind::Step(group)) => trigger(group),
            _ => None,
        };
        let prefix =
            trigger.and_then(|trigger| keymap.bindings(Map::Tenxer, trigger).first().copied());
        self.prefix
            .set_text(&prefix.map(key_label).unwrap_or_default());
        self.prefix.set_visible(prefix.is_some());

        while let Some(chip) = self.chips.first_child() {
            chip.unparent();
        }
        clear(&self.findings);
        let mut notes: Vec<(String, Severity)> = Vec::new();
        let unreachable = trigger.filter(|_| prefix.is_none());
        if let Some(trigger) = unreachable.filter(|_| !entry.is_empty()) {
            notes.push((
                rust_i18n::t!(
                    "“%{group}” has no key, so this action cannot run.",
                    group = title(trigger)
                )
                .into_owned(),
                Severity::Warning,
            ));
        }
        let mut turn_off_typing = false;
        for binding in entry {
            let mut found: Vec<Finding> = findings
                .iter()
                .filter(|finding| finding.binding == binding.normalized())
                .copied()
                .collect();
            // A key that never fires has nothing else worth saying.
            if found
                .iter()
                .any(|finding| finding.kind == Kind::NeverMatches)
            {
                found.retain(|finding| finding.kind == Kind::NeverMatches);
            }
            let inactive = unreachable.is_some() || found.iter().any(|finding| finding.inactive());
            let messages: Vec<String> = found
                .iter()
                .map(|finding| message(self.map, self.command, *finding))
                .collect();
            for (finding, text) in found.iter().zip(&messages) {
                turn_off_typing |= finding.kind == Kind::TypeToSearchShadowed;
                push_note(&mut notes, text.clone(), finding.severity);
            }
            if self.multiline_limit(*binding) {
                push_note(
                    &mut notes,
                    rust_i18n::t!(
                        "In text boxes with several lines, %{keys} does not start a new line or insert a tab.",
                        keys = key_label(*binding)
                    )
                    .into_owned(),
                    Severity::Warning,
                );
            }
            self.chips
                .append(&self.chip(*binding, &action, inactive, &messages));
        }
        if self.command == ESCAPE && !entry.contains(&Binding::plain(Key::Escape)) {
            push_note(
                &mut notes,
                rust_i18n::t!(
                    "%{keys} always works here as well.",
                    keys = key_label(Binding::plain(Key::Escape))
                )
                .into_owned(),
                Severity::Warning,
            );
        }
        for (text, severity) in &notes {
            self.findings.append(&finding_label(text, *severity));
        }
        if turn_off_typing && page.preferences.type_to_search() {
            let turn_off = gtk::Button::with_label(&crate::i18n::tr("Turn off type-to-search"));
            turn_off.add_css_class("settings-action-button");
            turn_off.set_halign(gtk::Align::Start);
            turn_off.set_margin_top(4);
            let preferences = Rc::downgrade(&page.preferences);
            turn_off.connect_clicked(move |_| {
                if let Some(preferences) = preferences.upgrade() {
                    preferences.set_type_to_search(false);
                }
            });
            self.findings.append(&turn_off);
        }
        self.findings
            .set_visible(self.findings.first_child().is_some());
        self.unbound.set_visible(entry.is_empty());
        self.reset
            .set_visible(keymap.overrides(self.map, self.command));
    }

    /// A key other than Enter or Tab that a multi-line text box reads as
    /// them.
    fn multiline_limit(&self, binding: Binding) -> bool {
        [ENTER, FOCUS_NEXT, FOCUS_PREVIOUS].contains(&self.command)
            && self.command.canonical() != Some(binding)
    }

    fn chip(&self, binding: Binding, action: &str, inactive: bool, notes: &[String]) -> gtk::Box {
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        chip.add_css_class("keymap-chip");
        if inactive {
            chip.add_css_class("inactive");
        }
        let text = key_label(binding);
        let key = gtk::Label::new(Some(&text));
        key.add_css_class("keymap-chip-key");
        key.add_css_class("settings-nowrap");
        key.set_valign(gtk::Align::Center);
        chip.append(&key);
        let remove = gtk::Button::builder()
            .child(&assets::primary_icon(icons::X, 12))
            .tooltip_text(crate::i18n::tr("Remove"))
            .valign(gtk::Align::Center)
            .build();
        remove.add_css_class("keymap-chip-remove");
        remove.set_has_frame(false);
        accessibility::set_label(
            &remove,
            &rust_i18n::t!(
                "Remove %{keys} from “%{action}”",
                keys = text.as_str(),
                action = action
            ),
        );
        if !notes.is_empty() {
            accessibility::set_description(&remove, Some(&notes.join(" ")));
        }
        let (map, command, page, add) = (
            self.map,
            self.command,
            self.page.clone(),
            self.add.downgrade(),
        );
        remove.connect_clicked(move |_| {
            let Some(page) = page.upgrade() else {
                return;
            };
            page.end_capture(false);
            let entry = page
                .preferences
                .keymap()
                .bindings(map, command)
                .iter()
                .copied()
                .filter(|kept| *kept != binding)
                .collect();
            page.preferences.set_bindings(map, command, entry);
            if let Some(add) = add.upgrade() {
                add.grab_focus();
            }
        });
        chip.append(&remove);
        chip
    }
}

/// A section of one table: its heading and group, and each row's slot with
/// the command whose names the page filter matches.
struct Section {
    map: Map,
    widget: gtk::Box,
    rows: Vec<(gtk::Box, Command)>,
}

fn table(
    map: Map,
    page: &Weak<Page>,
    rows: &mut Vec<Rc<Row>>,
    filtered: &mut Vec<Section>,
) -> gtk::Box {
    let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
    table.add_css_class("settings-preferences");
    table.add_css_class("keymap-table");
    table.set_widget_name(match map {
        Map::Default => "keymap-default",
        Map::Tenxer => "keymap-tenxer",
    });
    for (title, commands) in sections(map) {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        // Settings search hides the heading when nothing under it matches.
        widget.add_css_class("settings-search-section");
        let group = super::settings_group(&widget, title);
        let mut slots = Vec::new();
        for command in commands {
            let (row_widget, row) = Row::new(map, command, page);
            // The page filter hides the slot, leaving the row's own
            // visibility to Settings search.
            let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
            slot.append(&row_widget);
            group.append(&slot);
            slots.push((slot, command));
            rows.push(row);
        }
        table.append(&widget);
        filtered.push(Section {
            map,
            widget,
            rows: slots,
        });
    }
    table
}

/// Each command sits in the first section naming it in `map`'s reference,
/// then in the other map's. Commands `map` cannot run stay out, and the default
/// map files the 10xer section's runnable commands under "Search and tools".
fn sections(map: Map) -> Vec<(&'static str, Vec<Command>)> {
    let tenxer = shortcut_reference::command_sections(Map::Tenxer);
    let default = shortcut_reference::command_sections(Map::Default);
    let mut order: Vec<&'static str> = Vec::new();
    for (title, _) in tenxer.iter().chain(&default) {
        if !order.contains(title) {
            order.push(title);
        }
    }
    let (own, others) = match map {
        Map::Default => (&default, &tenxer),
        Map::Tenxer => (&tenxer, &default),
    };
    let commands: Vec<Command> = keys::registry()
        .filter(|command| {
            (map == Map::Tenxer || !matches!(command.kind(), CommandKind::Trigger(_)))
                && !conflicts::no_path(*command, map)
        })
        .collect();
    let section_of = |command: Command| {
        let title = [own, others]
            .into_iter()
            .flatten()
            .find(|(_, listed)| listed.contains(&command))
            .map(|(title, _)| *title)
            .or_else(|| {
                FALLBACK_SECTIONS
                    .iter()
                    .find(|(listed, _)| *listed == command)
                    .map(|(_, title)| *title)
            })
            .unwrap_or(SEARCH_AND_TOOLS);
        if map == Map::Default && title == TENXER_SECTION {
            SEARCH_AND_TOOLS
        } else {
            title
        }
    };
    order
        .into_iter()
        .map(|title| {
            let mut listed: Vec<Command> = Vec::new();
            let candidates = [own, others]
                .into_iter()
                .flatten()
                .filter(|(section, _)| *section == title)
                .flat_map(|(_, listed)| listed.iter().copied())
                .chain(commands.iter().copied());
            for command in candidates {
                if commands.contains(&command)
                    && section_of(command) == title
                    && !listed.contains(&command)
                {
                    listed.push(command);
                }
            }
            (title, listed)
        })
        .filter(|(_, listed)| !listed.is_empty())
        .collect()
}

const SEARCH_AND_TOOLS: &str = "Search and tools";
const TENXER_SECTION: &str = "10xer mode";

/// Commands no reference row names.
const FALLBACK_SECTIONS: &[(Command, &str)] = &[
    (PDF_ZOOM_RESET, "Preview"),
    (PREVIOUS_MATCH, "Preview"),
    (REFERENCE_CATEGORIES, SEARCH_AND_TOOLS),
    (REFERENCE_SEARCH, SEARCH_AND_TOOLS),
    (REFERENCE_LIST, SEARCH_AND_TOOLS),
    (FILTER_SLASH, SEARCH_AND_TOOLS),
];

fn trigger(group: Group) -> Option<Command> {
    keys::registry().find(|command| command.kind() == CommandKind::Trigger(group))
}

fn key_label(binding: Binding) -> String {
    keys::label(&Keys::One(binding.shown()))
}

/// `set_label` also labels a button by its text, which would win over the
/// accessible name the page gives it.
fn set_button_text(button: &gtk::Button, text: &str) {
    button.set_label(text);
    accessibility::clear_labelled_by(button);
}

fn groups(table: &gtk::Box) -> impl Iterator<Item = gtk::Widget> {
    children(table.upcast_ref())
        .flat_map(|section| children(&section))
        .filter(|child| child.has_css_class("settings-group"))
}

fn children(widget: &gtk::Widget) -> impl Iterator<Item = gtk::Widget> + use<> {
    std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn push_note(notes: &mut Vec<(String, Severity)>, text: String, severity: Severity) {
    if !notes.iter().any(|(listed, _)| *listed == text) {
        notes.push((text, severity));
    }
}

fn finding_label(text: &str, severity: Severity) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_width_chars(COPY_WIDTH_CHARS);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    keep_words_whole(&label);
    label.add_css_class("settings-option-description");
    label.add_css_class("keymap-finding");
    label.add_css_class(match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    });
    label
}

/// What a finding means for the user, naming the side that loses the key
/// where the checker knows it.
fn message(map: Map, command: Command, finding: Finding) -> String {
    let keys = key_label(finding.binding);
    let keys = keys.as_str();
    let named = title;
    let plain = |message: &str| rust_i18n::t!(message, keys = keys).into_owned();
    let shift = keys::modifiers_label(gtk::gdk::ModifierType::SHIFT_MASK);
    match finding.kind {
        Kind::Duplicate(other) => {
            let message = match conflicts::duplicate_outcome(map, command, finding.binding, other) {
                Outcome::Wins => "Takes %{keys} from “%{other}”.",
                Outcome::Loses => "“%{other}” takes %{keys} first.",
                Outcome::Either => "“%{other}” also uses %{keys}.",
            };
            rust_i18n::t!(message, keys = keys, other = named(other)).into_owned()
        }
        Kind::ShadowsChordGroup(group) => rust_i18n::t!(
            "%{keys} also starts “%{group}”, so either this key or that chord never runs.",
            keys = keys,
            group = trigger(group).map(named).unwrap_or_default()
        )
        .into_owned(),
        Kind::ShiftedSortStep => rust_i18n::t!(
            "%{shift} with a sort key reverses the sort, so %{keys} cannot be one.",
            keys = keys,
            shift = shift
        )
        .into_owned(),
        Kind::SortStepCannotReverse => rust_i18n::t!(
            "%{keys} sorts, but %{shift} cannot reverse it.",
            keys = keys,
            shift = shift
        )
        .into_owned(),
        Kind::TypeToSearchShadowed => plain("%{keys} does nothing while type-to-search is on."),
        Kind::TextFieldLimit(FieldLimit::Typing) => {
            plain("Doesn't work while a text field or menu has focus.")
        }
        Kind::TextFieldLimit(FieldLimit::FieldKey) => {
            plain("Text fields use %{keys} for editing, so there it usually edits text instead.")
        }
        Kind::ContextOverlap(other) => rust_i18n::t!(
            "“%{other}” also uses %{keys}; where both apply, only one of them acts.",
            keys = keys,
            other = named(other)
        )
        .into_owned(),
        Kind::ReservedByGtk => plain(
            "File lists use %{keys} to change the selection, which stops working where this key acts.",
        ),
        Kind::InputMethodKey => {
            plain("Input methods need %{keys} while you type, so it would interrupt typing.")
        }
        Kind::ChooserMnemonic => {
            plain("The file chooser may use %{keys} for its accept button instead.")
        }
        Kind::FixedKey(Fixed::SelectionToggle) => {
            plain("%{keys} selects the item under the cursor, so it never runs this action.")
        }
        Kind::FixedKey(Fixed::MediaKeys) => plain(
            "Media previews use %{keys} for playback first, so it never runs this action there.",
        ),
        Kind::FixedKey(Fixed::Backspace) => plain(
            "%{keys} goes to the parent folder and deletes text, which stops working where this key acts.",
        ),
        Kind::FixedKey(Fixed::Digits(digits)) => {
            let message = if digits == TAB_NUMBERS {
                "%{digits} select tabs before anything else reads them, so %{keys} never runs this action."
            } else if digits == PIN_NUMBERS {
                "%{digits} open pinned folders, and this key takes %{keys} from them."
            } else if digits == ACTIONS_NUMBERS {
                "%{digits} run custom actions, and this key takes %{keys} from them."
            } else {
                "%{digits} select tabs, and this key takes %{keys} from them."
            };
            let locale = rust_i18n::locale();
            let mut range = keys::label_in_map(&locale, &Keys::Digits(digits), map);
            if range.is_empty() {
                // The chord's trigger has no key in this table.
                range = keys::label_in_map(&locale, &Keys::Digits(digits.step()), map);
            }
            rust_i18n::t!(message, keys = keys, digits = range).into_owned()
        }
        Kind::NeverMatches => plain("%{keys} never reaches this action, so it does nothing."),
        Kind::NoPath => crate::i18n::tr(match map {
            Map::Default => "Only 10xer mode runs this action, so its keys do nothing here.",
            Map::Tenxer => "10xer mode does not run this action, so its keys do nothing here.",
        }),
    }
}

fn title(command: Command) -> String {
    title_in(command, &rust_i18n::locale())
}

fn title_in(command: Command, locale: &str) -> String {
    match keys::custom_action_slot(command) {
        Some(number) => rust_i18n::t!(name(command), locale = locale, number = number),
        None => rust_i18n::t!(name(command), locale = locale),
    }
    .into_owned()
}

/// The action a custom action slot runs, as Settings → Actions names it.
fn slot_action(command: Command, catalog: &ActionCatalog) -> Option<String> {
    let number = keys::custom_action_slot(command)?;
    Some(catalog.actions().get(number - 1).map_or_else(
        || crate::i18n::tr("No action"),
        |action| action.name().to_owned(),
    ))
}

fn search_names(command: Command, catalog: &ActionCatalog) -> String {
    let mut names = vec![title(command), title_in(command, "en")];
    names.extend(slot_action(command, catalog));
    names.join("\n").to_lowercase()
}

/// The message key of a row's name. Commands whose reference row describes
/// several keys at once have their own.
fn name(command: Command) -> &'static str {
    NAMES
        .iter()
        .find(|(named, _)| *named == command)
        .map_or(command.id(), |(_, name)| name)
}

const NAMES: &[(Command, &str)] = &[
    (UP, "Move up"),
    (DOWN, "Move down"),
    (LEFT, "Move left"),
    (RIGHT, "Move right"),
    (FIRST_ITEM, "First item"),
    (LAST_ITEM, "Last item"),
    (FIRST_ITEM_UP, "Jump to the first item"),
    (LAST_ITEM_DOWN, "Jump to the last item"),
    (PAGE_UP, "Page up"),
    (PAGE_DOWN, "Page down"),
    (HALF_PAGE_UP, "Half a page up"),
    (HALF_PAGE_DOWN, "Half a page down"),
    (ENTER, "Open or activate the focused item"),
    (ESCAPE, "Close preview or cancel the current interaction"),
    (FOCUS_NEXT, "Next interface control"),
    (FOCUS_PREVIOUS, "Previous interface control"),
    (HISTORY_BACK, "Back"),
    (HISTORY_FORWARD, "Forward"),
    (PARENT_FOLDER, "Go to the parent folder"),
    (HOME_FOLDER, "Go to Home"),
    (OPEN_IN_NEW_TAB, "Open in a new tab"),
    (OPEN_IN_NEW_WINDOW, "Open in a new window"),
    (PREVIEW, "Preview a file, or open a folder"),
    (COMPLETE, "Write the chosen folder into the prompt"),
    (OPEN, "Open the focused item"),
    (CLOSE_WINDOW, "Close the current window"),
    (COPY, "Copy"),
    (CUT, "Cut"),
    (PASTE, "Paste"),
    (SELECT_ALL, "Select all"),
    (UNDO, "Undo"),
    (REDO, "Redo"),
    (NEW_FOLDER, "Create a folder"),
    (
        NEW_FOLDER_WITH_SELECTION,
        "Create a folder containing the selection",
    ),
    (RENAME, "Rename"),
    (PROPERTIES, "Show item properties"),
    (CONTEXT_MENU, "Open the context menu"),
    (MOVE_TO_TRASH, "Move to Trash"),
    (DELETE_PERMANENTLY, "Permanently delete selected items"),
    (TENXER_DELETE_CONFIRM, "Confirm Move to Trash"),
    (DUPLICATE, "Duplicate selected items"),
    (COPY_PATH, "Copy path"),
    (PIN, "Pin a folder"),
    (YANK, "Yank to copy"),
    (YANK_CUT, "Yank to cut"),
    (UNYANK, "Clear copy and cut marks"),
    (PASTE_KEEP_BOTH, "Paste; Keep Both is focused on conflicts"),
    (PASTE_REPLACE, "Paste; Replace is focused on conflicts"),
    (CREATE, "Create a file; end with / for a folder"),
    (OPEN_WITH, "Open With…"),
    (MOVE_TO, "Move to…"),
    (COPY_TO, "Copy to…"),
    (RESTORE, "Restore from Trash"),
    (VISUAL_SELECT, "Visual select"),
    (VISUAL_UNSET, "Visual unset"),
    (INVERT_SELECTION, "Invert the selection"),
    (SEARCH, "Open global search"),
    (LOCATION, "Edit the location"),
    (REFRESH, "Refresh"),
    (HIDDEN_FILES, "Show or hide hidden files"),
    (FILTER, "Filter the current pane"),
    (FILTER_SLASH, "Filter while type-to-search is on"),
    (VIEW_COLUMNS, "Switch to Columns"),
    (VIEW_ICONS, "Switch to Icons"),
    (VIEW_LIST, "Switch to List"),
    (TEXT_LARGER, "Increase text size"),
    (TEXT_SMALLER, "Decrease text size"),
    (TEXT_RESET, "Reset text size"),
    (THUMBNAILS_LARGER, "Increase thumbnail size"),
    (THUMBNAILS_SMALLER, "Decrease thumbnail size"),
    (THUMBNAILS_RESET, "Reset thumbnail size"),
    (SIDEBAR, "Show or hide the sidebar"),
    (SIDEBAR_FOCUS, "Switch focus between sidebar and browser"),
    (SETTINGS, "Open Settings"),
    (REFERENCE, "Show or hide the shortcut reference"),
    (TENXER_MODE, "Toggle 10xer mode"),
    (TAB_NEW, "New tab"),
    (TAB_CLOSE, "Close tab"),
    (TAB_REOPEN, "Reopen the last closed tab"),
    (TAB_NEXT, "Next tab"),
    (TAB_PREVIOUS, "Previous tab"),
    (TAB_MOVE_LEFT, "Move tab left"),
    (TAB_MOVE_RIGHT, "Move tab right"),
    (JUMP_TO_FOLDER, "Jump to a folder"),
    (JUMP_VISITED, "Jump to a visited folder"),
    (JUMP_RECENT, "Jump to a recent folder"),
    (TERMINAL, "Open a terminal"),
    (ARROW_SCOPE, "Keep arrows in the file list on or off"),
    (
        OPEN_CONTAINING_FOLDER,
        "Open containing folder (global search)",
    ),
    (FIND, "Find a name in this listing"),
    (FIND_BACKWARD, "Find a name backward"),
    (REPEAT_FIND, "Repeat the last find"),
    (REPEAT_FIND_REVERSED, "Repeat the last find in reverse"),
    (SEARCH_SUBFOLDERS, "Search this folder and its subfolders"),
    (CHOOSE_FOLDER, "Choose a folder, as Accept does"),
    (REFERENCE_CATEGORIES, "Focus the reference categories"),
    (REFERENCE_SEARCH, "Search the shortcut reference"),
    (REFERENCE_LIST, "Focus the reference list"),
    (GO, "Go chord"),
    (GO_FIRST, "Go to the first item"),
    (GO_FOLLOW, "Follow search result"),
    (GO_PROMPT, "Go to a folder, typed path, or URI"),
    (GO_HOME, "Home"),
    (GO_CONFIG, "Configuration folder"),
    (GO_DOWNLOADS, "Downloads"),
    (GO_DOCUMENTS, "Documents"),
    (GO_MUSIC, "Music"),
    (GO_PICTURES, "Pictures"),
    (GO_VIDEOS, "Videos"),
    (GO_TRASH, "Trash"),
    (GO_NETWORK, "Network"),
    (GO_RECENT, "Recent"),
    (GO_PIN, "Pin a folder (go chord)"),
    (GO_UNPIN, "Unpin a folder"),
    (SORT_BY, "Sort chord"),
    (SORT_BY_NAME, "Sort by name"),
    (SORT_BY_MODIFIED, "Sort by modified date"),
    (SORT_BY_SIZE, "Sort by size"),
    (SORT_BY_TYPE, "Sort by type"),
    (COPY_AS, "Copy chord"),
    (COPY_AS_PATH, "Copy path (copy chord)"),
    (COPY_AS_NAME, "Copy name"),
    (ACTIONS, "Action chord"),
    (ACTIONS_TERMINAL, "Open a terminal in the focused folder"),
    (
        ACTIONS_COMPRESS,
        "Compress the selection, or the focused item",
    ),
    (ACTIONS_EXTRACT, "Extract here"),
    (ACTIONS_EXTRACT_TO, "Extract to a typed folder"),
    (TAB_CHORD, "Tab chord"),
    (TAB_CHORD_NEW, "New tab (tab chord)"),
    (TAB_CHORD_CLOSE, "Close tab (tab chord)"),
    (TAB_CHORD_PREVIOUS, "Previous tab (tab chord)"),
    (MEDIA_PLAY_PAUSE, "Play / pause"),
    (MEDIA_SEEK_BACK, "Seek back 5 seconds"),
    (MEDIA_SEEK_FORWARD, "Seek forward 5 seconds"),
    (MEDIA_VOLUME_UP, "Volume up"),
    (MEDIA_VOLUME_DOWN, "Volume down"),
    (MEDIA_MUTE, "Mute / unmute"),
    (MEDIA_PREVIOUS, "Previous file of the same type"),
    (MEDIA_NEXT, "Next file of the same type"),
    (TENXER_MEDIA_MUTE, "Mute or unmute the open preview"),
    (
        TENXER_MEDIA_PREVIOUS,
        "Previous media file in the open preview",
    ),
    (TENXER_MEDIA_NEXT, "Next media file in the open preview"),
    (MEDIA_SEEK_BACK_PAGE, "Seek back 30 seconds"),
    (MEDIA_SEEK_FORWARD_PAGE, "Seek forward 30 seconds"),
    (MEDIA_SEEK_START, "Seek to the start"),
    (MEDIA_SEEK_END, "Seek to the end"),
    (MEDIA_VOLUME_STEP_UP, "Volume up in the preview"),
    (MEDIA_VOLUME_STEP_DOWN, "Volume down in the preview"),
    (SCROLL_PREVIEW_DOWN, "Scroll the preview down"),
    (SCROLL_PREVIEW_UP, "Scroll the preview up"),
    (FIND_IN_PREVIEW, "Find in preview"),
    (PDF_ZOOM_RESET, "Reset zoom"),
    (PREVIOUS_MATCH, "Previous match"),
    (CUSTOM_ACTION_1, shortcut_reference::CUSTOM_ACTION),
    (CUSTOM_ACTION_2, shortcut_reference::CUSTOM_ACTION),
    (CUSTOM_ACTION_3, shortcut_reference::CUSTOM_ACTION),
    (CUSTOM_ACTION_4, shortcut_reference::CUSTOM_ACTION),
    (CUSTOM_ACTION_5, shortcut_reference::CUSTOM_ACTION),
    (CUSTOM_ACTION_6, shortcut_reference::CUSTOM_ACTION),
    (CUSTOM_ACTION_7, shortcut_reference::CUSTOM_ACTION),
    (CUSTOM_ACTION_8, shortcut_reference::CUSTOM_ACTION),
    (CUSTOM_ACTION_9, shortcut_reference::CUSTOM_ACTION),
    (CUSTOM_ACTION_10, shortcut_reference::CUSTOM_ACTION),
];
