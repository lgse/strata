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
//!   in place, as GTK's Icons and List views open it (`activate_in_place`), and Left at the
//!   view's start leaves it for the sidebar. Where the start is, is the component's: the
//!   first tile in Icons, any row in List (`at_left_edge` in `src/ui/browser_modes/focus.rs`).
//!   A secondary click or the Menu key opens the context menu through `menus::open`.
//! - **What the window asks of a view:** [`rows`] for the context menu, [`editing`] for the
//!   rename editor, [`publish`], and for the QA backend [`listed`] and [`lists`].
//! - **What both build:** the base [`collection`] a view's component adds its parts to, and
//!   the [`group`] around it, named by the folder and described as the view.

use rune::{El, Tx, col, text};
use strata_components::{
    collection::{
        element::{Activation, Collection, CollectionState, Describe, Intent},
        keyboard::{KeySettings, Keyboard},
        marquee::Marquee,
        pointer::{ClickCount, ClickModes, PointerIntent, PointerSettings},
    },
    contract::{Direction, EntryId, EntrySource},
    rename::{By, Initial, Polled},
};
use strata_core::i18n::tr;

use self::first_visit::FirstVisit;
use crate::{
    App, commands, events,
    listing::{Column, Load, Words},
    menus::{self, Request},
    pane::{Applied, Pane},
    rename, shell,
    slots::{At, Editing, Listener, Rows, Setup},
    surface::Surface,
};

/// A single-pane view's state that is not its component's.
pub(crate) struct Single {
    pub(crate) pane: Pane,
    clicks: ClickModes,
    /// The keys the component reads other than by its default.
    keys: KeySettings,
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
}

fn clicks(count: u8) -> ClickCount {
    match count {
        1 => ClickCount::One,
        _ => ClickCount::Two,
    }
}

/// The collection's controllers: the pointer's clicks, the keyboard and the marquee.
fn controlled() -> CollectionState {
    let mut collection = CollectionState::new();
    collection.controllers.pointer = Some(Box::new(PointerIntent::new()));
    collection.controllers.keyboard = Some(Box::new(Keyboard::default()));
    collection.controllers.marquee = Some(Box::new(Marquee::new()));
    collection
}

impl Single {
    /// The state of the view keyed `key`, opening a file and a folder with `(file, folder)`
    /// clicks, reading `keys`, and described as `described`, its translated `view_name`.
    pub(crate) fn new(
        key: &'static str,
        (file, folder): (u8, u8),
        keys: KeySettings,
        described: String,
    ) -> Self {
        Self {
            pane: Pane::new(key, controlled()),
            clicks: ClickModes::new(clicks(file), clicks(folder)),
            keys,
            words: Words::translated(),
            described,
            entries: tr("Files"),
            surface: Surface::translated(),
            first_visit: FirstVisit::default(),
        }
    }
}

/// A view that shows one pane: its key, and where its [`Single`] is in the [`App`].
pub(crate) trait SinglePane: 'static {
    /// The view's collection's key, the element that takes the keyboard, and the start of
    /// every element key the view makes.
    const KEY: &'static str;

    fn single(app: &App) -> &Single;

    fn single_mut(app: &mut App) -> &mut Single;

    /// The view's state and the rename editor's, borrowed together.
    fn renaming(app: &mut App) -> (&Single, &mut rename::Rename);
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
fn status<V: SinglePane>(app: &App) -> Option<(&str, bool)> {
    let single = V::single(app);
    single.surface.status(0, single.pane.listing()?)
}

/// What the entry list is described as: what it is, or, while it shows no entry, what the
/// GTK pane surface that holds the keyboard then is described as (`describe_pane_surface` in
/// `src/ui/accessibility.rs`).
fn described<V: SinglePane>(app: &App) -> String {
    match status::<V>(app) {
        Some((status, _)) => status.to_owned(),
        None => V::single(app).entries.clone(),
    }
}

fn shown_status<V: SinglePane>(app: &App) -> Option<El> {
    let (message, _) = status::<V>(app).filter(|(_, shown)| *shown)?;
    let look = &app.look;
    Some(
        col().w_full().px(look.space.lg).py(look.space.lg).child(
            text(message.to_owned())
                .font(look.kit.ty.ui)
                .color(look.kit.c.text_muted),
        ),
    )
}

fn source<V: SinglePane>(app: &App) -> (&dyn EntrySource, &CollectionState) {
    let pane = pane::<V>(app);
    (pane.source(), pane.collection())
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
    .settings(single.keys.clone())
    .settings(PointerSettings::default().modes(single.clicks))
    .describe(Describe::pure(|app: &App, row| {
        let single = V::single(app);
        let description = single
            .pane
            .listing()?
            .description(row.index, &single.words)?;
        Some(description.to_owned())
    }))
    .empty(shown_status::<V>)
    .on_press(|app: &mut App, _, _| interrupted::<V>(app))
    .on_wheel(|app: &mut App, _, _| {
        interrupted::<V>(app);
        false
    })
    .on_intent(heard::<V>)
    .on_cursor(|app: &mut App, _, tx| moved(app, tx))
    .on_selection(moved);
    // The editor answers in this view only while its edit names it.
    collection.rename(
        |app: &App| app.rename.state.renaming_in(V::KEY),
        rename::editor(app).slot(),
    )
}

