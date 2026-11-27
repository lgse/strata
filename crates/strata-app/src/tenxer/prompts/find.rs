// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_components::{
    collection::selection::{AnchorRule, Selection},
    contract::EntrySource,
};
use strata_core::i18n::tr;

use crate::{App, icons::Icons, list::List, shell, single::SinglePane};

#[derive(Default)]
pub(crate) struct FindState {
    query: String,
    backward: bool,
    highlighted: bool,
}

impl FindState {
    pub(crate) fn clear(&mut self) {
        self.query.clear();
        self.backward = false;
        self.highlighted = false;
    }

    pub(crate) fn query(&self) -> Option<&str> {
        (!self.query.is_empty()).then_some(self.query.as_str())
    }

    /// The query whose matches are highlighted now (`find_highlight`).
    pub(crate) fn highlight(&self) -> Option<&str> {
        self.query().filter(|_| self.highlighted)
    }
}

fn contains_case_insensitive(name: &str, query: &str) -> bool {
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return false;
    }
    let hay: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    hay.windows(needle.len())
        .any(|win| win == needle.as_slice())
}

fn next_match(
    source: &dyn EntrySource,
    selection: &Selection,
    query: &str,
    backward: bool,
) -> Option<usize> {
    let len = source.len();
    if len == 0 {
        return None;
    }
    let current = selection.cursor_index(source);
    (1..=len)
        .map(|step| match (current, backward) {
            (Some(idx), false) => (idx + step) % len,
            (Some(idx), true) => (idx + len - step % len) % len,
            (None, false) => step - 1,
            (None, true) => len - step,
        })
        .find(|&idx| contains_case_insensitive(source.entry(idx).name, query))
}

fn place_cursor(app: &mut App, index: usize, tx: &mut Tx) {
    if app.filter.pane.shows() {
        let Some(view) = shell::view_focus(app) else {
            return;
        };
        {
            let (source, collection) = app.filter.pane.parts_mut();
            let _ = collection
                .selection
                .move_cursor(source, index, AnchorRule::Keep);
            collection.note_selection(source);
        }
        app.filter.pane.reveal_index(view, index, tx);
        return;
    }
    if super::super::shows(app, "columns") {
        let depth = app.columns.active_depth();
        move_columns_cursor(app, depth, index, tx);
    } else if super::super::shows(app, "icons") {
        move_single_cursor::<Icons>(app, index, tx);
    } else if super::super::shows(app, "list") {
        move_single_cursor::<List>(app, index, tx);
    }
}

fn move_columns_cursor(app: &mut App, depth: usize, index: usize, tx: &mut Tx) {
    app.columns.move_cursor_to(depth, index, tx);
}

fn move_single_cursor<V: SinglePane>(app: &mut App, index: usize, tx: &mut Tx) {
    let (source, collection) = V::single_mut(app).pane.parts_mut();
    let _ = collection
        .selection
        .move_cursor(source, index, AnchorRule::Keep);
    collection.note_selection(source);
    collection.reveal(V::KEY, source, index, tx);
}

fn search_shown(app: &mut App, query: &str, backward: bool, tx: &mut Tx) -> bool {
    let found = if app.filter.pane.shows() {
        app.filter
            .pane
            .shown_with_selection()
            .and_then(|(source, sel)| next_match(source, sel, query, backward))
    } else if super::super::shows(app, "columns") {
        let depth = app.columns.active_depth();
        app.columns
            .shown_with_selection(depth)
            .and_then(|(source, sel)| next_match(source, sel, query, backward))
    } else if super::super::shows(app, "icons") {
        let pane = &Icons::single(app).pane;
        next_match(pane.source(), &pane.collection().selection, query, backward)
    } else if super::super::shows(app, "list") {
        let pane = &List::single(app).pane;
        next_match(pane.source(), &pane.collection().selection, query, backward)
    } else {
        None
    };
    let Some(index) = found else {
        return false;
    };
    place_cursor(app, index, tx);
    true
}

pub(super) fn submit(app: &mut App, query: &str, backward: bool, tx: &mut Tx) -> bool {
    if query.is_empty() {
        return true;
    }
    let find = &mut app.tenxer.prompt_state.find;
    find.query = query.to_owned();
    find.backward = backward;
    find.highlighted = true;
    search_shown(app, query, backward, tx)
}

pub(crate) fn repeat(app: &mut App, reverse: bool, tx: &mut Tx) {
    let Some(query) = app.tenxer.prompt_state.find.query().map(str::to_owned) else {
        app.tenxer.flash.show(tr("No previous find"), tx);
        return;
    };
    app.tenxer.prompt_state.find.highlighted = true;
    let backward = app.tenxer.prompt_state.find.backward != reverse;
    if !search_shown(app, &query, backward, tx) {
        report_miss(app, &query, tx);
    }
}

pub(crate) fn dismiss_highlight(app: &mut App) -> bool {
    std::mem::replace(&mut app.tenxer.prompt_state.find.highlighted, false)
}

pub(super) fn report_miss(app: &mut App, query: &str, tx: &mut Tx) {
    let msg = rust_i18n::t!("No matches for \u{201c}%{query}\u{201d}", query = query).into_owned();
    app.tenxer.flash.show(msg, tx);
}
