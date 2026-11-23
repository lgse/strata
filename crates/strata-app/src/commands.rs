// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};

use rune::{KeyEvent, Tx};
use strata_components::collection::element::Collection;
use strata_core::model::Location;
use strata_desktop::{
    keys::{self, Active, Command, Context, Digits, KeyPress, Keymap, Matcher, conflicts::Policy},
    settings::{KeymapOverrides, Preferences, report_keymap_conflicts, resolve_keymap},
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
#[cfg(any(test, feature = "qa"))]
pub(crate) use press::{Origin, XKey};

macro_rules! stages {
    ($($(#[$note:meta])* $stage:ident,)*) => {
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
    TabStrip,
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
    Commands(&'static [Matcher]),
    Digits(Digits),
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum On {
    /// Where the keyboard is: a key, a header button, a palette.
    Focus,
    /// The row a menu was opened on.
    Row(At),
    /// A column's folder: a background menu.
    Column(usize),
    Place(Location),
}

/// One call of a command.
pub(crate) struct Call<'a> {
    pub(crate) on: On,
    pub(crate) press: Option<&'a KeyPress>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// It ran, and the press is taken.
    Ran,
    Declined,
    /// The focused element's: nothing else is asked, and Rune is told the key is not taken.
    Left,
}

/// One command, by every route.
pub(crate) struct Action<S: 'static> {
    /// The name a menu row or a button asks for it by: the row's id, the button's.
    pub(crate) id: &'static str,
    /// The catalog's command this is, when the catalog has it.
    pub(crate) command: Option<Command>,
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

pub(crate) struct Live {
    tenxer: Cell<bool>,
    typing: Cell<bool>,
    keymap: RefCell<Rc<Keymap>>,
    /// The saved overrides `keymap` was resolved from.
    resolved: RefCell<KeymapOverrides>,
    /// The press a view's collection last asked the early stages for ([`heard`]).
    asked: Asked,
    #[cfg(any(test, feature = "qa"))]
    qa_origin: Cell<Option<press::Origin>>,
}

impl Live {
    pub(crate) fn registered(settings: &Preferences, report_to: Option<&Path>) -> Rc<Self> {
        let resolved = Rc::new(resolve_keymap(&settings.keymap, report_to));
        if let Some(file) = report_to {
            report_keymap_conflicts(
                &resolved,
                file,
                Policy {
                    type_to_search: settings.type_to_search,
                },
            );
        }
        let live = Rc::new(Self {
            tenxer: Cell::new(false),
            typing: Cell::new(false),
            keymap: RefCell::new(resolved),
            resolved: RefCell::new(settings.keymap.clone()),
            asked: Asked::default(),
            #[cfg(any(test, feature = "qa"))]
            qa_origin: Cell::new(None),
        });
        live.follow(settings);
        let weak = Rc::downgrade(&live);
        let source: std::rc::Weak<dyn Active> = weak;
        keys::set_active(source);
        live
    }

    pub(crate) fn follow(&self, settings: &Preferences) {
        self.tenxer.set(settings.tenxer_mode);
        self.typing
            .set(settings.type_to_search && !settings.tenxer_mode);
        if *self.resolved.borrow() != settings.keymap {
            self.keymap
                .replace(Rc::new(resolve_keymap(&settings.keymap, None)));
            self.resolved.replace(settings.keymap.clone());
        }
    }

    #[cfg(any(test, feature = "qa"))]
    pub(crate) fn set_qa_origin(&self, origin: Option<press::Origin>) {
        self.qa_origin.set(origin);
    }

    #[cfg(any(test, feature = "qa"))]
    fn qa_origin(&self) -> Option<press::Origin> {
        self.qa_origin.get()
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

pub(crate) fn has(app: &App, id: &str) -> bool {
    find(app, id).is_some()
}

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

fn call(app: &mut App, action: &Action<App>, call: &Call<'_>, tx: &mut Tx) -> Outcome {
    shell::publish(app);
    let outcome = (action.run)(app, call, tx);
    if outcome == Outcome::Ran {
        shell::settle(app, tx);
        wake_glib();
    }
    outcome
}

fn wake_glib() {
    glib::MainContext::default().wakeup();
}

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
        .filter(|(_, action)| !crate::chooser::declines_key(app, action, stage))
        .collect()
}

/// A press and the stages it is offered where it was heard.
struct Offer<'a> {
    press: &'a KeyPress,
    at: &'a Situation,
    span: Span,
}

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

pub(crate) fn read(app: &App, event: &KeyEvent, at: &Situation, tx: &Tx) -> KeyPress {
    let live = shell::live_keys(app);
    let typing = at.typing() || live.type_to_search_active();
    let press = press::read(event, origin_of(live, tx));
    keys::rewrite(press, Context { typing })
}

pub(crate) fn read_pressed(app: &App, event: &KeyEvent, tx: &Tx) -> KeyPress {
    press::read(event, origin_of(shell::live_keys(app), tx))
}

fn origin_of(live: &Live, tx: &Tx) -> press::Origin {
    let origin = press::Origin::of(tx);
    #[cfg(any(test, feature = "qa"))]
    if origin.physical.is_none()
        && let Some(qa) = live.qa_origin()
    {
        return qa;
    }
    #[cfg(not(any(test, feature = "qa")))]
    let _ = live;
    origin
}

fn hear(app: &mut App, event: &KeyEvent, heard: Heard, tx: &mut Tx) -> bool {
    let at = Situation::of(app, tx);
    let stamp = Stamp::of(event, &at, tx);
    let early = shell::live_keys(app).asked.early_for(&stamp);
    let Some(span) = span(&at, heard, early) else {
        return false;
    };
    let press = read(app, event, &at, tx);
    let offer = Offer {
        press: &press,
        at: &at,
        span,
    };
    let outcome = walk(app, &offer, tx);
    shell::live_keys(app).asked.note(stamp, span, outcome);
    outcome == Outcome::Ran
}

pub(crate) fn in_view(collection: Collection<App>) -> Collection<App> {
    in_view_with(collection, |_| {})
}

pub(crate) fn in_view_with(
    collection: Collection<App>,
    first: impl Fn(&mut App) + 'static,
) -> Collection<App> {
    collection.before_keys(move |app: &mut App, event, tx| {
        first(app);
        hear(app, event, Heard::BeforeView, tx)
    })
}

fn dispatch_with_preview_wake(app: &mut App, event: &KeyEvent, heard: Heard, tx: &mut Tx) -> bool {
    let was_in_preview = crate::preview::owns_focus(app);
    let handled = hear(app, event, heard, tx);
    if was_in_preview
        || crate::preview::owns_focus(app)
        || (!handled && event.key == rune::Key::Tab && crate::preview::is_open(app))
    {
        tx.wake_after(std::time::Duration::ZERO);
    }
    handled
}

pub(crate) fn from_element(app: &mut App, event: &KeyEvent, tx: &mut Tx) -> bool {
    dispatch_with_preview_wake(app, event, Heard::Element, tx)
}

pub(crate) fn on_key(app: &mut App, event: &KeyEvent, tx: &mut Tx) -> bool {
    dispatch_with_preview_wake(app, event, Heard::Window, tx)
}

#[cfg(test)]
mod tests;
