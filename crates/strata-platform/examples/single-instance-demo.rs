//! One instance of a Rune app, with a window per launch (P8).
//!
//! `cargo run -p strata-platform --features single-instance --example single-instance-demo a b`
//! opens a window listing `a b`. Run it again with `c` while that one runs: the second process
//! hands its launch to the first and exits, and the first opens a second window, focused when
//! the launcher gave an activation token (`XDG_ACTIVATION_TOKEN`, `DESKTOP_STARTUP_ID`). Run it
//! with no arguments to bring the newest window forward instead. The app quits when its last
//! window closes.
//!
//! It logs JSON lines on stdout for strata-platform's session tests: `claim`, `launch`,
//! `ready` and `focus` (each with the window's `key`), and `exit-error`.

#[cfg(feature = "single-instance")]
fn main() {
    demo::main();
}

#[cfg(not(feature = "single-instance"))]
fn main() {
    eprintln!("run with --features single-instance");
}

#[cfg(feature = "single-instance")]
mod demo {
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};

    use rune::{App, AppHandle, AppSpec, Cx, El, Tx, WindowSpec, col, text};
    use serde_json::{Value, json};
    use strata_platform::single_instance::{
        AppId, Claim, CommandLine, Event, Launch, Options, Primary, claim,
    };

    const APP_ID: &str = "org.example.SingleInstanceDemo";

    fn log(event: &str, fields: Value) {
        let mut line = json!({ "ev": event });
        if let (Some(obj), Value::Object(fields)) = (line.as_object_mut(), fields) {
            obj.extend(fields);
        }
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    /// The primary, for the last window's close to release.
    static PRIMARY: OnceLock<Primary> = OnceLock::new();
    static CONTROLLER: OnceLock<AppHandle<Controller>> = OnceLock::new();

    /// The open windows' keys and handles, newest last.
    static WINDOWS: Mutex<Vec<(String, AppHandle<Launched>)>> = Mutex::new(Vec::new());

    /// The windowless app every event reaches.
    #[derive(Default)]
    struct Controller {
        launches: u32,
    }

    /// One launch's window.
    #[derive(Default)]
    struct Launched {
        key: String,
        lines: Vec<String>,
    }

    fn controller_view(_: &Controller, _: &mut Cx<Controller>) -> El {
        col()
    }

    fn launched_view(s: &Launched, _: &mut Cx<Launched>) -> El {
        let key = s.key.clone();
        let mut column = col()
            .key("launch")
            .w_full()
            .h_full()
            .center()
            .gap(6.0)
            .on_window_focus(move |_: &mut Launched, focused: bool, _: &mut Tx| {
                log("focus", json!({ "key": key, "focused": focused }));
            });
        for line in &s.lines {
            column = column.child(text(line.clone()));
        }
        column
    }

    fn on_window_close(s: &mut Launched, _: &mut Tx) {
        let mut windows = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        windows.retain(|(key, _)| *key != s.key);
        if windows.is_empty() {
            // Give the app id up first, so a launch now starts a new primary.
            if let Some(primary) = PRIMARY.get() {
                primary.release();
            }
            if let Some(controller) = CONTROLLER.get() {
                controller.quit();
            }
        }
    }

    fn open_window(s: &mut Controller, lines: Vec<String>, token: Option<&str>) {
        s.launches += 1;
        let key = format!("launch-{}", s.launches);
        let state = Launched {
            key: key.clone(),
            lines,
        };
        let app = App::new(AppSpec::new(
            "Single instance demo",
            (480.0, 320.0),
            Launched::default,
            launched_view,
        ))
        .init(move || state)
        .windowless()
        .args(Vec::new())
        .on_close(on_window_close);
        let handle = app.handle();
        // Before the window starts, so it is created with the token.
        match token {
            Some(token) => handle.activate(token),
            None => handle.open(),
        }
        let ready = key.clone();
        rune::open_window(WindowSpec::from_app(
            key.clone(),
            app.on_ready(move |_| log("ready", json!({ "key": ready }))),
        ));
        WINDOWS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((key, handle));
    }

    fn on_launch(s: &mut Controller, cmd: CommandLine) {
        let args: Vec<String> = cmd
            .arguments()
            .iter()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let token = cmd.activation().token().map(str::to_owned);
        log(
            "launch",
            json!({ "remote": cmd.is_remote(), "args": args, "token": token }),
        );
        let newest = WINDOWS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last()
            .map(|(_, h)| h.clone());
        match newest {
            // A bare launch of a running app brings it forward.
            Some(window) if args.is_empty() => match &token {
                Some(token) => window.activate(token.clone()),
                None => window.open(),
            },
            _ => {
                let mut lines = vec![format!("launch {}", s.launches + 1)];
                lines.push(if args.is_empty() {
                    "no arguments".to_owned()
                } else {
                    format!("arguments: {}", args.join(" "))
                });
                if let Some(cwd) = cmd.cwd() {
                    lines.push(format!("from {}", cwd.display()));
                }
                lines.push(format!(
                    "activation token: {}",
                    if token.is_some() { "yes" } else { "none" }
                ));
                open_window(s, lines, token.as_deref());
            }
        }
        if cmd.is_remote() {
            cmd.print(&format!(
                "opened in the running instance ({})\n",
                std::process::id()
            ));
        }
    }

    fn on_event(s: &mut Controller, event: Event, _: &mut Tx) {
        match event {
            Event::CommandLine(cmd) => on_launch(s, cmd),
            Event::Activate(activation) => {
                let newest = WINDOWS
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .last()
                    .map(|(_, h)| h.clone());
                match (newest, activation.token()) {
                    (Some(window), Some(token)) => window.activate(token),
                    (Some(window), None) => window.open(),
                    (None, token) => open_window(s, vec!["activated".into()], token),
                }
            }
            Event::Open {
                uris, activation, ..
            } => {
                open_window(s, uris, activation.token());
            }
            Event::Action { .. } | Event::Reveal(_) => {}
        }
    }

    pub fn main() {
        let id = AppId::new(APP_ID).expect("a valid app id");
        let primary = match claim(&id, Launch::from_env(), &Options::default()) {
            Claim::Primary(p) => {
                log(
                    "claim",
                    json!({ "result": "primary", "pid": std::process::id() }),
                );
                p
            }
            Claim::Standalone(p, reason) => {
                log(
                    "claim",
                    json!({ "result": "standalone", "reason": format!("{reason:?}") }),
                );
                p
            }
            Claim::Forwarded(status) => {
                log("claim", json!({ "result": "forwarded", "status": status }));
                std::process::exit(status);
            }
        };
        rune::set_app_id(APP_ID);
        let app = App::new(AppSpec::new(
            "Single instance demo",
            (100.0, 100.0),
            Controller::default,
            controller_view,
        ))
        .windowless()
        .args(Vec::new());
        let _ = CONTROLLER.set(app.handle());
        primary.attach_rune(app.handle(), on_event);
        let primary = PRIMARY.get_or_init(|| primary);
        let result = app.try_run();
        primary.release();
        if let Err(error) = result {
            log("exit-error", json!({ "error": error }));
            std::process::exit(1);
        }
        std::process::exit(primary.exit_status());
    }
}
