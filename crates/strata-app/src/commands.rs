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
//! `docs/rune/parity/packages/B3.md`, "Design", has the oracle's stages line by line and the
//! parts of this that later pull requests add: a part's own table, what a part holds of the
//! keyboard, and the entry points ahead of a view's and Rune's own keys.

use std::{cell::Cell, rc::Rc};

use rune::{KeyEvent, Tx};
use strata_desktop::{
    keys::{self, Active, Command, Context, Digits, KeyPress, Keymap, Matcher},
    settings::Preferences,
};

use crate::{App, shell, slots::At};

mod browser;
mod owners;
pub(crate) mod press;

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
    /// What neither says: a key that types, for type-to-search. Nothing can list it.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "no registered command reads a press that way yet")
    )]
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
    /// would then act on top of it.
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

/// The settings the catalog reads at each press and each label: which map is in use, and
/// whether plain keys type into a search. The catalog asks whoever an interface registered
/// for its thread (`keys::set_active`), and holds it weakly, so the window keeps this.
///
/// The keymap is the shipped one: a saved keymap cannot be read on this interface until the
/// shared crate reads accelerator strings without GDK.
pub(crate) struct Live {
    tenxer: Cell<bool>,
    typing: Cell<bool>,
    keymap: Rc<Keymap>,
}

impl Live {
    /// The window's source, registered for this thread and holding `settings`' values.
    pub(crate) fn registered(settings: &Preferences) -> Rc<Self> {
        let live = Rc::new(Self {
            tenxer: Cell::new(false),
            typing: Cell::new(false),
            keymap: Rc::new(Keymap::defaults()),
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
        self.keymap.clone()
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

/// What a call did: the command's answer, and whether the view has to run for what the
/// browser said while it ran.
struct Called {
    outcome: Outcome,
    changed: bool,
}

/// One call, between the two halves of a handler that makes the browser act: the browser
/// takes what the parts changed first, and what it answers at once is applied after.
fn call(app: &mut App, action: &Action<App>, call: &Call<'_>, tx: &mut Tx) -> Called {
    shell::publish(app);
    let outcome = (action.run)(app, call, tx);
    let changed = shell::settle(app, tx);
    Called { outcome, changed }
}

/// Runs the command `id` on `on`: a chosen menu row, a pressed header button, a palette's
/// row. `Declined` when the window has no such command.
pub(crate) fn run(app: &mut App, id: &str, on: On, tx: &mut Tx) -> Outcome {
    let Some(action) = find(app, id) else {
        tracing::debug!(id, "a command no part of this interface has");
        return Outcome::Declined;
    };
    call(app, action, &Call { on, press: None }, tx).outcome
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

/// Offers `press` to each stage in the GTK chain's order, and within a stage to each command
/// that reads it there, until one runs or leaves it. Who has the keyboard decides which
/// stages are left ([`owners::reach`]).
fn walk(app: &mut App, press: &KeyPress, at: &Situation, tx: &mut Tx) -> Called {
    let key = Call {
        on: On::Focus,
        press: Some(press),
    };
    let mut changed = false;
    for stage in Stage::ALL {
        for (part, action) in reading(app, press, at, *stage) {
            let outcome = match reach(at, *stage, part) {
                Reach::Cut => Outcome::Declined,
                Reach::Taken => Outcome::Ran,
                Reach::Runs => {
                    let called = call(app, action, &key, tx);
                    changed |= called.changed;
                    called.outcome
                }
            };
            if outcome != Outcome::Declined {
                return Called { outcome, changed };
            }
        }
    }
    Called {
        outcome: Outcome::Declined,
        changed,
    }
}

/// The press `event` is where it lands, as every reader of a key on this interface reads
/// one: built from Rune's event with the settings as they are now, and through the aliases
/// with what owns typing there (a field, a menu, or type-to-search). A part that hears a key
/// in its own element and wants the catalog's answer for it calls this.
pub(crate) fn read(app: &App, event: &KeyEvent, at: &Situation, tx: &Tx) -> KeyPress {
    let typing = at.typing() || shell::live_keys(app).type_to_search_active();
    let press = press::read(event, press::Origin::of(tx));
    keys::rewrite(press, Context { typing })
}

/// The window's keys: what the focused element passed on, or every key when nothing has the
/// keyboard.
pub(crate) fn on_key(app: &mut App, event: &KeyEvent, tx: &mut Tx) -> bool {
    let at = Situation::of(app, tx);
    let press = read(app, event, &at, tx);
    let walked = walk(app, &press, &at, tx);
    if walked.outcome != Outcome::Ran && !walked.changed {
        // Nothing the view reads changed: no commit for a key that is not the window's. A
        // command that declined after the browser answered it still gets its frame.
        tx.live_only();
    }
    walked.outcome == Outcome::Ran
}

#[cfg(test)]
mod tests;
