// SPDX-License-Identifier: MIT

use std::sync::{Arc, Mutex};

use rune::{
    App as RuneApp, AppSpec, Cx, El, Tx,
    color::hex,
    compose::{Compositor, Frame},
    effect, el,
    render::{Instance, KIND_EFFECT, NO_CLIP},
};
use rune_fx::{DistortParams, ElectricStyle, FireStyle, Fx};

use super::*;

fn effect_instance(mark: u32, rect: [f32; 4], radius: f32, scale: f32, uv: [f32; 4]) -> Instance {
    Instance {
        rect,
        radii: [radius; 4],
        fill: [0.0, 0.0, 0.0, 1.0],
        border: [
            (mark & 0xFFFF) as f32,
            ((mark >> 16) & 0xFFFF) as f32,
            0.0,
            0.0,
        ],
        params: [0.0, KIND_EFFECT, 0.0, scale],
        uv,
        clip: NO_CLIP,
        fade: [0; 4],
    }
}

#[test]
fn compositor_starts_idle_with_zero_overhead() {
    let compositor = FxCompositor::new();
    assert!(!<FxCompositor as Compositor<()>>::draws_own(&compositor));
    assert!(!<FxCompositor as Compositor<()>>::animating(&compositor));
}

#[test]
fn collect_slots_ignores_non_effect_and_unknown_marks() {
    let regular = Instance {
        rect: [0.0, 0.0, 100.0, 100.0],
        radii: [0.0; 4],
        fill: [1.0; 4],
        border: [0.0; 4],
        params: [0.0, 0.0, 0.0, 1.0],
        uv: [0.5, 1.0, 1.0, 1.0],
        clip: NO_CLIP,
        fade: [0; 4],
    };
    let unknown = effect_instance(
        0xDEAD_BEEF,
        [0.0, 0.0, 100.0, 100.0],
        8.0,
        1.0,
        [0.5, 1.0, 1.0, 1.0],
    );
    assert!(collect_slots(&[regular, unknown], &[0, 0xDEAD_BEEF]).is_empty());
}

#[test]
fn decode_layer_decodes_all_fire_marks_and_scales_device_pixels_to_points() {
    let data = [0.45, 1.0, 0.9, 7.0];

    let bottom = decode_layer(
        MARK_FIRE_BOTTOM,
        &effect_instance(MARK_FIRE_BOTTOM, [0.0, 0.0, 1280.0, 800.0], 0.0, 1.0, data),
    )
    .expect("bottom blaze decodes");
    assert!(bottom.is_animating());
    assert!(
        bottom.shape.radius > 0.0,
        "fluid sim requires non-zero corner radius"
    );
    assert!(matches!(
        bottom.fx,
        Fx::Fire(p) if p.style == FireStyle::Top && p.embers > 0 && p.smoke > 0.0
    ));

    let border = decode_layer(
        MARK_FIRE_BORDER,
        &effect_instance(MARK_FIRE_BORDER, [0.0, 0.0, 1280.0, 800.0], 10.0, 1.0, data),
    )
    .expect("perimeter fire border decodes");
    assert!(border.is_animating());
    assert!(matches!(border.fx, Fx::Fire(p) if p.style == FireStyle::Border));

    let haze = decode_layer(
        MARK_FIRE_HAZE,
        &effect_instance(MARK_FIRE_HAZE, [0.0, 0.0, 1280.0, 800.0], 0.0, 1.0, data),
    )
    .expect("window heat haze decodes");
    assert!(haze.is_animating());
    assert!(matches!(
        haze.fx,
        Fx::Distort(DistortParams::HeatHaze { amount, whole_shape: false, .. }) if amount > 0.0
    ));

    let badge_flame = decode_layer(
        MARK_FIRE_BADGE_FLAME,
        &effect_instance(
            MARK_FIRE_BADGE_FLAME,
            [560.0, 360.0, 140.0, 52.0],
            0.0,
            1.0,
            data,
        ),
    )
    .expect("badge heat flame decodes");
    assert!(badge_flame.is_animating());
    assert!(
        badge_flame.shape.radius > 0.0,
        "zero roundness still gets safe sim radius"
    );
    assert!(matches!(badge_flame.fx, Fx::Fire(p) if p.style == FireStyle::Heat));

    let badge_haze = decode_layer(
        MARK_FIRE_BADGE_HAZE,
        &effect_instance(
            MARK_FIRE_BADGE_HAZE,
            [560.0, 360.0, 140.0, 52.0],
            12.0,
            1.0,
            data,
        ),
    )
    .expect("badge heat haze decodes");
    assert!(badge_haze.is_animating());
    assert!(matches!(
        badge_haze.fx,
        Fx::Distort(DistortParams::HeatHaze { amount, whole_shape: false, .. }) if amount > 0.0
    ));

    // HiDPI scale = 2.0 converts device-pixel rect & radii back to identical point-space shape.
    let hidpi_inst = effect_instance(
        MARK_FIRE_BADGE_HAZE,
        [1120.0, 720.0, 280.0, 104.0],
        24.0,
        2.0,
        data,
    );
    let hidpi_slots = collect_slots(&[hidpi_inst], &[MARK_FIRE_BADGE_HAZE]);
    assert_eq!(hidpi_slots.len(), 1);
    assert_eq!(
        hidpi_slots[0].layer.shape, badge_haze.shape,
        "scale = 2.0 device pixels must decode to the same point-space shape"
    );
    assert_eq!(
        hidpi_slots[0].placement,
        FxSlot::default().scale(2.0).clip(NO_CLIP, [0; 4])
    );

    // Light-theme slots (negative signed seed) boost fire occlusion for crisp contrast on bright UI.
    let light_bottom = decode_layer(
        MARK_FIRE_BOTTOM,
        &effect_instance(
            MARK_FIRE_BOTTOM,
            [0.0, 0.0, 1280.0, 800.0],
            0.0,
            1.0,
            [0.45, 1.0, 0.9, -7.0],
        ),
    )
    .expect("light-theme bottom blaze decodes");
    assert_eq!(
        light_bottom.seed, bottom.seed,
        "seed magnitude is preserved across themes"
    );
    let (Fx::Fire(dark_p), Fx::Fire(light_p)) = (bottom.fx, light_bottom.fx) else {
        panic!("expected Fx::Fire layers");
    };
    assert!(
        light_p.occlusion > dark_p.occlusion,
        "light-theme fire raises occlusion for contrast"
    );
}

