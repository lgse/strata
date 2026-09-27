// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{gdk, gio, glib, prelude::*};

use crate::ui::{
    browser::{
        BrowserView, PinStatus,
        palette::{FileCommand, PaletteTarget},
    },
    preferences::PreferenceManager,
    shortcut_footer::ShortcutFooter,
    shortcut_reference::{ContextHint, context_hint_for},
};

use super::{
    super::WindowContent,
    catalogue::{Command, CommandSpec},
};

pub(super) struct Commands {
    window: glib::WeakRef<gtk::ApplicationWindow>,
    browser: BrowserView,
    preferences: Rc<PreferenceManager>,
    sidebar: gtk::ToggleButton,
    shortcuts: ShortcutFooter,
}

#[derive(PartialEq)]
pub(super) struct CommandState {
    pub label: &'static str,
    pub reason: Option<&'static str>,
    pub current: bool,
    pub shortcut: &'static str,
}

impl Commands {
    pub(super) fn new(
        window: &gtk::ApplicationWindow,
        content: &WindowContent,
        preferences: &Rc<PreferenceManager>,
    ) -> Self {
        Self {
            window: window.downgrade(),
            browser: content.browser.clone(),
            preferences: preferences.clone(),
            sidebar: content.header.sidebar_toggle.clone(),
            shortcuts: content.footer.shortcuts.clone(),
        }
    }

    pub(super) fn state(&self, spec: &CommandSpec, target: &PaletteTarget) -> CommandState {
        let mut state = CommandState {
            label: spec.label,
            reason: None,
            current: false,
            shortcut: self.shortcut(spec, target),
        };
        match spec.command {
            Command::Hidden if self.preferences.sort_preferences().show_hidden => {
                state.label = "Hide hidden files";
            }
            Command::Sidebar if self.sidebar.is_active() => state.label = "Hide sidebar",
            Command::View(mode) => state.current = self.browser.view_mode() == mode,
            Command::File(file) => {
                state.reason = self.browser.palette_file_unavailable(file, target);
                if file == FileCommand::Pin
                    && self.browser.palette_pin_status(target) == PinStatus::Pinned
                {
                    state.label = "Unpin folder";
                    state.shortcut = "";
                }
            }
            Command::Terminal => {
                state.reason = self.browser.palette_target_unavailable(target).or_else(|| {
                    self.browser
                        .palette_terminal_location(target)
                        .is_none()
                        .then_some("Open a local folder first")
                });
            }
            Command::Filter if self.preferences.tenxer_mode() => {
                state.reason = Some("Pane filtering is unavailable in 10xer mode");
            }
            Command::Filter | Command::Refresh | Command::Location => {
                state.reason = self.browser.palette_target_unavailable(target);
            }
            _ => {}
        }
        state
    }

    fn shortcut(&self, spec: &CommandSpec, target: &PaletteTarget) -> &'static str {
        let tenxer = self.preferences.tenxer_mode();
        match spec.command {
            Command::File(FileCommand::CopyPaths | FileCommand::Pin)
                if self.preferences.type_to_search_active() || !target.accepts_item_shortcuts() =>
            {
                ""
            }
            Command::File(FileCommand::CopyPaths) => {
                context_hint_for(ContextHint::CopyPaths, tenxer)
            }
            Command::File(FileCommand::Pin) => context_hint_for(ContextHint::Pin, tenxer),
            Command::File(FileCommand::Rename) => context_hint_for(ContextHint::Rename, tenxer),
            Command::File(FileCommand::Duplicate) => {
                context_hint_for(ContextHint::Duplicate, tenxer)
            }
            Command::Terminal => context_hint_for(ContextHint::Terminal, tenxer),
            Command::Filter | Command::RecentFolders | Command::Sidebar if tenxer => "",
            Command::Shortcuts if tenxer => "F1 / ~",
            _ => spec.shortcut,
        }
    }

    pub(super) fn execute(
        &self,
        spec: &CommandSpec,
        target: &PaletteTarget,
        dismiss: impl FnOnce(),
    ) -> Result<(), &'static str> {
        if let Some(reason) = self.state(spec, target).reason {
            return Err(reason);
        }
        dismiss();
        match spec.command {
            Command::Search => self.action("search"),
            Command::RecentFolders => self.action("jump-folder"),
            Command::Settings => self.action("settings"),
            Command::Terminal => self.browser.execute_palette_terminal(target),
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
                super::super::super::apply_browser_mode(&self.browser, &self.preferences, mode);
            }
            Command::Hidden => self.browser.browser().toggle_hidden(),
            Command::Sidebar => self.sidebar.set_active(!self.sidebar.is_active()),
            Command::File(file) => self.browser.execute_palette_file(file, target),
        }
        Ok(())
    }

    fn action(&self, name: &str) {
        if let Some(window) = self.window.upgrade() {
            gio::prelude::ActionGroupExt::activate_action(&window, name, None);
        }
    }
}
