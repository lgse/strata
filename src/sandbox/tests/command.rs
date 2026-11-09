// SPDX-License-Identifier: MIT

use std::path::Path;

use super::{
    MEDIA_PREVIEW, PDF_PREVIEW, Sandbox, arguments_of, assert_contains_all, assert_contains_none,
    assert_no_argument, assert_validity,
};
use crate::sandbox::{
    DocumentFormat, MediaPreviewBackend, ParseOperation, runtime, sandbox_input_path,
};

const ISOLATED: &str = "--unshare-all --die-with-parent --new-session --clearenv";
const SOFTWARE_LIMITS: &str = "--as=2147483648 --cpu=10";

fn limit_from(arguments: &[String], flag: &str) -> u64 {
    arguments
        .iter()
        .find_map(|argument| argument.strip_prefix(flag))
        .unwrap_or_else(|| panic!("the sandbox must pass {flag}"))
        .parse()
        .expect("resource limits must be numeric")
}

fn has_window(arguments: &[String], window: [&str; 3]) -> bool {
    arguments.windows(3).any(|args| args == window)
}

fn photo_thumbnail() -> Sandbox<'static> {
    Sandbox::new(
        "/home/alice/Pictures/photo.jpg",
        ParseOperation::ThumbnailImage,
    )
    .value(256)
}

#[test]
fn file_size_limit_holds_a_full_resolution_decoded_frame() {
    // gdk-pixbuf decodes through glycin, which sizes a memfd to
    // `width * height * channels` before scaling down. RLIMIT_FSIZE covers that
    // buffer too, and exceeding it kills the loader with SIGXFSZ rather than
    // surfacing an error, so the limit has to clear the largest frame we expect.
    const LARGEST_SUPPORTED_PIXELS: u64 = 50_000_000;
    const RGBA_CHANNELS: u64 = 4;

    let arguments = photo_thumbnail().arguments();

    let file_size = limit_from(&arguments, "--fsize=");
    let address_space = limit_from(&arguments, "--as=");

    assert!(file_size >= LARGEST_SUPPORTED_PIXELS * RGBA_CHANNELS);
    assert!(file_size < address_space);
}

#[test]
fn sandbox_command_starts_absolute_bubblewrap() {
    let command = photo_thumbnail().command();
    assert_eq!(command.get_program(), Path::new("/usr/bin/bwrap"));
    let arguments = arguments_of(&command);
    assert_contains_all(
        &arguments.join(" "),
        &["--unshare-all", "--clearenv", "--info-fd 2"],
    );
    let runtime = runtime::current();
    let (path, root, prlimit) = (
        runtime.path.as_str(),
        runtime.root.as_str(),
        runtime.prlimit.as_str(),
    );
    for window in [
        ["--setenv", "PATH", path],
        ["--ro-bind", root, root],
        ["--", prlimit, "--as=2147483648"],
    ] {
        assert!(has_window(&arguments, window), "{window:?}");
    }
    match &runtime.gdk_pixbuf_module_file {
        Some(loaders) => assert!(has_window(
            &arguments,
            ["--setenv", "GDK_PIXBUF_MODULE_FILE", loaders]
        )),
        None => assert_no_argument(&arguments, &["GDK_PIXBUF_MODULE_FILE"]),
    }
}

#[test]
fn sandbox_exposes_only_runtime_input_and_private_output() {
    let joined = Sandbox::new("/home/alice/Downloads/untrusted.pdf", PDF_PREVIEW.clone())
        .value(2)
        .joined();

    assert_contains_all(
        &joined,
        &[
            "--unshare-all",
            "--clearenv",
            "--ro-bind /home/alice/Downloads/untrusted.pdf /input.pdf",
            "--bind /tmp/private-output /output",
            "--as=2147483648",
            "--cpu=10",
            "--fsize=536870912",
            "--setenv MALLOC_ARENA_MAX 1",
            "--size 536870912 --tmpfs /tmp",
            "preview-pdf /input.pdf /output/result.png 2:640x800 software",
        ],
    );
    // RLIMIT_NPROC counts every process owned by the host user, not just the
    // sandbox, and can prevent legitimate media decoders from starting.
    assert_contains_none(
        &joined,
        &["--nproc", "--ro-bind /home /home", "--share-net"],
    );
}

