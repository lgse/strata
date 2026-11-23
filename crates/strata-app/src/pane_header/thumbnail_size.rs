// SPDX-License-Identifier: MIT

use rune::{El, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant};
use strata_components::{
    controls::slider::{Range, Slider, SliderChange, SliderState},
    header,
};
use strata_core::{i18n::tr, model::Location};
use strata_kit::icons::IconName;

use crate::{
    App, buttons,
    icons::size,
    popover::{Kind, Plain},
    shell,
    slots::Frame,
};

/// The button's key, which the popover hangs from and gives the keyboard back to.
pub(crate) const BUTTON: &str = "icons-thumbnail-size";
const SLIDER: &str = "icons-thumbnail-slider";
const MAKE_DEFAULT: &str = "icons-thumbnail-make-default";
const RESET: &str = "icons-thumbnail-reset";

const RANGE: (f64, f64, f64) = (32.0, 256.0, 16.0);

/// The popover's state: whether it is open, and its slider.
pub(crate) struct ThumbnailSize {
    open: bool,
    over: Option<Location>,
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
            over: None,
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
        .on_press_tx(toggle)
        .build(&look.accent_icon_kit())
        .expanded(state.open);
    buttons::on_return(BUTTON, built)
}

fn toggle(app: &mut App, tx: &mut Tx) {
    if app.pane_header.thumbnails.open {
        close(app);
        return;
    }
    let shown = f64::from(size::shown(app));
    let over = size::shown_location(app).cloned();
    let state = &mut app.pane_header.thumbnails;
    state.slider.set_value(shown);
    state.over = over;
    state.open = true;
    tx.focus(SLIDER, true);
}

pub(crate) fn follow(app: &mut App, tx: &Tx) -> bool {
    if !app.pane_header.thumbnails.open {
        return false;
    }
    if crate::popover::focus_left(tx, BUTTON, &stops(app), &[]) {
        close(app);
        return true;
    }
    let icons = shell::view_focus(app) == Some(crate::icons::VIEW);
    if !icons || size::shown_location(app) != app.pane_header.thumbnails.over.as_ref() {
        close(app);
        return true;
    }
    let shown = f64::from(size::shown(app));
    let slider = &mut app.pane_header.thumbnails.slider;
    if slider.dragging() || slider.value() == shown {
        return false;
    }
    slider.set_value(shown);
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
    .free_drag(true)
    .on_change(slid)
    .build(app, look);
    let differs = size::differs_from_default(app);
    let mut body = col()
        .w(220.0 + 2.0 * look.space.lg)
        .padding(look.space.lg)
        .gap(look.space.md)
        .child(heading(app, value))
        .child(slider)
        .child(extremes(app));
    if differs {
        body = body.child(actions(app, labels));
    }
    let plain = Plain {
        key: "icons-thumbnail-popover",
        anchor: BUTTON.into(),
        kind: Kind::Dialog,
        label: labels.title.clone(),
        stops: stops(app),
        own_arrows: &[SLIDER],
        listing: shell::view_focus(app),
        viewports: shell::listing_viewports(app),
        close,
        parent: None,
    };
    plain.build(look, body, frame);
}

/// The popover's controls, in order: the slider, and its two actions while they show.
fn stops(app: &App) -> Vec<&'static str> {
    if size::differs_from_default(app) {
        vec![SLIDER, MAKE_DEFAULT, RESET]
    } else {
        vec![SLIDER]
    }
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
            .w_full()
            .role("menuitem");
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

fn slid(app: &mut App, value: f64, _: SliderChange, tx: &mut Tx) {
    size::set(app, value.round() as i32, tx);
}

#[cfg(test)]
mod tests;
