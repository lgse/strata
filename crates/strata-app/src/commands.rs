// SPDX-License-Identifier: MIT

//! The command registry and the key routing: what each command does, by every route that
//! asks for one, and which command a press is.
//!
//! - **One table.** An [`Action`] is a command: the name a menu row or a header button asks
//!   for it by, the shared catalog's command it is, the keys that run it, and what it does. A
//!   key, a chosen menu row and a pressed header button all end in the same `run`.
//! - **GTK decides what a key does.** The GTK window's dispatcher is a chain of stages
//!   (`Dispatcher::handle_key`, `src/ui/window/keyboard.rs`), and a press is the first
//!   stage's that reads it. [`Stage`] is that chain, in its order and under its names; an
//!   action says at which stage GTK reads its keys, with the matcher the stage uses.
//! - **Keys come from the shared catalog** (`strata_desktop::keys::bindings`), through both
//!   maps' tables. A press is built once, in [`press`].
//!
//! - **A press is heard in one of three places,** by who has the keyboard, and walked once
//!   ([`Heard`]): ahead of the shown view's own keys and again after them, at the window's
//!   root element for any other element, and at the window for a text field, a Rune modal
//!   or nothing at all. Rune has no capture phase; these are the places it gives.
//!
//! `docs/rune/parity/packages/B3.md`, "Design", has the oracle's stages line by line, and
//! "What Rune gives" has Rune's order for a key and what each place can and cannot hear.

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};

use rune::{KeyEvent, Tx};
use strata_components::collection::element::Collection;
use strata_desktop::{
    keys::{self, Active, Command, Context, Digits, KeyPress, Keymap, Matcher},
    settings::{KeymapOverrides, Preferences, resolve_keymap},
};

use crate::{App, shell, slots::At};

mod browser;
mod heard;
mod menu_key;
mod owners;
mod press;

use heard::{Asked, Heard, Span, Stamp, span};
pub(crate) use menu_key::view_keys;
pub(crate) use owners::{Holds, Situation};
use owners::{Reach, reach};

/// Declares [`Stage`] and the order a press is offered to its variants from one list, so a
/// stage cannot be declared and never walked.
macro_rules! stages {
    ($($(#[$note:meta])* $stage:ident,)*) => {
        /// Where the GTK window reads a key: the tab strip's controller, the dispatcher's
        /// entry, the stages of its chain in their order (`handle_key`,
        /// `src/ui/window/keyboard.rs:421`), what follows a key the chain left (`after_yield`,
        /// line 531), and the Settings key, which hears what the focused widget did not take
        /// (`composition/settings.rs:71`).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(crate) enum Stage {
            $($(#[$note])* $stage,)*
        }

        impl Stage {
            /// Every stage, in the order a press is offered to them.
            pub(crate) const ALL: &'static [Self] = &[$(Self::$stage,)*];
        }
    };
}

stages! {
    /// `TabWindow::handle_key` in `src/ui/window/composition/tabs.rs`.
    TabStrip,
    /// What the dispatcher does with every press before its chain: a pending bookmark, the
    /// open-with lookup (`keyboard.rs:136`, `:422`).
    Entry,
    RenameTab,
    ZoomShortcut,
    InputOwner,
    TenxerKeys,
    WindowCommands,
    DirectCommands,
    DefaultTabNavigation,
    InlineEditing,
    FilterAndLocationCommands,
    VideoControls,
    SidebarCommands,
    ContextMenuCommand,
    PropertiesCommand,
    SearchResultKeys,
    PreviewPasswordEscape,
    TextInput,
    FileCommands,
    PreviewNavigation,
    FocusNavigation,
    Dismissal,
    ItemNavigation,
    AfterYield,
    Unclaimed,
}

/// What a stage matches a press against.
pub(crate) enum Reads {
    /// Catalog commands, with `in_map` and `ignoring` as the GTK stage has them.
    Commands(&'static [Matcher]),
    /// A digit range of the catalog, such as the tab numbers. Which digit is the command's
    /// to read from the call's press.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "no registered command reads a digit range yet")
    )]
    Digits(Digits),
    /// What neither says: a key that types, for type-to-search, or a key the catalog has as a
    /// bare binding (BackSpace). Nothing can list it.
    Press(fn(&KeyPress) -> bool),
}

impl Reads {
    fn reads(&self, press: &KeyPress) -> bool {
        match self {
            Self::Commands(matchers) => press.is_any(matchers),
            Self::Digits(digits) => press.digit_in(*digits).is_some(),
            Self::Press(reads) => reads(press),
        }
    }
}

/// One stage's reading of a command's keys.
pub(crate) struct Keys<S: 'static> {
    pub(crate) stage: Stage,
    pub(crate) reads: Reads,
    /// The stage's guard, as GTK has it: whether it reads the key where the press lands.
    pub(crate) when: fn(&S, &Situation) -> bool,
}

fn always<S>(_: &S, _: &Situation) -> bool {
    true
}

impl<S> Keys<S> {
    /// The catalog commands `matchers` at `stage`, wherever the press lands.
    pub(crate) const fn at(stage: Stage, matchers: &'static [Matcher]) -> Self {
        Self {
            stage,
            reads: Reads::Commands(matchers),
            when: always,
        }
    }

    pub(crate) const fn when(mut self, when: fn(&S, &Situation) -> bool) -> Self {
        self.when = when;
        self
    }
}

/// Where a command acts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum On {
    /// Where the keyboard is: a key, a header button, a palette.
    Focus,
    /// The row a menu was opened on.
    Row(At),
    /// A column's folder: a background menu.
    Column(usize),
}

