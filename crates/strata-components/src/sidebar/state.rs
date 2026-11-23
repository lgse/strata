// SPDX-License-Identifier: MIT

use std::cell::Cell;
use std::collections::HashMap;

use rune_kit::shell::PaneState;
use strata_kit::look::Look;
use strata_kit::slots::sidebar as slot;

use super::data::ItemId;
use super::drag::RowDrag;
use super::intent::DropSite;
use crate::contract::dnd::{DropHover, DropOffer};
use crate::scroll::{Bar, BarMode};

fn sidebar_bar() -> Bar {
    let bar = Bar::new();
    bar.set_mode(BarMode::Always);
    bar
}

/// The narrowest full sidebar (`MIN_SIDEBAR_WIDTH`, lgse/strata `upstream/main` 35ee677ab).
pub const MIN_WIDTH: f32 = 169.0;
/// The widest full sidebar a resize reaches.
pub const MAX_WIDTH: f32 = 480.0;

/// What the sidebar shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Step {
    /// Icons and labels, resizable.
    #[default]
    Full,
    /// Icons only, with name tooltips.
    Rail,
    /// Nothing.
    Hidden,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Width {
    chosen: Step,
    auto_rail: bool,
    pub pane: PaneState,
}

impl Width {
    /// A full sidebar `width` points wide.
    pub fn new(width: f32) -> Width {
        Width {
            chosen: Step::Full,
            auto_rail: false,
            pane: PaneState::new(width).range(MIN_WIDTH, MAX_WIDTH),
        }
    }

    /// The same width, starting on `step` (a saved preference).
    pub fn with_step(mut self, step: Step) -> Width {
        self.chosen = step;
        self
    }

    /// The step the toggle last chose.
    pub fn chosen(&self) -> Step {
        self.chosen
    }

    /// What shows: the rail while the fit rule rails a sidebar chosen full.
    pub fn shown(&self) -> Step {
        match self.chosen {
            Step::Full if self.auto_rail => Step::Rail,
            step => step,
        }
    }

    /// Whether the fit rule has railed the sidebar.
    pub fn auto_rail(&self) -> bool {
        self.auto_rail
    }

    pub fn advance(&mut self) -> Step {
        self.chosen = match self.shown() {
            Step::Full => Step::Rail,
            Step::Rail => Step::Hidden,
            Step::Hidden => Step::Full,
        };
        self.auto_rail = false;
        self.chosen
    }

    pub fn choose(&mut self, step: Step) {
        self.chosen = step;
        self.auto_rail = false;
    }

    pub fn set_auto_rail(&mut self, rail: bool) {
        self.auto_rail = rail && self.chosen == Step::Full;
    }

    /// The full sidebar's width, for the host's fit sum.
    pub fn full_width(&self) -> f32 {
        self.pane.width()
    }

    /// The toggle's tooltip, without its shortcut: the next step.
    pub fn next_label(&self) -> &'static str {
        match self.shown() {
            Step::Full => "Compact sidebar",
            Step::Rail => "Hide sidebar",
            Step::Hidden => "Show sidebar",
        }
    }
}

impl Default for Width {
    fn default() -> Width {
        Width::new(slot::DEFAULT_PANE_WIDTH)
    }
}

/// What the host found about a drag's items for pinning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PinVerdict {
    /// Nobody has looked.
    #[default]
    Unchecked,
    /// The host is looking (a metadata query on a worker).
    Checking,
    /// Every item is a folder that can be pinned.
    Folders,
    /// Some item can't be pinned: a file, a trashed folder, a link, a timeout.
    Refused,
}

/// How a gap treats a drag the host hasn't checked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Unchecked {
    #[default]
    Accept,
    Wait,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PinCheck {
    pub local: PinVerdict,
    pub foreign: Option<(u64, PinVerdict)>,
    pub unchecked: Unchecked,
}

impl PinCheck {
    /// The verdict for a drag offering `offer`.
    pub fn verdict(&self, offer: &DropOffer) -> PinVerdict {
        self.verdict_for(offer.local, offer.serial)
    }

    /// The verdict for a drag from this process or not, with serial `serial`.
    fn verdict_for(&self, local: bool, serial: u64) -> PinVerdict {
        if local {
            return self.local;
        }
        match self.foreign {
            Some((s, v)) if s == serial => v,
            _ => PinVerdict::Unchecked,
        }
    }

