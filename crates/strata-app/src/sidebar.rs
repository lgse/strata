// SPDX-License-Identifier: MIT

use std::io;

use rune::{El, Tx};
use strata_components::context_menu::ContextMenuState;
use strata_components::contract::dnd::DropHover;
use strata_components::sidebar::{
    DeviceLabels, DropSite, ItemId, PinVerdict, PlacesSidebar, Section, SidebarData, SidebarIntent,
    SidebarLabels, SidebarState, Step, Via,
};
use strata_core::{
    i18n::{count, tr},
    model::Location,
};
use strata_fs::{
    adapters::bookmarks::{self, BookmarkWatch},
    app::{Browser, BrowserEvent},
};
use strata_kit::look::Look;

use crate::{
    App,
    commands::{self, Action, Holds, On},
    shell,
    slots::{self, Frame, Later, Listener, Part, Place, Setup},
};

mod devices;
mod drops;
mod header;
mod keys;
mod menu;
mod places;
mod reorder;
mod trash;
mod width;

pub(crate) use drops::spring_open;
pub(crate) use width::toggle_face;

use menu::OPEN_IN_TAB;
use places::{Environment, Listed, Navigation};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PinState {
    Available,
    Pinned,
    Unavailable,
}

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
    /// The element Up left for the header's toggle, which Down from the header comes back to.
    header_entry: Option<String>,
    /// A row's context menu ([`menu`]).
    menu: ContextMenuState,
    /// What the Trash holds, for its menu ([`trash`]).
    trash: trash::Probe,
    /// Validated folder names for the latest foreign drag over `PINNED` gaps.
    foreign_pins: drops::ForeignPinCache,
    /// DEVICES ([`devices`]).
    devices: devices::Devices,
    started: bool,
}

impl Part for Sidebar {
    const PLACE: Place<App> = Place::Sidebar(slots::Sidebar {
        build,
        focus: |app| {
            if app.chooser.is_confined() {
                return None;
            }
            app.sidebar.focus_key()
        },
        held_back: |app| {
            if app.chooser.is_confined() {
                return 0.0;
            }
            app.sidebar.held_back(&app.look)
        },
    });

    const COMMANDS: &'static [Action<App>] = &[
        width::TOGGLE,
        keys::TO_LEFT,
        keys::TO_DOWN,
        keys::TO_UP,
        keys::TO_RIGHT,
        header::TO_LEFT,
        header::TO_DOWN,
        header::TO_UP,
        header::TO_RIGHT,
    ];

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            state: SidebarState::default(),
            listed: Listed::default(),
            pins: Vec::new(),
            current: None,
            later: setup.later.clone(),
            watch: None,
            header_entry: None,
            menu: ContextMenuState::default(),
            trash: trash::Probe::default(),
            foreign_pins: None,
            devices: devices::Devices::default(),
            started: false,
        }
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.sidebar.menu.is_open().then_some(Holds::Menu)
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            if changes_active_place(event) {
                heard.then(|app: &mut App, _: &mut Tx| app.sidebar.follow(&app.browser));
            }
            if trash::changes_contents(event) {
                heard.then(|app: &mut App, _: &mut Tx| trash::refresh(app));
            }
        }))
    }

    fn customized(app: &mut App) {
        relist_if_started(app);
    }

    fn tab_hidden(app: &mut App, _tx: &mut Tx) {
        app.sidebar.state.scroll_bar.release();
    }

    fn tab_shown(app: &mut App, _tx: &mut Tx) {
        if !app.sidebar.started {
            start(app);
            return;
        }
        let saved = shell::preferences(app);
        let step = saved_step(saved.sidebar_expanded, saved.sidebar_compact);
        let collapsed = (
            saved.sidebar_pinned_collapsed,
            saved.sidebar_devices_collapsed,
        );
        let state = &mut app.sidebar.state;
        if state.width.chosen() != step {
            state.width.choose(step);
            state.scroll_bar.release();
        }
        (state.pinned_collapsed, state.devices_collapsed) = collapsed;
        relist(app);
    }

    fn poll(app: &mut App, _clock: f64, _tx: &mut Tx) -> bool {
        if app.sidebar.started {
            return false;
        }
        start(app);
        true
    }
}

