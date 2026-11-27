// SPDX-License-Identifier: MIT

use super::data::CompletionQuery;
use crate::contract::dnd::{DropHover, Dropped};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// A press (pointer, Return or Space) on an ancestor crumb.
    Click,
    /// The hierarchy menu.
    Menu,
    /// A file drag held over the crumb for the spring delay.
    Spring,
}

/// What the bar asks the host to do.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LocationIntent {
    /// Open a crumb's folder.
    Open {
        id: String,
        via: Via,
    },
    /// Open a crumb's folder in a new, active tab: a middle click on an ancestor.
    OpenInNewTab {
        id: String,
    },
    /// Open a crumb's folder in a new window: Ctrl or Shift and a middle click.
    OpenInNewWindow {
        id: String,
    },
    Submit {
        text: String,
    },
    Cancelled,
    /// List suggestions for `query.text`, and answer with `LocationBarState::deliver`.
    Complete(CompletionQuery),
    CancelCompletion,
    CopyPath,
    /// Files were dropped on an ancestor crumb: transfer them into its folder.
    DropInto {
        id: String,
        dropped: Dropped,
    },
    /// A drag's hover over an ancestor crumb changed.
    DropHover {
        id: String,
        hover: DropHover,
    },
}
