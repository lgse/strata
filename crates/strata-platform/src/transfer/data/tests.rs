use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;

#[test]
fn a_writer_runs_only_when_asked() {
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    let w = Writer::lazy(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        b"data".to_vec()
    });
    let spec = SourceSpec::new().with("text/plain", w.clone());
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    assert_eq!(w.ready(), None);
    assert_eq!(
        spec.writer("text/plain").and_then(Writer::get).as_deref(),
        Some(&b"data"[..])
    );
    assert_eq!(w.get().as_deref(), Some(&b"data"[..]));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[test]
fn a_writer_runs_once_for_concurrent_requests() {
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    let w = Writer::lazy(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        vec![7; 1000]
    });
    let readers: Vec<_> = (0..8)
        .map(|_| {
            let w = w.clone();
            std::thread::spawn(move || w.get().map(|b| b.len()))
        })
        .collect();
    for r in readers {
        assert_eq!(r.join().ok().flatten(), Some(1000));
    }
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[test]
fn a_panicking_writer_fails_its_requests() {
    let w = Writer::lazy(|| panic!("the writer failed"));
    let other = w.clone();
    assert_eq!(
        std::thread::spawn(move || other.get()).join().ok(),
        Some(None)
    );
    assert_eq!(w.get(), None);
}
