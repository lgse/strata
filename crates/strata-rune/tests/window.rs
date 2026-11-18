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

    /// A later `strata-rune` hands its folder to the running one and exits, and the running
    /// one opens a window on it, as GTK's GApplication does. The process outlives its first
    /// window: GLib work still reaches the one left, and the process ends with that one.
    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_later_launch_opens_its_window_in_the_running_process() {
        let s = Session::start();
        let mut launched = launch(&s, &[]);
        let other = s.runtime_dir().join("home").join("Other");
        std::fs::create_dir_all(&other).expect("folder");
        std::fs::write(other.join("plans.txt"), "plans").expect("file");

        let mut later = s.command(env!("CARGO_BIN_EXE_strata-rune"), Display::Wayland);
        later.arg(&other);
        let mut handed = s.spawn("strata-rune-later", later);
        assert!(
            handed.wait_exit().success(),
            "the later launch handed its folder over"
        );
        let second = s.wait_window_where("the window on the later launch's folder", |w| {
            w.pid == launched.app.pid() && w.title == "Other — Strata"
        });
        launched
            .app
            .wait_for(0, "the Columns view listing the later folder", |line| {
                lists(line, 1, "Other")
            });

        s.sway_command(&format!("[con_id={}] kill", launched.window.id));
        let took = change_reaches_poll(&launched.app, &other, "later.txt");
        eprintln!("with the first window closed, a change reached poll in {took:?}");
        launched
            .app
            .wait_for(0, "the window left listing the new file", |line| {
                lists(line, 2, "Other")
            });

        s.sway_command(&format!("[con_id={}] kill", second.id));
        assert!(
            launched.app.wait_exit().success(),
            "the process ended with its last window"
        );
    }

    /// The process's one GLib context is pumped through the window with the keyboard, and what
    /// that dispatch queued for another window wakes that window: a window shown without the
    /// keyboard still lists a file added to its folder, as every GTK window does.
    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_window_without_the_keyboard_still_hears_its_folder() {
        let s = Session::start();
        let launched = launch(&s, &[]);
        let other = s.runtime_dir().join("home").join("Other");
        std::fs::create_dir_all(&other).expect("folder");
        std::fs::write(other.join("plans.txt"), "plans").expect("file");
        let mut later = s.command(env!("CARGO_BIN_EXE_strata-rune"), Display::Wayland);
        later.arg(&other);
        assert!(
            s.spawn("strata-rune-later", later).wait_exit().success(),
            "the later launch handed its folder over"
        );
        s.wait_window_where("the window on the later launch's folder", |w| {
            w.pid == launched.app.pid() && w.title == "Other — Strata"
        });
        launched
            .app
            .wait_for(0, "the Columns view listing the later folder", |line| {
                lists(line, 1, "Other")
            });

        s.focus(&launched.window);
        let took = change_reaches_poll(&launched.app, &other, "later.txt");

        eprintln!("a change in the window without the keyboard reached poll in {took:?}");
        launched.app.wait_for(
            0,
            "the window without the keyboard listing the new file",
            |line| lists(line, 2, "Other"),
        );
    }

    /// A setting one window changes reaches the other, which does not have the keyboard: Ctrl+H
    /// in the first saves the hidden-files choice, and the second follows it, as every GTK
    /// window follows the one `PreferenceManager`.
    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_setting_one_window_changes_reaches_the_other() {
        const FOLLOWED: &str = "followed another window's change of the settings";
        let s = Session::start();
        let launched = launch(&s, &[]);
        let other = s.runtime_dir().join("home").join("Other");
        std::fs::create_dir_all(&other).expect("folder");
        std::fs::write(other.join("plans.txt"), "plans").expect("file");
        let mut later = s.command(env!("CARGO_BIN_EXE_strata-rune"), Display::Wayland);
        later.arg(&other);
        assert!(
            s.spawn("strata-rune-later", later).wait_exit().success(),
            "the later launch handed its folder over"
        );
        s.wait_window_where("the window on the later launch's folder", |w| {
            w.pid == launched.app.pid() && w.title == "Other — Strata"
        });
        launched
            .app
            .wait_for(0, "the Columns view listing the later folder", |line| {
                lists(line, 1, "Other")
            });
        s.focus(&launched.window);
        // The second window comes to rest, as `change_reaches_poll` lets the listing's last
        // events arrive: one still drawing would follow without being woken.
        std::thread::sleep(Duration::from_millis(500));
        let before = launched.app.line_count();

        s.input().key("ctrl+h");

        launched
            .app
            .wait_for(before, "the other window following the setting", |line| {
                line.contains(FOLLOWED).then_some(())
            });
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

    /// The offscreen host never calls `writer::flush_at_exit()`. Here a plain launch opens home
    /// and a second tab, exits when sway closes its window, and a second plain launch restores
    /// both tabs from the flushed `tabs.toml` (`docs/rune/shared-state.md`, "Tab sessions").
    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_plain_launch_flushes_its_tabs_at_exit_and_restores_them() {
        let s = Session::start();
        let home = s.runtime_dir().join("home");
        let alpha = home.join("alpha");
        std::fs::create_dir_all(&alpha).expect("alpha folder");
        std::fs::write(alpha.join("notes.txt"), "notes").expect("file");

        let mut first = s.command(env!("CARGO_BIN_EXE_strata-rune"), Display::Wayland);
        first.env("RUST_LOG", "strata_app=trace");
        let mut app = s.spawn("strata-rune-first", first);
        let window = s.wait_window_where("the home window with focus", |w| {
            w.pid == app.pid() && w.title == "~ — Strata" && w.focused
        });
        app.wait_for(0, "the Columns view listing home", |line| {
            lists(line, 1, "home")
        });

        // The cursor starts on `alpha`: Ctrl+Return opens it in a second tab and selects it.
        s.input().key("ctrl+Return");
        s.wait_window_where("the window retitled for the second tab", |w| {
            w.id == window.id && w.title == "alpha — Strata"
        });
        app.wait_for(0, "the second tab listing alpha", |line| {
            lists(line, 1, "alpha")
        });

        s.sway_command(&format!("[con_id={}] kill", window.id));
        assert!(
            app.wait_exit().success(),
            "first plain launch exited cleanly"
        );

        let mut second = s.command(env!("CARGO_BIN_EXE_strata-rune"), Display::Wayland);
        second.env("RUST_LOG", "strata_app=trace");
        let restored = s.spawn("strata-rune-second", second);
        let reopened = s.wait_window_where("the restored window on the active tab", |w| {
            w.pid == restored.pid() && w.title == "alpha — Strata" && w.focused
        });
        restored.wait_for(0, "the restored active tab listing alpha", |line| {
            lists(line, 1, "alpha")
        });

        s.input().key("ctrl+shift+1");
        s.wait_window_where("switching to the first restored tab", |w| {
            w.id == reopened.id && w.title == "~ — Strata"
        });
    }
}
