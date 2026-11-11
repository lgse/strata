//! What the palette asks its host to do.

use super::data::PaletteQuery;
use crate::contract::Point;

/// How a result was opened.
///
/// Used by: hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Via {
    /// Enter, or a new-tab chord.
    Key,
    /// A click.
    Click,
    /// A middle click.
    Middle,
}

/// What closed the palette.
///
/// Used by: hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum By {
    /// Escape in the field.
    Escape,
    /// A click outside the panel, or Escape while the field takes no keys (Rune's modal
    /// dismissal, which can't tell them apart).
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

/// Everything the palette asks of its host. Each comes with a `Tx` through the palette's
/// `on_intent`, or from [`super::PaletteState`]'s methods in a [`super::Polled`].
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PaletteIntent {
    /// Run this search and answer with [`super::PaletteState::deliver`]. It supersedes every
    /// earlier one: their answers are dropped, so the host may cancel them.
    Query(PaletteQuery),
    /// Open the result `id`; the palette has closed. `new_tab` for a new-tab chord or a middle
    /// click.
    Open { id: String, new_tab: bool, via: Via },
    /// Open the result's containing folder with it selected (Alt+Enter, or the host's menu
    /// calling [`super::PaletteState::reveal`]); the palette has closed.
    Reveal { id: String },
    /// A right-click on result `id`, now highlighted, at window point `at`: the host opens its
    /// menu there (Strata's "Open containing folder").
    ResultMenu { id: String, at: Point },
    /// A right-click, the Menu key or Shift+F10 on the field, at window point `at`: the host
    /// opens C21's field menu.
    FieldMenu { at: Point },
    /// The palette closed. After `Open` or `Reveal` too.
    Closed { by: By },
}
