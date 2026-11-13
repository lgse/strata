// SPDX-License-Identifier: MIT

//! The shell's slots: where each part of the interface goes in the window, and what the shell
//! gives it. A part is a module of this crate, registered in `lib.rs`. The shell builds no
//! part by name: it asks the registry who is in a slot.
//!
//! # The slots
//!
//! | [`Slot`] | Where the shell puts it | How many | The part gives, in its [`Place`] |
//! | --- | --- | --- | --- |
//! | `Tabs` | Across the window, above the header | One | Its element ([`Build`]) |
//! | `Location` | The header's flexible middle (`Header::center`) | One | Its element |
//! | `Sidebar` | Under the header, before the view | One | A [`Sidebar`]: its element, and the key that takes the keyboard when it leaves the view at its start |
//! | `View` | Under the header: the files | Any, and one is shown | A [`View`]: its element, the saved mode it answers to, the key that takes the keyboard back, what it does with the pointer, and what the parts that work in a view ask of the one that is shown ([`Rows`], [`Editing`], [`Probe`]) |
//! | `Drawer` | Around the view | One | A [`Wrap`]: it is handed the view's element and returns the split |
//! | `Footer` | Across the window, under the view | Any, top to bottom | Its element |
//! | `Overlays` | The window's layer, over everything placed | Any, later ones on top | Its surfaces, pushed on `Frame::layer`, and the anchor its component leaves in the tree, if any |
//! | `Dialogs` | The window's layer, over the overlays | Any, later ones on top | The same |
//!
//! What only one slot reads is in that slot's `Place`, so a part cannot give the shell
//! something it never calls. What the shell asks of every part is a method of [`Part`].
//!
//! Within a slot, and wherever the shell calls every part, the order is the order of the
//! registration lines. A slot with no part adds nothing: no element, no node in the
//! accessibility tree, no focus stop.
//!
//! Of the views, the one the saved `browser_mode` names is shown ([`View::shows`]), and the
//! first registered when none answers to it. A view that is not shown is built and then left
//! alone: it has no listener, and none of its hooks run.
//!
//! # What a part writes
//!
//! 1. A module with its state, and `impl Part` for that state: its place, the constructor,
//!    and only the methods it needs. Every other method does nothing by default.
//! 2. Two lines in `lib.rs`, each under the comment that names its package: `mod <module>;`
//!    and `<field>: <module>::<State>,` in [`parts!`]. The state is then `app.<field>`.
//!    rustfmt does not follow a module that a macro declares, so the two cannot be one. A
//!    registration line takes no `///` and no `#[cfg]`.
//!
//! Nothing else in the shell changes for a new part.
//!
//! # What the shell gives a part
//!
//! | A part needs | It gets |
//! | --- | --- |
//! | A place in the window | [`Part::PLACE`]. Its [`Build`] is called each time the view runs and returns the element for the slot's place, or nothing this time |
//! | Its state, by accessor | A field of [`App`](crate::App), so a component takes `\|app: &App\| &app.<field>…` and `\|app: &mut App\| &mut app.<field>…`. Fields are disjoint borrows: a handler may hold two parts at once |
//! | Its saved settings | [`Setup::settings`] in [`Part::new`], read once: the window does not yet follow a setting that another window or process changes while it is open. A part that changes one itself calls `shell::saved(app).change(…)` |
//! | Its share of `Browser`'s events | [`Part::listen`] returns a [`Listener`]. It runs inside the browser's observer, where it may only read the browser and queue: `heard.then(…)` queues an update that runs in the next `poll` with the whole `App`, in the browser's order across every part |
//! | What another service says (a volume monitor, a thumbnailer, a session store) | [`Setup::later`], taken in [`Part::new`] and kept in the part's state. The service's callback runs on the interface thread, inside GLib's dispatch like the browser's observer, and only queues: `later.then(…)` puts an update in the same queue, which both hosts apply after every dispatch. A queue of the part's own, read in `poll`, is not the same: `poll` waits for a frame |
//! | To act on the browser | A handler calls `shell::publish(app)` before it asks the browser to act on its cursor, and `shell::settle(app, tx)` when it is done |
//! | To tell the browser what the user changed in it | [`Part::publish`], called before GLib dispatches and around every applied queue; [`Part::caught_up`], called once a queue is applied |
//! | Its own timers | [`Part::poll`], called in Rune's `poll` after the queue is applied |
//! | The keyboard | A part focuses its own element with `Tx::focus`. A field or a menu gives the keyboard back to the shown view: `shell::view_focus(app)` is the key for a component's `focus_return`. A view whose keyboard leaves it at its start (Left in the first column) calls `shell::leave_view_at_start(app, tx)`: the sidebar takes it at its [`Sidebar::focus`], and nothing happens when there is no sidebar |
//! | The view, whichever is shown | `shell::shown_view(app)`. A part never names a view's module. The rename editor opens and polls through the shown view's [`Editing`]; a chosen menu row runs a command of the registry ([`crate::commands`]), which asks the view's [`Rows`] only for what differs from view to view; the QA backend reads its panes by the view's [`Probe`] |
//! | The id of an entry | `shell::id_of(app, &location)`: the window's one table, which [`Setup::ids`] also hands a part for its listener |
//! | A context menu on one of its rows, or on its folder | A view's handler calls `menus::open(app, Request { depth, entry, anchor }, tx)` for the collection's `Intent::ContextMenu`, with no entry for the folder. The opener decides what the menu is for and lists, once for every view, and asks the view only for a row's position ([`Rows`]) |
//! | Its commands: a window chord, a menu row, a header button | [`Part::COMMANDS`], a table of [`Action`]s. Each has the name a menu row or a header button asks for it by, the shared catalog's command it is, the stage of the GTK window's dispatcher that reads its keys with that stage's matcher and guard, and what it does. `rename.rs` is the example. Two parts that are active together may not register one name: the window does not start |
//! | The window's commands to wait while it has the keyboard | [`Part::keyboard`] says which of the GTK window's owners of input the part is now ([`Holds`]): an inline edit, a menu, a modal layer and whether a field of its own is being typed in, the location bar, the filter, another text field. `commands/owners.rs` holds what each leaves of the stages. A part whose element takes text or is a Rune modal answers it |
//!
//! # Rules for a part
//!
//! - **A listener only reads the browser and queues, with `heard`.** It never calls something
//!   that makes the browser act: the event that raised would be heard inside this one, and
//!   queued before it. It never queues with `later` either: that update would be ahead of
//!   what the other parts queue for the same event.
//! - **`later` is for what is not a browser event.** A service's callback uses it. So may a
//!   handler or a `poll` whose service answers at once: the shell applies the queue again
//!   after the parts' polls, and a handler's update is applied before the next frame. Where
//!   the part holds the `App` and the answer both, it changes the state directly.
//! - **A worker thread's result comes back through GLib,** not through `later`, which is not
//!   `Send`: a future on the interface thread's context (`glib::MainContext::spawn_local`)
//!   or `MainContext::invoke` runs the callback there, and the callback queues. The windowed
//!   host carries that dispatch into Rune's loop with `AppHandle::update_if`. A part never
//!   holds a `Waker`: under a covered window a `Waker` alone does not run `poll`.
//! - **A part keeps no `Rc<Browser>`.** It reads `app.browser` at the call, and it can build
//!   everything it shows again from the browser: a tab, or a view shown later, will ask.
//! - **[`Part::new`] starts no work:** no load, no timer, no observer. A view that is not
//!   shown is built too.
//! - **A handler that calls the browser ends with `shell::settle(app, tx)`,** after
//!   `shell::publish(app)` where the browser acts on its cursor. `heard` in
//!   `columns/intents.rs` and `call` in `commands.rs` are the pattern. Without the `settle`, what the browser answered at
//!   once stays queued until the next poll, and the frame drawn for the input shows the view
//!   one step behind the browser.
//! - **Element keys are one namespace across the parts.** Start a part's keys with its name.
//! - **Nothing outside a part's own module and its two lines is the part's to edit:** not
//!   `shell.rs`, not this module, not another part. Touching a file another package owns is a
//!   `contract` pull request of its own, never a change made in passing.
//! - **One line of this module is a part's to delete.** A slot nothing fills has a variant of
//!   [`Place`] that nothing makes, and that variant carries a `cfg_attr(…, expect(dead_code))`
//!   line. The package that fills the slot deletes that line, and that line only; clippy
//!   fails with "this lint expectation is unfulfilled" until it does.
//! - **A part's place can be tested without registering it.** `test_support::hosted_over`
//!   runs the window over a table a test makes up, as `shell/tests.rs` does with a part in
//!   every slot. `crate::parts_and(more)` is the registered table with a test's parts after
//!   it, for a made-up part beside the real views (`commands/tests/views.rs`).
//! - **No CI job builds this crate.** Run `mise run test-rune slots::` before pushing and
//!   again after merging `strata-next`: a second part in a slot for one, or a part registered
//!   twice, shows only there and in a debug build's start.
//!
//! # Saving a setting
//!
//! `shell::saved(app)` is the window's saved settings. A part calls two things on it:
//! `preferences()`, the values as they are now, and `change(|settings| …)`, which changes
//! them and queues the save of what changed through the bookkeeping the GTK application
//! saves with (`strata_desktop::settings::Saving`). Its other methods are the shell's and
//! start-up's.
//!
//! - **Call `change` from a handler.** It borrows all of `app`, so read whatever the new
//!   value depends on into a local first. From a queued update it works too, and the save's
//!   result is then heard a pump later (`shell::saved`).
//! - **`change` does not validate.** A plain value is assigned as it is. A value with a range
//!   or a rule goes through the function in `strata_desktop::settings` that holds the rule;
//!   where that rule is still inside a GTK setter, the package that needs it moves it there
//!   first (`docs/rune/shared-state.md`, "The rule for a setter's body").
//! - **Nothing is notified.** No part hears that a setting changed, and nothing follows a
//!   change another window or process saves: the part that changes a value also updates what
//!   it shows, in its own state.
//! - **`show_hidden`, `folders_first` and the sort keys are the browser's.** The shell stores
//!   them whenever the browser says they changed, over whatever `change` wrote. Change them
//!   through the browser.
//! - **A failed save is only logged.** There is no notice yet.
//! - **A test that seeds `settings.toml` gives it `mode` and `theme`,** unless it means the
//!   salvaged case: a file without them is salvaged, and every default is then saved into it
//!   with the first change.
//! - **The offscreen host does not flush the writer at exit.** A test waits for the file.
//!
//! # What waits on what
//!
//! | A part that needs | Waits on |
//! | --- | --- |
//! | A key the shown view's own controller takes (Escape, Delete, Space, Ctrl+A), as a command | Nothing: register it at the GTK dispatcher's stage that reads it. A view's collection asks the registry ahead of its own keys (`commands::in_view`), for every stage ahead of `ItemNavigation`, and the view keeps each key no command takes. A view built on a collection of its own calls `commands::in_view` on it once. A header button's or a menu row's command with no key is registered under the button's name (`shell.rs`) or the row's id (`menus.rs`) |
//! | To save a setting | Nothing: `shell::saved(app).change(…)`, by the rules under [Saving a setting](#saving-a-setting) |
//! | A `Browser` and a state for each tab | lgse/strata#2131, which opens with a `contract` change. There is one `Browser`, and one state for each part, made once in `App::new` |
//! | To change the shown view while the window is open | lgse/strata#2140, with a `contract` change: a setter, and a listener and a rebuild from the browser for a view that starts hidden |
//! | Another order within a slot | A `contract` change. The order is the registration lines', which is the packages', and a line is not moved from under its comment |
//! | The stored names `grid` and `explorer` read in one place | The preference store's move, lgse/strata#2125: they are read in `src/ui/preferences/stored.rs` on the GTK side. Until then the Icons view's `shows` answers to `icons` and `grid`, and the List view's to `list` and `explorer` |
//! | More of the Columns view than the listing and the handover | The listing a view shows is shared (`listing.rs`), and so is the cursor and selection handover (`handover.rs`), which Columns uses for each column and a single-pane view through `pane.rs`. What stays in `columns/` is the Columns component's own: `Listings` as its `ColumnSources`, the adapter that puts a change into `ColumnsState`, the active column and the mirror. A view that needs more of it opens a `contract` change |
//! | Panes the QA backend cannot read by a view's [`Probe`] | A `contract` change with the harness, lgse/strata#2135. The backend takes a pane to be the nearest named group around a keyed entry list, with rows keyed as the collection keys them; `qa/roles.rs` has to know the view's roles |
//!
//! # Who owns what
//!
//! The orchestrating session's defaults for the parity plan, for the owner to confirm.
//! Touching a file another package owns is a `contract` pull request of its own.
//!
//! | File | Owner |
//! | --- | --- |
//! | `shell.rs`, `slots.rs` and `slots/`, `host/`, `test_support.rs` | The contract |
//! | `commands.rs` and `commands/`: the command registry, the key routing and the press builder | The contract |
//! | `events.rs` and `listing.rs`: the reader a view's listener calls, and the listing it fills | The contract |
//! | `handover.rs`: the cursor and selection every view hands the browser and takes from it | The contract |
//! | `pane.rs`: the one folder a single-pane view (Icons, List) shows, the browser's active column | The contract |
//! | `single/`: what the single-pane views share beside their pane (intents, rename, the listener, the base collection, `Rows`) | The contract |
//! | `surface.rs`: what a listing that shows no entry says, in every view | The contract |
//! | `thumbnails/`: the render pool, a view's viewer in the window's store, the thumbnail kinds | The contract |
//! | `details.rs`: the text of a row's facts, as every view shows them | The contract |
//! | `fills.rs`: the metadata fills a view asks for the rows near its viewport | The contract |
//! | `lib.rs` | Each package, under its own comment and nowhere else |
//! | A part's module | The package that registered it |
//! | `columns/` | No area package: a change to it by any package is a `contract` pull request |
//! | `menus.rs` | The menus' package, lgse/strata#2133 |
//! | `rename.rs` | The entry-management package, lgse/strata#2134 |
//! | `settings.rs` | The stores' package, lgse/strata#2125, and after it the settings' package, lgse/strata#2150 |
//! | `qa/` | The harness, lgse/strata#2135. An area adds its own steps and predicates in files of its own |
//!
//! # The Columns view, as the example
//!
//! `columns/part.rs` is the whole of it:
//!
//! - `PLACE` is `Place::View`: `build` returns `columns::view(app).build(app)`, `shows`
//!   answers to `"columns"`, `focus` is the strip's key, and `pointer_at` feeds the mirror.
//!   `rows`, `editing` and `probe` are what the context menu, the rename editor and the QA
//!   backend called on the Columns view by name before there was a view slot.
//! - `new` reads the click counts, the mirror choice and "Type to search" from the settings,
//!   and takes the id table.
//! - `listen` copies the rows an event changed out of the browser ([`crate::events`]) and
//!   queues `app.columns.apply(change, …)` for each.
//! - `poll` runs the view's timers.
//! - `publish` hands the browser the cursor and the selection the view moved, and
//!   `caught_up` notes where the browser left the cursor.
//!
//! Its registration is `mod columns;` and `columns: columns::Columns,`. Its handlers are in
//! `columns/intents.rs`, where Left in the first column calls `shell::leave_view_at_start`. The rename editor (`rename.rs`) and the context menu (`menus.rs`)
//! are the two overlays: the editor's message is registered first, so it shows under a menu.

