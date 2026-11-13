// SPDX-License-Identifier: MIT

//! Who has the keyboard, and what that leaves of the GTK window's stages.
//!
//! The GTK dispatcher asks, stage by stage, whether a modal layer, a menu, an inline edit or
//! the location bar owns the press, and ends its chain there (`input_owner`,
//! `inline_editing` and `text_input` under `src/ui/window/keyboard/`). A part says which of
//! those it is ([`Holds`]), and [`reach`] holds the rule for each in one place.

use rune::Tx;
use strata_desktop::keys::{KeyPress, Modifiers};

use super::Stage;
use crate::{App, shell};

/// Which of the GTK window's owners of input a part is while it has the keyboard, the more
/// of the chain it keeps the later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Holds {
    /// A plain popover, such as the Icons header's thumbnail size: GTK's `gtk::Popover`, which
    /// `native_menu_owns_input` does not count as owning input (`input_owner.rs:116`), so
    /// window commands act while it is open. What GTK's dispatcher still does for focus in a
    /// popover ([`reach`]): the default Tab navigation does not run (`focus_in_popover`,
    /// `src/ui/window/keyboard/tenxer.rs:158`); a key without Ctrl or Alt ends the chain at
    /// focus navigation (`popover_navigation`, `focus.rs:112`), so dismissal and item
    /// navigation never hear it; and typing is the popover's for the aliases
    /// ([`Situation::typing`], `in_popover` in `context_of`). 10xer's own checks
    /// (`focus_keeps_typing`, `tenxer_pane_focus`) ask [`Situation::popover`]. A part answers
    /// it while focus is inside its popover, and closes the popover where GTK's goes away with
    /// its pane: a navigation that rebuilds the pane, or another view.
    Popover,
    /// A text field none of the others is. Every stage runs, and each command's guard asks
    /// [`Situation::text`], as GTK's stages ask `text_has_focus()`.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "no part has such a text field yet")
    )]
    Field,
    /// The filter's field, which three of GTK's stages treat as no text field
    /// ([`Situation::filter`]).
    Filter,
    /// The location bar: after `text_input` every key is the bar's
    /// (`src/ui/window/keyboard/commands.rs:177`).
    Location,
    /// An inline edit: after `inline_editing` every key is the field's (`commands.rs:85`).
    Edit,
    /// A menu: `input_owner` gives it every key (`input_owner.rs:21`), and what follows a
    /// key the chain left still runs.
    Menu,
    /// A modal layer: `input_owner` gives it every key (`input_owner.rs:30`), the tab
    /// strip's commands are blocked (`composition/tabs.rs:760`), and a window action takes
    /// its key without running (`composition.rs:239`). `field` says whether a text field of
    /// the layer has the keyboard (a palette's query, a dialog's name): typing is then the
    /// field's, as under any focused editable (`context_of` in `src/ui/keys/alias.rs`).
    Modal { field: bool },
}

impl Holds {
    /// The last stage of the chain that still runs: after it the key is the owner's.
    const fn last(self) -> Stage {
        match self {
            Self::Popover => Stage::Unclaimed,
            Self::Modal { .. } | Self::Menu => Stage::ZoomShortcut,
            Self::Edit => Stage::InlineEditing,
            Self::Location => Stage::TextInput,
            Self::Filter | Self::Field => Stage::ItemNavigation,
        }
    }
}

/// Where a press lands: what a command's guard may ask.
pub(crate) struct Situation {
    /// The owner among the active parts that keeps the most, and the part that is it.
    holds: Option<(Holds, &'static str)>,
    /// The key of the element that has the keyboard.
    focused: Option<String>,
    /// The key of the shown view's element that takes the keyboard.
    view: Option<&'static str>,
}

impl Situation {
    /// Who has the keyboard as the press arrives.
    pub(crate) fn of(app: &App, tx: &Tx) -> Self {
        let parts = shell::parts(app);
        let owners = parts
            .active()
            .filter_map(|part| Some(((part.keyboard)(app)?, part.name)));
        Self {
            holds: owners.max_by_key(|(holds, _)| *holds),
            focused: tx.focused().map(str::to_owned),
            view: shell::view_focus(app),
        }
    }

    /// Which owner has the keyboard, if a part says it is one.
    pub(crate) fn holds(&self) -> Option<Holds> {
        self.holds.map(|(holds, _)| holds)
    }

