// SPDX-License-Identifier: MIT

//! The Transparency section of Settings → Appearance (UI-54):
//! master transparency toggle, independent background and raised-surface transparency sliders,
//! frosted glass toggle, and unfocused-window multiplier slider.

use rune::{El, TextStyle, Tx, row, text};
use strata_components::controls::slider::{Range, Slider, SliderChange, SliderState, Snap};
use strata_core::i18n::tr;
use strata_desktop::settings::{
    MAX_SURFACE_TRANSPARENCY, MAX_UNFOCUSED_TRANSPARENCY_MULTIPLIER, MIN_SURFACE_TRANSPARENCY,
    MIN_UNFOCUSED_TRANSPARENCY_MULTIPLIER, TransparencyPreferences,
    UNFOCUSED_TRANSPARENCY_MULTIPLIER_STEP, normalized_surface_transparency,
    normalized_unfocused_transparency_multiplier,
};

use super::controls::{PageCtx, RowSpec, group_card, option_row, switch_row};
use crate::{App, shell};

const SLIDER_WIDTH: f32 = 180.0;
const VALUE_WIDTH: f32 = 48.0;

/// Slider states for the three numeric transparency controls in Settings → Appearance.
pub(super) struct TransparencySliders {
    background: SliderState,
    raised: SliderState,
    unfocused_multiplier: SliderState,
}

impl TransparencySliders {
    pub(super) fn new() -> Self {
        let defaults = TransparencyPreferences::default();
        let percent_range = Range::new(
            f64::from(MIN_SURFACE_TRANSPARENCY),
            f64::from(MAX_SURFACE_TRANSPARENCY),
            1.0,
        )
        .page(10.0);
        let multiplier_range = Range::new(
            MIN_UNFOCUSED_TRANSPARENCY_MULTIPLIER,
            MAX_UNFOCUSED_TRANSPARENCY_MULTIPLIER,
            UNFOCUSED_TRANSPARENCY_MULTIPLIER_STEP,
        )
        .page(0.25);
        Self {
            background: SliderState::new(
                f64::from(defaults.background_transparency),
                percent_range,
            ),
            raised: SliderState::new(
                f64::from(defaults.raised_surface_transparency),
                percent_range,
            ),
            unfocused_multiplier: SliderState::new(defaults.unfocused_multiplier, multiplier_range),
        }
    }

    /// Keeps slider positions aligned with saved settings when the user is not dragging them.
    pub(super) fn sync(&mut self, prefs: TransparencyPreferences) {
        if !self.background.dragging() {
            self.background
                .set_value(f64::from(prefs.background_transparency));
        }
        if !self.raised.dragging() {
            self.raised
                .set_value(f64::from(prefs.raised_surface_transparency));
        }
        if !self.unfocused_multiplier.dragging() {
            self.unfocused_multiplier
                .set_value(prefs.unfocused_multiplier);
        }
    }
}

struct SliderControl {
    key: &'static str,
    label_key: &'static str,
    value_text: String,
    disabled: bool,
    snaps: Vec<Snap>,
    get: fn(&App) -> &SliderState,
    get_mut: fn(&mut App) -> &mut SliderState,
    on_change: fn(&mut App, f64, SliderChange, &mut Tx),
}

pub(super) fn build(ctx: PageCtx<'_>) -> Option<El> {
    let app = ctx.app;
    let prefs = shell::transparency(app);
    let sub_disabled = !prefs.enabled;

    let rows = vec![
        switch_row(
            ctx,
            RowSpec::new(
                "transparency",
                "Transparency effects",
                "Make window surfaces translucent while keeping text, icons, and previews opaque.",
            ),
            prefs.enabled,
            |app, on, tx| {
                shell::saved(app).change_transparency(|t| t.enabled = on);
                shell::follow_look_tx(app, tx);
            },
        ),
        background_row(ctx, prefs, sub_disabled),
        raised_row(ctx, prefs, sub_disabled),
        switch_row(
            ctx,
            RowSpec::new(
                "frosted-glass",
                "Frosted glass",
                "Apply backdrop blur and a subtle frosted sheen to translucent dialogs, menus, and popovers.",
            )
            .indented()
            .disabled(sub_disabled),
            prefs.frosted_glass,
            |app, on, tx| {
                shell::saved(app).change_transparency(|t| t.frosted_glass = on);
                shell::follow_look_tx(app, tx);
            },
        ),
        unfocused_row(ctx, prefs, sub_disabled),
    ];
    group_card(&app.look, rows)
}