use rune::{Cx, El, Tx};
use strata_components::{contract::EntryId, rename::Polled};
use strata_desktop::settings::Preferences;

use crate::{
    App,
    commands::{Action, Holds},
};

mod ids;
mod inbox;
mod registry;

pub(crate) use ids::Ids;
pub(crate) use inbox::{Heard, Inbox, Later, Listener};
pub(crate) use registry::{Entry, Registry};

/// A place in the window. The module docs say where each is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    Tabs,
    Location,
    Sidebar,
    View,
    Drawer,
    Footer,
    Overlays,
    Dialogs,
}

impl Slot {
    /// The slots the shell fills with one element: a second part there is a mistake.
    pub(crate) const fn holds_one(self) -> bool {
        matches!(
            self,
            Self::Tabs | Self::Location | Self::Sidebar | Self::Drawer
        )
    }
}

/// What a part is built from, and what its listener may keep.
pub(crate) struct Setup<'a> {
    /// The saved settings, as the file held them when the process started.
    pub(crate) settings: &'a Preferences,
    /// The one table that names an entry for the components.
    pub(crate) ids: &'a Ids,
    /// Where the callback of a service other than `Browser` queues: clone it into the state.
    pub(crate) later: &'a Later<App>,
}

/// What a part builds its element with, for one run of the view.
pub(crate) struct Frame<'a, S: 'static> {
    pub(crate) cx: &'a mut Cx<S>,
    /// What floats over the window, the later the higher. The shell fills the slots in the
    /// order [`Slot`] declares them, so an overlay is over whatever a placed part pushed, and
    /// a dialog over every overlay.
    pub(crate) layer: &'a mut Vec<El>,
}

