// SPDX-License-Identifier: MIT

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

use crate::services::MediaPreviewSize;

const MEDIA_PREVIEW: super::ParseOperation =
    super::ParseOperation::PreviewMedia(MediaPreviewSize {
        width: 640,
        height: 800,
    });
const PDF_PREVIEW: super::ParseOperation = super::ParseOperation::PreviewPdf(PdfRenderSize {
    width: 640,
    height: 800,
});

use super::{
    Cancellation, MAX_RASTER_INPUT_BYTES, MediaPreviewBackend, OutputError, ParseOperation,
    PdfRenderSize, PrivateOutput, archive_output_error, gpu_devices, parse,
    polaris_gpu_available_at, resolve_renderer_executable, sandbox_command, sandbox_input_path,
    spawn_renderer, stage_secret_anon, valid_output, wait_for_renderer,
};

#[test]
fn renderer_outputs_are_never_read_through_symlinks() {
    let dir = tempfile::tempdir().expect("scratch directory");
    let real = dir.path().join("host-file");
    fs::write(&real, b"2 5").expect("host file");
    for name in ["result.png", "result.meta"] {
        let link = dir.path().join(name);
        std::os::unix::fs::symlink(&real, &link).expect("planted symlink");
        assert!(super::read_private_output(&link, 256).is_err());
        assert_eq!(super::read_metadata(&link), (0, 0));
    }
    assert_eq!(super::read_metadata(&real), (2, 5));
    assert_eq!(fs::read(&real).expect("unchanged host file"), b"2 5");
}

#[test]
fn renderer_outputs_require_nonempty_bounded_regular_files() {
    let dir = tempfile::tempdir().expect("scratch directory");
    let output = dir.path().join("result.png");
    assert!(super::read_private_output(&output, 4).is_err());
    assert!(super::read_private_output(dir.path(), 4).is_err());
    for bytes in [b"".as_slice(), b"data", b"extra"] {
        fs::write(&output, bytes).expect("renderer output");
        let result = super::read_private_output(&output, 4);
        if bytes.len() == 4 {
            assert_eq!(result.expect("exact size limit is allowed"), bytes);
        } else {
            assert!(result.is_err());
        }
    }
}

#[test]
fn renderer_output_fifo_is_rejected_without_blocking() {
    let dir = tempfile::tempdir().expect("scratch directory");
    let fifo = dir.path().join("result.png");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .expect("planted FIFO");
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = thread::spawn(move || {
        let _sent = sender.send(super::read_private_output(&fifo, 256));
    });
    let result = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("opening a renderer FIFO must not wait for a writer");
    assert!(result.is_err());
    reader.join().expect("output reader");
}

#[test]
fn renderer_metadata_is_bounded_and_requires_utf8() {
    let dir = tempfile::tempdir().expect("scratch directory");
    let output = dir.path().join("result.meta");
    assert_eq!(super::read_metadata(&output), (0, 0));
    assert_eq!(super::read_metadata(dir.path()), (0, 0));
    for bytes in [b"".as_slice(), b"2 5 \xff"] {
        fs::write(&output, bytes).expect("invalid metadata");
        assert_eq!(super::read_metadata(&output), (0, 0));
    }
    let bounded = format!("2 5{}", " ".repeat(253));
    fs::write(&output, &bounded).expect("metadata at size limit");
    assert_eq!(super::read_metadata(&output), (2, 5));
    fs::write(&output, format!("{bounded} ")).expect("oversized metadata");
    assert_eq!(super::read_metadata(&output), (0, 0));
}

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
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Pictures/photo.jpg"),
        Path::new("/tmp/private-output"),
        ParseOperation::ThumbnailImage,
        256,
        MediaPreviewBackend::Software,
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

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
    data.extend_from_slice(&13u32.to_be_bytes());
    data.extend_from_slice(b"IHDR");
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data
}

#[test]
fn sandbox_command_starts_absolute_bubblewrap() {
    let command = sandbox_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Pictures/photo.jpg"),
        Path::new("/tmp/private-output"),
        ParseOperation::ThumbnailImage,
        256,
        MediaPreviewBackend::Software,
        &[],
    );
    assert_eq!(command.get_program(), Path::new("/usr/bin/bwrap"));
    let joined = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(joined.contains("--unshare-all"));
    assert!(joined.contains("--clearenv"));
    assert!(joined.contains("--setenv PATH /usr/bin"));
}

