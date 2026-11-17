// SPDX-License-Identifier: MIT

//! What the single-pane views, Icons and List, share beside their [`Pane`]: one
//! implementation of what both do with it, as `AGENTS.md` asks of the list and icons modes.
//! Shared code, owned by the contract ([`crate::slots`], "Who owns what").
//!
//! A view keeps a [`Single`] in its state and names it with [`SinglePane`]; what stays in
//! its own module is its component, its [`KeySettings`], its probe's name and roles, and its
//! words. Everything here takes the view by that trait:
//!
//! - **Events in.** [`listen`] queues what each event changed ([`crate::events`]), and the
//!   pane puts what is for its folder into its listing and its collection. A new listing
//!   takes the keyboard, as the GTK window gives it.
//! - **Intents out.** [`heard`] takes what the collection asks to the browser: a folder opens
//!   in place, as GTK's Icons and List views open it (`activate_in_place`), a middle click
//!   (`MiddleClick::clicked`) runs `open-in-tab` or `open-in-window` on a folder and reveals a
//!   file in its folder, and Left at the view's start leaves it for the sidebar. Where the start
//!   is, is the component's: the first tile in Icons, any row in List (`at_left_edge` in
//!   `src/ui/browser_modes/focus.rs`).
//!   A secondary click or the Menu key opens the context menu through `menus::open`.
//! - **What the window asks of a view:** [`rows`] for the context menu, [`editing`] for the
//!   rename editor, [`publish`], and for the QA backend [`listed`] and [`lists`].
//! - **What both build:** the base [`collection`] a view's component adds its parts to, and
//!   the [`group`] around it, named by the folder and described as the view.
//! - **A filter's results** (A10's [`crate::filter::pane`]) show in the component's place, in
//!   a collection of their own built on [`results`], under the filter's band in the group.
//!   Their rows are no listing's: the view's rows hooks ask the filter first for an entry of
//!   theirs, and the rename editor opens and follows its row in the results.

use std::{borrow::Cow, cell::Cell, rc::Rc};

use rune::{El, Mods, Tx, WheelEvent, col};
use strata_components::{
    collection::{
        element::{Activation, Collection, CollectionState, Describe, Intent},
        keyboard::{KeyHandle, KeySettings, Keyboard, Letters, NothingToSelect},
        marquee::Marquee,
        pointer::{ClickModes, PointerIntent, PointerSettings},
    },
    context_menu::AnchorEdge,
    contract::{
        Direction, EntryId, EntrySource,
        dnd::{DropAction, DropOffer, Dropped},
    },
    rename::{By, Initial, Polled},
    scroll::AutoscrollSettings,
};
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_desktop::settings::Preferences;

use self::first_visit::FirstVisit;
use crate::{
    App,
    commands::{self, On},
    events::{self, Change},
    filter,
    listing::{Column, Words},
    menus::{self, Request},
    pane::{Applied, Pane},
    pane_header, rename, shell,
    slots::{At, Editing, Listener, Rows, Setup, Worn},
    surface::{self, Surface},
};

/// A single-pane view's state that is not its component's.
pub(crate) struct Single {
    pub(crate) pane: Pane,
    pub(crate) keys_handle: KeyHandle,
    wrap_rows: bool,
    left_leaves: bool,
    words: Words,
    /// `view_name` in `src/ui/accessibility.rs`: what the pane is described as, and how the
    /// QA harness finds it.
    described: String,
    /// `ENTRY_CONTAINER_DESCRIPTION` there: what the list of entries is.
    entries: String,
    /// What the pane says while it shows no entry.
    surface: Surface,
    /// Another folder shows, until its load ends or the user's input reaches the pane.
    first_visit: FirstVisit,
    /// Display scale at the last build, for drag preview icons.
    scale: Cell<f32>,
}

