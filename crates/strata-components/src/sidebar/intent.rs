//! What the sidebar reports. It owns no policy: opening a place, pinning, transfers, ejecting
//! and saving preferences are the host's, through these intents.

use rune::PointerPress;

use super::data::ItemId;
use super::state::Step;
use crate::contract::dnd::{DropHover, Dropped};

/// How a row was activated. Strata opens a place with its first item selected from the
/// keyboard, and with nothing selected from a click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Pointer,
    Keyboard,
}

/// A collapsible section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Pinned,
    Devices,
}

impl Section {
    /// Its heading, which is also its accessible name and its rail tooltip.
    pub fn title(self) -> &'static str {
        match self {
            Section::Pinned => "PINNED",
            Section::Devices => "DEVICES",
        }
    }

    pub(crate) fn id(self) -> &'static str {
        match self {
            Section::Pinned => "pinned",
            Section::Devices => "devices",
        }
    }
}

/// An insertion gap in PINNED: its place in the full pin list (0 before the first pin, the
/// pin count after the last), and the pins on either side when the view was built. Another
/// window may unpin a neighbour before the drop lands; the host then inserts beside the
/// neighbour that is left, and refuses the drop when both are gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    pub index: usize,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// A place in the sidebar a drag can be over. Only one shows an indicator at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropSite {
    /// The sidebar outside any target: the drag is over the sidebar.
    Background,
    /// A pin gap, by its full-list index.
    Gap(usize),
    /// A row's body.
    Item(ItemId),
}

/// What the sidebar asks the host to do.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SidebarIntent {
    /// Open a row's location.
    Open { item: ItemId, via: Via },
    /// Open a row's location in a new tab (a middle click). Never sent for an unmounted
    /// device, nor when [`super::PlacesSidebar::open_in_new_tab`] is off.
    OpenInNewTab { item: ItemId },
    /// A secondary click or the Menu key on a row, or on the sidebar outside any row
    /// (`item` is `None`, which a heading opens too): the host opens its menu (C21) at
    /// `point`.
    ContextMenu {
        item: Option<ItemId>,
        point: PointerPress,
    },
    /// A built-in place was dragged from `from` to `to` in the places block.
    MovePlace { from: usize, to: usize },
    /// The pin `id` was dragged from `from` to `to` in the full pin list.
    MovePin { id: String, from: usize, to: usize },
    /// A drag reached a pin gap: the host checks its items are folders, keys the result by
    /// `serial` and writes it to [`super::SidebarState::pin_check`]. `uris` is `None` when the
    /// binder reads them only at the drop (Rune's).
    PinHover {
        serial: u64,
        uris: Option<Vec<String>>,
        local: bool,
    },
    /// Folders were dropped into a pin gap. Never a transfer: the host pins the folders among
    /// the dropped items, or nothing.
    PinDrop { gap: Gap, dropped: Dropped },
    /// Files were dropped on a row: places, pins, mounted devices and the Trash. The host
    /// transfers or trashes them.
    DropInto { item: ItemId, dropped: Dropped },
    /// A drag's hover over a drop site changed: for spring-loading folders.
    DropHover { site: DropSite, hover: DropHover },
    /// Eject or unmount a device (its [`super::Release`] says which).
    Eject(String),
    /// Unlock an encrypted device.
    Unlock(String),
    /// Lock an encrypted device.
    Lock(String),
    /// A section's heading folded or unfolded it. The sidebar already flipped its own state;
    /// the host saves it and applies it to its other windows.
    SectionToggled { section: Section, collapsed: bool },
    /// The width stepped (the toggle, or [`super::Width::advance`] through the toggle).
    Stepped(Step),
    /// 10xer `h`, Left or Backspace in the sidebar: focus goes back to the files.
    LeaveToFiles,
}
