// SPDX-License-Identifier: MIT

use std::ops::Range;

use rune::Tx;
use strata_components::{
    collection::keyboard::{Command as Nav, Target},
    contract::{Direction, EntryKind},
    filter_bar::prompt::{By, FooterPrompt, PromptIntent},
    rename::{Initial, initial_selection},
};
use strata_core::{i18n::tr, model::FileEntry};

use crate::{App, clipboard, filter, new_item, rename, shell};

pub(crate) mod find;
pub(crate) mod folder;
mod names;
pub(crate) mod spec;
#[cfg(test)]
mod tests;

pub(crate) use spec::PromptKind;

use super::PROMPT_KEY;

#[derive(Clone, Debug)]
struct FilterDescent {
    navigation: u64,
    query: String,
    at_cursor: bool,
}

#[derive(Default)]
pub(crate) struct State {
    pub(super) rename_target: Option<(usize, FileEntry)>,
    pub(super) destination_targets: Vec<FileEntry>,
    pub(crate) completing: folder::Completing,
    pub(crate) find: find::FindState,
    filter_typed: String,
    filter_cursor_moved: bool,
    filter_descent: Option<FilterDescent>,
}

impl State {
    pub(crate) fn reset_all(&mut self) {
        self.rename_target = None;
        self.destination_targets.clear();
        self.completing.cancel();
        self.find.clear();
        self.filter_typed.clear();
        self.filter_cursor_moved = false;
        self.filter_descent = None;
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Asked {
    pub(super) kind: PromptKind,
    pub(super) serial: u64,
}

impl Asked {
    pub(super) fn holds(self, app: &App) -> bool {
        app.tenxer.prompt.kind() == Some(self.kind) && app.tenxer.prompt.serial() == self.serial
    }
}

pub(crate) fn prompt_builder(app: &App) -> FooterPrompt<App, PromptKind> {
    let builder = FooterPrompt::new(
        PROMPT_KEY,
        |app: &App| &app.tenxer.prompt,
        |app: &mut App| &mut app.tenxer.prompt,
    )
    .labels(spec::prompt_labels())
    .on_intent(on_intent);
    match shell::view_focus(app) {
        Some(focus) => builder.focus_return(focus),
        None => builder,
    }
}

/// What a prompt opens with: its text, and the part of it selected.
struct Prefill<'a> {
    text: &'a str,
    select: Option<Range<usize>>,
}

impl<'a> Prefill<'a> {
    const EMPTY: Prefill<'static> = Prefill {
        text: "",
        select: None,
    };

    fn text(text: &'a str) -> Self {
        Self { text, select: None }
    }
}

fn open_prompt(app: &mut App, kind: PromptKind, prefill: Prefill<'_>, tx: &mut Tx) {
    let Prefill { text, select } = prefill;
    super::cancel_chord(app);
    app.tenxer.prompt_state.filter_typed = if kind == PromptKind::Filter {
        text.to_owned()
    } else {
        String::new()
    };
    app.tenxer.prompt_state.filter_cursor_moved = false;
    app.tenxer.prompt_state.filter_descent = None;
    let polled = app.tenxer.prompt.open(kind, kind.spec(), text, select);
    let builder = prompt_builder(app);
    tx.focus_when_shown(builder.field_key(), false);
    builder.route(app, polled, tx);
}

pub(crate) fn open_find(app: &mut App, backward: bool, tx: &mut Tx) {
    let kind = if backward {
        PromptKind::FindBackward
    } else {
        PromptKind::Find
    };
    open_prompt(app, kind, Prefill::EMPTY, tx);
}

pub(crate) fn open_history(app: &mut App, recent: bool, tx: &mut Tx) {
    let kind = if recent {
        PromptKind::Recent
    } else {
        PromptKind::Jump
    };
    open_prompt(app, kind, Prefill::EMPTY, tx);
}

pub(crate) fn open_filter(app: &mut App, tx: &mut Tx) {
    let query = filter::listing_filter_query(app).unwrap_or_default();
    open_prompt(app, PromptKind::Filter, Prefill::text(&query), tx);
}

pub(crate) fn open_search(app: &mut App, tx: &mut Tx) {
    match filter::begin_listing_search(app) {
        Some(query) => open_prompt(app, PromptKind::Search, Prefill::text(&query), tx),
        None => app.tenxer.flash.show(tr("Nothing to search"), tx),
    }
}

pub(crate) fn open_go(app: &mut App, tx: &mut Tx) {
    app.tenxer.prompt_state.destination_targets.clear();
    open_prompt(app, PromptKind::Go, Prefill::EMPTY, tx);
}