#[test]
fn workbook_parser_uses_resource_limited_sandbox_and_validated_output() {
    let arguments = Sandbox::new("/fixtures/book.xlsx", ParseOperation::PreviewWorkbook)
        .installed_at("/app/strata", "/private-output")
        .joined();
    assert_contains_all(
        &arguments,
        &[
            ISOLATED,
            SOFTWARE_LIMITS,
            "preview-workbook /input.xlsx /output/result.json",
        ],
    );
    assert_contains_none(&arguments, &["--share-net", "--dev-bind"]);
    assert_validity(
        &ParseOperation::PreviewWorkbook,
        &[
            (br#"{"rows":[["a"],["1"]],"truncated":false}"#, true),
            (b"invalid", false),
        ],
    );
}

#[test]
fn word_document_parser_uses_resource_limited_sandbox_and_validated_output() {
    for (input, format, expected) in [
        (
            "/fixtures/report.docx",
            DocumentFormat::Docx,
            "preview-document /input.docx /output/result.json docx",
        ),
        (
            "/fixtures/report.ott",
            DocumentFormat::Odt,
            "preview-document /input.ott /output/result.json odt",
        ),
    ] {
        let arguments = Sandbox::new(input, ParseOperation::PreviewDocument(format))
            .installed_at("/app/strata", "/private-output")
            .joined();
        assert_contains_all(&arguments, &[ISOLATED, SOFTWARE_LIMITS, expected]);
        assert_contains_none(&arguments, &["--share-net"]);
        assert_validity(
            &ParseOperation::PreviewDocument(format),
            &[
                (br#"{"html":"<p>a</p>","truncated":false}"#, true),
                (br#"{"rows":[["a"]],"truncated":false}"#, false),
            ],
        );
    }
}

fn alternatives_binds(names: &[&str], architectures: &[&str]) -> Vec<String> {
    let mut binds = Vec::new();
    for architecture in architectures {
        for name in names {
            let path = format!("/etc/alternatives/{name}-{architecture}");
            binds.push(format!("--ro-bind-try {path} {path}"));
        }
    }
    binds
}

#[test]
fn metadata_probe_retains_software_sandbox_limits_and_narrow_runtime_access() {
    let arguments = Sandbox::new(
        "/home/alice/Videos/untrusted.mkv",
        ParseOperation::MediaMetadata,
    )
    .arguments();
    let joined = arguments.join(" ");
    assert_contains_all(
        &joined,
        &[
            ISOLATED,
            "--ro-bind /home/alice/Videos/untrusted.mkv /input.mkv",
            SOFTWARE_LIMITS,
            "media-metadata /input.mkv /output/result.json 0 software",
        ],
    );
    assert_contains_none(&joined, &["--dev-bind", "--share-net"]);
    assert_no_argument(&arguments, &["/etc/alternatives", "/home", "/sys"]);
    let blas = alternatives_binds(
        &["libblas.so.3", "liblapack.so.3"],
        &["x86_64-linux-gnu", "aarch64-linux-gnu"],
    );
    assert_contains_all(&joined, &blas);
}

#[test]
fn non_media_helpers_expose_gl_dispatch_links_without_the_whole_directory() {
    let arguments = photo_thumbnail().arguments();
    assert_no_argument(&arguments, &["/etc/alternatives"]);
    let mut exposed: Vec<_> = arguments
        .windows(3)
        .filter(|bind| bind[0] == "--ro-bind-try" && bind[1].starts_with("/etc/alternatives/glx--"))
        .map(|bind| format!("{} {} {}", bind[0], bind[1], bind[2]))
        .collect();
    let mut expected = alternatives_binds(
        &[
            "glx--libGL.so.1",
            "glx--libEGL.so.1",
            "glx--libGLESv2.so.2",
            "glx--libGLESv1_CM.so.1",
            "glx--libGL.so",
            "glx--libEGL.so",
            "glx--libGLESv2.so",
            "glx--libGLESv1_CM.so",
            "glx--libGLX_indirect.so.0",
        ],
        &["x86_64-linux-gnu", "aarch64-linux-gnu"],
    );
    exposed.sort();
    expected.sort();
    assert_eq!(exposed, expected);
}

#[test]
fn image_sandboxes_resolve_debian_imagemagick_alternatives() {
    let arguments = Sandbox::new(
        "/home/alice/Pictures/untrusted.heic",
        ParseOperation::PreviewImage,
    )
    .arguments();
    let tools: Vec<String> = ["magick", "convert"]
        .into_iter()
        .map(|tool| format!("--ro-bind-try /etc/alternatives/{tool} /etc/alternatives/{tool}"))
        .collect();
    assert_contains_all(&arguments.join(" "), &tools);
    assert_no_argument(&arguments, &["/etc/alternatives"]);
}

#[test]
fn media_previews_use_bounded_streaming_instead_of_driver_wide_resource_limits() {
    let arguments = Sandbox::new("/home/alice/Videos/untrusted.mkv", MEDIA_PREVIEW.clone())
        .backend(MediaPreviewBackend::Automatic)
        .arguments();
    let joined = arguments.join(" ");

    let prlimit = runtime::current().prlimit.as_str();
    assert!(has_window(
        &arguments,
        ["--setenv", runtime::PRLIMIT_VARIABLE, prlimit]
    ));
    assert!(
        !arguments.windows(2).any(|args| args == ["--", prlimit]),
        "the helper limits each decoder, not the accelerated helper's address space"
    );
    assert_contains_none(
        &joined,
        &[
            "--as=",
            "--cpu=",
            "--fsize=",
            "MALLOC_ARENA_MAX",
            "--bind /tmp/private-output /output",
        ],
    );
    assert_contains_all(
        &joined,
        &[
            "--size 536870912 --tmpfs /tmp",
            "preview-media /input.mkv /dev/stdout",
        ],
    );
}

#[test]
fn audio_artwork_forwards_prlimit_and_keeps_the_outer_resource_limits() {
    let arguments = Sandbox::new("/fixtures/covered.m4a", ParseOperation::AudioCover).arguments();
    let prlimit = runtime::current().prlimit.as_str();
    for window in [
        ["--setenv", runtime::PRLIMIT_VARIABLE, prlimit],
        ["--", prlimit, "--as=2147483648"],
    ] {
        assert!(has_window(&arguments, window), "{window:?}");
    }
    assert_contains_all(&arguments.join(" "), &[SOFTWARE_LIMITS]);
}

#[test]
fn sandbox_input_keeps_a_safe_filename_extension() {
    for (input, expected) in [
        ("/photos/DSC01986.ARW", "/input.ARW"),
        ("/tmp/no-extension", "/input"),
        ("/tmp/photo.ar-w", "/input"),
        ("/tmp/toolongextension.abcdefghij", "/input"),
    ] {
        assert_eq!(sandbox_input_path(Path::new(input)), expected, "{input}");
    }
}

#[test]
fn archive_listings_execute_inside_the_bounded_sandbox_without_leaking_secrets() {
    let joined = Sandbox::new(
        "/home/alice/Downloads/untrusted.zip",
        ParseOperation::ArchiveList {
            format: crate::services::ArchiveFormat::Zip,
            password: Some(crate::services::SecretString::new("s3cret".to_owned())),
        },
    )
    .joined();

    assert_contains_all(
        &joined,
        &[
            "--unshare-all",
            "--ro-bind /home/alice/Downloads/untrusted.zip /input.zip",
            "--bind /tmp/private-output /output",
            "--as=2147483648",
            "--cpu=10",
            "--fsize=536870912",
            "/app/strata --preview-helper archive-list /input.zip /output/result.archive.json zip software",
        ],
    );
    assert_contains_none(&joined, &["s3cret", "--share-net"]);
}

#[test]
fn video_thumbnails_execute_the_helper_inside_the_bounded_sandbox() {
    let joined = Sandbox::new(
        "/home/alice/Videos/untrusted.mkv",
        ParseOperation::ThumbnailVideo,
    )
    .value(128)
    .joined();

    assert_contains_all(
        &joined,
        &[
            "--unshare-all",
            "--ro-bind /home/alice/Videos/untrusted.mkv /input.mkv",
            "--bind /tmp/private-output /output",
            "--as=2147483648",
            "--cpu=10",
            "--fsize=33554432",
            "--ro-bind /tmp/strata /app/strata",
            "/app/strata --preview-helper thumbnail-video /input.mkv /output/result.png 128 software",
        ],
    );
    assert_contains_none(&joined, &["--share-net"]);
}