#[test]
fn sandbox_exposes_only_runtime_input_and_private_output() {
    let command = sandbox_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Downloads/untrusted.pdf"),
        Path::new("/tmp/private-output"),
        PDF_PREVIEW.clone(),
        2,
        MediaPreviewBackend::Software,
        &[],
    );
    let arguments: Vec<_> = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    let joined = arguments.join(" ");

    assert!(joined.contains("--unshare-all"));
    assert!(joined.contains("--clearenv"));
    assert!(joined.contains("--ro-bind /home/alice/Downloads/untrusted.pdf /input.pdf"));
    assert!(joined.contains("--bind /tmp/private-output /output"));
    assert!(joined.contains("--as=2147483648"));
    assert!(joined.contains("--cpu=10"));
    assert!(joined.contains("--fsize=536870912"));
    assert!(joined.contains("--setenv MALLOC_ARENA_MAX 1"));
    assert!(joined.contains("--size 536870912 --tmpfs /tmp"));
    assert!(joined.contains("preview-pdf /input.pdf /output/result.png 2:640x800 software"));
    // RLIMIT_NPROC counts every process owned by the host user, not just the
    // sandbox, and can prevent legitimate media decoders from starting.
    assert!(!joined.contains("--nproc"));
    assert!(!joined.contains("--ro-bind /home /home"));
    assert!(!joined.contains("--share-net"));
}

#[test]
fn workbook_parser_uses_resource_limited_sandbox_and_validated_output() {
    let command = sandbox_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/app/strata"),
        Path::new("/fixtures/book.xlsx"),
        Path::new("/private-output"),
        ParseOperation::PreviewWorkbook,
        0,
        MediaPreviewBackend::Software,
        &[],
    );
    let arguments = command
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(arguments.contains("--unshare-all --die-with-parent --new-session --clearenv"));
    assert!(arguments.contains("--as=2147483648 --cpu=10"));
    assert!(arguments.contains("preview-workbook /input.xlsx /output/result.json"));
    assert!(!arguments.contains("--share-net"));
    assert!(!arguments.contains("--dev-bind"));
    assert!(valid_output(
        ParseOperation::PreviewWorkbook,
        br#"{"rows":[["a"],["1"]],"truncated":false}"#
    ));
    assert!(!valid_output(ParseOperation::PreviewWorkbook, b"invalid"));
}

#[test]
fn word_document_parser_uses_resource_limited_sandbox_and_validated_output() {
    let command = sandbox_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/app/strata"),
        Path::new("/fixtures/report.docx"),
        Path::new("/private-output"),
        ParseOperation::PreviewDocument,
        0,
        MediaPreviewBackend::Software,
        &[],
    );
    let arguments = command
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(arguments.contains("--unshare-all --die-with-parent --new-session --clearenv"));
    assert!(arguments.contains("--as=2147483648 --cpu=10"));
    assert!(arguments.contains("preview-document /input.docx /output/result.json"));
    assert!(!arguments.contains("--share-net"));
    assert!(valid_output(
        ParseOperation::PreviewDocument,
        br#"{"html":"<p>a</p>","truncated":false}"#
    ));
    assert!(!valid_output(
        ParseOperation::PreviewDocument,
        br#"{"rows":[["a"]],"truncated":false}"#
    ));
}

#[test]
fn metadata_probe_retains_software_sandbox_limits_and_narrow_runtime_access() {
    let command = sandbox_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Videos/untrusted.mkv"),
        Path::new("/tmp/private-output"),
        ParseOperation::MediaMetadata,
        0,
        MediaPreviewBackend::Software,
        &[],
    );
    let arguments: Vec<_> = command
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect();
    let joined = arguments.join(" ");
    assert!(joined.contains("--unshare-all --die-with-parent --new-session --clearenv"));
    assert!(joined.contains("--ro-bind /home/alice/Videos/untrusted.mkv /input.mkv"));
    assert!(joined.contains("--as=2147483648 --cpu=10"));
    assert!(joined.contains("media-metadata /input.mkv /output/result.json 0 software"));
    assert!(!joined.contains("--dev-bind"));
    assert!(!joined.contains("--share-net"));
    assert!(
        !arguments
            .iter()
            .any(|argument| argument == "/etc/alternatives"
                || argument == "/home"
                || argument == "/sys")
    );
    for architecture in ["x86_64-linux-gnu", "aarch64-linux-gnu"] {
        for library in ["libblas.so.3", "liblapack.so.3"] {
            let path = format!("/etc/alternatives/{library}-{architecture}");
            assert!(joined.contains(&format!("--ro-bind-try {path} {path}")));
        }
    }
}