#[test]
fn decode_layer_rejects_non_finite_zero_size_and_non_positive_inputs() {
    let valid_rect = [0.0, 0.0, 640.0, 480.0];
    let valid_uv = [0.5, 1.0, 1.0, 3.0];
    let invalid_cases = [
        ("zero opacity", valid_rect, 8.0, 1.0, [0.5, 1.0, 0.0, 1.0]),
        ("zero intensity", valid_rect, 8.0, 1.0, [0.5, 0.0, 1.0, 1.0]),
        (
            "NaN opacity",
            valid_rect,
            8.0,
            1.0,
            [0.5, 1.0, f32::NAN, 1.0],
        ),
        (
            "NaN intensity",
            valid_rect,
            8.0,
            1.0,
            [0.5, f32::NAN, 1.0, 1.0],
        ),
        ("NaN time", valid_rect, 8.0, 1.0, [f32::NAN, 1.0, 1.0, 1.0]),
        ("NaN seed", valid_rect, 8.0, 1.0, [0.5, 1.0, 1.0, f32::NAN]),
        (
            "inf opacity",
            valid_rect,
            8.0,
            1.0,
            [0.5, 1.0, f32::INFINITY, 1.0],
        ),
        (
            "neg-inf time",
            valid_rect,
            8.0,
            1.0,
            [f32::NEG_INFINITY, 1.0, 1.0, 1.0],
        ),
        ("NaN radius", valid_rect, f32::NAN, 1.0, valid_uv),
        ("inf radius", valid_rect, f32::INFINITY, 1.0, valid_uv),
        ("zero width", [0.0, 0.0, 0.0, 480.0], 8.0, 1.0, valid_uv),
        ("zero height", [0.0, 0.0, 640.0, 0.0], 8.0, 1.0, valid_uv),
        (
            "NaN rect",
            [f32::NAN, 0.0, 640.0, 480.0],
            8.0,
            1.0,
            valid_uv,
        ),
        (
            "inf rect",
            [0.0, 0.0, f32::INFINITY, 480.0],
            8.0,
            1.0,
            valid_uv,
        ),
        ("zero scale", valid_rect, 8.0, 0.0, valid_uv),
        ("negative scale", valid_rect, 8.0, -2.0, valid_uv),
        ("NaN scale", valid_rect, 8.0, f32::NAN, valid_uv),
        ("inf scale", valid_rect, 8.0, f32::INFINITY, valid_uv),
    ];
    for mark in [
        MARK_FIRE_BOTTOM,
        MARK_PLASMA_BORDER,
        MARK_ELECTRIC_BORDER,
        MARK_ELECTRIC_CHARGED,
        MARK_ELECTRIC_ARC_NW,
    ] {
        for (label, rect, radius, scale, uv) in invalid_cases {
            let inst = effect_instance(mark, rect, radius, scale, uv);
            assert!(
                decode_layer(mark, &inst).is_none(),
                "decode_layer must reject {label} for mark {mark:#x}"
            );
        }
    }
}