pub(crate) fn open_create(app: &mut App, tx: &mut Tx) {
    if new_item::focused_create_parent(app).is_some() {
        open_prompt(app, PromptKind::Create, Prefill::EMPTY, tx);
    } else {
        app.tenxer
            .flash
            .show(tr("Can\u{2019}t create items here"), tx);
    }
}

fn focused_target_with_depth(app: &mut App) -> Option<(usize, FileEntry)> {
    if filter::targets::selected(app).is_some() {
        return filter::targets::preview_result(app).or_else(|| filter::targets::under_cursor(app));
    }
    shell::publish(app);
    let depth = super::focused_listing_depth(app)?;
    let entry = app.browser.cursor_entry(depth)?;
    Some((depth, entry))
}

pub(crate) fn open_rename(app: &mut App, tx: &mut Tx) {
    let Some((depth, entry)) = focused_target_with_depth(app) else {
        app.tenxer.flash.show(tr("Nothing to rename"), tx);
        return;
    };
    if !rename::can_rename(&entry) {
        app.tenxer
            .flash
            .show(tr("Can\u{2019}t rename items here"), tx);
        return;
    }
    let name = entry.display_name.clone();
    let kind = if entry.is_directory() {
        EntryKind::Folder
    } else {
        EntryKind::File
    };
    let (start, end) = initial_selection(&name, kind, Initial::Stem);
    app.tenxer.prompt_state.rename_target = Some((depth, entry));
    let prefill = Prefill {
        text: &name,
        select: Some(start..end),
    };
    open_prompt(app, PromptKind::Rename, prefill, tx);
}

pub(crate) fn open_transfer(app: &mut App, moving: bool, tx: &mut Tx) {
    let entries = super::command_targets(app);
    if entries.is_empty() {
        let msg = if moving {
            "Nothing to move"
        } else {
            "Nothing to copy"
        };
        app.tenxer.flash.show(tr(msg), tx);
        return;
    }
    if moving
        && !entries
            .iter()
            .all(|entry| clipboard::paths::can_remove(&entry.location))
    {
        app.tenxer
            .flash
            .show(tr("Can\u{2019}t move these items"), tx);
        return;
    }
    app.tenxer.prompt_state.destination_targets = entries;
    let kind = if moving {
        PromptKind::MoveTo
    } else {
        PromptKind::CopyTo
    };
    open_prompt(app, kind, Prefill::EMPTY, tx);
}

pub(crate) fn open_extract_to(app: &mut App, entry: FileEntry, tx: &mut Tx) {
    app.tenxer.prompt_state.destination_targets = vec![entry];
    open_prompt(app, PromptKind::ExtractTo, Prefill::EMPTY, tx);
}

fn close_only(app: &mut App, tx: &mut Tx) {
    let polled = app.tenxer.prompt.close();
    prompt_builder(app).route(app, polled, tx);
}

pub(super) fn focus_listing_or_results(app: &App, tx: &mut Tx) {
    if !filter::focus_visible_results(app, tx)
        && let Some(view) = shell::view_focus(app)
    {
        tx.focus_when_shown(view, true);
    }
}

pub(super) fn close_and_return(app: &mut App, tx: &mut Tx) {
    app.tenxer.prompt_state.rename_target = None;
    app.tenxer.prompt_state.destination_targets.clear();
    close_only(app, tx);
    focus_listing_or_results(app, tx);
}

fn on_intent(app: &mut App, intent: PromptIntent<PromptKind>, tx: &mut Tx) {
    match intent {
        PromptIntent::Edited { kind, text, .. } => on_edited(app, kind, text, tx),
        PromptIntent::Complete { kind, query } => folder::complete(app, kind, query, tx),
        PromptIntent::CancelCompletion => app.tenxer.prompt_state.completing.cancel(),
        PromptIntent::Step { kind, delta } => on_step(app, kind, delta, tx),
        PromptIntent::Submit {
            kind,
            text,
            chosen,
            stepped,
            settled,
            serial,
        } => on_submit(
            app,
            folder::SubmitArgs {
                kind,
                text,
                chosen,
                stepped,
                settled,
                serial,
            },
            tx,
        ),
        PromptIntent::Dismissed { kind, text, by } => on_dismissed(app, kind, (text, by), tx),
        _ => {}
    }
}

fn slash_over<'a>(before: &'a str, after: &str) -> Option<&'a str> {
    if before.trim().is_empty() {
        return None;
    }
    let prefix = before
        .char_indices()
        .zip(after.chars())
        .find(|((_, kept), now)| kept != now)
        .map_or(before.len().min(after.len()), |((at, _), _)| at);
    let (kept, now) = (&before[prefix..], &after[prefix..]);
    let suffix: usize = kept
        .chars()
        .rev()
        .zip(now.chars().rev())
        .take_while(|(kept, now)| kept == now)
        .map(|(kept, _)| kept.len_utf8())
        .sum();
    let suffix = suffix.min(now.len().saturating_sub(1));
    (&now[..now.len() - suffix] == "/").then_some(before)
}

