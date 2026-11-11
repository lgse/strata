use super::*;

fn offered() -> Vec<String> {
    vec!["text/uri-list".into(), "text/plain".into()]
}

#[test]
fn finish_before_a_drop_or_action_is_refused() {
    let mut s = DropState::new(3);
    s.accept(Some("text/plain"), &offered()).expect("accept");
    assert!(s.finish().is_err(), "no drop yet");
    s.on_drop();
    assert!(s.finish().is_err(), "no action yet");
    s.on_action(0);
    assert!(s.finish().is_err(), "the action is none");
    s.on_action(Action::Ask.bits());
    assert!(s.finish().is_err(), "ask isn't settled");
    s.on_action(Action::Copy.bits());
    assert_eq!(s.finish(), Ok(true));
}

#[test]
fn finish_after_accepting_nothing_is_refused() {
    let mut s = DropState::new(3);
    s.accept(Some("text/plain"), &offered()).expect("accept");
    s.accept(None, &offered()).expect("accept nothing");
    s.on_drop();
    s.on_action(Action::Copy.bits());
    assert!(s.finish().is_err());
}

#[test]
fn an_action_outside_the_mask_is_refused() {
    let mut s = DropState::new(3);
    let (copy, mv) = (Action::Copy.bits(), Action::Move.bits());
    assert!(s.set_actions(copy | mv, copy).is_ok());
    assert!(s.set_actions(copy, 0).is_ok(), "no preference is allowed");
    assert!(
        s.set_actions(copy, mv).is_err(),
        "preferred not in the mask"
    );
    assert!(
        s.set_actions(copy | mv, copy | mv).is_err(),
        "two preferred"
    );
    assert!(s.set_actions(8, 0).is_err(), "an unknown bit");
    assert!(
        s.accept(Some("image/png"), &offered()).is_err(),
        "a type not offered"
    );
    assert!(
        DropState::new(2).set_actions(copy, copy).is_err(),
        "version 2 has no actions"
    );
}

#[test]
fn nothing_after_finish() {
    let mut s = DropState::new(3);
    s.accept(Some("text/plain"), &offered()).expect("accept");
    s.on_drop();
    s.on_action(Action::Move.bits());
    assert_eq!(s.finish(), Ok(true));
    assert!(s.finish().is_err());
    assert!(s.accept(Some("text/plain"), &offered()).is_err());
    assert!(s.set_actions(1, 1).is_err());
}

#[test]
fn an_old_offer_is_destroyed_without_finish() {
    let mut s = DropState::new(1);
    s.accept(Some("text/plain"), &offered()).expect("accept");
    s.on_drop();
    assert_eq!(s.finish(), Ok(false));
}

/// A drop whose action is still ask, from a source offering `source`.
fn asked(source: &[Action]) -> DropState {
    let mut s = DropState::new(3);
    s.on_source_actions(source.iter().fold(0, |m, a| m | a.bits()));
    s.accept(Some("text/plain"), &offered()).expect("accept");
    s.on_action(Action::Ask.bits());
    s.on_drop();
    s
}

#[test]
fn an_ask_drop_settles_on_one_action_the_source_offers() {
    let (copy, ask) = (Action::Copy.bits(), Action::Ask.bits());
    let mut s = asked(&[Action::Copy, Action::Ask]);
    assert!(s.finish().is_err(), "ask isn't settled yet");
    // The compositor can only take copy: the drop is settled.
    s.set_actions(copy, copy).expect("answer copy");
    assert_eq!(s.finish(), Ok(true));
    // Before the drop, set_actions settles nothing.
    let mut before = DropState::new(3);
    before.on_source_actions(copy | ask);
    before.on_action(ask);
    before
        .set_actions(copy, copy)
        .expect("set_actions before the drop");
    before
        .accept(Some("text/plain"), &offered())
        .expect("accept");
    before.on_drop();
    assert!(
        before.finish().is_err(),
        "only a request after the drop settles ask"
    );
}

/// Round 2's case: wlroots ignores a preferred action the source doesn't offer and takes the
/// lowest common one, here ask again, so a finish would end winit's connection. After the
/// drop such answers are refused, and the drop stays on ask.
#[test]
fn an_ask_drop_answered_with_an_action_the_source_lacks_stays_ask() {
    let (copy, mv, ask) = (Action::Copy.bits(), Action::Move.bits(), Action::Ask.bits());
    for (mask, preferred, why) in [
        (mv | ask, mv, "move isn't offered"),
        (
            copy | ask,
            copy,
            "two actions: the compositor may still pick ask",
        ),
        (ask, ask, "ask answers nothing"),
        (0, 0, "no actions"),
    ] {
        let mut s = asked(&[Action::Copy, Action::Ask]);
        assert!(s.set_actions(mask, preferred).is_err(), "refused: {why}");
        assert!(s.finish().is_err(), "still ask: {why}");
    }
    // Source actions never heard: nothing is known to be offered.
    let mut s = DropState::new(3);
    s.accept(Some("text/plain"), &offered()).expect("accept");
    s.on_action(ask);
    s.on_drop();
    assert!(
        s.set_actions(copy, copy).is_err(),
        "the source's actions are unknown"
    );
    assert!(s.finish().is_err());
}