#[test]
fn media_previews_use_bounded_streaming_instead_of_driver_wide_resource_limits() {
    let operation = MEDIA_PREVIEW.clone();
    let command = sandbox_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Videos/untrusted.mkv"),
        Path::new("/tmp/private-output"),
        operation.clone(),
        0,
        MediaPreviewBackend::Automatic,
        &[],
    );

    let joined = command
        .get_args()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!joined.contains("/usr/bin/prlimit"));
    assert!(!joined.contains("--as="));
    assert!(!joined.contains("--cpu="));
    assert!(!joined.contains("--fsize="));
    assert!(!joined.contains("MALLOC_ARENA_MAX"));
    assert!(joined.contains("--size 536870912 --tmpfs /tmp"));
    assert!(!joined.contains("--bind /tmp/private-output /output"));
    assert!(joined.contains("preview-media /input.mkv /dev/stdout"));
}

#[test]
fn sandbox_input_keeps_a_safe_filename_extension() {
    assert_eq!(
        sandbox_input_path(Path::new("/photos/DSC01986.ARW")),
        "/input.ARW"
    );
    assert_eq!(sandbox_input_path(Path::new("/tmp/no-extension")), "/input");
    assert_eq!(sandbox_input_path(Path::new("/tmp/photo.ar-w")), "/input");
    assert_eq!(
        sandbox_input_path(Path::new("/tmp/toolongextension.abcdefghij")),
        "/input"
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
        gpu_devices(&dev, MediaPreviewBackend::Automatic),
        [
            dev.join("dri/renderD128"),
            dev.join("dri/renderD129"),
            dev.join("nvidia0"),
            dev.join("nvidia1"),
            dev.join("nvidiactl"),
        ]
    );
}

fn create_render_node(dev: &Path, drm: &Path, name: &str, pci_ids: Option<(u16, u16)>) -> PathBuf {
    fs::create_dir_all(dev.join("dri")).expect("create DRI directory");
    let node = dev.join("dri").join(name);
    fs::write(&node, []).expect("create render node");
    if let Some((vendor, device)) = pci_ids {
        let metadata = drm.join(name).join("device");
        fs::create_dir_all(&metadata).expect("create PCI metadata");
        fs::write(metadata.join("vendor"), format!("0x{vendor:04x}\n")).expect("write PCI vendor");
        fs::write(metadata.join("device"), format!("0x{device:04x}\n")).expect("write PCI device");
    }
    node
}

#[test]
fn every_polaris_range_uses_the_safe_default_but_remains_available_for_opt_in() {
    let root = PrivateOutput::create().expect("create temporary device tree");
    let dev = root.path().join("dev");
    let drm = root.path().join("sys/class/drm");
    let blocked = [0x67c0, 0x67df, 0x67e0, 0x67ff, 0x6980, 0x699f];
    for (index, device) in blocked.into_iter().enumerate() {
        create_render_node(
            &dev,
            &drm,
            &format!("renderD{}", 128 + index),
            Some((0x1002, device)),
        );
    }

    let devices = gpu_devices(&dev, MediaPreviewBackend::Automatic);
    assert_eq!(devices.len(), blocked.len());
    assert!(polaris_gpu_available_at(&dev, &drm));
    let command = sandbox_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Videos/untrusted.mkv"),
        Path::new("/tmp/private-output"),
        MEDIA_PREVIEW.clone(),
        0,
        MediaPreviewBackend::Automatic,
        &devices,
    );
    for device in &devices {
        assert!(
            command
                .get_args()
                .any(|argument| argument == device.as_os_str())
        );
    }
}

