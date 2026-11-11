//! ColorPicker (C20b): a saturation and value plane, a hue strip, an optional alpha strip, a
//! hex field and rows of swatches, inline or in a popover behind a button. It replaces GTK's
//! `ColorDialogButton` (Strata's theme editor) and `ColorChooserWidget` (its custom folder and
//! file colours).
//!
//! Work package: C20b, `docs/work/phase-1-leaves/C20b-color-picker.md`.
//!
//! The host holds a [`ColorPickerState`] and builds a [`ColorPicker`] over it each frame, with
//! its swatch rows as data and its `on_change`, calling [`ColorPicker::poll`] from its poll.
//! The picker does no I/O and holds no policy: what a colour is for, saving it, previewing it
//! and the recent colours list are the host's ([`remember`] keeps one as GTK does).
//!
//! **Rune 2a44156: partly.** Gradients (`linear_gradient`, `gradient`), `Shape` fills, the
//! slider role (`on_step`, `on_step_limit`, `range_value`, `a11y_value`), `on_drag` with its
//! cancel, radio groups (`focus_group`), `El::modal` (Tab trap, first-stop focus, the opener's
//! focus back, a press outside) and core `text_input` cover it, as do `rune::Color` and its
//! sRGB transfer functions. Missing: no picker in rune-kit (no issue); no two-dimensional
//! control for assistive technology (U-C20b-3); rune-kit's `Popover` has no Escape or initial
//! focus (U-C20b-1); a modal's dismissal can't tell Escape from a press outside (U-C20b-2);
//! no focus-change hook (anomalyco/rune#106). `El::modal_tx` (22aa8cde) gives the dismissal a
//! `Tx`. Not adopted: `Cx::force_focus_rings` (42f2d354, which would let a golden show focused
//! swatches) and 16-stop gradients (c9cd9df3); this needs 7.
//!
//! # The colour
//!
//! [`Srgba`] is what Strata saves: 8-bit sRGB with straight alpha, compared exactly. It
//! converts to and from `rune::Color` (linear sRGB, straight alpha): to it exactly, from it
//! rounded to the nearest 8-bit step as `Color::hex` rounds. Its hex text is `#rrggbb`, or
//! `#rrggbbaa` when translucent, lowercase; it reads `#RGB`, `#RGBA`, `#RRGGBB` and `#RRGGBBAA`
//! in either case, with or without the `#`.
//!
//! The state keeps the colour as [`Hsv`] (over the sRGB-encoded channels, as GTK and CSS), so a
//! grey keeps its hue and black its saturation: a drag through the plane's left or bottom edge,
//! a grey swatch, or a grey the host sets don't send the hue back to red.
//!
//! # What it does
//!
//! - **The plane** (saturation across, value up): drawn as the pure hue with white fading out
//!   to the right over it and black fading in downwards over that, which sRGB blending
//!   composites to HSV exactly. Probe (c20b-opaque-light at scale 1, 349 plane and 188 hue
//!   samples against `Hsv → Srgba`): worst 1/255 away from the cursors' shadows, 2/255 at
//!   their edge; so no `Pixels` fallback. Not checked in a window that blends in linear light
//!   (`srgb_blend(false)`), where the strips' gradients may interpolate differently. A press
//!   jumps the
//!   cursor there and a drag follows it; Escape or a lost pointer puts the colour back. Arrows
//!   move a whole percent from the nearest one, Alt+arrows ten (GTK's 0.01 and 0.1): Left and
//!   Right the saturation, Up and Down the value. So from an off-grid start (a drag, a
//!   swatch) Right then Left lands on a whole percent, not back on the start. Shift, Ctrl,
//!   Super and Cmd arrows pass on.
//! - **The strips:** hue in whole degrees, alpha in whole percents, each a slider whose track
//!   draws what it sets (the hue's seven stops; the colour fading in over a checkerboard).
//!   Press and drag as the plane; Rune's slider keys step one (Left and Down down, Right and
//!   Up up), Page Up and Page Down ten, Home and End the ends. The wheel passes on.
//! - **The hex field:** shows the colour and follows every change. Typed text commits on
//!   Return and when the field loses the keyboard (the host's poll), and before any other
//!   part changes the colour (as GTK applies its entry on focus-out); text that isn't a colour
//!   reverts, and is marked invalid while typed. Without alpha, eight digits drop theirs.
//!   Escape (or Shift+Escape) with typed text reverts it; the next Escape goes on. A right-click, the Menu key
//!   or Shift+F10 opens the host's C21 `FieldMenu` ([`ColorPicker::on_menu`]).
//! - **Swatches** ([`SwatchRow`]): a heading over a row of dots, a radio group named by it;
//!   each dot is checked while it is the colour (compared opaque without alpha). A row is one
//!   Tab stop (the checked dot, else the first); arrows move along it and pick the dot they
//!   reach, as Rune presses a radio its group's arrows reach (D11); Space, Return or a click
//!   picks one.
//! - **Tab** goes plane, hue, alpha, hex field, then each swatch row (one stop a row).
//! - **The button** ([`ColorPicker::button`]): shows the colour, named by the host's label with
//!   the hex as its description. It opens the picker in a rune-kit Popover with focus on the
//!   plane. Changes apply as they happen; Escape or Shift+Escape (Rune's modal dismisses on
//!   both) closes it and puts back the opening colour (`Cancel`), a press outside closes it
//!   and keeps the colour; focus goes back to the button and [`ColorPicker::on_close`] hears
//!   whether the colour changed.
//! - **Reports** ([`ColorChange`]): `Live` for each move of a drag, `Commit` when it settles
//!   (a release, a key, a hex commit, a swatch, an assistive action, a press outside the
//!   popover), `Cancel` when a drag is cancelled or the popover closes on Escape. Nothing is
//!   reported when the colour didn't change. The host saves `Commit` and `Cancel` alike: in
//!   the popover a `Cancel` follows the `Commit`s it undoes.
//! - **Disabled:** dimmed to half, out of the focus order, deaf to input; the button doesn't
//!   open, and an open popover closes from the host's poll, keeping the colour.
//! - **Sizes**, from Look tokens: the picker ten `frame.control` wide (240 at 13 px; the
//!   popover's 320 less its padding at most), the plane six `frame.control` tall plus a
//!   cursor, the strips `frame.control`, cursors `icon_size.md`, swatches `icon_size.lg`
//!   (Strata's 20 px dots), the checkerboard `space.sm` squares.
//! - **Colours:** Kit tokens only, but for what the colour itself draws (the hues, the plane's
//!   white and black): the cursors' ring `knob` with `elevation.knob`, the focus ring
//!   `border_focus`, the checkerboard `bg_base` and `border_base`, the swatch borders
//!   `border_base`, the current swatch's ring `text_base`, headings `text_muted`, the field
//!   as C20a's NumberField draws its own.
//!
//! Assistive technology hears the plane as one slider, "Saturation and value", valued
//! "Saturation 80%, value 60%" (no numeric range: there is no two-dimensional role); the
//! strips as sliders "Hue" ("200°") and "Alpha" ("50%") it can step and set; the hex field
//! "Hex color" with its text; each swatch row a radio group, each swatch a radio named by the
//! host's label or its hex. Words come from [`PickerWords`] for Strata to translate.
//!
//! # Perf
//!
//! A drag runs the picker's view again on each move (Rune lands drag commits at once); the
//! plane's only changing paint is its hue fill. Keys call `Tx::instant`, so colours don't
//! spring. `c20b::a_hue_drag_commits_once_a_move` measures it: 200 hue moves on the gallery
//! page, 200 commits, move and frame p50 1.36 ms, p95 1.56 ms (debug build). The `plane-drag`
//! script under `RUNE_COMMIT_BUDGET_MS=8` warns of nothing (frame CPU p95 2.0 ms, debug).
//!
//! # Bullets
//!
//! Scripts are in `tests/scripts/c20b/`, in-process tests in `crates/strata-gallery/tests/c20b.rs`
//! (`c20b::…`), unit tests in this module's `tests.rs`. The dialogs around the picker
//! (Strata's Customize and custom-colour modals, its theme editor), saving and previewing are
//! Strata's.
//!
//! | Bullet | Covered by |
//! | --- | --- |
//! | `settings/themes`: In that panel the name field is named "Theme name", and each color picker is named after its row label | script `names` (the buttons; the name field is Strata's); a11y goldens |
//! | `settings/themes`: The editor saves each picked color as `#rrggbb` | scripts `hex-entry`, `swatch-click` (the hex the picker reports without alpha); unit `hex_is_lowercase_and_six_digits_when_opaque` |
//! | `settings/themes`: Add theme opens … 14 color pickers seeded from the selected theme | **Partial**: buttons seeded from the state (PNG goldens); the panel is Strata's |
//! | `settings/themes`: Each picker change previews across the interface | **Partial**: `Live` and `Commit` reports (`c20b::a_plane_drag_reports_live_then_one_commit`; scripts `plane-arrow-right`, `plane-arrow-up`, `plane-alt-arrow-down`, `hue-arrow-right`, `hue-end`, `alpha-arrow-left`, `swatch-click`, `swatch-arrow`); the preview is Strata's |
//! | `settings/themes`: A swatch's color chooser draws its … hex entry, sliders … in the active theme | **Partial**: Kit tokens only, light and dark goldens, scripts `names`, `names-popover`, `tab-order`; no Cancel and Select buttons (D2) or value popups (D3) |
//! | `settings/themes`: Editor color swatches keep rounded corners … | **Partial**: the PNG goldens, at rest only (`Cx::force_focus_rings` could show them focused; not adopted) |
//! | `browser/folder-customization`: The custom-color button opens … a color chooser that recolors the dialog's header icon live | **Partial**: `Live` reports while dragging (script `plane-drag`, the page's preview dot); the dialog is Strata's |
//! | `browser/folder-customization`: Apply saves the chosen color as `#rrggbb` | **Partial**: the reported hex (scripts `hex-entry`, `plane-drag`); Apply is Strata's |
//! | `browser/folder-customization`: Closing the custom color dialog by … Escape, or a click outside it returns focus to the custom color button | **Partial**, for the popover: scripts `popover-escape`, `popover-shift-escape`, `popover-outside`; Strata's dialog is C15's |
//! | `browser/folder-customization`: In the custom color dialog, Escape from the editor returns to the palette … | N/A: one page (D1) |
//! | `browser/folder-customization`: The custom color dialog's back button is named "Back to palette" | N/A: one page (D1) |
//! | `browser/folder-customization`: The custom color editor's spin buttons show Strata's bundled plus and minus icons | N/A: no value popups (D3) |
//! | `settings/themes/icons`: The spin buttons in the Customize… custom color editor draw the bundled − and + steps | N/A: no value popups (D3) |
//! | `settings/themes`: A keyboard-focused Select button in a swatch's color chooser shows a 3 px accent halo | N/A: no Select button (D2) |
//!
//! Not from a bullet: hex forms and reverts (scripts `hex-short`, `hex-invalid`, `hex-escape`),
//! Escape cancelling a drag, Escape and Shift+Escape in the popover and its field, the host
//! saving a popover's `Cancel`, modified plane arrows passing on, typed hex committing before
//! a plane key or a swatch, assistive increments, the blur commit, the field menu's hold, the
//! disabled state and disabling an open popover (`c20b::`), hue kept through black (unit
//! `a_drag_through_black_keeps_the_hue`).
//!
//! # Deviations
//!
//! - **D1** (OWNER ruling): one page, not GTK's palette and editor pages; no "+" custom
//!   button, no "Back to palette".
//! - **D2** (OWNER ruling): the popover applies changes live; Escape puts back the opening
//!   colour, a press outside keeps it. GTK's `ColorDialog` is a modal window with Cancel and
//!   Select.
//! - **D3** (OWNER ruling): no H, S, V and A value popups (follow-up, with C20a's
//!   NumberField, which also gives assistive technology exact values).
//! - **D4**: no colour pick from the screen (GTK's eyedropper).
//! - **D5**: the hex field takes hex only, not colour names or `rgb()` (GTK's takes what
//!   `gdk_rgba_parse` does); it also takes `#RGBA` and hex without `#` (OWNER ruling).
//! - **D6**: a grey keeps its hue (GTK's goes to 0), as the work package asks.
//! - **D7**: lowercase hex, eight digits only when translucent (GTK shows `#RRGGBB`); with
//!   alpha off eight typed digits drop their alpha (OWNER ruling; GTK ignores typed alpha).
//! - **D8**: the hue strip runs across under the plane (GTK's stands at its left), to fit the
//!   popover.
//! - **D9**: the host's poll commits the hex field on a blur (#106) and reports a press
//!   outside the popover, a frame late (U-C20b-2).
//! - **D10** (OWNER ruling): hue steps a degree and pages ten, alpha a percent and ten (GTK's
//!   hue steps 3.6° and pages 36°).
//! - **D11**: arrows in a swatch row pick the colour they reach (Rune presses a radio its
//!   group's arrows reach, as ARIA's radio groups do); GTK's palette arrows only move focus.
//! - **D12** (OWNER ruling): the plane is one slider with a value text, no numeric range or
//!   set-value (U-C20b-3).
//! - **D13** (OWNER ruling): Escape in the hex field reverts typed text first.
//! - Core `text_input`, not rune-kit's `TextInput`, for C21's field menu (U-C21-2).
//! - The hex field's blur-and-hold logic is a copy of C20a's NumberField's (OWNER ruling;
//!   follow-up to share it).
//!
//! # Rune drafts (not filed)
//!
//! - **U-C20b-1, rune-kit's Popover has no Escape or initial focus.** Its content is a
//!   non-modal `dialog` with a scrim for presses outside; Escape does nothing and nothing in it
//!   takes focus (its README: "popover/select opening autofocus is still separate"). Repro: a
//!   `Popover` with a focusable child, opened by a button press: Escape leaves it open, Tab
//!   reaches the page behind it. Ask: `Popover::modal(true)` (Escape closes, Tab stays in, the
//!   first stop focused, the trigger focused on close). Here: `El::modal` on the body.
//! - **U-C20b-2, a modal's dismissal can't say why.** `El::modal_tx`'s closure gets the
//!   change's `Tx` (22aa8cde) but no reason: Rune calls it for Escape and for a press outside
//!   alike. A picker that cancels on Escape and keeps on a press outside takes Escape in an
//!   ancestor's `on_key` first. Ask: `modal_with(|s, why: Dismiss, tx| ..)`.
//! - **U-C20b-3, no two-dimensional control for assistive technology.** AccessKit (and ARIA)
//!   have no 2D slider: a saturation and value plane is one slider with a value text and no
//!   range, set-value or increments. Ask how a two-axis control should expose its two values
//!   and steps (two child value nodes on one focus stop, say).
//! - Reused: anomalyco/rune#106 (no focus-change hook), U-C21-2 (rune-kit fields take no menu
//!   press).

mod state;
mod swatch;
mod value;
mod view;
mod words;

pub use state::{ColorChange, ColorPickerState};
pub use swatch::{RECENT_MAX, Swatch, SwatchRow, remember};
pub use value::{Hsv, ParseHexError, Srgba};
pub use view::ColorPicker;
pub use words::PickerWords;

#[cfg(test)]
mod tests;
