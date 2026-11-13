// SPDX-License-Identifier: MIT

//! The Icons pane header's "Thumbnail size" button and its popover (A5), GTK's
//! `ThumbnailSizeMenu` and `SizeActions` in `src/ui/browser_modes/icons_controls.rs`.
//!
//! - **The button** sits in the header's actions row of the Icons view only, named and
//!   described "Thumbnail size", and opens the popover under it; Return presses it as Space
//!   does ([`buttons::on_return`]).
//! - **The popover:** a heading with the size in px, C20a's slider from 32 to 256 in steps of
//!   16, named "Thumbnail size", with "Small" and "Large" at its ends; each move sets the size
//!   as A1's commands do, kept by the shared policy (`icons::size::set`). A wheel notch over
//!   the slider moves it 1 px.
//! - **"Make this the default size" and "Reset to default size"** show only while the shown
//!   folder is remembered at a size other than the default, and each closes the popover
//!   (`closing_menu_action`).
//! - While it is open, the slider follows a size the window's keys change.
//! - Escape or a press outside closes it, and the keyboard goes back to the button. While it is
//!   open the window's commands wait, as under GTK's popover ([`Holds::Menu`] in the part).

use rune::{El, Key, KeyEvent, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, Placement, popover};
use strata_components::{
    controls::slider::{Range, Slider, SliderChange, SliderState},
    header,
};
use strata_core::i18n::tr;
use strata_kit::icons::IconName;

use crate::{App, buttons, icons::size, slots::Frame};

/// The button's key, which the popover hangs from and gives the keyboard back to.
pub(crate) const BUTTON: &str = "icons-thumbnail-size";
const SLIDER: &str = "icons-thumbnail-slider";
const MAKE_DEFAULT: &str = "icons-thumbnail-make-default";
const RESET: &str = "icons-thumbnail-reset";

/// GTK's `MIN_ICONS_THUMBNAIL_SIZE`, `MAX_ICONS_THUMBNAIL_SIZE` and the scale's step.
const RANGE: (f64, f64, f64) = (32.0, 256.0, 16.0);

/// The popover's state: whether it is open, and its slider.
pub(crate) struct ThumbnailSize {
    open: bool,
    slider: SliderState,
    labels: Labels,
}

struct Labels {
    title: String,
    small: String,
    large: String,
    make_default: String,
    reset: String,
}