#[test]
fn explicit_opt_in_exposes_mixed_and_unidentified_gpus_by_policy() {
    let root = PrivateOutput::create().expect("create temporary device tree");
    let dev = root.path().join("dev");
    let drm = root.path().join("sys/class/drm");
    create_render_node(&dev, &drm, "renderD128", Some((0x1002, 0x67df)));
    let modern = create_render_node(&dev, &drm, "renderD129", Some((0x1002, 0x73bf)));
    let unidentified = create_render_node(&dev, &drm, "renderD130", None);
    fs::write(dev.join("nvidia0"), []).expect("create NVIDIA device");
    fs::write(dev.join("nvidiactl"), []).expect("create NVIDIA control device");

    assert_eq!(
        gpu_devices(&dev, MediaPreviewBackend::VaApi),
        [
            dev.join("dri/renderD128"),
            modern.clone(),
            unidentified.clone()
        ]
    );
    assert_eq!(
        gpu_devices(&dev, MediaPreviewBackend::Vulkan),
        [
            dev.join("dri/renderD128"),
            modern.clone(),
            unidentified.clone(),
            dev.join("nvidia0"),
            dev.join("nvidiactl"),
        ]
    );
    assert_eq!(
        gpu_devices(&dev, MediaPreviewBackend::Automatic),
        [
            dev.join("dri/renderD128"),
            modern,
            unidentified,
            dev.join("nvidia0"),
            dev.join("nvidiactl"),
        ]
    );
    assert!(gpu_devices(&dev, MediaPreviewBackend::Software).is_empty());
    assert!(polaris_gpu_available_at(&dev, &drm));
}

#[test]
fn modern_and_unidentified_gpus_keep_the_accelerated_default() {
    let root = PrivateOutput::create().expect("create temporary device tree");
    let dev = root.path().join("dev");
    let drm = root.path().join("sys/class/drm");
    create_render_node(&dev, &drm, "renderD128", Some((0x1002, 0x73bf)));
    create_render_node(&dev, &drm, "renderD129", None);

    assert!(!polaris_gpu_available_at(&dev, &drm));
}

#[test]
fn media_sandbox_exposes_only_supplied_gpu_devices_and_sysfs() {
    let devices = [
        "/dev/dri/renderD128".into(),
        "/dev/nvidia0".into(),
        "/dev/nvidiactl".into(),
    ];
    let command = sandbox_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Videos/untrusted.mkv"),
        Path::new("/tmp/private-output"),
        MEDIA_PREVIEW.clone(),
        0,
        MediaPreviewBackend::Automatic,
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
    assert!(!joined.contains("--bind /tmp/private-output /output"));
    assert!(joined.contains("preview-media /input.mkv /dev/stdout"));
}

#[test]
fn software_media_sandbox_exposes_no_gpu_devices_or_sysfs() {
    let command = sandbox_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Videos/untrusted.mkv"),
        Path::new("/tmp/private-output"),
        MEDIA_PREVIEW.clone(),
        0,
        MediaPreviewBackend::Software,
        &["/dev/dri/renderD128".into(), "/dev/nvidia0".into()],
    );
    let joined = command
        .get_args()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(!joined.contains("--dev-bind-try"));
    assert!(!joined.contains("/sys"));
    assert!(joined.ends_with("640x800 software"));
}

#[test]
fn non_media_sandboxes_never_expose_gpu_devices_or_sysfs() {
    let command = sandbox_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Videos/untrusted.mkv"),
        Path::new("/tmp/private-output"),
        ParseOperation::ThumbnailVideo,
        128,
        MediaPreviewBackend::Automatic,
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
fn archive_listings_execute_inside_the_bounded_sandbox_without_leaking_secrets() {
    let command = sandbox_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Downloads/untrusted.zip"),
        Path::new("/tmp/private-output"),
        ParseOperation::ArchiveList {
            format: crate::services::ArchiveFormat::Zip,
            password: Some(crate::services::SecretString::new("s3cret".to_owned())),
        },
        0,
        MediaPreviewBackend::Software,
        &[],
    );
    let joined = command
        .get_args()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(joined.contains("--unshare-all"));
    assert!(joined.contains("--ro-bind /home/alice/Downloads/untrusted.zip /input.zip"));
    assert!(joined.contains("--bind /tmp/private-output /output"));
    assert!(joined.contains("--as=2147483648"));
    assert!(joined.contains("--cpu=10"));
    assert!(joined.contains("--fsize=536870912"));
    assert!(joined.contains(
        "/app/strata --preview-helper archive-list /input.zip /output/result.archive.json zip software"
    ));
    assert!(!joined.contains("s3cret"));
    assert!(!joined.contains("--share-net"));
}

