// SPDX-License-Identifier: MIT

//! 10xer footer prompts. While a prompt has focus only its own keys act; every
//! other key edits the text and never reaches a browsing command.

use gtk::{
    gdk::{Key, ModifierType as Modifiers},
    glib::Propagation,
};

use super::{Dispatcher, KeyResult, command_modifiers};
use crate::{
    app::Browser,
    model::Location,
    services::NavigationHistory,
    ui::{
        browser::CreateRefusal,
        go_completion::{Context, Step},
        shortcut_footer::{PromptSink, ShortcutFooter},
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
            Key::Tab | Key::KP_Tab | Key::ISO_Left_Tab
                if let Some(kind) = kind.filter(|kind| kind.completes_folders()) =>
            {
                let backward =
                    key == Key::ISO_Left_Tab || modifiers.contains(Modifiers::SHIFT_MASK);
                self.complete_folder(browser, kind, backward);
            }
            Key::Escape
                if kind.is_some_and(|kind| {
                    matches!(kind, Prompt::Create)
                        || kind.completes_folders()
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
            Some(Prompt::Go) => {
                // Closing clears the entry before navigation can show a dialog.
                self.return_to_listing(browser);
                if !text.trim().is_empty() {
                    self.view.keyboard_navigation();
                    self.view.open_typed_location(&text);
                }
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
            Some(kind @ (Prompt::MoveTo | Prompt::CopyTo | Prompt::ExtractTo)) => {
                self.submit_destination(browser, kind, &text);
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

    fn complete_folder(&self, browser: &Browser, kind: Prompt, backward: bool) {
        let text = self.shortcuts.prompt_text();
        let current = browser
            .active_location()
            .and_then(|location| location.native_path().map(std::path::Path::to_path_buf));
        let home = gtk::glib::home_dir();
        let listing = |include_hidden| {
            browser
                .active_depth()
                .map(|depth| browser.folder_names(depth, include_hidden))
                .unwrap_or_default()
        };
        let context = Context {
            current: current.as_deref(),
            home: &home,
            show_hidden: browser.preferences().show_hidden,
            listing: &listing,
        };
        let sink = self.shortcuts.prompt_sink(kind);
        let later = self.shortcuts.prompt_sink(kind);
        let step = self.go.step(&text, backward, &context, move |step| {
            show_step(&later, step)
        });
        show_step(&sink, step);
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

    /// Empty **Enter** closes the prompt; a destination that is not an
    /// existing local folder keeps it open with the reason.
    fn submit_destination(&self, browser: &Browser, kind: Prompt, text: &str) {
        let mut targets = self.destination_targets.borrow().clone();
        if text.trim().is_empty() || targets.is_empty() {
            return self.return_to_listing(browser);
        }
        let result = match kind {
            Prompt::ExtractTo => self.view.extract_to_typed(targets.remove(0), text),
            _ => self
                .view
                .transfer_to_typed(targets, text, kind == Prompt::MoveTo),
        };
        match result {
            Ok(()) => self.return_to_listing(browser),
            Err(reason) => self.shortcuts.prompt_sink(kind).show(None, Some(reason)),
        }
    }

    fn return_to_listing(&self, browser: &Browser) {
        self.shortcuts.dismiss_prompt();
        if !self.view.focus_visible_results() {
            browser.focus_active();
        }
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
    shortcuts.show_candidates(paths);
    show_candidate_hint(shortcuts);
}

fn show_candidate_hint(shortcuts: &ShortcutFooter) {
    let Some(kind) = shortcuts.open_prompt_kind() else {
        return;
    };
    let hint = match shortcuts.candidate_position() {
        None => Some("No matching folders".to_owned()),
        Some((_, 1)) => None,
        Some((index, count)) => Some(format!("{} of {count}", index + 1)),
    };
    shortcuts.prompt_sink(kind).show(None, hint.as_deref());
}

fn show_step(sink: &PromptSink, step: Step) {
    match step {
        Step::Complete { text, index, count } => {
            let position = (count > 1).then(|| format!("{} of {count}", index + 1));
            sink.show(Some(&text), position.as_deref());
        }
        Step::Pending => sink.show(None, Some("Listing folders\u{2026}")),
        Step::Hint(hint) => sink.show(None, Some(hint.text())),
    }
}
