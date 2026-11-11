use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::{IconLoader, IconState, LoadedIcon, Options};
use crate::icon_theme::request::IconRequest;
use crate::icon_theme::resolver::SearchPath;
use crate::icon_theme::test_support::{Scratch, fixtures};

const WAIT: Duration = Duration::from_secs(10);

/// A loader over `roots`, starting in `theme`, and how often its waker has been called.
fn loader(roots: Vec<std::path::PathBuf>, theme: &str) -> (IconLoader, Arc<AtomicUsize>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let waker: rune::Waker = Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let loader =
        IconLoader::start(waker, options(roots, theme)).unwrap_or_else(|e| panic!("start: {e}"));
    (loader, wakes)
}

fn options(roots: Vec<std::path::PathBuf>, theme: &str) -> Options {
    Options {
        search_path: SearchPath {
            icon_dirs: roots,
            pixmap_dirs: Vec::new(),
        },
        theme: theme.into(),
        ..Options::default()
    }
}

/// A waker that counts its calls and, while shut, holds the thread that calls it. The lookup
/// thread calls the waker after each lookup settles, so a shut gate holds it there while a test
/// queues work behind it or drops the loader.
#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

#[derive(Default)]
struct GateState {
    shut: bool,
    held: usize,
    wakes: usize,
}