#[test]
fn decode_layer_decodes_plasma_and_electric_marks_with_scale_and_theme_contrast() {
    let data = [0.55, 1.0, 0.9, 11.0];

    let plasma = decode_layer(
        MARK_PLASMA_BORDER,
        &effect_instance(
            MARK_PLASMA_BORDER,
            [390.0, 150.0, 500.0, 500.0],
            0.0,
            1.0,
            data,
        ),
    )
    .expect("plasma border decodes");
    assert!(plasma.is_animating());
    assert!(plasma.shape.radius > 0.0);
    assert!(matches!(
        plasma.fx,
        Fx::Fire(p) if p.style == FireStyle::Border && p.swirl > 0.0
    ));

    let border = decode_layer(
        MARK_ELECTRIC_BORDER,
        &effect_instance(
            MARK_ELECTRIC_BORDER,
            [390.0, 150.0, 500.0, 500.0],
            14.0,
            1.0,
            data,
        ),
    )
    .expect("electric overload border decodes");
    assert!(border.is_animating());
    assert!(matches!(
        border.fx,
        Fx::Electric(p) if p.style == ElectricStyle::Border && p.forks > 0.0
    ));

    let charged = decode_layer(
        MARK_ELECTRIC_CHARGED,
        &effect_instance(
            MARK_ELECTRIC_CHARGED,
            [0.0, 0.0, 1280.0, 800.0],
            10.0,
            1.0,
            data,
        ),
    )
    .expect("charged window perimeter decodes");
    assert!(charged.is_animating());
    assert!(matches!(
        charged.fx,
        Fx::Electric(p) if p.style == ElectricStyle::Charged
    ));

    let arc_cases = [
        ([400.0, 160.0], [20.0, 20.0], MARK_ELECTRIC_ARC_NW),
        ([880.0, 160.0], [1260.0, 20.0], MARK_ELECTRIC_ARC_NE),
        ([400.0, 640.0], [20.0, 780.0], MARK_ELECTRIC_ARC_SW),
        ([880.0, 640.0], [1260.0, 780.0], MARK_ELECTRIC_ARC_SE),
    ];
    for (from, to, expected_mark) in arc_cases {
        assert_eq!(electric_arc_mark(from, to), expected_mark);
        let rect = [
            from[0].min(to[0]),
            from[1].min(to[1]),
            (to[0] - from[0]).abs(),
            (to[1] - from[1]).abs(),
        ];
        let layer = decode_layer(
            expected_mark,
            &effect_instance(expected_mark, rect, 0.0, 1.0, data),
        )
        .expect("arc decodes");
        assert!(layer.is_animating());
        assert!(matches!(
            layer.fx,
            Fx::Electric(p) if p.style == ElectricStyle::Arc { a: from, b: to }
        ));

        // HiDPI scale = 2.0 round-trips device pixels back to identical point-space endpoints.
        let scaled_rect = rect.map(|v| v * 2.0);
        let scaled = collect_slots(
            &[effect_instance(expected_mark, scaled_rect, 0.0, 2.0, data)],
            &[expected_mark],
        );
        assert_eq!(scaled.len(), 1);
        assert!(matches!(
            scaled[0].layer.fx,
            Fx::Electric(p) if p.style == ElectricStyle::Arc { a: from, b: to }
        ));
        assert_eq!(
            scaled[0].placement,
            FxSlot::default().scale(2.0).clip(NO_CLIP, [0; 4])
        );
    }

    // Light-theme slots (negative signed seed) raise occlusion for contrast while keeping seed.
    let light_data = [0.55, 1.0, 0.9, -11.0];
    let light_plasma = decode_layer(
        MARK_PLASMA_BORDER,
        &effect_instance(
            MARK_PLASMA_BORDER,
            [390.0, 150.0, 500.0, 500.0],
            14.0,
            1.0,
            light_data,
        ),
    )
    .expect("light plasma decodes");
    assert_eq!(light_plasma.seed, plasma.seed);
    let (Fx::Fire(dark_p), Fx::Fire(light_p)) = (plasma.fx, light_plasma.fx) else {
        panic!("expected Fx::Fire plasma layers");
    };
    assert!(light_p.occlusion > dark_p.occlusion);

    let light_border = decode_layer(
        MARK_ELECTRIC_BORDER,
        &effect_instance(
            MARK_ELECTRIC_BORDER,
            [390.0, 150.0, 500.0, 500.0],
            14.0,
            1.0,
            light_data,
        ),
    )
    .expect("light electric border decodes");
    assert_eq!(light_border.seed, border.seed);
    let (Fx::Electric(dark_e), Fx::Electric(light_e)) = (border.fx, light_border.fx) else {
        panic!("expected Fx::Electric layers");
    };
    assert!(light_e.occlusion > dark_e.occlusion);
}