/// Rebuilds the active tab's sidebar rows when sidebar settings changed while it is shown.
pub(crate) fn relist_if_started(app: &mut App) {
    if app.sidebar.started {
        relist(app);
    }
}

fn changes_active_place(event: &BrowserEvent) -> bool {
    matches!(
        event,
        BrowserEvent::Reset
            | BrowserEvent::ColumnAdded { .. }
            | BrowserEvent::ColumnsTruncated { .. }
            | BrowserEvent::ColumnsRelocated { .. }
            | BrowserEvent::FocusChanged { .. }
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

fn start(app: &mut App) {
    let saved = shell::preferences(app);
    let mut state = SidebarState::new(&app.look);
    state
        .width
        .choose(saved_step(saved.sidebar_expanded, saved.sidebar_compact));
    state.pinned_collapsed = saved.sidebar_pinned_collapsed;
    state.devices_collapsed = saved.sidebar_devices_collapsed;
    let sidebar = &mut app.sidebar;
    state.pin_check = sidebar.state.pin_check.clone();
    sidebar.state = state;
    sidebar.started = true;
    let later = sidebar.later.clone();
    sidebar.watch = Some(bookmarks::watch_changes(move |pins| {
        heard_pins(&later, pins);
        true
    }));
    sidebar.pins = bookmarks::current();
    relist(app);
    devices::start(app);
    trash::start(app);
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
                true
            });
        }
        Err(error) => tracing::warn!(%error, "the pinned folders could not be read"),
    }
}

fn relist(app: &mut App) {
    let listed = places::listed(
        shell::preferences(app),
        &app.sidebar.pins,
        &Environment::current().local_only(app.browser.is_chooser_mode()),
    );
    app.sidebar.listed = listed;
    // A drag held across the change would carry a row that may no longer be where it was.
    app.sidebar.state.places_reorder.cancel();
    app.sidebar.state.pins_reorder.cancel();
    // A device's label is a setting too.
    devices::redress(app);
}

impl Sidebar {
    fn focus_key(&self) -> Option<String> {
        if self.state.width.shown() == Step::Hidden {
            return None;
        }
        let key = self
            .current
            .as_ref()
            .map_or_else(|| LIST.to_owned(), |item| item.key(KEY));
        Some(key)
    }

    fn held_back(&self, look: &Look) -> f32 {
        if self.state.width.shown() != Step::Rail {
            return 0.0;
        }
        let rail = strata_kit::slots::sidebar::rail_width(look);
        (self.state.width.full_width() - rail).max(0.0)
    }

    pub(crate) fn is_started(&self) -> bool {
        self.started
    }

    pub(crate) fn pin_state(&self, location: &Location) -> PinState {
        if Environment::current().is_standard(location) {
            return PinState::Unavailable;
        }
        let pins = if self.started {
            std::borrow::Cow::Borrowed(self.pins.as_slice())
        } else {
            std::borrow::Cow::Owned(bookmarks::current())
        };
        if pins.iter().any(|(pinned, _)| pinned == location) {
            PinState::Pinned
        } else {
            PinState::Available
        }
    }

    pub(crate) fn chosen_full(&self) -> bool {
        self.state.width.chosen() == Step::Full
    }

    pub(crate) fn is_auto_railed(&self) -> bool {
        self.state.width.auto_rail()
    }

    /// Applies the preview split's fit rule to a full sidebar, returning whether it changed.
    pub(crate) fn set_auto_rail(&mut self, rail: bool) -> bool {
        let before = self.state.width.auto_rail();
        self.state.width.set_auto_rail(rail);
        let changed = self.state.width.auto_rail() != before;
        if changed {
            self.state.scroll_bar.release();
        }
        changed
    }

