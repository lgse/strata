// SPDX-License-Identifier: MIT

//! GPU shader compositor for Rune `El::effect` slots backed by `rune-fx`.
//!
//! At rest (when no `KIND_EFFECT` slot is in the frame's painter stream), [`FxCompositor`]
//! delegates straight to `Frame::draw_ui` without allocating `FxRenderer`, reports
//! `draws_own() == false` and `animating() == false`, and adds zero idle CPU or GPU cost.

use rune::{
    compose::{Compositor, DeviceKey, Frame},
    render::{EffectPhase, EffectSite, Instance, KIND_EFFECT},
};
use rune_fx::{
    DistortParams, FireParams, FireStyle, Fx, FxFrame, FxLayer, FxOptions, FxRenderer, FxSlot,
    Shape,
};

#[cfg(test)]
mod tests;

/// Effect slot mark: rising backdrop heat haze above the bottom window blaze.
pub(crate) const MARK_FIRE_HAZE: u32 = 0x10F0_0001;
/// Effect slot mark: fluid-simulated bottom fire wall, smoke, and rising embers.
pub(crate) const MARK_FIRE_BOTTOM: u32 = 0x10F0_0002;
/// Effect slot mark: blazing filament border wrapping the window perimeter.
pub(crate) const MARK_FIRE_BORDER: u32 = 0x10F0_0003;
/// Effect slot mark: fluid-simulated heat flames licking off the centered `10X` emblem.
pub(crate) const MARK_FIRE_BADGE_FLAME: u32 = 0x10F0_0004;
/// Effect slot mark: heat haze shimmering above the centered `10X` emblem.
pub(crate) const MARK_FIRE_BADGE_HAZE: u32 = 0x10F0_0005;

struct GpuFx {
    device_key: DeviceKey,
    format: wgpu::TextureFormat,
    renderer: FxRenderer,
}

/// Decoded effect slot waiting to be prepared and drawn in painter order.
pub(crate) struct DecodedSlot {
    pub(crate) index: u32,
    pub(crate) layer: FxLayer,
    pub(crate) placement: FxSlot,
}

/// Window compositor that renders Strata's `rune-fx` effect slots and stays completely
/// transparent at rest.
pub(crate) struct FxCompositor {
    gpu: Option<GpuFx>,
    active: bool,
    animating: bool,
}

impl FxCompositor {
    pub(crate) const fn new() -> Self {
        Self {
            gpu: None,
            active: false,
            animating: false,
        }
    }

    fn renderer_for(
        &mut self,
        device: &wgpu::Device,
        device_key: DeviceKey,
        format: wgpu::TextureFormat,
    ) -> &mut FxRenderer {
        let rebuild = self
            .gpu
            .as_ref()
            .is_none_or(|gpu| gpu.device_key != device_key || gpu.format != format);
        if rebuild {
            let mut renderer = FxRenderer::new(
                device,
                format,
                FxOptions {
                    dither: is_8bit_format(format),
                },
            );
            renderer.add_format(device, format.remove_srgb_suffix());
            renderer.add_format(device, format.add_srgb_suffix());
            renderer.add_format(device, wgpu::TextureFormat::Rgba16Float);
            renderer.set_fire_sim(device, true);
            self.gpu = Some(GpuFx {
                device_key,
                format,
                renderer,
            });
        }
        &mut self
            .gpu
            .as_mut()
            .expect("FxRenderer just initialized")
            .renderer
    }
}

