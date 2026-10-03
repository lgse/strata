// SPDX-License-Identifier: MIT

//! 10xer footer prompts. While a prompt has focus only its own keys act; every
//! other key edits the text and never reaches a browsing command.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::{self, Propagation},
};

use super::{Dispatcher, KeyResult, command_modifiers};
use crate::{
    app::Browser,
    model::{FileEntry, Location},
    services::NavigationHistory,
    ui::{
        browser::{BrowserView, CreateRefusal},
        folder_picker::{self, FolderPicker, Refused, Request},
        shortcut_footer::{CandidateKeys, ShortcutFooter},
        tenxer_mode::Prompt,
    },
};

fn plain(modifiers: Modifiers) -> bool {
    !command_modifiers(modifiers)
        .intersects(Modifiers::CONTROL_MASK | Modifiers::ALT_MASK | Modifiers::SUPER_MASK)
}

impl Dispatcher {
    /// Shift is ignored because some layouts type **/** with it and **?** / **N** need it.
    pub(super) fn tenxer_prompt_keys(&self, key: Key, modifiers: Modifiers) -> KeyResult {
        if !plain(modifiers) || !self.view.item_view_has_focus() {
            return None;
        }
        match key {
            Key::slash | Key::KP_Divide => self.shortcuts.open_prompt(Prompt::Find),
            Key::question => self.shortcuts.open_prompt(Prompt::FindBackward),
            Key::z | Key::Z => {
                let kind = if key == Key::z {
                    Prompt::Jump
                } else {
                    Prompt::Recent
                };
                let opened = self.shortcuts.open_prompt(kind);
                let browser = self.view.browser();
                show_history_candidates(&self.shortcuts, &self.history, &browser, kind);
                opened
            }
            Key::n | Key::N => {
                self.repeat_find(key == Key::N);
                return Some(Propagation::Stop);
            }
            Key::f => {
                let query = self.view.listing_filter().unwrap_or_default();
                self.shortcuts.open_prompt_with(Prompt::Filter, &query)
            }
            Key::s => match self.view.begin_listing_search() {
                Some(query) => self.shortcuts.open_prompt_with(Prompt::Search, &query),
                None => {
                    self.shortcuts.show_feedback("Nothing to search");
                    true
                }
            },
            _ => return None,
        };
        Some(Propagation::Stop)
    }

    fn repeat_find(&self, reverse: bool) {
        match self.view.repeat_find(reverse, true) {
            None => self.shortcuts.show_feedback("No previous find"),
            Some(false) => self.report_miss(),
            Some(true) => {}
        }
    }

    fn report_miss(&self) {
        let query = self.view.find_query().unwrap_or_default();
        self.shortcuts
            .show_feedback(&format!("No matches for \u{201c}{query}\u{201d}"));
    }

    pub(super) fn prompt_key(
        &self,
        browser: &Browser,
        key: Key,
        modifiers: Modifiers,
    ) -> Propagation {
        let preferences = &self.type_to_search.preferences;
        if crate::ui::tenxer_mode::is_toggle_shortcut(key, modifiers) {
            preferences.set_tenxer_mode(!preferences.tenxer_mode());
            return Propagation::Stop;
        }
        if !plain(modifiers) {
            return Propagation::Proceed;
        }
        let kind = self.shortcuts.open_prompt_kind();
        match key {
            Key::Escape
                if kind.is_some_and(|kind| {
                    matches!(kind, Prompt::Create)
                        || kind.picks_folder()
                        || kind.holds_targets()
                        || kind.picks_history()
                }) =>
            {
                self.return_to_listing(browser)
            }
            Key::Escape => {
                if self.shortcuts.open_prompt_kind() == Some(Prompt::Filter) {
                    self.shortcuts.dismiss_prompt();
                    if !self.view.clear_listing_filter() {
                        browser.focus_active();
                    }
                    return Propagation::Stop;
                }
                if self.shortcuts.open_prompt_kind() == Some(Prompt::Search) {
                    let text = self.shortcuts.prompt_text();
                    self.shortcuts.dismiss_prompt();
                    self.view.commit_listing_search(&text);
                    return Propagation::Stop;
                }
                self.view.dismiss_find_highlight();
                self.return_to_listing(browser);
            }
            Key::Return | Key::KP_Enter => self.submit_prompt(browser),
            Key::Up | Key::KP_Up | Key::Down | Key::KP_Down
                if kind.is_some_and(Prompt::picks_history) =>
            {
                let delta = if matches!(key, Key::Up | Key::KP_Up) {
                    -1
                } else {
                    1
                };
                self.shortcuts.step_candidate(delta);
                show_candidate_hint(&self.shortcuts);
            }
            Key::Tab | Key::KP_Tab | Key::ISO_Left_Tab
                if kind.is_some_and(Prompt::picks_folder) =>
            {
                if let Some(chosen) = self.shortcuts.chosen_candidate() {
                    let current = active_folder(browser);
                    let text =
                        folder_picker::typed_path(&chosen, current.as_deref(), &glib::home_dir());
                    self.shortcuts.type_prompt_text(&text);
                }
            }
            Key::Up | Key::KP_Up | Key::Down | Key::KP_Down
                if kind.is_some_and(Prompt::picks_folder) =>
            {
                let delta = if matches!(key, Key::Up | Key::KP_Up) {
                    -1
                } else {
                    1
                };
                self.shortcuts.step_candidate(delta);
                if self.shortcuts.candidate_position().is_some() {
                    show_candidate_hint(&self.shortcuts);
                }
            }
            Key::Up | Key::KP_Up | Key::Down | Key::KP_Down
                if kind.is_some_and(Prompt::holds_targets) => {}
            Key::Up | Key::KP_Up => self.view.step_cursor_unfocused(-1),
            Key::Down | Key::KP_Down => self.view.step_cursor_unfocused(1),
            _ => return Propagation::Proceed,
        }
        Propagation::Stop
    }

