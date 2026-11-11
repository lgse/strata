//! A GLib `GApplication`, for the `single-instance` shim's interop tests (P8): the shim speaks
//! GApplication's wire protocol, so a launch of either reaches whichever is the primary.
//!
//! `GAPP_ID` names the app id. The peer registers it with `HANDLES_COMMAND_LINE` and reports
//! as JSON lines on stdout: `{"ev":"claim","result":"primary"}` or `"forwarded"`; as the primary
//! `{"ev":"launch","args":[…],"cwd":…,"platform":…}` for each command line, answered with exit
//! status 5 when an argument is `--exit5`, else 0; as a forwarder `{"ev":"exit","status":…}`.
//! It needs GIO, so it builds with `glib-bridge`, the feature that links it.

#[cfg(feature = "glib-bridge")]
fn main() {
    peer::main();
}

#[cfg(not(feature = "glib-bridge"))]
fn main() {
    eprintln!("run with --features glib-bridge");
}

#[cfg(feature = "glib-bridge")]
mod peer {
    use std::io::Write;

    use gio::prelude::*;
    use serde_json::{Value, json};

    fn log(event: &str, fields: Value) {
        let mut line = json!({ "ev": event });
        if let (Some(obj), Value::Object(fields)) = (line.as_object_mut(), fields) {
            obj.extend(fields);
        }
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }

    pub fn main() {
        let id = std::env::var("GAPP_ID").unwrap_or_else(|_| "org.example.GApplicationPeer".into());
        let app = gio::Application::new(Some(&id), gio::ApplicationFlags::HANDLES_COMMAND_LINE);
        app.connect_command_line(|_, cmd| {
            let args: Vec<String> = cmd
                .arguments()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            log(
                "launch",
                json!({
                    "remote": cmd.is_remote(),
                    "args": args,
                    "cwd": cmd.cwd().map(|c| c.display().to_string()),
                    "platform": cmd.platform_data().map(|p| p.print(true).to_string()),
                }),
            );
            if args.iter().any(|a| a == "--exit5") {
                glib::ExitCode::from(5)
            } else {
                glib::ExitCode::SUCCESS
            }
        });
        if let Err(e) = app.register(None::<&gio::Cancellable>) {
            log(
                "claim",
                json!({ "result": "error", "error": e.to_string() }),
            );
            std::process::exit(1);
        }
        let remote = app.is_remote();
        log(
            "claim",
            json!({ "result": if remote { "forwarded" } else { "primary" } }),
        );
        // The primary stays until the test ends it.
        let _hold = (!remote).then(|| app.hold());
        let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
        let status = app.run_with_args_os(&args);
        log("exit", json!({ "status": status.get() }));
        std::process::exit(i32::from(status.get()));
    }
}
