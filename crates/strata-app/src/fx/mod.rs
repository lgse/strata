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
    ElectricParams, ElectricStyle, FireParams, Fx, FxFrame, FxLayer, FxOptions, FxRenderer, FxSlot,
    Shape, rgb,
};

#[cfg(test)]
mod tests;

/// Effect slot mark: swirling plasma filaments hugging a rounded splash card perimeter.
pub(crate) const MARK_PLASMA_BORDER: u32 = 0x10F0_0006;
/// Effect slot mark: crackling overload lightning crawling around a splash card perimeter.
pub(crate) const MARK_ELECTRIC_BORDER: u32 = 0x10F0_0007;
/// Effect slot mark: calm crawling charged electric hairline around the outer window edge.
pub(crate) const MARK_ELECTRIC_CHARGED: u32 = 0x10F0_0008;
/// Effect slot mark: forking lightning arc shooting bottom-right -> top-left across the slot rect.
pub(crate) const MARK_ELECTRIC_ARC_NW: u32 = 0x10F0_0009;
/// Effect slot mark: forking lightning arc shooting bottom-left -> top-right across the slot rect.
pub(crate) const MARK_ELECTRIC_ARC_NE: u32 = 0x10F0_000A;
/// Effect slot mark: forking lightning arc shooting top-right -> bottom-left across the slot rect.
pub(crate) const MARK_ELECTRIC_ARC_SW: u32 = 0x10F0_000B;
/// Effect slot mark: forking lightning arc shooting top-left -> bottom-right across the slot rect.
pub(crate) const MARK_ELECTRIC_ARC_SE: u32 = 0x10F0_000C;

/// Selects the directional arc slot mark encoding the vector from `a` to `b`.
pub(crate) const fn electric_arc_mark(a: [f32; 2], b: [f32; 2]) -> u32 {
    match (b[0] < a[0], b[1] < a[1]) {
        (true, true) => MARK_ELECTRIC_ARC_NW,
        (false, true) => MARK_ELECTRIC_ARC_NE,
        (true, false) => MARK_ELECTRIC_ARC_SW,
        (false, false) => MARK_ELECTRIC_ARC_SE,
    }
}

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

/// Validated, finite point-space parameters extracted from one `KIND_EFFECT` instance.
struct SlotParams {
    rect: [f32; 4],
    radius: f32,
    time: f32,
    intensity: f32,
    opacity: f32,
    seed: u32,
    is_light: bool,
}

impl SlotParams {
    fn from_instance(inst: &Instance) -> Option<Self> {
        let scale = inst.params[3];
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let rect = inst.rect.map(|v| v / scale);
        let [time, intensity, opacity, seed_f] = inst.uv;
        let all_finite = rect.iter().all(|v| v.is_finite())
            && inst.radii.iter().all(|v| v.is_finite())
            && [time, intensity, opacity, seed_f]
                .iter()
                .all(|v| v.is_finite());
        if !all_finite || opacity <= 0.0 || intensity <= 0.0 || rect[2] <= 0.0 || rect[3] <= 0.0 {
            return None;
        }
        let radius = inst.radii.iter().fold(0.0_f32, |a, v| a.max(v.abs())) / scale;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "effect seed is a small finite integer; sign encodes light-theme contrast"
        )]
        let seed = seed_f.abs().min(u32::MAX as f32) as u32;
        Some(Self {
            rect,
            radius,
            time,
            intensity,
            opacity,
            seed,
            is_light: seed_f < 0.0,
        })
    }
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

    /// Releases cached `BindSet` and backdrop `TextureView` references on the one-time
    /// transition from active effects back to idle while keeping WGSL pipelines warm.
    fn flush_on_idle<S: 'static>(&mut self, f: &Frame<'_, S>) {
        if !self.active {
            return;
        }
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        if gpu.device_key != f.device_key() {
            self.gpu = None;
            return;
        }
        let empty = FxFrame {
            viewport: [f.size.0, f.size.1],
            scale: 1.0,
            layers: &[],
            backdrop: None,
            sources: &[],
        };
        // `BindSet` (`now`/`last`) evicts entries after two empty frames, releasing transient
        // bind groups and cloned backdrop `TextureView` handles.
        gpu.renderer
            .prepare_with_slots(f.device, f.queue, &empty, &[]);
        gpu.renderer
            .prepare_with_slots(f.device, f.queue, &empty, &[]);
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
            self.flush_on_idle(f);
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
            .scale(inst.params[3])
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
    let p = SlotParams::from_instance(inst)?;
    let (fx, shape, seed_offset) = decode_plasma_electric_layer(mark, &p)?;
    Some(
        fx.on(shape)
            .at(p.time)
            .seed(p.seed.wrapping_add(seed_offset))
            .opacity(p.opacity),
    )
}

