// SPDX-License-Identifier: MIT

use std::sync::OnceLock;

use rune::{El, Pixels, Shadow, SurfaceFit, effect, el};

use crate::fx::{
    MARK_ELECTRIC_BORDER, MARK_ELECTRIC_CHARGED, MARK_PLASMA_BORDER, electric_arc_mark,
};

use super::palette::{
    FxFrame, clamp01, ease_in_out_cubic, ease_out_cubic, envelope, segment, spring_overshoot,
};

/// Element key for the mounted splash artwork card (used by behavioral tests).
pub(crate) const SPLASH_CARD_KEY: &str = "tenxer-plasma-splash-card";

const ART: &[u8] = include_bytes!("../../../../data/brand/tenxer-splash.png");
const MIN_ARC_DISTANCE: f32 = 24.0;
const WINDOW_ARC_INSET: f32 = 18.0;

struct DecodedArt {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn decoded_art() -> Option<&'static DecodedArt> {
    static ART_CACHE: OnceLock<Option<DecodedArt>> = OnceLock::new();
    ART_CACHE
        .get_or_init(|| {
            let rgba = image::load_from_memory_with_format(ART, image::ImageFormat::Png)
                .ok()?
                .into_rgba8();
            let (width, height) = rgba.dimensions();
            Some(DecodedArt {
                width,
                height,
                rgba: rgba.into_raw(),
            })
        })
        .as_ref()
}

/// Builds a `Pixels` texture handle from the lazily-decoded splash PNG (decoded at most once).
pub(super) fn create_pixels() -> Option<Pixels> {
    let art = decoded_art()?;
    let pixels = Pixels::new();
    pixels.set(art.width, art.height, art.rgba.clone());
    Some(pixels)
}

#[derive(Clone, Copy)]
struct CardRect {
    x: f32,
    y: f32,
    side: f32,
    radius: f32,
}

#[derive(Clone, Copy)]
struct SplashLifecycle {
    card_opacity: f32,
    plasma_opacity: f32,
    plasma_intensity: f32,
    border_opacity: f32,
    border_intensity: f32,
    charged_opacity: f32,
    card: CardRect,
}

impl SplashLifecycle {
    fn at(frame: &FxFrame<'_>) -> Self {
        let t = frame.t;
        let enter_t = segment(t, 0.0, 0.20);
        let exit_t = segment(t, 0.76, 1.0);
        let rise = ease_out_cubic(enter_t);
        let pop = spring_overshoot(enter_t);
        let exit_e = ease_in_out_cubic(exit_t);

        let opacity_in = ease_out_cubic(segment(t, 0.0, 0.16));
        let card_opacity = clamp01(opacity_in * (1.0 - exit_e));

        let max_fit = frame.height.min(frame.width).max(1.0);
        let base = (frame.height * 0.64)
            .min(frame.width * 0.80)
            .clamp(160.0_f32.min(max_fit), 560.0);
        let scale = 0.86 + 0.14 * pop;
        let side = (base * scale).max(1.0);
        let translate_y = 20.0 * (1.0 - rise) - 10.0 * exit_e;
        let card = CardRect {
            x: (frame.width - side) * 0.5,
            y: (frame.height - side) * 0.5 + translate_y,
            side,
            radius: frame.look.round(14.0),
        };

        let plasma_in = ease_out_cubic(segment(t, 0.02, 0.18));
        let plasma_out = 1.0 - ease_in_out_cubic(segment(t, 0.76, 0.98));
        let plasma_opacity = clamp01(plasma_in * plasma_out);
        let plasma_surge = 0.92 + 0.22 * envelope(t, 0.04, 0.18, 0.42);

        let border_in = ease_out_cubic(segment(t, 0.04, 0.18));
        let border_out = 1.0 - ease_in_out_cubic(segment(t, 0.74, 0.96));
        let border_opacity = clamp01(border_in * border_out);
        let border_surge = 0.95 + 0.35 * envelope(t, 0.04, 0.14, 0.32);

        let charged_in = ease_out_cubic(segment(t, 0.02, 0.18));
        let charged_out = 1.0 - ease_in_out_cubic(segment(t, 0.74, 0.98));
        let charged_opacity = clamp01(charged_in * charged_out);

        Self {
            card_opacity,
            plasma_opacity,
            plasma_intensity: plasma_opacity * plasma_surge,
            border_opacity,
            border_intensity: border_opacity * border_surge,
            charged_opacity,
            card,
        }
    }
}

pub(super) fn build(frame: &FxFrame<'_>, pixels: Option<&Pixels>) -> El {
    let life = SplashLifecycle::at(frame);
    let mut root = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));

    if let Some(wash) = ambient_glow(frame, life) {
        root = root.child(wash);
    }
    if let Some(charged) = charged_perimeter(frame, life) {
        root = root.child(charged);
    }
    for arc in corner_arcs(frame, life) {
        root = root.child(arc);
    }
    if let Some(plasma) = plasma_border(frame, life) {
        root = root.child(plasma);
    }
    if let Some(card) = splash_card(frame, life, pixels) {
        root = root.child(card);
    }
    if let Some(electric) = overload_border(frame, life) {
        root = root.child(electric);
    }
    root
}

fn ambient_glow(frame: &FxFrame<'_>, life: SplashLifecycle) -> Option<El> {
    if life.card_opacity <= 0.005 {
        return None;
    }
    let rx = (life.card.side * 0.85).max(frame.width * 0.28);
    let ry = (life.card.side * 0.85).max(frame.height * 0.32);
    let stops = [
        (0.0, frame.focus(0.18 * life.card_opacity)),
        (0.48, frame.accent(0.10 * life.card_opacity)),
        (1.0, frame.accent(0.0)),
    ];
    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radial_gradient([0.5, 0.5], [rx, ry], stops),
    )
}

