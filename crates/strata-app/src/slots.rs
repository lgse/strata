// SPDX-License-Identifier: MIT

//! The shell's slots: where each part of the interface goes in the window, and what the shell
//! gives it. A part is a module of this crate, registered in `lib.rs`. The shell builds no
//! part by name: it asks the registry who is in a slot.
//!
//! # The slots
//!
//! | [`Slot`] | Where the shell puts it | How many |
//! | --- | --- | --- |
//! | `Tabs` | Across the window, above the header | One |
//! | `Location` | The header's flexible middle (`Header::center`) | One |
//! | `Sidebar` | Under the header, before the view | One |
//! | `View` | Under the header: the files | Any, and one is shown |
//! | `Drawer` | Around the view: it is handed the view's element and returns the split | One |
//! | `Footer` | Across the window, under the view | Any, top to bottom |
//! | `Overlays` | The window's layer, over everything placed | Any, later ones on top |
//! | `Dialogs` | The window's layer, over the overlays | Any, later ones on top |
//!
//! Within a slot, and wherever the shell calls every part, the order is the order of the
//! registration lines. A slot with no part adds nothing: no element, no node in the
//! accessibility tree, no focus stop.
//!
//! Of the views, the one the saved `browser_mode` names is shown ([`Part::shows`]), and the
//! first registered when none answers to it. A view that is not shown is built and then left
//! alone: none of its hooks run. Changing the view while the window is open is not here yet.
//!
//! # What a part writes
//!
//! 1. A module with its state, and `impl Part` for that state: the slot, the constructor, and
//!    only the hooks it needs. Every other hook does nothing by default.
//! 2. Two lines in `lib.rs`, each under the comment that names its package: `mod <module>;`
//!    and `<field>: <module>::<State>,` in [`parts!`]. The state is then `app.<field>`.
//!    rustfmt does not follow a module that a macro declares, so the two cannot be one.
//!
//! Nothing else in the shell changes for a new part.
//!
//! # What the shell gives a part
//!
//! | A part needs | It gets |
//! | --- | --- |
//! | A place in the window | [`Part::build`], called each time the view runs, returns the element for the slot's place. `Frame::layer` takes what floats over the window: an overlay or a dialog pushes its surfaces there and returns only an anchor, if its component has one. The drawer implements [`Part::wrap`] instead |
//! | Its state, by accessor | A field of [`App`](crate::App), so a component takes `\|app: &App\| &app.<field>…` and `\|app: &mut App\| &mut app.<field>…`. Fields are disjoint borrows: a handler may hold two parts at once |
//! | Its saved settings | [`Setup::settings`] in [`Part::new`], read once: the window does not yet follow a setting that changes while it is open |
//! | Its share of `Browser`'s events | [`Part::listen`] returns a [`Listener`]. It runs inside the browser's observer, where it may only read the browser and queue: `heard.then(…)` queues an update that runs in the next `poll` with the whole `App`, in the browser's order across every part |
//! | To act on the browser | A handler calls `shell::publish(app)` before it asks the browser to act on its cursor, and `shell::settle(app, tx)` when it is done, so what the call queued is applied before the next frame |
//! | To tell the browser what the user changed in it | [`Part::publish`], called before GLib dispatches and around every applied queue; [`Part::caught_up`], called once a queue is applied |
//! | Its own timers | [`Part::poll`], called in Rune's `poll` after the queue is applied |
//! | The pointer, for the shown view | [`Part::pointer_at`], for each move over the body |
//! | The keyboard | A part focuses its own element with `Tx::focus`. [`Part::FOCUS`] names the element that takes the keyboard back, and `shell::focus_key(app, Slot::View)` is where a field or a menu returns it (a component's `focus_return`) |
//! | The window's shortcuts to wait | [`Part::holds_keyboard`], true while the part has a text field open |
//!
//! Keys are otherwise not the slots' business. A component installs its own keyboard
//! controller on its element, and what the focused element passes on reaches the window's key
//! table in `shell::on_key`, which still names what each chord does. A part has no keys of its
//! own to register: the command registry that replaces the table brings that.
//!
//! # The Columns view, as the example
//!
//! `columns/part.rs` is the whole of it:
//!
//! - `SLOT` is `Slot::View`, `shows` answers to `"columns"`, and `FOCUS` is the strip's key.
//! - `new` reads the click counts, the mirror choice and "Type to search" from the settings,
//!   and takes the id table.
//! - `listen` copies the rows an event changed out of the browser ([`crate::events`]) and
//!   queues `app.columns.apply(change, …)` for each.
//! - `build` returns `columns::view(app).build(app)`; `poll` runs the view's timers.
//! - `publish` hands the browser the cursor and the selection the view moved, `caught_up`
//!   notes where the browser left the cursor, and `pointer_at` feeds the mirror.
//!
//! Its registration is `mod columns;` and `columns: columns::Columns,`. The rename editor
//! (`rename.rs`) and the context menu (`menus.rs`) are the two overlays: the editor's message
//! is registered first, so it shows under a menu.