    pub(crate) fn visible_pins(&self) -> Vec<Location> {
        if self.started {
            return self
                .listed
                .rows
                .iter()
                .filter(|row| matches!(row.item, ItemId::Pin(_)))
                .map(|row| row.location.clone())
                .collect();
        }
        let env = Environment::current();
        bookmarks::current()
            .into_iter()
            .map(|(location, _)| location)
            .filter(|location| !env.is_standard(location))
            .collect()
    }

    pub(crate) fn drop_over(&self) -> Option<&(DropSite, DropHover)> {
        self.state.drop_over()
    }

    pub(crate) fn item_location(&self, item: &ItemId) -> Option<&Location> {
        self.location_of(item).map(|row| &row.location)
    }

    pub(crate) fn set_local_pin_verdict(&mut self, verdict: PinVerdict) {
        self.state.pin_check.local = verdict;
    }

    pub(crate) fn end_external_drag(&mut self) {
        self.state.pin_check.local = PinVerdict::Unchecked;
        self.state.end_drag();
    }

    /// The row's location, mounted devices' included.
    fn location_of(&self, item: &ItemId) -> Option<&places::Row> {
        self.rows().find(|row| &row.item == item)
    }

    /// Every row that opens a location, in the order they are drawn.
    fn rows(&self) -> impl Iterator<Item = &places::Row> {
        self.listed.rows.iter().chain(self.devices.rows())
    }

    fn follow(&mut self, browser: &Browser) -> bool {
        let active = browser.active_location();
        let at = |row: &places::Row| active.as_ref() == Some(&row.location);
        let kept = self
            .current
            .as_ref()
            .and_then(|item| self.location_of(item))
            .filter(|row| at(row));
        let current = kept
            .or_else(|| self.rows().find(|row| at(row)))
            .map(|row| row.item.clone());
        if current == self.current {
            return false;
        }
        self.current = current;
        true
    }
}

#[cfg(test)]
impl Sidebar {
    pub(crate) fn places(&self) -> &[strata_components::sidebar::Place] {
        &self.listed.places
    }

    pub(crate) fn first_place_location(&self) -> Location {
        self.listed.rows[0].location.clone()
    }
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let sidebar = &app.sidebar;
    if !sidebar.started || app.chooser.is_confined() {
        sidebar.state.scroll_bar.release();
        return None;
    }
    let (pinned, devices) = (tr("PINNED"), tr("DEVICES"));
    let [eject, unmount, unlock, lock, releasing] = [
        "Eject",
        "Unmount",
        "Unlock",
        "Lock",
        "Writing data to the drive. Do not unplug it.",
    ]
    .map(tr);
    let data = SidebarData {
        places: &sidebar.listed.places,
        pins: &sidebar.listed.pins,
        devices: sidebar.devices.shown(),
        current: sidebar.current.as_ref(),
        labels: SidebarLabels {
            pinned: &pinned,
            devices: &devices,
            rows: |rows| count("items", rows),
            device: DeviceLabels {
                eject: &eject,
                unmount: &unmount,
                unlock: &unlock,
                lock: &lock,
                releasing: &releasing,
            },
        },
    };
    let menu = menu::element(app, frame.layer);
    Some(element(app, &app.look, data).child(menu))
}

fn element(app: &App, look: &Look, data: SidebarData<'_>) -> El {
    PlacesSidebar::new(
        &app.sidebar.state,
        |app: &mut App| &mut app.sidebar.state,
        data,
    )
    .key(KEY)
    .dnd(crate::dnd::binder(app))
    .content_types(crate::dnd::CONTENT_TYPES)
    .drop_action(drops::drop_action)
    .reorder(true)
    .confined_grip(app.browser.is_chooser_mode())
    // A middle click opens a row in a new tab once a part registers the command.
    .open_in_new_tab(!app.browser.is_chooser_mode() && commands::has(app, OPEN_IN_TAB.id))
    .tenxer(shell::preferences(app).tenxer_mode)
    .on_background_drag(crate::pointer::leading_origin_drag)
    .on_intent(heard)
    .build(look)
}

