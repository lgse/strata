//! Slider (C20a): a horizontal track with snap points, labelled or not, that reports while
//! dragging and on commit (plan §4 C20: the density sliders, text size and thumbnail size).
//!
//! Work package: C20a, `docs/work/phase-1-leaves/C20a-slider-number-field.md`.
//!
//! The host holds a [`SliderState`] (the value and its [`Range`]) and builds a [`Slider`]
//! over it each frame, with its snap points, name, value text and `on_change`. The slider
//! does no I/O and holds no policy: what a value means (Strata's density presets, the
//! thumbnail ladder its shortcuts walk, saving) stays with the host.
//!
//! **Rune 2a44156: partly.** The slider role (`on_step`, `on_step_limit`, `range_value`,
//! `range_step`, `a11y_value`, the set-value action as steps), `on_drag` with its cancel,
//! `on_wheel` that passes on what it doesn't take, an element's `on_key` ahead of the slider's
//! step keys, `on_layout_tx` and `instant_changes` cover it. Missing: rune-kit has no Slider
//! (anomalyco/rune#149), and `on_step`'s page is a fixed 10 steps (U-C13-2), so the page keys
//! go through `on_key`.
//!
//! # What it does
//!
//! - **Pointer.** A press on the knob grabs it where it is; a press on the track jumps the
//!   knob there (GTK's warp). A drag follows the pointer, landing on the step grid (or any
//!   value, [`Slider::free_drag`]), and on a snap point when it comes within `space.lg`
//!   (12 pt) of one, GTK's `MARK_SNAP_LENGTH`. Escape or a lost pointer cancels it back to
//!   where it started. While a press or drag holds it, the knob is engaged: it grows to the
//!   hit height (at most a quarter larger) and takes a `bg_accent` ring, as Strata's
//!   `:active` slider did, until the release or the cancel.
//! - **Keys** (focused): Left and Down a step down, Right and Up a step up (Rune's slider
//!   keys), Page Up and Page Down a page ([`Range::page`]), Home and End the ends. From a
//!   value between grid points a step lands on the next one that way. Shift and the other
//!   modifiers pass the keys on.
//! - **Snap points** ([`Snap`]): a tick under the track at each; a labelled one has a toggle
//!   under its tick that sets it, in the accent colour and pressed while the value is there.
//!   Labels sit centred under their ticks, kept inside the track and `space.md` (8 pt) apart,
//!   as Strata's `SnapLabels` places them, and the slider is at least as wide as the labels
//!   side by side with those gaps, so a menu sized to its content leaves them room.
//! - **Wheel:** passes on to what scrolls behind the slider, unless [`Slider::wheel`] gives an
//!   amount a notch (precise deltas: 40 pt a notch). Even then it reads the vertical delta
//!   only, as GTK does: a sideways wheel (Shift+wheel on Linux, a tilt wheel) and a wheel with
//!   Ctrl, Alt, Cmd or Super held pass on, so Strata's Ctrl+wheel (text size) and
//!   Ctrl+Shift+wheel (thumbnail size) reach the window over the slider.
//! - **Reports** ([`SliderChange`]): `Live` for each move of a drag, `Commit` once it is
//!   released and for every key, snap label, wheel step or assistive action, `Cancel` when a
//!   drag is cancelled. Nothing is reported when the value didn't move.
//! - **Disabled:** dimmed to half, out of the focus order, deaf to input.
//! - **Sizes**, from Look tokens so they follow the text size as Strata's em-based CSS did:
//!   the hit height is `frame.control`, the knob `icon_size.md`, the track a quarter of the
//!   knob, a tick the room under the knob, labels in `kit.ty.small`.
//! - **Colours** (Kit only): the track `border_base`, the fill `bg_accent`, ticks
//!   `border_strong`, the knob `knob` with rune-kit Switch's depth gradient and
//!   `elevation.knob`, the focus ring `border_focus`, labels `text_muted` and the current one
//!   `text_accent`.
//!
//! Assistive technology hears a slider named by [`Slider::label`], with its value, minimum,
//! maximum and step, and the host's [`Slider::value_text`] as its value text ("Compact · 4
//! px"); it steps it, and sets it as steps. Each snap label is a toggle button, pressed at
//! its value.
//!
//! # Bullets
//!
//! Scripts are in `tests/scripts/c20a/`, in-process tests in `crates/strata-gallery/tests/c20a.rs`
//! (`c20a::…`), unit tests in this module's `tests.rs`. Saving, the preset names, other
//! windows following and load clamping are Strata's.
//!
//! `browser/view-modes/density`:
//!
//! | Bullet | Covered by |
//! | --- | --- |
//! | Each slider has tick marks at its Compact and Airy values, with "Compact" and "Airy" buttons under them | the PNG goldens show the ticks and buttons; script `slider-drag-snaps` and unit `a_drag_near_a_snap_point_lands_on_it` check the snapping; a11y goldens the buttons |
//! | The Compact and Airy buttons sit at least 8 px apart. The menu reserves their combined width, such as in Russian at text size 13 or 24 px | unit `a_content_sized_menu_reserves_the_snap_labels_width` (Russian-length labels at 24 px in a content-sized column); PNG goldens |
//! | Clicking "Compact" or "Airy" snaps that slider to the preset; the label of the current preset shows in the accent color | scripts `slider-snap-airy`, `slider-snap-compact`; PNG goldens |
//! | A mouse wheel over a slider scrolls the menu and leaves the value unchanged | script `slider-wheel-passes` (the value); `c20a::a_wheel_over_a_density_slider_scrolls_its_menu` (the scroll) |
//! | Left and Right step a focused slider by 1 px; Page Up and Page Down step it by 4 px | scripts `slider-arrow-right`, `-arrow-left`, `-page-up`, `-page-down`, `-arrow-at-max` |
//! | Each slider's accessible label is "Frame" or "Content", and its accessible value text is its value label | script `slider-names`; a11y goldens |
//! | The snap button of the current preset exposes the accessible pressed state | scripts `slider-snap-airy`, `slider-snap-compact` |
//!
//! `browser/view-modes/icons`: "A wheel notch over the slider moves it 1 px" (the slider's
//! part: [`Slider::wheel`]): script `slider-wheel-steps`. The popover and its other wheel
//! rules are C3's and Strata's.
//!
//! Not from a bullet: the engaged knob while a press holds it (`c20a::a_held_press_engages_the_knob_until_release_or_cancel`;
//! PNG goldens `c20a-engaged-*`), live, commit and cancel reports, a press on the track, Home and End,
//! assistive set-value, and a sideways or modified wheel passing a wheel-taking slider by
//! (`c20a::` tests; unit `a_sideways_wheel_moves_nothing`).
//!
//! # C13
//!
//! C13's media player keeps its own seek and volume sliders for now (its D5). Its volume
//! slider adopts this one in a follow-up (owner ruling): free drag, a 0.1 step with a 0.2
//! page, Home and End, a "60%" value text, cancel and live and commit reports cover it, and
//! the follow-up decides whether it keeps the waveform's colours. Its timeline (and the plain
//! bar's seek slider, the same code) stays its own: a waveform with a live playhead and hover
//! line, a seek held until the clock reports it, chapter ticks and a storyboard bubble.
//!
//! # Rune drafts (not filed)
//!
//! - Reused: U-C13-2 (`on_step` pages are 10 steps), anomalyco/rune#149 (no rune-kit Slider).
//! - **#149, comment draft:** what a kit Slider needs beyond the issue's list, from this one:
//!   snap points with a snap distance (GTK's 12 px), labelled snaps as toggles, a page size
//!   (with U-C13-2), a free drag beside a stepped keyboard, a wheel that's off by default so
//!   the slider's scroll view keeps it, `Cancel` beside live and final reports, and sizes
//!   that follow the text size. Repro of the page gap: a slider with `on_step` and
//!   `range_step(1.0)`, focused, Page Up: `f` hears +10 where Strata wants +4.

mod range;
mod state;
mod view;

pub use range::Range;
pub use state::{SliderChange, SliderState};
pub(crate) use state::{takes_wheel, wheel_notches};
pub use view::{Slider, Snap};

#[cfg(test)]
mod tests;