/// Builds a part's element for its slot's place, or nothing this time.
pub(crate) type Build<S> = fn(&S, &mut Frame<'_, S>) -> Option<El>;

/// Builds the drawer around the files: the two beside each other, as one element.
pub(crate) type Wrap<S> = fn(&S, El, &mut Frame<'_, S>) -> El;

/// What a view gives the shell.
pub(crate) struct View<S: 'static> {
    pub(crate) build: Build<S>,
    /// Whether this is the view the saved `browser_mode` names. A view answers to every name
    /// the setting has had for it: the GTK application reads `grid` as Icons and `explorer`
    /// as List (`src/ui/preferences/stored.rs`).
    pub(crate) shows: fn(&str) -> bool,
    /// The key of the element that takes the keyboard when a field or a menu gives it back.
    pub(crate) focus: &'static str,
    /// Whether the view shows a folder in file-type groups when the saved `group_by_type`
    /// asks (`BrowserMode::supports_type_grouping` in `src/ui/browser_modes.rs`: the List
    /// view alone). The one source of it: the shell tells the browser
    /// (`Browser::set_type_grouping`), and a view's pane follows the browser.
    pub(crate) supports_type_grouping: bool,
    pub(crate) pointer_at: Option<PointerAt<S>>,
    pub(crate) rows: Rows<S>,
    pub(crate) editing: Editing<S>,
    #[cfg(feature = "qa")]
    pub(crate) probe: Probe<S>,
}

/// A row of the shown view: the browser's column, and the entry's id.
pub(crate) type At = (usize, EntryId);

/// What the parts that act on a row ask of the view it is in: the context menu and the
/// rename editor.
pub(crate) struct Rows<S: 'static> {
    /// The browser's position of a row.
    pub(crate) position: fn(&S, At) -> Option<usize>,
    /// The entry so many rows after a row, as the view lists them.
    pub(crate) neighbour: fn(&S, At, isize) -> Option<EntryId>,
    /// Opens a row's entry as a click on it does.
    pub(crate) activate: fn(&mut S, At),
    /// Selects a row alone, with the cursor and the keyboard on it.
    pub(crate) select_only: fn(&mut S, At, &mut Tx),
    /// Selects every row of one of the browser's columns.
    pub(crate) select_all: fn(&mut S, usize),
    /// Scrolls a row into view, just far enough: what a committed rename shows of the row it
    /// renamed, wherever the new name sorts (`reveal_completed_rename`).
    pub(crate) reveal: fn(&mut S, At, &mut Tx),
}

