// SPDX-License-Identifier: MIT

use std::{fs, path::PathBuf, thread, time::Duration};

use crate::sandbox::{
    NEXT_DIRECTORY, OutputError, PrivateOutput, prune_orphaned_private_outputs, read_metadata,
    read_private_output,
};

#[test]
fn renderer_outputs_are_never_read_through_symlinks() {
    let dir = tempfile::tempdir().expect("scratch directory");
    let real = dir.path().join("host-file");
    fs::write(&real, b"2 5").expect("host file");
    for name in ["result.png", "result.meta", "result.model", "result.error"] {
        let link = dir.path().join(name);
        std::os::unix::fs::symlink(&real, &link).expect("planted symlink");
        assert!(read_private_output(&link, 256).is_err());
        assert_eq!(read_metadata(&link), (0, 0));
    }
    assert_eq!(read_metadata(&real), (2, 5));
    assert_eq!(fs::read(&real).expect("unchanged host file"), b"2 5");
}

#[test]
fn renderer_outputs_require_nonempty_bounded_regular_files() {
    let dir = tempfile::tempdir().expect("scratch directory");
    let output = dir.path().join("result.png");
    assert!(read_private_output(&output, 4).is_err());
    assert!(read_private_output(dir.path(), 4).is_err());
    for bytes in [b"".as_slice(), b"data", b"extra"] {
        fs::write(&output, bytes).expect("renderer output");
        let result = read_private_output(&output, 4);
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
        let _sent = sender.send(read_private_output(&fifo, 256));
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
    assert_eq!(read_metadata(&output), (0, 0));
    assert_eq!(read_metadata(dir.path()), (0, 0));
    for bytes in [b"".as_slice(), b"2 5 \xff"] {
        fs::write(&output, bytes).expect("invalid metadata");
        assert_eq!(read_metadata(&output), (0, 0));
    }
    let bounded = format!("2 5{}", " ".repeat(253));
    fs::write(&output, &bounded).expect("metadata at size limit");
    assert_eq!(read_metadata(&output), (2, 5));
    fs::write(&output, format!("{bounded} ")).expect("oversized metadata");
    assert_eq!(read_metadata(&output), (0, 0));
}

#[test]
fn helper_output_failures_classify_by_kind() {
    let dir = tempfile::tempdir().expect("scratch directory");
    let missing = dir.path().join("absent.bin");
    assert_eq!(read_private_output(&missing, 16), Err(OutputError::Missing));

    let empty = dir.path().join("empty.bin");
    fs::write(&empty, b"").expect("empty output");
    assert_eq!(read_private_output(&empty, 16), Err(OutputError::Empty));

    let bounded = dir.path().join("bounded.bin");
    fs::write(&bounded, b"12345678").expect("bounded output");
    assert_eq!(
        read_private_output(&bounded, 8).expect("exact size limit is allowed"),
        b"12345678"
    );
    assert_eq!(read_private_output(&bounded, 7), Err(OutputError::Oversize));

    assert_eq!(
        read_private_output(dir.path(), 16),
        Err(OutputError::NonRegular)
    );
}

fn sorted_names(directory: &std::path::Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(directory)
        .expect("list scratch directory")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn preview_sweep_removes_unlocked_preview_directories_and_keeps_everything_else() {
    let tmp = tempfile::tempdir().expect("scratch directory");
    let live = PrivateOutput::create_in(tmp.path()).expect("live preview directory");
    let live_name = live
        .path()
        .file_name()
        .expect("name")
        .to_string_lossy()
        .into_owned();
    let orphaned_with_our_pid = format!("strata-preview-{}-0", std::process::id());
    for name in [
        orphaned_with_our_pid.as_str(),
        "strata-preview-1-1",
        "strata-preview-bogus-3",
        "strata-preview-7",
        "unrelated",
    ] {
        fs::create_dir(tmp.path().join(name)).expect("fixture directory");
    }
    fs::write(tmp.path().join("strata-preview-5-9"), b"").expect("fixture file");
    std::os::unix::fs::symlink(
        tmp.path().join("unrelated"),
        tmp.path().join("strata-preview-6-1"),
    )
    .expect("fixture symlink");

    prune_orphaned_private_outputs(tmp.path());

    let mut expected = vec![
        live_name,
        "strata-preview-5-9".to_owned(),
        "strata-preview-6-1".to_owned(),
        "strata-preview-7".to_owned(),
        "strata-preview-bogus-3".to_owned(),
        "unrelated".to_owned(),
    ];
    expected.sort();
    assert_eq!(sorted_names(tmp.path()), expected);
}

#[test]
fn private_output_skips_leftover_directories_that_reuse_its_name() {
    let tmp = tempfile::tempdir().expect("scratch directory");
    let next = NEXT_DIRECTORY.load(std::sync::atomic::Ordering::Relaxed);
    let leftovers: Vec<PathBuf> = (next..next + 4)
        .map(|sequence| {
            tmp.path()
                .join(format!("strata-preview-{}-{sequence}", std::process::id()))
        })
        .collect();
    for leftover in &leftovers {
        fs::create_dir(leftover).expect("leftover directory");
    }

    let output = PrivateOutput::create_in(tmp.path()).expect("create past leftovers");

    assert!(output.path().is_dir());
    assert!(!leftovers.iter().any(|leftover| leftover == output.path()));
}