    fn submit_prompt(&self, browser: &Browser) {
        let text = self.shortcuts.prompt_text();
        let found = match self.shortcuts.open_prompt_kind() {
            Some(Prompt::Filter) => {
                self.shortcuts.dismiss_prompt();
                self.view.commit_listing_filter(&text);
                return;
            }
            Some(Prompt::Search) => {
                self.shortcuts.dismiss_prompt();
                self.view.commit_listing_search(&text);
                return;
            }
            Some(kind @ (Prompt::Go | Prompt::MoveTo | Prompt::CopyTo | Prompt::ExtractTo)) => {
                self.submit_folder(kind, text);
                return;
            }
            Some(Prompt::Jump | Prompt::Recent) => {
                let Some(path) = self.shortcuts.chosen_candidate() else {
                    show_candidate_hint(&self.shortcuts);
                    return;
                };
                self.return_to_listing(browser);
                self.view.keyboard_navigation();
                self.view
                    .browser()
                    .navigate_with_selection(Location::local(path), true);
                return;
            }
            Some(Prompt::Create) => {
                self.submit_create(browser, &text);
                return;
            }
            Some(Prompt::Rename) => {
                self.submit_rename(browser, &text);
                return;
            }
            _ if text.is_empty() => true,
            Some(kind @ (Prompt::Find | Prompt::FindBackward)) => {
                self.view.find(&text, kind == Prompt::FindBackward, false)
            }
            None => true,
        };
        // The cursor already moved under the prompt; one focus move follows it.
        self.return_to_listing(browser);
        if !found {
            self.report_miss();
        }
    }

    /// Acts at once on a folder stepped to with **↑** / **↓**, or on a typed
    /// path that exists. Otherwise Enter waits for the search to finish, so
    /// the best match wins rather than the first found.
    fn submit_folder(&self, kind: Prompt, text: String) {
        let prompt = FolderPrompt {
            view: self.view.clone(),
            shortcuts: self.shortcuts.clone(),
            picker: self.destinations.clone(),
            targets: self.destination_targets.clone(),
            revision: self.destination_revision.clone(),
        };
        if self.shortcuts.candidate_stepped() {
            return prompt.submit(kind, &text, None);
        }
        let current = active_folder(&self.view.browser());
        let Some(target) =
            folder_picker::typed_target(&text, current.as_deref(), &glib::home_dir())
        else {
            return prompt.submit_when_settled(kind, text);
        };
        glib::MainContext::default().spawn_local(async move {
            let probed = target.clone();
            let exists = gtk::gio::spawn_blocking(move || probed.exists()).await;
            // Go shows a file in its folder; a transfer reports it is no folder.
            if exists.unwrap_or(false) {
                prompt.submit(kind, &text, Some(target));
            } else {
                prompt.submit_when_settled(kind, text);
            }
        });
    }

    fn submit_create(&self, browser: &Browser, text: &str) {
        let hint = match self.view.create_typed_entry(text) {
            Ok(()) => return self.return_to_listing(browser),
            Err(CreateRefusal::Invalid(message)) => message.to_owned(),
            Err(CreateRefusal::Exists(name)) => {
                format!("\u{201c}{name}\u{201d} already exists")
            }
            Err(CreateRefusal::Unsupported) => {
                self.return_to_listing(browser);
                self.shortcuts
                    .show_feedback("Can\u{2019}t create items here");
                return;
            }
        };
        self.shortcuts
            .prompt_sink(Prompt::Create)
            .show(None, Some(&hint));
    }

