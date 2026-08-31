// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use super::{
    Cancellation, MEDIA_WALL_TIME_LIMIT, ParseOperation, WALL_TIME_LIMIT, parse, sandbox_command,
};

#[test]
fn sandbox_exposes_only_runtime_input_and_private_output() {
    let command = sandbox_command(
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Downloads/untrusted.pdf"),
        Path::new("/tmp/private-output"),
        ParseOperation::PreviewPdf,
        2,
    );
    let arguments: Vec<_> = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    let joined = arguments.join(" ");

    assert!(joined.contains("--unshare-all"));
    assert!(joined.contains("--clearenv"));
    assert!(joined.contains("--ro-bind /home/alice/Downloads/untrusted.pdf /input"));
    assert!(joined.contains("--bind /tmp/private-output /output"));
    assert!(joined.contains("--as=2147483648"));
    assert!(joined.contains("--cpu=10"));
    assert!(joined.contains("--fsize=33554432"));
    // RLIMIT_NPROC counts every process owned by the host user, not just the
    // sandbox, and can prevent legitimate media decoders from starting.
    assert!(!joined.contains("--nproc"));
    assert!(!joined.contains("--ro-bind /home /home"));
    assert!(!joined.contains("--share-net"));
}

#[test]
fn media_previews_use_a_longer_wall_timeout_instead_of_a_cpu_limit() {
    let operation = ParseOperation::PreviewMedia;
    let command = sandbox_command(
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Videos/untrusted.mkv"),
        Path::new("/tmp/private-output"),
        operation,
        0,
    );

    assert_eq!(operation.wall_time_limit(), MEDIA_WALL_TIME_LIMIT);
    assert!(MEDIA_WALL_TIME_LIMIT > WALL_TIME_LIMIT);
    assert!(
        command
            .get_args()
            .all(|argument| !argument.to_string_lossy().starts_with("--cpu="))
    );
}

#[test]
fn cancelled_requests_fail_without_starting_a_renderer() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let error = parse(
        Path::new("does-not-need-to-exist"),
        ParseOperation::PreviewImage,
        0,
        &cancellation,
    )
    .err()
    .expect("cancelled parse must fail");

    assert_eq!(error, "Preview cancelled");
}
