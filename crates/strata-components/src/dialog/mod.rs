//! StrataDialog: the modal shell every Strata dialog builds on, with its backdrop, motion,
//! dismissal, initial focus, and Enter and Escape (C15, `app/dialogs`).
//!
//! Work package: C15, `docs/work/phase-1-leaves/C15-strata-dialog.md`.
//!
//! Two builders:
//!
//! - [`StrataDialog`] is an action dialog: a header with an icon, a title, a subtitle, an
//!   optional spinner and a close button; a body; and a footer of buttons, the primary last.
//!   Every action dialog uses it: delete confirmations, conflicts, Connect to server,
//!   passphrases, Format, Open With, the update dialog, C26's mount prompts.
//! - [`DialogShell`] is what both share: the blurred backdrop, open and close motion,
//!   dismissal, the size rules and the Enter routing, around any panel. The search palette
//!   (C17), the shortcut reference (C19) and Settings draw their own panel in it.
//!
//! Both push into the overlay layer `rune_kit::root(base, layer)` mounts, as rune-kit's
//! overlays do, and take the look as `&Look` at build time. Handlers are `Fn(&mut S)`
//! closures; the component holds no state of its own. Strata's policy (which form counts as
//! dirty, where focus goes after a dialog that changes the listing) stays with the caller.
//!
//! # Why not rune-kit's `Dialog`
//!
//! The work package says to wrap `rune_kit::Dialog`. It builds on Rune's primitives instead
//! (`El::modal`, `El::backdrop_blur`, presence `Transition`s, `El::default_button`,
//! `El::autofocus`), because rune-kit's `Dialog` at the pinned rev:
//!
//! - has a fixed 368 pt height, or `fit()`, which doesn't scroll, so a long dialog can't stay
//!   inside the window;
//! - hardcodes its radius, padding and title type instead of reading the `Look`;
//! - has no footer slot, no icon, no danger tone and no spinner;
//! - builds its scrim internally, so it can't take `backdrop_blur`;
//! - has no scale-and-fade motion and no switch for the animations setting;
//! - puts its close button first in Tab order, which makes the close button, not the first
//!   field, the dialog's initial focus.
//!
//! # Keyboard
//!
//! - **Escape** calls `on_dismiss`, from anywhere in the dialog, blocking or not. Rune's modal
//!   handles it. A blocking dialog's caller decides what dismissal means (C16 docks its
//!   progress dialog).
//! - **Enter** on a focused button, checkbox, switch or other non-text control presses that
//!   control, so Enter with Cancel focused cancels (Strata's rule, which differs from AppKit's,
//!   where Return always goes to the default button). The panel's key handler turns the Enter
//!   into a Space press, which Rune applies on the next frame.
//! - **Enter** in a single-line field, or with focus on nothing, presses the primary button
//!   (Rune's `default_button`). A disabled primary drops its handler, so Enter does nothing.
//!   [`StrataDialog::default_button`]`(false)` keeps the primary from being the default
//!   button, for one that must be pressed on purpose (C26's host-key and busy-volume
//!   prompts): Enter then presses only a focused button.
//! - **Enter** in a multi-line field types a newline only in a field built with
//!   [`text_area`] (or given [`newline_on_enter`]). Rune's multi-line editor treats a plain
//!   Return in a paragraph as submit (a chat composer's rule), and a field with nothing to
//!   submit presses the default button, so a plain `TextArea` in a dialog submits it.
//! - **Left and Right** on a footer button move along the footer and stop at its ends.
//!
//! # Focus
//!
//! - Rune's modal traps Tab, makes the background inert, and on close returns focus to the
//!   element focused before the dialog opened, nested dialogs in stack order. A dialog that
//!   opens as another closes keeps the first one's opener. Focus that something else took
//!   while the dialog closed stays where it is.
//! - Initial focus is the first Tab stop in the dialog: the first body control, so the first
//!   field. [`Focus::Primary`], [`Focus::Secondary`] or [`Focus::Action`] start on a footer
//!   button instead, and [`Focus::Close`] on the close button: the dialog marks it
//!   `El::autofocus`, which Rune's modal focuses as it opens. Tab's cycle stays document
//!   order's, only starting there. A form starts on another field by setting its
//!   `TextEdit::focused` when it opens.
//! - The close button comes last in document order and is pinned to the header's corner, so
//!   that the first field, not the close button, is the first Tab stop. The trade-off: a
//!   screen reader reaches the close button last.
//! - [`StrataDialog::return_focus`] names where focus goes on close instead of the opener:
//!   Strata uses it for dialogs that change the listing.
//!
//! # Motion
//!
//! Opening scales the panel from 96 % and fades it and the backdrop in over 200 ms; closing
//! reverses it. [`DialogShell::fade_only`] drops the scale (Settings). With `look.animations`
//! off, or [`DialogShell::reduced_motion`] on, a dialog shows and goes at once. Pass
//! `cx.is_reduced_motion()` to `reduced_motion` for the system's Reduce Motion; P5's setting
//! reaches the dialog as `look.animations`.
//!
//! # Known gaps
//!
//! - A chained dialog opened by a keyboard close shows its default button's focus ring
//!   (`app/dialogs`: "Closing a dialog by keyboard as a dialog chained on it opens…", the
//!   no-ring half, unverified in Strata). Rune has no way to clear keyboard focus mode.
//! - Rune doesn't clamp a long frame's step in presence motion to 33 ms (`app/dialogs`: "A
//!   frame longer than 33 ms…", unverified in Strata). Since Rune `aea4606`, motion that a late
//!   frame starts holds its first pose for the overrun instead of jumping, but motion already
//!   running still takes the whole step.
//! - Up and Down don't move focus between body controls by position (`app/dialogs`: "With
//!   focus outside a text field, an unmodified arrow key…", unverified). Left and Right move
//!   along the footer, and Down opens a focused select, which is Rune's own.
//! - A dialog whose action disables its opener (Settings → Keyboard → Reset all → Reset)
//!   returns focus to that disabled opener: Rune's modal restore drops only an opener that has
//!   gone (`sync_modals`, `runtime/input/focus.rs:443` at the pinned rev). Strata can name the
//!   target with [`StrataDialog::return_focus`].
//! - A click on the backdrop during the close fade reaches whatever is under it: the exiting
//!   dialog is inert in Rune, as Strata's insensitive GTK layer is.
//!
//! # Coverage of `app/dialogs`
//!
//! What checks each Behavior bullet. Scripts are `tests/scripts/c15/<name>.json`; "test" is
//! a test in `crates/strata-gallery/tests/c15.rs`, which checks what the accessibility tree
//! doesn't show (opacity, scale, the backdrop, focus rings); "unit" is in `dialog/tests.rs`;
//! "golden" is a scene's PNG or accessibility golden.
//!
//! | Bullet (opening words) | Checked by |
//! | --- | --- |
//! | While a dialog is open, the window content behind it is blurred… | test `the_backdrop_stays_until_the_last_stacked_dialog_closes` ([`BACKDROP_KEY`]) |
//! | Opening an action or search dialog fades in the backdrop… | test `a_dialog_scales_and_fades_in_and_out_in_about_200_ms` |
//! | Opening Settings fades its whole layer… | test `a_fade_only_panel_fades_without_scaling_on_its_first_open` (the shell's `fade_only`; the Settings layer itself is C20c's and Strata's) |
//! | The first Settings open of a process fades in… | the same test: the first open after mount fades |
//! | With Reduce motion on… | test `with_animations_off_a_dialog_shows_and_goes_at_once` |
//! | A frame longer than 33 ms… | **known gap** (Rune; issue draft) |
//! | Closing Settings during its fade-in… | test `closing_mid_open_fades_out_from_where_it_was` |
//! | Clicking the backdrop outside a dialog dismisses it… | script `backdrop` |
//! | A dialog in a non-cancellable in-progress state… | script `backdrop-blocking` |
//! | The mount authentication dialog… / The Compress dialog… / The archive password dialog… ignores backdrop clicks | script `backdrop-dirty` (the `blocking(dirty)` mechanism); which dialogs count as dirty is **Strata policy** |
//! | Clicking the backdrop repeatedly during the close animation… | script `backdrop-repeat` |
//! | Confirmation dialogs open with the confirm button focused… | scripts `keyboard-confirm`, `confirm-enter`, `confirm-escape`; golden `c15-danger` (a11y) |
//! | A keyboard-focused confirm button… shows a 3 px… halo | test `a_keyboard_focused_confirm_button_shows_its_ring_until_focus_moves`; golden `c15-danger-keyboard` (PNG) |
//! | That halo sits flush… danger color… | golden `c15-danger-keyboard` (PNG) |
//! | Moving keyboard focus off the confirm button… | test `a_keyboard_focused_confirm_button_shows_its_ring_until_focus_moves` |
//! | With focus outside a text field, Enter activates the focused button… | scripts `enter-focused-button`, `enter-checkbox` |
//! | With focus outside a text field, an unmodified arrow key… | **partial**: scripts `arrows` (footer), `arrows-select` (Down opens a select); Up and Down between body controls is a known gap |
//! | An error dialog that offers Open as Administrator… | **Strata policy** (`initial_focus(Focus::Action(..))`, a secondary labelled Close) |
//! | While a dialog is open, focus that moves outside it… | script `tab-cycle` |
//! | Closing the last open dialog by any route… | scripts `restore-escape`, `chained-restore`; test `every_route_that_closes_a_dialog_returns_focus_to_its_opener` |
//! | A dialog registered as changing the listing… | script `return-focus` (`return_focus`) |
//! | When that widget is gone… cursor row… | **Strata policy**: Rune leaves nothing focused, and Strata's window falls back to the cursor row |
//! | A widget that takes focus while a dialog closes keeps it | script `focus-taken` |
//! | Closing a dialog over a modal that stays open… | script `nested` |
//! | When the recorded opener is the remaining modal's own layer… | **not applicable**: a dialog's panel and layer never take focus, so neither is ever a recorded opener; its fallback is the next bullet's |
//! | When the dialog's action disabled its opener… | **Rune gap** (above; issue draft), or **Strata policy** with `return_focus` |
//! | When that opener is gone… first focusable control in the remaining modal | script `nested-opener-gone` |
//! | That fallback skips boxes, scrolled windows, and viewports… | script `nested-opener-gone` |
//! | When a custom action's delete fails in Settings… | **Strata policy** |
//! | A dialog closed from the keyboard keeps the focus ring… | test `a_keyboard_close_keeps_the_ring_on_the_opener` |
//! | A dialog closed from the keyboard over a modal that stays open… | test `a_keyboard_close_over_a_dialog_keeps_the_ring_on_its_opener_there` |
//! | Closing a dialog by keyboard as a dialog chained on it opens… | **partial**: script `chained` (focus on the chained default button); its "no focus ring" is a known gap |
//! | Enter in a single-line text or password field… | script `form-submit` |
//! | Enter in a form field does nothing while the primary button is disabled | script `form-disabled` |
//! | Enter in a multi-line text field inserts a newline… | script `form-multiline`; unit `return_in_a_dialog_text_area_types_a_newline_through_the_field` |
//! | A form text field can be limited in bytes or UTF-16 units… | script `form-limit-cut`; unit tests |
//! | That field's "N chars remaining" caption… | script `form-limit` |
//! | Changing a field's limit or unit trims… | script `form-limit-unit`; unit `an_insertion_keeps_its_longest_leading_part_that_fits` |
//! | An install command's copy button… | **Strata policy** (the updates work's dialog content) |
//! | A dialog's shadow fades past its rounded edges… | script `backdrop` (a click 5 pt outside the panel's edge, in its shadow); the shadow's look in the PNG goldens |
//! | A dialog larger than the window scrolls inside the window… | script `small-window` |
//! | When the window is narrower than a dialog's minimum width plus 84 px… | golden `c15-small-window` (PNG): layout, which tests don't assert |
//! | Message dialog text keeps Korean words… | unit `short_korean_words_are_kept_whole_and_long_runs_stay_breakable` only: CI renders without a Hangul font, so no golden draws Korean |
//!
//! # Follow-ups
//!
//! - The constants below could become a `dialog` slot (`dialog.backdrop.blur`,
//!   `dialog.panel.radius`, …) in strata-kit, in a W2 addition of its own.

