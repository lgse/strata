// SPDX-License-Identifier: MIT

use super::*;
use std::{fs, os::unix::fs::symlink};

#[test]
fn image_paths_decode_spaces_and_confine_symlinks_and_traversal() {
    let directory = tempfile::tempdir().expect("fixture");
    let root = directory.path().join("document");
    fs::create_dir(&root).expect("document folder");
    fs::write(root.join("image with spaces.png"), b"image").expect("image");
    fs::write(directory.path().join("outside.png"), b"private").expect("outside file");
    symlink("../outside.png", root.join("escape.png")).expect("escaping link");
    symlink("image with spaces.png", root.join("inside.png")).expect("internal link");
    for path in [
        "image%20with%20spaces.png",
        "inside.png",
        "image%20with%20spaces.png#fragment",
    ] {
        assert_eq!(
            read_image(&root, path).expect("safe local image").0,
            b"image"
        );
    }
    for path in [
        "../outside.png",
        "%2e%2e/outside.png",
        "escape.png",
        "/outside.png",
        "%2foutside.png",
        "https://example.test/image.png",
        "//example.test/image.png",
        "file:///outside.png",
        "data:image/png;base64,AA==",
        "missing.png",
        "image%00.png",
        "image.txt",
    ] {
        assert!(read_image(&root, path).is_err(), "must reject {path}");
    }
}

#[test]
fn image_read_rejects_special_files_and_oversized_input() {
    let root = tempfile::tempdir().expect("fixture");
    fs::create_dir(root.path().join("folder.png")).expect("directory");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        root.path().join("pipe.png"),
        rustix::fs::FileType::Fifo,
        Mode::RUSR | Mode::WUSR,
        0,
    )
    .expect("FIFO");
    File::create(root.path().join("large.png"))
        .expect("large image")
        .set_len(IMAGE_INPUT_LIMIT + 1)
        .expect("size");
    for path in ["folder.png", "pipe.png", "large.png"] {
        assert!(read_image(root.path(), path).is_err(), "reject {path}");
    }
}

#[test]
fn media_cancellation_and_remote_document_fail_before_rendering() {
    let cancellation = Cancellation::default();
    assert!(
        render(
            &DocumentMedia::Image("image.png".into()),
            None,
            &cancellation
        )
        .expect_err("remote documents cannot load local images")
        .contains("remote documents")
    );
    assert!(
        render(
            &DocumentMedia::Mermaid("x".repeat(DIAGRAM_INPUT_LIMIT + 1)),
            None,
            &cancellation
        )
        .expect_err("oversized Mermaid source must not launch a helper")
        .contains("limit")
    );
    cancellation.cancel();
    assert_eq!(
        render(
            &DocumentMedia::Mermaid("flowchart LR\nA-->B".into()),
            None,
            &cancellation
        )
        .expect_err("cancelled media must not launch a helper"),
        "Preview cancelled"
    );
}
