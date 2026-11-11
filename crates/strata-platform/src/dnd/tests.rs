//! The guard of a drag source against Rune's capture, headless at the pinned Rune rev: the
//! workaround for U10 (draft, not filed) relies on Rune ending a held drag on Escape, and on
//! `Tx::send_key` delivering one. If a Rune move changes either, these fail. The
//! compositor's proof that it took the pointer, which a session gets from the drag's own
//! `wl_data_device.enter`, is delivered here by [`holding`].

use rune::{Cx, El, Env, KeyEvent, Phase, Runtime, Tx, col, el};

use super::hub::tests::FAKE_START;
use super::session::Release;
use super::*;

struct S {
    dnd: Dnd<S>,
    phases: Vec<(Phase, bool)>,
    started: u32,
    keys: Vec<String>,
    /// Phases an app's own `on_drag` heard, which starts its drag with `start_drag` and
    /// routes nothing through `owns`.
    own: Vec<(Phase, bool)>,
}

fn view(s: &S, _: &mut Cx<S>) -> El {
    let source = DragSource::new(|_: &S, _| Some(DragItem::new().text("hello")))
        .on_start(|s: &mut S, _| s.started += 1)
        .on_drag(|s: &mut S, d: &mut rune::Drag, _| s.phases.push((d.phase, d.cancelled())));
    let own =
        el().key("own")
            .size(100.0, 60.0)
            .on_drag(|s: &mut S, d: &mut rune::Drag, tx: &mut Tx| {
                s.own.push((d.phase, d.cancelled()));
                if d.phase == Phase::Move && d.dx > 8.0 && s.started == 0 {
                    s.started += 1;
                    let dnd = s.dnd.clone();
                    dnd.start_drag(
                        tx,
                        d,
                        "own",
                        DragItem::new().text("own"),
                        |_: &mut S, _, _| {},
                    )
                    .expect("the fake start");
                }
            });
    col()
        .w_full()
        .h_full()
        .child(s.dnd.drag_source(el().size(100.0, 60.0), "tile", source))
        .child(own)
}

/// The compositor's proof that it took the pointer arrives, in an update of its own.
fn holding(rt: &mut Runtime<S>) {
    rt.update_if(|s, tx| {
        s.dnd.clone().holding_for_test(tx);
        false
    });
}

fn heard_key(s: &mut S, ev: &KeyEvent, _: &mut Tx) -> bool {
    s.keys.push(format!("{:?}", ev.key));
    false
}

fn runtime(release: Release) -> (Runtime<S>, (f32, f32)) {
    FAKE_START.set(Some(release));
    let state = S {
        dnd: Dnd::new(),
        phases: Vec::new(),
        started: 0,
        keys: Vec::new(),
        own: Vec::new(),
    };
    let mut rt = Runtime::new(state, view, (300.0, 200.0), Env::new(true, 1.0));
    rt.on_key = Some(heard_key);
    rt.frame(0.0);
    let at = rt.find("tile").expect("the tile");
    (rt, at)
}

/// Wayland: once the system drag starts, the synthetic Escape ends Rune's capture at once
/// (no focus bounce needed), the cancelled Up it brings is swallowed, the Escape reaches no
/// key handler, and the pointer coming back with no button reaches the element no more.
#[test]
fn on_wayland_the_escape_ends_runes_capture_at_the_start() {
    let (mut rt, (x, y)) = runtime(Release::Escape);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(x + 4.0, y);
    rt.frame(0.0);
    assert_eq!(rt.state.started, 0, "under the threshold it's a press");
    rt.pointer_move(x + 12.0, y);
    rt.frame(0.0);
    assert_eq!(rt.state.started, 1, "past the threshold the drag starts");
    assert!(rt.pressing(), "no Escape before the compositor's proof");
    holding(&mut rt);
    rt.frame(0.0);
    assert!(
        !rt.pressing(),
        "Rune's capture ended once the compositor had the pointer"
    );
    // The compositor takes the pointer, then gives it back with no button held.
    rt.pointer_leave();
    rt.pointer_move(x + 40.0, y + 20.0);
    rt.frame(0.0);
    rt.pointer_move(x + 45.0, y + 25.0);
    rt.frame(0.0);
    assert!(!rt.pressing());
    assert_eq!(
        rt.state.phases,
        vec![(Phase::Down, false), (Phase::Move, false)],
        "the element's own handler heard the press and the move under the threshold only"
    );
    assert!(
        rt.state.keys.is_empty(),
        "the Escape leaked: {:?}",
        rt.state.keys
    );
    // The next press is the element's again.
    rt.pointer_down(x, y);
    rt.frame(0.0);
    assert_eq!(rt.state.phases.last(), Some(&(Phase::Down, false)));
}