mod action;
mod keys;
mod limit;
mod shell;
mod text;

#[cfg(test)]
mod tests;

pub use action::{Focus, RESERVED, StrataDialog, Tone};
pub use keys::{newline_on_enter, text_area};
pub use limit::{Measure, fit, limit_on_change, remaining, trim_insertion, trim_to_fit};
pub use shell::DialogShell;
pub use text::keep_words_whole;

/// The backdrop's key, shared by every dialog: the bottom dialog of a stack draws it, so a
/// dialog that opens as another closes keeps the same backdrop, and the blur doesn't dip.
pub const BACKDROP_KEY: &str = "strata-dialog-backdrop";

/// The backdrop's blur, a Gaussian sigma in points. Strata blurs the window content with GTK's
/// `push_blur(3.5)` (`src/ui/blur.rs`), whose radius is taken here as twice the sigma; check
/// against a Strata screenshot (open item).
pub const BACKDROP_BLUR: f32 = 1.75;

/// The scale an action dialog opens from and closes to (Strata's `.modal-hidden` transform,
/// `src/style.css`).
pub const ENTRANCE_SCALE: f32 = 0.96;

/// How long opening and closing take, in seconds (Strata's 200 ms transitions,
/// `src/style.css` and `src/ui/modal.rs`).
pub const MOTION_SECONDS: f32 = 0.2;

