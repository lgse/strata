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
pub(crate) mod press;

/// Where the GTK window reads a key: the tab strip's controller, the dispatcher's entry, the
/// stages of its chain in their order (`handle_key`, `src/ui/window/keyboard.rs:421`), what
/// follows a key the chain left (`after_yield`, line 531), and the Settings key, which hears
/// what the focused widget did not take (`composition/settings.rs:71`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Stage {
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

impl Stage {
    /// Every stage, in the order a press is offered to them.
    pub(crate) const ALL: [Self; 25] = [
        Self::TabStrip,
        Self::Entry,
        Self::RenameTab,
        Self::ZoomShortcut,
        Self::InputOwner,
        Self::TenxerKeys,
        Self::WindowCommands,
        Self::DirectCommands,
        Self::DefaultTabNavigation,
        Self::InlineEditing,
        Self::FilterAndLocationCommands,
        Self::VideoControls,
        Self::SidebarCommands,
        Self::ContextMenuCommand,
        Self::PropertiesCommand,
        Self::SearchResultKeys,
        Self::PreviewPasswordEscape,
        Self::TextInput,
        Self::FileCommands,
        Self::PreviewNavigation,
        Self::FocusNavigation,
        Self::Dismissal,
        Self::ItemNavigation,
        Self::AfterYield,
        Self::Unclaimed,
    ];
}

/// What a stage matches a press against.
pub(crate) enum Reads {
    /// Catalog commands, with `in_map` and `ignoring` as the GTK stage has them.
    Commands(&'static [Matcher]),
    /// A digit range of the catalog, such as the tab numbers. Which digit is the command's
    /// to read from the call's press.
    #[expect(dead_code, reason = "no registered command reads a digit range yet")]
    Digits(Digits),
    /// What neither says: a key that types, for type-to-search. Nothing can list it.
    #[expect(dead_code, reason = "no registered command reads a press that way yet")]
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
pub(crate) struct Keys {
    pub(crate) stage: Stage,
    pub(crate) reads: Reads,
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
    #[expect(dead_code, reason = "no registered command reads its press yet")]
    pub(crate) press: Option<&'a KeyPress>,
}

/// What a command answers, which is what a stage of the GTK chain answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// It ran, and the press is taken.
    Ran,
    /// Not for it after all: the next command that reads the press is asked.
    Declined,
    /// The focused element's: nothing else is asked, and Rune is told the key is not taken.
    #[expect(dead_code, reason = "no registered command leaves a key yet")]
    Left,
}

/// One command, by every route.
pub(crate) struct Action<S: 'static> {
    /// The name a menu row or a button asks for it by: the row's id, the button's.
    pub(crate) id: &'static str,
    /// The catalog's command this is, when the catalog has it.
    pub(crate) command: Option<Command>,
    /// Where the GTK window's stages read its keys. Empty for a command no key runs.
    pub(crate) keys: &'static [Keys],
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

    pub(crate) const fn keys(mut self, keys: &'static [Keys]) -> Self {
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

/// Every command an active part of the window has.
fn actions(_app: &App) -> impl Iterator<Item = &'static Action<App>> {
    browser::ACTIONS.iter()
}

/// The command a menu row or a button names.
fn find(app: &App, id: &str) -> Option<&'static Action<App>> {
    actions(app).find(|action| action.id == id)
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
    actions(app).flat_map(|action| {
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
fn call(app: &mut App, action: &Action<App>, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    shell::publish(app);
    let outcome = (action.run)(app, call, tx);
    shell::settle(app, tx);
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

/// Offers `press` to each stage in the GTK chain's order, and within a stage to each command
/// that reads it there, until one runs or leaves it.
fn walk(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Outcome {
    let key = Call {
        on: On::Focus,
        press: Some(press),
    };
    for stage in Stage::ALL {
        let reading = actions(app).filter(|action| {
            let mut keys = action.keys.iter();
            keys.any(|keys| keys.stage == stage && keys.reads.reads(press))
        });
        for action in reading.collect::<Vec<_>>() {
            match call(app, action, &key, tx) {
                Outcome::Declined => {}
                outcome => return outcome,
            }
        }
    }
    Outcome::Declined
}

/// The window's keys: what the focused element passed on. While a part's field has the
/// keyboard they wait, as the GTK window's do: a key the field passes on is not a shortcut.
pub(crate) fn on_key(app: &mut App, event: &KeyEvent, tx: &mut Tx) -> bool {
    let waiting = shell::holds_keyboard(app);
    // What owns typing where the press lands: a field, or type-to-search (`context_of` in
    // `src/ui/keys/alias.rs`). A plain key that stands for an arrow is typing there.
    let typing = waiting || shell::live_keys(app).type_to_search_active();
    let press = press::read(event, press::Origin::of(tx));
    let press = keys::rewrite(press, Context { typing });
    if !waiting && walk(app, &press, tx) == Outcome::Ran {
        return true;
    }
    // Nothing the view reads changed: no commit for a key that is not the window's.
    tx.live_only();
    false
}

#[cfg(test)]
mod tests;
