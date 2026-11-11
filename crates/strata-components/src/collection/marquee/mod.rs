//! Marquee (rubber-band) selection (C1d).
//!
//! [`MarqueeMachine`] turns a held drag into a band over a collection's entries, mounted or
//! not, and applies it to the collection's [`Selection`] through C1a's band operations. It
//! also scrolls the view when the pointer is held near a viewport edge. It is plain data with
//! no Rune elements, driven through a [`Surface`]. [`Marquee`] puts it on the collection
//! element's marquee seam (C1e), which hands it the gesture once the pointer controller (C1b)
//! emits "begin marquee" past the drag threshold:
//! `state.controllers.marquee = Some(Box::new(Marquee::new()))`.
//! It follows Strata at `upstream/main` 35ee677ab: `src/ui/marquee.rs`,
//! `src/ui/marquee/state.rs` and `src/ui/marquee/geometry.rs`.
//!
//! # The band
//!
//! The band runs from the press (the anchor) to the pointer. The anchor is held in content
//! coordinates, so it stays on the entry it was pressed on while the view scrolls. The pointer
//! is held in viewport coordinates, as Strata keeps it in the scrolled window's, so a scroll
//! under a still pointer moves the band's far corner through the content. The entries it
//! crosses come from [`CollectionGeometry::range_in`] by index, never from mounted rows, so a
//! band dragged over 10,000 rows selects them all, and rows selected before they scrolled out
//! of view stay selected.
//!
//! The modifiers held at the press pick how the band combines with the selection
//! ([`mode_for`]): plain replaces it, Ctrl toggles each crossed entry against the selection at
//! press, Shift adds to it. The selection from the press is kept until the release, so moving
//! the band back off an entry restores it (C1a).
//!
//! # Auto-scroll
//!
//! Within [`EdgeScroll::edge`] of the viewport's top or bottom, the view glides that way at a
//! speed that grows with the distance past that line, up to [`EdgeScroll::max_speed`] at the
//! edge and beyond ([`edge_speed`], Strata's `auto_scroll_step`). Each tick retargets one Rune
//! glide a short lead ahead ([`glide_lead`]), which settles at that speed, rather than
//! stepping instant scrolls, which commit every frame and which Rune warns about. Leaving the
//! edge band mid-band, or reaching the end of the content, ends the glide on a target that
//! brings it to rest without overshoot ([`stop_lead`]). Ending the band (the release, Escape,
//! another selecting operation) stops the view dead where it shows ([`Outcome::StopScroll`]),
//! as Strata's timer stops, so the last row the band took stays under the pointer.
//!
//! # Time
//!
//! The machine reads the host's clock from the [`Surface`] (virtual in tests) and asks for its
//! next tick with [`Outcome::WakeAfter`]. It wakes only while it glides or has a band update
//! waiting; a still band costs nothing, and a scroll by hand runs frames, and so ticks, by
//! itself.
//!
//! # Decisions and deviations
//!
//! - **D1: the band updates the selection at about 10 Hz while the view scrolls.** A
//!   selection change commits the view, and during auto-scroll a new row enters the band on
//!   most frames. Rune's debug builds warn when one source commits on 6 or more frames and at
//!   least a quarter of a scroll's frames (`rune/src/tooling/perf_warn/commits.rs`). So while
//!   the view moves, the band is applied at most once per [`MarqueeSettings::apply_every`]
//!   (100 ms) and once per [`MarqueeSettings::apply_frames`] frames (6; probed silent at one
//!   commit in 6 frames, warning at one in 4), so it stays silent below 60 Hz too, at a
//!   slower update. While the view is still, every move applies at once, as do the release
//!   and the frame the scroll stops. Strata applies every frame.
//! - **D2: Escape and a cancelled gesture restore the selection the band started from**
//!   ([`Selection::cancel_band`]), as the work package asks. Strata has no marquee Escape, and
//!   its cancelled gesture (`connect_cancel` → `end`) keeps what the band selected.
//! - **D3: the band leaves the cursor and the anchor alone**, as Strata does: it never moves
//!   the preview or quick preview, and never scrolls to the cursor. That keeps Strata's known
//!   gap lgse/strata#1871: a Shift+arrow after a marquee extends from the old range origin.
//! - **D4: vertical auto-scroll only.** The collection scrolls vertically; Columns' horizontal
//!   strip is C2's. Strata scrolls both axes.
//! - **D5: Ctrl+Shift toggles**, as Strata's `band_selection` lets Ctrl win.
//! - **D6: the settings come from the collection's settings** (`Collection::settings`, read
//!   with `Ctx::settings::<MarqueeSettings>()`), C1e's one settings path for every controller,
//!   rather than by accessor: the element keeps its controllers boxed, where no accessor
//!   reaches. The band's live rect is state, not settings, and the adapter keeps it.
//!
//! # A listing that changes under the band
//!
//! After a re-sort or a reload, the same indices name other entries, and the band applies
//! again to the rows now under it, as Strata's band hit-tests by position. The machine sees
//! such a change through the selection's revision, or a cheap fingerprint of the listing (its
//! length and the ids at the ends of what the band crossed). C1e's `Ctx` doesn't expose the
//! host's listing version, so a re-sort that keeps both end ids and the length goes unseen
//! until the band next crosses a new row.
//!
//! # Drawing
//!
//! [`view::band_view`] draws the band from a live rect, in the selection colour, with Look and
//! Kit tokens only; [`MarqueeMachine::drawn`] is the rect to give it. [`Marquee`] hands the
//! element that view for its marquee layer, which sits over the viewport, clipped to it, and
//! rides the scroll offset; it moves the live rect after every input, so a move commits no
//! view. Only showing and hiding the band, and selection changes, do.
//!
//! Work package: C1d, `docs/work/phase-2-composites/C1d-marquee.md`.

#![warn(missing_docs)]

mod adapter;
mod machine;
mod settings;
mod speed;
pub mod view;

#[cfg(test)]
mod tests;

pub use adapter::Marquee;
pub use machine::{MarqueeMachine, Outcome, Surface, mode_for};
pub use settings::MarqueeSettings;
pub use speed::{EdgeScroll, edge_speed, glide_lead, stop_lead};

#[cfg(doc)]
use crate::{collection::selection::Selection, contract::CollectionGeometry};