use rune::{Cx, El, Tx};
use strata_desktop::settings::Preferences;

use crate::{App, columns::Ids};

mod inbox;
mod registry;

pub(crate) use inbox::{Heard, Inbox, Listener};
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
}

/// What a part builds its element with, for one run of the view.
pub(crate) struct Frame<'a, S: 'static> {
    pub(crate) cx: &'a mut Cx<S>,
    /// What floats over the window, the later the higher. The shell fills the slots in the
    /// order [`Slot`] declares them, so an overlay is over whatever a placed part pushed, and
    /// a dialog over every overlay.
    pub(crate) layer: &'a mut Vec<El>,
}

/// One part of the interface: the state the shell keeps for it, and what the shell asks of
/// it. Every hook but the constructor takes the whole [`App`], because a part acts on the
/// browser and on its neighbours; the state is `app.<field>`.
pub(crate) trait Part: Sized + 'static {
    const SLOT: Slot;

    /// The key of the element that takes the keyboard when it is given back to this part.
    const FOCUS: Option<&'static str> = None;

    fn new(setup: &Setup<'_>) -> Self;

    /// For a view: whether it is the one the saved `browser_mode` names.
    fn shows(_mode: &str) -> bool {
        false
    }

    /// What reads this part's share of an event out of the browser. It runs inside the
    /// browser's observer, while Rune holds the application state: it only queues.
    fn listen(_setup: &Setup<'_>) -> Option<Listener<App>> {
        None
    }

    /// The part's element for its slot's place, or nothing this time.
    fn build(_app: &App, _frame: &mut Frame<'_, App>) -> Option<El> {
        None
    }

    /// For the drawer: the files and the drawer beside them, as one element.
    fn wrap(_app: &App, files: El, _frame: &mut Frame<'_, App>) -> El {
        files
    }

    /// Hands the browser what the user changed in this part since it last agreed with it.
    fn publish(_app: &mut App) {}

    /// The queue of the browser's events has been applied, this part's updates among them.
    fn caught_up(_app: &mut App) {}

    /// One poll: the part's own timers. True when the view has to run.
    fn poll(_app: &mut App, _clock: f64, _tx: &mut Tx) -> bool {
        false
    }

    /// For the shown view: the pointer moved over the body, to `at` in window points.
    fn pointer_at(_app: &mut App, _at: (f32, f32)) {}

    /// The part has a text field open, so a key the field passes on is not a shortcut.
    fn holds_keyboard(_app: &App) -> bool {
        false
    }
}

/// Registers the parts and makes the application state from them: [`App`](crate::App), with
/// a field for each line, and `PARTS`, the table the shell's [`Registry`] reads.
///
/// A line is `<field>: <module>::<State>,` under the comment that names its package. Every
/// package has a comment of its own so that two branches never add a line at the same place,
/// which is what lets git merge them without a conflict.
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
