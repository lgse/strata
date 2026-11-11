//! The pointer state machine: presses, moves and releases in, selection operations and
//! [`Outcome`]s out. Plain data, no Rune elements.

use std::time::Duration;

use rune::Mods;

use super::clicks::Clicks;
use super::rename::{Due, PendingRename};
use super::settings::{ClickCount, PointerSettings};
use crate::collection::selection::{Changed, Extend, Selection};
use crate::contract::{EntryId, EntrySource, Point};

/// The button behind a press.
///
/// Used by: the C1e adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    /// The primary button: select, open, drag, marquee, rename.
    Primary,
    /// The middle button: open in a new tab or window.
    Middle,
    /// The secondary button: the context menu.
    Secondary,
}

/// Where in an entry's row a press lands, as Strata's pointer intent tells them apart
/// (`src/ui/pointer.rs`).
///
/// Used by: the C1e adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Spot {
    /// The drawn text of the entry's name: content, and where a slow click renames. In Icons,
    /// the caption band.
    Name,
    /// The rest of the entry itself: its icon or thumbnail, and in List the other columns.
    Content,
    /// Blank row or card space beside the entry, or no entry at all.
    Gutter,
}

/// A button going down.
///
/// Used by: the C1e adapter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    /// Which button.
    pub button: Button,
    /// Where, in content coordinates: where a marquee starts.
    pub at: Point,
    /// Where, in window points: what click pairs and drags measure from.
    pub window: Point,
    /// The entry under the press, by index into the source, or `None` over empty space.
    pub index: Option<usize>,
    /// Which part of the entry. Empty space is [`Spot::Gutter`].
    pub spot: Spot,
    /// The modifiers held. The press keeps them: letting go of one before the button changes
    /// nothing.
    pub mods: Mods,
    /// The host's clock, in seconds.
    pub now: f64,
}

/// The button coming up, or the gesture ending without a release.
///
/// Used by: the C1e adapter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Release {
    /// The pointer's travel since the press, in points.
    pub travel: Point,
    /// The modifiers held now. Only a middle click reads them, as Strata does.
    pub mods: Mods,
    /// The host's clock, in seconds.
    pub now: f64,
    /// The gesture ended without a release (Escape, focus lost): nothing it would have done
    /// on release happens.
    pub cancelled: bool,
}

/// How an entry was opened.
///
/// Used by: the C1e adapter, hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActivateBy {
    /// A single click, where the entry's kind opens on one.
    Click,
    /// A double click.
    DoubleClick,
}

/// What the machine asks for besides selection changes. What each means is the host's policy.
///
/// Variants may be added.
///
/// Used by: the C1e adapter, hosts.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Outcome {
    /// Open the entry.
    Activate {
        /// The entry.
        id: EntryId,
        /// By one click or two.
        how: ActivateBy,
    },
    /// A plain single click on the entry finished without opening or arming a rename: where
    /// single-click previews follow. Never sent for a press that dragged.
    Clicked {
        /// The entry.
        id: EntryId,
    },
    /// Open the folder in a new tab, or a new window (a middle click).
    OpenInNew {
        /// The folder.
        id: EntryId,
        /// A new window rather than a tab (Ctrl or Shift held at release).
        window: bool,
    },
    /// Start a file drag of these entries: the host builds the payload.
    BeginDrag {
        /// The dragged entries, in display order.
        ids: Vec<EntryId>,
        /// Where the pointer is as the drag starts, in window points.
        at: Point,
    },
    /// Start a marquee at the press.
    BeginMarquee {
        /// The press, in content coordinates.
        at: Point,
        /// The modifiers held at the press.
        mods: Mods,
    },
    /// Start renaming the entry.
    BeginRename {
        /// The entry.
        id: EntryId,
    },
    /// Open the entry's context menu, or the background's.
    ContextMenu {
        /// The entry, or `None` for the background.
        target: Option<EntryId>,
        /// The press, in window points.
        at: Point,
    },
    /// Call [`PointerMachine::tick`] again after this long.
    WakeAfter(Duration),
}

