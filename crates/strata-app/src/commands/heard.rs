// SPDX-License-Identifier: MIT

use std::cell::RefCell;

use rune::{KeyEvent, PhysicalKeyCode, Tx};

use super::{Outcome, Situation, Stage};

/// Where Rune let the registry hear a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Heard {
    /// A view's collection, ahead of its controllers ([`super::in_view`]).
    BeforeView,
    Element,
    Window,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Span {
    /// Every stage ahead of [`Stage::ItemNavigation`].
    BeforeView,
    /// [`Stage::ItemNavigation`] and what follows it.
    AfterView,
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
    Left,
}

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

    pub(super) fn note(&self, press: Stamp, span: Span, outcome: Outcome) {
        let early = match (span, outcome) {
            (Span::BeforeView, Outcome::Declined) => Some((press, Early::Declined)),
            (Span::BeforeView, Outcome::Left) => Some((press, Early::Left)),
            _ => None,
        };
        *self.0.borrow_mut() = early;
    }
}
