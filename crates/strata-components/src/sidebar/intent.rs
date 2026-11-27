// SPDX-License-Identifier: MIT

use rune::PointerPress;

use super::data::ItemId;
use super::drag::Landing;
use super::state::Step;
use crate::contract::dnd::{DropHover, Dropped};

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
    pub(crate) fn id(self) -> &'static str {
        match self {
            Section::Pinned => "pinned",
            Section::Devices => "devices",
        }
    }
}

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
    Open {
        item: ItemId,
        via: Via,
    },
    OpenInNewTab {
        item: ItemId,
    },
    ContextMenu {
        item: Option<ItemId>,
        point: PointerPress,
    },
    MovePlace {
        from: usize,
        to: usize,
        landing: Landing,
    },
    MovePin {
        id: String,
        from: usize,
        to: usize,
        landing: Landing,
    },
    PinHover {
        serial: u64,
        uris: Option<Vec<String>>,
        local: bool,
    },
    PinDrop {
        gap: Gap,
        dropped: Dropped,
    },
    DropInto {
        item: ItemId,
        dropped: Dropped,
    },
    /// A drag's hover over a drop site changed: for spring-loading folders.
    DropHover {
        site: DropSite,
        hover: DropHover,
    },
    /// Eject or unmount a device (its [`super::Release`] says which).
    Eject(String),
    /// Unlock an encrypted device.
    Unlock(String),
    /// Lock an encrypted device.
    Lock(String),
    SectionToggled {
        section: Section,
        collapsed: bool,
    },
    /// The width stepped (the toggle, or [`super::Width::advance`] through the toggle).
    Stepped(Step),
    /// 10xer `h`, Left or Backspace in the sidebar: focus goes back to the files.
    LeaveToFiles,
    /// A reorderable place or pin row crossed the drag threshold.
    RowDragStart {
        item: ItemId,
    },
    RowDragMove {
        item: ItemId,
        landing: Option<Landing>,
        at: [f32; 2],
    },
    RowDragEnd {
        item: ItemId,
        dropped_outside: Option<[f32; 2]>,
    },
}