#[test]
fn archive_listings_accept_only_valid_listing_payloads() {
    let operation = ParseOperation::ArchiveList {
        format: crate::services::ArchiveFormat::Tar,
        password: None,
    };
    let payload = serde_json::json!({
        "status": "open",
        "entries": [{"name": "a.txt", "directory": false, "size": 1}],
        "message": null,
    })
    .to_string()
    .into_bytes();
    assert!(valid_output(operation.clone(), &payload));
    assert!(!valid_output(operation.clone(), b"not json"));
    assert!(!valid_output(operation.clone(), b"{\"status\":\"open\"}"));
    assert!(!valid_output(
        operation.clone(),
        b"{\"status\":\"bogus\",\"entries\":[]}"
    ));
    assert!(valid_output(
        operation.clone(),
        b"{\"status\":\"too-large\",\"entries\":[],\"message\":null}"
    ));
    assert!(valid_output(
        operation.clone(),
        b"{\"status\":\"error\",\"entries\":[],\"message\":\"broken\"}"
    ));
    assert!(!valid_output(
        operation.clone(),
        b"{\"status\":\"error\",\"entries\":[],\"message\":\"\"}"
    ));
}

#[test]
fn helper_output_failures_classify_by_kind() {
    let dir = tempfile::tempdir().expect("scratch directory");
    let missing = dir.path().join("absent.bin");
    assert_eq!(
        super::read_private_output(&missing, 16),
        Err(OutputError::Missing)
    );

    let empty = dir.path().join("empty.bin");
    fs::write(&empty, b"").expect("empty output");
    assert_eq!(
        super::read_private_output(&empty, 16),
        Err(OutputError::Empty)
    );

    let bounded = dir.path().join("bounded.bin");
    fs::write(&bounded, b"12345678").expect("bounded output");
    assert_eq!(
        super::read_private_output(&bounded, 8).expect("exact size limit is allowed"),
        b"12345678"
    );
    assert_eq!(
        super::read_private_output(&bounded, 7),
        Err(OutputError::Oversize)
    );

    assert_eq!(
        super::read_private_output(dir.path(), 16),
        Err(OutputError::NonRegular)
    );
}

#[test]
fn archive_output_failures_map_to_archive_errors_not_image_errors() {
    assert_eq!(
        archive_output_error(&OutputError::Oversize),
        crate::adapters::ARCHIVE_TOO_LARGE_MESSAGE
    );
    for error in [
        OutputError::Missing,
        OutputError::NonRegular,
        OutputError::Empty,
        OutputError::Io("boom".to_owned()),
    ] {
        assert_eq!(
            archive_output_error(&error),
            crate::adapters::INVALID_ARCHIVE
        );
    }
    assert_eq!(
        OutputError::Missing.message(),
        "The preview renderer produced no output"
    );
    assert_eq!(
        OutputError::NonRegular.message(),
        "The preview renderer produced a non-regular output"
    );
    assert_eq!(
        OutputError::Empty.message(),
        "The preview renderer produced an invalid output size"
    );
    assert_eq!(
        OutputError::Oversize.message(),
        "The preview renderer produced an invalid output size"
    );
}

#[test]
fn video_thumbnails_execute_the_helper_inside_the_bounded_sandbox() {
    let command = sandbox_command(
        Path::new("/usr/bin/bwrap"),
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Videos/untrusted.mkv"),
        Path::new("/tmp/private-output"),
        ParseOperation::ThumbnailVideo,
        128,
        MediaPreviewBackend::Software,
        &[],
    );
    let joined = command
        .get_args()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(joined.contains("--unshare-all"));
    assert!(joined.contains("--ro-bind /home/alice/Videos/untrusted.mkv /input.mkv"));
    assert!(joined.contains("--bind /tmp/private-output /output"));
    assert!(joined.contains("--as=2147483648"));
    assert!(joined.contains("--cpu=10"));
    assert!(joined.contains("--fsize=33554432"));
    assert!(joined.contains("--ro-bind /tmp/strata /app/strata"));
    assert!(joined.contains(
        "/app/strata --preview-helper thumbnail-video /input.mkv /output/result.png 128 software"
    ));
    assert!(!joined.contains("--share-net"));
}