/// One call of a command.
pub(crate) struct Call<'a> {
    pub(crate) on: On,
    /// The press, when a key made the call: its digit and what it typed are the command's
    /// to read.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "no registered command reads its press yet")
    )]
    pub(crate) press: Option<&'a KeyPress>,
}

/// What a command answers, which is what a stage of the GTK chain answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// It ran, and the press is taken.
    Ran,
    /// Not for it after all: the next command that reads the press is asked. A command
    /// answers this when it did nothing, never after it changed something: the next command
    /// would then act on top of it, and a key nothing takes gets no commit to show it.
    Declined,
    /// The focused element's: nothing else is asked, and Rune is told the key is not taken.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "no registered command leaves a key yet")
    )]
    Left,
}

/// One command, by every route.
pub(crate) struct Action<S: 'static> {
    /// The name a menu row or a button asks for it by: the row's id, the button's.
    pub(crate) id: &'static str,
    /// The catalog's command this is, when the catalog has it.
    pub(crate) command: Option<Command>,
    /// Where the GTK window's stages read its keys. Empty for a command no key runs.
    pub(crate) keys: &'static [Keys<S>],
    pub(crate) run: fn(&mut S, &Call<'_>, &mut Tx) -> Outcome,
}

impl<S> Action<S> {
    pub(crate) const fn new(
        id: &'static str,
        run: fn(&mut S, &Call<'_>, &mut Tx) -> Outcome,
    ) -> Self {
        Self {
            id,
            command: None,
            keys: &[],
            run,
        }
    }

    pub(crate) const fn command(mut self, command: Command) -> Self {
        self.command = Some(command);
        self
    }

    pub(crate) const fn keys(mut self, keys: &'static [Keys<S>]) -> Self {
        self.keys = keys;
        self
    }
}

/// The settings the catalog reads at each press and each label: which map is in use, whether
/// plain keys type into a search, and the keymap with the saved overrides. The catalog asks
/// whoever an interface registered for its thread (`keys::set_active`), and holds it weakly,
/// so the window keeps this.
pub(crate) struct Live {
    tenxer: Cell<bool>,
    typing: Cell<bool>,
    keymap: RefCell<Rc<Keymap>>,
    /// The saved overrides `keymap` was resolved from.
    resolved: RefCell<KeymapOverrides>,
    /// The press a view's collection last asked the early stages for ([`heard`]).
    asked: Asked,
}

impl Live {
    /// The window's source, registered for this thread and holding `settings`' values. With
    /// `report_to`, the saved keymap's entries that cannot apply are logged against that
    /// file, as GTK logs them once for its process (`PreferenceManager::keymap`).
    pub(crate) fn registered(settings: &Preferences, report_to: Option<&Path>) -> Rc<Self> {
        let live = Rc::new(Self {
            tenxer: Cell::new(false),
            typing: Cell::new(false),
            keymap: RefCell::new(Rc::new(resolve_keymap(&settings.keymap, report_to))),
            resolved: RefCell::new(settings.keymap.clone()),
            asked: Asked::default(),
        });
        live.follow(settings);
        let weak = Rc::downgrade(&live);
        let source: std::rc::Weak<dyn Active> = weak;
        keys::set_active(source);
        live
    }

