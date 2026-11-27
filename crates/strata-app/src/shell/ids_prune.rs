// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use strata_components::contract::EntryId;

use crate::App;

const PRUNE_THRESHOLD: usize = 131_072;

pub(super) fn prune_if_needed(app: &App) {
    if !can_prune(app) {
        return;
    }
    prune_now(app);
}

fn prune_now(app: &App) {
    let mut keep = HashSet::new();
    collect_live_ids(app, &mut keep);
    let dropped = app.window.ids.retain_ids(&keep);
    if dropped > 0 {
        tracing::debug!(
            dropped,
            retained = keep.len(),
            "pruned unreferenced entry ids"
        );
    }
}

fn can_prune(app: &App) -> bool {
    let over_threshold = app.window.ids.len() > PRUNE_THRESHOLD;
    let idle = app.window.events.is_empty() && !app.window.tabs.has_held_updates();
    over_threshold && idle && !crate::dnd::is_dragging(app)
}

fn collect_live_ids(app: &App, keep: &mut HashSet<EntryId>) {
    // Active tab views, filter hits, peek, and restore memory.
    app.columns.collect_ids(keep);
    app.icons.single.pane.collect_ids(keep);
    app.list.single.pane.collect_ids(keep);
    app.filter.pane.collect_ids(keep);
    app.peek.collect_ids(keep);
    app.window.here.restore.collect_ids(keep);
    // Parked tabs.
    app.window.tabs.collect_parked_ids(keep);
    // Window-wide state holding EntryIds.
    app.window.worn.collect_ids(keep);
    app.clipboard.collect_ids(keep);
}

#[cfg(test)]
mod tests;