    pub fn takes(&self, verdict: PinVerdict) -> bool {
        match self.unchecked {
            Unchecked::Accept => verdict != PinVerdict::Refused,
            Unchecked::Wait => self.shows(verdict),
        }
    }

    /// Whether a gap shows its insertion line and opens for a drag with `verdict`.
    pub fn shows(&self, verdict: PinVerdict) -> bool {
        match verdict {
            PinVerdict::Folders => true,
            PinVerdict::Unchecked => self.unchecked == Unchecked::Accept,
            PinVerdict::Checking | PinVerdict::Refused => false,
        }
    }
}

/// The sidebar's state. Build it with [`SidebarState::new`] from the look, or `Default`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SidebarState {
    pub width: Width,
    pub pinned_collapsed: bool,
    pub devices_collapsed: bool,
    /// The drag that reorders the built-in places.
    pub places_reorder: RowDrag,
    /// The drag that reorders the pins.
    pub pins_reorder: RowDrag,
    /// The host's verdicts on drags over the pin gaps.
    pub pin_check: PinCheck,
    /// The `g` chord is armed: rows with a keycap show it.
    pub keycaps: bool,
    pub scroll_bar: Bar,
    drop: Option<(DropSite, DropHover)>,
    /// The drag over the sidebar, as its offer said: from this process, and its serial.
    pub(crate) drag: Option<(bool, u64)>,
    pub(crate) dropped: Option<ItemId>,
    /// Each row's laid-out box, in window points: where a menu the keyboard opens on it goes.
    pub(crate) boxes: HashMap<ItemId, [f32; 4]>,
    pub(crate) resizing: bool,
    pub(crate) last_shown: Cell<Option<Step>>,
}

impl Default for SidebarState {
    fn default() -> Self {
        SidebarState {
            width: Width::default(),
            pinned_collapsed: false,
            devices_collapsed: false,
            places_reorder: RowDrag::default(),
            pins_reorder: RowDrag::default(),
            pin_check: PinCheck::default(),
            keycaps: false,
            scroll_bar: sidebar_bar(),
            drop: None,
            drag: None,
            dropped: None,
            boxes: HashMap::new(),
            resizing: false,
            last_shown: Cell::new(None),
        }
    }
}

impl SidebarState {
    /// A full sidebar at the look's `sidebar.pane.width`.
    pub fn new(look: &Look) -> SidebarState {
        SidebarState {
            width: Width::new(slot::pane_width(look)),
            places_reorder: RowDrag::new().gap(slot::row_gap(look)),
            pins_reorder: RowDrag::new().gap(slot::row_gap(look)),
            ..SidebarState::default()
        }
    }

    pub fn row_box(&self, item: &ItemId) -> Option<[f32; 4]> {
        self.boxes.get(item).copied()
    }

    /// Where a drag is over the sidebar, and whether that site takes it.
    pub fn drop_over(&self) -> Option<&(DropSite, DropHover)> {
        self.drop.as_ref()
    }

    pub fn dragging(&self) -> bool {
        self.drop.is_some()
    }

    /// Whether `section` is folded.
    pub fn collapsed(&self, section: super::Section) -> bool {
        match section {
            super::Section::Pinned => self.pinned_collapsed,
            super::Section::Devices => self.devices_collapsed,
        }
    }

    pub(crate) fn toggle(&mut self, section: super::Section) -> bool {
        let c = match section {
            super::Section::Pinned => &mut self.pinned_collapsed,
            super::Section::Devices => &mut self.devices_collapsed,
        };
        *c = !*c;
        *c
    }

    pub fn set_drop_hover(&mut self, site: DropSite, hover: DropHover) {
        match hover {
            DropHover::Left => {
                if self.drop.as_ref().is_some_and(|(s, _)| *s == site) {
                    self.end_drag();
                }
            }
            h => self.drop = Some((site, h)),
        }
    }

    /// The drag dropped or left the window: no site holds it.
    pub fn end_drag(&mut self) {
        self.drop = None;
        self.drag = None;
    }

    /// Remembers which drag is over a gap, so the gaps read the host's verdict on it.
    pub(crate) fn note_drag(&mut self, offer: &DropOffer) {
        self.drag = Some((offer.local, offer.serial));
    }

    /// The host's verdict on the drag over the gaps.
    pub fn gap_verdict(&self) -> PinVerdict {
        match self.drag {
            Some((local, serial)) => self.pin_check.verdict_for(local, serial),
            None => PinVerdict::Unchecked,
        }
    }
}
