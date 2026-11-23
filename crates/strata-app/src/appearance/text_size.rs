// SPDX-License-Identifier: MIT

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

pub(crate) fn step(app: &mut App, steps: i32) {
    set(app, |size| size.stepped(steps));
}

fn set(app: &mut App, to: impl FnOnce(TextSize) -> TextSize) -> Outcome {
    crate::tenxer::cancel_chord(app);
    let size = to(shell::preferences(app).text_size);
    shell::saved(app).change(|settings| settings.text_size = size);
    shell::follow_look(app);
    Outcome::Ran
}
