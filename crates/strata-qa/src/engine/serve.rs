// SPDX-License-Identifier: MIT

//! The session: reads steps from stdin and answers each with one reply line.

use std::{
    fs::File,
    io::{BufRead, Write},
    panic::AssertUnwindSafe,
};

use futures_channel::mpsc;
use futures_lite::{FutureExt as _, StreamExt as _};
use serde_json::{Value, json};

use super::{dispatch::execute, scan::scan, scope::Remembered};
use crate::{Backend, Outcome, Reply, changed_keys, empty_app_state, parse_request};

/// Names the streams a run replaces: `stdout=closed,stderr=null`.
const STREAMS_VARIABLE: &str = "STRATA_QA_STREAMS";
/// What the process exits with after the session panicked; the runner tells it from a crash.
const PANIC_EXIT: i32 = 70;

/// Takes over stdout for replies before anything can log there; application output goes to
/// stderr, or wherever `STRATA_QA_STREAMS` sends it.
pub fn reply_stream() -> Result<File, String> {
    let protocol = rustix::io::fcntl_dupfd_cloexec(std::io::stdout(), 3)
        .map_err(|error| format!("cannot keep stdout for replies: {error}"))?;
    match std::env::var(STREAMS_VARIABLE) {
        Ok(streams) => replace_streams(&streams)?,
        Err(_) => rustix::stdio::dup2_stdout(std::io::stderr())
            .map_err(|error| format!("cannot move application output to stderr: {error}"))?,
    }
    Ok(File::from(protocol))
}

fn replace_streams(spec: &str) -> Result<(), String> {
    for part in spec.split(',') {
        let (stream, kind) = part
            .split_once('=')
            .ok_or_else(|| format!("{STREAMS_VARIABLE} has no stream=kind in {part:?}"))?;
        let replacement: rustix::fd::OwnedFd = match kind {
            "closed" => {
                let (reader, writer) =
                    rustix::pipe::pipe().map_err(|error| format!("cannot open a pipe: {error}"))?;
                drop(reader);
                writer
            }
            "null" => File::options()
                .write(true)
                .open("/dev/null")
                .map_err(|error| format!("cannot open /dev/null: {error}"))?
                .into(),
            other => return Err(format!("{STREAMS_VARIABLE}: unknown kind {other:?}")),
        };
        match stream {
            "stdout" => rustix::stdio::dup2_stdout(&replacement),
            "stderr" => rustix::stdio::dup2_stderr(&replacement),
            other => return Err(format!("{STREAMS_VARIABLE}: unknown stream {other:?}")),
        }
        .map_err(|error| format!("cannot replace {stream}: {error}"))?;
    }
    Ok(())
}

/// Reads stdin on a thread of its own and hands over each line that is not blank.
pub fn command_lines() -> mpsc::UnboundedReceiver<String> {
    let (sender, receiver) = mpsc::unbounded();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            if sender.unbounded_send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

/// Runs the session until stdin closes, then asks the backend to quit. The first reply says
/// whether the application became ready; every line after that gets one reply.
///
/// A panic would otherwise stay in the task that runs this and leave the runner waiting, so
/// it is reported as a last reply and the process exits.
pub async fn serve<B: Backend>(
    backend: &B,
    protocol: File,
    commands: mpsc::UnboundedReceiver<String>,
) {
    let last_words = protocol.try_clone();
    let session = Session {
        backend,
        protocol,
        previous: None,
        kept: Remembered::default(),
    };
    let Err(panic) = AssertUnwindSafe(session.run(commands)).catch_unwind().await else {
        return;
    };
    let message = panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    if let Ok(mut protocol) = last_words {
        let reply = Reply {
            ok: false,
            app: Value::Null,
            changed: Vec::new(),
            criticals: backend.criticals(),
            result: Value::Null,
            failures: Vec::new(),
            error: Some(format!("the QA controller panicked: {message}")),
        };
        if let Ok(line) = serde_json::to_string(&reply) {
            let _ = writeln!(protocol, "{line}");
            let _ = protocol.flush();
        }
    }
    std::process::exit(PANIC_EXIT);
}

struct Session<'a, B: Backend> {
    backend: &'a B,
    protocol: File,
    /// The `app` of the reply before, which `changed` is told against.
    previous: Option<Value>,
    kept: Remembered<B>,
}

impl<B: Backend> Session<'_, B> {
    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<String>) {
        let ready = match self.backend.ready().await {
            Ok(()) => Outcome {
                result: json!({ "ready": true }),
                ..Outcome::default()
            },
            Err(error) => Outcome::error(error),
        };
        let started = ready.error.is_none();
        self.reply(ready);
        if started {
            while let Some(line) = commands.next().await {
                let outcome = match parse_request(&line) {
                    Ok(command) => execute(self.backend, &self.kept, command).await,
                    Err(error) => Outcome::error(error),
                };
                self.reply(outcome);
            }
        }
        self.backend.quit();
    }

    fn reply(&mut self, outcome: Outcome) {
        let backend = self.backend;
        let app = backend
            .window()
            .map(|window| scan(backend, &window).observation.app_state())
            .and_then(|app| serde_json::to_value(app).ok())
            .unwrap_or_else(|| {
                if backend.windowless() {
                    empty_app_state()
                } else {
                    Value::Null
                }
            });
        let changed = changed_keys(self.previous.as_ref(), &app);
        self.previous = Some(app.clone());
        let reply = Reply {
            ok: outcome.error.is_none() && outcome.failures.is_empty(),
            app,
            changed,
            criticals: backend.criticals(),
            result: outcome.result,
            failures: outcome.failures,
            error: outcome.error,
        };
        let line = serde_json::to_string(&reply).unwrap_or_else(|error| {
            format!(r#"{{"ok":false,"error":"cannot encode the reply: {error}"}}"#)
        });
        if writeln!(self.protocol, "{line}")
            .and_then(|()| self.protocol.flush())
            .is_err()
        {
            backend.quit();
        }
    }
}
