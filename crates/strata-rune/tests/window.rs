// SPDX-License-Identifier: MIT

//! The windowed binary in a private headless sway (`mise run test-platform window::`).
//!
//! The offscreen host cannot show whether GLib work reaches a window: there Rune runs
//! winit's loop, and nothing else iterates the GLib context. These tests start `strata-rune`
//! on a folder, change the folder, and wait for the browser's events to reach the interface,
//! which logs them at trace level, and for the Columns view to list what the folder holds,
//! which it logs too. The second covers the window first: upstream Rune #139 is a covered
//! Wayland window whose `poll` and timers stop. `window::qa` drives the QA mode's session
//! host, a real window that serves the runner's protocol.
//!
//! Nothing here looks at the picture: that the rows are drawn is the owner's hand check.

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
    #[cfg(feature = "qa")]
    mod qa;

    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use crate::support::{Display, Proc, Session, Window};

    const HEARD: &str = "browser events reached the interface";

    fn heard(line: &str) -> Option<()> {
        line.contains(HEARD).then_some(())
    }

    /// Whether `line` is the Columns view listing `rows` rows of the folder named `folder`.
    fn lists(line: &str, rows: usize, folder: &str) -> Option<()> {
        line.contains(&format!("the Columns view lists {rows} rows of {folder}"))
            .then_some(())
    }

    struct Launched {
        app: Proc,
        window: Window,
        folder: PathBuf,
    }

    /// Starts `strata-rune` on a folder of the session's own, which holds `folders` and one
    /// file, and waits for its listing.
    fn launch(s: &Session, folders: &[&str]) -> Launched {
        let folder = s.runtime_dir().join("home").join("Projects");
        for name in folders {
            std::fs::create_dir_all(folder.join(name)).expect("folder");
        }
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join("notes.txt"), "notes").expect("file");
        let mut command = s.command(env!("CARGO_BIN_EXE_strata-rune"), Display::Wayland);
        command.arg(&folder).env("RUST_LOG", "strata_app=trace");
        let app = s.spawn("strata-rune", command);
        let window = s.wait_window_where("the window titled for its folder", |w| {
            w.pid == app.pid() && w.title == "Projects — Strata"
        });
        app.wait_for(0, "the listing's events", heard);
        app.wait_for(0, "the Columns view listing the folder", |line| {
            lists(line, folders.len() + 1, "Projects")
        });
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
        let launched = launch(&s, &[]);

        assert_eq!(
            launched.window.app_id.as_deref(),
            Some("io.github.lgse.Strata.Rune")
        );
        let took = change_reaches_poll(&launched.app, &launched.folder, "shown.txt");
        eprintln!("a change reached poll in {took:?}");
        let app = &launched.app;
        app.wait_for(0, "the Columns view listing the new file", |line| {
            lists(line, 2, "Projects")
        });
    }

    /// The offscreen host feeds keys to Rune directly. Here one crosses the compositor and
    /// winit, which shows three things only a window can: the listing has the keyboard from
    /// the start, the key reaches the Columns view, and the wait before the child column
    /// opens, a GLib timer a handler started, ends while Rune's loop is idle.
    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_key_moves_the_cursor_and_the_child_column_follows() {
        let s = Session::start();
        let launched = launch(&s, &["alpha", "beta"]);
        std::fs::write(launched.folder.join("beta").join("inside.txt"), "inside").expect("file");
        s.wait_window_where("the window with the keyboard", |w| {
            w.id == launched.window.id && w.focused
        });
        let before = launched.app.line_count();

        // The cursor starts on `alpha`, the first entry.
        s.input().key("Down");

        launched.app.wait_for(
            before,
            "the column of the folder under the cursor",
            |line| lists(line, 1, "beta"),
        );
    }

    /// With a `wake_after` tick in place of the embedded GLib loop, a change took 120 ms shown
    /// and 750 ms covered; a `Waker` alone never delivered it.
    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_cover_does_not_slow_a_folder_change() {
        let s = Session::start();
        let launched = launch(&s, &[]);
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