/// The view's element: a group keyed `<key>-pane`, named by the folder and described as
/// the view, around the component's.
pub(crate) fn group<V: SinglePane>(app: &App, component: El) -> El {
    col()
        .key(format!("{}-pane", V::KEY))
        .role("group")
        .label(title::<V>(app))
        .description(V::single(app).described.clone())
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(component)
}

/// One poll of what the view says while it shows no entry, a read announced once it outlasts
/// GTK's grace period, and of a first visit whose load ended. True when the view has to run.
pub(crate) fn poll<V: SinglePane>(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let Single {
        pane,
        surface,
        first_visit,
        ..
    } = V::single_mut(app);
    let placed = first_visit.settle(pane);
    surface.watch(std::iter::once(pane.listing()), clock, tx) | placed
}

/// The user's input reached the view's pane: a first visit leaves the cursor to them.
fn interrupted<V: SinglePane>(app: &mut App) {
    V::single_mut(app).first_visit.interrupt();
}

/// What reads the view's share of each event out of the browser.
pub(crate) fn listen<V: SinglePane>(setup: &Setup<'_>) -> Option<Listener<App>> {
    let ids = setup.ids.clone();
    Some(Box::new(move |event, browser, heard| {
        let read = events::read(event, browser, &ids);
        heard.copied(read.copied);
        for change in read.changes {
            heard.then(move |app: &mut App, tx: &mut Tx| {
                let single = V::single_mut(app);
                if single.pane.apply(change, tx) == Applied::Entered {
                    // The GTK window gives a new listing the keyboard.
                    single.first_visit.arm();
                    tx.focus(V::KEY, false);
                }
                true
            });
        }
    }))
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
fn open_in_place<V: SinglePane>(app: &App, depth: usize, id: EntryId, how: Activation) {
    let Some(position) = pane::<V>(app).position(depth, id) else {
        return;
    };
    let focused = app
        .browser
        .focused_item()
        .is_some_and(|(at, focused, _)| (at, focused) == (depth, position));
    match how {
        Activation::Key if focused => app.browser.activate_focused_in_place(),
        _ => app.browser.activate_in_place(depth, position),
    }
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
        Intent::Activate { id, how } => open_in_place::<V>(app, depth, id, how),
        // A slow second click on the name.
        Intent::BeginRename { id } => rename::begin_on(app, depth, id, tx),
        // The opener ends an open rename itself, as a press outside the field does in GTK.
        Intent::ContextMenu { target, anchor } => {
            let request = Request {
                depth,
                entry: target,
                anchor,
            };
            menus::open(app, request, tx);
        }
        // Left at the view's start: the sidebar takes the keyboard, when there is one. The
        // component decides where its start is, from the view's keys (the first tile in Icons,
        // any row in List); only that Left reaches here.
        Intent::Edge(Direction::Left) => {
            shell::leave_view_at_start(app, tx);
        }
        // Drags, new tabs and the pane header an edge moves to are other packages' parts.
        _ => {}
    }
    shell::settle(app, tx);
}

/// The collection moved its cursor or changed its selection: the browser takes it now, so
/// the frame drawn for the input shows the browser in step.
fn moved(app: &mut App, tx: &mut Tx) {
    shell::settle(app, tx);
}

/// What the context menu and the rename editor ask of the view's rows.
pub(crate) const fn rows<V: SinglePane>() -> Rows<App> {
    Rows {
        position: |app, (depth, id)| pane::<V>(app).position(depth, id),
        neighbour: |app, (depth, id), step| pane::<V>(app).neighbour(depth, id, step),
        // What the context menu's Open asks (`open_context_entry` in
        // `src/ui/browser/context_menu/item_menu.rs`): what a click on the row does.
        activate: |app, (depth, id)| open_in_place::<V>(app, depth, id, Activation::Click),
        // The keyboard is the editor's that opens next (Tab's `advance` and the menu's Rename
        // both open one on the row): focusing the list here would take it from the field, as
        // the Columns view does not either.
        select_only: |app, (depth, id), _| V::single_mut(app).pane.select_only(depth, id),
        // Select All in a single pane is the browser's (`select_all` in
        // `src/ui/browser/view_state.rs`): the view adopts it.
        select_all: |app, depth| app.browser.select_all(depth),
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

mod first_visit;

#[cfg(test)]
mod tests;
