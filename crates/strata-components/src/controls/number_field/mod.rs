//! NumberField (C20a): a spin button, a text field between − and + with a minimum, a maximum,
//! a step and an optional unit (plan §4 C20: thumbnail workers, text size, the corner
//! roundness percent).
//!
//! Work package: C20a, `docs/work/phase-1-leaves/C20a-slider-number-field.md`.
//!
//! The host holds a [`NumberFieldState`] (the value, its [`Range`], its unit and the text
//! being edited) and builds a [`NumberField`] over it each frame, calling
//! [`NumberField::poll`] from its poll. It reports committed values only; saving is the
//! host's.
//!
//! **Rune 2a44156: partly.** A core `text_input` (`on_input_key`, `on_input_change`,
//! `on_secondary_press`, `align`, `invalid`), a `textbox` frame that keeps the field's focus
//! through a press on a stepper inside it (probe P2b), `on_step` on a spin button that isn't
//! focusable (probe P3), `on_wheel` and `range_value` cover it. Missing: a text field can't
//! take the spin button role (U-C20a-1), there's no focus-change hook (anomalyco/rune#106,
//! U-C6-3), and rune-kit's TextInput takes no menu press (U-C21-2).
//!
//! # What it does
//!
//! - **Typing.** The text commits on Return, on a step, and when the field loses the
//!   keyboard. A number in the range (its unit may be typed after it) moves the value there,
//!   rounded to the range's decimals; anything else reverts to the value, as GTK's spin
//!   button does under Strata's `UpdatePolicy::IfValid`. While the text isn't a value the
//!   field is marked invalid.
//! - **Steps.** − and + (insensitive at their ends), Up and Down a step, Page Up and Page Down
//!   a page (ten steps unless [`Range::page`] says), Ctrl+Home and Ctrl+Page Down the minimum,
//!   Ctrl+End and Ctrl+Page Up the maximum (GTK's bindings), and the wheel over the field a
//!   step a notch (precise deltas: 40 pt). Each commits the typed text first. Plain Home and
//!   End move the caret. From a value between grid points (one the host set off the grid) a
//!   step lands on the next grid point that way, as the [`Slider`](crate::controls::slider)
//!   does; GTK's spin button without snap-to-ticks would add a whole step instead. The wheel
//!   reads the vertical delta only, as GTK's does: a sideways one (Shift+wheel on Linux, a
//!   tilt wheel) and one with Ctrl, Alt, Cmd or Super held pass on to the window.
//! - **The field.** Rune's core field, not rune-kit's `TextInput`, which builds its field
//!   itself and takes no menu press (U-C21-2): through [`NumberField::on_menu`] the host opens
//!   C21's `FieldMenu`. The frame is drawn as rune-kit's TextInput draws its box, from the
//!   same Kit tokens: `bg_base`, `radius.md`, the input elevation, the hover overlay, the
//!   `border_focus` outline, a `danger.fg` outline while invalid, half opacity when
//!   disabled. Its height is `frame.control + space.xs` (28 at 13 px), the steppers
//!   `frame.control` squares with `icon_size.md` Lucide − (`IconName::Minus`) and +, the
//!   number centred in `kit.ty.ui_compact`, as wide as the wider end, the unit after it in
//!   `text_muted`.
//! - **Disabled:** dimmed, the field read-only and out of the focus order, no steps.
//!
//! Assistive technology hears a spin button named by [`NumberField::label`], with its value,
//! minimum, maximum, step and a value text with its unit ("60%"); it steps it. Inside it is
//! the text field (same name) that has the keyboard. The steppers are hidden from it, and
//! never focused, as GTK's are.
//!
//! # Bullets
//!
//! Scripts are in `tests/scripts/c20a/`, in-process tests in `crates/strata-gallery/tests/c20a.rs`,
//! unit tests in this module's `tests.rs`. Saving, applying the value and other windows
//! following are Strata's.
//!
//! | Bullet | Covered by |
//! | --- | --- |
//! | `browser/thumbnails/workers`: Settings → General → Performance has a Thumbnail workers spin button from 1 to 16, with − before the number and + after it | scripts `number-increase`, `number-decrease`; goldens |
//! | `browser/thumbnails/workers`: Assistive technologies read the control as one spin button named "Thumbnail workers" | script `number-names`; a11y goldens. **Partial**: a spin button holding a text field (D1) |
//! | `browser/thumbnails/workers`: Clicking − or +, or typing a count and pressing Return, saves `thumbnail_workers`… | scripts `number-increase`, `number-decrease`, `number-type-return` (the commit; saving is Strata's) |
//! | `browser/thumbnails/workers`: At 16, clicking + leaves the count at 16 | script `number-increase-at-max`; unit `steps_hold_at_the_ends` |
//! | `browser/thumbnails/workers`: + is insensitive at 16, and − at 1 | PNG goldens `c20a-max-*`, `c20a-min-*` |
//! | `browser/thumbnails/workers`: Clicking the number edits it; there is no reset to the default | scripts `number-type-return`, `number-type-blur`, `number-type-invalid` |
//! | `browser/view-modes/text-size`: Settings → Appearance → Text has a "Text size" spin button, accessible name "Text size in pixels", accepting 8 to 48… | scripts `number-type-out-of-range`, `number-names` |
//! | `settings/themes/corner-roundness`: …a Corner roundness stepper from 0 to 200 in steps of 10 | scripts `number-arrow-up`, `-arrow-down`, `-page-up`, `-ctrl-end`, `number-wheel` |
//! | `settings/themes/corner-roundness`: …its stepper's accessible name is "Corner roundness in percent" | script `number-names` (the name; the group's place is Strata's) |
//! | `settings/themes/icons`: Under any icon theme, the Text size, Corner roundness, and Thumbnail workers spin buttons draw Lucide − and + steps | PNG goldens |
//!
//! Not from a bullet: the field menu holding a typed value until it closes (and not when the
//! host opens none: unit `a_menu_press_that_opens_no_menu_keeps_the_blur_commit`), a stepper
//! press keeping the keyboard in the field, assistive increments, and a sideways or modified
//! wheel passing the field by (`c20a::` tests).
//!
//! # Deviations
//!
//! - **D1** (OWNER ruling): the spin button is a wrapper around the text field, until
//!   U-C20a-1; GTK's is one control.
//! - **D2** (OWNER ruling): − and + don't repeat while held (GTK's do).
//! - **D3**: letters aren't refused as they're typed (GTK's numeric mode refuses them); text
//!   that isn't a value is marked invalid and reverts when it commits.
//! - **D4**: the host calls [`NumberField::poll`] each frame to commit on a blur (#106).
//!
//! # Rune drafts (not filed)
//!
//! - **U-C20a-1, a text field can't be a spin button.** `a11y/roles.rs` maps any element with
//!   an input to a text input before reading its role, so `text_input(..).role("spinbutton")
//!   .range_value(4.0, 1.0, 16.0)` is a text input in the tree. And `on_step` on a field
//!   would take Left and Right from its caret: Rune's step keys (`runtime/input/focus.rs`)
//!   run before the field's editor. Ask: a field whose role is `spinbutton` maps to
//!   `Role::SpinButton` and keeps its text value beside its range; `on_step` on a field
//!   steps on Up, Down, Page Up and Page Down only. Here: a spin button wrapper with
//!   `on_step` and `focusable(false)` around the field.
//! - **rune-kit NumberField (proposal draft):** a field with min, max, step, page, decimals,
//!   an optional unit, − and + inside the box, GTK's keys, the wheel, revert on an invalid
//!   commit, and the spin button role (with U-C20a-1), taking `on_secondary_press` for an
//!   app's menu (with U-C21-2).
//! - Reused: anomalyco/rune#106 and U-C6-3 (no focus-change hook), U-C21-1 and U-C21-2 (field
//!   menus).

mod state;
mod view;

pub use crate::controls::slider::Range;
pub use state::NumberFieldState;
pub use view::NumberField;

#[cfg(test)]
mod tests;
