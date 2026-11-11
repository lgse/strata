//! PlacesSidebar (C8): the browser's sidebar of built-in places, PINNED and DEVICES, in three
//! widths (full, icon rail, hidden), with pin insertion gaps, device rows and the Trash
//! target.
//!
//! Work package: C8, `docs/work/phase-1-leaves/C8-places-sidebar.md`. Feature map nodes
//! `browser/sidebar`, `browser/sidebar/pins`, `browser/sidebar/pins/drag-to-pin`,
//! `browser/sidebar/rail` and `devices/volumes/drive-order`. The GTK reference is lgse/strata
//! `upstream/main` 35ee677ab (`src/ui/window/sidebar*.rs`); it has the rail and the
//! collapsible sections but not yet the drive groups (lgse/strata#2075), for which the feature
//! map is the spec.
//!
//! # Parts
//!
//! - [`PlacesSidebar`] builds the sidebar from [`SidebarData`] (plain data the host rebuilds
//!   each view: no I/O) and its [`SidebarState`] (held by the app, reached by accessor).
//! - [`WidthToggle`] is the header's button that steps full → rail → hidden → full.
//! - Everything the sidebar does is a [`SidebarIntent`]: open, open in a new tab, the context
//!   menu, reorders, pin drops, drops into rows, eject, unlock and lock, section toggles and
//!   width steps. The host owns the policy.
//!
//! # Built on rune-kit
//!
//! Full-width rows are rune-kit's `SidebarRow`s, with their reorder drag, selection and
//! focus. The sidebar replaces rune-kit's `Sidebar`, `SidebarSection` and `SidebarHeader`:
//! their list presses each row the arrow keys reach (`focus_follows`), which would open a
//! place on every arrow press; their headings take no keyboard focus; and a collapsed section
//! drops every row, where Strata keeps the open pin. Over each built row the sidebar sets
//! Strata's look ([`strata_kit::slots::sidebar`]): the accent tint and icon on the current row,
//! a neutral hover, the rail's tooltip.
//!
//! In the rail each row is the sidebar's own icon-only list item, with its icon where the full
//! row has it, so a step moves no icon and the keyboard order stays. Its name stays its
//! accessible name and becomes its tooltip. Rail rows don't drag to reorder (U18).
//! Device buttons and the headings' titles, rules and counts are left out; a heading keeps
//! its row with only its chevron, and a separator line stands above each section.
//!
//! # Drops
//!
//! Every drop zone attaches through the host's [`Dnd`](crate::contract::dnd::Dnd) binder:
//!
//! - **Pin gaps.** A gap before each pin and after the last ([`Gap`] carries its full-list
//!   index and both neighbours). The gap's drop area is a band reaching
//!   `sidebar.gap.reach` over each neighbour's edge. A drop is always a [`SidebarIntent::PinDrop`],
//!   never a transfer. [`pin_action`] refuses a Move-only drag from another app and a drag the
//!   host refused. The host checks the items are folders after [`SidebarIntent::PinHover`], and
//!   writes its verdict to [`SidebarState::pin_check`]; the gap opens and draws its line only
//!   for a drag the verdict shows ([`PinCheck::shows`]). Rune's binder reads a drag's paths only
//!   at the drop, so under it the host uses [`Unchecked::Accept`] and checks at the drop.
//! - **Rows.** Places, local pins and mounted devices take files into their location
//!   ([`SidebarIntent::DropInto`]), with the host's `drop_action`. Remote pins take none.
//! - **The sidebar.** A zone over the whole sidebar takes nothing; it notices a drag is
//!   there, so an empty PINNED section shows its gap and rows drop their hover fill and focus
//!   ring. One site owns the drop indicator at a time.
//!
//! Each zone reaches the binder with its element's key (W1's `Dnd::drop_zone_keyed`), unique in
//! the window given a sidebar key of its own ([`PlacesSidebar::key`], [`DEFAULT_KEY`] by
//! default): the sidebar by that key, gap `i` by `<key>-band-{i}`, and a row by
//! [`ItemId::key`] (`<key>-place-…`, `<key>-pin-…`, `<key>-device-…`).
//!
//! # The Trash target
//!
//! The Trash icon's box is keyed [`trash_target_key`] (`places-trash-icon` by default,
//! [`TRASH_TARGET`]) in the full sidebar and the rail. It is absent while the sidebar is
//! hidden, and C23 skips its flight then. [`PlacesSidebar::trash_icon`] lets C23 dress that
//! box (its pulse, shake or landing element).
//!
//! # Deliberate changes
//!
//! - **Reorders step rows aside.** Strata draws an accent line in the gap where a dragged
//!   row will land. rune-kit's reorder lifts the row and springs its neighbours aside instead,
//!   which shows the same place. Dropping a row on either gap next to it is still a no-op.
//! - **A built-in place dragged past its block lands at the block's end.** Strata drops a
//!   built-in place on a pin as a no-op. Places and pins are two separate reorder lists, so a
//!   place never lands among the pins, but rune-kit clamps a drag past the end of its list to
//!   the end (U21).
//! - **The open-gap animation.** Strata draws a 2 px line in the gap. Here a gap taking a drag
//!   opens to `sidebar.gap.open` (half a row) on the kit's disclosure spring, and draws the
//!   line inside; with animations off it opens at once.
//! - **Gaps are hidden from assistive technology.** Strata's gaps are each an empty group
//!   named "PINNED" (a known gap of `browser/sidebar/pins/drag-to-pin`); here they are drop
//!   targets only, out of the tree.
//! - **No keyboard pin reorder.** The work package's Tests line asks for "pin reorder by
//!   keyboard", but no feature-map bullet covers one, and rune-kit's `Reorder` has no keyboard
//!   move (U21). Left out; pins reorder by drag.
//! - **The list is one Tab stop**, as rune-kit's lists are: Tab enters it on the current row,
//!   and the arrows (10xer `j` and `k`) step through rows and headings. Strata's rows are each
//!   a Tab stop (`browser/sidebar`: "In the icon rail, 10xer `j`/`k` and Tab also stop on the
//!   chevron-only headings").
//! - **Device buttons are pointer-only**, as rune-kit's row controls are: the list's arrows
//!   step rows, and the row's menu (C21) has the same actions. So Tab doesn't stop on an eject
//!   button (`devices/volumes`: "Tabbing to the eject button shows one ring").
//!
//! # What stays in Strata
//!
//! Which places show and in what order, labels, the pins' file, whether a drag's items are
//! folders, transfers and trashing, device discovery, drive order and grouping, mounting,
//! ejecting, unlocking, the automatic rail's fit rule (C9's `demand()` plus Strata's sum,
//! through [`Width::set_auto_rail`]), saving preferences and syncing windows, the `g` chord's
//! numbering, the update notice ([`PlacesSidebar::footer`]) and every menu (C21).
//!
//! # Rune gaps
//!
//! Drafts for the owner to file, numbered after S5's index (U17). Headless repros are in
//! `strata-spikes/c8-probes` (`RESULTS.md`).
//!
//! - **U18** rune-kit: `SidebarRow` has no icon-only (rail) form and no tooltip: with an empty
//!   label it still keeps the label slot and its gap, which don't fit a rail and draw an
//!   ellipsis. And `SplitView`'s sidebar knows only open and closed, with no compact step.
//!   Here: the rail builds its own icon-only row (without the reorder drag, which only
//!   `SidebarRow` can drive), `.help` names it, and the sidebar sizes its own column and draws
//!   its own grip on `PaneState`.
//! - **U19** rune-kit: `Sidebar`'s list always presses the row focus reaches
//!   (`focus_follows`), and `SidebarHeader` takes no focus (`focusable(false)`). A file
//!   manager opens a place only on Enter or a click, and steps onto headings. Asks for
//!   `Sidebar::follow_focus(false)` and keyboard-focusable headers. Here: the sidebar's own
//!   list and headings.
//! - **U20** rune-kit: a collapsed `SidebarSection` drops every row, with no way to keep the
//!   row of the open item in view. Here: the sidebar's own section.
//! - **U21** rune-kit `Reorder`: no keyboard move; a drag past the list's end clamps to the
//!   end, with no way to cancel a drop outside the list; with `pointer_buttons([Middle])` a
//!   middle-button drag reorders too; `Reorder::follow` is private, so an app can't drive it
//!   from its own row; `SidebarRow::reorder` takes a plain `fn` accessor (no capture); and its
//!   drop runs with no `Tx`. Rune then presses the row on the same release, but only when the
//!   pointer is still over it (`runtime/input/pointer.rs`, `release`), so a drag released off
//!   the row ends with no press at all. Here: [`PlacesSidebar::reorder`] takes the host's
//!   accessors, and the drop parks its move in the state with the row it came from. That row's
//!   press reports the move instead of opening the row; any other press or key activation
//!   reports it first and then acts; and the host drains a move no press followed with
//!   [`SidebarState::take_reordered`] from its poll, so no move is lost or reported late. A drag
//!   that lands in place moves nothing and still presses.
//! - **U22** rune-kit: `SidebarRow`'s icon box is fixed at 16 points
//!   ([`strata_kit::slots::sidebar::ICON_BOX`]), and its row inset is a private 8
//!   ([`strata_kit::slots::sidebar::ROW_INSET`]); its selected fill is fixed to `c.pressed`;
//!   its busy spinner is `icon_muted`, where Strata's pending-release spinner takes the accent;
//!   it names the row by its label only, so the pending row's "Writing data to the drive. Do
//!   not unplug it." is its description, where Strata makes it the row's and spinner's label;
//!   and its colour `over` isn't public. Here: the fill set over the built row (with the lift
//!   fill kept under the tint while the row is lifted), the icon passed as an `avatar` coloured
//!   by the sidebar, the rail's icon box and inset matching rune-kit's so a step moves no icon
//!   at any text size, and `over` copied. An icon larger than the box (K2 at large text sizes)
//!   overflows it, centred, in both widths.
//! - **U23** Rune: a press handler can't tell a click from a keyboard press (`Tx::button` says
//!   only which pointer button). Strata opens a place with its first item selected from the
//!   keyboard only. Here: each row's `on_key` takes Return and Space before the list's focus
//!   navigation presses the row, and reports [`Via::Keyboard`].
//! - Hover-time paths and actions for file drags are S5's **U3**; C8's folder check waits on it.