/// The collection's controllers: the pointer's clicks, the keyboard and the marquee.
fn controlled(handle: KeyHandle) -> CollectionState {
    let mut collection = CollectionState::new();
    collection.controllers.pointer = Some(Box::new(PointerIntent::new()));
    collection.controllers.keyboard = Some(Box::new(Keyboard::new(handle)));
    collection.controllers.marquee = Some(Box::new(Marquee::new()));
    collection
}

/// The state of a filter's results in the pane (A10's [`crate::filter`]), with the controllers
/// and the keys the pane's listing has, the Menu key's menu where the view opens it: the
/// centre in List, the right edge in Icons.
pub(crate) fn results_state(list: bool, handle: KeyHandle) -> CollectionState {
    let mut collection = controlled(handle);
    let menu_at = match list {
        true => AnchorEdge::Center,
        false => AnchorEdge::Right,
    };
    commands::view_keys(&mut collection, menu_at);
    collection
}

/// What a single-pane view is made with.
pub(crate) struct Made {
    /// The view's collection's key ([`SinglePane::KEY`]).
    pub(crate) key: &'static str,
    pub(crate) wrap_rows: bool,
    pub(crate) left_leaves: bool,
    /// The view's translated `view_name`.
    pub(crate) described: String,
    /// Where on the cursor's entry the Menu key opens its menu (`item_menu_target` in
    /// `src/ui/browser_modes/context_menu.rs`): the centre in List, the right edge elsewhere.
    pub(crate) menu_at: AnchorEdge,
    pub(crate) worn: Rc<Worn>,
}

impl Single {
    pub(crate) fn new(made: Made) -> Self {
        let Made {
            key,
            wrap_rows,
            left_leaves,
            described,
            menu_at,
            worn,
        } = made;
        let keys_handle = KeyHandle::new();
        let mut collection = controlled(keys_handle.clone());
        commands::view_keys(&mut collection, menu_at);
        Self {
            pane: Pane::new(key, collection).with_worn(worn),
            keys_handle,
            wrap_rows,
            left_leaves,
            words: Words::translated(),
            described,
            entries: tr("Files"),
            surface: Surface::translated(),
            first_visit: FirstVisit::default(),
            scale: Cell::new(1.0),
        }
    }

    pub(crate) fn scale(&self) -> f32 {
        self.scale.get()
    }

    pub(crate) fn set_scale(&self, scale: f32) {
        self.scale.set(scale);
    }

    pub(crate) fn in_band(&self) -> bool {
        self.pane.collection().selection.in_band()
    }
}

pub(crate) fn sync_worn<V: SinglePane>(app: &mut App) {
    let worn = shell::worn(app).clone();
    V::single_mut(app).pane.set_worn(&worn);
}

/// A view that shows one pane: its key, and where its [`Single`] is in the [`App`].
pub(crate) trait SinglePane: 'static {
    /// The view's collection's key, the element that takes the keyboard, and the start of
    /// every element key the view makes.
    const KEY: &'static str;

    fn single(app: &App) -> &Single;

    fn single_mut(app: &mut App) -> &mut Single;

    fn fills_mut(app: &mut App) -> &mut crate::fills::Fills;

    /// The view's state and the rename editor's, borrowed together.
    fn renaming(app: &mut App) -> (&Single, &mut rename::Rename);

    /// The view's live click-activation modes, read from `settings`.
    fn clicks(settings: &Preferences) -> ClickModes;

    /// The view's layout, as C1e keys a remembered scroll offset: the mode and anything that
    /// changes wrapping. An offset comes back only in the layout it was measured in, as GTK's
    /// comes back only in the same mode, and in Icons at the same page width.
    fn layout_key(app: &App) -> u64;
}

fn pane<V: SinglePane>(app: &App) -> &Pane {
    &V::single(app).pane
}

/// The folder the view shows, as the pane and its list are named: `display_name`, as GTK's
/// `describe_pane` and `describe_entry_container` name them.
pub(crate) fn title<V: SinglePane>(app: &App) -> String {
    pane::<V>(app)
        .listing()
        .map(Column::title)
        .unwrap_or_default()
        .to_owned()
}

