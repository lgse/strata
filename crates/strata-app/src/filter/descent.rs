// SPDX-License-Identifier: MIT

//! A `/` typed into the filter's field over a query: the first match's folder opens, and the
//! filter stays open there, empty (`connect_path_separator` in
//! `src/ui/browser/listing_filter.rs`, `finish_filter_descent` in
//! `src/ui/window/keyboard/prompts.rs`).
//!
//! GTK takes the `/` before the field does. Here the field has taken it by the time its text
//! changes, so the part tells the `/` from the edit ([`slash_over`]) and puts the query back.
//! Only a lone `/` counts, typed or pasted, over the caret or a selection: no file name holds
//! one, and longer text holding it, such as a pasted path, is typed.

use rune::Tx;

use super::{OpenIn, results, route};
use crate::{App, shell};

/// The query a lone `/` was typed over, when `after` is `before` with a `/` in the place of
/// its caret or its selection. `None` for any other edit, and over an empty query.
pub(super) fn slash_over<'a>(before: &'a str, after: &str) -> Option<&'a str> {
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

/// Enters the first match's folder for the `/` waiting, once the results for its query show:
/// the Enter target, as GTK's `settled_filter_match`. A file there, or no match, ends it.
pub(super) fn finish(app: &mut App, tx: &mut Tx) {
    let Some(query) = app.filter.descent.clone() else {
        return;
    };
    let settled = app.filter.debounce.borrow().is_none() && !app.filter.session.awaiting();
    let shown = app.filter.session.is_active() && app.filter.state.text().trim() == query.trim();
    if !(settled && shown) {
        return;
    }
    app.filter.descent = None;
    let Some(id) = app.filter.state.target(results(app)) else {
        return;
    };
    let Some((folder, true)) = app.filter.found.get(&id).cloned() else {
        return;
    };
    app.filter.state.edit.replace_text("");
    let polled = app.filter.state.text_edited(Default::default());
    route(app, polled, tx);
    app.filter.typed.clear();
    shell::publish(app);
    app.filter.serial += 1;
    app.filter.open_in = Some(OpenIn {
        depth: 0,
        folder: folder.clone(),
    });
    app.columns.mirror_no_landing();
    app.browser.navigate(folder);
    app.columns.hold_header(Some(0));
    app.filter.refocus = super::Refocus::Waiting;
    app.filter.state.focus_field();
    shell::settle(app, tx);
}
