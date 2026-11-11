//! Review round 3: Rune drops a capture without an `Up` (a press replaces it; a modal
//! mounting clears it), so nothing may end a capture "later" on a timer; the source is picked
//! at the press point, not its centre; and a source that loses its `on_drag` is a known limit.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rune::{Cx, El, Env, KeyEvent, Phase, Runtime, Tx, col, el};

use super::super::hub::tests::FAKE_START;
use super::super::hub::{Job, Poster, Routed};
use super::super::session::Release;
use super::super::*;

#[derive(Default, Clone, Copy, PartialEq)]
enum Mode {
    #[default]
    Plain,
    /// The source sits in a 40 pt clip, 30 pt down: its centre is clipped at the start.
    Clipped,
    /// After the start the source is drawn without its `on_drag` (same key, same path).
    LosesHandler,
}

struct P {
    dnd: Dnd<P>,
    mode: Mode,
    started: bool,
    keys: Vec<String>,
    /// Phases another `on_drag` element heard.
    other: Vec<(Phase, bool)>,
}

fn view(s: &P, _: &mut Cx<P>) -> El {
    let source = DragSource::new(|_: &P, _| Some(DragItem::new().text("hello")))
        .on_start(|s: &mut P, _| s.started = true);
    let tile = if s.mode == Mode::LosesHandler && s.started {
        el().key("tile").size(100.0, 60.0)
    } else {
        s.dnd.drag_source(el().size(100.0, 60.0), "tile", source)
    };
    let other =
        el().key("other")
            .size(100.0, 60.0)
            .on_drag(|s: &mut P, d: &mut rune::Drag, _: &mut Tx| {
                s.other.push((d.phase, d.cancelled()));
            });
    let button = el()
        .key("button")
        .size(100.0, 30.0)
        .on_press(|_: &mut P| {});
    let c = col().w_full().h_full();
    let c = match s.mode {
        Mode::Clipped => c.child(
            col()
                .key("clip")
                .w(100.0)
                .h(40.0)
                .clip_rounded(0.5)
                .child(el().size(100.0, 30.0))
                .child(tile),
        ),
        _ => c.child(tile),
    };
    c.child(other).child(button)
}

fn heard(s: &mut P, ev: &KeyEvent, _: &mut Tx) -> bool {
    s.keys.push(format!("{:?}", ev.key));
    false
}

type Jobs = Arc<Mutex<Vec<Job>>>;

/// A runtime whose window posts its jobs into `Jobs`, as an attached window's go to its
/// update: a timer of P1's, if it had one, would fire through them.
fn runtime(mode: Mode) -> (Runtime<P>, Jobs) {
    FAKE_START.set(Some(Release::Escape));
    let st = P {
        dnd: Dnd::new(),
        mode,
        started: false,
        keys: Vec::new(),
        other: Vec::new(),
    };
    let jobs: Jobs = Arc::default();
    let sink = jobs.clone();
    let poster: Poster = Arc::new(move |job| {
        sink.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(job);
    });
    st.dnd.set_poster_for_test(poster);
    let mut rt = Runtime::new(st, view, (300.0, 300.0), Env::new(true, 1.0));
    rt.on_key = Some(heard);
    rt.frame(0.0);
    (rt, jobs)
}

fn start(rt: &mut Runtime<P>, x: f32, y: f32) -> u64 {
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(0.0);
    rt.pointer_move(x + 12.0, y);
    rt.frame(0.0);
    assert!(rt.state.started, "the drag started");
    rt.state.dnd.drag_id_for_test().expect("a drag")
}

fn route(rt: &mut Runtime<P>, r: Routed) {
    let cell = std::cell::Cell::new(Some(r));
    rt.update_if(move |s, tx| {
        if let Some(r) = cell.take() {
            s.dnd.clone().route_for_test(r, s, tx);
        }
        true
    });
    rt.frame(0.0);
}

fn holding(rt: &mut Runtime<P>) {
    rt.update_if(|s, tx| {
        s.dnd.clone().holding_for_test(tx);
        false
    });
    rt.frame(0.0);
}

/// Waits out any timer P1 started (the old late Escape waited 150 ms), then runs whatever it
/// posted, as the window's updates would.
fn let_time_pass(rt: &mut Runtime<P>, jobs: &Jobs) {
    std::thread::sleep(Duration::from_millis(400));
    let posted: Vec<Job> =
        std::mem::take(&mut *jobs.lock().unwrap_or_else(PoisonError::into_inner));
    for job in posted {
        let cell = std::cell::Cell::new(Some(job));
        rt.update_if(move |s, tx| match cell.take() {
            Some(job) => s.dnd.clone().run_job_for_test(job, s, tx),
            None => false,
        });
        rt.frame(0.0);
    }
    rt.frame(0.0);
}