impl ThumbnailSize {
    pub(crate) fn new() -> Self {
        let (min, max, step) = RANGE;
        Self {
            open: false,
            slider: SliderState::new(64.0, Range::new(min, max, step)),
            labels: Labels {
                title: tr("Thumbnail size"),
                small: tr("Small"),
                large: tr("Large"),
                make_default: tr("Make this the default size"),
                reset: tr("Reset to default size"),
            },
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }
}

/// The header button.
pub(crate) fn button(app: &App) -> El {
    let look = &app.look;
    let state = &app.pane_header.thumbnails;
    let built = header::action(look, IconName::Pictures, &state.labels.title)
        .on_press_tx(|app: &mut App, _: &mut Tx| toggle(app))
        .build(&look.kit)
        .expanded(state.open);
    buttons::on_return(BUTTON, built)
}

fn toggle(app: &mut App) {
    if app.pane_header.thumbnails.open {
        close(app);
        return;
    }
    let shown = f64::from(size::shown(app));
    let state = &mut app.pane_header.thumbnails;
    state.slider.set_value(shown);
    state.open = true;
}

/// While the popover is open its slider follows a size the window's keys changed, as GTK's
/// scale does (`icons_thumbnail_scale` in `src/ui/browser/view_modes.rs`), unless a drag holds
/// it. True when it moved.
pub(crate) fn follow(app: &mut App) -> bool {
    let shown = f64::from(size::shown(app));
    let state = &mut app.pane_header.thumbnails;
    if !state.open || state.slider.dragging() || state.slider.value() == shown {
        return false;
    }
    state.slider.set_value(shown);
    true
}

fn close(app: &mut App) {
    app.pane_header.thumbnails.open = false;
}

/// The popover's surface, in the layer while it is open.
pub(crate) fn build(app: &App, frame: &mut Frame<'_, App>) {
    let state = &app.pane_header.thumbnails;
    if !state.open {
        return;
    }
    let look = &app.look;
    let kit = &look.kit;
    let labels = &state.labels;
    let value = format!("{} px", state.slider.value().round());
    let slider = Slider::new(
        SLIDER,
        |app: &App| &app.pane_header.thumbnails.slider,
        |app: &mut App| &mut app.pane_header.thumbnails.slider,
    )
    .label(labels.title.clone())
    .value_text(value.clone())
    .wheel(Some(1.0))
    .on_change(slid)
    .build(app, look);
    let mut body = col()
        .key("icons-thumbnail-popover")
        .role("dialog")
        .label(labels.title.clone())
        .w(220.0 + 2.0 * look.space.lg)
        .padding(look.space.lg)
        .gap(look.space.md)
        .child(heading(app, value))
        .child(slider)
        .child(extremes(app));
    if size::differs_from_default(app) {
        body = body.child(actions(app, labels));
    }
    // Rune's modal keeps Tab inside, and the keyboard goes back to the button on close; Escape
    // is the host's, as rune-kit's popover has none.
    let body = body
        .focus_return(BUTTON)
        .on_key(|app: &mut App, event: &KeyEvent, _: &mut Tx| {
            if event.key != Key::Escape {
                return false;
            }
            close(app);
            true
        })
        .modal_tx(true, |app: &mut App, _: &mut Tx| close(app));
    let surface = popover::raised(kit, body, kit.radius.md);
    frame.layer.push(popover::anchored(
        kit,
        surface,
        BUTTON,
        Placement::BottomEnd,
        look.space.xs,
    ));
}

/// "Thumbnail size" with the size in px beside it.
fn heading(app: &App, value: String) -> El {
    let look = &app.look;
    let kit = &look.kit;
    let title = &app.pane_header.thumbnails.labels.title;
    row()
        .w_full()
        .gap(look.space.lg)
        .items_center()
        .child(
            text(title.clone())
                .font(kit.ty.ui_medium)
                .color(kit.c.text_base)
                .grow(1.0),
        )
        .child(text(value).font(kit.ty.ui).color(kit.c.text_muted))
}

/// "Small" and "Large" under the slider's ends.
fn extremes(app: &App) -> El {
    let kit = &app.look.kit;
    let labels = &app.pane_header.thumbnails.labels;
    let end = |label: &str| {
        text(label.to_owned())
            .font(kit.ty.small)
            .color(kit.c.text_muted)
    };
    row()
        .w_full()
        .child(end(&labels.small).grow(1.0))
        .child(end(&labels.large))
}

/// "Make this the default size" and "Reset to default size", each closing the popover.
fn actions(app: &App, labels: &Labels) -> El {
    let kit = &app.look.kit;
    let action = |key: &'static str, label: &str, act: fn(&mut App, &mut Tx)| {
        let button = Button::new(label.to_owned())
            .variant(ButtonVariant::Ghost)
            .on_press_tx(move |app: &mut App, tx: &mut Tx| {
                close(app);
                act(app, tx);
            })
            .build(kit)
            .w_full();
        buttons::on_return(key, button)
    };
    col()
        .w_full()
        .gap(app.look.space.xxs)
        .child(action(MAKE_DEFAULT, &labels.make_default, make_default))
        .child(action(RESET, &labels.reset, size::reset_folder))
}

fn make_default(app: &mut App, tx: &mut Tx) {
    let chosen = app.pane_header.thumbnails.slider.value().round() as i32;
    size::make_default(app, chosen, tx);
}

/// A move of the slider: the size follows it, as GTK's `value-changed` does.
fn slid(app: &mut App, value: f64, _: SliderChange, tx: &mut Tx) {
    size::set(app, value.round() as i32, tx);
}

#[cfg(test)]
mod tests;