/// X11: Rune keeps the capture through the drag (winit has the grab), the real release is
/// swallowed, and a user's Escape cancels the press, which the guard turns into a cancel.
#[test]
fn on_x11_the_real_release_is_swallowed() {
    let (mut rt, (x, y)) = runtime(Release::RealUp);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_move(x + 12.0, y);
    rt.frame(0.0);
    assert_eq!(rt.state.started, 1);
    assert!(
        rt.pressing(),
        "no Escape on X11: the capture runs to the release"
    );
    rt.pointer_move(x + 30.0, y);
    rt.pointer_up(x + 30.0, y);
    rt.frame(0.0);
    assert!(!rt.pressing());
    assert_eq!(
        rt.state.phases,
        vec![(Phase::Down, false)],
        "moves and the release after the start are the system drag's"
    );
    assert!(rt.state.keys.is_empty(), "{:?}", rt.state.keys);
}

/// Review round 1, B2: a fast flick. The threshold move and the release reach Rune in one
/// batch, so the release has ended the capture before any Escape P1 sends is handled: an
/// Escape sent then would reach the app's key handlers.
#[test]
fn a_fast_flick_on_wayland_sends_no_escape_to_the_app() {
    let (mut rt, (x, y)) = runtime(Release::Escape);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(x + 12.0, y);
    rt.pointer_up(x + 12.0, y);
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(rt.state.started, 1, "the flick crossed the threshold");
    assert!(!rt.pressing());
    assert!(
        rt.state.keys.is_empty(),
        "the Escape leaked: {:?}",
        rt.state.keys
    );
}

/// The flick again, with the compositor's proof arriving after the release: the guard has
/// seen the Up, so no Escape goes out.
#[test]
fn a_proof_after_the_release_sends_no_escape() {
    let (mut rt, (x, y)) = runtime(Release::Escape);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(x + 12.0, y);
    rt.pointer_up(x + 12.0, y);
    rt.frame(0.0);
    holding(&mut rt);
    rt.frame(0.0);
    assert_eq!(rt.state.started, 1);
    assert!(
        rt.state.keys.is_empty(),
        "the Escape leaked: {:?}",
        rt.state.keys
    );
}

/// Review round 1: an app's own `on_drag` calling `start_drag` without routing its press
/// through `owns` gets no Escape, so no cancelled Up it didn't ask for.
#[test]
fn start_drag_from_a_press_the_guard_never_saw_sends_no_escape() {
    let (mut rt, _) = runtime(Release::Escape);
    let (x, y) = rt.find("own").expect("the app's own element");
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(x + 12.0, y);
    rt.frame(0.0);
    assert_eq!(rt.state.started, 1);
    holding(&mut rt);
    rt.frame(0.0);
    assert!(
        !rt.state.own.iter().any(|(_, cancelled)| *cancelled),
        "the app's handler heard a cancel it didn't ask for: {:?}",
        rt.state.own
    );
    assert!(rt.state.keys.is_empty(), "{:?}", rt.state.keys);
}

/// Review round 2, R2-B1: the source leaves the tree when its drag starts (an app hiding what
/// it moves, a tab torn off). Rune's `cancel_drag` finds no live element, so an Escape would
/// go on to the app's keys: none may be sent.
mod gone {
    use rune::{Cx, El, Env, KeyEvent, Runtime, Tx, col, el};

    use super::super::hub::tests::FAKE_START;
    use super::super::session::Release;
    use super::super::*;

    struct G {
        dnd: Dnd<G>,
        hide: bool,
        keys: Vec<String>,
    }

    fn view(s: &G, _: &mut Cx<G>) -> El {
        let source = DragSource::new(|_: &G, _| Some(DragItem::new().text("hello")))
            .on_start(|s: &mut G, _| s.hide = true);
        let mut c = col().w_full().h_full();
        if !s.hide {
            c = c.child(s.dnd.drag_source(el().size(100.0, 60.0), "tile", source));
        }
        c.child(el().key("rest").size(100.0, 60.0))
    }

    fn heard(s: &mut G, ev: &KeyEvent, _: &mut Tx) -> bool {
        s.keys.push(format!("{:?}", ev.key));
        false
    }

