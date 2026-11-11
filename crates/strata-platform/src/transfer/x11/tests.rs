use std::sync::mpsc;
use std::time::Duration;

use super::*;

fn read(selection: Selection, tx: mpsc::Sender<Result<Vec<u8>, ReadError>>) -> Cmd {
    Cmd::Read {
        selection,
        owner: 1,
        mime: "text/plain".into(),
        limits: ReadLimits::default(),
        done: Reply::new(Box::new(move |r| {
            let _ = tx.send(r);
        })),
    }
}

#[test]
fn reads_still_queued_when_the_actor_stops_answer_shutdown() {
    let (cmds, rx) = mpsc::channel();
    let (tx, answers) = mpsc::channel();
    for selection in [Selection::Clipboard, Selection::Primary] {
        cmds.send(read(selection, tx.clone())).expect("queued");
    }
    refuse_all(&rx);
    for _ in 0..2 {
        assert_eq!(
            answers.recv_timeout(Duration::from_secs(5)),
            Ok(Err(ReadError::Shutdown))
        );
    }
}

#[test]
fn a_read_sent_to_a_stopped_actor_answers_shutdown() {
    let (cmds, rx) = mpsc::channel();
    let (tx, answers) = mpsc::channel();
    // The actor took what was queued and returned: its receiver is gone.
    drop(rx);
    assert!(cmds.send(read(Selection::Clipboard, tx)).is_err());
    assert_eq!(
        answers.recv_timeout(Duration::from_secs(5)),
        Ok(Err(ReadError::Shutdown))
    );
}

#[test]
fn a_reply_answers_once() {
    let (tx, answers) = mpsc::channel();
    let reply = Reply::new(Box::new(move |r| {
        let _ = tx.send(r);
    }));
    reply.send(Ok(b"data".to_vec()));
    assert_eq!(
        answers.recv_timeout(Duration::from_secs(5)),
        Ok(Ok(b"data".to_vec()))
    );
    assert!(
        answers.recv_timeout(Duration::from_millis(50)).is_err(),
        "no second answer"
    );
}