    /// Takes the values as they are now. The GTK application reads its preference manager at
    /// each press; the window calls this before each press is read and each view is built.
    pub(crate) fn follow(&self, settings: &Preferences) {
        self.tenxer.set(settings.tenxer_mode);
        // `type_to_search_active` in `src/ui/preferences/input_settings.rs`.
        self.typing
            .set(settings.type_to_search && !settings.tenxer_mode);
        // As GTK's `edit_keymap` rebuilds the tables after a change, without logging again.
        if *self.resolved.borrow() != settings.keymap {
            self.keymap
                .replace(Rc::new(resolve_keymap(&settings.keymap, None)));
            self.resolved.replace(settings.keymap.clone());
        }
    }
}

impl Active for Live {
    fn tenxer_mode(&self) -> bool {
        self.tenxer.get()
    }

    fn type_to_search_active(&self) -> bool {
        self.typing.get()
    }

    fn keymap(&self) -> Rc<Keymap> {
        self.keymap.borrow().clone()
    }
}

/// The name the window's own commands go by where a part's go by the part's.
const WINDOW: &str = "window";

/// Every command the window has now, each with the part that registered it: the active
/// parts' in the order they are registered, then the window's own.
fn actions(app: &App) -> impl Iterator<Item = (&'static str, &'static Action<App>)> {
    let parts = shell::parts(app).active();
    let of_parts = parts.flat_map(|part| part.commands.iter().map(|action| (part.name, action)));
    of_parts.chain(browser::ACTIONS.iter().map(|action| (WINDOW, action)))
}

/// The window's own commands, for the check that no part registers one of them again.
pub(crate) fn own() -> &'static [Action<App>] {
    browser::ACTIONS
}

/// The command a menu row or a button names.
fn find(app: &App, id: &str) -> Option<&'static Action<App>> {
    let mut actions = actions(app).map(|(_, action)| action);
    actions.find(|action| action.id == id)
}

/// Whether the window has the command `id`: a menu row or a button that names it is not
/// inert.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the rows and the buttons are drawn whether or not one acts"
    )
)]
pub(crate) fn has(app: &App, id: &str) -> bool {
    find(app, id).is_some()
}

/// One stage's reading of one command's keys, for whatever lists keys and does not press
/// them: a palette, a label, a conflict check. A [`Reads::Commands`] says which catalog
/// command, map and tolerated modifiers each of its matchers reads (`Matcher::command`,
/// `map` and `ignored`).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "nothing on this interface lists keys yet")
)]
pub(crate) struct Bound {
    /// The command's name on this interface.
    pub(crate) id: &'static str,
    /// The catalog's command it is.
    pub(crate) command: Option<Command>,
    pub(crate) stage: Stage,
    pub(crate) reads: &'static Reads,
}

/// Every key the window's commands answer, in the order the parts register them.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "nothing on this interface lists keys yet")
)]
pub(crate) fn bound(app: &App) -> impl Iterator<Item = Bound> {
    actions(app).flat_map(|(_, action)| {
        action.keys.iter().map(|keys| Bound {
            id: action.id,
            command: action.command,
            stage: keys.stage,
            reads: &keys.reads,
        })
    })
}

/// One call, between the two halves of a handler that makes the browser act: the browser
/// takes what the parts changed first, and what it answers at once is applied after.
///
/// Applied only when the command ran. Rune commits for a key a handler took and for a `poll`
/// that says something changed, and for nothing else: what the browser had queued when a
/// command declined stays queued for the frame's `poll`, which gets the view run for it.
fn call(app: &mut App, action: &Action<App>, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    shell::publish(app);
    let outcome = (action.run)(app, call, tx);
    if outcome == Outcome::Ran {
        shell::settle(app, tx);
    }
    outcome
}

/// Runs the command `id` on `on`: a chosen menu row, a pressed header button, a palette's
/// row. `Declined` when the window has no such command.
pub(crate) fn run(app: &mut App, id: &str, on: On, tx: &mut Tx) -> Outcome {
    let Some(action) = find(app, id) else {
        tracing::debug!(id, "a command no part of this interface has");
        return Outcome::Declined;
    };
    call(app, action, &Call { on, press: None }, tx)
}

/// The commands that read `press` at `stage` where it lands, in the order they are asked.
fn reading(
    app: &App,
    press: &KeyPress,
    at: &Situation,
    stage: Stage,
) -> Vec<(&'static str, &'static Action<App>)> {
    let reads =
        |keys: &Keys<App>| keys.stage == stage && keys.reads.reads(press) && (keys.when)(app, at);
    actions(app)
        .filter(|(_, action)| action.keys.iter().any(reads))
        .collect()
}

