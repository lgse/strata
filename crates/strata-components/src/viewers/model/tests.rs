use std::f32::consts::{FRAC_PI_2, TAU};

use super::{CameraView, Encoding, Mesh, ModelFile, ModelKind, ModelState, Stage};

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn stl() -> ModelFile {
    ModelFile {
        name: "bracket.stl".into(),
        extension: "stl".into(),
        position: None,
        kind: ModelKind::Mesh,
    }
}

fn mesh(view: CameraView) -> Mesh {
    Mesh {
        view,
        encoding: Encoding::BinaryStl,
        triangles: 12_480,
    }
}

/// A mesh loaded at Front left.
fn loaded() -> ModelState {
    let mut st = ModelState::default();
    st.open(stl());
    st.image([512.0, 512.0], Some(mesh(CameraView::FrontLeft)));
    st
}

/// The render for the view a step asked for lands.
fn render(st: &mut ModelState) {
    let view = st.camera();
    st.image([512.0, 512.0], Some(mesh(view)));
}

#[test]
fn next_turns_clockwise_and_previous_the_other_way() {
    use CameraView::*;
    let mut v = FrontLeft;
    let mut seen = vec![];
    for _ in 0..4 {
        v = v.step(1);
        seen.push(v);
    }
    assert_eq!(seen, [BackLeft, BackRight, FrontRight, FrontLeft]);
    assert_eq!(FrontLeft.step(-1), FrontRight);
}

#[test]
fn the_indicator_puts_front_left_lower_left_and_back_right_upper_right() {
    // Screen space, y down: a positive sine is below the centre.
    let at = |v: CameraView| (v.angle().cos(), v.angle().sin());
    let (x, y) = at(CameraView::FrontLeft);
    assert!(x < 0.0 && y > 0.0);
    let (x, y) = at(CameraView::BackRight);
    assert!(x > 0.0 && y < 0.0);
    let (x, y) = at(CameraView::FrontRight);
    assert!(x > 0.0 && y > 0.0);
    let (x, y) = at(CameraView::BackLeft);
    assert!(x < 0.0 && y < 0.0);
}

#[test]
fn each_step_takes_the_shorter_turn_and_a_lap_keeps_going_round() {
    let mut st = loaded();
    let start = st.angle();
    st.step(-1).expect("steps");
    // Front left to Front right is a quarter turn, not three quarters the other way.
    assert!(
        near(st.angle() - start, -FRAC_PI_2),
        "{}",
        st.angle() - start
    );
    render(&mut st);
    let start = st.angle();
    for _ in 0..4 {
        st.step(1).expect("steps");
        render(&mut st);
    }
    // Four Nexts are a whole lap the same way: the unwrapped angle grows by a turn.
    assert!(near(st.angle() - start, TAU), "{}", st.angle() - start);
}

#[test]
fn a_press_before_the_first_picture_or_during_a_render_does_nothing() {
    let mut st = ModelState::default();
    st.open(stl());
    assert_eq!(st.step(1), None);
    assert_eq!(st.camera(), CameraView::FrontLeft);
    st.image([512.0, 512.0], Some(mesh(CameraView::FrontLeft)));
    assert_eq!(st.step(1), Some(CameraView::BackLeft));
    assert!(st.rendering());
    assert_eq!(st.step(1), None, "ignored while BackLeft renders");
    assert_eq!(st.camera(), CameraView::BackLeft);
}

#[test]
fn a_failed_render_keeps_the_picture_returns_the_camera_and_retries() {
    let mut st = loaded();
    st.step(1);
    st.progress(Stage::Rendering { triangles: 12_480 });
    st.view_failed("The sandboxed preview renderer failed");
    assert!(st.has_image() && !st.rendering());
    assert_eq!(st.camera(), CameraView::FrontLeft, "back to the view shown");
    assert_eq!(st.error(), Some("The sandboxed preview renderer failed"));
    // The button tries the same view again.
    assert_eq!(st.step(1), Some(CameraView::BackLeft));
    assert_eq!(st.error(), None);
}

#[test]
fn a_new_file_starts_over_at_front_left_and_loading() {
    let mut st = loaded();
    st.step(1);
    render(&mut st);
    st.step(1);
    assert_eq!(st.camera(), CameraView::BackRight);
    st.open(ModelFile {
        name: "gear.3mf".into(),
        extension: "3mf".into(),
        position: None,
        kind: ModelKind::Mesh,
    });
    assert_eq!(st.camera(), CameraView::FrontLeft);
    assert!(near(st.angle(), CameraView::FrontLeft.angle()));
    assert_eq!(st.busy(), Some("Loading preview…"));
    assert_eq!(st.format_badge(), "3MF");
    assert_eq!(st.triangle_badge(), None);
}

