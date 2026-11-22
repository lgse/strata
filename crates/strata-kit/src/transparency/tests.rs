// SPDX-License-Identifier: MIT

use super::*;
use crate::look::{Look, LookParams};

fn approx(a: f32, b: f32) {
    assert!(
        (a - b).abs() < 1e-5,
        "expected {a} to be approximately {b} (diff = {})",
        (a - b).abs()
    );
}

#[test]
fn disabled_transparency_keeps_every_surface_completely_opaque() {
    let base = Look::fallback(true);
    let mut look = base.clone();
    look.apply_params(&LookParams {
        transparency: TransparencyParams {
            enabled: false,
            background_percent: 80,
            raised_percent: 60,
            frosted_glass: true,
            unfocused_multiplier: 1.5,
            window_active: false,
        },
        ..LookParams::default()
    });

    assert!(!look.transparency.enabled);
    assert!(!look.transparency.frosted_glass);
    approx(look.transparency.background_alpha, 1.0);
    approx(look.transparency.raised_alpha, 1.0);
    approx(look.transparency.overlay_alpha, 1.0);
    approx(look.transparency.backdrop_blur, 0.0);
    assert_eq!(look.kit.c.bg_base, base.kit.c.bg_base);
    assert_eq!(look.kit.c.bg_deep, base.kit.c.bg_deep);
    assert_eq!(look.kit.c.layer_01, base.kit.c.layer_01);
    assert_eq!(look.kit.c.text_base, base.kit.c.text_base);
}

#[test]
fn enabled_transparency_scales_background_raised_and_overlay_independently_and_keeps_foreground_opaque()
 {
    let base = Look::fallback(true);
    let mut look = base.clone();
    look.apply_params(&LookParams {
        transparency: TransparencyParams {
            enabled: true,
            background_percent: 30,
            raised_percent: 60,
            frosted_glass: true,
            unfocused_multiplier: 1.0,
            window_active: true,
        },
        ..LookParams::default()
    });

    assert!(look.transparency.enabled);
    assert!(look.transparency.frosted_glass);
    approx(look.transparency.background_alpha, 0.70);
    approx(look.transparency.raised_alpha, 0.40);
    approx(look.transparency.overlay_alpha, OVERLAY_ALPHA_FLOOR_FROSTED);
    approx(look.transparency.backdrop_blur, FROSTED_BLUR_SIGMA);

    approx(look.kit.c.bg_base.a, 0.70);
    approx(look.kit.c.page.a, 0.70);
    approx(look.kit.c.bg_deep.a, 0.40);
    approx(look.kit.c.surface_raised.a, 0.40);
    approx(look.kit.c.layer_01.a, OVERLAY_ALPHA_FLOOR_FROSTED);

    // Foreground text, icons, accents, and borders stay at their original full opacity.
    assert_eq!(look.kit.c.text_base, base.kit.c.text_base);
    assert_eq!(look.kit.c.text_muted, base.kit.c.text_muted);
    assert_eq!(look.kit.c.icon_base, base.kit.c.icon_base);
    assert_eq!(look.kit.c.icon_accent, base.kit.c.icon_accent);
    assert_eq!(look.kit.c.bg_accent, base.kit.c.bg_accent);
    assert_eq!(look.kit.c.border_base, base.kit.c.border_base);
    assert_eq!(look.kit.c.text_inverse, base.kit.c.text_inverse);
}

#[test]
fn overlay_floor_tracks_frosted_glass_toggle_and_raises_only_when_below_floor() {
    let plain = Transparency::resolve(
        TransparencyParams {
            enabled: true,
            background_percent: 50,
            raised_percent: 50,
            frosted_glass: false,
            unfocused_multiplier: 1.0,
            window_active: true,
        },
        true,
    );
    assert!(!plain.frosted_glass);
    approx(plain.backdrop_blur, 0.0);
    approx(plain.overlay_alpha, OVERLAY_ALPHA_FLOOR_PLAIN);

    let subtle = Transparency::resolve(
        TransparencyParams {
            enabled: true,
            background_percent: 5,
            raised_percent: 5,
            frosted_glass: true,
            unfocused_multiplier: 1.0,
            window_active: true,
        },
        true,
    );
    approx(subtle.raised_alpha, 0.95);
    approx(subtle.overlay_alpha, 0.95);
}

