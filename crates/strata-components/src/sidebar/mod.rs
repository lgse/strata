// SPDX-License-Identifier: MIT

mod build;
mod data;
mod drag;
mod drops;
mod heading;
mod intent;
mod rows;
mod state;
mod toggle;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{Mods, Tx};

use crate::contract::dnd::{DropAction, DropOffer};

pub use build::PlacesSidebar;
pub use data::{
    Device, DeviceGroup, DeviceLabels, ItemId, LockState, Pin, Place, PlaceKind, Release, RowIcon,
    SidebarData, SidebarLabels,
};
pub use drag::{Landing, RowDrag, Rows, landing};
pub use drops::pin_action;
pub use intent::{DropSite, Gap, Section, SidebarIntent, Via};
pub use state::{MAX_WIDTH, MIN_WIDTH, PinCheck, PinVerdict, SidebarState, Step, Unchecked, Width};
pub use toggle::WidthToggle;

/// The key the sidebar's elements start with unless the host sets one.
pub const DEFAULT_KEY: &str = "places";

pub const TRASH_TARGET: &str = "places-trash-icon";

/// The Trash icon's key under the sidebar key `key`.
pub fn trash_target_key(key: &str) -> String {
    format!("{key}-trash-icon")
}

/// Where the sidebar's intents go.
pub(crate) type Emit<S> = Rc<dyn Fn(&mut S, SidebarIntent, &mut Tx)>;

/// The host's drop action for a row.
pub(crate) type ActionFn<S> = Rc<dyn Fn(&S, &ItemId, &DropOffer, Mods) -> Option<DropAction>>;
