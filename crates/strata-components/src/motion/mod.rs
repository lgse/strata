//! Motion effects (C23): the trash flight and the permanent-delete dissolve, built against
//! keys, so they work with any row and any target.
//!
//! Work package: C23, `docs/work/phase-1-leaves/C23-motion-effects.md`.
//!
//! Rune 2a44156: **partly**. `El::matched` flies the icons (one straight path on a
//! spring, drawn above the window), `El::sequence` draws their fades and shrinks and the
//! target's pulses on the render clock, `Live` values move the dissolve's fragments with no
//! commit, and `Cx::window_layout` finds sources and targets by key. No flight can start late
//! or bend (U-C23-1), a `Beat` has no offset or rotation (U-C23-2), and nothing captures a
//! subtree's pixels (U-C23-3).
//!
//! # Using it
//!
//! The host holds an [`Effects`] per window, mounts a [`MotionLayer`] as the last child of its
//! view's root, calls [`Effects::tick`] from its poll every frame, and acts on the [`Event`]s.
//! The effects own no I/O and no policy: when one plays is the host's (Strata plays them only
//! when the operation finishes before its progress card would show: C16's `Dock::complete`
//! returns false).
//!
//! ```ignore
//! // The operation ended before its card showed:
//! let legs = trashed.iter().map(|e| (part_path(scope, e.id, ICON), look.icon(e.icon)));
//! let flight = Flight::into_target(trash_target_key(SIDEBAR), legs)
//!     .target_in(format!("{SIDEBAR}-scroll"))
//!     .rows_in(viewport_path(scope));
//! match s.fx.fly(flight, &s.look, tx) {
//!     // Remove the rows at Launched. Effects overlap: keep what each one is owed by its id.
//!     Some(id) => s.owed.push((id, Owed::RemoveAtLaunch(trashed_ids))),
//!     None => s.remove_trashed(),             // nothing plays: the end state now
//! }
//! // poll:
//! for event in s.fx.tick(now, tx) { … }
//! // view:
//! root.child(MotionLayer::new(|s: &App| &s.fx).ghost(row_ghost).build(s, &s.look, cx))
//! // and C8's Trash icon:
//! sidebar.trash_icon(|el| dress_target(&s.fx, &trash_target_key(SIDEBAR), el))
//! ```
//!
//! - **The flight.** [`Flight::into_target`] flies icons from rows into a target (C8's
//!   `trash_target_key`), which swells and rings as they land. [`Flight::out_of_target`]
//!   flies them from the target onto restored rows, which must be mounted and kept hidden
//!   (C1e's `RowMarks::vanished`) until [`Event::Landed`]. [`Flight::release`] lifts them up off
//!   rows restored in the target's own view while it shakes. At most 7 icons fly, picked
//!   evenly. A leg whose row side is unmounted, off the window or outside `rows_in` flies
//!   nothing; a target that is absent (the sidebar hidden), off the window or outside
//!   `target_in` flies nothing at all. Every flight [`Effects::fly`] starts sends
//!   [`Event::Launched`], [`Event::Landed`] and [`Event::Done`], once each and in order, even
//!   when nothing flies (then all three in one tick), so a restore's hidden rows always show.
//! - **The dissolve.** [`Dissolve::rows`] breaks the rows (C1e's `row_path`) into fragments
//!   that scatter and fade. Mark them vanished in the same handler and keep them listed until
//!   [`Event::Done`], so the rows around them hold still, then remove them (an instant commit
//!   lands the survivors at once). Only rows that show in `within` break up. While one plays,
//!   a second returns `None`. The layer takes no input, so clicks reach the view under it.
//! - **Off.** With `look.animations` false, [`Effects::fly`] and [`Effects::dissolve`] return
//!   `None` and the host applies the end state at once. Under Rune's reduced motion the layer
//!   draws nothing and each effect ends at its next ticks, with all its events.
//! - **Assistive technology** hears nothing of the effects: the layer is hidden from it and
//!   takes no focus. A vanished row leaves the tree; the rows' removal is the host's listing.
//! - **Commits.** A flight commits the host's own call, its flip (with the host's row removal
//!   at Launched), its landing and its end, and nothing per frame: the spring and the
//!   sequences run on Rune's clock. A dissolve commits its start and its end; in between the
//!   tick moves its fragments' live values every frame (one poll a frame, no view).
//!
//! # Host rules
//!
//! - **Don't flip on an instant commit.** `Tx::instant` (or a settle) on the commit that
//!   answers [`Event::Launched`] turns the matched swap off: the icons appear at their landing
//!   places. The test `an_instant_flip_flies_nothing_at_the_pin` pins it. An instant commit
//!   later in the 0.4 s flight keeps the flight when it leaves the flyers' layout as it was
//!   (Rune `5a9f64e8`, `runtime/commit/apply.rs`); one that moves a flyer, a settle, or a flyer
//!   with `instant_changes` still lands it. `f7bf9e04` (a matched flight starts from where it
//!   was seen after an earlier commit in the frame) is in the pin too, not re-checked here.
//! - **Remove the rows at Launched**, not before: the effect reads their rects first.
//! - **Start an effect from a handler, or in a poll that commits** (returns true), before or
//!   after the tick: the layer must draw it before it moves, and a poll that doesn't commit
//!   runs no view. It waits for that draw for 3 ticks at most (no layer mounted), then ends.
//! - [`Effects::settle`] ends everything at once (a tab or view-mode switch under it), with
//!   each effect's owed events. It leaves the targets' pulses as they are: a pulse playing
//!   plays out, and [`dress_target`] keeps the target in its wrapper.
//! - [`dress_target`] wraps the target from its first pulse on, for good: the target's
//!   element path gains a `c23-pulse-<n>` step then, and the wrapper's key changes with each
//!   pulse, which mounts the target afresh.
//!
//! # Deliberate changes (owner rulings)
//!
//! 1. **A straight path.** Strata's icons arc above the line between row and target; a
//!    matched flight travels one straight screen path (U-C23-1).
//! 2. **Staggered by arrival.** Strata starts each icon 12 ms after the one before (up to
//!    36 ms); here they leave together on springs 12 ms longer each, so they land in turn
//!    (U-C23-1).
//! 3. **Fades and shrinks as sequence beats** that follow Strata's per-frame curves and CSS
//!    keyframes. A beat has one easing for its opacity and its scale, so where Strata's two
//!    curves differ (a flyer's fade and its shrink) the beats sample them every 15–20 ms with
//!    straight segments between. The rings take the accent alphas (0.16–0.34) of Strata's
//!    box shadows and their keyframe easings, but a ring is a 1.5 pt border that scales, not a
//!    spread shadow, and one opacity drives its border and its glow: the release's glow-only
//!    middle shows a faint border, and the impact's start a faint glow.
//! 4. **The release has no burst**, and its shake is scale only (U-C23-2).
//! 5. **Fragments are pieces of a ghost** of the row, the host's look for its entry, not of its
//!    rendered pixels (U-C23-3). With no ghost they are flat shards drawn with `canvas`.
//! 6. **Survivors stay live** (hover, clicks) during the dissolve; Strata freezes the view as a
//!    snapshot. Fragments hold their place in the window if the list scrolls under them, as
//!    Strata's snapshot does.
//! 7. **The host holds the source rows one frame** after [`Effects::fly`] (until Launched).
//! 8. **A restore's survivors don't freeze**: the restored rows are listed and hidden at
//!    launch, and show at Landed (partial `operations/trash/animation` bullet).
//! 9. **No rune-fx compositor.** The WP asked to measure fragments in a `rune-fx` compositor
//!    slot against `canvas`. The owner ruled the compositor out on integration cost without
//!    a measurement: it needs `rune::run_with` in every host, wgpu in strata-components, the
//!    shared gallery spec, and every golden rendered through it. The two measured candidates
//!    are ghost pieces and canvas shards (perf notes below).
//!
//! # Bullets
//!
//! | Bullet | Where |
//! | --- | --- |
//! | **trash/animation** A Move to Trash that succeeds before its card flies its icons from their rows into the sidebar Trash button | tests `a_flight_launches_flies_lands_and_cleans_up`, `flyers_close_on_the_target_and_land_in_turn` (the flight); script `trash-flight` (end state only); goldens `c23-flight`; when is **Strata policy** (C16) |
//! | … still running when its card appears never flies; fails, partly fails or is cancelled: no flight; a failing foreground operation leaves it to play; a failed restore: no flight; restore only after the confirmation | **Strata policy** |
//! | Restoring while viewing Trash lifts the icons off their rows and launches them upward with a burst | **partial**: lift and fade, no burst; test `a_release_lifts_off_the_rows_and_shakes_the_target` (the lift); script `release` (end state only) |
//! | Restoring into a normal folder flies icons from the Trash button onto the restored rows | test `a_restore_flies_out_onto_the_rows_and_lands_before_they_show`; script `restore-flight` |
//! | During that flight the pre-restore listing stays frozen, and the restored rows appear when the icons land | **partial**: restored rows hidden until Landed, survivors not frozen (deviation 8) |
//! | When restore icons leave the Trash button it plays a release pulse; restores in Trash shake it | tests (release, restore); review scene `c23-pulse` |
//! | With animations disabled, no flight plays and the operation still completes | test `animations_off_play_nothing`; script `trash-off`; goldens `c23-off` |
//! | **delete/dissolve** The deleted rows that were visible break into fragments that scatter and fade | tests `a_dissolve_breaks_only_the_rows_that_show_and_ends`, `nothing_commits_while_a_dissolve_plays`; script `dissolve`; goldens `c23-dissolve` |
//! | A permanent delete that succeeds before its card dissolves its rows, and no card appears | the dissolve as above; when is **Strata policy** |
//! | While it runs, clicks reach the file view | test `the_layer_takes_no_clicks`; script `dissolve` (a survivor clicked mid-dissolve) |
//! | A second quick delete while the first dissolve plays plays no dissolve, and the view takes clicks after | test `a_second_dissolve_while_one_plays_plays_nothing`; script `dissolve-twice` |
//! | Surviving rows show at their pre-delete positions, then at their new positions | host's (vanished marks, removal at Done): the gallery does it; script `dissolve` |
//! | A failed, partial or cancelled deletion plays no dissolve | **Strata policy** |
//! | With animations disabled, no dissolve plays and the listing updates at once | test `animations_off_play_nothing`; script `dissolve-off` |
//!
//! # Rune drafts (not filed)
//!
//! - **U-C23-1: a matched flight can't start late or bend.** Two `El::matched` flyers swapped
//!   in one commit can differ only by their springs: a staggered departure takes a commit per
//!   flyer, and an arcing path (a file thrown into a Trash button) can't be had. Repro: mount
//!   two tiles with `.matched("a")`, `.matched("b")`, swap both to a corner in one commit with
//!   `.motion()` springs; there is no way to start "b" 12 ms after "a" or lift its path.
//!   Asks: a start delay on `.motion()`, or a path bend (`matched_arc(height)`).
//! - **U-C23-2: a `Beat` has opacity and scale only.** A shudder (±8° with a scale) or a lift
//!   can't be a finite sequence; `El::rotate` springs on commits instead.
//! - **U-C23-3: an element's drawn pixels can't be captured.** A dissolve (or a motion-blur
//!   node layer, rune-fx's own gap) needs a subtree rendered into `Pixels` or a texture; only
//!   an `effect_backdrop` slot copies the whole target up to a point, and only inside a
//!   compositor.
//! - **U-C23-4: a handler can't read a layout.** `Tx` has no `presented` or `layout`, so an
//!   effect started from a handler spends a frame capturing where its sources are.
//!
//! # Performance
//!
//! Release builds, headless, `motion::tests::perf` (ignored; run with `--ignored`).
//! See the WP's handoff for the numbers kept.

mod dissolve;
mod flight;
mod layer;
mod model;
mod pulse;

#[cfg(test)]
mod tests;

pub use dissolve::{DURATION, Dissolve, MAX_FRAGMENTS};
pub use flight::{
    Flight, MAX_FLYERS, MAX_STAGGER, Mode, RELEASE_TRAVEL, RESTORE_TRAVEL, STAGGER, TRAVEL,
    flyer_key, launch_key,
};
pub use layer::{LAYER, MotionLayer};
pub use model::{EffectId, Effects, Event};
pub use pulse::{BOUNCE, PulseKind, RECEIVING, SHAKE, dress_target, ring_key};
