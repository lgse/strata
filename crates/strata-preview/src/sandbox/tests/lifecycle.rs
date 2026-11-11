// SPDX-License-Identifier: MIT

use std::{
    fs,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use crate::sandbox::{
    Cancellation, MAX_RASTER_INPUT_BYTES, MediaPreviewBackend, ParseOperation, PrivateOutput,
    parse, resolve_renderer_executable, spawn_renderer, wait_for_renderer,
};

#[test]
fn renderer_uses_a_private_snapshot_after_the_original_executable_is_replaced() {
    let directory = PrivateOutput::create().expect("create private output");
    let running = directory.path().join("running-strata");
    fs::write(&running, b"running executable").expect("write running executable");
    let replaced = directory.path().join("replaced-strata");

    let executable = resolve_renderer_executable(&replaced, &running, directory.path())
        .expect("snapshot running executable");

    assert_eq!(executable, directory.path().join("strata-preview-helper"));
    assert_eq!(
        fs::read(executable).expect("read snapshot"),
        b"running executable"
    );
}

#[test]
fn renderer_uses_the_original_executable_while_it_is_available() {
    let directory = PrivateOutput::create().expect("create private output");
    let current = directory.path().join("strata");
    fs::write(&current, b"current executable").expect("write current executable");

    let executable = resolve_renderer_executable(
        &current,
        &directory.path().join("unused-running-strata"),
        directory.path(),
    )
    .expect("resolve current executable");

    assert_eq!(executable, current);
    assert!(!directory.path().join("strata-preview-helper").exists());
}

#[test]
fn cancelled_requests_fail_without_starting_a_renderer() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let error = parse(
        Path::new("does-not-need-to-exist"),
        ParseOperation::PreviewImage,
        0,
        MediaPreviewBackend::Software,
        &cancellation,
    )
    .err()
    .expect("cancelled parse must fail");

    assert_eq!(error, "Preview cancelled");
}

#[test]
fn archive_parse_infra_failures_report_renderer_failure() {
    let error = parse(
        Path::new("does-not-need-to-exist.zip"),
        ParseOperation::ArchiveList {
            format: crate::model::ArchiveFormat::Zip,
            password: None,
        },
        0,
        MediaPreviewBackend::Software,
        &Cancellation::default(),
    )
    .err()
    .expect("missing archive input must fail");
    assert_eq!(
        error,
        crate::sandbox::archive_listing::ARCHIVE_PREVIEW_FAILED_MESSAGE
    );

    let error = parse(
        Path::new("does-not-need-to-exist.png"),
        ParseOperation::PreviewImage,
        0,
        MediaPreviewBackend::Software,
        &Cancellation::default(),
    )
    .err()
    .expect("missing image input must fail");
    assert_eq!(
        error, "No such file or folder",
        "non-archive operations keep their own message"
    );
}

#[test]
fn rejects_oversized_raster_inputs_before_starting_a_renderer() {
    let directory = PrivateOutput::create().expect("create temporary directory");
    let input = directory.path().join("oversized.png");
    fs::File::create(&input)
        .expect("create sparse input")
        .set_len(MAX_RASTER_INPUT_BYTES + 1)
        .expect("size sparse input");

    let error = parse(
        &input,
        ParseOperation::ThumbnailImage,
        64,
        MediaPreviewBackend::Software,
        &Cancellation::default(),
    )
    .err()
    .expect("oversized raster input must fail");

    assert_eq!(error, "Preview input exceeds the supported size limit");
    assert_eq!(ParseOperation::ThumbnailVideo.input_size_limit(), None);
}

#[test]
fn running_thumbnail_process_trees_are_stopped_on_timeout_and_cancellation() {
    let directory = PrivateOutput::create().expect("create process marker directory");
    let timeout_marker = directory.path().join("timeout-marker");
    let mut timeout_command = process_tree_command(&timeout_marker);
    let mut timed_out = spawn_renderer(&mut timeout_command).expect("start timeout renderer");
    wait_for_process_marker(&timeout_marker);
    let started = Instant::now();
    let error = wait_for_renderer(
        &mut timed_out,
        &Cancellation::default(),
        Duration::from_millis(40),
    )
    .expect_err("running renderer must time out");
    assert_eq!(error, "The preview renderer timed out");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(timed_out.try_wait().expect("inspect renderer").is_some());
    assert_process_marker_stopped(&timeout_marker);

    let cancellation_marker = directory.path().join("cancellation-marker");
    let mut cancellation_command = process_tree_command(&cancellation_marker);
    let mut cancelled =
        spawn_renderer(&mut cancellation_command).expect("start cancellable renderer");
    wait_for_process_marker(&cancellation_marker);
    let cancellation = Cancellation::default();
    let cancellation_request = cancellation.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(40));
        cancellation_request.cancel();
    });
    let error = wait_for_renderer(&mut cancelled, &cancellation, Duration::from_secs(5))
        .expect_err("running renderer must be cancelled");
    canceller.join().expect("join canceller");

    assert_eq!(error, "Preview cancelled");
    assert!(cancelled.try_wait().expect("inspect renderer").is_some());
    assert_process_marker_stopped(&cancellation_marker);
}

fn process_tree_command(marker: &Path) -> Command {
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            "sh -c 'while :; do printf x >> \"$1\"; sleep 0.01; done' writer \"$1\" & wait",
            "thumbnail-provider",
        ])
        .arg(marker);
    command
}

fn wait_for_process_marker(marker: &Path) {
    for _ in 0..50 {
        if fs::metadata(marker).is_ok_and(|metadata| metadata.len() > 0) {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("thumbnail provider descendant did not start");
}

fn assert_process_marker_stopped(marker: &Path) {
    let length = fs::metadata(marker).expect("inspect process marker").len();
    thread::sleep(Duration::from_millis(80));
    assert_eq!(
        fs::metadata(marker)
            .expect("reinspect process marker")
            .len(),
        length,
        "renderer descendant survived termination"
    );
}

fn sandbox_whose_init_outlives_it(ends: &str) -> Command {
    let script = format!(
        r#"setsid sleep 0.4 </dev/null >/dev/null 2>&1 &
init=$!
until [ "$(cut -d' ' -f6 /proc/$init/stat 2>/dev/null)" = "$init" ]; do :; done
printf '{{\n    "child-pid": %s,\n    "net-namespace": 1\n}}\n' "$init" >&2
{ends}"#
    );
    let mut command = Command::new("sh");
    command.args(["-c", &script]);
    command
}

#[test]
fn a_sandbox_is_waited_for_until_the_init_of_its_namespace_has_exited() {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    for (row, ends, cancelled) in [
        ("an exit of its own", "exec 2>&-; exit 0", false),
        ("a kill", "exec 2>&-; wait", true),
    ] {
        let mut renderer =
            spawn_renderer(&mut sandbox_whose_init_outlives_it(ends)).expect("start the sandbox");
        let init = renderer.init.take().expect("the reported namespace init");
        let watched = rustix::io::dup(&init).expect("watch the namespace init");
        renderer.init = Some(init);
        let cancellation = Cancellation::default();
        if cancelled {
            cancellation.cancel();
        }
        let started = Instant::now();

        let _ = wait_for_renderer(&mut renderer, &cancellation, Duration::from_secs(5));

        let mut fds = [PollFd::new(&watched, PollFlags::IN)];
        let gone = poll(&mut fds, Some(&Timespec::default())).expect("poll the namespace init");
        assert_eq!(gone, 1, "{row}: the namespace init still runs");
        assert!(started.elapsed() < Duration::from_secs(2), "{row}");
    }
}