    fn submit_rename(&self, browser: &Browser, text: &str) {
        let Some(entry) = self.rename_target.borrow().clone() else {
            return self.return_to_listing(browser);
        };
        let hint = match self.view.rename_typed_entry(entry, text) {
            Ok(()) => return self.return_to_listing(browser),
            Err(CreateRefusal::Invalid(message)) => message.to_owned(),
            Err(CreateRefusal::Exists(name)) => {
                format!("\u{201c}{name}\u{201d} already exists")
            }
            Err(CreateRefusal::Unsupported) => {
                self.return_to_listing(browser);
                self.shortcuts
                    .show_feedback("Can\u{2019}t rename items here");
                return;
            }
        };
        self.shortcuts
            .prompt_sink(Prompt::Rename)
            .show(None, Some(&hint));
    }

    fn return_to_listing(&self, browser: &Browser) {
        return_to_listing(&self.shortcuts, &self.view, browser);
    }
}

/// Closes the open prompt and gives the listing keyboard focus back.
pub(super) fn return_to_listing(shortcuts: &ShortcutFooter, view: &BrowserView, browser: &Browser) {
    shortcuts.dismiss_prompt();
    if !view.focus_visible_results() {
        browser.focus_active();
    }
}

pub(super) fn show_history_candidates(
    shortcuts: &ShortcutFooter,
    history: &NavigationHistory,
    browser: &Browser,
    kind: Prompt,
) {
    if !kind.picks_history() || shortcuts.open_prompt_kind() != Some(kind) {
        return;
    }
    let text = shortcuts.prompt_text();
    let current = browser.active_location();
    let excluded = current.as_ref().and_then(Location::native_path);
    let items = if kind == Prompt::Jump {
        history.search_excluding(&text, excluded)
    } else {
        history.recent_excluding(&text, excluded)
    };
    let paths = items.into_iter().map(|item| item.path).collect();
    let keys = CandidateKeys {
        enter: "Open",
        tab: None,
    };
    shortcuts.show_candidates(paths, keys);
    show_candidate_hint(shortcuts);
}

/// What a **g Space**, **M**, **C**, or **; E** prompt needs to act on the
/// chosen folder after the key that asked for it.
struct FolderPrompt {
    view: BrowserView,
    shortcuts: ShortcutFooter,
    picker: FolderPicker,
    targets: Rc<RefCell<Vec<FileEntry>>>,
    revision: Rc<Cell<u64>>,
}

impl FolderPrompt {
    fn submit_when_settled(self, kind: Prompt, text: String) {
        if self.picker.is_pending() {
            self.shortcuts
                .prompt_sink(kind)
                .show(None, Some(folder_picker::SEARCHING));
        }
        let picker = self.picker.clone();
        picker.when_settled(move || self.submit(kind, &text, None));
    }

    /// Acts on `typed`, an existing path the text names, or else on the
    /// chosen folder, unless the prompt changed meanwhile.
    fn submit(&self, kind: Prompt, text: &str, typed: Option<PathBuf>) {
        if self.shortcuts.open_prompt_kind() != Some(kind) || self.shortcuts.prompt_text() != text {
            return;
        }
        if kind == Prompt::Go {
            self.open(text, typed.is_some());
        } else {
            self.send(kind, text, typed);
        }
    }

    /// Opens the chosen folder. With none listed, or when the text names an
    /// existing path, the text goes to navigation as **Ctrl+L** would take
    /// it, so URIs and files still open.
    fn open(&self, text: &str, typed: bool) {
        let chosen = (!typed)
            .then(|| self.shortcuts.chosen_candidate())
            .flatten();
        // Closing clears the entry before navigation can show a dialog.
        self.return_to_listing();
        if text.trim().is_empty() {
            return;
        }
        self.view.keyboard_navigation();
        match chosen {
            Some(folder) => self
                .view
                .browser()
                .navigate_with_selection(Location::local(folder), true),
            None => self.view.open_typed_location(text),
        }
    }

    fn send(&self, kind: Prompt, text: &str, typed: Option<PathBuf>) {
        if text.trim().is_empty() || self.targets.borrow().is_empty() {
            return self.return_to_listing();
        }
        let Some(destination) = typed.or_else(|| self.shortcuts.chosen_candidate()) else {
            let current = active_folder(&self.view.browser());
            let reason = folder_picker::scope(text, current.as_deref(), &glib::home_dir())
                .err()
                .unwrap_or(folder_picker::NO_MATCHES);
            return self.shortcuts.prompt_sink(kind).show(None, Some(reason));
        };
        send_to_destination(
            &self.view,
            &self.shortcuts,
            &self.targets,
            &self.revision,
            kind,
            destination,
        );
    }

    fn return_to_listing(&self) {
        return_to_listing(&self.shortcuts, &self.view, &self.view.browser());
    }
}

