use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;

#[derive(Clone)]
struct Fake {
    key: String,
    open: Arc<AtomicBool>,
}

impl Window for Fake {
    fn key(&self) -> &str {
        &self.key
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }
}

type Queued = Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>;

/// An app: its turns queue until [`App::run`], as `AppHandle::update_if`'s do. A closed app
/// never runs them.
#[derive(Clone)]
struct App {
    window: Fake,
    queued: Queued,
}

impl App {
    fn new(key: &str) -> App {
        App {
            window: Fake {
                key: key.to_owned(),
                open: Arc::new(AtomicBool::new(true)),
            },
            queued: Queued::default(),
        }
    }

    fn turn(&self) -> Turn {
        let q = self.queued.clone();
        Arc::new(move |f| q.lock().unwrap().push(f))
    }

    fn run(&self) {
        let fs: Vec<_> = std::mem::take(&mut *self.queued.lock().unwrap());
        for f in fs {
            f();
        }
    }

    fn queued(&self) -> usize {
        self.queued.lock().unwrap().len()
    }

    fn close(&self) {
        self.window.open.store(false, Ordering::SeqCst);
    }
}

fn counting() -> (Arc<AtomicUsize>, impl Fn() -> Box<dyn FnOnce() + Send>) {
    let n = Arc::new(AtomicUsize::new(0));
    let m = n.clone();
    (n, move || {
        let m = m.clone();
        Box::new(move || {
            m.fetch_add(1, Ordering::SeqCst);
        })
    })
}

fn attached(apps: &[&App]) -> Arc<Windows<Fake>> {
    let w = Arc::new(Windows::default());
    for a in apps {
        w.attach(a.window.clone(), a.turn());
    }
    w
}

/// B1: the first window to attach closes; a round still reaches the other, and every round
/// after it too, while the closed one holds the single turn it was given.
#[test]
fn a_round_reaches_a_live_window_after_the_first_closes() {
    let (first, second) = (App::new("first"), App::new("second"));
    let w = attached(&[&first, &second]);
    let (dispatched, job) = counting();
    first.close();
    for round in 1..=5 {
        w.post(&job);
        second.run();
        assert_eq!(dispatched.load(Ordering::SeqCst), round, "round {round}");
    }
    assert_eq!(
        first.queued(),
        1,
        "a closed window holds one turn, not one a round"
    );
}

#[test]
fn a_live_window_gets_no_second_turn_while_one_is_outstanding() {
    let app = App::new("main");
    let w = attached(&[&app]);
    let (dispatched, job) = counting();
    assert_eq!(w.post(&job), 1);
    assert_eq!(w.post(&job), 0, "its first turn is still to run");
    app.run();
    assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    assert_eq!(w.post(&job), 1, "posted to again once it ran");
}

/// Should-fix 5: closed windows don't pile up among those posted to.
#[test]
fn a_closed_window_is_pruned_once_two_later_rounds_ran_without_it() {
    let (closed, live) = (App::new("closed"), App::new("live"));
    let w = attached(&[&closed, &live]);
    let (_, job) = counting();
    closed.close();
    w.post(&job);
    live.run();
    w.post(&job);
    live.run();
    w.prune();
    assert_eq!(w.len(), 2, "one later round isn't enough");
    w.post(&job);
    live.run();
    w.prune();
    assert_eq!(w.len(), 1);
    assert_eq!(w.find(|_| true).as_deref(), Some("live"));
}

/// A hidden window's native side is gone too, but its app still runs turns: it stays.
#[test]
fn a_hidden_window_whose_turns_run_is_kept() {
    let (hidden, live) = (App::new("hidden"), App::new("live"));
    let w = attached(&[&hidden, &live]);
    let (_, job) = counting();
    hidden.close();
    for _ in 0..4 {
        w.post(&job);
        hidden.run();
        live.run();
        w.prune();
    }
    assert_eq!(w.len(), 2);
}

/// P4's review 2, should-fix A: Rune orders one app's updates, not one app's against
/// another's, so the live window's app can run three rounds on its own wakes before the
/// hidden window's app runs its first. The hidden window is set aside then, but its late turn
/// brings it back, and the rounds after reach it again.
#[test]
fn a_hidden_window_whose_turn_runs_after_three_later_rounds_is_kept() {
    let (hidden, live) = (App::new("hidden"), App::new("live"));
    let w = attached(&[&hidden, &live]);
    let (dispatched, job) = counting();
    hidden.close();
    for _ in 0..3 {
        w.post(&job);
        live.run();
        w.prune();
    }
    assert_eq!(hidden.queued(), 1, "its first turn is still to run");
    hidden.run();
    assert_eq!(dispatched.load(Ordering::SeqCst), 4);
    assert_eq!(w.len(), 2);
    assert_eq!(w.post(&job), 2, "both windows are posted to again");
    assert_eq!(hidden.queued(), 1);
}

/// The windows set aside are bounded: beyond [`DORMANT`] the oldest go, and a turn of theirs
/// that runs after brings nothing back. The newest still come back.
#[test]
fn closed_windows_set_aside_beyond_the_bound_are_dropped() {
    let closed: Vec<App> = (0..=DORMANT)
        .map(|i| App::new(&format!("closed{i}")))
        .collect();
    let live = App::new("live");
    let mut all: Vec<&App> = closed.iter().collect();
    all.push(&live);
    let w = attached(&all);
    let (_, job) = counting();
    for c in &closed {
        c.close();
    }
    for _ in 0..3 {
        w.post(&job);
        live.run();
        w.prune();
    }
    assert_eq!(w.len(), 1);
    closed[0].run();
    assert_eq!(w.len(), 1, "the oldest went beyond the bound");
    closed[DORMANT].run();
    assert_eq!(w.len(), 2, "the newest was kept aside");
}

/// A window set aside and then detached (its `on_close` ran late) is gone for good.
#[test]
fn a_window_set_aside_then_detached_stays_gone() {
    let (closed, live) = (App::new("closed"), App::new("live"));
    let w = attached(&[&closed, &live]);
    let (_, job) = counting();
    closed.close();
    for _ in 0..3 {
        w.post(&job);
        live.run();
        w.prune();
    }
    assert!(w.detach("closed"), "it was attached");
    closed.run();
    assert_eq!(w.len(), 1);
    assert_eq!(w.find(|_| true).as_deref(), Some("live"));
}

#[test]
fn an_open_window_is_kept_however_late_its_turn() {
    let (slow, live) = (App::new("slow"), App::new("live"));
    let w = attached(&[&slow, &live]);
    let (_, job) = counting();
    for _ in 0..4 {
        w.post(&job);
        live.run();
        w.prune();
    }
    assert_eq!(w.len(), 2);
}

/// Rune recreates a window (a Wayland hide and show): one entry, with the new turn.
#[test]
fn attaching_a_window_again_replaces_its_entry_and_turn() {
    let (old, new) = (App::new("main"), App::new("main"));
    let w = attached(&[&old, &new]);
    assert_eq!(w.len(), 1);
    let (dispatched, job) = counting();
    w.post(&job);
    new.run();
    assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    assert_eq!(old.queued(), 0);
}

#[test]
fn detach_drops_a_window_at_once() {
    let (a, b) = (App::new("a"), App::new("b"));
    let w = attached(&[&a, &b]);
    assert!(w.detach("a"));
    assert!(!w.detach("a"));
    let (_, job) = counting();
    assert_eq!(w.post(&job), 1);
    assert_eq!(a.queued(), 0);
}
