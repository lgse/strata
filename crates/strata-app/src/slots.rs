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
//! | `Sidebar` | Under the header, before the view | One | Its element |
//! | `View` | Under the header: the files | Any, and one is shown | A [`View`]: its element, the saved mode it answers to, the key that takes the keyboard back, what it does with the pointer, and what the rename editor, a context menu and the QA backend ask of the view that is shown |
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
//! | Its saved settings | [`Setup::settings`] in [`Part::new`], read once: the window does not yet follow a setting that changes while it is open |
//! | Its share of `Browser`'s events | [`Part::listen`] returns a [`Listener`]. It runs inside the browser's observer, where it may only read the browser and queue: `heard.then(…)` queues an update that runs in the next `poll` with the whole `App`, in the browser's order across every part |
//! | What another service says (a volume monitor, a thumbnailer, a session store) | [`Setup::later`], kept in the part's state. The service's callback runs inside GLib's dispatch like the browser's observer, and only queues: `later.then(…)` puts an update in the same queue, which both hosts apply after every dispatch. A queue of the part's own, read in `poll`, is not the same: `poll` waits for a frame |
//! | To act on the browser | A handler calls `shell::publish(app)` before it asks the browser to act on its cursor, and `shell::settle(app, tx)` when it is done |
//! | To tell the browser what the user changed in it | [`Part::publish`], called before GLib dispatches and around every applied queue; [`Part::caught_up`], called once a queue is applied |
//! | Its own timers | [`Part::poll`], called in Rune's `poll` after the queue is applied |
//! | The keyboard | A part focuses its own element with `Tx::focus`. A field or a menu gives the keyboard back to the shown view: `shell::view_focus(app)` is the key for a component's `focus_return` |
//! | The view, whichever is shown | `shell::shown_view(app)`. A part never names a view's module: the rename editor asks the shown view's [`Editing`], a context menu sends a chosen row to its `chosen`, and the QA backend reads its [`Probe`] |
//! | The window's shortcuts to wait | [`Part::holds_keyboard`], true while the part has a text field open |
//!
//! # Rules for a part
//!
//! - **A listener only reads the browser and queues.** It never calls something that makes
//!   the browser act: the event that raised would be heard inside this one, and queued
//!   before it.
//! - **A part keeps no `Rc<Browser>`.** It reads `app.browser` at the call, and it can build
//!   everything it shows again from the browser: a tab, or a view shown later, will ask.
//! - **[`Part::new`] starts no work:** no load, no timer, no observer. A view that is not
//!   shown is built too.
//! - **A handler that calls the browser ends with `shell::settle(app, tx)`,** after
//!   `shell::publish(app)` where the browser acts on its cursor. `heard` and `chosen` in
//!   `columns/intents.rs` are the pattern. Without the `settle`, what the browser answered at
//!   once stays queued until the next poll, and the frame drawn for the input shows the view
//!   one step behind the browser.
//! - **Element keys are one namespace across the parts.** Start a part's keys with its name.
//! - **Nothing outside a part's own module and its two lines is the part's to edit:** not
//!   `shell.rs`, not this module, not another part.
//! - **A part's place can be tested without registering it.** `test_support::hosted_over`
//!   runs the window over a table a test makes up, as `shell/tests.rs` does with a part in
//!   every slot.
//! - **No CI job builds this crate.** Run `mise run test-rune slots::` before pushing and
//!   again after merging `strata-next`: a second part in a slot for one, or a part registered
//!   twice, shows only there and in a debug build's start.
//!
//! # What waits on what
//!
//! | A part that needs | Waits on |
//! | --- | --- |
//! | A window chord (Ctrl+T, Ctrl+L, the sidebar's key) or one of the header's five buttons | The command registry, lgse/strata#2126. A component's own keys are its own; `shell::on_key`'s table and `shell::bar` are not an area's to add to. The runs that need them stay `pending` on that package |
//! | To save a setting | The preference store's move to `strata-desktop`, lgse/strata#2125. The shell saves the sort and the hidden-files choice and nothing else |
//! | A `Browser` and a state for each tab | lgse/strata#2131, which opens with a `contract` change. There is one `Browser`, and one state for each part, made once in `App::new` |
//! | To change the shown view while the window is open | lgse/strata#2140, with a `contract` change: a setter, and a listener and a rebuild from the browser for a view that starts hidden |
//! | Another order within a slot | A `contract` change. The order is the registration lines', which is the packages', and a line is not moved from under its comment |
//!
//! # Who owns what
//!
//! | File | Owner |
//! | --- | --- |
//! | `shell.rs`, `slots.rs` and `slots/`, the id table among them | The contract: a change is a `contract` pull request of its own |
//! | `events.rs`, the listing reader a view's listener calls | The contract too. It reads into the Columns view's listing types (`columns::{Row, Edit, Load}`); sharing those with another view is a `contract` change that view's package opens with |
//! | A part's module | The package that registered it |
//! | `lib.rs` | Each package, under its own comment and nowhere else |
//!
//! # The Columns view, as the example
//!
//! `columns/part.rs` is the whole of it:
//!
//! - `PLACE` is `Place::View`: `build` returns `columns::view(app).build(app)`, `shows`
//!   answers to `"columns"`, `focus` is the strip's key, and `pointer_at` feeds the mirror.
//!   `editing`, `chosen` and `probe` are what the rename editor, the context menu and the QA
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
//! `columns/intents.rs`. The rename editor (`rename.rs`) and the context menu (`menus.rs`)
//! are the two overlays: the editor's message is registered first, so it shows under a menu.

