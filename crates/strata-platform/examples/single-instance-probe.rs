//! The `single-instance` shim without Rune, for strata-platform's session tests (P8). It
//! claims `org.example.SingleInstanceProbe` and reports as JSON lines on stdout:
//!
//! - `{"ev":"claim","result":"primary"}` (with `"file_manager1"`: `owned`, `queued` or
//!   `failed: …`), `"forwarded"` with `"status"`, or `"standalone"` with `"reason"`;
//! - as the primary, one line per event: `launch` (with `remote`, `args` and `cwd` as hex
//!   bytes, `xdg` and `sid` tokens), `activate`, `open`, `action` and `reveal`.
//!
//! As the primary it answers each launch by its arguments: `--print` prints a line to the
//! launch's stdout, `--usage-error` a line to its stderr and exit status 1, and
//! `--release-after` answers the launch, then stops being the primary at once: from its main
//! thread, or with `PROBE_RELEASE_ON=bus` from the bus thread. `--hold` never answers. Its
//! stdin takes
//! `release` (stop being the primary) and `quit`; the action `quit` ends it too. With
//! `PROBE_SERVICE=1` the end of stdin doesn't: the bus starts it with none.
//!
//! The environment sets the rest: `PROBE_GATE=1` waits for a line on stdin before claiming,
//! `PROBE_SERVICE=1` and `PROBE_FILE_MANAGER1=1` set those options, `PROBE_FILE_MANAGER1_NAME`
//! the name asked for in place of `org.freedesktop.FileManager1`, and `PROBE_BUS_TIMEOUT_MS`
//! the bus timeout. It accepts the actions `go` and `quit`.

#[cfg(feature = "single-instance")]
fn main() {
    probe::main();
}

#[cfg(not(feature = "single-instance"))]
fn main() {
    eprintln!("run with --features single-instance");
}

#[cfg(feature = "single-instance")]
mod probe {
    use std::ffi::OsStr;
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::sync::mpsc::{SyncSender, sync_channel};
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use serde_json::{Value, json};
    use strata_platform::single_instance::{
        AppId, Claim, CommandLine, Event, Launch, NameRequest, Options, Primary, RevealMethod,
        Unclaimed, claim,
    };

    const APP_ID: &str = "org.example.SingleInstanceProbe";