/// What the view says in place of entries while it shows none ([`status`]), and whether it
/// draws it.
fn status<V: SinglePane>(app: &App) -> Option<(Cow<'_, str>, bool)> {
    let single = V::single(app);
    single.surface.status(0, single.pane.listing()?)
}

/// What the entry list is described as: what it is, or, while it shows no entry, what the
/// GTK pane surface that holds the keyboard then is described as (`describe_pane_surface` in
/// `src/ui/accessibility.rs`).
fn described<V: SinglePane>(app: &App) -> String {
    match status::<V>(app) {
        Some((status, _)) => status.into_owned(),
        None => V::single(app).entries.clone(),
    }
}

fn shown_status<V: SinglePane>(app: &App) -> Option<El> {
    let (said, _) = status::<V>(app).filter(|(_, shown)| *shown)?;
    let single = V::single(app);
    let (listing, depth) = (single.pane.listing()?, single.pane.depth()?);
    let failed = surface::failed(listing, depth);
    Some(surface::drawn(app, &single.surface, &said, failed))
}

fn source<V: SinglePane>(app: &App) -> (&dyn EntrySource, &CollectionState) {
    let pane = pane::<V>(app);
    (pane.source(), pane.collection())
}

fn autoscroll_settings(app: &App) -> AutoscrollSettings {
    let mut settings = AutoscrollSettings::STRATA;
    settings.margin = strata_kit::slots::collection::row_inset(&app.look);
    settings
}

fn live_keys(app: &App, single: &Single) -> KeySettings {
    let settings = shell::preferences(app);
    let mut keys = if settings.tenxer_mode {
        KeySettings::tenxer()
    } else if settings.type_to_search {
        KeySettings::default()
    } else {
        KeySettings::default().letters(Letters::Arrows)
    };
    keys.wrap_rows = single.wrap_rows;
    keys.left_leaves_without_cursor = single.left_leaves;
    keys.multiple(app.chooser.allows_multiple())
}

fn live_pointer<V: SinglePane>(app: &App) -> PointerSettings {
    let chooser = app.browser.is_chooser_mode();
    let modes = crate::clicks::for_chooser(V::clicks(shell::preferences(app)), chooser);
    PointerSettings::default()
        .modes(modes)
        .rename(!chooser)
        .multiple(app.chooser.allows_multiple())
}

/// The collection under the view's component: what the host sets on it, before the
/// component adds its own parts.
pub(crate) fn collection<V: SinglePane>(app: &App) -> Collection<App> {
    let single = V::single(app);
    // The registry reads a key ahead of the view's own, and a first visit hears it first.
    let collection = commands::in_view_with(
        Collection::new(
            V::KEY,
            source::<V>,
            |app: &mut App| V::single_mut(app).pane.parts_mut(),
            |app: &App| &app.look,
            |app: &App| pane::<V>(app).version(),
        ),
        interrupted::<V>,
    )
    .name(title::<V>(app))
    .description(described::<V>(app))
    .settings(live_keys(app, single))
    .settings(live_pointer::<V>(app))
    .settings(autoscroll_settings(app))
    .describe(Describe::pure(|app: &App, row| {
        let single = V::single(app);
        let description = single
            .pane
            .listing()?
            .description(row.index, &single.words)?;
        Some(description.to_owned())
    }))
    .dnd(crate::dnd::binder(app))
    .content_types(crate::dnd::CONTENT_TYPES)
    .on_drag_phase(|app: &mut App, d, key, tx| crate::dnd::drag_phase(app, d, key, tx))
    .on_drop(dropped::<V>)
    .drop_action(drop_action::<V>)
    .marks(|app: &App, id| {
        let marks = crate::dnd::with_marks(app, id, crate::clipboard::marks(app, id));
        crate::peek::with_marks(app, V::KEY, id, marks)
    })
    .empty(shown_status::<V>)
    .on_press(|app: &mut App, _, _| {
        interrupted::<V>(app);
        crate::chooser::clicked_entry_in_chooser(app);
    })
    // C3 replaces this with its own, which asks the Icons view's `IconsGrid::on_wheel` first.
    .on_wheel(wheeled::<V>)
    .on_intent(heard::<V>)
    .on_cursor(|app: &mut App, _, tx| moved(app, tx))
    .on_selection(moved);
    // The editor answers in this view only while its edit names it.
    collection.rename(
        |app: &App| app.rename.state.renaming_in(V::KEY),
        rename::editor(app).slot(),
    )
}

