//! Pointer intent and click modes (C1b).
//!
//! [`PointerMachine`] turns presses, moves and releases on a collection into selection
//! operations on its [`Selection`] and [`Outcome`]s: open, a plain click (where previews
//! follow), a file drag, a marquee, a rename, a new tab or window, a context menu. It is plain
//! data with no Rune elements, driven through a [`Surface`]. [`PointerIntent`] puts it on the
//! collection element's pointer seam (C1e). It follows Strata at `upstream/main` 35ee677ab:
//! `src/ui/pointer.rs`, `src/ui/collection_interaction.rs`, `src/ui/browser_modes/clicks.rs` and
//! `src/ui/browser/columns/rows/click.rs`.
//!
//! # A primary press
//!
//! The modifiers are the ones held at the press: letting go of Ctrl or Shift before the button
//! changes nothing. What the press does depends on where it lands ([`Spot`]):
//!
//! | Press on | Plain | Ctrl | Shift | Past the threshold |
//! | --- | --- | --- | --- | --- |
//! | An entry's name or content | Selects it alone, now; inside a multi-selection, keeps the group | Unselected: adds it now. Selected: toggles it off on release (a deviation, below) | Ranges now | A file drag of the selection |
//! | Blank space on a selected entry | As content | As content | As content | A file drag of the selection |
//! | Blank space on an unselected entry | Selects it alone, now | Toggles on release | Ranges on release | A marquee |
//! | Anything, with Alt held | As without Alt | Toggles on release | Ranges on release | A marquee |
//! | Empty space | Clears the selection, now | Nothing | Nothing | A marquee |
//!
//! Ctrl+Shift ranges by [`PointerSettings::ctrl_shift`], [`Extend::Replace`] by default, as
//! Strata's `pointer_selection` does. A drag needs [`PointerSettings::drags`].
//!
//! Selecting an entry the moment it is pressed is Strata's: a drag then shows and carries its
//! real source. Where a drag is a marquee instead (blank space on an unselected entry, or any
//! press with Alt), Ctrl and Shift wait for the release: the band applies them to the
//! selection as it was before the press, which Strata takes when the band's press begins
//! (`src/ui/marquee/state.rs`), before the row's own press changes anything.
//!
//! **Deviation:** Strata toggles a Ctrl-pressed entry at the press even when it is selected,
//! so a Ctrl-drag from a selected entry drags the selection without it. Here that toggle
//! waits for the release, and a drag keeps the entry: "Ctrl-drag … adds it to the selection".
//!
//! Which action a drag asks for (Ctrl copies, Shift moves) is the host's: it reads `tx.mods()`
//! when it hears `Intent::BeginDrag` (X2 and P1), as Strata's `drag_actions_for_modifiers`
//! (`src/ui/browser/clipboard.rs`) does.
//!
//! # Threshold
//!
//! A press becomes a drag or a marquee once it travels more than
//! [`PointerSettings::drag_threshold`] along either axis, as GTK and Strata measure it. After
//! that it never clicks: no opening, no preview, no rename, even if the pointer comes back.
//! Neither does a cancelled gesture, nor a press whose entry moved during it (a re-sort or a
//! reload), which is Strata's "recycled row" rule.
//!
//! # Click modes
//!
//! On a plain release, in Strata's order:
//!
//! 1. A plain press that kept a multi-selection collapses it to the entry. Strata's Columns
//!    does it in its own release path. In Icons and List, Strata claims only modified releases
//!    (`PointerSequence::install` in `collection_interaction.rs`), so a plain release reaches
//!    GTK's own list-item gesture, which selects the item alone on release; a drag cancels
//!    that gesture first, so the group still drags (`browser_modes/clicks.rs`: "Capture must
//!    snapshot selection before GTK changes it for the bubble gesture").
//! 2. The entry opens ([`Outcome::Activate`]) on the first click where its kind opens on one
//!    ([`ClickModes`]), or on the second click of a run where it opens on two. Alt doesn't
//!    stop it, as Strata's `plain_single` reads only Ctrl and Shift.
//! 3. Otherwise a slow click may arm a rename (below).
//! 4. Otherwise a first click sends [`Outcome::Clicked`].
//!
//! A Ctrl or Shift click never opens. A press continues a run (2, 3, …) when it comes within
//! the double-click interval and distance of the one before, as GTK counts it; Strata acts on
//! the first and second press of a run only, so a third press opens nothing, renames nothing
//! and sends no click. Runs are counted here, from [`PointerSettings::double_click`]: Rune's
//! own count uses a fixed 0.5 s on Linux (S5 U13).
//! The click modes themselves, and keeping them across restarts, are the host's preferences.
//!
//! # Slow-click rename
//!
//! A plain first click on the name of the entry that was the only one selected before the
//! press arms a rename, when the click doesn't open it (or the entry is
//! [`PointerSettings::already_open`]) and [`PointerSettings::rename`] is on. An automatic pick
//! ([`Selection::auto_select`]) counts as that only selection, as GTK shows it selected in
//! Strata. Alt and Super, which stop it, are the ones held at the press. The rename comes one
//! double-click interval later from [`PointerMachine::tick`], so
//! that a second press makes a double click instead. Any press, any key the collection hears
//! (`PointerController::key_seen`), a reset (`PointerController::reset`: another folder, or a
//! host that saw a press or a key elsewhere, such as another Columns column) or any selection
//! change cancels it.
//!
//! **Deviation from the work package doc:** it asks for the second click "before a rename
//! timeout". Strata has none: a click on the sole selection's name renames however long after
//! the first it comes (`starts_slow_rename` in `browser_modes/clicks.rs`,
//! `is_slow_click_rename` in `columns/rows/click.rs`). The only timer is the one-interval wait
//! above (`schedule_click_rename` in `browser/inline_edit.rs`).
//!
//! # Middle and secondary presses
//!
//! A middle click (released within the threshold, the first of a run) opens a folder in a new
//! tab, or a new window with Ctrl or Shift held at the release, as Strata reads them then. On a
//! file it selects the file and opens nothing. Strata reveals a file in its own folder, which
//! is this one except in recursive search results; that reveal is left to C17 and C18.
//!
//! A secondary press selects an unselected entry alone and keeps a selection it lands in, then
//! asks for the menu ([`Outcome::ContextMenu`]). Where the menu goes is C21's.
//!
//! # On the element
//!
//! Install [`PointerIntent`] as the collection's pointer controller, and pass the
//! [`PointerSettings`] with `Collection::settings` each time the view builds: the interval from
//! P5, the click modes from the preferences, `already_open` from Columns. With none, the
//! defaults apply. A read-only collection (`Collection::read_only`, C14b) neither drags nor
//! renames: a drag from an entry starts neither a file drag nor a band (an Alt drag still
//! bands), and a Ctrl press on a selected entry that then drags neither toggles nor drags.
//! Settings arrive by type through the element's `Ctx::settings`, its one settings path for
//! controllers, so C1b keeps no handle of its own.
//!
//! The host must give the element its clock (`Collection::clock`): C1b times double clicks and
//! the slow-click rename by it. Without one every input reads time 0, so any second press on an
//! entry pairs into a double click and a rename never comes due.
//!
//! The element hit-tests: a press's `Region` becomes the [`Spot`] (a region C1b doesn't know
//! is gutter). A file drag starts only from the `Intent::BeginDrag` C1b sends past the
//! threshold, with the pointer's window point; rows have no drag source (C1e's decision). A
//! context menu's anchor is a zero-size rect at the press.
//!
//! # Hand-off to the marquee (C1d)
//!
//! C1b decides when a press becomes a band. On the move that takes a primary press past the
//! threshold, it emits `Effect::BeginMarquee { at, mods }`, with `at` the press in content
//! coordinates and `mods` the modifiers held at the press, when the press was on:
//!
//! - empty space (a plain one has already cleared the selection, as Strata does at the press);
//! - blank space beside an entry that wasn't selected (a plain one has already selected that
//!   entry alone; a Ctrl or Shift one has changed nothing);
//! - anything, with Alt held.
//!
//! The element then calls `MarqueeController::begin(at, mods)` and `moved` with that move's
//! point, and routes the rest of the gesture to the marquee while it is active. C1b is done
//! with the gesture: its release never reaches C1b, and C1b's next press starts afresh. How
//! `mods` maps to a band mode (Ctrl toggles, Shift adds) is C1d's.
//!
//! With Alt held, a plain press has selected the entry alone or kept its multi-selection, as
//! without Alt; a Ctrl or Shift one has changed nothing.
//!
//! # Hand-off to Columns (C2)
//!
//! Strata cancels a pending slow-click rename on a row press in any column
//! (`ViewState::cancel_click_rename`, `src/ui/browser/inline_edit.rs`). C2 does the same by
//! calling `reset` on the other columns' pointer controllers. **Seam request for C2's planner:**
//! C1e has no "a press happened here" hook, so a press that changes no selection and sends no
//! intent (a Ctrl press on blank row space, say) reaches C2 through no callback; C2 needs one
//! from the element to reset the other columns on every press.
//!
//! # Left to other work packages
//!
//! Bullets of the `pointer-intent` and `click-modes` nodes C1b doesn't decide:
//!
//! - Icons: the gutter beside the thumbnail inside a card is blank space: C3's regions.
//! - List: the unused Name-column space and its padding are blank, other cells content: C4's
//!   regions.
//! - The click-mode preference, its defaults and persistence, and the single-click preview
//!   preference: the host's.
//! - Arrow keys select without opening: C1c.
//! - Selection after opening a folder: the host's, from `Activation`.
//!
//! # Tests
//!
//! `tests.rs` holds one test per bullet the machine decides; `tests/element.rs` drives it
//! through the element, with modifiers held in process (written before Rune scripts could hold
//! them with `{"hold": …}`, Rune `4805095`). The page scripts in
//! `tests/scripts/c1/` use the page's default settings (two clicks open everything).
//! `middle-dragged` checks only the dragged half of its bullet: the page's intent label can't
//! count tabs, so the double middle click is `a_double_middle_click_opens_one_tab_and_a_dragged_one_none`.

//! Work package: C1b, `docs/work/phase-2-composites/C1b-pointer-intent.md`.

#![warn(missing_docs)]

mod adapter;
mod clicks;
mod machine;
mod rename;
mod settings;

#[cfg(test)]
mod tests;

pub use adapter::PointerIntent;
pub use machine::{ActivateBy, Button, Outcome, PointerMachine, Press, Release, Spot, Surface};
pub use settings::{ClickCount, ClickModes, PointerSettings};

#[cfg(doc)]
use crate::collection::selection::{Extend, Selection};