mod build;
mod data;
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
    Device, DeviceGroup, ItemId, LockState, Pin, Place, PlaceKind, Release, RowIcon, SidebarData,
};
pub use drops::pin_action;
pub use heading::count_words;
pub use intent::{DropSite, Gap, Section, SidebarIntent, Via};
pub use rows::PENDING_RELEASE;
pub use state::{MAX_WIDTH, MIN_WIDTH, PinCheck, PinVerdict, SidebarState, Step, Unchecked, Width};
pub use toggle::WidthToggle;

/// The key the sidebar's elements start with unless the host sets one.
pub const DEFAULT_KEY: &str = "places";

/// The Trash icon's key under [`DEFAULT_KEY`]: C23's flight target.
pub const TRASH_TARGET: &str = "places-trash-icon";

/// The Trash icon's key under the sidebar key `key`.
pub fn trash_target_key(key: &str) -> String {
    format!("{key}-trash-icon")
}

/// Where the sidebar's intents go.
pub(crate) type Emit<S> = Rc<dyn Fn(&mut S, SidebarIntent, &mut Tx)>;

/// The host's drop action for a row.
pub(crate) type ActionFn<S> = Rc<dyn Fn(&S, &ItemId, &DropOffer, Mods) -> Option<DropAction>>;

/// Finds a list's reorder drag in the app's state.
pub type ReorderAccess<S> = fn(&mut S) -> &mut rune_kit::shell::Reorder;
