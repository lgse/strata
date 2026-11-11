use std::cell::RefCell;
use std::rc::Rc;

use rune::Tx;

use super::{Clipboard, Pending, run};
use crate::clipboard::Content;
use crate::clipboard::api::{Change, CopyId, Data, Error, Limits, Lost, ReadError, Selection};
use crate::clipboard::hub::{Job, Out, Sink};

#[derive(Default)]
struct S {
    heard: Vec<Change>,
    read: Vec<Result<Data, ReadError>>,
}

fn lost(copy: u64) -> Change {
    Change::Lost {
        selection: Selection::Clipboard,
        copy: CopyId(copy),
        why: Lost::Taken,
    }
}

#[test]
fn queued_changes_reach_on_change_in_order_once_each() {
    let clip = Clipboard::<S>::new().on_change(|s, c, _| s.heard.push(c.clone()));
    clip.0.queue(lost(1));
    clip.0.queue(Change::Emptied {
        selection: Selection::Clipboard,
    });
    let mut s = S::default();
    assert!(run::<S>(clip.0.id, Job::Pump, &mut s, &mut Tx::default()));
    assert_eq!(
        s.heard,
        [
            lost(1),
            Change::Emptied {
                selection: Selection::Clipboard
            }
        ]
    );
    assert!(!run::<S>(clip.0.id, Job::Pump, &mut s, &mut Tx::default()));
    assert_eq!(s.heard.len(), 2);
}

#[test]
fn a_change_heard_inside_on_change_waits_for_the_next_job() {
    // A callback that copies can make the platform report again; that goes to the queue,
    // never into the running callback.
    let me: Rc<RefCell<Option<Clipboard<S>>>> = Rc::default();
    let inner = me.clone();
    let clip = Clipboard::<S>::new().on_change(move |s, c, _| {
        s.heard.push(c.clone());
        if let Some(clip) = inner.borrow().as_ref()
            && s.heard.len() == 1
        {
            clip.0.queue(lost(2));
        }
    });
    *me.borrow_mut() = Some(clip.clone());
    clip.0.queue(lost(1));
    let mut s = S::default();
    run::<S>(clip.0.id, Job::Pump, &mut s, &mut Tx::default());
    assert_eq!(
        s.heard,
        [lost(1), lost(2)],
        "drained in the same job, after the first"
    );
}

#[test]
fn a_reads_answer_runs_its_callback_once() {
    let clip = Clipboard::<S>::new();
    clip.0.reads.borrow_mut().insert(
        7,
        Pending::Data(Box::new(|s: &mut S, r, _: &mut Tx| s.read.push(r))),
    );
    let data = Data {
        mime: "text/plain".into(),
        bytes: b"hi".to_vec(),
    };
    let mut s = S::default();
    let answer = |d: &Data| Job::Read {
        id: 7,
        result: Ok(Out::Data(d.clone())),
    };
    assert!(run::<S>(
        clip.0.id,
        answer(&data),
        &mut s,
        &mut Tx::default()
    ));
    assert!(!run::<S>(
        clip.0.id,
        answer(&data),
        &mut s,
        &mut Tx::default()
    ));
    assert_eq!(s.read, [Ok(data)]);
}

#[test]
fn a_read_before_attach_is_refused_and_its_callback_never_runs() {
    let clip = Clipboard::<S>::new();
    let r = clip.read(
        Selection::Clipboard,
        &["text/plain"],
        Limits::default(),
        |_, _, _| panic!("never runs"),
    );
    assert_eq!(r, Err(Error::NotAttached));
    assert!(clip.0.reads.borrow().is_empty());
}

#[test]
fn a_dropped_handle_hears_nothing() {
    let clip = Clipboard::<S>::new();
    let id = clip.0.id;
    drop(clip);
    let mut s = S::default();
    assert!(!run::<S>(id, Job::Pump, &mut s, &mut Tx::default()));
}

#[test]
fn a_copy_offering_a_target_that_names_a_request_is_refused() {
    let clip = Clipboard::<S>::new();
    for name in ["SAVE_TARGETS", "TARGETS", "DELETE"] {
        let c = Content::new().text("a").with(name, "x");
        assert!(c.mimes().all(|m| m != name), "{name} isn't offered");
        assert!(
            matches!(clip.copy(Selection::Clipboard, c), Err(Error::Refused(_))),
            "{name}"
        );
    }
    assert_eq!(
        clip.copy(Selection::Clipboard, Content::new().with("PIXMAP", "x")),
        Err(Error::NotAttached),
        "PIXMAP is data"
    );
}
