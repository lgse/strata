//! The sidebar's own state, held by the app and reached by accessor: the width step, the
//! collapsed sections, the reorder drags, the drop hover and the host's pin check.

use rune_kit::shell::{PaneState, Reorder};
use strata_kit::look::Look;
use strata_kit::slots::sidebar as slot;

use super::data::ItemId;
use super::intent::DropSite;
use crate::contract::dnd::{DropHover, DropOffer};

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

/// The sidebar's width: the step the user chose, an automatic rail the host's fit rule set,
/// and the full width the user dragged to.
#[derive(Clone, Debug, PartialEq)]
pub struct Width {
    chosen: Step,
    auto_rail: bool,
    /// The full sidebar's width and range. Its grip drags it; the rail and hidden steps leave
    /// it as it was, so the full sidebar comes back at the same width.
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

    /// One press of the toggle: full to rail to hidden to full, from what shows. Choosing the
    /// rail holds it at any width; stepping from hidden shows the full sidebar even where the
    /// fit rule had railed it, until the host's rule next finds it short of room.
    pub fn advance(&mut self) -> Step {
        self.chosen = match self.shown() {
            Step::Full => Step::Rail,
            Step::Rail => Step::Hidden,
            Step::Hidden => Step::Full,
        };
        self.auto_rail = false;
        self.chosen
    }

    /// The user chose `step` some other way (a saved preference, Settings). A chosen rail
    /// holds when the window widens again, even if the fit rule had railed the sidebar first.
    pub fn choose(&mut self, step: Step) {
        self.chosen = step;
        self.auto_rail = false;
    }

    /// The host's fit rule (C9's `demand()` and Strata's sum) railed the sidebar or released
    /// it. Call it only when the fit changes: the rule is edge-triggered. A chosen rail or a
    /// hidden sidebar ignores it.
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
    /// Show the insertion line and take the drop; the host checks the items at the drop.
    /// For binders that can't read a drag's items while it hovers (Rune's).
    #[default]
    Accept,
    /// Show nothing until the host says [`PinVerdict::Folders`]. Strata's, under a binder that
    /// reads URIs while the drag hovers.
    Wait,
}

/// The host's verdicts on drags over the pin gaps: one for its own drag (it knows what it
/// dragged), one for the last foreign drag it checked, keyed by the drag's serial.
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

    /// Whether a gap takes a drop of a drag with `verdict`: one it shows, or under
    /// [`Unchecked::Accept`] any it hasn't refused, for the host to check at the drop.
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
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SidebarState {
    pub width: Width,
    pub pinned_collapsed: bool,
    pub devices_collapsed: bool,
    /// The drag that reorders the built-in places.
    pub places_reorder: Reorder,
    /// The drag that reorders the pins.
    pub pins_reorder: Reorder,
    /// The host's verdicts on drags over the pin gaps.
    pub pin_check: PinCheck,
    /// The `g` chord is armed: rows with a keycap show it.
    pub keycaps: bool,
    drop: Option<(DropSite, DropHover)>,
    /// The drag over the sidebar, as its offer said: from this process, and its serial.
    pub(crate) drag: Option<(bool, u64)>,
    /// A reorder just dropped: the row dragged, and the move it made. The drop runs with no
    /// `Tx`, and Rune presses the row on the release that ends the drag when the pointer is
    /// still over it (U21). That press reports the move instead of opening the row; any other
    /// activation reports it first; and the host drains one no press followed with
    /// [`SidebarState::take_reordered`].
    pub(crate) reordered: Option<(ItemId, super::SidebarIntent)>,
}

impl SidebarState {
    /// A full sidebar at the look's `sidebar.pane.width`.
    pub fn new(look: &Look) -> SidebarState {
        SidebarState {
            width: Width::new(slot::pane_width(look)),
            places_reorder: Reorder::new().gap(slot::row_gap(look)),
            pins_reorder: Reorder::new().gap(slot::row_gap(look)),
            ..SidebarState::default()
        }
    }

    /// Where a drag is over the sidebar, and whether that site takes it.
    pub fn drop_over(&self) -> Option<&(DropSite, DropHover)> {
        self.drop.as_ref()
    }

    /// A drag is over the sidebar: rows show no hover fill and no focus ring, and an empty
    /// PINNED section shows its gap.
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

    /// A drop site's hover changed: the sidebar's zones call this, and a host or test may to
    /// show a drop. One site owns the indicator: arriving takes it, and a
    /// leave clears it only if that site still holds it, so a late leave from the site a drag
    /// moved off never clears the one it moved onto.
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
    pub(crate) fn end_drag(&mut self) {
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

    /// A reorder's move that no press has reported yet: the drag was released off its row
    /// (U21). Call it from the app's poll every frame and handle the intent it returns; a
    /// press or key on the sidebar reports one itself, before acting.
    pub fn take_reordered(&mut self) -> Option<super::SidebarIntent> {
        self.reordered.take().map(|(_, intent)| intent)
    }
}