/// B1: no proof reached the drag, and after it ended the user clicked a button: Rune's press
/// replaced the capture without an Up. Nothing may later send an Escape to the app.
#[test]
fn a_click_after_a_drag_no_proof_reached_gets_no_escape() {
    let (mut rt, jobs) = runtime(Mode::Plain);
    let (x, y) = rt.find("tile").expect("tile");
    let id = start(&mut rt, x, y);
    route(
        &mut rt,
        Routed::Ended {
            id,
            outcome: rune::DragOutcome::Cancelled,
        },
    );
    assert!(rt.pressing(), "no proof: Rune still holds the press");
    let (bx, by) = rt.find("button").expect("button");
    rt.pointer_move(bx, by);
    rt.pointer_down(bx, by);
    rt.frame(0.0);
    rt.pointer_up(bx, by);
    rt.frame(0.0);
    assert!(!rt.pressing(), "the press replaced the capture");
    let_time_pass(&mut rt, &jobs);
    assert!(
        rt.state.keys.is_empty(),
        "an Escape reached the app: {:?}",
        rt.state.keys
    );
}

/// B1: the user's next press, on another drag element, is held when time passes: it must not
/// be cancelled.
#[test]
fn the_next_drag_is_not_cancelled_by_an_earlier_one() {
    let (mut rt, jobs) = runtime(Mode::Plain);
    let (x, y) = rt.find("tile").expect("tile");
    let id = start(&mut rt, x, y);
    route(
        &mut rt,
        Routed::Ended {
            id,
            outcome: rune::DragOutcome::Cancelled,
        },
    );
    let (ox, oy) = rt.find("other").expect("other");
    rt.pointer_move(ox, oy);
    rt.pointer_down(ox, oy);
    rt.frame(0.0);
    let_time_pass(&mut rt, &jobs);
    assert!(rt.pressing(), "the new press is still held");
    assert_eq!(
        rt.state.other,
        vec![(Phase::Down, false)],
        "the new drag was cancelled"
    );
    assert!(rt.state.keys.is_empty(), "{:?}", rt.state.keys);
}

/// What a drag no proof reached leaves: Rune holds the capture until the next press (or a
/// focus change), with no Escape of P1's. Path 3 in the module docs.
#[test]
fn a_drag_no_proof_reached_holds_the_capture_until_the_next_press() {
    let (mut rt, jobs) = runtime(Mode::Plain);
    let (x, y) = rt.find("tile").expect("tile");
    let id = start(&mut rt, x, y);
    route(
        &mut rt,
        Routed::Ended {
            id,
            outcome: rune::DragOutcome::Copied,
        },
    );
    let_time_pass(&mut rt, &jobs);
    assert!(rt.pressing(), "held until the next press");
    let (bx, by) = rt.find("button").expect("button");
    rt.pointer_move(bx, by);
    rt.pointer_down(bx, by);
    rt.pointer_up(bx, by);
    rt.frame(0.0);
    assert!(!rt.pressing());
    assert!(rt.state.keys.is_empty(), "{:?}", rt.state.keys);
}

/// SF1: the source's centre is clipped away at the start (a row more than half scrolled out),
/// and it was pressed on its visible strip: the pick at the press point still finds it, so
/// the proof ends the capture.
#[test]
fn a_source_whose_centre_is_clipped_still_gets_its_escape() {
    let (mut rt, _) = runtime(Mode::Clipped);
    let (cx, cy) = rt.find("clip").expect("clip");
    start(&mut rt, cx, cy + 15.0);
    holding(&mut rt);
    assert!(!rt.pressing(), "the proof ended the capture");
    assert!(rt.state.keys.is_empty(), "{:?}", rt.state.keys);
}

/// B2, a known limit (path 5 in the module docs): a source drawn without its `on_drag` after
/// the start, same key and path. P1 can't see handlers, so it sends its Escape; Rune's
/// `cancel_drag` refuses a capture whose element lost its `on_drag`, the Escape reaches the
/// app's keys once, and the capture stays held. If a Rune move changes this, update the docs.
#[test]
fn known_limit_a_source_that_loses_its_on_drag_lets_one_escape_through() {
    let (mut rt, _) = runtime(Mode::LosesHandler);
    let (x, y) = rt.find("tile").expect("tile");
    start(&mut rt, x, y);
    holding(&mut rt);
    assert_eq!(rt.state.keys, vec!["Escape".to_owned()]);
    assert!(rt.pressing());
}

mod modal {
    use super::*;

    struct M {
        dnd: Dnd<M>,
        dialog: bool,
        dismissed: u32,
        keys: Vec<String>,
    }

