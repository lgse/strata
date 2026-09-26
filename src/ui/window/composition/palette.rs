// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, gio, glib, prelude::*};

use crate::ui::{
    blur::BlurBin,
    browser::{BrowserView, PinStatus, palette::FileCommand},
    preferences::PreferenceManager,
    shortcut_footer::ShortcutFooter,
};

use super::WindowContent;
use catalogue::{COMMANDS, Command};

mod catalogue;
#[cfg(test)]
mod tests;

thread_local! {
    static RECENT_COMMANDS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

struct Palette {
    window: glib::WeakRef<gtk::ApplicationWindow>,
    layer: gtk::Box,
    field: gtk::Entry,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    empty: gtk::Label,
    browser: BrowserView,
    preferences: Rc<PreferenceManager>,
    sidebar: gtk::ToggleButton,
    shortcuts: ShortcutFooter,
    blurred_root: BlurBin,
    focus_before: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    rows: RefCell<Vec<usize>>,
    states: RefCell<Vec<CommandState>>,
}

struct CommandState {
    label: &'static str,
    reason: Option<&'static str>,
    current: bool,
}

pub(super) fn install(
    window: &gtk::ApplicationWindow,
    content: &WindowContent,
    preferences: &Rc<PreferenceManager>,
) {
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("search-backdrop");
    layer.add_css_class("command-palette-backdrop");
    layer.add_css_class("app-modal-layer");
    layer.set_focusable(true);
    layer.set_visible(false);
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    panel.add_css_class("search-dialog");
    panel.set_halign(gtk::Align::Center);
    panel.set_valign(gtk::Align::Center);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    bar.add_css_class("search-bar");
    bar.append(&crate::assets::primary_icon(
        crate::assets::icons::KEYBOARD,
        20,
    ));
    let field = gtk::Entry::builder()
        .placeholder_text("Search commands…")
        .hexpand(true)
        .build();
    field.add_css_class("search-field");
    crate::ui::accessibility::set_label(&field, "Search commands");
    bar.append(&field);
    panel.append(&bar);
    let list = gtk::ListBox::new();
    list.add_css_class("search-results");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(true);
    let empty = gtk::Label::new(Some("No matching commands"));
    empty.add_css_class("search-status");
    panel.append(&empty);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(360)
        .max_content_height(440)
        .child(&list)
        .build();
    panel.append(&scroller);
    let footer = gtk::Label::new(Some("↑↓  navigate     Enter  run     Escape  close"));
    footer.add_css_class("search-footer");
    footer.add_css_class("search-hint");
    panel.append(&footer);
    crate::ui::modal::layout::install(&layer, &panel);
    content.overlay.add_overlay(&layer);
    let palette = Rc::new(Palette {
        window: window.downgrade(),
        layer,
        field,
        list,
        scroller,
        empty,
        browser: content.browser.clone(),
        preferences: preferences.clone(),
        sidebar: content.header.sidebar_toggle.clone(),
        shortcuts: content.footer.shortcuts.clone(),
        blurred_root: content.blurred_root.clone(),
        focus_before: RefCell::new(None),
        rows: RefCell::new(Vec::new()),
        states: RefCell::new(Vec::new()),
    });
    let weak = Rc::downgrade(&palette);
    palette.layer.connect_has_focus_notify(move |layer| {
        if layer.has_focus()
            && let Some(palette) = weak.upgrade()
        {
            palette.field.grab_focus_without_selecting();
        }
    });
    let weak = Rc::downgrade(&palette);
    palette.field.connect_changed(move |_| {
        if let Some(palette) = weak.upgrade() {
            palette.render();
        }
    });
    let weak = Rc::downgrade(&palette);
    palette.list.connect_row_activated(move |_, row| {
        if let Some(palette) = weak.upgrade() {
            palette.activate(row.index());
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&palette);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        weak.upgrade()
            .filter(|palette| palette.owns_keyboard())
            .map_or(glib::Propagation::Proceed, |palette| {
                palette.key(key, modifiers)
            })
    });
    window.add_controller(keys);
    let action = gio::SimpleAction::new("command-palette", None);
    action.connect_activate(move |_, _| palette.show());
    window.add_action(&action);
    content
        .header
        .commands
        .set_action_name(Some("win.command-palette"));
}

impl Palette {
    fn owns_keyboard(&self) -> bool {
        self.window
            .upgrade()
            .and_then(|window| super::super::visible_modal_layer(&window))
            .is_some_and(|layer| layer == self.layer)
    }

    fn show(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if self.layer.is_visible() {
            if self.owns_keyboard() {
                self.hide();
            }
            return;
        }
        if super::super::visible_modal_layer(&window).is_some()
            || self.browser.rename_is_active()
            || self.browser.new_entry_is_active()
        {
            return;
        }
        self.field.set_text("");
        self.present(&window);
        let scroll = self.scroller.vadjustment();
        scroll.set_value(scroll.lower());
    }

    fn present(&self, window: &gtk::ApplicationWindow) {
        self.focus_before
            .replace(gtk::prelude::RootExt::focus(window).map(|w| w.downgrade()));
        self.browser.prepare_palette();
        self.states.replace(
            COMMANDS
                .iter()
                .map(|spec| self.state(spec.command, spec.label))
                .collect(),
        );
        self.render();
        self.blurred_root.set_blurred(true);
        self.layer.set_visible(true);
        self.field.grab_focus_without_selecting();
    }

    fn hide(&self) {
        self.layer.set_visible(false);
        self.blurred_root.set_blurred(false);
        if !self
            .focus_before
            .take()
            .and_then(|w| w.upgrade())
            .is_some_and(|w| w.is_mapped() && w.grab_focus())
        {
            self.browser.browser().focus_active();
        }
    }

    fn state(&self, command: Command, default_label: &'static str) -> CommandState {
        let mut state = CommandState {
            label: default_label,
            reason: None,
            current: false,
        };
        match command {
            Command::Hidden if self.preferences.sort_preferences().show_hidden => {
                state.label = "Hide hidden files";
            }
            Command::Sidebar if self.sidebar.is_active() => state.label = "Hide sidebar",
            Command::View(mode) => state.current = self.browser.view_mode() == mode,
            Command::File(file) => {
                state.reason = self.browser.palette_file_unavailable(file);
                if file == FileCommand::Pin
                    && self.browser.palette_pin_status() == PinStatus::Pinned
                {
                    state.label = "Unpin folder";
                }
            }
            Command::Terminal => {
                if self.browser.palette_terminal_location().is_none() {
                    state.reason = Some("Open a local folder first");
                }
            }
            Command::Filter | Command::Refresh | Command::Location
                if self.browser.browser().active_location().is_none() =>
            {
                state.reason = Some("Open a folder first");
            }
            _ => {}
        }
        state
    }

    fn render(&self) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let mut rows = catalogue::matches(&self.field.text());
        let recent = if self.field.text().is_empty() {
            RECENT_COMMANDS.with(|recent| recent.borrow().clone())
        } else {
            Vec::new()
        };
        rows.retain(|index| !recent.contains(index));
        let rows: Vec<_> = recent.iter().copied().chain(rows).collect();
        self.empty.set_visible(rows.is_empty());
        let groups: Vec<_> = rows
            .iter()
            .map(|index| {
                if recent.contains(index) {
                    "Recents"
                } else {
                    COMMANDS[*index].group
                }
            })
            .collect();
        let grouped = self.field.text().is_empty();
        self.list.set_header_func(move |row, previous| {
            let group = groups[row.index() as usize];
            if grouped && previous.is_none_or(|previous| groups[previous.index() as usize] != group)
            {
                let heading = gtk::Label::new(Some(group));
                heading.set_xalign(0.0);
                heading.add_css_class("command-group");
                row.set_header(Some(&heading));
            } else {
                row.set_header(None::<&gtk::Widget>);
            }
        });
        let states = self.states.borrow();
        for &index in &rows {
            let Some(state) = states.get(index) else {
                continue;
            };
            let spec = &COMMANDS[index];
            let row = gtk::ListBoxRow::new();
            row.add_css_class("search-result");
            crate::ui::accessibility::set_label(&row, state.label);
            row.update_property(&[gtk::accessible::Property::Description(
                state.reason.unwrap_or(if recent.contains(&index) {
                    "Recents"
                } else {
                    spec.group
                }),
            )]);
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
            text.set_hexpand(true);
            let label = gtk::Label::new(Some(state.label));
            label.set_xalign(0.0);
            label.add_css_class("search-result-name");
            text.append(&label);
            if let Some(reason) = state.reason {
                let reason = gtk::Label::new(Some(reason));
                reason.set_xalign(0.0);
                reason.add_css_class("search-result-path");
                text.append(&reason);
                row.add_css_class("command-unavailable");
            }
            content.append(&text);
            let hint = gtk::Label::new(Some(if state.current {
                "Current"
            } else {
                spec.shortcut
            }));
            hint.add_css_class("search-hint");
            content.append(&hint);
            row.set_child(Some(&content));
            self.list.append(&row);
        }
        self.rows.replace(rows);
        self.list.select_row(self.list.row_at_index(0).as_ref());
    }

    fn key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        match key {
            gdk::Key::Escape => self.hide(),
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if let Some(row) = self.list.selected_row() {
                    self.activate(row.index());
                }
            }
            gdk::Key::Up | gdk::Key::Down
                if !modifiers
                    .intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) =>
            {
                let count = self.rows.borrow().len() as i32;
                if count > 0 {
                    let current = self.list.selected_row().map_or(0, |row| row.index());
                    let next =
                        (current + if key == gdk::Key::Up { -1 } else { 1 }).rem_euclid(count);
                    if let Some(row) = self.list.row_at_index(next) {
                        self.list.select_row(Some(&row));
                        row.grab_focus();
                        self.field.grab_focus_without_selecting();
                    }
                }
            }
            _ if key == gdk::Key::F5
                || (modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                    && matches!(
                        key,
                        gdk::Key::k
                            | gdk::Key::K
                            | gdk::Key::t
                            | gdk::Key::T
                            | gdk::Key::backslash
                            | gdk::Key::comma
                            | gdk::Key::p
                            | gdk::Key::P
                    )) => {}
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    fn activate(&self, row: i32) {
        let Some(index) = self.rows.borrow().get(row as usize).copied() else {
            return;
        };
        let command = COMMANDS[index].command;
        self.hide();
        if self.state(command, COMMANDS[index].label).reason.is_some() {
            if let Some(window) = self.window.upgrade() {
                self.present(&window);
                let position = self
                    .rows
                    .borrow()
                    .iter()
                    .position(|candidate| *candidate == index);
                if let Some(row) =
                    position.and_then(|position| self.list.row_at_index(position as i32))
                {
                    self.list.select_row(Some(&row));
                    row.grab_focus();
                    self.field.grab_focus_without_selecting();
                }
            }
            return;
        }
        match command {
            Command::Search => self.action("search"),
            Command::RecentFolders => self.action("jump-folder"),
            Command::Settings => self.action("settings"),
            Command::Terminal => self.browser.execute_palette_terminal(),
            Command::Refresh => self.action("refresh"),
            Command::Filter => {
                self.browser.show_filter();
            }
            Command::Location => self.browser.begin_location_edit(),
            Command::Shortcuts => {
                self.shortcuts
                    .handle_key(gdk::Key::F1, gdk::ModifierType::empty());
            }
            Command::View(mode) => {
                super::super::apply_browser_mode(&self.browser, &self.preferences, mode);
            }
            Command::Hidden => self.browser.browser().toggle_hidden(),
            Command::Sidebar => self.sidebar.set_active(!self.sidebar.is_active()),
            Command::File(file) => self.browser.execute_palette_file(file),
        }
        RECENT_COMMANDS.with(|recent| record_recent(&mut recent.borrow_mut(), index));
    }

    fn action(&self, name: &str) {
        if let Some(window) = self.window.upgrade() {
            gio::prelude::ActionGroupExt::activate_action(&window, name, None);
        }
    }
}

fn record_recent(recent: &mut Vec<usize>, index: usize) {
    recent.retain(|previous| *previous != index);
    recent.insert(0, index);
    recent.truncate(5);
}