/// The collection a filter's results show in, in the listing's place (GTK's "Search
/// results"): keyed as the view's, so its keyboard and commands are the view's, with the view's
/// keys and clicks. Its rows are the filter's ([`filter::pane`]).
pub(crate) fn results<V: SinglePane>(app: &App) -> Collection<App> {
    let single = V::single(app);
    commands::in_view(Collection::new(
        V::KEY,
        |app: &App| {
            let hits = &app.filter.pane;
            (hits.source(), hits.collection())
        },
        |app: &mut App| app.filter.pane.parts_mut(),
        |app: &App| &app.look,
        |app: &App| app.filter.pane.version(),
    ))
    .name(tr(filter::pane::SEARCH_RESULTS))
    .settings(live_keys(app, single))
    // GTK's results keep an anchor of their own, which neither a right-click nor a
    // Shift+press sets (findings lgse/strata#2375 and #2431).
    .settings(live_pointer::<V>(app).missing_anchor_stays(true))
    .settings(autoscroll_settings(app))
    .on_drag_phase(|app: &mut App, d, key, tx| crate::dnd::drag_phase(app, d, key, tx))
    .marks(|app: &App, id| {
        crate::dnd::with_marks(
            app,
            id,
            crate::clipboard::with_marks(app, id, filter::pane::marks(app, id)),
        )
    })
    .empty(filter::pane::status)
    .on_intent(filter::pane::heard)
    .on_cursor(|app: &mut App, _, _| filter::picked(app))
    .on_selection(|app: &mut App, _| filter::picked(app))
    .rename(
        |app: &App| app.rename.state.renaming_in(V::KEY),
        rename::editor(app).slot(),
    )
}

/// The view's element: a group keyed `<key>-pane`, named by the folder and described as
/// the view, around the pane's header ([`pane_header`]), with the view's own `actions` among
/// the header's, and the component's.
pub(crate) fn group<V: SinglePane>(app: &App, component: El, actions: Option<El>) -> El {
    let title = title::<V>(app);
    let tenxer = shell::preferences(app).tenxer_mode;
    let mut group = col()
        .key(format!("{}-pane", V::KEY))
        .role("group")
        .label(title.clone())
        .description(V::single(app).described.clone())
        .w_full()
        .grow(1.0)
        .min_h(0.0);
    if !tenxer {
        group = group
            .child(pane_header::bar(app, V::KEY, &title, actions))
            // The filter's field, under the header as GTK's revealer is.
            .children(filter::pane::band(app));
    }
    // What is left under the header: the component fills its parent's height.
    group.child(body(app).child(component))
}

/// The box under the header the component fills. A filter's results take the view's key, so
/// theirs is keyed apart: Rune then lays them out as new elements, and their collection hears
/// its viewport's size, which an element in the place of one of the same size never reports.
fn body(app: &App) -> El {
    let body = col().w_full().grow(1.0).min_h(0.0);
    match app.filter.pane.shows() {
        true => body.key("filter-results"),
        false => body,
    }
}

