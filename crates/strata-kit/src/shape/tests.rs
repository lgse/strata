use super::*;
use crate::look::{Look, RowHeights};
use crate::slots;

/// Every token K2 writes, for comparing two looks.
#[derive(Debug, PartialEq)]
struct K2Tokens {
    radius: [f32; 5],
    roundness: f32,
    ty: Vec<rune::TextStyle>,
    ramp: crate::look::Ramp,
    icon_size: crate::look::IconSizes,
    frame: crate::look::Frame,
    content: crate::look::Content,
    row: RowHeights,
    font: Option<Families>,
    glow_on: bool,
    animations: bool,
}

fn tokens(look: &Look) -> K2Tokens {
    let r = look.kit.radius;
    K2Tokens {
        radius: [r.xs, r.sm, r.md, r.lg, r.xl],
        roundness: look.roundness,
        ty: {
            let t = look.kit.ty;
            vec![
                t.ui,
                t.ui_medium,
                t.ui_compact,
                t.small,
                t.small_medium,
                t.body,
                t.body_medium,
                t.text_small,
                t.text_small_medium,
                t.text_xsmall,
                t.text_xsmall_medium,
                t.mono,
            ]
        },
        ramp: look.ty,
        icon_size: look.icon_size,
        frame: look.frame,
        content: look.content,
        row: look.row,
        font: look.font.clone(),
        glow_on: look.glow.on,
        animations: look.animations,
    }
}

/// Parameter sets at Strata's defaults and at the ends of every range.
fn samples() -> Vec<LookParams> {
    let d = LookParams::default();
    vec![
        d.clone(),
        LookParams {
            roundness: 0,
            frame_density: 0,
            content_density: 0,
            text_size: 8,
            ..d.clone()
        },
        LookParams {
            roundness: 200,
            frame_density: 12,
            content_density: 16,
            text_size: 48,
            text_scale: 1.25,
            font: Some("DejaVu Sans".into()),
            element_glow: false,
            animations: false,
            ..d.clone()
        },
        LookParams {
            text_size: 10,
            text_scale: 1.1818,
            frame_density: 5,
            content_density: 9,
            roundness: 40,
            ..d
        },
    ]
}

#[test]
fn applying_the_same_params_twice_changes_nothing() {
    for p in samples() {
        let once = Look::fallback(false).with_params(&p);
        let twice = once.clone().with_params(&p);
        assert_eq!(tokens(&once), tokens(&twice), "{p:?}");
    }
}

#[test]
fn new_params_replace_old_ones_completely() {
    for old in samples() {
        for new in samples() {
            let fresh = Look::fallback(true).with_params(&new);
            let over = Look::fallback(true).with_params(&old).with_params(&new);
            assert_eq!(tokens(&fresh), tokens(&over), "{old:?} then {new:?}");
        }
    }
}

/// A host rebuilds the look from each new theme's kit; the user's switches must survive that.
#[test]
fn a_look_rebuilt_from_another_kit_keeps_the_glow_and_animation_switches() {
    let off = LookParams {
        element_glow: false,
        animations: false,
        ..LookParams::default()
    };
    let before = Look::fallback(false).with_params(&off);
    let rebuilt = Look::from_kit(rune_kit::Kit::neutral(true)).with_params(&off);
    for look in [before, rebuilt] {
        assert!(!look.glow.on);
        assert!(!look.animations);
    }
    let on = Look::fallback(false)
        .with_params(&off)
        .with_params(&LookParams::default());
    assert!(on.glow.on && on.animations);
}

#[test]
fn zero_roundness_squares_every_corner() {
    let look = Look::fallback(false).with_params(&LookParams {
        roundness: 0,
        ..LookParams::default()
    });
    let r = look.kit.radius;
    assert_eq!([r.xs, r.sm, r.md, r.lg, r.xl], [0.0; 5]);
    assert_eq!(look.round(12.0), 0.0);
}

#[test]
fn full_roundness_doubles_every_radius_from_one_hundred_percent() {
    let at = |roundness| {
        Look::fallback(false).with_params(&LookParams {
            roundness,
            ..LookParams::default()
        })
    };
    let (full, double) = (at(100), at(200));
    let (a, b) = (full.kit.radius, double.kit.radius);
    for (x, y) in [
        (a.xs, b.xs),
        (a.sm, b.sm),
        (a.md, b.md),
        (a.lg, b.lg),
        (a.xl, b.xl),
    ] {
        assert!(x > 0.0);
        assert_eq!(y, 2.0 * x);
    }
    assert_eq!(double.round(5.0), 2.0 * full.round(5.0));
}

#[test]
fn scaled_radii_round_to_hundredths() {
    assert_eq!(scale_radius(9.0, factor(40)), 3.6);
    assert_eq!(scale_radius(5.0, 1.0 / 3.0), 1.67);
}

#[test]
fn roundness_outside_its_range_is_clamped() {
    assert_eq!(factor(250), factor(ROUNDNESS.max));
    assert_eq!(ROUNDNESS.stepped(ROUNDNESS.max, 1), ROUNDNESS.max);
    assert_eq!(ROUNDNESS.stepped(ROUNDNESS.min, -1), ROUNDNESS.min);
    assert_eq!(ROUNDNESS.stepped(195, 0), ROUNDNESS.clamp(195));
    assert_eq!(ROUNDNESS.stepped(60, -2), 40);
}

#[test]
fn params_leave_spacing_ladders_and_the_glow_look_alone() {
    let fallback = Look::fallback(false);
    for p in samples() {
        let look = fallback.clone().with_params(&p);
        assert_eq!(look.space, fallback.space, "{p:?}");
        assert_eq!(look.tiles, fallback.tiles, "{p:?}");
        assert_eq!(look.thumbnails, fallback.thumbnails, "{p:?}");
        // The params own only the switch; the theme owns the glow's colour and reach.
        let glow = crate::look::Glow {
            on: fallback.glow.on,
            ..look.glow
        };
        assert_eq!(glow, fallback.glow, "{p:?}");
    }
}

#[test]
fn slot_tokens_follow_the_params() {
    let small = Look::fallback(false).with_params(&LookParams::default());
    let airy = Look::fallback(false).with_params(&LookParams {
        content_density: 8,
        frame_density: 4,
        ..LookParams::default()
    });
    assert!(slots::list::row_height(&airy) > slots::list::row_height(&small));
    assert!(slots::sidebar::row_height(&airy) > slots::sidebar::row_height(&small));
    assert!(slots::tabs::tab_height(&airy) > slots::tabs::tab_height(&small));
}