impl<S: 'static> Compositor<S> for FxCompositor {
    fn render(&mut self, f: &mut Frame<'_, S>) {
        let slots = collect_slots(&f.rt.instances, &f.rt.marks);
        if slots.is_empty() {
            self.active = false;
            self.animating = false;
            f.draw_ui();
            return;
        }

        let layers: Vec<FxLayer> = slots.iter().map(|s| s.layer).collect();
        let placements: Vec<FxSlot> = slots.iter().map(|s| s.placement).collect();
        self.active = true;
        self.animating = layers.iter().any(FxLayer::is_animating);

        let (device, queue, size) = (f.device, f.queue, f.size);
        let device_key = f.device_key();
        let srgb_blend = f.srgb_blend();
        let fx = self.renderer_for(device, device_key, f.format);
        fx.set_srgb_blend(srgb_blend);
        fx.prepare_with_slots(
            device,
            queue,
            &FxFrame {
                viewport: [size.0, size.1],
                scale: 1.0,
                layers: &layers,
                backdrop: None,
                sources: &[],
            },
            &placements,
        );
        fx.simulate(queue, f.encoder, None);

        let clear = f.rt.clear_color();
        f.ui.prepare(device, queue, &mut f.rt.text.atlas, &f.rt.instances, size);
        f.ui.draw_with_backdrop(
            f.encoder,
            f.target,
            clear,
            &mut |phase, site: &EffectSite<'_>| {
                let Some(layer_idx) = slots.iter().position(|s| s.index == site.index) else {
                    return;
                };
                match phase {
                    EffectPhase::Backdrop(enc) => {
                        if let Some(bd) = site.backdrop {
                            fx.blur_backdrop(device, queue, enc, bd, layer_idx);
                        }
                    }
                    EffectPhase::Draw(pass) => {
                        fx.draw_layer_in(pass, site.format, site.origin, layer_idx);
                    }
                }
            },
        );
    }

    fn animating(&self) -> bool {
        self.animating
    }

    fn draws_own(&self) -> bool {
        self.active
    }
}

/// Scans `instances` for recognized `KIND_EFFECT` slots and decodes them into `rune-fx` layers.
pub(crate) fn collect_slots(instances: &[Instance], marks: &[u32]) -> Vec<DecodedSlot> {
    let mut out = Vec::new();
    for (idx, inst) in instances.iter().enumerate() {
        if inst.params[1] != KIND_EFFECT {
            continue;
        }
        let mark = decode_mark(inst, marks.get(idx).copied().unwrap_or(0));
        let Some(layer) = decode_layer(mark, inst) else {
            continue;
        };
        let placement = FxSlot::default()
            .scale(inst.params[3].max(1e-3))
            .clip(inst.clip, inst.fade);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "painter instance index fits u32"
        )]
        out.push(DecodedSlot {
            index: idx as u32,
            layer,
            placement,
        });
    }
    out
}

/// Recovers the 32-bit `El::effect` tag from Rune's packed 16-bit halves in `border[0..2]`.
fn decode_mark(inst: &Instance, fallback: u32) -> u32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Rune packs exact non-negative 16-bit halves into border[0..2]"
    )]
    let packed = (inst.border[0] as u32 & 0xFFFF) | ((inst.border[1] as u32 & 0xFFFF) << 16);
    if packed != 0 { packed } else { fallback }
}

/// Decodes one `KIND_EFFECT` instance into a `rune-fx` layer based on its `mark` and `effect_data`.
pub(crate) fn decode_layer(mark: u32, inst: &Instance) -> Option<FxLayer> {
    let scale = inst.params[3].max(1e-3);
    let [x, y, w, h] = inst.rect.map(|v| v / scale);
    let radius = inst.radii.iter().fold(0.0_f32, |a, v| a.max(v.abs())) / scale;
    let [time, intensity, opacity, seed_f] = inst.uv;
    if opacity <= 0.0 || intensity <= 0.0 || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let is_light = seed_f < 0.0;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "effect seed is a small non-negative integer; sign encodes light-theme contrast"
    )]
    let seed = seed_f.abs() as u32;

    let layer = match mark {
        MARK_FIRE_HAZE => {
            let shape = bottom_hearth_shape(x, y, w, h);
            let haze_height = (h * 0.48).clamp(180.0, 380.0);
            Fx::distort(DistortParams::HeatHaze {
                height: haze_height,
                scale: 14.0,
                speed: 2.2,
                amount: 2.8 * intensity,
                whole_shape: false,
            })
            .on(shape)
            .at(time)
            .seed(seed.wrapping_add(11))
            .opacity(opacity)
        }
        MARK_FIRE_BOTTOM => {
            let shape = bottom_hearth_shape(x, y, w, h);
            // Quantize height to whole points so the fluid sim grid stays stable across frames.
            let fire_height = (h * 0.27).clamp(130.0, 220.0).round();
            Fx::fire(bottom_fire_params(fire_height, intensity, is_light))
                .on(shape)
                .at(time)
                .seed(seed.wrapping_add(5))
                .opacity(opacity)
        }
        MARK_FIRE_BORDER => {
            let shape = inset_border_shape(x, y, w, h, radius);
            Fx::fire(border_fire_params(intensity, is_light))
                .on(shape)
                .at(time)
                .seed(seed.wrapping_add(17))
                .opacity(opacity)
        }
        MARK_FIRE_BADGE_FLAME => {
            let shape = badge_shape(x, y, w, h, radius);
            Fx::fire(badge_fire_params(intensity, is_light))
                .on(shape)
                .at(time)
                .seed(seed.wrapping_add(23))
                .opacity(opacity)
        }
        MARK_FIRE_BADGE_HAZE => {
            let shape = badge_shape(x, y, w, h, radius);
            Fx::distort(DistortParams::HeatHaze {
                height: 42.0,
                scale: 9.0,
                speed: 2.1,
                amount: 1.35 * intensity,
                whole_shape: false,
            })
            .on(shape)
            .at(time)
            .seed(seed.wrapping_add(29))
            .opacity(opacity)
        }
        _ => return None,
    };
    Some(layer)
}