/// What the rename editor asks of the view it edits in.
pub(crate) struct Editing<S: 'static> {
    /// Opens the editor on a row and shows the row. None when the view has no such row, or
    /// cannot rename in place.
    pub(crate) begin: fn(&mut S, At, &mut Tx) -> Option<Polled>,
    /// One poll of the editor against the row's listing and viewport.
    pub(crate) tick: fn(&mut S) -> Polled,
    /// For a view where something follows the cursor, as the child column does in Columns:
    /// an edit is open, or none is, and what follows waits while one is.
    pub(crate) hold: Option<fn(&S, bool)>,
    /// For such a view: what follows the cursor follows it now, with no wait.
    pub(crate) follow_now: Option<fn(&S)>,
}

/// What the QA backend needs to know of a view to read its panes out of the tree: which
/// elements are the entry lists, by key, and what Rune calls a list and an entry in it. A
/// pane is the nearest named group around a list. An entry is a node of the entry role keyed
/// as the collection keys an entry's row (`collection::element::row_key`), so a node a view
/// puts in the list that is no entry, such as a table's headings or a group's header, must
/// not take that key's form.
#[cfg(feature = "qa")]
pub(crate) struct Probe<S: 'static> {
    /// The view's name in the QA protocol: `view_name` in `src/ui/accessibility.rs`.
    pub(crate) name: &'static str,
    /// Rune's role of an entry list, and of an entry in one.
    pub(crate) roles: (&'static str, &'static str),
    /// Whether the folder the launch opened has been listed, or could not be.
    pub(crate) listed: fn(&S) -> bool,
    /// The key of each entry list in the tree, the first pane's first.
    pub(crate) lists: fn(&S) -> Vec<String>,
}

