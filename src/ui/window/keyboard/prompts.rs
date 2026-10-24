// SPDX-License-Identifier: MIT

//! Browser filter navigation and 10xer footer prompts. A prompt owns its keys; every
//! other key edits the text and never reaches a browsing command.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{
    gdk::ModifierType as Modifiers,
    glib::{self, Propagation},
    prelude::*,
};

use super::{Dispatcher, KeyResult};
use crate::{
    app::Browser,
    model::{FileEntry, Location},
    services::NavigationHistory,
    ui::{
        browser::{BrowserView, CreateRefusal},
        folder_picker::{self, FolderPicker, Refused, Request},
        keys::{KeyPress, bindings::*},
        shortcut_footer::{CandidateKeys, ShortcutFooter},
        tenxer_mode::Prompt,
    },
};

const SHIFT: Modifiers = Modifiers::SHIFT_MASK;

pub(super) struct FilterDescent {
    navigation: u64,
    query: String,
    input: Option<FilterEntry>,
}

struct FilterEntry {
    entry: glib::WeakRef<gtk::Entry>,
    changed: Option<glib::SignalHandlerId>,
}

impl Drop for FilterEntry {
    fn drop(&mut self) {
        if let (Some(entry), Some(changed)) = (self.entry.upgrade(), self.changed.take()) {
            entry.disconnect(changed);
        }
    }
}

impl FilterDescent {
    fn is_current(&self, view: &BrowserView, shortcuts: &ShortcutFooter) -> bool {
        if view.browser().navigation_generation() != self.navigation {
            return false;
        }
        match &self.input {
            Some(input) => input.entry.upgrade().is_some_and(|entry| {
                entry
                    .root()
                    .and_then(|root| root.focus())
                    .is_some_and(|focus| focus == entry || focus.is_ancestor(&entry))
                    && entry.text() == self.query
            }),
            None => {
                shortcuts.open_prompt_kind() == Some(Prompt::Filter)
                    && shortcuts.prompt_has_focus()
                    && shortcuts.prompt_text() == self.query
            }
        }
    }
}

pub(super) fn cancel_stale_filter_descent(
    view: &BrowserView,
    shortcuts: &ShortcutFooter,
    pending: &RefCell<Option<FilterDescent>>,
) {
    let stale = pending
        .borrow()
        .as_ref()
        .is_some_and(|pending| !pending.is_current(view, shortcuts));
    if stale {
        pending.take();
    }
}

pub(super) fn finish_filter_descent(
    view: &BrowserView,
    shortcuts: &ShortcutFooter,
    pending: &RefCell<Option<FilterDescent>>,
) {
    cancel_stale_filter_descent(view, shortcuts, pending);
    if pending.borrow().is_none() {
        return;
    }
    let Some(entry) = view.first_settled_filter_match() else {
        return;
    };
    let footer = pending
        .take()
        .is_some_and(|pending| pending.input.is_none());
    if let Some(entry) = entry.filter(FileEntry::is_directory) {
        if footer {
            shortcuts.dismiss_prompt();
        }
        view.keyboard_navigation();
        view.browser().navigate(entry.location);
        if footer {
            shortcuts.open_prompt(Prompt::Filter);
        } else {
            view.show_filter_with_query("");
        }
    }
}

impl Dispatcher {
    pub(super) fn normal_filter_keys(&self, press: &KeyPress) -> KeyResult {
        if self.type_to_search.preferences.tenxer_mode() || !press.is(FILTER_DESCEND) {
            return None;
        }
        let entry = self.view.focused_filter_entry()?;
        self.begin_filter_descent(Some(entry));
        Some(Propagation::Stop)
    }

    fn begin_filter_descent(&self, entry: Option<gtk::Entry>) {
        let query = entry
            .as_ref()
            .map(|entry| entry.text().to_string())
            .unwrap_or_else(|| self.shortcuts.prompt_text());
        if query.trim().is_empty() {
            return;
        }
        let input = entry.map(|entry| {
            let pending = Rc::downgrade(&self.filter_descent);
            let changed = entry.connect_changed(move |_| {
                if let Some(pending) = pending.upgrade() {
                    pending.take();
                }
            });
            FilterEntry {
                entry: entry.downgrade(),
                changed: Some(changed),
            }
        });
        self.filter_descent.replace(Some(FilterDescent {
            navigation: self.view.browser().navigation_generation(),
            query,
            input,
        }));
        finish_filter_descent(&self.view, &self.shortcuts, &self.filter_descent);
    }

