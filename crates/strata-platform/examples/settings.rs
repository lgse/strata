//! Prints the desktop's settings and every change to them, as JSON lines on stdout (the
//! `settings` feature).
//!
//! `--no-window` runs a [`Watcher`] alone, prints what its sink hears, and exits when stdin
//! closes:
//!
//! - `{"ev":"snapshot","settings":{…}}` once, first;
//! - `{"ev":"changed","key":"text-scaling-factor","settings":{…}}` for each change;
//! - `{"ev":"fallback","kind":"no-portal","key":null,"detail":"…"}` the first time each
//!   fallback happens.
//!
//! By default it opens a window that shows the values, fed by `settings::follow`. It logs
//! `{"ev":"landed"}` each time an update `follow` posts runs on Rune's loop (just before it
//! stores the value), `{"ev":"view","settings":{…}}` each time the view runs, and `ready` and
//! `frame`.
//!
//! `--first-answer-ms N` and `--timeout-ms N` set the watcher's [`Options`] (defaults 100 and
//! 10000); the window ignores them, since `follow` takes the defaults.
//!
//! `cargo run -p strata-platform --example settings --features settings` reads the desktop's
//! portal. The session tests run it on a private bus only (`tests/session/p5.rs`).
//!
//! [`Watcher`]: strata_platform::settings::Watcher
//! [`Options`]: strata_platform::settings::Options

#[cfg(feature = "settings")]
mod demo {
    use std::io::{BufRead, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use rune::{App, AppSpec, Cx, El, WindowHandle, col, text};
    use serde_json::{Value, json};
    use strata_platform::settings::{
        self, DesktopSettings, Event, Fallback, Key, Options, Watcher,
    };

    fn log(event: &str, fields: Value) {
        let mut line = json!({ "ev": event });
        if let (Some(obj), Value::Object(fields)) = (line.as_object_mut(), fields) {
            obj.extend(fields);
        }
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    fn settings_json(s: &DesktopSettings) -> Value {
        let known: Vec<&str> = Key::ALL
            .into_iter()
            .filter(|k| s.is_known(*k))
            .map(Key::name)
            .collect();
        json!({
            "text_scale": s.text_scale,
            "double_click_ms": s.double_click.as_millis() as u64,
            "drag_threshold": s.drag_threshold,
            "cursor_blink": s.cursor_blink,
            "cursor_blink_time_ms": s.cursor_blink_time.as_millis() as u64,
            "cursor_blink_timeout_s": s.cursor_blink_timeout.as_secs(),
            "enable_animations": s.enable_animations,
            "reduced_motion": s.reduced_motion,
            "remember_recent_files": s.remember_recent_files,
            "animations": s.animations(),
            "known": known,
        })
    }

    fn print(event: &Event) {
        match event {
            Event::Snapshot(s) => log("snapshot", json!({ "settings": settings_json(s) })),
            Event::Changed { setting, settings } => log(
                "changed",
                json!({ "key": setting.key().name(), "settings": settings_json(settings) }),
            ),
            Event::Fallback(f) => {
                let (kind, key, detail) = match f {
                    Fallback::NoBus(d) => ("no-bus", None, d.clone()),
                    Fallback::NoPortal(d) => ("no-portal", None, d.clone()),
                    Fallback::Unset(k) => ("unset", Some(k.name()), String::new()),
                    Fallback::Invalid { key, value } => {
                        ("invalid", Some(key.name()), value.clone())
                    }
                    Fallback::PortalLost => ("portal-lost", None, String::new()),
                    other => ("other", None, format!("{other:?}")),
                };
                log(
                    "fallback",
                    json!({ "kind": kind, "key": key, "detail": detail }),
                );
            }
            other => log("other", json!({ "event": format!("{other:?}") })),
        }
    }

    fn number(args: &[String], flag: &str) -> Option<u64> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .and_then(|n| n.parse().ok())
    }

    fn options(args: &[String]) -> Options {
        let mut options = Options::default();
        if let Some(ms) = number(args, "--first-answer-ms") {
            options = options.first_answer(Duration::from_millis(ms));
        }
        if let Some(ms) = number(args, "--timeout-ms") {
            options = options.timeout(Duration::from_millis(ms));
        }
        options
    }

    fn no_window(options: Options) {
        let watcher = match Watcher::start(options, |event| print(&event)) {
            Ok(watcher) => watcher,
            Err(error) => {
                log("exit-error", json!({ "error": error.to_string() }));
                std::process::exit(1);
            }
        };
        log(
            "started",
            json!({ "settings": settings_json(&watcher.snapshot()) }),
        );
        for line in std::io::stdin().lock().lines() {
            if line.is_err() {
                break;
            }
        }
        drop(watcher);
        log("stopped", json!({}));
    }

    #[derive(Default)]
    struct State {
        desktop: DesktopSettings,
    }

    /// `follow`'s accessor: it runs inside each update `follow` posts, on Rune's loop.
    fn desktop(s: &mut State) -> &mut DesktopSettings {
        log("landed", json!({}));
        &mut s.desktop
    }

    fn view(s: &State, _: &mut Cx<State>) -> El {
        let d = &s.desktop;
        log("view", json!({ "settings": settings_json(d) }));
        col()
            .w_full()
            .h_full()
            .center()
            .child(text(format!("text scale {}", d.text_scale)))
            .child(text(format!(
                "double click {} ms",
                d.double_click.as_millis()
            )))
            .child(text(format!("drag threshold {} px", d.drag_threshold)))
            .child(text(format!(
                "caret blink {} every {} ms for {} s",
                d.cursor_blink,
                d.cursor_blink_time.as_millis(),
                d.cursor_blink_timeout.as_secs()
            )))
            .child(text(format!(
                "animations {} (switch {}, reduced motion {})",
                d.animations(),
                d.enable_animations,
                d.reduced_motion
            )))
            .child(text(format!(
                "remember recent files {}",
                d.remember_recent_files
            )))
    }

    fn window() {
        rune::tooling::set_app_id("strata-settings-example");
        let app = App::new(AppSpec::new(
            "settings",
            (480.0, 280.0),
            State::default,
            view,
        ));
        let watcher = match settings::follow(&app.handle(), desktop) {
            Ok(watcher) => watcher,
            Err(error) => {
                log("exit-error", json!({ "error": error.to_string() }));
                std::process::exit(1);
            }
        };
        // Seeded with what the watcher has, so a snapshot that matches it re-runs nothing.
        let first = watcher.snapshot();
        static DRAWN: AtomicBool = AtomicBool::new(false);
        let result = app
            .init(move || State { desktop: first })
            .on_ready(|_: &WindowHandle| log("ready", json!({})))
            .on_frame_drawn(|_| {
                if !DRAWN.swap(true, Ordering::Relaxed) {
                    log("frame", json!({}));
                }
            })
            .try_run();
        drop(watcher);
        if let Err(error) = result {
            log("exit-error", json!({ "error": error }));
            std::process::exit(1);
        }
    }

    pub fn main() {
        let args = rune::tooling::extra_args();
        if args.iter().any(|a| a == "--no-window") {
            no_window(options(args));
        } else {
            window();
        }
    }
}

#[cfg(feature = "settings")]
fn main() {
    demo::main();
}

#[cfg(not(feature = "settings"))]
fn main() {
    eprintln!("run with --features settings");
}
