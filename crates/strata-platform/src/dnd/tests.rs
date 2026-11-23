// SPDX-License-Identifier: MIT

use rune::{Cx, El, Env, KeyEvent, Phase, Runtime, Tx, col, el};

use super::hub::tests::FAKE_START;
use super::session::Release;
use super::*;

struct S {
    dnd: Dnd<S>,
    phases: Vec<(Phase, bool)>,
    started: u32,
    keys: Vec<String>,
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

mod reads;
mod review3;

#[test]
fn a_target_reads_the_offered_form_of_each_listed_type() {
    let offer = DropOffer {
        mimes: vec![
            "text/plain".to_owned(),
            "text/plain;charset=utf-8".to_owned(),
            "application/octet-stream;name=\"photo.jpg\"".to_owned(),
        ],
        actions: Actions::COPY,
        local: false,
        mods: None,
        serial: 1,
        peeked: Vec::new(),
    };
    let target = DropTarget::<S>::new(|_, _, _| {})
        .mime_types(&[
            "application/octet-stream",
            "text/plain;charset=utf-8",
            "text/plain",
        ])
        .read_all();
    assert_eq!(
        target.reads(&offer),
        [
            "application/octet-stream;name=\"photo.jpg\"",
            "text/plain;charset=utf-8",
            "text/plain",
        ]
    );
    assert!(offer.offers("application/octet-stream"));
    assert!(!offer.offers("image/png"));
    let first = DropTarget::<S>::new(|_, _, _| {}).mime_types(&["application/octet-stream"]);
    assert_eq!(
        first.reads(&offer),
        ["application/octet-stream;name=\"photo.jpg\""]
    );
}
