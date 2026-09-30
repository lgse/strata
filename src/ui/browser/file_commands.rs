// SPDX-License-Identifier: MIT

use std::{path::PathBuf, rc::Rc};

use gtk::{glib, prelude::*};

use super::{
    BrowserView,
    clipboard::{self, copy_locations, copy_names},
    desktop::{can_open_terminal, launch_terminal},
    paths::is_trash_location,
};
use crate::{
    adapters::gio_file_for_location,
    model::{FileEntry, Location, SortDirection, SortKey},
    services::{ActionHandle, InvocationSource},
};

pub(crate) use super::transfer::ConflictFocus;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Yank {
    Nothing,
    Done,
    Refused,
}

pub(crate) struct NumberedActions {
    pub(crate) targets: Vec<Location>,
    pub(crate) actions: Vec<Rc<ActionHandle>>,
    paths: Vec<PathBuf>,
    parent: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum KeyboardRefocus {
    Sort(usize),
    Rename(crate::services::OperationRequestId),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CreateRefusal {
    Unsupported,
    Invalid(&'static str),
    Exists(String),
}

impl BrowserView {
    pub fn command_targets(&self) -> Vec<FileEntry> {
        if let Some(results) = self.selected_search_results() {
            if !results.is_empty() {
                return results;
            }
            return self.selected_search_result().into_iter().collect();
        }
        self.state.sync_mode_selection();
        self.focused_listing_depth()
            .map(|depth| self.state.browser.command_entries(depth))
            .unwrap_or_default()
    }

    pub fn focused_target(&self) -> Option<FileEntry> {
        if self.selected_search_results().is_some() {
            return self.selected_search_result();
        }
        self.state.sync_mode_selection();
        self.focused_listing_depth()
            .and_then(|depth| self.state.browser.cursor_entry(depth))
    }

    pub fn yank_targets(&self, cut: bool) -> Yank {
        let entries = self.command_targets();
        if entries.is_empty() {
            return Yank::Nothing;
        }
        if !cut {
            self.state.copy_entries(&entries);
            return Yank::Done;
        }
        if self.state.cut_entries(&entries) {
            Yank::Done
        } else {
            Yank::Refused
        }
    }

    pub fn unyank(&self) {
        clipboard::unyank();
    }

    pub fn copy_target_text(&self, names: bool) -> bool {
        let entries = self.command_targets();
        if entries.is_empty() {
            return false;
        }
        if names {
            copy_names(&entries);
        } else {
            copy_locations(&entries);
        }
        true
    }

    pub fn paste_preferring(&self, focus: ConflictFocus, nothing: Rc<dyn Fn()>) {
        if !clipboard::clipboard_may_paste() {
            nothing();
            return;
        }
        if let Some(location) = self.paste_location() {
            self.state.paste_preferring(location, focus, nothing);
        }
    }

    pub fn confirm_targets_delete(&self, permanent: bool) -> bool {
        let entries = self.command_targets();
        if entries.is_empty() {
            return false;
        }
        let permanent = permanent || self.focused_location_is_trash();
        self.state.request_confirmed_delete(entries, permanent);
        true
    }

    pub fn can_create_entry(&self) -> bool {
        self.new_entry_parent()
            .is_some_and(|(_, parent)| !is_trash_location(&parent) && !parent.is_recent_location())
    }

    pub fn create_typed_entry(&self, text: &str) -> Result<(), CreateRefusal> {
        let (name, directory) = match text.strip_suffix('/') {
            Some(name) => (name, true),
            None => (text, false),
        };
        crate::services::validate_basename(name).map_err(CreateRefusal::Invalid)?;
        let Some((_, parent)) = self
            .new_entry_parent()
            .filter(|(_, parent)| !is_trash_location(parent) && !parent.is_recent_location())
        else {
            return Err(CreateRefusal::Unsupported);
        };
        // The operation still checks collisions atomically; this provides editable feedback.
        if let Some(path) = parent.native_path()
            && std::fs::symlink_metadata(path.join(name)).is_ok()
        {
            return Err(CreateRefusal::Exists(name.to_owned()));
        }
        self.state
            .browser
            .create_exact_entry(parent, name.to_owned(), directory);
        Ok(())
    }
}

pub(crate) fn can_rename(entry: &FileEntry) -> bool {
    !is_trash_location(&entry.location)
}

impl BrowserView {
    /// Renames `entry` to exactly `text`. An unchanged name does nothing, and an
    /// invalid or occupied one is refused before anything is renamed; the
    /// operation's own failures are reported like an inline rename's.
    pub fn rename_typed_entry(&self, entry: FileEntry, text: &str) -> Result<(), CreateRefusal> {
        if !can_rename(&entry) {
            return Err(CreateRefusal::Unsupported);
        }
        crate::services::validate_basename(text).map_err(CreateRefusal::Invalid)?;
        if entry.display_name == text || entry.native_name == std::ffi::OsStr::new(text) {
            return Ok(());
        }
        // The rename itself refuses to replace; this only keeps the prompt open to fix the name.
        if let Some(parent) = entry
            .location
            .native_path()
            .and_then(std::path::Path::parent)
            && std::fs::symlink_metadata(parent.join(text)).is_ok()
        {
            return Err(CreateRefusal::Exists(text.to_owned()));
        }
        if let Some(request) = self.state.browser.rename(entry, text.to_owned()) {
            self.state
                .keyboard_refocus
                .set(Some(KeyboardRefocus::Rename(request)));
        }
        Ok(())
    }
}

impl super::ViewState {
    pub(super) fn finish_keyboard_refocus(
        self: &Rc<Self>,
        finished: KeyboardRefocus,
        completed: bool,
    ) {
        if self.keyboard_refocus.get() != Some(finished) {
            return;
        }
        self.keyboard_refocus.set(None);
        if completed {
            self.refocus_cursor();
        }
    }

    fn refocus_cursor(self: &Rc<Self>) {
        const FRAMES: u8 = 12;
        let state = Rc::downgrade(self);
        let frames = std::cell::Cell::new(0u8);
        self.overlay.add_tick_callback(move |_, _| {
            let Some(state) = state.upgrade() else {
                return glib::ControlFlow::Break;
            };
            // A replaced row briefly keeps focus outside its list; wait it out.
            let on_pane = state
                .overlay
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focus| {
                    focus.is::<gtk::Stack>()
                        || focus.is::<gtk::ListView>()
                        || focus.is::<gtk::GridView>()
                });
            if on_pane
                && (BrowserView {
                    state: state.clone(),
                })
                .item_view_has_focus()
            {
                state.browser.focus_active();
            }
            frames.set(frames.get() + 1);
            if frames.get() < FRAMES {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }
}

impl BrowserView {
    pub fn sort_focused_pane(&self, key: SortKey, direction: SortDirection) {
        if let Some(depth) = self.focused_listing_depth() {
            self.state.browser.set_sort(depth, key, direction);
            self.state
                .keyboard_refocus
                .set(Some(KeyboardRefocus::Sort(depth)));
        }
    }

    /// Opens a terminal in the keyboard-focused pane's folder, ignoring the
    /// cursor and selection. Returns false where no terminal can open.
    pub fn open_focused_folder_terminal(&self) -> bool {
        let Some(location) = self
            .focused_listing_depth()
            .and_then(|depth| self.state.browser.location_at(depth))
            .filter(can_open_terminal)
        else {
            return false;
        };
        launch_terminal(&location, &self.state.overlay);
        true
    }

    pub fn toggle_hidden_from_keys(&self) {
        self.state.browser.toggle_hidden();
        self.state.refocus_cursor();
    }

    pub(crate) fn numbered_actions(&self) -> NumberedActions {
        let entries = self.command_targets();
        let targets = entries.iter().map(|entry| entry.location.clone()).collect();
        let parent = self
            .focused_listing_depth()
            .and_then(|depth| self.state.browser.location_at(depth))
            .and_then(|location| location.native_path().map(PathBuf::from));
        let (inputs, paths) = match (
            crate::ui::actions::inputs_for_entries(&entries),
            crate::ui::actions::native_paths(&entries),
        ) {
            (Some(inputs), Some(paths)) if parent.is_some() => (inputs, paths),
            _ => (Vec::new(), Vec::new()),
        };
        let actions = if inputs.is_empty() {
            Vec::new()
        } else {
            crate::ui::actions::shared().catalog().numbered(&inputs)
        };
        NumberedActions {
            targets,
            actions,
            paths,
            parent,
        }
    }

    pub(crate) fn run_numbered_action(&self, numbered: NumberedActions, action: Rc<ActionHandle>) {
        let Some(parent) = numbered.parent else {
            return;
        };
        let browser = Rc::downgrade(&self.state.browser);
        crate::ui::actions::run_action(
            &self.state.overlay,
            action,
            numbered.paths,
            parent,
            InvocationSource::Selection,
            Some(Rc::new(move || {
                if let Some(browser) = browser.upgrade() {
                    browser.focus_active();
                }
            })),
        );
    }

    /// Looks up applications for the fill, or the focused item, off the key
    /// handler, then shows Open With over them. The chooser appears only while
    /// `current` holds and the targets and folder are unchanged; otherwise the
    /// lookup ends silently. `report` receives why nothing can be chosen.
    /// `None` means there was nothing to open.
    pub fn open_targets_with(
        &self,
        current: Rc<dyn Fn() -> bool>,
        report: Rc<dyn Fn(&str)>,
    ) -> Option<glib::JoinHandle<()>> {
        let entries = self.command_targets();
        if entries.is_empty() {
            return None;
        }
        let locations: Vec<Location> = entries.iter().map(|entry| entry.location.clone()).collect();
        let files: Vec<_> = locations.iter().map(gio_file_for_location).collect();
        let folder = self.state.browser.active_location();
        let state = Rc::downgrade(&self.state);
        Some(glib::MainContext::default().spawn_local(async move {
            let unchanged = || {
                current()
                    && state.upgrade().is_some_and(|state| {
                        let view = BrowserView { state };
                        view.state.browser.active_location() == folder
                            && view
                                .command_targets()
                                .iter()
                                .map(|entry| &entry.location)
                                .eq(locations.iter())
                    })
            };
            let Some(resolved) = crate::ui::open_with::resolve(&files, &unchanged).await else {
                return;
            };
            let Some(state) = state.upgrade() else {
                return;
            };
            let applications = match resolved {
                Ok(applications) => applications,
                Err(reason) => return report(reason),
            };
            if let Some(reason) = applications.unavailable_reason() {
                return report(reason);
            }
            let browser = Rc::downgrade(&state.browser);
            crate::ui::open_with::show(
                &state.overlay,
                files,
                applications.content_types,
                applications.recommended,
                applications.other,
                crate::ui::open_with::OpenWithContext::Explicit,
                Rc::new(move || {
                    if let Some(browser) = browser.upgrade() {
                        browser.focus_active();
                    }
                }),
            );
        }))
    }
}
