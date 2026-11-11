use std::sync::Arc;
use std::time::Duration;

use super::*;

/// A waiter on another thread, and a way to see whether it has returned.
fn waiter(gate: &Arc<Gate>, before: u64) -> std::sync::mpsc::Receiver<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let gate = gate.clone();
    std::thread::spawn(move || {
        gate.wait(before);
        let _ = tx.send(());
    });
    rx
}

#[test]
fn stop_releases_a_reader_waiting_for_a_turn() {
    let gate = Arc::new(Gate::default());
    let done = waiter(&gate, gate.turns());
    assert!(
        done.recv_timeout(Duration::from_millis(50)).is_err(),
        "it waits"
    );
    gate.stop();
    assert!(
        done.recv_timeout(Duration::from_secs(5)).is_ok(),
        "stop released it"
    );
    assert!(gate.stopped());
}

#[test]
fn a_turn_releases_the_waiter() {
    let gate = Arc::new(Gate::default());
    let done = waiter(&gate, gate.turns());
    assert!(
        done.recv_timeout(Duration::from_millis(50)).is_err(),
        "it waits"
    );
    gate.turned();
    assert!(
        done.recv_timeout(Duration::from_secs(5)).is_ok(),
        "the turn released it"
    );
}

#[test]
fn a_turn_before_the_wait_is_not_lost() {
    let gate = Arc::new(Gate::default());
    let before = gate.turns();
    // The UI thread dispatched between the reader's wake and its wait.
    gate.turned();
    let done = waiter(&gate, before);
    assert!(done.recv_timeout(Duration::from_secs(5)).is_ok());
}
