// SPDX-License-Identifier: MIT

//! The interface's text size from the keyboard, as GTK's window changes it (`zoom_shortcut` in
//! `src/ui/window/keyboard.rs`, `TextSizeShortcut::for_shortcut` in
//! `src/ui/preferences/text_size.rs`):
//!
//! - **The keys** are read at the dispatcher's `zoom_shortcut` stage, ahead of every owner of
//!   input but a modal layer or a popover, so they work while a rename is open: Ctrl+plus and
//!   Ctrl+= one pixel larger, Ctrl+minus one smaller, Ctrl+0 back to the default. Each is the
//!   active map's binding with its own modifiers, as `press.is` reads it, so Ctrl+Alt+= is
//!   the Icons view's tile size and not this.
//! - **The size** steps by the shared rule (`strata_desktop::settings::TextSize`): 8 to 48 px,
//!   13 by default. It is saved at once, and the look follows it (`shell::follow_look`).
//!
//! Left out: GTK's notice "Text size: N px", which this interface has no surface for yet.

use strata_desktop::{
    keys::{
        Modifiers,
        bindings::{TEXT_LARGER, TEXT_RESET, TEXT_SMALLER},
    },
    settings::TextSize,
};

use crate::{
    App,
    commands::{Action, Keys, Outcome, Stage},
    shell,
};

// `text-larger`, `text-smaller` and `text-reset`, among the window's own commands
// (`commands::own`), for the popover's stepper to run too.

pub(crate) const LARGER: Action<App> =
    Action::new("text-larger", |app, _, _| set(app, |size| size.stepped(1)))
        .command(TEXT_LARGER)
        .keys(&[Keys::at(
            Stage::ZoomShortcut,
            &[TEXT_LARGER.ignoring(Modifiers::empty())],
        )]);

pub(crate) const SMALLER: Action<App> = Action::new("text-smaller", |app, _, _| {
    set(app, |size| size.stepped(-1))
})
.command(TEXT_SMALLER)
.keys(&[Keys::at(
    Stage::ZoomShortcut,
    &[TEXT_SMALLER.ignoring(Modifiers::empty())],
)]);

pub(crate) const RESET: Action<App> =
    Action::new("text-reset", |app, _, _| set(app, |_| TextSize::default()))
        .command(TEXT_RESET)
        .keys(&[Keys::at(
            Stage::ZoomShortcut,
            &[TEXT_RESET.ignoring(Modifiers::empty())],
        )]);

/// Saves the size `to` makes of the saved one, and has the look follow it. The key is taken
/// at either end of the range too, as GTK's stage takes it.
fn set(app: &mut App, to: fn(TextSize) -> TextSize) -> Outcome {
    let size = to(shell::preferences(app).text_size);
    shell::saved(app).change(|settings| settings.text_size = size);
    shell::follow_look(app);
    Outcome::Ran
}
