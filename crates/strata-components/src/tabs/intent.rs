// SPDX-License-Identifier: MIT

//! What the strip asks of its host.

use rune_kit::shell::{TabTearMove, TabTearOut};

use super::data::TabId;
use crate::contract::dnd::{DropHover, DropOffer, Dropped};

/// How a tab came to be selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Via {
    /// A press on it: the pointer, or Return or Space with it focused.
    Strip,
    /// A file or content drag rested on it for the spring delay.
    Spring,
}

/// Where a drag is over the strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropSite {
    /// A tab.
    Tab(TabId),
    /// The New tab button.
    NewTab,
    /// The strip between and around the tabs.
    Strip,
}

/// How an in-process drag opens new tabs on the strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TabOpen {
    AtGap,
    AtEnd,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TabsIntent {
    /// Select tab `id`.
    Select {
        id: TabId,
        via: Via,
    },
    Close {
        id: TabId,
    },
    /// The New tab button: open a tab at the active tab's location and select it.
    New,
    /// A drag moved tab `id` from `from` to `to`.
    Reorder {
        id: TabId,
        from: usize,
        to: usize,
    },
    TearOut {
        id: TabId,
        out: TabTearOut,
    },
    /// A torn-out tab moved, or came back (`out.torn` false), with the button still down.
    TearMove {
        id: TabId,
        out: TabTearMove,
    },
    DropInto {
        id: TabId,
        dropped: Dropped,
    },
    OpenTabs {
        at: usize,
        dropped: Dropped,
    },
    DropHover {
        site: DropSite,
        hover: DropHover,
        offer: DropOffer,
    },
    /// The window's title changed: the host applies it with `Tx::set_title`.
    Title(String),
}
