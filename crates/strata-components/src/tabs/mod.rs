// SPDX-License-Identifier: MIT

mod build;
mod closed;
mod data;
mod drag;
mod drops;
mod intent;
mod new_tab;
mod probe;
mod sizing;
mod state;
mod wheel;

#[cfg(test)]
mod tests;

pub use build::BrowserTabs;
pub use closed::{CLOSED_TABS_KEPT, Closed, ClosedTabs};
pub use data::{
    Accepts, LABEL_CHARS, Tab, TabIcon, TabId, TabsData, TitleFormat, elide_end, window_title,
};
pub use intent::{DropSite, TabOpen, TabsIntent, Via};
pub use new_tab::{NewTabButton, TabsLabels};
pub use rune_kit::shell::TabWords;
pub use state::{BrowserTabsState, HOVER_DELAY, Polled};

pub const DEFAULT_KEY: &str = "tabs";

pub fn tab_key(key: &str, id: TabId) -> String {
    format!("t-{key}-{id}")
}

/// The New tab button's key in the strip keyed `key`.
pub fn new_tab_key(key: &str) -> String {
    format!("{key}-new")
}

/// The strip's sideways scroll viewport's key (rune-kit's `<key>-scroll`).
pub fn scroll_key(key: &str) -> String {
    format!("{key}-scroll")
}