/// What the sidebar gives the shell.
pub(crate) struct Sidebar<S: 'static> {
    pub(crate) build: Build<S>,
    /// The key of the element that takes the keyboard when it leaves the view at its start,
    /// as Left in the first column asks.
    pub(crate) focus: &'static str,
}

/// Tells a view the pointer moved over the body, to this place in window points.
pub(crate) type PointerAt<S> = fn(&mut S, (f32, f32));

/// A part's slot, with what the shell reads of a part there and nowhere else.
///
/// A slot with no part yet has a variant nothing makes, which the compiler would report.
/// Each such variant expects that, on a line of its own: the package that fills the slot
/// deletes its variant's `cfg_attr` line, and the lint says so if it forgets.
pub(crate) enum Place<S: 'static> {
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "no part fills the tabs slot yet")
    )]
    Tabs(Build<S>),
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "no part fills the location slot yet")
    )]
    Location(Build<S>),
    Sidebar(Sidebar<S>),
    View(View<S>),
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "no part fills the drawer slot yet")
    )]
    Drawer(Wrap<S>),
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "no part fills the footer slot yet")
    )]
    Footer(Build<S>),
    Overlays(Build<S>),
    Dialogs(Build<S>),
}

impl<S> Place<S> {
    pub(crate) const fn slot(&self) -> Slot {
        match self {
            Self::Tabs(_) => Slot::Tabs,
            Self::Location(_) => Slot::Location,
            Self::Sidebar(_) => Slot::Sidebar,
            Self::View(_) => Slot::View,
            Self::Drawer(_) => Slot::Drawer,
            Self::Footer(_) => Slot::Footer,
            Self::Overlays(_) => Slot::Overlays,
            Self::Dialogs(_) => Slot::Dialogs,
        }
    }