    pub(super) fn tenxer_prompt_keys(&self, press: &KeyPress) -> KeyResult {
        if !self.view.item_view_has_focus() {
            return None;
        }
        if press.is(FIND) {
            self.shortcuts.open_prompt(Prompt::Find);
        } else if press.is(FIND_BACKWARD) {
            self.shortcuts.open_prompt(Prompt::FindBackward);
        } else if press.is_any(&[JUMP_VISITED, JUMP_RECENT]) {
            let kind = if press.is(JUMP_VISITED) {
                Prompt::Jump
            } else {
                Prompt::Recent
            };
            self.shortcuts.open_prompt(kind);
            let browser = self.view.browser();
            show_history_candidates(&self.shortcuts, &self.history, &browser, kind);
        } else if press.is_any(&[REPEAT_FIND, REPEAT_FIND_REVERSED]) {
            self.repeat_find(press.is(REPEAT_FIND_REVERSED));
        } else if press.is(FILTER_LISTING) {
            let query = self.view.listing_filter().unwrap_or_default();
            self.shortcuts.open_prompt_with(Prompt::Filter, &query);
        } else if press.is(SEARCH_SUBFOLDERS) {
            match self.view.begin_listing_search() {
                Some(query) => {
                    self.shortcuts.open_prompt_with(Prompt::Search, &query);
                }
                None => self
                    .shortcuts
                    .show_feedback(&crate::i18n::tr("Nothing to search")),
            }
        } else {
            return None;
        }
        Some(Propagation::Stop)
    }

    fn repeat_find(&self, reverse: bool) {
        match self.view.repeat_find(reverse, true) {
            None => self
                .shortcuts
                .show_feedback(&crate::i18n::tr("No previous find")),
            Some(false) => self.report_miss(),
            Some(true) => {}
        }
    }

    fn report_miss(&self) {
        let query = self.view.find_query().unwrap_or_default();
        self.shortcuts
            .show_feedback(&rust_i18n::t!("No matches for “%{query}”", query = query));
    }

    pub(super) fn prompt_key(&self, browser: &Browser, press: &KeyPress) -> Propagation {
        let preferences = &self.type_to_search.preferences;
        if press.is(TENXER_MODE) {
            preferences.set_tenxer_mode(!preferences.tenxer_mode());
            return Propagation::Stop;
        }
        let kind = self.shortcuts.open_prompt_kind();
        let step = if press.is(UP.ignoring(SHIFT)) {
            Some(-1)
        } else if press.is(DOWN.ignoring(SHIFT)) {
            Some(1)
        } else {
            None
        };
        if press.is(ESCAPE_ANY_SHIFT) {
            if kind.is_some_and(|kind| {
                matches!(kind, Prompt::Create)
                    || kind.picks_folder()
                    || kind.holds_targets()
                    || kind.picks_history()
            }) {
                self.return_to_listing(browser);
            } else if kind == Some(Prompt::Filter) {
                self.shortcuts.dismiss_prompt();
                if !self.view.clear_listing_filter() {
                    browser.focus_active();
                }
            } else if kind == Some(Prompt::Search) {
                let text = self.shortcuts.prompt_text();
                self.shortcuts.dismiss_prompt();
                self.view.commit_listing_search(&text);
            } else {
                self.view.dismiss_find_highlight();
                self.return_to_listing(browser);
            }
        } else if press.is(ENTER_ANY_SHIFT) {
            self.submit_prompt(browser);
        } else if kind == Some(Prompt::Filter) && press.is(FILTER_DESCEND) {
            self.begin_filter_descent(None);
        } else if let Some(delta) = step
            && kind.is_some_and(Prompt::picks_history)
        {
            self.shortcuts.step_candidate(delta);
            show_candidate_hint(&self.shortcuts);
        } else if press.is(COMPLETE.ignoring(SHIFT)) && kind.is_some_and(Prompt::picks_folder) {
            if let Some(chosen) = self.shortcuts.chosen_candidate() {
                let current = active_folder(browser);
                let text =
                    folder_picker::typed_path(&chosen, current.as_deref(), &glib::home_dir());
                self.shortcuts.type_prompt_text(&text);
            }
        } else if let Some(delta) = step
            && kind.is_some_and(Prompt::picks_folder)
        {
            self.shortcuts.step_candidate(delta);
            if self.shortcuts.candidate_position().is_some() {
                show_candidate_hint(&self.shortcuts);
            }
        } else if let Some(delta) = step {
            if !kind.is_some_and(Prompt::holds_targets) {
                self.view.step_cursor_unfocused(delta);
            }
        } else {
            return Propagation::Proceed;
        }
        Propagation::Stop
    }

