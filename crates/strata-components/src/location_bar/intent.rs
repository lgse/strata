//! What the location bar reports. It owns no policy: parsing a typed path, listing folders,
//! opening a location, transfers and the clipboard are the host's, through these intents.

use super::data::CompletionQuery;
use crate::contract::dnd::{DropHover, Dropped};

/// How a crumb was opened. Strata opens a clicked or chosen ancestor selecting the folder the
/// user came from, and a spring-loaded one as a plain navigation.
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
    Open { id: String, via: Via },
    /// Open a crumb's folder in a new, active tab: a middle click on an ancestor.
    OpenInNewTab { id: String },
    /// Open a crumb's folder in a new window: Ctrl or Shift and a middle click.
    OpenInNewWindow { id: String },
    /// Open the typed location: Enter, the Navigate button, or a chosen suggestion. The bar is
    /// already back on its crumbs; a host that keeps the entry open (a location that mounts
    /// first) calls `LocationBarState::resume_edit` from this intent.
    Submit { text: String },
    /// The edit ended without navigating: Escape, the Cancel button, or a press outside the
    /// entry (which still does what it does). The host gives the keyboard back to its view.
    Cancelled,
    /// List suggestions for `query.text`, and answer with `LocationBarState::deliver`.
    Complete(CompletionQuery),
    /// Stop listing: the window lost focus, the entry closed or the host hid it. A late answer
    /// would be dropped anyway.
    CancelCompletion,
    /// The current crumb's Copy path button: copy the folder's path. The bar shows "Path
    /// copied" for two seconds itself.
    CopyPath,
    /// Files were dropped on an ancestor crumb: transfer them into its folder.
    DropInto { id: String, dropped: Dropped },
    /// A drag's hover over an ancestor crumb changed.
    DropHover { id: String, hover: DropHover },
}
