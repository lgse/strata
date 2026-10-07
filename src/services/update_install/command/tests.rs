// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn hung_verifier_is_terminated() {
    let result = run(
        Command::new("sh").args(["-c", "exec sleep 60"]),
        &InstallCancel::new(),
        Duration::from_millis(50),
    );
    assert!(matches!(result, Err(InstallStop::Failed(message)) if message.contains("timed out")));
}

#[test]
fn verifier_output_is_bounded() {
    let result = run(
        Command::new("sh").args(["-c", "exec yes"]),
        &InstallCancel::new(),
        Duration::from_secs(2),
    );
    assert!(
        matches!(result, Err(InstallStop::Failed(message)) if message.contains("too much output"))
    );
}

#[test]
fn running_verifier_can_be_cancelled() {
    let cancel = InstallCancel::new();
    let worker_cancel = cancel.clone();
    let worker = std::thread::spawn(move || {
        run(
            Command::new("sh").args(["-c", "exec sleep 60"]),
            &worker_cancel,
            Duration::from_secs(2),
        )
    });
    std::thread::sleep(Duration::from_millis(50));
    cancel.cancel();
    assert!(matches!(
        worker.join().expect("join verifier"),
        Err(InstallStop::Cancelled)
    ));
}
