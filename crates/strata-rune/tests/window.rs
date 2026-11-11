// SPDX-License-Identifier: MIT

//! The windowed binary in a private headless sway (`mise run test-platform window::`).
//!
//! The offscreen host cannot show whether GLib work reaches a window: there Rune runs
//! winit's loop, and nothing else iterates the GLib context. These tests start `strata-rune`
//! on a folder, change the folder, and wait for the browser's events to reach the interface,
//! which logs them at trace level. The second covers the window first: upstream Rune #139 is
//! a covered Wayland window whose `poll` and timers stop.

#[path = "../../strata-platform/tests/support/mod.rs"]
#[expect(
    dead_code,
    reason = "W3's harness, shared with strata-platform's session tests: these use part of it"
)]
#[expect(
    unexpected_cfgs,
    unused_imports,
    reason = "the harness gates helpers on strata-platform's features, which this crate lacks"
)]
mod support;

mod window {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use crate::support::{Display, Proc, Session, Window};

    const HEARD: &str = "browser events reached the interface";

    fn heard(line: &str) -> Option<()> {
        line.contains(HEARD).then_some(())
    }

    struct Launched {
        app: Proc,
        window: Window,
        folder: PathBuf,
    }

    /// Starts `strata-rune` on a folder of the session's own, and waits for its listing.
    fn launch(s: &Session) -> Launched {
        let folder = s.runtime_dir().join("home").join("Projects");
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join("notes.txt"), "notes").expect("file");
        let mut command = s.command(env!("CARGO_BIN_EXE_strata-rune"), Display::Wayland);
        command.arg(&folder).env("RUST_LOG", "strata_app=trace");
        let app = s.spawn("strata-rune", command);
        let window = s.wait_window_where("the window titled for its folder", |w| {
            w.pid == app.pid() && w.title == "Projects — Strata"
        });
        app.wait_for(0, "the listing's events", heard);
        Launched {
            app,
            window,
            folder,
        }
    }

    /// Creates a file in `folder` and returns how long its change took to reach `poll`.
    fn change_reaches_poll(app: &Proc, folder: &Path, name: &str) -> Duration {
        // The listing's last events may still be on their way.
        std::thread::sleep(Duration::from_millis(500));
        let before = app.line_count();
        let changed = Instant::now();
        std::fs::write(folder.join(name), name).expect("file");
        app.wait_for(before, &format!("the change of {name}"), heard);
        changed.elapsed()
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_folder_change_reaches_the_window() {
        let s = Session::start();
        let launched = launch(&s);

        assert_eq!(
            launched.window.app_id.as_deref(),
            Some("io.github.lgse.Strata.Rune")
        );
        let took = change_reaches_poll(&launched.app, &launched.folder, "shown.txt");
        eprintln!("a change reached poll in {took:?}");
    }

    /// With a `wake_after` tick in place of the embedded GLib loop, a change took 120 ms shown
    /// and 750 ms covered; a `Waker` alone never delivered it.
    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_cover_does_not_slow_a_folder_change() {
        let s = Session::start();
        let launched = launch(&s);
        let shown = change_reaches_poll(&launched.app, &launched.folder, "shown.txt");
        let cover = s.peer(Display::Wayland);
        cover.map("cover");
        let covering = s.wait_window(cover.pid());
        s.sway_command(&format!("[con_id={}] fullscreen enable", covering.id));
        s.wait_window_where("the cover in front", |w| w.id == covering.id && w.focused);

        let covered = change_reaches_poll(&launched.app, &launched.folder, "covered.txt");

        eprintln!("a change reached poll in {shown:?} shown and {covered:?} covered");
        assert!(
            covered < shown + Duration::from_millis(300),
            "{covered:?} covered, {shown:?} shown"
        );
    }
}