    /// What builds the part's element. None for the drawer, which wraps the view's.
    pub(crate) const fn build(&self) -> Option<Build<S>> {
        match self {
            Self::View(view) => Some(view.build),
            Self::Sidebar(sidebar) => Some(sidebar.build),
            Self::Drawer(_) => None,
            Self::Tabs(build)
            | Self::Location(build)
            | Self::Footer(build)
            | Self::Overlays(build)
            | Self::Dialogs(build) => Some(*build),
        }
    }
}

/// One part of the interface: the state the shell keeps for it, and what the shell asks of
/// it. Every hook but the constructor takes the whole [`App`], because a part acts on the
/// browser and on its neighbours; the state is `app.<field>`.
pub(crate) trait Part: Sized + 'static {
    /// The part's slot, and what it gives the shell there.
    const PLACE: Place<App>;

    /// Builds the state. It starts no work: a view that is not shown is built too.
    fn new(setup: &Setup<'_>) -> Self;

    /// What reads this part's share of an event out of the browser. It runs inside the
    /// browser's observer, while Rune holds the application state: it only queues.
    fn listen(_setup: &Setup<'_>) -> Option<Listener<App>> {
        None
    }

    /// Hands the browser what the user changed in this part since it last agreed with it.
    fn publish(_app: &mut App) {}

    /// The queue of the browser's events has been applied, this part's updates among them.
    fn caught_up(_app: &mut App) {}

    /// One poll: the part's own timers. True when the view has to run.
    fn poll(_app: &mut App, _clock: f64, _tx: &mut Tx) -> bool {
        false
    }

    /// What this part's commands do, by any route: a key, a menu row, a header button
    /// ([`crate::commands`]). A menu row runs the command whose id is the row's, and a
    /// header button the one named for it, so a part lights either by registering it here.
    const COMMANDS: &'static [Action<App>] = &[];

    /// What this part has of the keyboard now, as one of the GTK window's owners of input:
    /// a text field, an inline edit, a menu, a modal layer. The window's commands that the
    /// owner keeps from running wait. `None` while the part has none of it.
    fn keyboard(_app: &App) -> Option<Holds> {
        None
    }
}