/// Round 3's case, from sway: a copy drop, then set_actions(none) after it. The compositor
/// would now have no action, but its answer can't reach a dropped offer (leave has come), so
/// the request is refused and the drop stays a copy that finish may end.
#[test]
fn after_a_settled_drop_set_actions_is_refused() {
    let (copy, mv) = (Action::Copy.bits(), Action::Move.bits());
    let mut s = DropState::new(3);
    s.on_source_actions(copy | mv);
    s.accept(Some("text/plain"), &offered()).expect("accept");
    s.set_actions(copy, copy).expect("before the drop");
    s.on_action(copy);
    s.on_drop();
    assert!(s.set_actions(0, 0).is_err(), "no actions, after the drop");
    assert!(
        s.set_actions(mv, mv).is_err(),
        "another action, after the drop"
    );
    assert_eq!(s.finish(), Ok(true), "the copy stands");
}

/// P0's review, S1: after the drop, the settled action may be confirmed as `(a, a)` before
/// finish, and that is the only other request let through.
#[test]
fn after_the_drop_the_settled_action_may_be_confirmed() {
    let (copy, mv) = (Action::Copy.bits(), Action::Move.bits());
    let mut s = DropState::new(3);
    s.on_source_actions(copy | mv);
    s.accept(Some("text/plain"), &offered()).expect("accept");
    s.set_actions(copy | mv, mv).expect("before the drop");
    s.on_action(mv);
    s.on_drop();
    assert_eq!(s.settled(), Some(Action::Move));
    assert!(s.set_actions(mv, mv).is_ok(), "confirming move");
    assert!(
        s.set_actions(mv, 0).is_err(),
        "a confirmation names its action"
    );
    assert!(
        s.set_actions(copy, copy).is_err(),
        "another action isn't a confirmation"
    );
    assert_eq!(
        s.settled(),
        Some(Action::Move),
        "the confirmation changes nothing"
    );
    assert_eq!(s.finish(), Ok(true));
}

#[test]
fn an_action_the_source_lacks_or_ask_is_never_confirmed() {
    let (copy, mv, ask) = (Action::Copy.bits(), Action::Move.bits(), Action::Ask.bits());
    let mut s = DropState::new(3);
    s.on_source_actions(copy | ask);
    s.accept(Some("text/plain"), &offered()).expect("accept");
    s.on_action(ask);
    s.on_drop();
    assert_eq!(s.settled(), None, "ask isn't settled");
    assert!(s.set_actions(ask, ask).is_err(), "ask isn't confirmed");
    // A move the source never offered, as an action event might claim.
    let mut s = DropState::new(3);
    s.on_source_actions(copy);
    s.accept(Some("text/plain"), &offered()).expect("accept");
    s.on_action(mv);
    s.on_drop();
    assert!(s.set_actions(mv, mv).is_err());
}

// ---- DropState against wlroots' data_offer_choose_action (review round 3) ----

const ASK: u32 = 4;

/// wlroots' `data_offer_choose_action` (`types/data_device/wlr_data_offer.c`): the
/// compositor's forced action if the source and target share it, else the target's preferred
/// action if they share it, else the lowest action they share, else none.
fn wlr_choose(target: u32, preferred: u32, source: u32, forced: u32) -> u32 {
    let shared = target & source;
    if shared == 0 {
        0
    } else if forced & shared != 0 {
        forced
    } else if preferred & shared != 0 {
        preferred
    } else {
        1 << shared.trailing_zeros()
    }
}

/// Every request set_actions can carry: any mask of the three actions, and no preference or
/// one action.
fn requests() -> Vec<(u32, u32)> {
    (0..8u32)
        .flat_map(|mask| [0u32, 1, 2, 4].map(|preferred| (mask, preferred)))
        .collect()
}

