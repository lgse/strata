// SPDX-License-Identifier: MIT

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use rune::{
    App as RuneApp, AppSpec, Cx, El,
    color::hex,
    compose::{Compositor, Frame},
    effect, el,
    render::{Instance, KIND_EFFECT, NO_CLIP},
};
use rune_fx::{DistortParams, FireStyle, Fx};

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
fn decode_layer_decodes_all_fire_marks_and_culls_zero_opacity_or_intensity() {
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

    // Zero opacity or zero intensity culls cleanly before touching FxRenderer.
    assert!(
        decode_layer(
            MARK_FIRE_BOTTOM,
            &effect_instance(
                MARK_FIRE_BOTTOM,
                [0.0, 0.0, 640.0, 480.0],
                8.0,
                1.0,
                [0.5, 1.0, 0.0, 1.0]
            ),
        )
        .is_none()
    );
    assert!(
        decode_layer(
            MARK_FIRE_HAZE,
            &effect_instance(
                MARK_FIRE_HAZE,
                [0.0, 0.0, 640.0, 480.0],
                8.0,
                1.0,
                [0.5, 0.0, 1.0, 1.0]
            ),
        )
        .is_none()
    );
}

/// Wrapper that observes `FxCompositor`'s `draws_own()` and `animating()` state right after render.
struct SpyCompositor {
    inner: FxCompositor,
    saw_draws_own: Arc<AtomicBool>,
    saw_animating: Arc<AtomicBool>,
}

impl Compositor<()> for SpyCompositor {
    fn render(&mut self, f: &mut Frame<'_, ()>) {
        self.inner.render(f);
        self.saw_draws_own.store(
            <FxCompositor as Compositor<()>>::draws_own(&self.inner),
            Ordering::SeqCst,
        );
        self.saw_animating.store(
            <FxCompositor as Compositor<()>>::animating(&self.inner),
            Ordering::SeqCst,
        );
    }

    fn animating(&self) -> bool {
        <FxCompositor as Compositor<()>>::animating(&self.inner)
    }

    fn draws_own(&self) -> bool {
        <FxCompositor as Compositor<()>>::draws_own(&self.inner)
    }
}

fn idle_scene(_: &(), _: &mut Cx<()>) -> El {
    el().size(160.0, 120.0)
        .fill(hex("#1a1a1f").expect("valid hex color"))
}

fn full_fire_scene(_: &(), _: &mut Cx<()>) -> El {
    let data = [0.6, 1.0, 1.0, 3.0];
    el().size(240.0, 180.0)
        .fill(hex("#14141a").expect("valid hex color"))
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
        )
}

#[test]
fn headless_gpu_compositor_renders_fire_and_stays_idle_without_slots() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Idle frame: no KIND_EFFECT slots -> draws_own == false, animating == false.
    let idle_draws = Arc::new(AtomicBool::new(true));
    let idle_anim = Arc::new(AtomicBool::new(true));
    let idle_path = dir.path().join("idle.png");
    RuneApp::new(AppSpec::new("fx-idle", (160.0, 120.0), || (), idle_scene))
        .compositor(SpyCompositor {
            inner: FxCompositor::new(),
            saw_draws_own: Arc::clone(&idle_draws),
            saw_animating: Arc::clone(&idle_anim),
        })
        .args([
            "--snap".to_owned(),
            idle_path.to_string_lossy().into_owned(),
            "--scale".to_owned(),
            "1".to_owned(),
        ])
        .run();
    assert!(
        !idle_draws.load(Ordering::SeqCst),
        "idle frame must not draw own"
    );
    assert!(
        !idle_anim.load(Ordering::SeqCst),
        "idle frame must not animate"
    );

    // Active fire frame: fluid sim + heat haze + border + badge heat render without error.
    let fire_draws = Arc::new(AtomicBool::new(false));
    let fire_anim = Arc::new(AtomicBool::new(false));
    let fire_path = dir.path().join("fire.png");
    RuneApp::new(AppSpec::new(
        "fx-fire",
        (240.0, 180.0),
        || (),
        full_fire_scene,
    ))
    .compositor(SpyCompositor {
        inner: FxCompositor::new(),
        saw_draws_own: Arc::clone(&fire_draws),
        saw_animating: Arc::clone(&fire_anim),
    })
    .args([
        "--snap".to_owned(),
        fire_path.to_string_lossy().into_owned(),
        "--scale".to_owned(),
        "1".to_owned(),
    ])
    .run();
    assert!(
        fire_draws.load(Ordering::SeqCst),
        "fire frame must report draws_own"
    );
    assert!(
        fire_anim.load(Ordering::SeqCst),
        "fire frame must report animating"
    );
    assert!(fire_path.exists(), "headless fire snap wrote output");
}