#[test]
fn unfocused_multiplier_scales_transparency_when_inactive_and_clamps_cleanly() {
    // 1.50x makes 20% transparent -> 30% transparent (0.70 alpha).
    let more_clear = Transparency::resolve(
        TransparencyParams {
            enabled: true,
            background_percent: 20,
            raised_percent: 10,
            frosted_glass: true,
            unfocused_multiplier: 1.5,
            window_active: false,
        },
        true,
    );
    approx(more_clear.background_alpha, 0.70);
    approx(more_clear.raised_alpha, 0.85);

    // 0.50x makes 40% transparent -> 20% transparent (more solid when unfocused).
    let more_solid = Transparency::resolve(
        TransparencyParams {
            enabled: true,
            background_percent: 40,
            raised_percent: 20,
            frosted_glass: true,
            unfocused_multiplier: 0.5,
            window_active: false,
        },
        true,
    );
    approx(more_solid.background_alpha, 0.80);
    approx(more_solid.raised_alpha, 0.90);

    // 1.00x keeps focused and unfocused identical.
    let same = Transparency::resolve(
        TransparencyParams {
            enabled: true,
            background_percent: 25,
            raised_percent: 15,
            frosted_glass: true,
            unfocused_multiplier: 1.0,
            window_active: false,
        },
        true,
    );
    approx(same.background_alpha, 0.75);
    approx(same.raised_alpha, 0.85);

    // Clamps cleanly at 0.0 alpha when percent * multiplier exceeds 100%.
    let clamped = Transparency::resolve(
        TransparencyParams {
            enabled: true,
            background_percent: 80,
            raised_percent: 70,
            frosted_glass: false,
            unfocused_multiplier: 2.0,
            window_active: false,
        },
        true,
    );
    approx(clamped.background_alpha, 0.0);
    approx(clamped.raised_alpha, 0.0);
    approx(clamped.overlay_alpha, OVERLAY_ALPHA_FLOOR_PLAIN);
}

#[test]
fn apply_params_is_idempotent_across_zero_alpha_and_re_disabling() {
    let base = Look::fallback(true);
    let mut look = base.clone();

    look.apply_params(&LookParams {
        transparency: TransparencyParams {
            enabled: true,
            background_percent: 100,
            raised_percent: 100,
            frosted_glass: true,
            unfocused_multiplier: 1.0,
            window_active: true,
        },
        ..LookParams::default()
    });
    approx(look.kit.c.bg_base.a, 0.0);
    approx(look.kit.c.bg_deep.a, 0.0);

    let mid = LookParams {
        transparency: TransparencyParams {
            enabled: true,
            background_percent: 25,
            raised_percent: 10,
            frosted_glass: false,
            unfocused_multiplier: 1.0,
            window_active: true,
        },
        ..LookParams::default()
    };
    look.apply_params(&mid);
    let first_bg = look.kit.c.bg_base;
    let first_deep = look.kit.c.bg_deep;
    look.apply_params(&mid);
    assert_eq!(look.kit.c.bg_base, first_bg);
    assert_eq!(look.kit.c.bg_deep, first_deep);
    approx(look.kit.c.bg_base.a, 0.75);
    approx(look.kit.c.bg_deep.a, 0.90);

    look.apply_params(&LookParams::default());
    assert_eq!(look.kit.c.bg_base, base.kit.c.bg_base);
    assert_eq!(look.kit.c.bg_deep, base.kit.c.bg_deep);
    assert_eq!(look.kit.c.layer_01, base.kit.c.layer_01);
}
