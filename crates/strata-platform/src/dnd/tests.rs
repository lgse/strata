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
                    dnd.start_drag(tx, DragItem::new().text("own"), |_: &mut S, _, _| {})
                        .expect("the fake start");
                }
            });
    col()
        .w_full()
        .h_full()
        .child(s.dnd.drag_source(el().size(100.0, 60.0), "tile", source))
        .child(own)
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
fn on_wayland_cancel_pointer_ends_runes_capture_at_the_start() {
    let (mut rt, (x, y)) = runtime(Release::CancelPointer);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(x + 4.0, y);
    rt.frame(0.0);
    assert_eq!(rt.state.started, 0, "under the threshold it's a press");
    rt.pointer_move(x + 12.0, y);
    rt.frame(0.0);
    assert_eq!(rt.state.started, 1, "past the threshold the drag starts");
    assert!(
        !rt.pressing(),
        "Rune's capture ended immediately via Tx::cancel_pointer"
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
    assert!(rt.state.keys.is_empty(), "{:?}", rt.state.keys);
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
        "no cancel_pointer on X11: the capture runs to the release"
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
fn a_fast_flick_on_wayland_sends_no_keys_to_the_app() {
    let (mut rt, (x, y)) = runtime(Release::CancelPointer);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(x + 12.0, y);
    rt.pointer_up(x + 12.0, y);
    rt.frame(0.0);
    rt.frame(0.0);
    assert_eq!(rt.state.started, 1, "the flick crossed the threshold");
    assert!(!rt.pressing());
    assert!(rt.state.keys.is_empty(), "{:?}", rt.state.keys);
}

#[test]
fn start_drag_from_a_press_the_guard_never_saw_does_not_cancel_capture() {
    let (mut rt, _) = runtime(Release::CancelPointer);
    let (x, y) = rt.find("own").expect("the app's own element");
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(x + 12.0, y);
    rt.frame(0.0);
    assert_eq!(rt.state.started, 1);
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
    fn capture_ends_cleanly_when_the_source_leaves_the_tree() {
        FAKE_START.set(Some(Release::CancelPointer));
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
        assert!(!rt.pressing());
        assert!(rt.state.keys.is_empty(), "{:?}", rt.state.keys);
    }
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
    fn capture_ends_cleanly_when_the_source_moves_to_another_path() {
        FAKE_START.set(Some(Release::CancelPointer));
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
        assert!(!rt.pressing());
        assert!(rt.state.keys.is_empty(), "{:?}", rt.state.keys);
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
