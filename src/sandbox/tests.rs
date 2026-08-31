// SPDX-License-Identifier: GPL-3.0-or-later

use std::{fs, path::Path};

use super::{
    Cancellation, MEDIA_WALL_TIME_LIMIT, ParseOperation, PrivateOutput, WALL_TIME_LIMIT,
    gpu_devices, parse, sandbox_command, valid_output,
};

fn limit_from(arguments: &[String], flag: &str) -> u64 {
    arguments
        .iter()
        .find_map(|argument| argument.strip_prefix(flag))
        .unwrap_or_else(|| panic!("the sandbox must pass {flag}"))
        .parse()
        .expect("resource limits must be numeric")
}

#[test]
fn file_size_limit_holds_a_full_resolution_decoded_frame() {
    // gdk-pixbuf decodes through glycin, which sizes a memfd to
    // `width * height * channels` before scaling down. RLIMIT_FSIZE covers that
    // buffer too, and exceeding it kills the loader with SIGXFSZ rather than
    // surfacing an error, so the limit has to clear the largest frame we expect.
    const LARGEST_SUPPORTED_PIXELS: u64 = 50_000_000;
    const RGBA_CHANNELS: u64 = 4;

    let command = sandbox_command(
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Pictures/photo.jpg"),
        Path::new("/tmp/private-output"),
        ParseOperation::ThumbnailImage,
        256,
        &[],
    );
    let arguments: Vec<_> = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();

    let file_size = limit_from(&arguments, "--fsize=");
    let address_space = limit_from(&arguments, "--as=");

    assert!(file_size >= LARGEST_SUPPORTED_PIXELS * RGBA_CHANNELS);
    assert!(file_size < address_space);
}

#[test]
fn sandbox_exposes_only_runtime_input_and_private_output() {
    let command = sandbox_command(
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Downloads/untrusted.pdf"),
        Path::new("/tmp/private-output"),
        ParseOperation::PreviewPdf,
        2,
        &[],
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
    assert!(joined.contains("--fsize=536870912"));
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
        &[],
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
fn discovers_only_supported_gpu_devices_in_stable_order() {
    let root = PrivateOutput::create().expect("create temporary device tree");
    let dev = root.path().join("dev");
    fs::create_dir_all(dev.join("dri")).expect("create DRI directory");
    for path in [
        "dri/renderD129",
        "dri/card0",
        "dri/renderD128",
        "dri/renderD",
        "nvidia1",
        "nvidiactl",
        "nvidia0",
        "nvidia-uvm",
        "nvidia-modeset",
        "unrelated",
    ] {
        fs::write(dev.join(path), []).expect("create device entry");
    }

    assert_eq!(
        gpu_devices(&dev),
        [
            dev.join("dri/renderD128"),
            dev.join("dri/renderD129"),
            dev.join("nvidia0"),
            dev.join("nvidia1"),
            dev.join("nvidiactl"),
        ]
    );
}

#[test]
fn media_sandbox_exposes_only_supplied_gpu_devices_and_sysfs() {
    let devices = [
        "/dev/dri/renderD128".into(),
        "/dev/nvidia0".into(),
        "/dev/nvidiactl".into(),
    ];
    let command = sandbox_command(
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Videos/untrusted.mkv"),
        Path::new("/tmp/private-output"),
        ParseOperation::PreviewMedia,
        0,
        &devices,
    );
    let joined = command
        .get_args()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");

    for device in &devices {
        let device = device.to_string_lossy();
        assert!(joined.contains(&format!("--dev-bind-try {device} {device}")));
    }
    assert!(joined.contains("--ro-bind /sys /sys"));
    assert!(!joined.contains("--cpu=10"));
    assert!(joined.contains("/output/result.media"));
}

#[test]
fn non_media_sandboxes_never_expose_gpu_devices_or_sysfs() {
    let command = sandbox_command(
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Videos/untrusted.mkv"),
        Path::new("/tmp/private-output"),
        ParseOperation::ThumbnailVideo,
        128,
        &["/dev/dri/renderD128".into(), "/dev/nvidia0".into()],
    );
    let joined = command
        .get_args()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(!joined.contains("--dev-bind-try"));
    assert!(!joined.contains("/sys"));
    assert!(joined.contains("--cpu=10"));
}

#[test]
fn accepts_only_png_webm_or_mp4_output_signatures() {
    assert!(valid_output(
        ParseOperation::PreviewImage,
        b"\x89PNG\r\n\x1a\ncontent"
    ));
    assert!(valid_output(
        ParseOperation::PreviewMedia,
        b"\x1a\x45\xdf\xa3content"
    ));
    assert!(valid_output(
        ParseOperation::PreviewMedia,
        b"\0\0\0\x18ftypisom"
    ));
    assert!(!valid_output(ParseOperation::PreviewMedia, b""));
    assert!(!valid_output(
        ParseOperation::PreviewMedia,
        b"unrelated data"
    ));
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
