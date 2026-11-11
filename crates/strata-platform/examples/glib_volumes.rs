//! The mounted volumes, listed through the GLib bridge and kept current as mounts come and go.
//!
//!   cargo run -p strata-platform --features glib-bridge --example glib_volumes
//!
//! This talks to the desktop's real volume monitor (and so to gvfsd on the session bus), unlike
//! the bridge's tests.

#[cfg(feature = "glib-bridge")]
mod demo {
    use gio::prelude::*;
    use rune::{AppSpec, Color, Cx, El, TextStyle, Tx, Waker, col, oklch, text};
    use strata_platform::glib_bridge::{GlibThread, Keep, Signal, Task};

    #[derive(Default)]
    pub struct State {
        bridge: Option<GlibThread>,
        listing: Option<Task<Vec<String>>>,
        changes: Option<Signal<()>>,
        mounts: Vec<String>,
        error: Option<String>,
    }

    fn list_mounts(bridge: &GlibThread) -> Task<Vec<String>> {
        bridge.handle().spawn(|| async {
            gio::VolumeMonitor::get()
                .mounts()
                .iter()
                .map(|mount| mount.name().to_string())
                .collect()
        })
    }

    fn setup(s: &mut State, waker: Waker) {
        let bridge = match GlibThread::start(waker) {
            Ok(bridge) => bridge,
            Err(e) => {
                s.error = Some(e.to_string());
                return;
            }
        };
        // The volume monitor's first `get` is on the bridge, so its signals arrive there.
        s.changes = Some(bridge.handle().listen(|emit| {
            let monitor = gio::VolumeMonitor::get();
            let removed = emit.clone();
            let added = monitor.connect_mount_added(move |_, _| {
                let _ = emit.emit(());
            });
            let gone = monitor.connect_mount_removed(move |_, _| {
                let _ = removed.emit(());
            });
            Keep::new().handler(&monitor, added).handler(&monitor, gone)
        }));
        s.listing = Some(list_mounts(&bridge));
        s.bridge = Some(bridge);
    }

    fn poll(s: &mut State, _: f64, _: &mut Tx) -> bool {
        let mut changed = false;
        if let Some(mounts) = s.listing.as_ref().and_then(|task| task.take()) {
            s.mounts = mounts;
            changed = true;
        }
        let mounts_changed = s.changes.as_ref().is_some_and(|c| c.drain().count() > 0);
        if let (true, Some(bridge)) = (mounts_changed, &s.bridge) {
            s.listing = Some(list_mounts(bridge));
        }
        changed
    }

    fn view(s: &State, _: &mut Cx<State>) -> El {
        let line = |content: String| {
            text(content)
                .font(TextStyle::new(14.0, 400.0))
                .color(Color::WHITE)
        };
        let lines: Vec<El> = match (&s.error, s.mounts.is_empty()) {
            (Some(error), _) => vec![line(format!("No GLib bridge: {error}"))],
            (None, true) => vec![line("No mounts".into())],
            (None, false) => s.mounts.iter().cloned().map(line).collect(),
        };
        col()
            .w_full()
            .h_full()
            .fill(oklch(0.18, 0.01, 250.0))
            .children(lines)
    }

    pub fn main() {
        rune::run(AppSpec {
            setup: Some(setup),
            poll: Some(poll),
            ..AppSpec::new("Mounts", (360.0, 240.0), State::default, view)
        });
    }
}

#[cfg(feature = "glib-bridge")]
fn main() {
    demo::main();
}

#[cfg(not(feature = "glib-bridge"))]
fn main() {
    eprintln!("run with --features glib-bridge");
}