use rune::{Cx, El, Tx};
use strata_components::{context_menu::MenuIntent, contract::EntryId, rename::Polled};
use strata_core::model::Location;
use strata_desktop::settings::Preferences;

use crate::App;

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
    /// Whether this is the view the saved `browser_mode` names.
    pub(crate) shows: fn(&str) -> bool,
    /// The key of the element that takes the keyboard when a field or a menu gives it back.
    pub(crate) focus: &'static str,
    pub(crate) pointer_at: Option<PointerAt<S>>,
    pub(crate) editing: Editing<S>,
    /// A row of the open context menu was chosen. The menu has closed.
    pub(crate) chosen: fn(&mut S, MenuIntent, &mut Tx),
    #[cfg(feature = "qa")]
    pub(crate) probe: Probe<S>,
}

/// A row of the shown view: the browser's column, and the entry's id.
pub(crate) type At = (usize, EntryId);

/// What the rename editor asks of the view it edits in.
pub(crate) struct Editing<S: 'static> {
    /// The id the view knows the entry at a location by.
    pub(crate) id_of: fn(&S, &Location) -> EntryId,
    /// The browser's position of a row.
    pub(crate) position: fn(&S, At) -> Option<usize>,
    /// The entry so many rows after a row, as the view lists them.
    pub(crate) neighbour: fn(&S, At, isize) -> Option<EntryId>,
    /// Opens the editor on a row and shows the row. None when the view has no such row.
    pub(crate) begin: fn(&mut S, At, &mut Tx) -> Option<Polled>,
    /// One poll of the editor against the row's listing and viewport.
    pub(crate) tick: fn(&mut S) -> Polled,
    /// An edit is open, or none is: what follows the cursor waits while one is.
    pub(crate) hold: fn(&S, bool),
    /// Selects a row alone, with the cursor and the keyboard on it.
    pub(crate) select_only: fn(&mut S, At, &mut Tx),
    /// What follows the cursor follows it now, with no wait.
    pub(crate) follow_now: fn(&S),
}

/// What the QA backend reads of the shown view beside the tree.
#[cfg(feature = "qa")]
pub(crate) struct Probe<S: 'static> {
    /// Whether the folder the launch opened has been listed, or could not be.
    pub(crate) listed: fn(&S) -> bool,
    /// The key of each entry list in the tree, the first pane's first.
    pub(crate) lists: fn(&S) -> Vec<String>,
}

/// Tells a view the pointer moved over the body, to this place in window points.
pub(crate) type PointerAt<S> = fn(&mut S, (f32, f32));

/// A part's slot, with what the shell reads of a part there and nowhere else.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "a slot's variant is first made by the part that fills it; remove this when \
                  every slot has a part and the lint says it is unfulfilled"
    )
)]
pub(crate) enum Place<S: 'static> {
    Tabs(Build<S>),
    Location(Build<S>),
    Sidebar(Build<S>),
    View(View<S>),
    Drawer(Wrap<S>),
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
            Self::Drawer(_) => None,
            Self::Tabs(build)
            | Self::Location(build)
            | Self::Sidebar(build)
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

    /// The part has a text field open, so a key the field passes on is not a shortcut.
    fn holds_keyboard(_app: &App) -> bool {
        false
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
    };
}
pub(crate) use parts;

#[cfg(test)]
mod tests;