/// An action dialog's default minimum width, in points (`.action-dialog`, `src/style.css`).
/// Strata's other widths: compact 440, wide 520, message dialogs 560.
pub const WIDTH: f32 = 480.0;

/// The width of a message dialog, whose text wraps (`message_dialog_layout`,
/// `src/ui/controls.rs`).
pub const MESSAGE_WIDTH: f32 = 560.0;

/// The panel's corner radius before roundness (`.action-dialog`'s 9 px, `src/style.css`).
pub const PANEL_RADIUS: f32 = 9.0;

/// The header icon box's side and corner radius before roundness (`.action-dialog-symbol`).
pub const ICON_BOX: f32 = 40.0;
pub const ICON_BOX_RADIUS: f32 = 6.0;

/// A keyboard-focused footer button's ring: 3 pt wide, flush against its border, in the
/// accent (or danger) colour at 28 % (`app/dialogs`, lgse/strata#2049).
pub const FOCUS_RING_WIDTH: f32 = 3.0;
pub const FOCUS_RING_ALPHA: f32 = 0.28;

/// The longest Hangul word, in characters, that [`keep_words_whole`] keeps on one line
/// (`app/dialogs`: "Message dialog text keeps Korean words of up to 12 characters whole").
pub const KOREAN_WORD_MAX: usize = 12;