/// For every source mask, compositor-forced action, request before the drop (its action
/// event applied before the drop or, as it may be, after) and request after the drop (or
/// none): DropState never lets `finish` through when wlroots would take it as a protocol
/// error (no action, or ask), and never refuses one wlroots would take.
#[test]
fn no_finish_is_allowed_that_wlroots_rejects() {
    let requests = requests();
    let (mut unsafe_finishes, mut refused_valid, mut sequences) = (Vec::new(), 0usize, 0usize);
    for source in 0..8u32 {
        for forced in [0u32, 1, 2] {
            for &(mask, preferred) in &requests {
                for late in [false, true] {
                    let mut st = DropState::new(3);
                    st.on_source_actions(source);
                    st.accept(Some("text/plain"), &offered()).expect("accept");
                    let mut server = 0;
                    if st.set_actions(mask, preferred).is_ok() {
                        server = wlr_choose(mask, preferred, source, forced);
                    }
                    if server == 0 {
                        // No action: the compositor cancels instead of dropping.
                        continue;
                    }
                    if !late {
                        st.on_action(server);
                    }
                    st.on_drop();
                    if late {
                        st.on_action(server);
                    }
                    let after = std::iter::once(None).chain(requests.iter().copied().map(Some));
                    for post in after {
                        let mut s = st.clone();
                        let mut srv = server;
                        if let Some((m, p)) = post
                            && s.set_actions(m, p).is_ok()
                        {
                            // Its answer never reaches the dropped offer.
                            srv = wlr_choose(m, p, source, forced);
                        }
                        sequences += 1;
                        let allowed = s.finish().is_ok();
                        let valid = srv != 0 && srv != ASK;
                        if allowed && !valid {
                            unsafe_finishes.push((
                                source,
                                forced,
                                (mask, preferred),
                                late,
                                post,
                                srv,
                            ));
                        }
                        if !allowed && valid {
                            refused_valid += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(sequences > 10_000, "{sequences} sequences");
    assert!(
        unsafe_finishes.is_empty(),
        "{} of {sequences} sequences let a finish through that wlroots rejects, e.g. \
         (source, forced, before, late, after, compositor's action): {:?}",
        unsafe_finishes.len(),
        &unsafe_finishes[..unsafe_finishes.len().min(5)]
    );
    assert_eq!(refused_valid, 0, "finishes refused that wlroots would take");
}

// ---- offers without a compositor: closed proxies on a connection to nothing ----

use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::Duration;

use wayland_backend::client::Backend;

/// A connection with no server behind it: offers on it can only be closed ones.
fn nowhere(ui: Option<std::thread::ThreadId>) -> (Backend, Home) {
    let (ours, _theirs) = UnixStream::pair().expect("socket pair");
    let backend = Backend::connect(ours).expect("libwayland-client");
    let home = Home {
        backend: backend.downgrade(),
        ui,
    };
    (backend, home)
}

fn other_thread() -> std::thread::ThreadId {
    std::thread::spawn(|| std::thread::current().id())
        .join()
        .expect("thread")
}

#[test]
fn a_read_that_fails_at_once_may_read_again_from_done() {
    let (_backend, home) = nowhere(None);
    let offer = WlSelection::new(None, vec!["text/plain".into()], home);
    let (tx, rx) = mpsc::channel();
    let again = offer.clone();
    // On a thread of its own, so a deadlock fails the test instead of hanging it.
    std::thread::spawn(move || {
        offer.read(
            "text/plain",
            ReadLimits::default(),
            Box::new(move |first| {
                again.read(
                    "text/plain",
                    ReadLimits::default(),
                    Box::new(move |second| {
                        let _ = tx.send((first, second));
                    }),
                );
            }),
        );
    });
    let got = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("no deadlock");
    assert_eq!(got, (Err(ReadError::Gone), Err(ReadError::Gone)));
}

#[test]
fn a_drag_offer_may_be_called_from_a_done_that_ran_at_once() {
    let (_backend, home) = nowhere(None);
    let offer = DragOffer::closed(vec!["text/plain".into()], home);
    let (tx, rx) = mpsc::channel();
    let again = offer.clone();
    std::thread::spawn(move || {
        offer.read("text/plain", ReadLimits::default(), move |first| {
            let finished = again.finish();
            let _ = tx.send((first, finished.is_err()));
        });
    });
    let got = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("no deadlock");
    assert_eq!(got, (Err(ReadError::Gone), true));
}

#[test]
fn offers_on_winits_display_refuse_other_threads() {
    let (_backend, home) = nowhere(Some(other_thread()));
    let selection = WlSelection::new(None, vec!["text/plain".into()], home.clone());
    let (tx, rx) = mpsc::channel();
    selection.read(
        "text/plain",
        ReadLimits::default(),
        Box::new(move |r| {
            let _ = tx.send(r);
        }),
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)),
        Ok(Err(ReadError::OffUiThread))
    );
    // Each refusal names the thread, not just any refusal: the offer is closed too, which
    // refuses with another reason.
    let drag = DragOffer::closed(vec!["text/plain".into()], home);
    assert_eq!(drag.accept(Some("text/plain")), Err(Error::Refused(OFF_UI)));
    assert_eq!(
        drag.set_actions(&[Action::Copy], None),
        Err(Error::Refused(OFF_UI))
    );
    assert_eq!(drag.finish(), Err(Error::Refused(OFF_UI)));
    let (tx, rx) = mpsc::channel();
    drag.read("text/plain", ReadLimits::default(), move |r| {
        let _ = tx.send(r);
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)),
        Ok(Err(ReadError::OffUiThread))
    );
    // The UI thread (here, this one) may.
    let (_backend, mine) = nowhere(Some(std::thread::current().id()));
    let drag = DragOffer::closed(vec!["text/plain".into()], mine);
    assert_eq!(
        drag.accept(Some("text/plain")),
        Err(Error::Refused("the drag is gone"))
    );
}
