use std::io::Write;
use std::time::{Duration, Instant};

use super::*;

fn limits(max_bytes: usize, idle_ms: u64) -> ReadLimits {
    ReadLimits {
        max_bytes,
        idle: Duration::from_millis(idle_ms),
    }
}

#[test]
fn a_read_ends_at_eof() {
    let (r, w) = pipe().expect("pipe");
    let writer = std::thread::spawn(move || {
        let mut w = File::from(w);
        for chunk in [&b"hello "[..], b"from ", b"a pipe"] {
            w.write_all(chunk).expect("write");
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    assert_eq!(
        read_all(r, limits(1024, 2000)).as_deref(),
        Ok(&b"hello from a pipe"[..])
    );
    writer.join().expect("writer");
}

#[test]
fn a_read_over_the_cap_stops() {
    let (r, w) = pipe().expect("pipe");
    let writer = std::thread::spawn(move || {
        let _ = File::from(w).write_all(&[1; 4096]);
    });
    assert_eq!(read_all(r, limits(4095, 2000)), Err(ReadError::TooLarge));
    writer.join().expect("writer");
}

#[test]
fn a_silent_writer_times_out() {
    let (r, w) = pipe().expect("pipe");
    let started = Instant::now();
    assert_eq!(read_all(r, limits(1024, 50)), Err(ReadError::TimedOut));
    assert!(started.elapsed() >= Duration::from_millis(50));
    drop(w);
}

#[test]
fn writing_to_a_reader_that_left_reports_it() {
    let (r, w) = pipe().expect("pipe");
    drop(r);
    // Without SIGPIPE ignored this would kill the test binary; Rust ignores it, and the
    // write sees EPIPE.
    assert_eq!(
        write_all(w, &[0; 1024], Duration::from_secs(1)),
        Err(WriteError::ReaderLeft)
    );
}

#[test]
fn a_stalled_reader_times_out_a_write() {
    let (r, w) = pipe().expect("pipe");
    // A pipe holds 64 KiB by default; 1 MiB fills it, and nobody reads.
    assert_eq!(
        write_all(w, &vec![0; 1 << 20], Duration::from_millis(50)),
        Err(WriteError::TimedOut)
    );
    drop(r);
}
