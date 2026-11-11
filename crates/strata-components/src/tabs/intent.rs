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

/// Everything the strip does is one of these. The host owns every policy: it selects, closes,
/// moves and opens tabs in its data, transfers and saves drops, and sets the window title.
#[derive(Clone, Debug, PartialEq)]
pub enum TabsIntent {
    /// Select tab `id`.
    Select { id: TabId, via: Via },
    /// Close tab `id`: its X button or a middle click. The host may refuse (running
    /// operations); it records the tab in its `ClosedTabs` only if it does close it.
    Close { id: TabId },
    /// The New tab button: open a tab at the active tab's location and select it.
    New,
    /// A drag moved tab `id` from `from` to `to`.
    Reorder { id: TabId, from: usize, to: usize },
    /// Tab `id` was dragged off the strip and let go: open it in a window of its own. Only
    /// with [`super::BrowserTabs::tear_out`].
    TearOut { id: TabId, out: TabTearOut },
    /// A torn-out tab moved, or came back (`out.torn` false), with the button still down.
    TearMove { id: TabId, out: TabTearMove },
    /// Files or content dropped on tab `id`: transfer the files into its folder with the action
    /// the host chooses, or save the content there, and select the tab.
    DropInto { id: TabId, dropped: Dropped },
    /// A drag the host said opens tabs dropped on the strip: open one tab per dragged folder,
    /// in order, from index `at`, and select the last.
    OpenTabs { at: usize, dropped: Dropped },
    /// A drag reached or left `site`, with what it offers: for a host that checks a drag's
    /// items by its serial.
    DropHover {
        site: DropSite,
        hover: DropHover,
        offer: DropOffer,
    },
    /// The window's title changed: the host applies it with `Tx::set_title`.
    Title(String),
}
