// SPDX-License-Identifier: MIT

use rune::{El, Tx, row};
use strata_components::controls::{
    number_field::{NumberField, NumberFieldState},
    slider::{Range, Slider, SliderChange, SliderState, Snap},
};
use strata_core::i18n::tr;

use crate::App;

const SLIDER_WIDTH: f32 = 180.0;
const FIELD_SLOT_WIDTH: f32 = 72.0;

/// Paired state for a slider and its direct numeric text input.
pub(super) struct SliderFieldState {
    slider: SliderState,
    field: NumberFieldState,
}

impl SliderFieldState {
    pub(super) fn new(value: f64, range: Range, suffix: impl Into<String>) -> Self {
        Self {
            slider: SliderState::new(value, range),
            field: NumberFieldState::new(value, range).suffix(suffix),
        }
    }

    pub(super) fn slider(&self) -> &SliderState {
        &self.slider
    }

    pub(super) fn slider_mut(&mut self) -> &mut SliderState {
        &mut self.slider
    }

    pub(super) fn field(&self) -> &NumberFieldState {
        &self.field
    }

    pub(super) fn field_mut(&mut self) -> &mut NumberFieldState {
        &mut self.field
    }

    pub(super) fn hold(&mut self, held: bool) {
        self.field.hold(held);
    }

    pub(super) fn focused(&self) -> bool {
        self.field.edit().focused
    }

    /// Keeps both controls aligned with saved preferences when not actively edited.
    pub(super) fn sync(&mut self, value: f64) {
        if !self.slider.dragging() {
            self.slider.set_value(value);
        }
        if !self.field.edit().focused && !self.field.held() {
            self.field.set_value(value);
        }
    }

    pub(super) fn set_from_slider(&mut self, value: f64) {
        self.field.set_value(value);
    }

    pub(super) fn set_from_field(&mut self, value: f64) {
        self.slider.set_value(value);
    }
}

#[derive(Clone, Copy)]
pub(super) struct SliderFieldAccess {
    pub(super) state_mut: fn(&mut App) -> &mut SliderFieldState,
    pub(super) slider: fn(&App) -> &SliderState,
    pub(super) slider_mut: fn(&mut App) -> &mut SliderState,
    pub(super) field: fn(&App) -> &NumberFieldState,
    pub(super) field_mut: fn(&mut App) -> &mut NumberFieldState,
}

macro_rules! slider_field_access {
    ($($field:ident).+) => {
        $crate::settings_dialog::slider_field::SliderFieldAccess {
            state_mut: |app| &mut app.$($field).+,
            slider: |app| app.$($field).+.slider(),
            slider_mut: |app| app.$($field).+.slider_mut(),
            field: |app| app.$($field).+.field(),
            field_mut: |app| app.$($field).+.field_mut(),
        }
    };
}
pub(super) use slider_field_access;

pub(super) struct SliderField {
    key: &'static str,
    label_key: &'static str,
    access: SliderFieldAccess,
    on_change: fn(&mut App, f64, &mut Tx),
    snaps: Vec<Snap>,
    disabled: bool,
}

impl SliderField {
    pub(super) fn new(
        key: &'static str,
        label_key: &'static str,
        access: SliderFieldAccess,
        on_change: fn(&mut App, f64, &mut Tx),
    ) -> Self {
        Self {
            key,
            label_key,
            access,
            on_change,
            snaps: Vec::new(),
            disabled: false,
        }
    }

    pub(super) fn snaps(mut self, snaps: Vec<Snap>) -> Self {
        self.snaps = snaps;
        self
    }

    pub(super) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub(super) fn poll(&self, app: &mut App, tx: &mut Tx) -> bool {
        self.number_field().poll(app, tx)
    }

    pub(super) fn build(self, app: &App) -> El {
        let look = &app.look;
        let access = self.access;
        let on_change = self.on_change;
        let value_text = (access.field)(app).value_text();
        let field = self.number_field().build(app, look);
        let slider = Slider::new(self.key, access.slider, access.slider_mut)
            .snaps(self.snaps)
            .label(tr(self.label_key))
            .value_text(value_text)
            .disabled(self.disabled)
            .on_change(move |app, value, _: SliderChange, tx| {
                (access.state_mut)(app).set_from_slider(value);
                on_change(app, value, tx);
            })
            .build(app, look);
        row()
            .gap(look.space.md)
            .items_center()
            .child(row().w(SLIDER_WIDTH).items_center().child(slider))
            .child(
                row()
                    .min_w(FIELD_SLOT_WIDTH)
                    .justify_end()
                    .items_center()
                    .child(field),
            )
    }

    fn number_field(&self) -> NumberField<App> {
        let access = self.access;
        let on_change = self.on_change;
        NumberField::new(
            format!("{}-field", self.key),
            access.field,
            access.field_mut,
        )
        .label(tr(self.label_key))
        .steppers(false)
        .disabled(self.disabled)
        .on_change(move |app, value, tx| {
            (access.state_mut)(app).set_from_field(value);
            on_change(app, value, tx);
        })
    }
}