/// What the sidebar asked for.
fn heard(app: &mut App, intent: SidebarIntent, tx: &mut Tx) {
    match intent {
        SidebarIntent::Open { item, via } => open(app, &item, via, tx),
        SidebarIntent::OpenInNewTab { item } => {
            if let Some(row) = app.sidebar.location_of(&item) {
                let place = On::Place(row.location.clone());
                commands::run(app, OPEN_IN_TAB.id, place, tx);
            }
        }
        SidebarIntent::ContextMenu {
            item: Some(item),
            point,
        } => menu::open(app, &item, point, tx),
        SidebarIntent::Unlock(id) => devices::unlock(app, &id),
        SidebarIntent::Lock(id) => devices::lock(app, &id),
        SidebarIntent::Eject(id) => devices::release(app, &id, tx),
        SidebarIntent::LeaveToFiles => {
            if let Some(view) = shell::view_focus(app) {
                tx.focus(view, true);
            }
        }
        SidebarIntent::MovePlace { from, landing, .. } => reorder::move_place(app, from, landing),
        SidebarIntent::MovePin { from, landing, .. } => reorder::move_pin(app, from, landing),
        SidebarIntent::DropInto { item, dropped } => drops::drop_into(app, &item, dropped, tx),
        SidebarIntent::PinHover {
            serial,
            uris,
            local,
        } => drops::pin_hover(app, serial, uris, local),
        SidebarIntent::PinDrop { gap, dropped } => drops::pin_drop(app, gap, dropped, tx),
        SidebarIntent::RowDragStart { item } => drops::row_drag_start(app, &item),
        SidebarIntent::RowDragMove { landing, at, .. } => {
            drops::row_drag_move(app, landing, at, tx)
        }
        SidebarIntent::RowDragEnd {
            dropped_outside, ..
        } => drops::row_drag_end(app, dropped_outside, tx),
        SidebarIntent::SectionToggled { section, collapsed } => {
            shell::saved(app).change(|settings| match section {
                Section::Pinned => settings.sidebar_pinned_collapsed = collapsed,
                Section::Devices => settings.sidebar_devices_collapsed = collapsed,
            });
        }
        _ => {}
    }
}

pub(crate) fn show_keycaps(app: &mut App, shown: bool) {
    if shown && app.sidebar.started {
        relist(app);
    }
    app.sidebar.state.keycaps = shown;
}

pub(crate) fn sync_pins_from_store(app: &mut App) {
    if !app.sidebar.started {
        return;
    }
    let pins = bookmarks::current();
    if app.sidebar.pins != pins {
        app.sidebar.pins = pins;
        relist(app);
    }
}

/// Opens a row's location, with its row marked before the folder opens.
fn open(app: &mut App, item: &ItemId, via: Via, tx: &mut Tx) {
    let Some(row) = app.sidebar.location_of(item).cloned() else {
        // An unmounted volume: it opens once mounted.
        if let ItemId::Device(id) = item {
            devices::mount(app, id);
        }
        return;
    };
    let tenxer = shell::preferences(app).tenxer_mode;
    let before = app.browser.active_location();
    app.sidebar.current = Some(row.item);
    let select_first = via == Via::Keyboard && !tenxer;
    shell::publish(app);
    let browser = app.browser.clone();
    match row.navigation {
        Navigation::Direct => browser.navigate_with_selection(row.location, select_first),
        Navigation::Validate => browser.navigate_location(row.location, select_first),
    }
    shell::settle(app, tx);
    let moved = app.browser.active_location() != before;
    if returns_focus_to_view(tenxer, via, moved)
        && let Some(view) = shell::view_focus(app)
    {
        tx.focus(view, true);
    }
}

fn returns_focus_to_view(tenxer: bool, via: Via, moved: bool) -> bool {
    tenxer && via == Via::Keyboard && moved
}

#[cfg(test)]
mod tests;