/// One poll of what the view says while it shows no entry, a read announced once it outlasts
/// GTK's grace period, and of a first visit whose load ended, whose cursor the window keeps
/// as the visit's and on the first row shown ([`first_visit::keep_on_first_row`]). True when
/// the view has to run.
pub(crate) fn poll<V: SinglePane>(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let memory = shell::restore_memory(app).clone();
    let Single {
        pane,
        surface,
        first_visit,
        ..
    } = V::single_mut(app);
    let placed = first_visit.settle(pane);
    if placed {
        memory.set_visit_cursor(pane.depth().zip(pane.collection().selection.cursor()));
    }
    let kept = first_visit::keep_on_first_row(pane, &memory);
    surface.watch(std::iter::once(pane.listing()), clock, tx) | placed | kept
}

/// The user's input reached the view's pane: a first visit leaves the cursor to them.
pub(crate) fn interrupted<V: SinglePane>(app: &mut App) {
    V::single_mut(app).first_visit.interrupt();
}

/// A wheel over the view's entries: it ends a first visit, and scrolls as ever.
pub(crate) fn wheeled<V: SinglePane>(app: &mut App, _: WheelEvent, _: &mut Tx) -> bool {
    interrupted::<V>(app);
    false
}

/// What reads the view's share of each event out of the browser.
pub(crate) fn listen<V: SinglePane>(setup: &Setup<'_>) -> Option<Listener<App>> {
    let ids = setup.ids.clone();
    Some(Box::new(move |event, browser, heard| {
        if matches!(event, strata_fs::app::BrowserEvent::FolderCountsInvalidated) {
            heard.then(|app: &mut App, _: &mut Tx| {
                V::fills_mut(app).invalidate();
                true
            });
        }
        let read = events::read(event, browser, &ids);
        heard.copied(read.copied);
        for change in read.changes {
            heard.then(move |app: &mut App, tx: &mut Tx| apply::<V>(app, change, tx));
        }
    }))
}

/// Applies one change the listener read, or a rebuild's.
pub(crate) fn apply<V: SinglePane>(app: &mut App, change: Change, tx: &mut Tx) -> bool {
    let layout = V::layout_key(app);
    let memory = shell::restore_memory(app).clone();
    let single = V::single_mut(app);
    single.pane.set_layout_key(layout);
    single.pane.share_memory(&memory);
    let applied = single.pane.apply(change, tx);
    // A folder back at its remembered position arms no first visit.
    single.first_visit.follow(applied, &single.pane);
    if matches!(applied, Applied::Entered | Applied::Returned) {
        // The GTK window gives a new listing the keyboard.
        tx.focus(V::KEY, false);
    }
    true
}

/// A modal layer opened over the view: the pane's pointer gesture ends.
pub(crate) fn end_gestures<V: SinglePane>(app: &mut App) {
    V::single_mut(app).pane.parts_mut().1.end_gestures();
}

/// Hands the browser what the view changed since it last agreed with it.
pub(crate) fn publish<V: SinglePane>(app: &mut App) {
    let browser = app.browser.clone();
    V::single_mut(app).pane.publish(&browser);
}

/// Opens entry `id` of column `depth` in place, as GTK's Icons and List views do: a folder
/// becomes the folder shown. Return opens the browser's focused item (`activate_focused` in
/// `src/ui/browser/view_navigation.rs`), a click the entry clicked (`activate_in_place` in
/// `src/ui/browser_modes/clicks.rs`).
fn open_in_place<V: SinglePane>(app: &mut App, (depth, id): At, how: Activation, tx: &mut Tx) {
    let Some(position) = pane::<V>(app).position(depth, id) else {
        return;
    };
    if crate::chooser::confirm_key_activation(app, (depth, position), how, tx) {
        return;
    }
    let focused = app
        .browser
        .focused_item()
        .is_some_and(|(at, focused, _)| (at, focused) == (depth, position));
    match how {
        Activation::Key if focused => app.browser.activate_focused_in_place(),
        _ => app.browser.activate_in_place(depth, position),
    }
}

