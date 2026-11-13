// SPDX-License-Identifier: MIT

//! The sidebar: the built-in places and the pinned folders, in strata-components' C8
//! `PlacesSidebar`, as the GTK window's sidebar lists and opens them (`build_sidebar` and
//! `bind_place_row` in `src/ui/window/sidebar.rs`). [`places`] says which rows show.
//!
//! - **Opening a row.** A click or Return opens its location; from the keyboard the first
//!   item is selected, as GTK's `connect_activate` asks. Home, Network and the pins are
//!   checked before they open (`navigate_location`), the other places open at once
//!   (`navigate_with_selection`).
//! - **The open place.** The row of the browser's active location carries the accent. A
//!   clicked row is marked before its folder opens, as GTK's `select_sidebar_row` does, and
//!   of two rows for one location the marked one stays marked (`sync_active_place`).
//! - **The keyboard.** Left at the start of the view comes here (`shell::leave_view_at_start`),
//!   onto the open place's row, else the first (`focus_active_place`). In 10xer mode the
//!   component's `h`, Left and Backspace give it back to the view; the default map's arrows
//!   out of the sidebar (GTK's `sidebar_navigation`) are not here yet.
//! - **The pins** come from GTK's bookmarks file, through the store both interfaces share
//!   (`strata_fs::adapters::bookmarks`), read when the sidebar starts and again whenever the
//!   store hears that the file changed.
//! - **The width and the sections** start as the settings saved them; a section's heading
//!   saves its own state.
//!
//! The width toggle, reordering, drops, devices, the full Trash's icon, the `g` keycaps and
//! the context menus are not here yet: `docs/rune/parity/packages/A3.md` lists them.

use std::io;

use rune::{El, Tx};
use strata_components::sidebar::{
    ItemId, PlacesSidebar, Section, SidebarData, SidebarIntent, SidebarState, Step, Via,
};
use strata_core::model::Location;
use strata_fs::{
    adapters::bookmarks::{self, BookmarkWatch},
    app::{Browser, BrowserEvent},
};
use strata_kit::look::Look;

use crate::{
    App, shell,
    slots::{self, Frame, Later, Listener, Part, Place, Setup},
};

mod places;

use places::{Environment, Listed, Navigation};

/// The key every element of the sidebar's starts with.
const KEY: &str = "sidebar";

/// The sidebar's list: what takes the keyboard from the view while no row is open.
const LIST: &str = "sidebar-list";

pub(crate) struct Sidebar {
    state: SidebarState,
    listed: Listed,
    /// The pins as the store last gave them.
    pins: Vec<(Location, String)>,
    /// The row that carries the accent: the active location's.
    current: Option<ItemId>,
    later: Later<App>,
    /// Held while the sidebar shows: the store calls only the watches still held.
    watch: Option<BookmarkWatch>,
    started: bool,
}

impl Part for Sidebar {
    const PLACE: Place<App> = Place::Sidebar(slots::Sidebar {
        build,
        focus: |app| app.sidebar.focus_key(),
    });

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            state: SidebarState::default(),
            listed: Listed::default(),
            pins: Vec::new(),
            current: None,
            later: setup.later.clone(),
            watch: None,
            started: false,
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            if changes_active_place(event) {
                heard.then(|app: &mut App, _: &mut Tx| app.sidebar.follow(&app.browser));
            }
        }))
    }

    fn poll(app: &mut App, _clock: f64, _tx: &mut Tx) -> bool {
        if app.sidebar.started {
            return false;
        }
        start(app);
        true
    }
}

/// `event_changes_active_place` in `src/ui/window/sidebar_places.rs`.
fn changes_active_place(event: &BrowserEvent) -> bool {
    matches!(
        event,
        BrowserEvent::Reset
            | BrowserEvent::ColumnAdded { .. }
            | BrowserEvent::ColumnsTruncated { .. }
            | BrowserEvent::ColumnsRelocated { .. }
            | BrowserEvent::FocusChanged { .. }
            // A clicked place is marked before its folder opens; a refusal leaves the
            // previous location open.
            | BrowserEvent::LocationNavigationRejected { .. }
    )
}

/// The step the saved settings start the sidebar on.
fn saved_step(expanded: bool, compact: bool) -> Step {
    match (expanded, compact) {
        (false, _) => Step::Hidden,
        (true, true) => Step::Rail,
        (true, false) => Step::Full,
    }
}