    /// Whether a text field has the keyboard: GTK's `text_has_focus()`.
    pub(crate) fn text(&self) -> bool {
        matches!(
            self.holds(),
            Some(
                Holds::Field
                    | Holds::Filter
                    | Holds::Location
                    | Holds::Edit
                    | Holds::Modal { field: true }
            )
        )
    }

    /// Whether a plain popover has it: GTK's `focus_in_popover()` ([`Holds::Popover`]).
    #[expect(
        dead_code,
        reason = "10xer's stages ask it (`focus_keeps_typing`, `tenxer_pane_focus`): A11's"
    )]
    pub(crate) fn popover(&self) -> bool {
        self.holds() == Some(Holds::Popover)
    }

    /// Whether the filter's field has it: GTK's `filter_has_focus()`.
    pub(crate) fn filter(&self) -> bool {
        self.holds() == Some(Holds::Filter)
    }

    /// The key of the element that has it.
    pub(super) fn focused(&self) -> Option<&str> {
        self.focused.as_deref()
    }

    /// Whether no element has it.
    pub(super) fn nowhere(&self) -> bool {
        self.focused.is_none()
    }

    /// Whether the shown view's listing has it: GTK's `item_view_has_focus()`.
    pub(crate) fn in_view(&self) -> bool {
        self.focused.is_some() && self.focused.as_deref() == self.view
    }

    /// Whether an element of the part whose keys start with `part` has it (`slots`, "Rules
    /// for a part"): GTK's `sidebar.contains(focused)`. `part` is the name alone, and the key
    /// is that name or goes on from it at a separator: `tabs-strip` is not within `tab`.
    pub(crate) fn within(&self, part: &str) -> bool {
        let rest = self
            .focused
            .as_deref()
            .and_then(|key| key.strip_prefix(part));
        rest.is_some_and(|rest| !rest.starts_with(char::is_alphanumeric))
    }

    /// What owns typing where the press lands, for the aliases (`context_of` in
    /// `src/ui/keys/alias.rs`): a text field, a modal layer's among them, or a popover.
    pub(super) fn typing(&self) -> bool {
        self.text() || matches!(self.holds(), Some(Holds::Menu | Holds::Popover))
    }
}

/// What `popover_navigation` lets on (`event.without(CONTROL_MASK | ALT_MASK)`).
const CTRL_OR_ALT: Modifiers = Modifiers::CONTROL_MASK.union(Modifiers::ALT_MASK);

/// What a stage is to a press, given who has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reach {
    /// The stage reads the press.
    Runs,
    /// A command that reads the press there takes it and does not run: a window action under
    /// a modal layer or an inline edit, whose guard declines after GTK has taken the key.
    Taken,
    /// The stage does not run: the key is the owner's.
    Cut,
}

/// GTK's rule for `stage` while the owner in `at` has the keyboard, for a command of the
/// part named `part` and the press `press`.
pub(super) fn reach(at: &Situation, stage: Stage, part: &str, press: &KeyPress) -> Reach {
    let Some((holds, owner)) = at.holds else {
        return Reach::Runs;
    };
    let runs = match stage {
        // What the dispatcher does on every press, and what the focused widget left.
        Stage::Entry | Stage::Unclaimed => true,
        Stage::TabStrip => !matches!(holds, Holds::Modal { .. }),
        // `default_tab_navigation` declines while focus is in a popover (`focus.rs:44`).
        Stage::DefaultTabNavigation if holds == Holds::Popover => false,
        // `popover_navigation` (`focus.rs:112`) ends the chain for a key without Ctrl or Alt
        // while focus is in a popover: the rest of `focus_navigation`, `dismissal` and
        // `item_navigation` never hear it, so Delete on its slider trashes nothing.
        Stage::FocusNavigation | Stage::Dismissal | Stage::ItemNavigation
            if holds == Holds::Popover && !press.modifiers().intersects(CTRL_OR_ALT) =>
        {
            false
        }
        // Follows every answer but "taken" (`keyboard.rs:147`). The guard of a window action
        // lets through the action that owns the layer on top, so that it can close it.
        Stage::AfterYield => match holds {
            Holds::Modal { .. } if part != owner => return Reach::Taken,
            Holds::Edit => return Reach::Taken,
            _ => true,
        },
        chain => chain <= holds.last(),
    };
    if runs { Reach::Runs } else { Reach::Cut }
}