fn decode_plasma_electric_layer(mark: u32, p: &SlotParams) -> Option<(Fx, Shape, u32)> {
    match mark {
        MARK_PLASMA_BORDER => Some((
            Fx::fire(plasma_border_params(p.intensity, p.is_light)),
            card_shape(p.rect, p.radius),
            37,
        )),
        MARK_ELECTRIC_BORDER => Some((
            Fx::electric(electric_border_params(p.intensity, p.is_light)),
            card_shape(p.rect, p.radius),
            41,
        )),
        MARK_ELECTRIC_CHARGED => Some((
            Fx::electric(electric_charged_params(p.intensity, p.is_light)),
            inset_border_shape(p.rect, p.radius),
            43,
        )),
        MARK_ELECTRIC_ARC_NW | MARK_ELECTRIC_ARC_NE | MARK_ELECTRIC_ARC_SW
        | MARK_ELECTRIC_ARC_SE => {
            let (a, b) = arc_endpoints(mark, p.rect);
            Some((
                Fx::electric(electric_arc_params(a, b, p.intensity, p.is_light)),
                Shape::rounded(p.rect[0], p.rect[1], p.rect[2], p.rect[3], 0.0),
                47,
            ))
        }
        _ => None,
    }
}

fn arc_endpoints(mark: u32, [x, y, w, h]: [f32; 4]) -> ([f32; 2], [f32; 2]) {
    let x1 = x + w;
    let y1 = y + h;
    match mark {
        MARK_ELECTRIC_ARC_NW => ([x1, y1], [x, y]),
        MARK_ELECTRIC_ARC_NE => ([x, y1], [x1, y]),
        MARK_ELECTRIC_ARC_SW => ([x1, y], [x, y1]),
        _ => ([x, y], [x1, y1]),
    }
}

/// Slightly inset window rect so the outer charged perimeter band hugs inside the frame.
fn inset_border_shape([x, y, w, h]: [f32; 4], radius: f32) -> Shape {
    const INSET: f32 = 6.0;
    let inset = INSET.min(w * 0.25).min(h * 0.25);
    let iw = (w - 2.0 * inset).max(4.0);
    let ih = (h - 2.0 * inset).max(4.0);
    let ir = (radius - inset).clamp(2.0, iw.min(ih) * 0.5);
    Shape::rounded(x + inset, y + inset, iw, ih, ir)
}

/// Splash card shape guaranteed to keep a non-zero corner radius for border filament tapers.
fn card_shape([x, y, w, h]: [f32; 4], radius: f32) -> Shape {
    let max_r = (w.min(h) * 0.5).max(1.0);
    let safe_r = radius.clamp(6.0_f32.min(max_r), max_r);
    Shape::rounded(x, y, w, h, safe_r)
}

fn plasma_border_params(intensity: f32, is_light: bool) -> FireParams {
    let base = FireParams::plasma();
    FireParams {
        height: 44.0,
        scale: 18.0,
        speed: 1.45,
        intensity: (if is_light { 1.25 } else { 1.08 }) * intensity,
        warp: 1.1,
        sides: 0.62,
        base_glow: 0.72,
        occlusion: if is_light { 0.90 } else { 0.28 },
        swirl: 0.95,
        ..base
    }
}

fn electric_border_params(intensity: f32, is_light: bool) -> ElectricParams {
    ElectricParams {
        style: ElectricStyle::Border,
        color: rgb(0x3ea6ff),
        core: rgb(0xf2fbff),
        cool: rgb(0xb04cff),
        amplitude: 6.5,
        frequency: 2.6,
        speed: 3.8,
        core_width: 0.75,
        intensity: (if is_light { 1.25 } else { 1.12 }) * intensity,
        glow: 6.5,
        forks: 0.72,
        flicker: 0.55,
        strands: 2,
        gap: 3.5,
        fork_len: 28.0,
        occlusion: if is_light { 0.88 } else { 0.18 },
        zap: None,
    }
}

fn electric_arc_params(a: [f32; 2], b: [f32; 2], intensity: f32, is_light: bool) -> ElectricParams {
    ElectricParams {
        style: ElectricStyle::Arc { a, b },
        color: rgb(0x38b4ff),
        core: rgb(0xf5fcff),
        cool: rgb(0xc44dff),
        amplitude: 9.5,
        frequency: 2.5,
        speed: 4.4,
        core_width: 0.8,
        intensity: (if is_light { 1.3 } else { 1.15 }) * intensity,
        glow: 7.0,
        forks: 0.78,
        flicker: 0.6,
        strands: 3,
        gap: 0.0,
        fork_len: 32.0,
        occlusion: if is_light { 0.88 } else { 0.16 },
        zap: None,
    }
}

fn electric_charged_params(intensity: f32, is_light: bool) -> ElectricParams {
    let base = ElectricParams::charged();
    ElectricParams {
        color: rgb(0x479eff),
        core: rgb(0xedf7ff),
        cool: rgb(0x9b5cff),
        intensity: base.intensity * (if is_light { 1.25 } else { 1.0 }) * intensity,
        glow: base.glow * 1.15,
        occlusion: if is_light { 0.72 } else { 0.14 },
        ..base
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
