// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use strata_components::{
    collection::element::CollectionState,
    contract::{EntryId, EntrySource},
};
use strata_core::model::FileEntry;
use strata_fs::app::Browser;
use strata_qa::{Observation, PaneObservation};

use crate::{
    App,
    handover::{Presented, Shown},
    listing::{Column, Load},
    shell,
};

/// What failed, one line each.
#[derive(Default)]
struct Failures(Vec<String>);

impl Failures {
    fn fail(&mut self, invariant: &str, what: String) {
        self.0.push(format!("{invariant}: {what}"));
    }
}

mod keyboard;

pub(super) fn check(app: &App, observation: &Observation, focused: bool) -> Vec<String> {
    let mut failures = Failures::default();
    let shown = keyboard::Shown {
        observation,
        focused,
    };
    keyboard::check(app, &shown, &mut failures);
    hidden_views(app, &mut failures);
    hidden_tabs(app, &mut failures);
    let Some(view) = shell::shown_view(app) else {
        failures.fail("browser-view", "the window shows no view".to_owned());
        return failures.0;
    };
    let presented = (view.probe.presented)(app);
    let pending = presented.pending.iter();
    for depth in pending.filter(|depth| !presented.results.contains(depth)) {
        failures.fail(
            "handover",
            format!(
                "column {depth}'s selection changed in the view, and the browser has not taken it"
            ),
        );
    }
    let columns = Browsed::all(&app.browser);
    for column in &columns {
        column.holds_itself(&mut failures);
    }
    held_columns(app, &presented, &columns, &mut failures);
    let marked = presented.shown.iter();
    for shown in marked.filter(|shown| !presented.results.contains(&shown.depth)) {
        if let Some(browsed) = columns.get(shown.depth) {
            let kept = presented
                .kept
                .iter()
                .find_map(|(depth, id)| (*depth == shown.depth).then_some(*id));
            marks(shown, browsed, kept, &mut failures);
        }
    }
    tree(&presented, &observation.panes, &mut failures);
    failures.0
}

/// A hidden tab runs nothing: what was routed to it is held, or dropped for its rebuild.
fn hidden_tabs(app: &App, failures: &mut Failures) {
    for (key, accounted) in shell::hidden_tabs(app) {
        if !accounted {
            failures.fail(
                "hidden-tab",
                format!("tab {} ran an update while it was hidden", key.0),
            );
        }
    }
}

fn hidden_views(app: &App, failures: &mut Failures) {
    for view in shell::parts(app).hidden_views() {
        let presented = (view.probe.presented)(app);
        for depth in &presented.pending {
            failures.fail(
                "handover",
                format!(
                    "the hidden {} view changed column {depth}'s selection",
                    view.probe.name
                ),
            );
        }
    }
}

/// One of the browser's columns, read once.
struct Browsed {
    depth: usize,
    entries: Vec<FileEntry>,
    show_hidden: bool,
    loading: bool,
    /// Sorted.
    selected: Vec<usize>,
    cursor: Option<usize>,
    anchor: Option<usize>,
    unfilled: bool,
}

impl Browsed {
    fn all(browser: &Browser) -> Vec<Self> {
        (0..)
            .map_while(|depth| Self::read(browser, depth))
            .collect()
    }

    fn read(browser: &Browser, depth: usize) -> Option<Self> {
        let snapshot = browser.column_snapshot(depth)?;
        let entries = browser.with_column_entries(depth, <[FileEntry]>::to_vec)?;
        let cursor = browser.cursor_entry(depth).and_then(|cursor| {
            entries
                .iter()
                .position(|entry| entry.location == cursor.location)
        });
        let mut selected = snapshot.selected_positions;
        selected.sort_unstable();
        Some(Self {
            depth,
            show_hidden: browser
                .column_preferences(depth)
                .is_some_and(|preferences| preferences.show_hidden),
            loading: snapshot.loading,
            selected,
            cursor,
            anchor: browser.selection_anchor_position(depth),
            unfilled: browser.filled_entries(depth).is_empty(),
            entries,
        })
    }

    fn name(&self, position: usize) -> String {
        self.entries.get(position).map_or_else(
            || format!("position {position} of {}", self.entries.len()),
            |entry| entry.display_name.clone(),
        )
    }

    fn names(&self, positions: &[usize]) -> Vec<String> {
        positions
            .iter()
            .map(|position| self.name(*position))
            .collect()
    }

    /// The positions the view lists: every entry that is not hidden while hidden files are.
    fn visible(&self) -> Vec<usize> {
        let shows = |entry: &FileEntry| self.show_hidden || !entry.is_hidden;
        let entries = self.entries.iter().enumerate();
        entries
            .filter(|(_, entry)| shows(entry))
            .map(|(position, _)| position)
            .collect()
    }