/// A middle click on a folder row (`MiddleClick::clicked` in
/// `src/ui/browser_modes/clicks.rs`): `open-in-tab`, or `open-in-window` with Ctrl or Shift.
fn open_in_new<V: SinglePane>(app: &mut App, (depth, id): At, window: bool, tx: &mut Tx) {
    let Some(position) = pane::<V>(app).position(depth, id) else {
        return;
    };
    let Some(entry) = app.browser.entry_at(depth, position) else {
        return;
    };
    let command = if window {
        "open-in-window"
    } else {
        "open-in-tab"
    };
    commands::run(app, command, On::Place(entry.location), tx);
}

/// A middle click on a file row (`MiddleClick::clicked` in
/// `src/ui/browser_modes/clicks.rs`): empties the folder's filter and reveals the file in its
/// folder.
fn reveal<V: SinglePane>(app: &mut App, (depth, id): At, tx: &mut Tx) {
    let Some(position) = pane::<V>(app).position(depth, id) else {
        return;
    };
    let Some(entry) = app.browser.entry_at(depth, position) else {
        return;
    };
    let Some(folder) = entry.location.parent() else {
        return;
    };
    if let Some(open) = app.browser.open_depth(&folder) {
        filter::clear_for_reveal(app, open, tx);
    }
    shell::publish(app);
    app.browser.reveal_locations(folder, vec![entry.location]);
    tx.focus(V::KEY, false);
}

/// What the view's collection asks of its host, taken to `Browser`.
fn heard<V: SinglePane>(app: &mut App, intent: Intent, tx: &mut Tx) {
    // What the input did to the cursor and the selection goes first: the browser acts on
    // its own.
    shell::publish(app);
    let Some(depth) = pane::<V>(app).depth() else {
        return;
    };
    match intent {
        Intent::Activate { id, how } => open_in_place::<V>(app, (depth, id), how, tx),
        // A slow second click on the name.
        Intent::BeginRename { id } => rename::begin_on(app, depth, id, tx),
        Intent::Clicked { id } => clicked::<V>(app, depth, id),
        // The opener ends an open rename itself, as a press outside the field does in GTK.
        Intent::ContextMenu { target, anchor } => {
            let request = Request {
                depth,
                entry: target,
                anchor,
            };
            menus::open(app, request, tx);
        }
        Intent::OpenInNew { id, window } => open_in_new::<V>(app, (depth, id), window, tx),
        Intent::Reveal { id } => reveal::<V>(app, (depth, id), tx),
        // Left at the view's start: the sidebar takes the keyboard, when there is one. The
        // component decides where its start is, from the view's keys (the first tile in Icons,
        // any row in List); only that Left reaches here. 10xer's keep it in the view, as GTK's
        // native arrows there stop at the edge (`move_icon_cursor`).
        Intent::Edge(Direction::Left) => {
            if !shell::preferences(app).tenxer_mode {
                shell::leave_view_at_start(app, tx);
            }
        }
        Intent::Other(other) if other.downcast_ref::<NothingToSelect>().is_some() => {
            crate::tenxer::flash_nothing_to_select(app, tx);
        }
        Intent::BeginDrag { ids, .. } => {
            let entries: Vec<_> = ids
                .into_iter()
                .filter_map(|id| {
                    let position = pane::<V>(app).position(depth, id)?;
                    app.browser.entry_at(depth, position)
                })
                .collect();
            crate::dnd::begin_entries(app, entries, V::single(app).scale(), tx);
        }
        // The pane header an edge moves to is another package's part.
        _ => {}
    }
    shell::settle(app, tx);
}

fn drop_destination<V: SinglePane>(app: &App, target: Option<EntryId>) -> Option<Location> {
    let pane = pane::<V>(app);
    let depth = pane.depth()?;
    match target {
        Some(id) => {
            let position = pane.position(depth, id)?;
            app.browser
                .entry_at(depth, position)
                .filter(FileEntry::is_directory)
                .map(|entry| entry.location)
        }
        None => app.browser.location_at(depth),
    }
}