/// What the machine reads and changes: its settings, the source, the selection, and where its
/// outcomes go. C1e's `Ctx` is one ([`PointerIntent`](super::PointerIntent)); tests use a
/// plain one.
///
/// Used by: the C1e adapter.
pub trait Surface {
    /// The settings now.
    fn settings(&self) -> PointerSettings;

    /// The entries, in display order.
    fn source(&self) -> &dyn EntrySource;

    /// The selection, to read.
    fn selection(&self) -> &Selection;

    /// Runs one selection operation, and returns what it changed.
    fn apply(&mut self, op: &mut dyn FnMut(&mut Selection, &dyn EntrySource) -> Changed)
    -> Changed;

    /// Hands on an outcome.
    fn emit(&mut self, outcome: Outcome);
}

/// What crossing the drag threshold does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    Drag,
    Marquee,
    Nothing,
}

/// A selection operation a press leaves to its release, because a drag would do something
/// else instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Deferred {
    None,
    Toggle,
    Extend(Extend),
}

/// One held button.
#[derive(Clone, Copy, Debug)]
struct Gesture {
    press: Press,
    /// The entry at the press, and its index then.
    entry: Option<(usize, EntryId)>,
    /// Which click of a run the press is: 1, 2, 3, …
    count: u32,
    plan: Plan,
    deferred: Deferred,
    /// A plain press inside a multi-selection kept the group for a drag; a click collapses it.
    kept_group: bool,
    /// The entry was the only selected one before the press.
    sole_before: bool,
    crossed: bool,
}

/// The pointer state machine of one collection.
///
/// Feed it each press, move and release, and run [`PointerMachine::tick`] when it asked to be
/// woken. It reads its settings from the [`Surface`], changes the selection through it and
/// emits [`Outcome`]s for the rest. The [module docs](super) give the rules.
///
/// Used by: the C1e adapter.
#[derive(Debug, Default)]
pub struct PointerMachine {
    clicks: Clicks,
    gesture: Option<Gesture>,
    rename: Option<PendingRename>,
}

impl PointerMachine {
    /// A machine with no gesture held.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a slow-click rename is waiting.
    pub fn rename_pending(&self) -> bool {
        self.rename.is_some()
    }

    /// A button went down. Any press cancels a pending rename and ends the last gesture, even
    /// one whose release never came (a system drag can take it). True when the machine took
    /// the press.
    pub fn press(&mut self, press: &Press, s: &mut dyn Surface) -> bool {
        self.rename = None;
        self.gesture = None;
        let settings = s.settings();
        let entry = press
            .index
            .filter(|&i| i < s.source().len())
            .map(|i| (i, s.source().id_at(i)));
        match press.button {
            Button::Secondary => {
                self.clicks.break_run();
                secondary(press, entry, s);
                true
            }
            Button::Middle => {
                let count = self.count(press, entry, &settings);
                if entry.is_none() {
                    return false;
                }
                self.gesture = Some(Gesture::new(*press, entry, count, Plan::Nothing));
                true
            }
            Button::Primary => {
                let count = self.count(press, entry, &settings);
                self.gesture = Some(primary(press, entry, count, &settings, s));
                true
            }
        }
    }

    fn count(
        &mut self,
        press: &Press,
        entry: Option<(usize, EntryId)>,
        settings: &PointerSettings,
    ) -> u32 {
        self.clicks.press(
            press.button,
            entry.map(|(_, id)| id),
            press.window,
            press.now,
            settings.double_click.as_secs_f64(),
            settings.double_click_distance,
        )
    }