    pub(super) fn submit_prompt(&self, browser: &Browser) {
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

    fn submit_folder(&self, kind: Prompt, text: String) {
        let submitted = self.destination_revision.get().wrapping_add(1);
        self.destination_revision.set(submitted);
        let prompt = FolderPrompt {
            view: self.view.clone(),
            shortcuts: self.shortcuts.clone(),
            picker: self.destinations.clone(),
            targets: self.destination_targets.clone(),
            revision: self.destination_revision.clone(),
            submitted,
            navigation: self.view.browser().navigation_generation(),
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
            if exists.unwrap_or(false) {
                prompt.submit(kind, &text, Some(target));
            } else {
                prompt.submit_when_settled(kind, text);
            }
        });
    }

    fn submit_create(&self, browser: &Browser, text: &str) {
        let entry = match self.view.prepare_typed_entry(text) {
            Ok(entry) => entry,
            Err(refusal) => {
                return refuse_typed_name(
                    &self.shortcuts,
                    &self.view,
                    browser,
                    Prompt::Create,
                    refusal,
                );
            }
        };
        let submitted = self.next_typed_name();
        let revision = self.typed_name_revision.clone();
        let shortcuts = self.shortcuts.clone();
        let view = self.view.clone();
        let text = text.to_owned();
        glib::MainContext::default().spawn_local(async move {
            let refusal = entry.refusal().await;
            let still_open = revision.get() == submitted
                && shortcuts.open_prompt_kind() == Some(Prompt::Create)
                && shortcuts.prompt_text() == text;
            if !still_open {
                return;
            }
            let browser = view.browser();
            match refusal {
                Some(refusal) => {
                    refuse_typed_name(&shortcuts, &view, &browser, Prompt::Create, refusal);
                }
                None => {
                    view.create_typed_entry(entry);
                    return_to_listing(&shortcuts, &view, &browser);
                }
            }
        });
    }

    fn submit_rename(&self, browser: &Browser, text: &str) {
        let Some(entry) = self.rename_target.borrow().clone() else {
            return self.return_to_listing(browser);
        };
        let rename = match self.view.prepare_typed_rename(entry, text) {
            Ok(None) => return self.return_to_listing(browser),
            Ok(Some(rename)) => rename,
            Err(refusal) => {
                return refuse_typed_name(
                    &self.shortcuts,
                    &self.view,
                    browser,
                    Prompt::Rename,
                    refusal,
                );
            }
        };
        let folder = rename
            .entry
            .location
            .parent()
            .map(|folder| crate::adapters::gio_file_for_location(&folder));
        let paths = rename.native_paths();
        let submitted = self.next_typed_name();
        let revision = self.typed_name_revision.clone();
        let target = self.rename_target.clone();
        let shortcuts = self.shortcuts.clone();
        let view = self.view.clone();
        let text = text.to_owned();
        // The rename itself refuses to replace or to take a name the location
        // would store as another; this only keeps the prompt open to fix it.
        glib::MainContext::default().spawn_local(async move {
            let unstorable = match &folder {
                Some(folder) => {
                    crate::adapters::typed_name_refusal(folder, &rename.name, false).await
                }
                None => None,
            };
            let taken = match paths {
                Some((current, wanted)) if unstorable.is_none() => {
                    gtk::gio::spawn_blocking(move || {
                        crate::adapters::typed_rename_is_taken(&current, &wanted)
                    })
                    .await
                    .unwrap_or(false)
                }
                _ => false,
            };
            let still_open = revision.get() == submitted
                && shortcuts.open_prompt_kind() == Some(Prompt::Rename)
                && shortcuts.prompt_text() == text
                && target
                    .borrow()
                    .as_ref()
                    .is_some_and(|entry| entry.location == rename.entry.location);
            if !still_open {
                return;
            }
            let browser = view.browser();
            let refusal = match unstorable {
                Some(message) => Some(CreateRefusal::Unstorable(message)),
                None => taken.then_some(CreateRefusal::Exists(text)),
            };
            match refusal {
                Some(refusal) => {
                    refuse_typed_name(&shortcuts, &view, &browser, Prompt::Rename, refusal);
                }
                None => {
                    view.start_typed_rename(rename);
                    return_to_listing(&shortcuts, &view, &browser);
                }
            }
        });
    }