fn drop_action<V: SinglePane>(
    app: &App,
    target: Option<EntryId>,
    offer: &DropOffer,
    mods: Mods,
) -> Option<DropAction> {
    let dest = drop_destination::<V>(app, target)?;
    crate::dnd::drop_action(app, &dest, offer, mods)
}

fn dropped<V: SinglePane>(app: &mut App, target: Option<EntryId>, dropped: Dropped, tx: &mut Tx) {
    if let Some(dest) = drop_destination::<V>(app, target) {
        crate::dnd::commit_file_drop(app, dest, &dropped, tx);
    }
}

pub(crate) fn spring_open(app: &mut App, depth: usize, id: EntryId, tx: &mut Tx) {
    let view = shell::shown_view(app);
    let position = if view.is_some_and(|shown| (shown.shows)("icons")) {
        app.icons.single.pane.position(depth, id)
    } else if view.is_some_and(|shown| (shown.shows)("list")) {
        app.list.single.pane.position(depth, id)
    } else {
        None
    };
    let Some(entry) = position
        .and_then(|pos| app.browser.entry_at(depth, pos))
        .filter(FileEntry::is_directory)
    else {
        return;
    };
    shell::publish(app);
    app.browser.navigate_location(entry.location, false);
    shell::settle(app, tx);
}

/// A plain click on a file previews it when single-click previews are enabled and two clicks
/// open files (`PendingActivationKind::Standard { preview: true }`).
fn clicked<V: SinglePane>(app: &App, depth: usize, id: EntryId) {
    let settings = shell::preferences(app);
    let modes = crate::clicks::for_chooser(V::clicks(settings), app.browser.is_chooser_mode());
    if crate::clicks::one_click_files(modes) || !settings.single_click_previews {
        return;
    }
    let Some(position) = pane::<V>(app).position(depth, id) else {
        return;
    };
    let previews = app.browser.entry_at(depth, position).is_some_and(|entry| {
        !entry.is_directory() && crate::preview::entry_supports_quick_preview(&entry)
    });
    if previews {
        app.browser.preview(depth, position);
    }
}

/// The collection moved its cursor or changed its selection: the browser takes it now, so
/// the frame drawn for the input shows the browser in step.
pub(crate) fn moved(app: &mut App, tx: &mut Tx) {
    // The user's own move: the cursor is no first visit's any more.
    shell::restore_memory(app).set_visit_cursor(None);
    shell::settle(app, tx);
}

/// What the context menu and the rename editor ask of the view's rows.
pub(crate) const fn rows<V: SinglePane>() -> Rows<App> {
    Rows {
        position: |app, (depth, id)| pane::<V>(app).position(depth, id),
        neighbour: |app, (depth, id), step| pane::<V>(app).neighbour(depth, id, step),
        // What the context menu's Open asks (`open_context_entry` in
        // `src/ui/browser/context_menu/item_menu.rs`): what a click on the row does, or on a
        // filter's result.
        activate: |app, (depth, id)| match filter::targets::at(app, depth, id) {
            Some(_) => filter::open_hit(app, depth, id),
            None => {
                if let Some(position) = pane::<V>(app).position(depth, id) {
                    app.browser.activate_in_place(depth, position);
                }
            }
        },
        // The keyboard is the editor's that opens next (Tab's `advance` and the menu's Rename
        // both open one on the row): focusing the list here would take it from the field, as
        // the Columns view does not either.
        select_only: |app, (depth, id), tx| match filter::targets::at(app, depth, id) {
            Some(_) => filter::pane::select_only::<V>(app, id, tx),
            None => V::single_mut(app).pane.select_only(depth, id),
        },
        // Select All in a single pane is the browser's (`select_all` in
        // `src/ui/browser/view_state.rs`): the view adopts it. GTK's does so under a filter's
        // results too, where the listing it selects is not shown (finding lgse/strata#2422); Ctrl+A
        // in the results is their own key.
        select_all: |app, depth| app.browser.select_all(depth),
        reveal: reveal_row::<V>,
    }
}