/// Wrapper that records `(draws_own, animating)` after every frame `FxCompositor` renders.
struct SpyCompositor {
    inner: FxCompositor,
    history: Arc<Mutex<Vec<(bool, bool)>>>,
}

impl<S: 'static> Compositor<S> for SpyCompositor {
    fn render(&mut self, f: &mut Frame<'_, S>) {
        self.inner.render(f);
        let draws = <FxCompositor as Compositor<S>>::draws_own(&self.inner);
        let anim = <FxCompositor as Compositor<S>>::animating(&self.inner);
        self.history
            .lock()
            .expect("history lock")
            .push((draws, anim));
    }

    fn animating(&self) -> bool {
        <FxCompositor as Compositor<S>>::animating(&self.inner)
    }

    fn draws_own(&self) -> bool {
        <FxCompositor as Compositor<S>>::draws_own(&self.inner)
    }
}

struct ToggleState {
    fire_active: bool,
    polls: u32,
}

fn toggle_poll(state: &mut ToggleState, _clock: f64, _tx: &mut Tx) -> bool {
    state.polls += 1;
    // Frame 0 renders with fire active; switch back to idle before frame 1 so the compositor
    // exercises its one-time active -> idle flush and steady idle path.
    if state.polls >= 2 && state.fire_active {
        state.fire_active = false;
        return true;
    }
    false
}

fn toggle_scene(state: &ToggleState, _: &mut Cx<ToggleState>) -> El {
    let mut root = el()
        .size(240.0, 180.0)
        .fill(hex("#14141a").expect("valid hex color"));
    if state.fire_active {
        let data = [0.6, 1.0, 1.0, 3.0];
        root = root
            .child(
                effect(MARK_FIRE_HAZE)
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .effect_backdrop(true)
                    .effect_data(data),
            )
            .child(
                effect(MARK_FIRE_BORDER)
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .radius(10.0)
                    .effect_data(data),
            )
            .child(
                effect(MARK_FIRE_BOTTOM)
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .effect_data(data),
            )
            .child(
                effect(MARK_FIRE_BADGE_FLAME)
                    .absolute(70.0, 72.0)
                    .size(100.0, 36.0)
                    .radius(10.0)
                    .effect_data(data),
            )
            .child(
                effect(MARK_FIRE_BADGE_HAZE)
                    .absolute(70.0, 72.0)
                    .size(100.0, 36.0)
                    .radius(10.0)
                    .effect_backdrop(true)
                    .effect_data(data),
            );
    }
    root
}

fn idle_scene(_: &(), _: &mut Cx<()>) -> El {
    el().size(160.0, 120.0)
        .fill(hex("#1a1a1f").expect("valid hex color"))
}