fn on_edited(app: &mut App, kind: PromptKind, text: String, tx: &mut Tx) {
    let was_moved = std::mem::replace(&mut app.tenxer.prompt_state.filter_cursor_moved, false);
    match kind {
        PromptKind::Filter => {
            let before = std::mem::replace(&mut app.tenxer.prompt_state.filter_typed, text.clone());
            if let Some(query) = slash_over(&before, &text).map(str::to_owned) {
                app.tenxer.prompt.edit.replace_text(before.clone());
                app.tenxer.prompt.edit.set_caret(before.len());
                app.tenxer.prompt_state.filter_typed = before;
                app.tenxer.prompt_state.filter_descent = Some(FilterDescent {
                    navigation: app.browser.navigation_generation(),
                    query,
                    at_cursor: was_moved,
                });
                finish_filter_descent(app, tx);
                return;
            }
            app.tenxer.prompt_state.filter_descent = None;
            filter::set_listing_filter(app, &text, tx);
        }
        PromptKind::Search => filter::set_listing_search(app, &text, tx),
        _ => {}
    }
}

pub(crate) fn finish_filter_descent(app: &mut App, tx: &mut Tx) {
    let Some(pending) = app.tenxer.prompt_state.filter_descent.clone() else {
        return;
    };
    let current = app.browser.navigation_generation() == pending.navigation
        && app.tenxer.prompt.kind() == Some(PromptKind::Filter)
        && app.tenxer.prompt.focused()
        && app.tenxer.prompt.text() == pending.query;
    if !current {
        app.tenxer.prompt_state.filter_descent = None;
        return;
    }
    let Some(settled) = filter::settled_descent_folder(app, pending.at_cursor) else {
        return;
    };
    app.tenxer.prompt_state.filter_descent = None;
    let Some(folder) = settled else {
        return;
    };
    close_only(app, tx);
    shell::publish(app);
    app.browser.navigate(folder);
    shell::settle(app, tx);
    open_prompt(app, PromptKind::Filter, Prefill::EMPTY, tx);
}

fn on_step(app: &mut App, kind: PromptKind, delta: i32, tx: &mut Tx) {
    let dir = if delta < 0 {
        Direction::Up
    } else {
        Direction::Down
    };
    let _ = super::run_collection_command(app, Nav::Cursor(Target::Step(dir)), tx);
    if kind == PromptKind::Filter {
        app.tenxer.prompt_state.filter_cursor_moved = true;
    }
}

fn on_submit(app: &mut App, args: folder::SubmitArgs, tx: &mut Tx) {
    match args.kind {
        PromptKind::Filter => {
            close_only(app, tx);
            filter::commit_listing_filter(app, &args.text, tx);
        }
        PromptKind::Search => {
            close_only(app, tx);
            filter::commit_listing_search(app, &args.text, tx);
        }
        PromptKind::Go | PromptKind::MoveTo | PromptKind::CopyTo | PromptKind::ExtractTo => {
            folder::submit_folder(app, args, tx);
        }
        PromptKind::Jump | PromptKind::Recent => {
            folder::submit_history(app, args.chosen, args.serial, tx);
        }
        PromptKind::Create => names::submit_create(app, args.serial, &args.text, tx),
        PromptKind::Rename => names::submit_rename(app, args.serial, &args.text, tx),
        PromptKind::Find | PromptKind::FindBackward => {
            let backward = args.kind == PromptKind::FindBackward;
            let found = find::submit(app, &args.text, backward, tx);
            close_and_return(app, tx);
            if !found {
                find::report_miss(app, &args.text, tx);
            }
        }
    }
}

fn on_dismissed(app: &mut App, kind: PromptKind, (text, by): (String, By), tx: &mut Tx) {
    app.tenxer.prompt_state.completing.cancel();
    app.tenxer.prompt_state.filter_descent = None;
    if kind == PromptKind::Rename {
        app.tenxer.prompt_state.rename_target = None;
    }
    if kind.picks_folder() {
        app.tenxer.prompt_state.destination_targets.clear();
    }
    if by != By::Escape {
        return;
    }
    match kind {
        PromptKind::Filter => {
            if !filter::clear_listing_filter(app, tx) {
                focus_listing_or_results(app, tx);
            }
        }
        PromptKind::Search => filter::commit_listing_search(app, &text, tx),
        PromptKind::Find | PromptKind::FindBackward => {
            find::dismiss_highlight(app);
            focus_listing_or_results(app, tx);
        }
        _ => focus_listing_or_results(app, tx),
    }
}