fn charged_perimeter(frame: &FxFrame<'_>, life: SplashLifecycle) -> Option<El> {
    if life.charged_opacity <= 0.005 {
        return None;
    }
    Some(
        effect(MARK_ELECTRIC_CHARGED)
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radius(frame.window_radius())
            .effect_bleed(24.0)
            .effect_data(frame.effect_slot_data(life.charged_opacity, life.charged_opacity, 13)),
    )
}

fn plasma_border(frame: &FxFrame<'_>, life: SplashLifecycle) -> Option<El> {
    if life.plasma_opacity <= 0.005 {
        return None;
    }
    let c = life.card;
    Some(
        effect(MARK_PLASMA_BORDER)
            .absolute(c.x, c.y)
            .size(c.side, c.side)
            .radius(c.radius)
            .effect_bleed(72.0)
            .effect_data(frame.effect_slot_data(life.plasma_intensity, life.plasma_opacity, 19)),
    )
}

fn overload_border(frame: &FxFrame<'_>, life: SplashLifecycle) -> Option<El> {
    if life.border_opacity <= 0.005 {
        return None;
    }
    let c = life.card;
    Some(
        effect(MARK_ELECTRIC_BORDER)
            .absolute(c.x, c.y)
            .size(c.side, c.side)
            .radius(c.radius)
            .effect_bleed(72.0)
            .effect_data(frame.effect_slot_data(life.border_intensity, life.border_opacity, 23)),
    )
}

fn splash_card(frame: &FxFrame<'_>, life: SplashLifecycle, pixels: Option<&Pixels>) -> Option<El> {
    if life.card_opacity <= 0.005 {
        return None;
    }
    let c = life.card;
    let mut card = el()
        .key(SPLASH_CARD_KEY)
        .absolute(c.x, c.y)
        .size(c.side, c.side)
        .radius(c.radius)
        .fill(frame.palette.surface.alpha(0.92))
        .border(1.5, frame.focus(0.72))
        .box_shadow(Shadow {
            x: 0.0,
            y: 0.0,
            blur: 36.0,
            spread: 2.0,
            color: frame.accent(0.45 * life.card_opacity),
            inset: false,
        })
        .box_shadow(Shadow {
            x: 0.0,
            y: 0.0,
            blur: 14.0,
            spread: 0.0,
            color: frame.bright(0.32 * life.card_opacity),
            inset: true,
        })
        .a11y_hidden();

    if let Some(px) = pixels {
        card = card.pixels_fit(px, SurfaceFit::Fit);
    }
    Some(card.layer_opacity(life.card_opacity))
}

struct ArcSpec {
    from: [f32; 2],
    to: [f32; 2],
    strength: f32,
    seed_offset: u32,
}

fn corner_arcs(frame: &FxFrame<'_>, life: SplashLifecycle) -> Vec<El> {
    let t = frame.t;
    let slam = envelope(t, 0.04, 0.14, 0.30);
    let pair_a = slam
        .max(envelope(t, 0.32, 0.42, 0.56) * 0.90)
        .max(envelope(t, 0.60, 0.68, 0.78) * 0.85);
    let pair_b = slam
        .max(envelope(t, 0.24, 0.34, 0.48) * 0.90)
        .max(envelope(t, 0.48, 0.58, 0.74) * 0.88);

    let c = life.card;
    let r_inset = c.radius * 0.28;
    let pad = WINDOW_ARC_INSET
        .min(frame.width * 0.15)
        .min(frame.height * 0.15);
    let specs = [
        ArcSpec {
            from: [c.x + r_inset, c.y + r_inset],
            to: [pad, pad],
            strength: pair_a,
            seed_offset: 31,
        },
        ArcSpec {
            from: [c.x + c.side - r_inset, c.y + r_inset],
            to: [frame.width - pad, pad],
            strength: pair_b,
            seed_offset: 67,
        },
        ArcSpec {
            from: [c.x + r_inset, c.y + c.side - r_inset],
            to: [pad, frame.height - pad],
            strength: pair_b,
            seed_offset: 103,
        },
        ArcSpec {
            from: [c.x + c.side - r_inset, c.y + c.side - r_inset],
            to: [frame.width - pad, frame.height - pad],
            strength: pair_a,
            seed_offset: 139,
        },
    ];

    specs
        .into_iter()
        .filter_map(|spec| arc_slot(frame, spec))
        .collect()
}

fn arc_slot(frame: &FxFrame<'_>, spec: ArcSpec) -> Option<El> {
    if spec.strength <= 0.01 {
        return None;
    }
    let dist = (spec.to[0] - spec.from[0]).hypot(spec.to[1] - spec.from[1]);
    if dist < MIN_ARC_DISTANCE {
        return None;
    }
    let x = spec.from[0].min(spec.to[0]);
    let y = spec.from[1].min(spec.to[1]);
    let w = (spec.to[0] - spec.from[0]).abs().max(2.0);
    let h = (spec.to[1] - spec.from[1]).abs().max(2.0);
    let mark = electric_arc_mark(spec.from, spec.to);
    Some(
        effect(mark)
            .absolute(x, y)
            .size(w, h)
            .effect_bleed(110.0)
            .effect_data(frame.effect_slot_data(
                spec.strength * 1.15,
                spec.strength,
                spec.seed_offset,
            )),
    )
}