    fn holds_itself(&self, failures: &mut Failures) {
        let depth = self.depth;
        let mut listed = HashSet::new();
        for entry in &self.entries {
            if !listed.insert(&entry.location) {
                let name = &entry.display_name;
                failures.fail("browser", format!("column {depth} lists {name} twice"));
            }
        }
        let len = self.entries.len();
        let marked = self.selected.iter().chain(&self.anchor);
        for position in marked.filter(|position| **position >= len) {
            failures.fail(
                "browser",
                format!("column {depth} marks position {position} of its {len} entries"),
            );
        }
    }
}

/// The view's listings against the browser's columns, and its active column.
fn held_columns(
    app: &App,
    presented: &Presented<'_>,
    columns: &[Browsed],
    failures: &mut Failures,
) {
    let (held, open) = (presented.listings.len(), columns.len());
    if held != open {
        failures.fail(
            "browser-view",
            format!("the browser has {open} columns, the view keeps listings for {held}"),
        );
    }
    if let Some(strip) = presented.strip.filter(|strip| *strip != open) {
        failures.fail(
            "browser-view",
            format!("the browser has {open} columns, the view draws {strip}"),
        );
    }
    let active = app.browser.active_depth();
    if presented.active != active {
        failures.fail(
            "browser-view",
            format!(
                "the browser's active column is {active:?}, the view's {:?}",
                presented.active
            ),
        );
    }
    for (listing, browsed) in presented.listings.iter().zip(columns) {
        listing_of(app, listing, browsed, failures);
    }
}

/// One listing against the browser's column of the same depth.
fn listing_of(app: &App, listing: &Column, browsed: &Browsed, failures: &mut Failures) {
    let depth = browsed.depth;
    if app.browser.location_at(depth).as_ref() != Some(listing.location()) {
        failures.fail(
            "browser-view",
            format!("column {depth} lists another folder than the browser's"),
        );
        return;
    }
    let loading = *listing.load() == Load::Loading;
    if loading != browsed.loading {
        failures.fail(
            "browser-view",
            format!(
                "column {depth} is loading in the {} and listed in the {}",
                side(loading),
                side(!loading)
            ),
        );
    }
    rows_of(app, listing, browsed, failures);
}

/// The side that says something, of the view and the browser.
fn side(view: bool) -> &'static str {
    if view { "view" } else { "browser" }
}

/// The rows a listing shows against the browser's entries.
fn rows_of(app: &App, listing: &Column, browsed: &Browsed, failures: &mut Failures) {
    let depth = browsed.depth;
    let mut seen = HashSet::new();
    let mut positions = Vec::new();
    for index in 0..listing.len() {
        let id = listing.id_at(index);
        let name = row_name(listing, id);
        if !seen.insert(id) {
            failures.fail("browser-view", format!("column {depth} shows {name} twice"));
        }
        let Some(position) = listing.position_of(id) else {
            failures.fail(
                "browser-view",
                format!("column {depth} shows {name}, which has no position in its listing"),
            );
            continue;
        };
        positions.push(position);
        let entry = browsed.entries.get(position);
        let ours = entry.and_then(|entry| shell::known_id(app, &entry.location));
        if ours != Some(id) {
            failures.fail(
                "browser-view",
                format!(
                    "column {depth} shows {name} where the browser lists {}",
                    browsed.name(position)
                ),
            );
        }
    }
    positions.sort_unstable();
    let visible = browsed.visible();
    // While it loads, the view lists what has been published of the browser's entries.
    if !browsed.loading && positions != visible {
        failures.fail(
            "browser-view",
            format!(
                "column {depth} shows {:?}, the browser lists {:?}",
                browsed.names(&positions),
                browsed.names(&visible)
            ),
        );
    }
}

fn row_name(listing: &Column, id: EntryId) -> String {
    listing.row_of(id).map_or_else(
        || "a row with no entry".to_owned(),
        |row| row.name().to_owned(),
    )
}

/// What one column of the view marks, in the browser's positions.
struct Marked {
    cursor: Option<usize>,
    selected: Vec<usize>,
    anchor: Option<usize>,
    provisional: bool,
}

impl Marked {
    fn of(shown: &Shown<'_>) -> Self {
        let (column, selection) = (shown.column, &shown.collection.selection);
        let position = |id: EntryId| column.position_of(id);
        let cursor = selection.cursor().and_then(position);
        let mut selected: Vec<usize> = selection
            .selected_ids(column)
            .into_iter()
            .filter_map(position)
            .collect();
        selected.sort_unstable();
        Self {
            cursor,
            selected,
            // The anchor a view without one would range from: its cursor, as `adopt` gives it.
            anchor: selection.anchor().and_then(position).or(cursor),
            provisional: selection.is_provisional(),
        }
    }

