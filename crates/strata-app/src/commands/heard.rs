// SPDX-License-Identifier: MIT

//! Where a press is heard, and which stages each place asks, so that a press is walked once.
//!
//! Rune has no capture phase. It gives a key to the focused element's handlers and those
//! around it, innermost first, then to its own focus navigation, then to the window; from a
//! text field or inside a modal it calls no element's handler at all. A view's own keys stand
//! where the GTK chain's last stage does, so the registry asks ahead of them from a hook on
//! the view's collection, and after them from the window's root element.

use std::cell::RefCell;

use rune::{KeyEvent, PhysicalKeyCode, Tx};

use super::{Outcome, Situation, Stage};

/// Where Rune let the registry hear a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Heard {
    /// A view's collection, ahead of its controllers ([`super::in_view`]).
    BeforeView,
    /// The window's root element, which hears what the focused element and those around it
    /// left, before Rune's own focus navigation ([`super::from_element`]).
    Element,
    /// The window, which hears a key last, or first when nothing has the keyboard
    /// ([`super::on_key`]).
    Window,
}

/// The stages one place asks. The view's own keys stand where the chain's last stage does:
/// GTK's list hears a key the window's capture left, and the window's actions follow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Span {
    /// Every stage ahead of [`Stage::ItemNavigation`].
    BeforeView,
    /// [`Stage::ItemNavigation`] and what follows it.
    AfterView,
    /// [`Stage::AfterYield`] and what follows it, after an early stage left the press to the
    /// view (`Propagation::Proceed` in GTK's `handle_key`, followed by `after_yield`).
    AfterLeft,
    Whole,
}

impl Span {
    pub(super) fn has(self, stage: Stage) -> bool {
        match self {
            Self::BeforeView => stage < Stage::ItemNavigation,
            Self::AfterView => stage >= Stage::ItemNavigation,
            Self::AfterLeft => stage >= Stage::AfterYield,
            Self::Whole => true,
        }
    }
}

/// What a collection's hook did with the press it just asked the early stages for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Early {
    /// No early stage took or left it; the stages from [`Stage::ItemNavigation`] on remain.
    Declined,
    /// An early stage left it to the view ([`Outcome::Left`]); only [`Stage::AfterYield`] and
    /// [`Stage::Unclaimed`] remain.
    Left,
}

/// What a press heard at `heard` is offered, so that it is walked once wherever it lands.
/// `early` says a collection's hook has asked the early stages for this very press, and what
/// they answered.
///
/// - A collection with the hook is given the key: the hook asks the stages ahead of the
///   view's own keys, every time, and the root element the rest once the view has left the
///   key. The hook does not stand back for a press it seems to have asked for: the root
///   forgets a press only when it hears it, so after a key the view or a command took, the
///   same key again in the same frame looks the same, and it is a press of its own.
/// - No hook asked, whatever has the keyboard, a view without one included: the root element
///   asks every stage.
/// - A part holds the keyboard ([`super::Holds`]): Rune calls no element's handler from a
///   text field or from inside a modal, so the window asks, and the elements' handlers stand
///   back.
/// - Nothing has it: the window asks every stage.
///
/// A focused element Rune skips for element keys (a text field, an element that can no longer
/// take focus) whose part does not say it holds the keyboard is asked nowhere.
pub(super) fn span(at: &Situation, heard: Heard, early: Option<Early>) -> Option<Span> {
    let free = at.holds().is_none();
    match heard {
        Heard::BeforeView => free.then_some(Span::BeforeView),
        Heard::Element if !free => None,
        Heard::Element => Some(match early {
            Some(Early::Declined) => Span::AfterView,
            Some(Early::Left) => Span::AfterLeft,
            None => Span::Whole,
        }),
        Heard::Window => (!free || at.nowhere()).then_some(Span::Whole),
    }
}

/// One press, as the places that hear it can tell it from the next: Rune hands each of them
/// the same event at the same clock, with the keyboard where it was. A window's host gives
/// each event its own time of arrival too; a headless one gives none, so two presses of one
/// key in one frame look the same there, which is why the hook asks every time.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Stamp {
    clock: u64,
    arrived: Option<rune::time::Instant>,
    physical: Option<PhysicalKeyCode>,
    repeat: bool,
    event: KeyEvent,
    focused: Option<String>,
}

impl Stamp {
    pub(super) fn of(event: &KeyEvent, at: &Situation, tx: &Tx) -> Self {
        Self {
            clock: tx.clock().to_bits(),
            arrived: tx.input_time(),
            physical: tx.physical_key(),
            repeat: tx.key_repeat(),
            event: event.clone(),
            focused: at.focused().map(str::to_owned),
        }
    }
}

/// The press a collection's hook last asked the early stages for.
#[derive(Default)]
pub(super) struct Asked(RefCell<Option<(Stamp, Early)>>);

impl Asked {
    pub(super) fn early_for(&self, press: &Stamp) -> Option<Early> {
        self.0
            .borrow()
            .as_ref()
            .filter(|(stamp, _)| stamp == press)
            .map(|(_, early)| *early)
    }

    /// Notes what `span` asks of `press`: the early stages, or the rest, after which the
    /// press is done with.
    pub(super) fn note(&self, press: Stamp, span: Span, outcome: Outcome) {
        let early = match (span, outcome) {
            (Span::BeforeView, Outcome::Declined) => Some((press, Early::Declined)),
            (Span::BeforeView, Outcome::Left) => Some((press, Early::Left)),
            _ => None,
        };
        *self.0.borrow_mut() = early;
    }
}