/// The sidebar's first poll: its state from the look and the settings, the pins from the
/// store, and the watch that hears the store again.
fn start(app: &mut App) {
    let saved = shell::preferences(app);
    let mut state = SidebarState::new(&app.look);
    state
        .width
        .choose(saved_step(saved.sidebar_expanded, saved.sidebar_compact));
    state.pinned_collapsed = saved.sidebar_pinned_collapsed;
    state.devices_collapsed = saved.sidebar_devices_collapsed;
    let sidebar = &mut app.sidebar;
    sidebar.state = state;
    sidebar.started = true;
    let later = sidebar.later.clone();
    sidebar.watch = Some(bookmarks::watch_changes(move |pins| {
        heard_pins(&later, pins);
        true
    }));
    sidebar.pins = bookmarks::current();
    relist(app);
    app.sidebar.follow(&app.browser);
}

/// The store read the bookmarks file again, or could not.
fn heard_pins(later: &Later<App>, pins: Result<&Vec<(Location, String)>, &io::Error>) {
    match pins {
        Ok(pins) => {
            let pins = pins.clone();
            later.then(move |app: &mut App, _: &mut Tx| {
                if app.sidebar.pins == pins {
                    return false;
                }
                app.sidebar.pins = pins;
                relist(app);
                app.sidebar.follow(&app.browser);
                true
            });
        }
        Err(error) => tracing::warn!(%error, "the pinned folders could not be read"),
    }
}

/// Lists the rows again from the settings and the pins.
fn relist(app: &mut App) {
    let listed = places::listed(
        shell::preferences(app),
        &app.sidebar.pins,
        &Environment::current(),
    );
    app.sidebar.listed = listed;
}

impl Sidebar {
    /// Where the keyboard comes in from the view: the open place's row, else the list's first
    /// row (`focus_active_place` in `src/ui/window/sidebar_places.rs`).
    fn focus_key(&self) -> String {
        self.current
            .as_ref()
            .map_or_else(|| LIST.to_owned(), |item| item.key(KEY))
    }

    fn location_of(&self, item: &ItemId) -> Option<&places::Row> {
        self.listed.rows.iter().find(|row| &row.item == item)
    }

    /// Marks the active location's row: the marked row while it is still that location's,
    /// else the first row for it. True when the mark moved.
    fn follow(&mut self, browser: &Browser) -> bool {
        let active = browser.active_location();
        let at = |row: &places::Row| active.as_ref() == Some(&row.location);
        let kept = self
            .current
            .as_ref()
            .and_then(|item| self.location_of(item))
            .filter(|row| at(row));
        let current = kept
            .or_else(|| self.listed.rows.iter().find(|row| at(row)))
            .map(|row| row.item.clone());
        if current == self.current {
            return false;
        }
        self.current = current;
        true
    }
}

fn build(app: &App, _frame: &mut Frame<'_, App>) -> Option<El> {
    let sidebar = &app.sidebar;
    if !sidebar.started {
        return None;
    }
    let data = SidebarData {
        places: &sidebar.listed.places,
        pins: &sidebar.listed.pins,
        devices: &[],
        current: sidebar.current.as_ref(),
    };
    Some(element(app, &app.look, data))
}

fn element(app: &App, look: &Look, data: SidebarData<'_>) -> El {
    PlacesSidebar::new(
        &app.sidebar.state,
        |app: &mut App| &mut app.sidebar.state,
        data,
    )
    .key(KEY)
    // Tabs are not on this interface yet.
    .open_in_new_tab(false)
    .tenxer(shell::preferences(app).tenxer_mode)
    .on_intent(heard)
    .build(look)
}

/// What the sidebar asked for.
fn heard(app: &mut App, intent: SidebarIntent, tx: &mut Tx) {
    match intent {
        SidebarIntent::Open { item, via } => open(app, &item, via, tx),
        SidebarIntent::LeaveToFiles => {
            if let Some(view) = shell::view_focus(app) {
                tx.focus(view, true);
            }
        }
        SidebarIntent::SectionToggled { section, collapsed } => {
            shell::saved(app).change(|settings| match section {
                Section::Pinned => settings.sidebar_pinned_collapsed = collapsed,
                Section::Devices => settings.sidebar_devices_collapsed = collapsed,
            });
        }
        _ => {}
    }
}

/// Opens a row's location, with its row marked before the folder opens.
fn open(app: &mut App, item: &ItemId, via: Via, tx: &mut Tx) {
    let Some(row) = app.sidebar.location_of(item).cloned() else {
        return;
    };
    app.sidebar.current = Some(row.item);
    let select_first = via == Via::Keyboard;
    shell::publish(app);
    let browser = app.browser.clone();
    match row.navigation {
        Navigation::Direct => browser.navigate_with_selection(row.location, select_first),
        Navigation::Validate => browser.navigate_location(row.location, select_first),
    }
    shell::settle(app, tx);
}

#[cfg(test)]
mod tests;
