// SPDX-License-Identifier: MIT

use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    os::fd::AsRawFd,
    sync::{Arc, atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

use gtk::glib;

use super::{NAMES, Watched, describe, watch};

fn number_of(name: &str) -> u64 {
    NAMES
        .iter()
        .find_map(|(number, known)| (*known == name).then_some(*number))
        .expect("a known system call")
}

fn call(name: &str, arguments: &[u64]) -> String {
    let mut line = number_of(name).to_string();
    for index in 0..6 {
        line.push_str(&format!(
            " {:#x}",
            arguments.get(index).copied().unwrap_or(0)
        ));
    }
    line + " 0x7ffc0000 0x7f0000000"
}

#[test]
fn a_blocked_ui_thread_is_reported_with_its_call_and_path_until_it_runs_again() {
    let folder = tempfile::tempdir().expect("temporary folder");
    let fifo = folder.path().join("fifo");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )
    .expect("fifo");
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("default context lock");
    let context = glib::MainContext::default();
    let stopped = Arc::new(AtomicBool::new(false));
    let (lines, reported) = mpsc::channel();
    watch(
        Watched {
            context: context.clone(),
            thread: rustix::thread::gettid().as_raw_nonzero().get(),
            limit: Duration::from_secs(1),
        },
        stopped.clone(),
        move |line| {
            let _ = lines.send(line);
        },
    )
    .expect("watchdog");

    let answering_until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < answering_until {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(reported.try_recv().ok(), None);

    let writer = std::thread::spawn({
        let fifo = fifo.clone();
        move || {
            let stall = reported.recv_timeout(Duration::from_secs(10));
            let _writer = OpenOptions::new().write(true).open(&fifo).expect("writer");
            (stall, reported)
        }
    });
    let _reader = File::open(&fifo).expect("reader");
    let (stall, reported) = writer.join().expect("writer thread");
    let stall = stall.expect("a stall report while the UI thread is blocked");
    assert!(
        stall.starts_with("UI thread stalled ")
            && stall.contains(&format!("ms in openat({fifo:?}) wchan=")),
        "{stall}"
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    let ran_again = loop {
        context.iteration(false);
        if let Ok(line) = reported.recv_timeout(Duration::from_millis(10))
            && line.starts_with("UI thread ran again after ")
        {
            break true;
        }
        if Instant::now() > deadline {
            break false;
        }
    };
    stopped.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(ran_again);
}

#[test]
fn a_stalled_call_names_the_file_behind_its_descriptor_or_folder() {
    let folder = tempfile::tempdir().expect("temporary folder");
    let opened = File::open(folder.path()).expect("folder");
    let file_path = folder.path().join("file");
    let file = File::create(&file_path).expect("file");
    let child = CString::new("child").expect("name");
    let empty = CString::new("").expect("name");
    let mut polled = [0_u8; 8];
    polled[..4].copy_from_slice(&file.as_raw_fd().to_ne_bytes());
    let folder_fd = opened.as_raw_fd() as u64;
    let file_fd = file.as_raw_fd() as u64;

    for (line, expected) in [
        (
            call("openat", &[folder_fd, child.as_ptr() as u64]),
            format!("in openat({:?})", folder.path().join("child")),
        ),
        (
            call("statx", &[file_fd, empty.as_ptr() as u64, 0x1000]),
            format!("in statx({file_path:?})"),
        ),
        (
            call("fsync", &[file_fd]),
            format!("in fsync({file_path:?})"),
        ),
        (
            call("ppoll", &[polled.as_ptr() as u64, 1]),
            format!("in ppoll({file_path:?})"),
        ),
        ("running".to_owned(), "while running code".to_owned()),
    ] {
        assert_eq!(describe(&line), expected, "{line}");
    }
}
