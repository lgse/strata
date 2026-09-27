// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc, time::Duration};

use gtk::{gdk, gio, glib, prelude::*};

use crate::ui::{
    blur::BlurBin,
    browser::{BrowserView, palette::PaletteTarget},
    preferences::PreferenceManager,
};

use super::WindowContent;
use catalogue::COMMANDS;
use commands::{CommandState, Commands};

mod catalogue;
mod commands;
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
    commands: Commands,
    target: RefCell<Option<PaletteTarget>>,
    blurred_root: BlurBin,
    focus_before: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    rows: RefCell<Vec<CommandRow>>,
    refresh_source: RefCell<Option<glib::SourceId>>,
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
        commands: Commands::new(window, content, preferences),
        target: RefCell::new(None),
        blurred_root: content.blurred_root.clone(),
        focus_before: RefCell::new(None),
        rows: RefCell::new(Vec::new()),
        refresh_source: RefCell::new(None),
    });
    let weak = Rc::downgrade(&palette);
    preferences.bind_preference(
        &palette.layer,
        |manager| (manager.tenxer_mode(), manager.type_to_search_active()),
        move |_, _| {
            if let Some(palette) = weak.upgrade() {
                palette.refresh();
            }
        },
    );
    let weak = Rc::downgrade(&palette);
    palette.layer.connect_map(move |_| {
        let Some(palette) = weak.upgrade() else {
            return;
        };
        let weak = Rc::downgrade(&palette);
        // Undo history spans windows and has no change signal.
        let source = glib::timeout_add_local(Duration::from_millis(200), move || {
            let Some(palette) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            palette.refresh();
            glib::ControlFlow::Continue
        });
        palette.refresh_source.replace(Some(source));
    });
    let weak = Rc::downgrade(&palette);
    palette.layer.connect_unmap(move |_| {
        if let Some(palette) = weak.upgrade()
            && let Some(source) = palette.refresh_source.take()
        {
            source.remove();
        }
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
        self.focus_before
            .replace(gtk::prelude::RootExt::focus(&window).map(|w| w.downgrade()));
        self.target
            .replace(Some(self.browser.capture_palette_target()));
        self.field.set_text("");
        self.render();
        self.blurred_root.set_blurred(true);
        self.layer.set_visible(true);
        self.field.grab_focus_without_selecting();
        let scroll = self.scroller.vadjustment();
        scroll.set_value(scroll.lower());
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

    fn render(&self) {
        let target = self.target.borrow();
        let Some(target) = target.as_ref() else {
            return;
        };
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
        let mut commands = Vec::with_capacity(rows.len());
        for &index in &rows {
            let spec = &COMMANDS[index];
            let state = self.commands.state(spec, target);
            let group = if recent.contains(&index) {
                "Recents"
            } else {
                spec.group
            };
            let row = CommandRow::new(index, group, state);
            self.list.append(&row.widget);
            commands.push(row);
        }
        self.rows.replace(commands);
        self.list.select_row(self.list.row_at_index(0).as_ref());
    }

    fn refresh(&self) {
        if !self.layer.is_visible() {
            return;
        }
        let target = self.target.borrow();
        let Some(target) = target.as_ref() else {
            return;
        };
        for row in self.rows.borrow_mut().iter_mut() {
            let state = self.commands.state(&COMMANDS[row.command], target);
            if state != row.state {
                row.state = state;
                row.update();
            }
        }
    }

    fn key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        if matches!(key, gdk::Key::p | gdk::Key::P)
            && modifiers.contains(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK)
            && !modifiers.intersects(gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK)
        {
            self.hide();
            return glib::Propagation::Stop;
        }
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
        let Some(index) = self.rows.borrow().get(row as usize).map(|row| row.command) else {
            return;
        };
        let target = self.target.borrow();
        let Some(target) = target.as_ref() else {
            return;
        };
        if self
            .commands
            .execute(&COMMANDS[index], target, || self.hide())
            .is_err()
        {
            self.render();
            let position = self
                .rows
                .borrow()
                .iter()
                .position(|candidate| candidate.command == index);
            if let Some(row) = position.and_then(|position| self.list.row_at_index(position as i32))
            {
                self.list.select_row(Some(&row));
                row.grab_focus();
                self.field.grab_focus_without_selecting();
            }
            return;
        }
        RECENT_COMMANDS.with(|recent| record_recent(&mut recent.borrow_mut(), index));
    }
}

struct CommandRow {
    command: usize,
    group: &'static str,
    state: CommandState,
    widget: gtk::ListBoxRow,
}

impl CommandRow {
    fn new(command: usize, group: &'static str, state: CommandState) -> Self {
        let row = Self {
            command,
            group,
            state,
            widget: gtk::ListBoxRow::new(),
        };
        row.widget.add_css_class("search-result");
        row.update();
        row
    }

    fn update(&self) {
        let state = &self.state;
        crate::ui::accessibility::set_label(&self.widget, state.label);
        self.widget
            .update_property(&[gtk::accessible::Property::Description(
                state.reason.unwrap_or(self.group),
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
            self.widget.add_css_class("command-unavailable");
        } else {
            self.widget.remove_css_class("command-unavailable");
        }
        content.append(&text);
        let hint = gtk::Label::new(Some(if state.current {
            "Current"
        } else {
            state.shortcut
        }));
        hint.add_css_class("search-hint");
        content.append(&hint);
        self.widget.set_child(Some(&content));
    }
}

fn record_recent(recent: &mut Vec<usize>, index: usize) {
    recent.retain(|previous| *previous != index);
    recent.insert(0, index);
    recent.truncate(5);
}