#[test]
fn the_badges_follow_the_load() {
    let mut st = ModelState::default();
    st.open(stl());
    assert_eq!(st.format_badge(), "STL");
    st.progress(Stage::Rendering { triangles: 1 });
    assert_eq!(st.triangle_badge().as_deref(), Some("1 triangle"));
    assert_eq!(st.busy(), Some("Rendering 1 triangle…"));
    st.image([512.0, 512.0], Some(mesh(CameraView::FrontLeft)));
    assert_eq!(st.format_badge(), "Binary STL");
    assert_eq!(st.triangle_badge().as_deref(), Some("12,480 triangles"));
    assert_eq!(st.busy(), None);
}

#[test]
fn an_embedded_picture_shows_alone_once_loaded() {
    let mut st = ModelState::default();
    st.open(ModelFile {
        name: "benchy.bgcode".into(),
        extension: "bgcode".into(),
        position: None,
        kind: ModelKind::Embedded,
    });
    assert!(!st.plain());
    assert_eq!(st.format_badge(), "BGCODE");
    st.progress(Stage::Thumbnail);
    assert_eq!(st.busy(), Some("Reading thumbnail…"));
    st.image([300.0, 300.0], None);
    assert!(st.plain());
    assert!(st.busy().is_none());
}

// The element, in a runtime.

mod element {
    use std::cell::Cell;

    use rune::{Cx, DragItem, El, Env, Pixels, Runtime};
    use strata_kit::look::Look;

    use super::{CameraView, loaded, mesh};
    use crate::contract::{Dnd, DragSource};
    use crate::viewers::model::{ModelState, ModelView};

    thread_local! {
        static DRAG_OUTS: Cell<u32> = const { Cell::new(0) };
    }

    struct App {
        model: ModelState,
        pixels: Pixels,
        look: Look,
    }

    fn view(s: &App, cx: &mut Cx<App>) -> El {
        // A binder that counts the drags it would start.
        let dnd = Dnd::custom(
            |el, _| el,
            |el: El, _source| {
                el.drag_out(|_: &App| {
                    DRAG_OUTS.with(|d| d.set(d.get() + 1));
                    Some(DragItem::new().text("bracket.stl"))
                })
            },
        );
        ModelView::new(
            &s.model,
            |s: &mut App| &mut s.model,
            |s: &mut App| &mut s.model.viewport,
        )
        .pixels(&s.pixels)
        .drag_source(dnd, DragSource::new(|_: &App| None))
        .build(&s.look, cx)
    }

    /// `preview/preview-panel/model-views`: "The model picture keeps the grab cursor and file
    /// drag." No orbit by drag: a drag on the picture drags the file, and the camera stays.
    #[test]
    fn a_drag_on_the_picture_drags_the_file_and_never_turns_the_camera() {
        let pixels = Pixels::new();
        pixels.set(8, 8, vec![200; 8 * 8 * 4]);
        let app = App {
            model: loaded(),
            pixels,
            look: Look::fallback(false),
        };
        let mut rt = Runtime::new(app, view, (420.0, 640.0), Env::new(false, 1.0));
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let r = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("model-picture"))
            .expect("the picture")
            .rect;
        let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        for i in 1..=6 {
            rt.pointer_move(x + 15.0 * i as f32, y);
            rt.frame(1.0 / 60.0);
        }
        rt.pointer_up(x + 90.0, y);
        rt.frame(1.0 / 60.0);
        assert!(DRAG_OUTS.with(Cell::get) > 0, "the file drag started");
        assert_eq!(rt.state.model.camera(), CameraView::FrontLeft);
        assert_eq!(rt.state.model.mesh(), Some(mesh(CameraView::FrontLeft)));
    }

    /// C13 review 1 (the C14b ruling): a plain picture fits the pane's width when the height
    /// would make it wider. At Rune 5d421fd `aspect_fit` with `max_w` took the cap for the
    /// cross room (C13's Rune draft U-C13-4), so a wide picture in a narrow, tall pane spilled
    /// out of it.
    #[test]
    fn a_plain_picture_fits_a_narrow_pane() {
        let mut model = ModelState::default();
        model.open(crate::viewers::model::ModelFile {
            name: "wide.3mf".into(),
            extension: "3mf".into(),
            position: None,
            kind: crate::viewers::model::ModelKind::Embedded,
        });
        model.image([600.0, 300.0], None);
        assert!(model.plain());
        let pixels = Pixels::new();
        pixels.set(8, 4, vec![200; 8 * 4 * 4]);
        let app = App {
            model,
            pixels,
            look: Look::fallback(false),
        };
        let mut rt = Runtime::new(app, view, (300.0, 600.0), Env::new(false, 1.0));
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let r = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("model-picture"))
            .expect("the picture")
            .rect;
        assert!(
            r[0] >= 0.0 && r[0] + r[2] <= 300.0,
            "the picture spills: {r:?}"
        );
    }
}
