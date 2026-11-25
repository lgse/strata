// SPDX-License-Identifier: MIT

use rune::{El, Tx};
use strata_components::controls::slider::{Range, Snap};
use strata_desktop::settings::{
    MAX_SURFACE_TRANSPARENCY, MAX_UNFOCUSED_TRANSPARENCY_MULTIPLIER, MIN_SURFACE_TRANSPARENCY,
    MIN_UNFOCUSED_TRANSPARENCY_MULTIPLIER, TransparencyPreferences,
    UNFOCUSED_TRANSPARENCY_MULTIPLIER_STEP, normalized_surface_transparency,
    normalized_unfocused_transparency_multiplier,
};

use super::controls::{PageCtx, RowSpec, group_card, option_row, switch_row};
use super::slider_field::{SliderField, SliderFieldState, slider_field_access};
use crate::{App, shell};

/// Paired slider and numeric input states for the three transparency controls in Settings → Appearance.
pub(super) struct TransparencySliders {
    background: SliderFieldState,
    raised: SliderFieldState,
    unfocused_multiplier: SliderFieldState,
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
            background: SliderFieldState::new(
                f64::from(defaults.background_transparency),
                percent_range,
                "%",
            ),
            raised: SliderFieldState::new(
                f64::from(defaults.raised_surface_transparency),
                percent_range,
                "%",
            ),
            unfocused_multiplier: SliderFieldState::new(
                defaults.unfocused_multiplier,
                multiplier_range,
                "\u{00D7}",
            ),
        }
    }

    pub(super) fn hold(&mut self, held: bool) {
        self.background.hold(held);
        self.raised.hold(held);
        self.unfocused_multiplier.hold(held);
    }

    pub(super) fn has_focused_field(&self) -> bool {
        self.background.focused() || self.raised.focused() || self.unfocused_multiplier.focused()
    }

    /// Keeps slider positions and numeric inputs aligned with saved settings when not actively edited.
    pub(super) fn sync(&mut self, prefs: TransparencyPreferences) {
        self.background
            .sync(f64::from(prefs.background_transparency));
        self.raised
            .sync(f64::from(prefs.raised_surface_transparency));
        self.unfocused_multiplier.sync(prefs.unfocused_multiplier);
    }
}

pub(super) fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    changed |= background_field().poll(app, tx);
    changed |= raised_field().poll(app, tx);
    changed |= unfocused_field().poll(app, tx);
    changed
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
        background_row(ctx, sub_disabled),
        raised_row(ctx, sub_disabled),
        switch_row(
            ctx,
            RowSpec::new(
                "frosted-glass",
                "Frosted glass",
                "Apply backdrop blur and a subtle frosted sheen to translucent dialogs, menus, and popovers.",
            )
            .note(Some(
                "Requires a compositor that supports backdrop blur and has it enabled.",
            ))
            .indented()
            .disabled(sub_disabled),
            prefs.frosted_glass,
            |app, on, tx| {
                shell::saved(app).change_transparency(|t| t.frosted_glass = on);
                shell::follow_look_tx(app, tx);
            },
        ),
        unfocused_row(ctx, sub_disabled),
    ];
    group_card(&app.look, rows)
}

fn background_field() -> SliderField {
    SliderField::new(
        "settings-background-transparency",
        "Background transparency",
        slider_field_access!(settings_dialog.transparency.background),
        apply_background_transparency,
    )
}

fn raised_field() -> SliderField {
    SliderField::new(
        "settings-raised-transparency",
        "Raised surface transparency",
        slider_field_access!(settings_dialog.transparency.raised),
        apply_raised_transparency,
    )
}

fn unfocused_field() -> SliderField {
    SliderField::new(
        "settings-unfocused-transparency",
        "Unfocused window multiplier",
        slider_field_access!(settings_dialog.transparency.unfocused_multiplier),
        apply_unfocused_multiplier,
    )
    .snaps(vec![Snap::at(1.0)])
}

fn background_row(ctx: PageCtx<'_>, disabled: bool) -> Option<El> {
    let control = background_field().disabled(disabled).build(ctx.app);
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

fn raised_row(ctx: PageCtx<'_>, disabled: bool) -> Option<El> {
    let control = raised_field().disabled(disabled).build(ctx.app);
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

fn unfocused_row(ctx: PageCtx<'_>, disabled: bool) -> Option<El> {
    let control = unfocused_field().disabled(disabled).build(ctx.app);
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

fn apply_background_transparency(app: &mut App, value: f64, _: &mut Tx) {
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

fn apply_raised_transparency(app: &mut App, value: f64, _: &mut Tx) {
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

fn apply_unfocused_multiplier(app: &mut App, value: f64, _: &mut Tx) {
    let mult = normalized_unfocused_transparency_multiplier(value);
    if (shell::transparency(app).unfocused_multiplier - mult).abs() < f64::EPSILON {
        return;
    }
    shell::saved(app).change_transparency(|t| t.unfocused_multiplier = mult);
    shell::follow_look(app);
}
