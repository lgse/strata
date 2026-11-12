// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{JobRequest, JobService, JobStatus};

#[test]
fn setup_failure_finishes_the_job_and_releases_its_slot() {
    let fixture = Fixture::new();
    let action = fixture.action(
        "setup",
        &command("true", vec![]),
        ExecutionMode::WholeSelection,
    );
    assert!(action.is_available());
    let service = JobService::new(fixture.runner.clone());
    let inputs = vec![PathBuf::from(format!("/{}", "x".repeat(1024))); 4096];
    let failed = service
        .enqueue(JobRequest {
            action: action.clone(),
            inputs,
            parent: fixture.path().to_owned(),
            source: InvocationSource::Selection,
        })
        .expect("queue large selection");
    let mut queued = Vec::new();
    for _ in 0..2 {
        queued.push(
            service
                .enqueue(JobRequest {
                    action: action.clone(),
                    inputs: vec![fixture.path().join("input")],
                    parent: fixture.path().to_owned(),
                    source: InvocationSource::Selection,
                })
                .expect("queue normal selection"),
        );
    }
    let deadline = Instant::now() + EVENT_TIMEOUT;
    while service.finished_count() < 3 && Instant::now() < deadline {
        service.pump();
        thread::sleep(Duration::from_millis(10));
    }
    let result = service.snapshot_of(failed).expect("failed job retained");
    assert_eq!(result.status, JobStatus::Failed);
    assert!(
        result
            .message
            .as_deref()
            .is_some_and(|message| message.contains("Too many selected paths"))
    );
    for id in queued {
        assert_eq!(
            service.snapshot_of(id).expect("job retained").status,
            JobStatus::Succeeded
        );
    }
    assert_eq!(service.running_count(), 0);
    assert_eq!(
        fs::read_dir(fixture.path().join("runtime/actions"))
            .expect("scratch")
            .count(),
        0
    );
}

#[test]
fn newline_free_output_is_bounded_and_preserves_its_tail() {
    let fixture = Fixture::new();
    let action = fixture.action(
        "noisy",
        &script(
            ActionRuntime::Python,
            "main.py",
            "import sys\nsys.stdout.write('x' * 262144 + 'END')\nsys.stderr.write('error tail')\n",
        ),
        ExecutionMode::WholeSelection,
    );
    assert!(action.is_available());
    let (code, _, log) = run(&fixture.runner, request(action, &[], fixture.path())).ended();
    assert_eq!(code, Some(0));
    assert!(
        log.len() <= LIVE_LOG_BYTES + "…\n".len(),
        "{} bytes",
        log.len()
    );
    assert!(log.contains("END"), "tail missing");
    assert!(log.contains("error tail"));
}

#[test]
fn an_exited_leader_does_not_wait_for_descendant_output_handles() {
    let fixture = Fixture::new();
    let pid_file = fixture.path().join("descendant.pid");
    let source = format!(
        "import subprocess, sys\nfrom pathlib import Path\nchild = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\nPath({}).write_text(str(child.pid))\nprint('leader finished', flush=True)\n",
        serde_json::to_string(&pid_file).expect("pid path")
    );
    let action = fixture.action(
        "descendant",
        &script(ActionRuntime::Python, "main.py", &source),
        ExecutionMode::WholeSelection,
    );
    assert!(action.is_available());
    let started = Instant::now();
    let outcome = run(&fixture.runner, request(action, &[], fixture.path()));
    let elapsed = started.elapsed();
    if let Ok(pid) = fs::read_to_string(pid_file) {
        let pid = Pid::from_raw(pid.parse().expect("descendant pid")).expect("positive pid");
        let _stopped = kill_process(pid, Signal::KILL);
    }
    let (code, _, log) = outcome.ended();
    assert_eq!(code, Some(0));
    assert!(log.contains("leader finished"));
    assert!(
        elapsed < Duration::from_secs(3),
        "completion took {elapsed:?}"
    );
    assert_eq!(
        fs::read_dir(fixture.path().join("runtime/actions"))
            .expect("scratch")
            .count(),
        0
    );
}

#[test]
fn a_missing_working_directory_fails_instead_of_using_stratas_cwd() {
    let fixture = Fixture::new();
    let action = fixture.action(
        "cwd",
        &command("true", vec![]),
        ExecutionMode::WholeSelection,
    );
    assert!(action.is_available());
    let outcome = run(
        &fixture.runner,
        request(action, &[], &fixture.path().join("gone")),
    );
    assert!(matches!(
        outcome.events.last(),
        Some(ActionRunEvent::Failed(_))
    ));
}