    /// The held pointer moved; `travel` is its travel since the press. Past the threshold, the
    /// press becomes a drag or a marquee and is no longer a click. True while a gesture is held.
    pub fn moved(&mut self, travel: Point, s: &mut dyn Surface) -> bool {
        let threshold = s.settings().drag_threshold;
        let Some(g) = self.gesture.as_mut() else {
            return false;
        };
        if g.crossed || !past(travel, threshold) {
            return true;
        }
        g.crossed = true;
        self.clicks.break_run();
        if g.press.button != Button::Primary {
            return true;
        }
        match g.plan {
            Plan::Drag => {
                // A Ctrl-drag carries the entry: it stays selected instead of toggling off.
                g.deferred = Deferred::None;
                let ids = s.selection().selected_ids(s.source());
                if !ids.is_empty() {
                    let at = Point::new(g.press.window.x + travel.x, g.press.window.y + travel.y);
                    s.emit(Outcome::BeginDrag { ids, at });
                }
            }
            Plan::Marquee => {
                // The band applies the modifiers to the selection at press.
                g.deferred = Deferred::None;
                s.emit(Outcome::BeginMarquee {
                    at: g.press.at,
                    mods: g.press.mods,
                });
            }
            Plan::Nothing => {}
        }
        true
    }

    /// The button came up, or the gesture was cancelled. A press that crossed the threshold,
    /// was cancelled, or whose entry moved during it (a re-sort, a reload) does nothing more.
    pub fn release(&mut self, release: &Release, s: &mut dyn Surface) -> bool {
        let settings = s.settings();
        let threshold = settings.drag_threshold;
        let Some(g) = self.gesture.take() else {
            return false;
        };
        if release.cancelled || g.crossed || past(release.travel, threshold) {
            self.clicks.break_run();
            return true;
        }
        let Some((index, id)) = g.entry else {
            return true;
        };
        if s.source().index_of(id) != Some(index) {
            self.clicks.break_run();
            return true;
        }
        match g.press.button {
            Button::Middle => middle(&g, index, id, release.mods, s),
            Button::Primary => {
                self.rename = primary_release(&g, index, id, &settings, s).map(|due| {
                    s.emit(Outcome::WakeAfter(settings.double_click));
                    PendingRename {
                        id,
                        due: release.now + due,
                        revision: s.selection().revision(),
                    }
                });
            }
            Button::Secondary => {}
        }
        true
    }

    /// Runs a pending rename once its interval has passed. True when it renamed.
    pub fn tick(&mut self, now: f64, s: &mut dyn Surface) -> bool {
        let Some(pending) = self.rename else {
            return false;
        };
        match pending.due(now, s.selection(), s.source()) {
            Due::Wait => {
                // A tick can come early, from another controller's wake-up: ask for our own.
                s.emit(Outcome::WakeAfter(Duration::from_secs_f64(
                    pending.due - now,
                )));
                false
            }
            Due::Cancelled => {
                self.rename = None;
                false
            }
            Due::Rename(id) => {
                self.rename = None;
                s.emit(Outcome::BeginRename { id });
                true
            }
        }
    }

    /// A key reached the collection: it cancels a pending rename.
    pub fn interrupt(&mut self) {
        self.rename = None;
    }

    /// Drops the press held and a pending rename, and ends the click run: the collection shows
    /// another folder, or the host saw a press or a key elsewhere (another Columns column).
    pub fn reset(&mut self) {
        self.gesture = None;
        self.rename = None;
        self.clicks.break_run();
    }
}

impl Gesture {
    fn new(press: Press, entry: Option<(usize, EntryId)>, count: u32, plan: Plan) -> Self {
        Self {
            press,
            entry,
            count,
            plan,
            deferred: Deferred::None,
            kept_group: false,
            sole_before: false,
            crossed: false,
        }
    }
}

/// Travel past `threshold` on either axis, as GTK's drag check and Strata's
/// `exceeds_drag_threshold` measure it.
fn past(travel: Point, threshold: f32) -> bool {
    travel.x.abs() > threshold || travel.y.abs() > threshold
}

fn apply(s: &mut dyn Surface, mut op: impl FnMut(&mut Selection, &dyn EntrySource) -> Changed) {
    s.apply(&mut op);
}

/// A secondary press selects an unselected entry alone and opens its menu; inside the
/// selection it keeps the selection.
fn secondary(press: &Press, entry: Option<(usize, EntryId)>, s: &mut dyn Surface) {
    if let Some((index, _)) = entry
        && !s.selection().is_selected_at(s.source(), index)
    {
        apply(s, |sel, src| sel.select_only(src, index));
    }
    s.emit(Outcome::ContextMenu {
        target: entry.map(|(_, id)| id),
        at: press.window,
    });
}

