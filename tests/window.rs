// SPDX-License-Identifier: MIT

#[path = "../crates/strata-platform/tests/support/mod.rs"]
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

    fn launch(s: &Session, folders: &[&str]) -> Launched {
        let folder = s.runtime_dir().join("home").join("Projects");
        for name in folders {
            std::fs::create_dir_all(folder.join(name)).expect("folder");
        }
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join("notes.txt"), "notes").expect("file");
        let mut command = s.command(env!("CARGO_BIN_EXE_strata"), Display::Wayland);
        command.arg(&folder).env("RUST_LOG", "strata_app=trace");
        let app = s.spawn("strata", command);
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

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_later_launch_opens_its_window_in_the_running_process() {
        let s = Session::start();
        let mut launched = launch(&s, &[]);
        let other = s.runtime_dir().join("home").join("Other");
        std::fs::create_dir_all(&other).expect("folder");
        std::fs::write(other.join("plans.txt"), "plans").expect("file");

        let mut later = s.command(env!("CARGO_BIN_EXE_strata"), Display::Wayland);
        later.arg(&other);
        let mut handed = s.spawn("strata-later", later);
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

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_window_without_the_keyboard_still_hears_its_folder() {
        let s = Session::start();
        let launched = launch(&s, &[]);
        let other = s.runtime_dir().join("home").join("Other");
        std::fs::create_dir_all(&other).expect("folder");
        std::fs::write(other.join("plans.txt"), "plans").expect("file");
        let mut later = s.command(env!("CARGO_BIN_EXE_strata"), Display::Wayland);
        later.arg(&other);
        assert!(
            s.spawn("strata-later", later).wait_exit().success(),
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

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_setting_one_window_changes_reaches_the_other() {
        const FOLLOWED: &str = "followed another window's change of the settings";
        let s = Session::start();
        let launched = launch(&s, &[]);
        let other = s.runtime_dir().join("home").join("Other");
        std::fs::create_dir_all(&other).expect("folder");
        std::fs::write(other.join("plans.txt"), "plans").expect("file");
        let mut later = s.command(env!("CARGO_BIN_EXE_strata"), Display::Wayland);
        later.arg(&other);
        assert!(
            s.spawn("strata-later", later).wait_exit().success(),
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
        std::thread::sleep(Duration::from_millis(500));
        let before = launched.app.line_count();

        s.input().key("ctrl+h");

        launched
            .app
            .wait_for(before, "the other window following the setting", |line| {
                line.contains(FOLLOWED).then_some(())
            });
    }

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_close_the_window_manager_asks_for_waits_for_a_running_job() {
        const REFUSED: &str = "the window manager's close was refused";
        let s = Session::start();
        let config = s.runtime_dir().join("home").join(".config").join("strata");
        let action = config.join("actions").join("wait");
        std::fs::create_dir_all(&action).expect("the action's folder");
        std::fs::write(
            action.join("action.toml"),
            "schema_version = 1\nid = \"wait\"\nname = \"Wait\"\nmenu = \"top\"\n[when]\nkinds = [\"file\", \"folder\"]\n[run]\nruntime = \"command\"\nprogram = \"/bin/sleep\"\nargs = [\"3\"]\n",
        )
        .expect("action.toml");
        std::fs::write(
            config.join("settings.toml"),
            "[keymap.default]\n\"custom_action.1\" = [\"F9\"]\n",
        )
        .expect("settings.toml");
        let mut launched = launch(&s, &[]);
        // The custom actions load on the window's first poll.
        std::thread::sleep(Duration::from_millis(1000));
        let before = launched.app.line_count();
        s.input().key("F9");
        launched.app.wait_for(before, "the job queued", |line| {
            line.contains("a custom action's job was queued")
                .then_some(())
        });

        let before = launched.app.line_count();
        s.sway_command(&format!("[con_id={}] kill", launched.window.id));
        launched.app.wait_for(before, "the close refused", |line| {
            line.contains(REFUSED).then_some(())
        });
        let closed = launched.window.id;
        let reopened = s.wait_window_where("the window open again", |w| {
            w.pid == launched.app.pid() && w.id != closed && w.title == "Projects — Strata"
        });

        std::thread::sleep(Duration::from_millis(3500));
        s.sway_command(&format!("[con_id={}] kill", reopened.id));
        assert!(
            launched.app.wait_exit().success(),
            "with the job done, the close ends the process"
        );
    }

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

    #[test]
    #[ignore = "needs a private session: mise run test-platform"]
    fn a_plain_launch_flushes_its_tabs_at_exit_and_restores_them() {
        let s = Session::start();
        let home = s.runtime_dir().join("home");
        let alpha = home.join("alpha");
        std::fs::create_dir_all(&alpha).expect("alpha folder");
        std::fs::write(alpha.join("notes.txt"), "notes").expect("file");

        let mut first = s.command(env!("CARGO_BIN_EXE_strata"), Display::Wayland);
        first.env("RUST_LOG", "strata_app=trace");
        let mut app = s.spawn("strata-first", first);
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

        let mut second = s.command(env!("CARGO_BIN_EXE_strata"), Display::Wayland);
        second.env("RUST_LOG", "strata_app=trace");
        let restored = s.spawn("strata-second", second);
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
