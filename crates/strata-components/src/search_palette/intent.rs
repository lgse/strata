// SPDX-License-Identifier: MIT

//! What the palette asks its host to do.

use super::data::PaletteQuery;
use crate::contract::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Via {
    /// Enter, or a new-tab chord.
    Key,
    /// A click.
    Click,
    /// A middle click.
    Middle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum By {
    /// Escape in the field.
    Escape,
    Dismiss,
    /// The host's toggle: Ctrl+K again, or Ctrl+Shift+K.
    Toggle,
    /// A result opened.
    Open,
    /// A result's folder opened.
    Reveal,
    /// The host closed it ([`super::PaletteState::close`]).
    Host,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PaletteIntent {
    Query(PaletteQuery),
    Open {
        id: String,
        new_tab: bool,
        via: Via,
    },
    Reveal {
        id: String,
    },
    ResultMenu {
        id: String,
        at: Point,
    },
    FieldMenu {
        at: Point,
    },
    /// The palette closed. After `Open` or `Reveal` too.
    Closed {
        by: By,
    },
}