impl Gate {
    fn waker(self: &Arc<Self>) -> rune::Waker {
        let gate = self.clone();
        Arc::new(move || {
            let mut state = gate.lock();
            state.wakes += 1;
            state.held += 1;
            gate.changed.notify_all();
            while state.shut {
                state = gate
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            state.held -= 1;
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_shut(&self, shut: bool) {
        self.lock().shut = shut;
        self.changed.notify_all();
    }

    fn wakes(&self) -> usize {
        self.lock().wakes
    }

    /// Waits until a thread is held in the waker.
    fn holding(&self) -> bool {
        let deadline = Instant::now() + WAIT;
        let mut state = self.lock();
        while state.held == 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

fn gated(options: Options) -> (IconLoader, Arc<Gate>) {
    let gate = Arc::new(Gate::default());
    let loader = IconLoader::start(gate.waker(), options).unwrap_or_else(|e| panic!("start: {e}"));
    (loader, gate)
}

/// Holds the lookup thread: it settles a lookup of a missing icon, then waits in the waker.
/// Lookups run in order, so every job queued before has finished, its waker call included.
fn hold(icons: &IconLoader, gate: &Gate) {
    static HELD: AtomicUsize = AtomicUsize::new(0);
    let name = format!("held-{}", HELD.fetch_add(1, Ordering::Relaxed));
    gate.set_shut(true);
    let held = icons.load(&IconRequest::named([name], 22, 1.0));
    assert_eq!(held.wait(WAIT), IconState::NotFound);
    assert!(gate.holding(), "the lookup thread reached the waker");
}

fn missing(i: usize) -> IconRequest {
    IconRequest::named([format!("missing-{i}")], 22, 1.0)
}

fn decode_loader() -> (IconLoader, Arc<AtomicUsize>) {
    loader(vec![fixtures().join("decode")], "Decode")
}

/// Waits until the waker has been called `n` times. The lookup calls it last, after the
/// state `poll` reads; a found icon's decode calls it once more, in either order.
fn woken(wakes: &AtomicUsize, n: usize) -> bool {
    let deadline = Instant::now() + WAIT;
    while wakes.load(Ordering::SeqCst) < n {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::yield_now();
    }
    true
}

/// Waits for `poll` to turn true, as an app's `poll` would see it.
fn polled(loader: &IconLoader) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if loader.poll() {
            return true;
        }
        std::thread::yield_now();
    }
    false
}

#[test]
fn a_found_svg_wakes_the_app_and_decodes_at_size_times_scale() {
    let (icons, wakes) = decode_loader();
    let icon = icons.load(&IconRequest::named(["good"], 22, 2.0));
    assert_eq!(icon.wait(WAIT), IconState::Ready);
    assert!(
        woken(&wakes, 2),
        "the lookup and the decode each wake the app"
    );
    assert!(icons.poll());
    assert_eq!(icon.image().and_then(rune::Image::size), Some([44, 44]));
    assert!(icon.pixels().is_some());
    assert!(!icon.is_symbolic());
    assert!(
        icon.path()
            .is_some_and(|p| p.ends_with("Decode/scalable/apps/good.svg"))
    );
    // Nothing settled since.
    assert!(!icons.poll());
}

#[test]
fn a_png_decodes_at_its_own_size() {
    let (icons, _) = decode_loader();
    let icon = icons.load(&IconRequest::named(["good-png"], 48, 1.0));
    assert_eq!(icon.wait(WAIT), IconState::Ready);
    assert_eq!(icon.image().and_then(rune::Image::size), Some([16, 16]));
}

#[test]
fn a_missing_icon_settles_as_not_found_and_wakes() {
    let (icons, wakes) = decode_loader();
    let icon = icons.load(&IconRequest::named(
        ["nothing", "nothing-symbolic"],
        22,
        1.0,
    ));
    assert_eq!(icon.wait(WAIT), IconState::NotFound);
    assert!(woken(&wakes, 1));
    assert!(icons.poll());
    assert!(!icons.poll());
    assert!(icon.pixels().is_none() && icon.path().is_none());
}

#[test]
fn a_broken_file_fails_without_trying_another() {
    let (icons, _) = decode_loader();
    for name in ["bad", "bad-svg"] {
        let icon = icons.load(&IconRequest::named([name, "good"], 22, 1.0));
        assert!(
            matches!(icon.wait(WAIT), IconState::Failed(_)),
            "{name}: {icon:?}"
        );
    }
    let missing = icons.load(&IconRequest::file(
        fixtures().join("decode/none.png"),
        22,
        1.0,
    ));
    assert!(matches!(missing.wait(WAIT), IconState::Failed(_)));
}

#[test]
fn a_file_over_the_limit_is_refused() {
    let wakes: rune::Waker = Arc::new(|| {});
    let options = Options {
        max_file_bytes: 8,
        search_path: SearchPath::default(),
        ..Options::default()
    };
    let icons = IconLoader::start(wakes, options).unwrap_or_else(|e| panic!("start: {e}"));
    let path = fixtures().join("decode/Decode/scalable/apps/good.svg");
    let icon = icons.load(&IconRequest::file(path, 22, 1.0));
    assert!(matches!(icon.wait(WAIT), IconState::Failed(_)));
    assert!(icon.image().is_none());
}

#[test]
fn a_symbolic_icon_says_so() {
    let (icons, _) = decode_loader();
    let icon = icons.load(&IconRequest::named(["good-symbolic", "good"], 16, 1.0));
    assert_eq!(icon.wait(WAIT), IconState::Ready);
    assert!(icon.is_symbolic());
}

#[test]
fn the_same_request_shares_one_handle_and_recent_ones_stay() {
    let (icons, _) = decode_loader();
    let request = IconRequest::named(["good"], 22, 1.0);
    let first = icons.load(&request);
    first.wait(WAIT);
    let again = icons.load(&request);
    assert!(Arc::ptr_eq(&first.0, &again.0));
    drop((first, again));
    // No handle left: the recent icons kept it.
    let kept = icons.load(&request);
    assert_eq!(kept.state(), IconState::Ready);
    // Another size or scale is another icon.
    let other = icons.load(&IconRequest::named(["good"], 22, 2.0));
    assert!(!Arc::ptr_eq(&kept.0, &other.0));
}

#[test]
fn an_icon_too_large_for_the_recent_ones_goes_when_its_handles_do() {
    let (icons, _) = decode_loader();
    let request = IconRequest::named(["good"], 200, 2.0);
    let first = icons.load(&request);
    first.wait(WAIT);
    let weak = Arc::downgrade(&first.0);
    drop(first);
    assert!(weak.upgrade().is_none());
}

#[test]
fn the_recent_icons_count_what_decoded_not_what_was_asked() {
    // Room for 2 icons of 8 px: 128 pixels. good-png asked at 8 decodes at its own 16x16.
    let (icons, _) = gated(Options {
        lru_entries: 2,
        lru_max_px: 8,
        ..options(vec![fixtures().join("decode")], "Decode")
    });
    let icon = icons.load(&IconRequest::named(["good-png"], 8, 1.0));
    assert_eq!(icon.wait(WAIT), IconState::Ready);
    let png = Arc::downgrade(&icon.0);
    drop(icon);
    // The most recent icon stays even over the limit, so a view showing it alone doesn't look
    // it up again each run.
    assert!(png.upgrade().is_some());
    let icon = icons.load(&IconRequest::named(["good"], 8, 1.0));
    assert_eq!(icon.wait(WAIT), IconState::Ready);
    let svg = Arc::downgrade(&icon.0);
    drop(icon);
    assert!(
        png.upgrade().is_none(),
        "256 pixels don't fit beside another icon"
    );
    assert!(svg.upgrade().is_some());
}

#[test]
fn set_theme_changes_later_lookups_only() {
    let root = fixtures();
    let (icons, _) = loader(vec![root.join("user"), root.join("system")], "Child");
    let request = IconRequest::named(["order"], 48, 1.0);
    let child = icons.load(&request);
    child.wait(WAIT);
    icons.set_theme("hicolor");
    assert_eq!(icons.theme(), "hicolor");
    let hicolor = icons.load(&request);
    hicolor.wait(WAIT);
    // The fixture files are empty, so they fail to decode; the paths are what matter.
    assert!(
        child
            .path()
            .is_some_and(|p| p.ends_with("Grand/48x48/apps/order.png"))
    );
    assert!(
        hicolor
            .path()
            .is_some_and(|p| p.ends_with("hicolor/48x48/apps/order.png"))
    );
    assert_eq!((child.theme(), hicolor.theme()), ("Child", "hicolor"));
}

#[test]
fn rescan_finds_a_new_icon_and_drops_stale_handles() {
    let scratch = Scratch::new("loader");
    scratch.put("T/index.theme", "[Icon Theme]\nDirectories=scalable/apps\n[scalable/apps]\nSize=16\nType=Scalable\nMinSize=1\nMaxSize=512\n");
    scratch.put("T/scalable/apps/first.svg", "");
    let (icons, wakes) = loader(vec![scratch.root().to_path_buf()], "T");
    let request = IconRequest::named(["late"], 22, 1.0);
    let before = icons.load(&request);
    assert_eq!(before.wait(WAIT), IconState::NotFound);
    assert!(woken(&wakes, 1));
    assert!(icons.poll());
    // Nothing changed: a rescan keeps the handles and doesn't wake the view.
    icons.rescan();
    let same = icons.load(&request);
    same.wait(WAIT);
    assert!(Arc::ptr_eq(&before.0, &same.0));
    assert!(!icons.poll());
    let svg = std::fs::read_to_string(fixtures().join("decode/Decode/scalable/apps/good.svg"));
    scratch.put(
        "T/scalable/apps/late.svg",
        svg.unwrap_or_else(|e| panic!("good.svg: {e}")),
    );
    icons.rescan();
    assert!(polled(&icons), "a change wakes the view");
    let after = icons.load(&request);
    assert_eq!(after.wait(WAIT), IconState::Ready);
    // The handle kept from before keeps what it found.
    assert_eq!(before.state(), IconState::NotFound);
}

/// Waits until the gate's waker has been called `n` times.
fn gate_woken(gate: &Gate, n: usize) -> bool {
    let deadline = Instant::now() + WAIT;
    while gate.wakes() < n {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::yield_now();
    }
    true
}

#[test]
fn an_icon_shown_before_a_change_on_disk_keeps_its_pixels_until_the_new_lookup_lands() {
    let decode = fixtures().join("decode/Decode");
    let read = |p: &str| std::fs::read(decode.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
    let scratch = Scratch::new("no-flash");
    scratch.put("T/index.theme", "[Icon Theme]\nDirectories=scalable/apps\n[scalable/apps]\nSize=16\nType=Scalable\nMinSize=1\nMaxSize=512\n");
    scratch.put("T/scalable/apps/icon.svg", read("scalable/apps/good.svg"));
    let (icons, gate) = gated(options(vec![scratch.root().to_path_buf()], "T"));
    let request = IconRequest::named(["icon"], 22, 1.0);
    let first = icons.load(&request);
    assert_eq!(first.wait(WAIT), IconState::Ready);
    // The lookup and the decode: nothing else will call the waker but the thread.
    assert!(gate_woken(&gate, 2));
    let shown = |icons: &IconLoader| {
        let icon = icons.load(&request);
        assert_eq!(icon.state(), IconState::Ready, "{icon:?}");
        icon
    };
    // Something else in the theme changes; then `icon` itself gains a PNG, which beats its SVG.
    let changes: [(&str, Vec<u8>, &str); 2] = [
        (
            "T/scalable/apps/other.svg",
            read("scalable/apps/good.svg"),
            "icon.svg",
        ),
        (
            "T/scalable/apps/icon.png",
            read("16x16/apps/good-png.png"),
            "icon.png",
        ),
    ];
    let mut before = first;
    for (path, bytes, found) in changes {
        icons.poll();
        gate.set_shut(true);
        scratch.put(path, bytes);
        icons.rescan();
        // The thread has seen the change and is held before any new lookup runs.
        assert!(gate.holding());
        assert!(icons.poll(), "a change wakes the view");
        assert!(Arc::ptr_eq(&shown(&icons).0, &before.0), "{path}");
        gate.set_shut(false);
        let deadline = Instant::now() + WAIT;
        let after = loop {
            let icon = shown(&icons);
            if !Arc::ptr_eq(&icon.0, &before.0) {
                break icon;
            }
            assert!(Instant::now() < deadline, "{path}: no new handle");
            icons.poll();
            std::thread::yield_now();
        };
        assert!(
            after.path().is_some_and(|p| p.ends_with(found)),
            "{after:?}"
        );
        before = after;
    }
}

#[test]
fn dropping_the_loader_stops_its_thread_before_the_queued_lookups_and_keeps_what_was_found() {
    let (icons, gate) = gated(options(vec![fixtures().join("decode")], "Decode"));
    let icon = icons.load(&IconRequest::named(["good"], 22, 1.0));
    assert_eq!(icon.wait(WAIT), IconState::Ready);
    hold(&icons, &gate);
    // Held handles, so only the stop can skip them.
    let queued: Vec<LoadedIcon> = (0..50).map(|i| icons.load(&missing(i))).collect();
    let alive = Arc::downgrade(&icons.inner.common);
    drop(icons);
    gate.set_shut(false);
    let deadline = Instant::now() + WAIT;
    while alive.strong_count() > 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(alive.strong_count(), 0, "the thread let go of its state");
    let looked_up = queued
        .iter()
        .filter(|i| i.state() != IconState::Pending)
        .count();
    assert_eq!(looked_up, 0, "no lookup ran after the loader dropped");
    assert_eq!(icon.state(), IconState::Ready);
}

#[test]
fn a_request_dropped_and_pushed_out_of_the_recent_icons_before_its_turn_is_skipped() {
    let (icons, gate) = gated(Options {
        lru_entries: 4,
        ..options(vec![fixtures().join("decode")], "Decode")
    });
    hold(&icons, &gate);
    let before = gate.wakes();
    for i in 0..10 {
        drop(icons.load(&missing(i)));
    }
    let last = icons.load(&IconRequest::named(["last"], 22, 1.0));
    gate.set_shut(false);
    assert_eq!(last.wait(WAIT), IconState::NotFound);
    hold(&icons, &gate);
    let looked_up = gate.wakes() - before - 1;
    gate.set_shut(false);
    // Each lookup of a missing icon wakes once: the 3 recent icons left besides `last`, and
    // `last`. The other 7 left the recent icons with no handle before their turn.
    assert_eq!(looked_up, 3 + 1);
}