/// What a primary press changes now, and what it leaves to a drag or the release.
fn primary(
    press: &Press,
    entry: Option<(usize, EntryId)>,
    count: u32,
    settings: &PointerSettings,
    s: &mut dyn Surface,
) -> Gesture {
    let (ctrl, shift) = (press.mods.ctrl, press.mods.shift);
    let range = if ctrl {
        settings.ctrl_shift
    } else {
        Extend::Replace
    };
    let mut g = Gesture::new(*press, entry, count, Plan::Marquee);
    let Some((index, _)) = entry else {
        if !ctrl && !shift {
            apply(s, |sel, _| sel.clear());
        }
        return g;
    };
    let selected = s.selection().is_selected_at(s.source(), index);
    let len = s.selection().len();
    g.sole_before = selected && len == 1;
    // The entry itself, or blank space on a selected one, is where a drag drags files.
    let on_entry = press.spot != Spot::Gutter || selected;
    // A file drag carries the selection the press makes, so it is made now. A band applies
    // Ctrl or Shift to the selection as it was before the press (Strata takes it at the band's
    // press, `marquee/state.rs`), so where a drag is a band they wait for the release.
    let now = on_entry && !press.mods.alt;
    if shift {
        if now {
            apply(s, |sel, src| sel.extend_to(src, index, range));
        } else {
            g.deferred = Deferred::Extend(range);
        }
    } else if ctrl {
        if now && !selected {
            apply(s, |sel, src| sel.toggle(src, index));
        } else {
            g.deferred = Deferred::Toggle;
        }
    } else if on_entry && selected && len > 1 {
        g.kept_group = true;
    } else {
        apply(s, |sel, src| sel.select_only(src, index));
    }
    g.plan = if press.mods.alt || !on_entry {
        Plan::Marquee
    } else if settings.drags {
        Plan::Drag
    } else {
        Plan::Nothing
    };
    g
}

/// A primary click's release: the deferred operation, the group collapse, then opening,
/// arming a rename (returns its delay in seconds) or a plain click, in Strata's order.
fn primary_release(
    g: &Gesture,
    index: usize,
    id: EntryId,
    settings: &PointerSettings,
    s: &mut dyn Surface,
) -> Option<f64> {
    match g.deferred {
        Deferred::None => {}
        Deferred::Toggle => apply(s, |sel, src| sel.toggle(src, index)),
        Deferred::Extend(mode) => apply(s, |sel, src| sel.extend_to(src, index, mode)),
    }
    let mods = g.press.mods;
    if mods.ctrl || mods.shift {
        return None;
    }
    if g.kept_group {
        apply(s, |sel, src| sel.select_only(src, index));
    }
    let kind = s.source().entry(index).kind;
    let activate = match (g.count, settings.modes.for_kind(kind)) {
        (1, ClickCount::One) => Some(ActivateBy::Click),
        (2, ClickCount::Two) => Some(ActivateBy::DoubleClick),
        _ => None,
    };
    let renames = g.count == 1
        && !mods.alt
        && !mods.logo
        && g.press.spot == Spot::Name
        && g.sole_before
        && settings.rename
        && (activate.is_none() || settings.already_open == Some(id));
    if renames {
        return Some(settings.double_click.as_secs_f64());
    }
    if let Some(how) = activate {
        s.emit(Outcome::Activate { id, how });
    } else if g.count == 1 {
        s.emit(Outcome::Clicked { id });
    }
    None
}

/// A middle click opens a folder in a new tab, or a window with Ctrl or Shift held at release,
/// and selects a file. Only the first click of a run counts.
fn middle(g: &Gesture, index: usize, id: EntryId, mods: Mods, s: &mut dyn Surface) {
    if g.count != 1 {
        return;
    }
    if s.source().entry(index).kind.is_folder_like() {
        s.emit(Outcome::OpenInNew {
            id,
            window: mods.ctrl || mods.shift,
        });
    } else {
        apply(s, |sel, src| sel.select_only(src, index));
    }
}
