// SPDX-License-Identifier: MIT

//! What a drop reads: with [`DropTarget::read_next`], one type at a time, as the target
//! chooses from what it read so far; and why a read failed, past the read's bound.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use rune::{Cx, El, Env, Runtime, col, el};

use super::super::hub::{InPort, Job, Poster};
use super::super::window::TestDrop;
use super::super::*;
use crate::transfer::ReadError;

const LISTED: [&str; 3] = ["a/first", "a/second", "a/third"];

struct R {
    dnd: Dnd<R>,
    dropped: Vec<DropData>,
}

/// Uses the first type it reads, and the second only when the first is no use.
fn target() -> DropTarget<R> {
    DropTarget::new(|s: &mut R, data, _| s.dropped.push(data))
        .mime_types(&LISTED)
        .read_next(|data| {
            let first = &data.failed;
            match (data.items.is_empty(), first.is_empty()) {
                (true, true) => Some(LISTED[0].to_owned()),
                (true, false) => Some(LISTED[1].to_owned()),
                // A type read already ends the reads rather than reading it again.
                (false, _) => Some(data.items[0].0.clone()),
            }
        })
}

fn view(s: &R, _: &mut Cx<R>) -> El {
    col()
        .w_full()
        .h_full()
        .child(s.dnd.drop_target(el().size(100.0, 60.0), "zone", target()))
}

/// Another client's side of the drag: answers each read at once from `answers`, and records
/// what was read, in order.
struct Port {
    answers: HashMap<&'static str, Result<Vec<u8>, ReadError>>,
    read: Arc<Mutex<Vec<String>>>,
}

impl InPort for Port {
    fn status(&self, _: Option<&str>, _: Option<DragAction>) {}

    fn read(&self, mime: &str, done: Box<dyn FnOnce(Result<Vec<u8>, ReadError>) + Send>) {
        self.read
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(mime.to_owned());
        done(
            self.answers
                .get(mime)
                .cloned()
                .unwrap_or(Err(ReadError::NotOffered)),
        );
    }

    fn dropped_action(&self, chosen: DragAction) -> DragAction {
        chosen
    }

    fn finish(&self, _: Option<DragAction>) {}
}

/// Drops a drag offering every listed type on the target, answering as `answers` says, runs
/// its reads to the end, and returns what was read and what the target heard.
fn drop_with(
    answers: HashMap<&'static str, Result<Vec<u8>, ReadError>>,
) -> (Vec<String>, Vec<DropData>) {
    let jobs: Arc<Mutex<Vec<Job>>> = Arc::default();
    let sink = jobs.clone();
    let poster: Poster = Arc::new(move |job| {
        sink.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(job);
    });
    let state = R {
        dnd: Dnd::new(),
        dropped: Vec::new(),
    };
    state.dnd.set_poster_for_test(poster);
    let mut rt = Runtime::new(state, view, (300.0, 200.0), Env::new(true, 1.0));
    rt.frame(0.0);
    let read = Arc::new(Mutex::new(Vec::new()));
    let port: Rc<dyn InPort> = Rc::new(Port {
        answers,
        read: read.clone(),
    });
    let drop = TestDrop {
        port,
        mimes: LISTED.iter().map(|&m| m.to_owned()).collect(),
        key: "zone".to_owned(),
    };
    let dropped = std::cell::Cell::new(Some(drop));
    rt.update_if(move |s, tx| {
        if let Some(drop) = dropped.take() {
            s.dnd.clone().drop_on_for_test(drop, s, tx);
        }
        false
    });
    while run_posted(&mut rt, &jobs) {}
    let read = read.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (read, std::mem::take(&mut rt.state.dropped))
}

/// Runs the jobs posted so far, as the window's update would; false when there were none.
fn run_posted(rt: &mut Runtime<R>, jobs: &Arc<Mutex<Vec<Job>>>) -> bool {
    let posted = std::mem::take(&mut *jobs.lock().unwrap_or_else(PoisonError::into_inner));
    let ran = !posted.is_empty();
    for job in posted {
        let job = std::cell::Cell::new(Some(job));
        rt.update_if(move |s, tx| {
            if let Some(job) = job.take() {
                s.dnd.clone().run_job_for_test(job, s, tx);
            }
            false
        });
    }
    ran
}

/// A target that reads one type at a time reads the next only while the last was no use,
/// never a type it has read, and hears why a read failed: a read past its bound is
/// [`ReadError::TooLarge`], which the target can report.
#[test]
fn a_drop_reads_one_type_at_a_time_as_the_target_chooses() {
    let second = (LISTED[1], Ok(b"second".to_vec()));
    let cases = [
        (
            "the first type is used",
            HashMap::from([(LISTED[0], Ok(b"first".to_vec())), second.clone()]),
            vec![LISTED[0]],
        ),
        (
            "the first type is too large",
            HashMap::from([(LISTED[0], Err(ReadError::TooLarge)), second]),
            vec![LISTED[0], LISTED[1]],
        ),
    ];
    for (what, answers, wanted) in cases {
        let expected_failure = answers[LISTED[0]].clone().err();
        let (read, dropped) = drop_with(answers);
        assert_eq!(read, wanted, "{what}: what was read");
        let [data] = dropped.as_slice() else {
            panic!("{what}: one drop heard, not {}", dropped.len());
        };
        let failed: Vec<_> = data.failed.iter().map(|(_, e)| e.clone()).collect();
        assert_eq!(
            failed,
            expected_failure.into_iter().collect::<Vec<_>>(),
            "{what}"
        );
        assert_eq!(data.items.len(), 1, "{what}: one type read, the one used");
    }
}
