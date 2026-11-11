//! The peer process. See the library docs and [`strata_test_peer::protocol`].

mod out;
mod wayland;
mod x11;

use std::io::BufRead;

use strata_test_peer::protocol::{Command, Request};

fn main() {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    let result = if wayland {
        wayland::run()
    } else if std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) {
        x11::run()
    } else {
        Err("neither WAYLAND_DISPLAY nor DISPLAY is set".to_owned())
    };
    if let Err(e) = result {
        eprintln!("strata-test-peer: {e}");
        std::process::exit(1);
    }
}

/// Reads requests from stdin on a thread of its own. Each one goes to `send`, then `wake` runs
/// so the backend's loop notices. End of input sends a [`Command::Quit`].
fn read_stdin(send: impl Fn(Request) + Send + 'static, wake: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Request>(&line) {
                Ok(req) => send(req),
                Err(e) => out::error(format!("bad request {line:?}: {e}")),
            }
            wake();
        }
        send(Request {
            id: 0,
            command: Command::Quit,
        });
        wake();
    });
}