/// Hearth strip anchored along the window bottom so `FireStyle::Top` flames erupt upward cleanly
/// across the full width with a non-zero corner radius for the fluid sim's edge taper.
fn bottom_hearth_shape(x: f32, y: f32, w: f32, h: f32) -> Shape {
    const EDGE_PAD: f32 = 16.0;
    const BASE_LIFT: f32 = 6.0;
    const HEARTH_HEIGHT: f32 = 48.0;
    const HEARTH_RADIUS: f32 = 14.0;
    Shape::rounded(
        x - EDGE_PAD,
        y + h - BASE_LIFT,
        w + 2.0 * EDGE_PAD,
        HEARTH_HEIGHT,
        HEARTH_RADIUS,
    )
}

/// Slightly inset window rect so `FireStyle::Border`'s outer filament band hugs inside the frame.
fn inset_border_shape(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Shape {
    const INSET: f32 = 6.0;
    let inset = INSET.min(w * 0.25).min(h * 0.25);
    let iw = (w - 2.0 * inset).max(4.0);
    let ih = (h - 2.0 * inset).max(4.0);
    let ir = (radius - inset).clamp(2.0, iw.min(ih) * 0.5);
    Shape::rounded(x + inset, y + inset, iw, ih, ir)
}

/// Badge shape guaranteed to keep a non-zero corner radius for `firesim.wgsl`'s edge taper.
fn badge_shape(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Shape {
    let max_r = (w.min(h) * 0.5).max(1.0);
    let safe_r = radius.clamp(6.0_f32.min(max_r), max_r);
    Shape::rounded(x, y, w, h, safe_r)
}

fn bottom_fire_params(height: f32, intensity: f32, is_light: bool) -> FireParams {
    FireParams {
        style: FireStyle::Top,
        height,
        scale: 38.0,
        speed: 1.75,
        intensity: 1.18 * intensity,
        embers: 38,
        ember_rise: 165.0,
        ember_size: 1.05,
        ember_life: 1.55,
        warp: 1.15,
        sides: 0.36,
        base_glow: 0.9,
        occlusion: if is_light { 0.95 } else { 0.40 },
        swirl: 0.35,
        smoke: if is_light { 0.85 } else { 0.65 },
        ..FireParams::default()
    }
}

fn border_fire_params(intensity: f32, is_light: bool) -> FireParams {
    FireParams {
        style: FireStyle::Border,
        height: 36.0,
        scale: 22.0,
        speed: 1.45,
        intensity: (if is_light { 1.1 } else { 0.92 }) * intensity,
        embers: 0,
        warp: 1.0,
        sides: 0.52,
        base_glow: 0.65,
        occlusion: if is_light { 0.95 } else { 0.26 },
        swirl: 0.65,
        smoke: 0.0,
        ..FireParams::default()
    }
}

fn badge_fire_params(intensity: f32, is_light: bool) -> FireParams {
    FireParams {
        height: 32.0,
        scale: 12.0,
        speed: 1.5,
        intensity: 1.1 * intensity,
        embers: 10,
        ember_rise: 48.0,
        ember_size: 0.75,
        ember_life: 1.2,
        warp: 0.9,
        base_glow: 0.55,
        occlusion: if is_light { 0.95 } else { 0.20 },
        smoke: if is_light { 0.6 } else { 0.35 },
        ..FireParams::heat()
    }
}

fn is_8bit_format(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba8Unorm
            | wgpu::TextureFormat::Rgba8UnormSrgb
            | wgpu::TextureFormat::Bgra8Unorm
            | wgpu::TextureFormat::Bgra8UnormSrgb
    )
}