    #[test]
    fn no_escape_when_the_source_has_left_the_tree() {
        FAKE_START.set(Some(Release::Escape));
        let st = G {
            dnd: Dnd::new(),
            hide: false,
            keys: Vec::new(),
        };
        let mut rt = Runtime::new(st, view, (300.0, 200.0), Env::new(true, 1.0));
        rt.on_key = Some(heard);
        rt.frame(0.0);
        let (x, y) = rt.find("tile").expect("tile");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(0.0);
        rt.pointer_move(x + 12.0, y);
        rt.frame(0.0);
        assert!(rt.state.hide, "the drag started and hid its source");
        rt.update_if(|s, tx| {
            s.dnd.clone().holding_for_test(tx);
            false
        });
        rt.frame(0.0);
        assert!(
            rt.state.keys.is_empty(),
            "the Escape leaked: {:?}",
            rt.state.keys
        );
    }
}

/// Starts a Wayland drag from the tile, under the threshold first, and returns the drag's id.
fn started(rt: &mut Runtime<S>, (x, y): (f32, f32)) -> u64 {
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(x + 12.0, y);
    rt.frame(0.0);
    assert_eq!(rt.state.started, 1);
    rt.state.dnd.drag_id_for_test().expect("a drag")
}

fn route(rt: &mut Runtime<S>, r: super::hub::Routed) {
    let cell = std::cell::Cell::new(Some(r));
    rt.update_if(move |s, tx| {
        if let Some(r) = cell.take() {
            s.dnd.clone().route_for_test(r, s, tx);
        }
        false
    });
}

/// Review round 2: the proof's update first checks the drag still runs (`still_holding`).
/// Here the window lost focus before the update (Rune ended the capture at that input, and
/// its cancelled Up waits for the next frame), and the drag has dropped meanwhile: no Escape.
#[test]
fn a_proof_for_a_drag_that_already_dropped_sends_no_escape() {
    let (mut rt, at) = runtime(Release::Escape);
    let id = started(&mut rt, at);
    rt.window_focus(false);
    super::hub::tests::FAKE_LIVE.set(Some(false));
    route(&mut rt, super::hub::Routed::Holding { id });
    super::hub::tests::FAKE_LIVE.set(None);
    rt.frame(0.0);
    rt.frame(0.0);
    assert!(
        rt.state.keys.is_empty(),
        "the Escape leaked: {:?}",
        rt.state.keys
    );
}

/// R2-B1: the source moves to another place in the tree when its drag starts (wrapped in a
/// new parent): Rune's capture names the old path, so no Escape may be sent.
mod moved {
    use rune::{Cx, El, Env, KeyEvent, Runtime, Tx, col, el};

    use super::super::hub::tests::FAKE_START;
    use super::super::session::Release;
    use super::super::*;

    struct M {
        dnd: Dnd<M>,
        wrap: bool,
        keys: Vec<String>,
    }

    fn view(s: &M, _: &mut Cx<M>) -> El {
        let source = DragSource::new(|_: &M, _| Some(DragItem::new().text("hello")))
            .on_start(|s: &mut M, _| s.wrap = true);
        let tile = s.dnd.drag_source(el().size(100.0, 60.0), "tile", source);
        let c = col().w_full().h_full();
        if s.wrap {
            c.child(col().key("wrapper").child(tile))
        } else {
            c.child(tile)
        }
    }

    fn heard(s: &mut M, ev: &KeyEvent, _: &mut Tx) -> bool {
        s.keys.push(format!("{:?}", ev.key));
        false
    }

    #[test]
    fn no_escape_when_the_source_moved_to_another_path() {
        FAKE_START.set(Some(Release::Escape));
        let st = M {
            dnd: Dnd::new(),
            wrap: false,
            keys: Vec::new(),
        };
        let mut rt = Runtime::new(st, view, (300.0, 200.0), Env::new(true, 1.0));
        rt.on_key = Some(heard);
        rt.frame(0.0);
        let (x, y) = rt.find("tile").expect("tile");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(0.0);
        rt.pointer_move(x + 12.0, y);
        rt.frame(0.0);
        assert!(rt.state.wrap, "the drag started and moved its source");
        rt.update_if(|s, tx| {
            s.dnd.clone().holding_for_test(tx);
            false
        });
        rt.frame(0.0);
        assert!(
            rt.state.keys.is_empty(),
            "the Escape leaked: {:?}",
            rt.state.keys
        );
    }
}

mod review3;