#[test]
fn headless_gpu_compositor_renders_fire_and_flushes_cleanly_to_idle() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Pure idle frame: no KIND_EFFECT slots -> draws_own == false, animating == false.
    let idle_history = Arc::new(Mutex::new(Vec::new()));
    let idle_path = dir.path().join("idle.png");
    RuneApp::new(AppSpec::new("fx-idle", (160.0, 120.0), || (), idle_scene))
        .compositor(SpyCompositor {
            inner: FxCompositor::new(),
            history: Arc::clone(&idle_history),
        })
        .args([
            "--snap".to_owned(),
            idle_path.to_string_lossy().into_owned(),
            "--scale".to_owned(),
            "1".to_owned(),
        ])
        .run();
    assert_eq!(
        idle_history.lock().expect("idle history").as_slice(),
        &[(false, false)],
        "idle frame must neither draw own nor animate"
    );

    // Active -> idle transition: frame 0 renders all 5 fire slots (fluid sim + backdrop haze),
    // then frame 1 flushes back to idle and frame 2 stays idle.
    let transition_history = Arc::new(Mutex::new(Vec::new()));
    let out_dir = dir.path().join("frames");
    RuneApp::new(AppSpec {
        poll: Some(toggle_poll),
        ..AppSpec::new(
            "fx-transition",
            (240.0, 180.0),
            || ToggleState {
                fire_active: true,
                polls: 0,
            },
            toggle_scene,
        )
    })
    .compositor(SpyCompositor {
        inner: FxCompositor::new(),
        history: Arc::clone(&transition_history),
    })
    .args([
        "--frames".to_owned(),
        "3".to_owned(),
        "--out".to_owned(),
        out_dir.to_string_lossy().into_owned(),
        "--scale".to_owned(),
        "1".to_owned(),
    ])
    .run();

    let history = transition_history
        .lock()
        .expect("transition history")
        .clone();
    assert_eq!(
        history.first().copied(),
        Some((true, true)),
        "active fire frame must report draws_own and animating: {history:?}"
    );
    assert_eq!(
        history.last().copied(),
        Some((false, false)),
        "after active -> idle transition, compositor must flush back to draws_own=false, animating=false: {history:?}"
    );
    assert!(
        history.len() >= 3 && history[1..].iter().all(|&s| s == (false, false)),
        "every frame after fire unmounts must remain idle: {history:?}"
    );
}

fn plasma_toggle_scene(state: &ToggleState, _: &mut Cx<ToggleState>) -> El {
    let mut root = el()
        .size(240.0, 180.0)
        .fill(hex("#14141a").expect("valid hex color"));
    if state.fire_active {
        let data = [0.45, 1.0, 1.0, 5.0];
        root = root
            .child(
                effect(MARK_ELECTRIC_CHARGED)
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .radius(10.0)
                    .effect_bleed(24.0)
                    .effect_data(data),
            )
            .child(
                effect(MARK_ELECTRIC_ARC_NW)
                    .absolute(12.0, 12.0)
                    .size(58.0, 28.0)
                    .effect_bleed(96.0)
                    .effect_data(data),
            )
            .child(
                effect(MARK_ELECTRIC_ARC_SE)
                    .absolute(170.0, 140.0)
                    .size(58.0, 28.0)
                    .effect_bleed(96.0)
                    .effect_data(data),
            )
            .child(
                effect(MARK_PLASMA_BORDER)
                    .absolute(70.0, 40.0)
                    .size(100.0, 100.0)
                    .radius(12.0)
                    .effect_bleed(64.0)
                    .effect_data(data),
            )
            .child(
                effect(MARK_ELECTRIC_BORDER)
                    .absolute(70.0, 40.0)
                    .size(100.0, 100.0)
                    .radius(12.0)
                    .effect_bleed(64.0)
                    .effect_data(data),
            );
    }
    root
}

#[test]
fn headless_gpu_compositor_renders_plasma_and_electricity_and_flushes_cleanly_to_idle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let transition_history = Arc::new(Mutex::new(Vec::new()));
    let out_dir = dir.path().join("plasma-frames");
    RuneApp::new(AppSpec {
        poll: Some(toggle_poll),
        ..AppSpec::new(
            "fx-plasma-transition",
            (240.0, 180.0),
            || ToggleState {
                fire_active: true,
                polls: 0,
            },
            plasma_toggle_scene,
        )
    })
    .compositor(SpyCompositor {
        inner: FxCompositor::new(),
        history: Arc::clone(&transition_history),
    })
    .args([
        "--frames".to_owned(),
        "3".to_owned(),
        "--out".to_owned(),
        out_dir.to_string_lossy().into_owned(),
        "--scale".to_owned(),
        "1".to_owned(),
    ])
    .run();

    let history = transition_history
        .lock()
        .expect("plasma transition history")
        .clone();
    assert_eq!(
        history.first().copied(),
        Some((true, true)),
        "active plasma + electricity frame must report draws_own and animating: {history:?}"
    );
    assert!(
        history.len() >= 3 && history[1..].iter().all(|&s| s == (false, false)),
        "every frame after plasma splash unmounts must remain idle: {history:?}"
    );
}