    /// What the browser's marks come to in a view, as `handover::adopt` writes them.
    fn adopted(browsed: &Browsed) -> Self {
        let cursor = browsed.cursor;
        Self {
            cursor,
            selected: browsed.selected.clone(),
            anchor: browsed.anchor.or(cursor),
            provisional: browsed.unfilled
                && cursor.is_some_and(|cursor| browsed.selected == [cursor]),
        }
    }
}

/// A column shown against the browser's cursor, selection, anchor and provisional flag.
fn marks(shown: &Shown<'_>, browsed: &Browsed, kept: Option<EntryId>, failures: &mut Failures) {
    let depth = shown.depth;
    let (view, mut browser) = (Marked::of(shown), Marked::adopted(browsed));
    let unplaced = browser.cursor.is_none() && browser.selected.is_empty();
    if unplaced && let Some(id) = kept.filter(|id| shown.collection.selection.cursor() == Some(*id))
    {
        browser.cursor = shown.column.position_of(id);
        browser.anchor = browsed.anchor.or(browser.cursor);
    }
    let one = |position: Option<usize>| position.map(|position| browsed.name(position));
    let mut differs = |mark: &str, view: String, browser: String| {
        failures.fail(
            "browser-view",
            format!("column {depth}'s {mark} is {view} in the view and {browser} in the browser"),
        );
    };
    if view.cursor != browser.cursor {
        differs(
            "cursor",
            format!("{:?}", one(view.cursor)),
            format!("{:?}", one(browser.cursor)),
        );
    }
    if view.selected != browser.selected {
        let names = |positions: &[usize]| format!("{:?}", browsed.names(positions));
        differs("selection", names(&view.selected), names(&browser.selected));
    }
    if view.anchor != browser.anchor {
        differs(
            "anchor",
            format!("{:?}", one(view.anchor)),
            format!("{:?}", one(browser.anchor)),
        );
    }
    if view.provisional != browser.provisional {
        differs(
            "load cursor",
            view.provisional.to_string(),
            browser.provisional.to_string(),
        );
    }
}

fn tree(presented: &Presented<'_>, panes: &[PaneObservation], failures: &mut Failures) {
    let mut columns = presented.shown.iter();
    for pane in panes {
        let name = &pane.name;
        let mut passed = Vec::new();
        let found = columns.find(|shown| {
            let here = shown.column.title() == name;
            if !here {
                passed.push(shown.depth);
            }
            here
        });
        for depth in passed {
            failures.fail(
                "tree-view",
                format!("column {depth} has no pane in the tree"),
            );
        }
        let Some(shown) = found else {
            failures.fail(
                "tree-view",
                format!("the tree has a pane {name} after the columns the view shows"),
            );
            return;
        };
        let drawn = presented
            .drawn
            .iter()
            .find(|drawn| drawn.depth == shown.depth);
        match drawn {
            Some(drawn) => pane_of(
                pane,
                shown.depth,
                (drawn.column, drawn.collection),
                failures,
            ),
            None => pane_of(
                pane,
                shown.depth,
                (shown.column, shown.collection),
                failures,
            ),
        }
    }
    for shown in columns {
        let depth = shown.depth;
        failures.fail(
            "tree-view",
            format!("column {depth} has no pane in the tree"),
        );
    }
}

fn pane_of(
    pane: &PaneObservation,
    depth: usize,
    (column, collection): (&Column, &CollectionState),
    failures: &mut Failures,
) {
    let selection = &collection.selection;
    let mut seen = HashSet::new();
    for entry in &pane.entries {
        let name = &entry.name;
        if !seen.insert(name) {
            failures.fail(
                "tree-view",
                format!("column {depth}'s pane has {name} twice"),
            );
        }
        let row = (0..column.len())
            .map(|index| column.id_at(index))
            .find(|id| column.row_of(*id).is_some_and(|row| row.name() == name));
        let Some(id) = row else {
            failures.fail(
                "tree-view",
                format!("column {depth}'s pane has {name}, the view no such row"),
            );
            continue;
        };
        let selected = selection.contains(column, id);
        if entry.selected != selected {
            failures.fail(
                "tree-view",
                format!(
                    "{name} in column {depth} is {} in the tree and {} in the view",
                    selected_word(entry.selected),
                    selected_word(selected)
                ),
            );
        }
    }
}

fn selected_word(selected: bool) -> &'static str {
    if selected { "selected" } else { "not selected" }
}