    fn log(event: &str, fields: Value) {
        let mut line = json!({ "ev": event });
        if let (Some(obj), Value::Object(fields)) = (line.as_object_mut(), fields) {
            obj.extend(fields);
        }
        // A closed stdout (a test checks one) must not stop the probe.
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    fn hex(bytes: &OsStr) -> String {
        bytes
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn flag(name: &str) -> bool {
        std::env::var_os(name).is_some_and(|v| v == "1")
    }

    /// The primary, and the way to ask the main thread to release it.
    static PRIMARY: OnceLock<Primary> = OnceLock::new();
    static RELEASE: OnceLock<SyncSender<()>> = OnceLock::new();
    static HELD: Mutex<Vec<CommandLine>> = Mutex::new(Vec::new());

    fn on_event(event: Event) {
        match event {
            Event::CommandLine(cmd) => {
                let args: Vec<String> = cmd.arguments().iter().map(|a| hex(a)).collect();
                log(
                    "launch",
                    json!({
                        "remote": cmd.is_remote(),
                        "args": args,
                        "cwd": cmd.cwd().map(|c| hex(c.as_os_str())),
                        "xdg": cmd.activation().xdg_activation_token,
                        "sid": cmd.activation().desktop_startup_id,
                    }),
                );
                let has = |a: &str| cmd.arguments().iter().any(|x| x == a);
                if has("--print") {
                    cmd.print("hello from the primary\n");
                }
                if has("--hold") {
                    // Never answered: kept until the probe exits.
                    HELD.lock().unwrap_or_else(|e| e.into_inner()).push(cmd);
                    return;
                }
                if has("--release-after") {
                    drop(cmd);
                    if std::env::var_os("PROBE_RELEASE_ON").is_some_and(|v| v == "bus") {
                        if let Some(primary) = PRIMARY.get() {
                            primary.release();
                        }
                        log("released", json!({ "on": "bus" }));
                    } else if let Some(release) = RELEASE.get() {
                        let _ = release.send(());
                    }
                    return;
                }
                if has("--usage-error") {
                    cmd.print_err("usage: single-instance-probe [--print] [FILE...]\n");
                    cmd.exit(1);
                }
            }
            Event::Activate(activation) => log(
                "activate",
                json!({ "xdg": activation.xdg_activation_token, "sid": activation.desktop_startup_id }),
            ),
            Event::Open {
                uris,
                hint,
                activation,
            } => log(
                "open",
                json!({ "uris": uris, "hint": hint, "xdg": activation.xdg_activation_token }),
            ),
            Event::Action {
                name,
                parameter,
                activation,
            } => {
                log(
                    "action",
                    json!({
                        "name": name,
                        "parameter": parameter.map(|p| format!("{p:?}")),
                        "xdg": activation.xdg_activation_token,
                    }),
                );
                // A probe the bus started has no stdin to quit by.
                if name == "quit" {
                    std::process::exit(0);
                }
            }
            Event::Reveal(reveal) => log(
                "reveal",
                json!({
                    "method": match reveal.method {
                        RevealMethod::Folders => "folders",
                        RevealMethod::Items => "items",
                        RevealMethod::ItemProperties => "item-properties",
                    },
                    "uris": reveal.uris,
                    "startup_id": reveal.startup_id,
                }),
            ),
        }
    }

    pub fn main() {
        if flag("PROBE_GATE") {
            let _ = std::io::stdin().read_line(&mut String::new());
        }
        let id = AppId::new(APP_ID).expect("a valid app id");
        let mut options = Options {
            service: flag("PROBE_SERVICE"),
            file_manager1: flag("PROBE_FILE_MANAGER1"),
            actions: vec!["go".into(), "quit".into()],
            file_manager1_name: std::env::var("PROBE_FILE_MANAGER1_NAME").ok(),
            ..Options::default()
        };
        if let Some(ms) = std::env::var("PROBE_BUS_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            options.bus_timeout = Duration::from_millis(ms);
        }
        let primary = match claim(&id, Launch::from_env(), &options) {
            Claim::Primary(p) => {
                let file_manager1 = p.file_manager1().map(|r| match r {
                    NameRequest::Owned => "owned".to_owned(),
                    NameRequest::Queued => "queued".to_owned(),
                    NameRequest::Failed(e) => format!("failed: {e}"),
                });
                log(
                    "claim",
                    json!({
                        "result": "primary",
                        "pid": std::process::id(),
                        "file_manager1": file_manager1,
                    }),
                );
                p
            }
            Claim::Standalone(p, reason) => {
                let reason = match reason {
                    Unclaimed::NoBus(e) => format!("no-bus: {e}"),
                    Unclaimed::Timeout => "timeout".into(),
                    Unclaimed::Error(e) => format!("error: {e}"),
                };
                log("claim", json!({ "result": "standalone", "reason": reason }));
                p
            }
            Claim::Forwarded(status) => {
                log("claim", json!({ "result": "forwarded", "status": status }));
                std::process::exit(status);
            }
        };
        let primary = PRIMARY.get_or_init(|| primary);
        let (release, released) = sync_channel(1);
        let _ = RELEASE.set(release);
        primary.attach(on_event);
        // Lines from stdin, and release requests from a sink, on the main thread.
        let (input, commands) = sync_channel(16);
        let lines_input = input.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lines() {
                let line = line
                    .map(|l| l.trim().to_owned())
                    .unwrap_or_else(|_| "quit".into());
                if lines_input.send(line).is_err() {
                    return;
                }
            }
            let _ = lines_input.send("quit".into());
        });
        std::thread::spawn(move || {
            while released.recv().is_ok() {
                if input.send("release-after".into()).is_err() {
                    return;
                }
            }
        });
        for command in commands {
            match command.as_str() {
                "release" => {
                    primary.release();
                    log("released", json!({ "claimed": primary.is_claimed() }));
                }
                "release-after" => {
                    primary.release();
                    log("released", json!({ "on": "main" }));
                }
                "quit" => break,
                _ => {}
            }
        }
        if options.service {
            loop {
                std::thread::park();
            }
        }
        log("exit", json!({}));
    }
}
