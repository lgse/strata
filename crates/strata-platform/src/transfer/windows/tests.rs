// SPDX-License-Identifier: MIT

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

#[derive(Clone)]
struct Fake {
    key: String,
    generation: u32,
    open: Arc<AtomicBool>,
}

impl Fake {
    fn new(key: &str, generation: u32) -> Self {
        Fake {
            key: key.to_owned(),
            generation,
            open: Arc::new(AtomicBool::new(true)),
        }
    }

    fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
    }
}

impl Window for Fake {
    fn key(&self) -> &str {
        &self.key
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }
}

#[test]
fn prune_drops_closed_windows_and_keeps_open_ones() {
    let closed = Fake::new("closed", 1);
    let live = Fake::new("live", 1);
    let w = Windows::default();
    w.attach(closed.clone());
    w.attach(live.clone());
    assert_eq!(w.len(), 2);
    closed.close();
    w.prune();
    assert_eq!(w.len(), 1);
    assert_eq!(w.find(|_| true).as_deref(), Some("live"));
}

/// Rune recreates a window (a Wayland hide and show): one entry, with the new handle.
#[test]
fn attaching_a_window_again_replaces_its_entry() {
    let old = Fake::new("main", 1);
    let new = Fake::new("main", 2);
    let w = Windows::default();
    w.attach(old);
    w.attach(new);
    assert_eq!(w.len(), 1);
    assert_eq!(w.find(|f| f.generation == 2).as_deref(), Some("main"));
    assert_eq!(w.find(|f| f.generation == 1), None);
}

#[test]
fn detach_drops_a_window_at_once() {
    let w = Windows::default();
    w.attach(Fake::new("a", 1));
    w.attach(Fake::new("b", 1));
    assert!(w.detach("a"));
    assert!(!w.detach("a"));
    assert_eq!(w.len(), 1);
    assert_eq!(w.find(|_| true).as_deref(), Some("b"));
}