/// Registers the parts and makes the application state from them: [`App`](crate::App), with
/// a field for each line, and `PARTS`, the table the shell's [`Registry`] reads.
///
/// A line is `<field>: <module>::<State>,` under the comment that names its package, and
/// nothing else: no `///` and no `#[cfg]`. Every package has a comment of its own so that two
/// branches never add a line at the same place, which is what lets git merge them without a
/// conflict.
macro_rules! parts {
    ($($field:ident: $part:ty,)*) => {
        /// The application state: what the window keeps for itself, and each registered
        /// part's state (`slots::parts!`).
        pub(crate) struct App {
            pub(crate) look: ::strata_kit::look::Look,
            pub(crate) browser: ::std::rc::Rc<::strata_fs::app::Browser>,
            pub(crate) window: $crate::shell::Window,
            $(pub(crate) $field: $part,)*
        }

        /// Every registered part's state, built and not yet in an [`App`].
        pub(crate) struct Built {
            $($field: $part,)*
        }

        impl Built {
            /// Builds each part, in the order they are registered.
            pub(crate) fn new(setup: &$crate::slots::Setup<'_>) -> Self {
                Self {
                    $($field: <$part as $crate::slots::Part>::new(setup),)*
                }
            }
        }

        impl App {
            pub(crate) fn assemble(
                look: ::strata_kit::look::Look,
                browser: ::std::rc::Rc<::strata_fs::app::Browser>,
                window: $crate::shell::Window,
                parts: Built,
            ) -> Self {
                Self {
                    look,
                    browser,
                    window,
                    $($field: parts.$field,)*
                }
            }
        }

        /// The registered parts, in the order of their lines.
        pub(crate) const PARTS: &[$crate::slots::Entry<App>] = &[
            $($crate::slots::Entry::of::<$part>(stringify!($field)),)*
        ];

        /// The registered parts with `more` after them: a table for a test that hosts a
        /// made-up part beside the real ones (`test_support::hosted_over`).
        #[cfg(test)]
        pub(crate) fn parts_and(
            more: Vec<$crate::slots::Entry<App>>,
        ) -> Vec<$crate::slots::Entry<App>> {
            let mut parts = vec![$($crate::slots::Entry::of::<$part>(stringify!($field)),)*];
            parts.extend(more);
            parts
        }
    };
}
pub(crate) use parts;

#[cfg(test)]
mod tests;