/// Scrolls the row of entry `id` into view, when the pane shows column `depth`: a filter's
/// result's, while they show.
fn reveal_row<V: SinglePane>(app: &mut App, (depth, id): At, tx: &mut Tx) {
    if filter::targets::at(app, depth, id).is_some() {
        filter::pane::reveal::<V>(app, id, tx);
        return;
    }
    let pane = &V::single(app).pane;
    let Some(listing) = pane.listing().filter(|_| pane.depth() == Some(depth)) else {
        return;
    };
    if let Some(index) = listing.index_of(id) {
        pane.collection().reveal(V::KEY, listing, index, tx);
    }
}

/// What the rename editor asks of the view: no follower, since nothing follows the cursor.
pub(crate) const fn editing<V: SinglePane>() -> Editing<App> {
    Editing {
        begin: begin_rename::<V>,
        tick: tick_rename::<V>,
        hold: None,
        follow_now: None,
    }
}

/// Opens the editor on the entry `id` and shows it. `None` when the view does not show the
/// entry: a hidden one has nowhere to edit in.
fn begin_rename<V: SinglePane>(app: &mut App, (depth, id): At, tx: &mut Tx) -> Option<Polled> {
    // A filter's result renames in the results, which the listing under them does not draw.
    if app.filter.pane.shows() {
        let hits = &app.filter.pane;
        let index = hits.source().index_of(id)?;
        let entry = hits.source().entry(index);
        let polled = app
            .rename
            .state
            .begin(V::KEY, id, entry.name, entry.kind, Initial::Stem);
        hits.reveal_index(V::KEY, index, tx);
        return Some(polled);
    }
    let (single, rename) = V::renaming(app);
    let pane = &single.pane;
    let listing = pane.listing().filter(|_| pane.depth() == Some(depth))?;
    let index = listing.index_of(id)?;
    let entry = listing.entry(index);
    let polled = rename
        .state
        .begin(V::KEY, id, entry.name, entry.kind, Initial::Stem);
    pane.collection().reveal(V::KEY, listing, index, tx);
    Some(polled)
}

/// One poll of the editor against the view's listing and viewport. An edit whose folder has
/// gone ends, and keeps the name.
fn tick_rename<V: SinglePane>(app: &mut App) -> Polled {
    if app.filter.pane.shows() {
        let hits = &app.filter.pane;
        return app
            .rename
            .state
            .tick(V::KEY, hits.source(), hits.collection());
    }
    let (single, rename) = V::renaming(app);
    let pane = &single.pane;
    match pane.listing() {
        Some(listing) => rename.state.tick(V::KEY, listing, pane.collection()),
        None => rename.state.cancel(By::Listing),
    }
}

/// Whether the folder the launch opened has been listed, or could not be: the probe's.
#[cfg(feature = "qa")]
pub(crate) fn listed<V: SinglePane>(app: &App) -> bool {
    use crate::listing::Load;
    matches!(pane::<V>(app).load(), Some(Load::Listed | Load::Failed(_)))
}

/// The key of the view's one entry list, while it shows a folder: the probe's.
#[cfg(feature = "qa")]
pub(crate) fn lists<V: SinglePane>(app: &App) -> Vec<String> {
    match pane::<V>(app).listing() {
        Some(_) => vec![V::KEY.to_owned()],
        None => Vec::new(),
    }
}

/// What the view holds of the browser's columns, and the filter's results it draws in the
/// listing's place while they show: the probe's.
#[cfg(feature = "qa")]
pub(crate) fn presented<V: SinglePane>(app: &App) -> crate::handover::Presented<'_> {
    let pane = pane::<V>(app);
    let mut presented = pane.presented();
    let hits = &app.filter.pane;
    if let (Some(depth), Some(column)) = (pane.depth(), hits.hits()) {
        presented.drawn.push(crate::handover::Drawn {
            depth,
            column,
            collection: hits.collection(),
        });
    }
    presented
}

mod first_visit;

#[cfg(test)]
mod tests;