fn background_row(ctx: PageCtx<'_>, prefs: TransparencyPreferences, disabled: bool) -> Option<El> {
    let value_text = format!("{}%", prefs.background_transparency);
    let control = slider_control(
        ctx.app,
        SliderControl {
            key: "settings-background-transparency",
            label_key: "Background transparency",
            value_text,
            disabled,
            snaps: Vec::new(),
            get: |app| &app.settings_dialog.transparency.background,
            get_mut: |app| &mut app.settings_dialog.transparency.background,
            on_change: apply_background_transparency,
        },
    );
    option_row(
        ctx,
        RowSpec::new(
            "background-transparency",
            "Background transparency",
            "Transparency of the main file panes and preview background (0% is solid).",
        )
        .indented()
        .disabled(disabled),
        control,
    )
}

fn raised_row(ctx: PageCtx<'_>, prefs: TransparencyPreferences, disabled: bool) -> Option<El> {
    let value_text = format!("{}%", prefs.raised_surface_transparency);
    let control = slider_control(
        ctx.app,
        SliderControl {
            key: "settings-raised-transparency",
            label_key: "Raised surface transparency",
            value_text,
            disabled,
            snaps: Vec::new(),
            get: |app| &app.settings_dialog.transparency.raised,
            get_mut: |app| &mut app.settings_dialog.transparency.raised,
            on_change: apply_raised_transparency,
        },
    );
    option_row(
        ctx,
        RowSpec::new(
            "raised-transparency",
            "Raised surface transparency",
            "Independent transparency for raised chrome such as the sidebar, header, tab bar, and footer.",
        )
        .indented()
        .disabled(disabled),
        control,
    )
}

fn unfocused_row(ctx: PageCtx<'_>, prefs: TransparencyPreferences, disabled: bool) -> Option<El> {
    let value_text = format!("{:.2}\u{00D7}", prefs.unfocused_multiplier);
    let control = slider_control(
        ctx.app,
        SliderControl {
            key: "settings-unfocused-transparency",
            label_key: "Unfocused window multiplier",
            value_text,
            disabled,
            snaps: vec![Snap::at(1.0)],
            get: |app| &app.settings_dialog.transparency.unfocused_multiplier,
            get_mut: |app| &mut app.settings_dialog.transparency.unfocused_multiplier,
            on_change: apply_unfocused_multiplier,
        },
    );
    option_row(
        ctx,
        RowSpec::new(
            "unfocused-transparency",
            "Unfocused window multiplier",
            "Multiply transparency when the window is unfocused (higher is more see-through; 1.00× keeps it unchanged).",
        )
        .indented()
        .disabled(disabled),
        control,
    )
}

fn slider_control(app: &App, spec: SliderControl) -> El {
    let look = &app.look;
    let kit = &look.kit;
    let readout_font = TextStyle {
        size: (kit.ty.ui.size * 0.92).round().max(1.0),
        ..kit.ty.mono
    };
    let slider = Slider::new(spec.key, spec.get, spec.get_mut)
        .snaps(spec.snaps)
        .label(tr(spec.label_key))
        .value_text(spec.value_text.clone())
        .disabled(spec.disabled)
        .on_change(spec.on_change)
        .build(app, look);
    let readout = row().min_w(VALUE_WIDTH).justify_end().items_center().child(
        text(spec.value_text)
            .font(readout_font)
            .color(kit.c.text_muted)
            .a11y_hidden(),
    );
    row()
        .gap(look.space.md)
        .items_center()
        .child(row().w(SLIDER_WIDTH).items_center().child(slider))
        .child(readout)
}

fn apply_background_transparency(app: &mut App, value: f64, _: SliderChange, _: &mut Tx) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "slider range is clamped 0..=100"
    )]
    let pct = normalized_surface_transparency(value.round().max(0.0) as u32);
    if shell::transparency(app).background_transparency == pct {
        return;
    }
    shell::saved(app).change_transparency(|t| t.background_transparency = pct);
    shell::follow_look(app);
}

fn apply_raised_transparency(app: &mut App, value: f64, _: SliderChange, _: &mut Tx) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "slider range is clamped 0..=100"
    )]
    let pct = normalized_surface_transparency(value.round().max(0.0) as u32);
    if shell::transparency(app).raised_surface_transparency == pct {
        return;
    }
    shell::saved(app).change_transparency(|t| t.raised_surface_transparency = pct);
    shell::follow_look(app);
}

fn apply_unfocused_multiplier(app: &mut App, value: f64, _: SliderChange, _: &mut Tx) {
    let mult = normalized_unfocused_transparency_multiplier(value);
    if (shell::transparency(app).unfocused_multiplier - mult).abs() < f64::EPSILON {
        return;
    }
    shell::saved(app).change_transparency(|t| t.unfocused_multiplier = mult);
    shell::follow_look(app);
}
