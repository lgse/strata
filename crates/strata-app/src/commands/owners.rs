// SPDX-License-Identifier: MIT

use rune::Tx;

use super::Stage;
use crate::{App, shell};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Holds {
    Field,
    Filter,
    Location,
    /// An inline edit: after `inline_editing` every key is the field's (`commands.rs:85`).
    Edit,
    Prompt,
    Menu,
    Modal {
        field: bool,
    },
    Popover,
}

impl Holds {
    /// The last stage of the chain that still runs: after it the key is the owner's.
    const fn last(self) -> Stage {
        match self {
            // Never read: `reach` cuts every stage for a popover first.
            Self::Popover => Stage::TabStrip,
            Self::Modal { .. } | Self::Menu => Stage::ZoomShortcut,
            Self::Prompt => Stage::TenxerKeys,
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
    selected: Option<String>,
    /// The key of the shown view's element that takes the keyboard.
    view: Option<&'static str>,
}

fn selected_path(tx: &Tx) -> Option<String> {
    let selection = tx.text_selection()?;
    selection
        .texts()
        .iter()
        .find(|text| text.is_selected())
        .map(|text| text.path().to_owned())
}

fn key_within_part(key: &str, part: &str) -> bool {
    key.strip_prefix(part)
        .is_some_and(|rest| !rest.starts_with(char::is_alphanumeric))
}

impl Situation {
    /// Who has the keyboard as the press arrives.
    pub(crate) fn of(app: &App, tx: &Tx) -> Self {
        let parts = shell::parts(app);
        let owners = parts
            .active()
            .filter_map(|part| Some(((part.keyboard)(app)?, part.name)));
        let focused = tx.focused().map(str::to_owned);
        let selected = if focused.is_none() {
            selected_path(tx)
        } else {
            None
        };
        Self {
            holds: owners.max_by_key(|(holds, _)| *holds),
            focused,
            selected,
            view: shell::view_focus(app),
        }
    }

    /// Which owner has the keyboard, if a part says it is one.
    pub(crate) fn holds(&self) -> Option<Holds> {
        self.holds.map(|(holds, _)| holds)
    }

    pub(crate) fn text(&self) -> bool {
        matches!(
            self.holds(),
            Some(
                Holds::Field
                    | Holds::Filter
                    | Holds::Location
                    | Holds::Edit
                    | Holds::Prompt
                    | Holds::Modal { field: true }
            )
        )
    }

    pub(crate) fn filter(&self) -> bool {
        self.holds() == Some(Holds::Filter)
    }

    /// The key of the element that has it.
    pub(crate) fn focused(&self) -> Option<&str> {
        self.focused.as_deref()
    }

    /// Whether no element has it.
    pub(super) fn nowhere(&self) -> bool {
        self.focused.is_none()
    }

    pub(crate) fn in_view(&self) -> bool {
        self.focused.is_some() && self.focused.as_deref() == self.view
    }

    pub(crate) fn within(&self, part: &str) -> bool {
        if let Some(key) = self.focused.as_deref() {
            return key_within_part(key, part);
        }
        self.selected
            .as_deref()
            .is_some_and(|path| path.split('/').any(|seg| key_within_part(seg, part)))
    }

    pub(crate) fn in_top_bar(&self) -> bool {
        self.within("header") || self.within("location")
    }

    pub(super) fn typing(&self) -> bool {
        self.text() || matches!(self.holds(), Some(Holds::Menu | Holds::Popover))
    }
}

/// What a stage is to a press, given who has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reach {
    /// The stage reads the press.
    Runs,
    Taken,
    /// The stage does not run: the key is the owner's.
    Cut,
}

pub(super) fn reach(at: &Situation, stage: Stage, part: &str) -> Reach {
    let Some((holds, owner)) = at.holds else {
        return Reach::Runs;
    };
    // A popover takes every key: no stage hears it ([`Holds::Popover`]).
    if holds == Holds::Popover {
        return Reach::Cut;
    }
    let runs = match stage {
        // What the dispatcher does on every press, and what the focused widget left.
        Stage::Entry | Stage::Unclaimed => true,
        Stage::TabStrip => !matches!(holds, Holds::Modal { .. }),
        Stage::AfterYield => match holds {
            Holds::Modal { .. } if part != owner => return Reach::Taken,
            Holds::Edit | Holds::Prompt => return Reach::Taken,
            _ => true,
        },
        chain => chain <= holds.last(),
    };
    if runs { Reach::Runs } else { Reach::Cut }
}