/// Lists the folders a **g Space**, **M**, **C**, or **; E** prompt's text
/// picks out.
pub(super) fn show_folder_candidates(
    shortcuts: &ShortcutFooter,
    picker: &FolderPicker,
    browser: &Browser,
    targets: &[FileEntry],
    kind: Prompt,
) {
    if !kind.picks_folder() || shortcuts.open_prompt_kind() != Some(kind) {
        return;
    }
    let text = shortcuts.prompt_text();
    let current = active_folder(browser);
    let home = glib::home_dir();
    let request = Request {
        text: &text,
        current: current.as_deref(),
        home: &home,
        show_hidden: browser.preferences().show_hidden,
        uris: kind == Prompt::Go,
        refused: refused_destinations(kind, targets),
    };
    let shortcuts = shortcuts.clone();
    picker.update(request, move |shown| {
        if shortcuts.open_prompt_kind() != Some(kind) {
            return;
        }
        let listed = !shown.paths.is_empty();
        let keys = CandidateKeys {
            enter: destination_action(kind),
            tab: Some("Complete"),
        };
        shortcuts.show_candidates(shown.paths, keys);
        if listed {
            show_candidate_hint(&shortcuts);
        } else {
            shortcuts.prompt_sink(kind).show(None, shown.hint);
        }
    });
}

fn destination_action(kind: Prompt) -> &'static str {
    match kind {
        Prompt::MoveTo => "Move",
        Prompt::CopyTo => "Copy",
        Prompt::ExtractTo => "Extract",
        _ => "Open",
    }
}

/// Folders a move or copy would refuse: the folders being sent, and for a
/// move the folder the targets already share.
fn refused_destinations(kind: Prompt, targets: &[FileEntry]) -> Refused {
    if !matches!(kind, Prompt::MoveTo | Prompt::CopyTo) {
        return Refused::default();
    }
    let trees = targets
        .iter()
        .filter(|entry| entry.is_directory())
        .filter_map(|entry| entry.location.native_path().map(Path::to_path_buf))
        .collect();
    let mut parents = targets.iter().map(|entry| {
        entry
            .location
            .native_path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    });
    let first = parents.next().flatten();
    let folder = (kind == Prompt::MoveTo && parents.all(|parent| parent == first))
        .then_some(first)
        .flatten();
    Refused { trees, folder }
}

fn active_folder(browser: &Browser) -> Option<PathBuf> {
    browser
        .active_location()
        .and_then(|location| location.native_path().map(Path::to_path_buf))
}

fn show_candidate_hint(shortcuts: &ShortcutFooter) {
    let Some(kind) = shortcuts.open_prompt_kind() else {
        return;
    };
    let hint = match shortcuts.candidate_position() {
        None => Some(folder_picker::NO_MATCHES.to_owned()),
        Some((_, 1)) => None,
        Some((index, count)) => Some(format!("{} of {count}", index + 1)),
    };
    shortcuts.prompt_sink(kind).show(None, hint.as_deref());
}

/// Sends the prompt's fixed targets into `destination` once it is confirmed
/// to be a folder, unless the prompt changed meanwhile.
pub(super) fn send_to_destination(
    view: &BrowserView,
    shortcuts: &ShortcutFooter,
    targets: &RefCell<Vec<FileEntry>>,
    revision: &Rc<Cell<u64>>,
    kind: Prompt,
    destination: PathBuf,
) {
    let mut targets = targets.borrow().clone();
    if targets.is_empty() {
        return;
    }
    let revision = revision.clone();
    let submitted = revision.get().wrapping_add(1);
    revision.set(submitted);
    let view = view.clone();
    let shortcuts = shortcuts.clone();
    let navigation = view.browser().navigation_generation();
    let submitted_text = shortcuts.prompt_text();
    gtk::glib::MainContext::default().spawn_local(async move {
        let result = gtk::gio::spawn_blocking(move || match std::fs::metadata(&destination) {
            Ok(metadata) if metadata.is_dir() => Ok(destination),
            Ok(_) => Err("Not a folder"),
            Err(_) => Err("No such folder"),
        })
        .await;
        if revision.get() != submitted
            || view.browser().navigation_generation() != navigation
            || shortcuts.open_prompt_kind() != Some(kind)
            || shortcuts.prompt_text() != submitted_text
        {
            return;
        }
        let result = match result {
            Ok(Ok(destination)) if kind == Prompt::ExtractTo => {
                view.extract_to_folder(targets.remove(0), destination);
                Ok(())
            }
            Ok(Ok(destination)) => {
                view.transfer_to_folder(targets, destination, kind == Prompt::MoveTo)
            }
            Ok(Err(reason)) => Err(reason),
            Err(_) => Err("Unable to check folder"),
        };
        match result {
            Ok(()) => return_to_listing(&shortcuts, &view, &view.browser()),
            Err(reason) => shortcuts.prompt_sink(kind).show(None, Some(reason)),
        }
    });
}