    fn view(s: &M, _: &mut Cx<M>) -> El {
        let source = DragSource::new(|_: &M, _| Some(DragItem::new().text("hello")))
            .on_end(|s: &mut M, _, _| s.dialog = true);
        let mut c = col().w_full().h_full().child(s.dnd.drag_source(
            el().size(100.0, 60.0),
            "tile",
            source,
        ));
        if s.dialog {
            c = c.child(el().key("dialog").size(100.0, 60.0).focusable(true).modal(
                false,
                |s: &mut M| {
                    s.dismissed += 1;
                    s.dialog = false;
                },
            ));
        }
        c
    }

    fn heard(s: &mut M, ev: &KeyEvent, _: &mut Tx) -> bool {
        s.keys.push(format!("{:?}", ev.key));
        false
    }

    /// B1: no proof; the drag ends and `on_end` opens a modal, whose mounting clears Rune's
    /// capture without an Up. Nothing may dismiss it later.
    #[test]
    fn a_modal_opened_in_on_end_is_not_dismissed() {
        FAKE_START.set(Some(Release::Escape));
        let st = M {
            dnd: Dnd::new(),
            dialog: false,
            dismissed: 0,
            keys: Vec::new(),
        };
        let jobs: Jobs = Arc::default();
        let sink = jobs.clone();
        st.dnd.set_poster_for_test(Arc::new(move |job| {
            sink.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(job);
        }));
        let mut rt = Runtime::new(st, view, (300.0, 300.0), Env::new(true, 1.0));
        rt.on_key = Some(heard);
        rt.frame(0.0);
        let (x, y) = rt.find("tile").expect("tile");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(0.0);
        rt.pointer_move(x + 12.0, y);
        rt.frame(0.0);
        let id = rt.state.dnd.drag_id_for_test().expect("a drag");
        let cell = std::cell::Cell::new(Some(Routed::Ended {
            id,
            outcome: rune::DragOutcome::Cancelled,
        }));
        rt.update_if(move |s, tx| {
            if let Some(r) = cell.take() {
                s.dnd.clone().route_for_test(r, s, tx);
            }
            true
        });
        rt.frame(0.0);
        rt.frame(0.0);
        assert!(rt.state.dialog, "on_end opened the modal");
        std::thread::sleep(Duration::from_millis(400));
        let posted: Vec<Job> =
            std::mem::take(&mut *jobs.lock().unwrap_or_else(PoisonError::into_inner));
        for job in posted {
            let cell = std::cell::Cell::new(Some(job));
            rt.update_if(move |s, tx| match cell.take() {
                Some(job) => s.dnd.clone().run_job_for_test(job, s, tx),
                None => false,
            });
            rt.frame(0.0);
        }
        rt.frame(0.0);
        assert_eq!(rt.state.dismissed, 0, "the modal was dismissed");
        assert!(rt.state.dialog);
        assert!(rt.state.keys.is_empty(), "{:?}", rt.state.keys);
    }
}

mod modal_at_start {
    use super::*;

    struct M {
        dnd: Dnd<M>,
        dialog: bool,
        keys: Vec<String>,
    }

    fn view(s: &M, _: &mut Cx<M>) -> El {
        let source = DragSource::new(|_: &M, _| Some(DragItem::new().text("hello")))
            .on_start(|s: &mut M, _| s.dialog = true);
        let mut c = col().w_full().h_full().child(s.dnd.drag_source(
            el().size(100.0, 60.0),
            "tile",
            source,
        ));
        if s.dialog {
            c = c.child(
                el().key("dialog")
                    .size(100.0, 60.0)
                    .focusable(true)
                    .modal(false, |s: &mut M| s.dialog = false),
            );
        }
        c
    }

    fn heard(s: &mut M, ev: &KeyEvent, _: &mut Tx) -> bool {
        s.keys.push(format!("{:?}", ev.key));
        false
    }

    /// A known limit (path 2 in the module docs): `on_start` opens a modal, whose mounting
    /// clears Rune's capture without an Up before the compositor's proof arrives. P1 can't
    /// see that, so the proof's Escape goes to the modal, which closes. If a Rune move
    /// changes this, update the docs.
    #[test]
    fn known_limit_a_modal_opened_in_on_start_takes_the_escape() {
        FAKE_START.set(Some(Release::Escape));
        let st = M {
            dnd: Dnd::new(),
            dialog: false,
            keys: Vec::new(),
        };
        let mut rt = Runtime::new(st, view, (300.0, 300.0), Env::new(true, 1.0));
        rt.on_key = Some(heard);
        rt.frame(0.0);
        let (x, y) = rt.find("tile").expect("tile");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(0.0);
        rt.pointer_move(x + 12.0, y);
        rt.frame(0.0);
        rt.frame(0.0);
        assert!(rt.state.dialog, "on_start opened the modal");
        assert!(!rt.pressing(), "its mounting cleared the capture");
        rt.update_if(|s, tx| {
            s.dnd.clone().holding_for_test(tx);
            false
        });
        rt.frame(0.0);
        rt.frame(0.0);
        assert!(!rt.state.dialog, "the Escape closed the modal");
    }
}