    fn next_typed_name(&self) -> u64 {
        let submitted = self.typed_name_revision.get().wrapping_add(1);
        self.typed_name_revision.set(submitted);
        submitted
    }

    fn return_to_listing(&self, browser: &Browser) {
        return_to_listing(&self.shortcuts, &self.view, browser);
    }
}

fn refuse_typed_name(
    shortcuts: &ShortcutFooter,
    view: &BrowserView,
    browser: &Browser,
    prompt: Prompt,
    refusal: CreateRefusal,
) {
    let hint = match refusal {
        CreateRefusal::Invalid(message) => crate::i18n::tr(message),
        CreateRefusal::Exists(name) => {
            rust_i18n::t!("“%{name}” already exists", name = name).into_owned()
        }
        CreateRefusal::Unstorable(message) => message,
        CreateRefusal::Unsupported => {
            return_to_listing(shortcuts, view, browser);
            shortcuts.show_feedback(&crate::i18n::tr(if prompt == Prompt::Create {
                "Can’t create items here"
            } else {
                "Can’t rename items here"
            }));
            return;
        }
    };
    shortcuts.prompt_sink(prompt).show(None, Some(&hint));
}

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

struct FolderPrompt {
    view: BrowserView,
    shortcuts: ShortcutFooter,
    picker: FolderPicker,
    targets: Rc<RefCell<Vec<FileEntry>>>,
    revision: Rc<Cell<u64>>,
    submitted: u64,
    navigation: u64,
}

impl FolderPrompt {
    fn is_current(&self, kind: Prompt, text: &str) -> bool {
        self.revision.get() == self.submitted
            && self.view.browser().navigation_generation() == self.navigation
            && self.shortcuts.open_prompt_kind() == Some(kind)
            && self.shortcuts.prompt_text() == text
    }

    fn submit_when_settled(self, kind: Prompt, text: String) {
        if !self.is_current(kind, &text) {
            return;
        }
        if self.picker.is_pending() {
            self.shortcuts
                .prompt_sink(kind)
                .show(None, Some(&crate::i18n::tr(folder_picker::SEARCHING)));
        }
        let picker = self.picker.clone();
        picker.when_settled(move || self.submit(kind, &text, None));
    }

    fn submit(&self, kind: Prompt, text: &str, typed: Option<PathBuf>) {
        if !self.is_current(kind, text) {
            return;
        }
        if kind == Prompt::Go {
            self.open(text, typed.is_some());
        } else {
            self.send(kind, text, typed);
        }
    }

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
            return self
                .shortcuts
                .prompt_sink(kind)
                .show(None, Some(&crate::i18n::tr(reason)));
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
            tab: Some("completion.complete"),
        };
        shortcuts.show_candidates(shown.paths, keys);
        if listed {
            show_candidate_hint(&shortcuts);
        } else {
            shortcuts
                .prompt_sink(kind)
                .show(None, shown.hint.map(crate::i18n::tr).as_deref());
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
        None => Some(crate::i18n::tr(folder_picker::NO_MATCHES)),
        Some((_, 1)) => None,
        Some((index, count)) => Some(
            rust_i18n::t!(
                "%{position} of %{count}",
                position = crate::i18n::integer((index + 1) as u64),
                count = crate::i18n::integer(count as u64)
            )
            .into_owned(),
        ),
    };
    shortcuts.prompt_sink(kind).show(None, hint.as_deref());
}

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
            Err(reason) => shortcuts
                .prompt_sink(kind)
                .show(None, Some(&crate::i18n::tr(reason))),
        }
    });
}