#[test]
fn accepts_only_bounded_png_outputs_and_never_compressed_media() {
    assert!(valid_output(ParseOperation::ThumbnailImage, &png(256, 256)));
    assert!(!valid_output(ParseOperation::ThumbnailImage, &png(257, 1)));
    assert!(valid_output(ParseOperation::PreviewImage, &png(800, 800)));
    assert!(!valid_output(ParseOperation::PreviewImage, &png(801, 1)));
    assert!(valid_output(PDF_PREVIEW.clone(), &png(640, 800)));
    assert!(!valid_output(PDF_PREVIEW.clone(), &png(641, 799)));
    assert!(!valid_output(PDF_PREVIEW.clone(), &png(640, 801)));
    assert!(!valid_output(PDF_PREVIEW.clone(), &png(0, 100)));
    assert!(!valid_output(
        ParseOperation::PreviewImage,
        b"\x89PNG\r\n\x1a\n"
    ));
    assert!(!valid_output(
        MEDIA_PREVIEW.clone(),
        b"\x1a\x45\xdf\xa3content"
    ));
    assert!(!valid_output(MEDIA_PREVIEW.clone(), b"\0\0\0\x18ftypisom"));
    assert!(!valid_output(MEDIA_PREVIEW.clone(), b""));
    assert!(!valid_output(MEDIA_PREVIEW.clone(), b"unrelated data"));
}

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
fn staged_secrets_have_no_directory_entry() {
    use std::os::fd::AsRawFd;

    let secret = stage_secret_anon(b"s3cret").expect("stage secret");
    let number = secret.as_raw_fd();

    let link =
        std::fs::read_link(format!("/proc/self/fd/{number}")).expect("inspect secret descriptor");
    assert!(
        link.to_string_lossy().ends_with(" (deleted)"),
        "secret descriptor must resolve to an unlinked inode, got {link:?}"
    );
    let stat = rustix::fs::fstat(&secret).expect("stat secret");
    assert_eq!(
        stat.st_nlink, 0,
        "anonymous secret inode must have no links"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(format!("/proc/self/fd/{number}"))
            .expect("stat secret path")
            .permissions()
            .mode()
            & 0o777,
        0o600,
        "anonymous secret inode must stay mode-0600"
    );

    let round_trip = std::fs::read(format!("/proc/self/fd/{number}")).expect("read secret");
    assert_eq!(round_trip, b"s3cret");
}

#[test]
fn staged_secrets_are_inherited_only_when_explicitly_mapped_to_stdin() {
    use std::os::fd::AsRawFd;
    use std::process::{Command, Stdio};

    let secret = stage_secret_anon(b"synthetic archive password").expect("stage secret");
    let path = format!("/proc/self/fd/{}", secret.as_raw_fd());
    let unrelated = Command::new("/bin/cat")
        .arg(&path)
        .output()
        .expect("spawn unrelated child");
    assert!(!unrelated.status.success());
    assert!(unrelated.stdout.is_empty());

    let intended = Command::new("/bin/cat")
        .arg("/proc/self/fd/0")
        .stdin(Stdio::from(fs::File::from(secret)))
        .output()
        .expect("spawn intended child");
    assert!(intended.status.success());
    assert_eq!(intended.stdout, b"synthetic archive password");
}

#[test]
fn archive_parse_infra_failures_report_renderer_failure() {
    let error = parse(
        Path::new("does-not-need-to-exist.zip"),
        ParseOperation::ArchiveList {
            format: crate::services::ArchiveFormat::Zip,
            password: None,
        },
        0,
        MediaPreviewBackend::Software,
        &Cancellation::default(),
    )
    .err()
    .expect("missing archive input must fail");
    assert_eq!(error, crate::adapters::ARCHIVE_PREVIEW_FAILED_MESSAGE);

    let error = parse(
        Path::new("does-not-need-to-exist.png"),
        ParseOperation::PreviewImage,
        0,
        MediaPreviewBackend::Software,
        &Cancellation::default(),
    )
    .err()
    .expect("missing image input must fail");
    assert!(
        error.starts_with("Unable to open preview input: "),
        "non-archive operations keep their own message, got {error:?}"
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

#[test]
fn archive_operation_debug_redacts_the_password() {
    let operation = ParseOperation::ArchiveList {
        format: crate::services::ArchiveFormat::Zip,
        password: Some(crate::services::SecretString::new(
            "strata-security-repro-password".to_owned(),
        )),
    };
    let rendered = format!("{operation:?}");
    assert!(
        !rendered.contains("strata-security-repro-password"),
        "Debug must never expose the password, got {rendered:?}"
    );
    assert!(
        rendered.contains("Some([REDACTED])"),
        "a present password must render redacted-but-present, got {rendered:?}"
    );
    let operation = ParseOperation::ArchiveList {
        format: crate::services::ArchiveFormat::Zip,
        password: None,
    };
    let rendered = format!("{operation:?}");
    assert!(
        rendered.contains("password: None"),
        "an absent password must stay distinguishable, got {rendered:?}"
    );
}