/// A press and the stages it is offered where it was heard.
struct Offer<'a> {
    press: &'a KeyPress,
    at: &'a Situation,
    span: Span,
}

/// Offers a press to each stage of its span in the GTK chain's order, and within a stage to
/// each command that reads it there, until one runs or leaves it. Who has the keyboard
/// decides which stages are left ([`owners::reach`]).
fn walk(app: &mut App, offer: &Offer<'_>, tx: &mut Tx) -> Outcome {
    let Offer { press, at, span } = *offer;
    let key = Call {
        on: On::Focus,
        press: Some(press),
    };
    for stage in Stage::ALL.iter().filter(|stage| span.has(**stage)) {
        for (part, action) in reading(app, press, at, *stage) {
            let outcome = match reach(at, *stage, part) {
                Reach::Cut => Outcome::Declined,
                Reach::Taken => Outcome::Ran,
                Reach::Runs => call(app, action, &key, tx),
            };
            if outcome != Outcome::Declined {
                return outcome;
            }
        }
    }
    Outcome::Declined
}

/// The press `event` is where it lands, as every reader of a key on this interface reads
/// one: built from Rune's event with the settings as they are now, and through the aliases
/// with what owns typing there (a field, a menu, or type-to-search). A part that hears a key
/// in its own element and wants the catalog's answer for it calls this.
pub(crate) fn read(app: &App, event: &KeyEvent, at: &Situation, tx: &Tx) -> KeyPress {
    // Asked first, whoever owns typing: it takes the saved map as it is now, which the
    // aliases and every matcher after them read.
    let live = shell::live_keys(app);
    let typing = at.typing() || live.type_to_search_active();
    let press = press::read(event, press::Origin::of(tx));
    keys::rewrite(press, Context { typing })
}

/// One place's hearing of a key: true when a command took it. A key no command takes is not
/// the registry's: Rune goes on with it, and commits nothing for this handler.
fn hear(app: &mut App, event: &KeyEvent, heard: Heard, tx: &mut Tx) -> bool {
    let at = Situation::of(app, tx);
    let stamp = Stamp::of(event, &at, tx);
    let asked = &shell::live_keys(app).asked;
    let Some(span) = span(&at, heard, asked.early_for(&stamp)) else {
        return false;
    };
    asked.note(stamp, span);
    let press = read(app, event, &at, tx);
    let offer = Offer {
        press: &press,
        at: &at,
        span,
    };
    walk(app, &offer, tx) == Outcome::Ran
}

/// Puts the registry ahead of a view's own keys. A view calls it once on the collection its
/// component is built on, and on each column's where it has several: a command registered
/// at a stage of the GTK chain then reads its key before the list does, as GTK's window
/// reads it before its list, and the view's controllers keep every key no command takes.
///
/// A view that does not call it loses no command: the root element then asks every stage,
/// but after the view's own keys, so a command cannot take a key that view's controller
/// takes. A collection inside another that has the hook does not get one of its own: each
/// hook asks the early stages of every key it is given.
pub(crate) fn in_view(collection: Collection<App>) -> Collection<App> {
    in_view_with(collection, |_| {})
}

/// [`in_view`], telling `first` of each key before the registry reads it, for a view that has
/// to know the keyboard reached it: a single pane's first visit ends on any key, a bare
/// modifier or a repeat included, as GTK's `is_user_input` counts every key press in the pane.
///
/// `first` runs even when nothing then takes the key, and Rune then commits nothing, so
/// `first` must change nothing the view draws (ending the visit draws nothing).
pub(crate) fn in_view_with(
    collection: Collection<App>,
    first: impl Fn(&mut App) + 'static,
) -> Collection<App> {
    collection.before_keys(move |app: &mut App, event, tx| {
        first(app);
        hear(app, event, Heard::BeforeView, tx)
    })
}

/// The root element's keys: what the focused element and those around it left. Rune asks
/// here before its own focus navigation, its default button and the window.
pub(crate) fn from_element(app: &mut App, event: &KeyEvent, tx: &mut Tx) -> bool {
    hear(app, event, Heard::Element, tx)
}

/// The window's keys: every key when nothing has the keyboard, and what a text field or a
/// Rune modal passes on when a part says it holds the keyboard.
pub(crate) fn on_key(app: &mut App, event: &KeyEvent, tx: &mut Tx) -> bool {
    hear(app, event, Heard::Window, tx)
}

#[cfg(test)]
mod tests;
