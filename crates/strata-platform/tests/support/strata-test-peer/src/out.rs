//! Everything the peer writes: one JSON line per reply or event, from any thread.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use strata_test_peer::protocol::{Event, INLINE_TEXT, Message, Received, Reply, fnv1a64};

fn write(msg: &Message) {
    let Ok(line) = serde_json::to_string(msg) else {
        return;
    };
    let mut out = std::io::stdout().lock();
    // A closed stdout means the test is gone; the peer exits once stdin closes too.
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

pub fn reply(id: u64, result: Result<Value, String>) {
    if id == 0 {
        return;
    }
    let reply = match result {
        Ok(value) => Reply {
            id,
            ok: true,
            value,
            error: None,
        },
        Err(e) => Reply {
            id,
            ok: false,
            value: Value::Null,
            error: Some(e),
        },
    };
    write(&Message::Reply(reply));
}

pub fn ok(id: u64) {
    reply(id, Ok(Value::Null));
}

pub fn value(id: u64, v: impl serde::Serialize) {
    reply(id, serde_json::to_value(v).map_err(|e| e.to_string()));
}

pub fn event(e: Event) {
    write(&Message::Event(e));
}

pub fn error(message: String) {
    eprintln!("strata-test-peer: {message}");
    event(Event::Error { message });
}

/// Where received data larger than [`INLINE_TEXT`], or not UTF-8, is written.
fn dir() -> PathBuf {
    std::env::var_os("STRATA_PEER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// A path in the peer's directory for a file of its own, unique within the process.
pub fn scratch_path(what: &str) -> PathBuf {
    dir().join(format!("peer-{}-{what}", std::process::id()))
}

/// Describes received bytes, inline when they are short UTF-8 text, else in a file.
pub fn received(data: &[u8]) -> Received {
    static N: AtomicU64 = AtomicU64::new(0);
    let text = (data.len() <= INLINE_TEXT)
        .then(|| std::str::from_utf8(data).ok().map(str::to_owned))
        .flatten();
    let file = match text {
        Some(_) => None,
        None => {
            let n = N.fetch_add(1, Ordering::Relaxed);
            let path = scratch_path(&format!("received-{n}.bin"));
            match std::fs::write(&path, data) {
                Ok(()) => Some(path),
                Err(e) => {
                    error(format!("writing {}: {e}", path.display()));
                    None
                }
            }
        }
    };
    Received {
        len: data.len() as u64,
        fnv1a64: fnv1a64(data),
        text,
        file,
    }
}

/// Writes a source's bytes for one reader on a thread of its own, so the peer's loop keeps
/// running: a peer reading its own selection serves and reads at once.
pub fn serve(mime: String, data: Result<Vec<u8>, String>, fd: impl Into<std::fs::File>) {
    let mut file = fd.into();
    event(Event::SourceSend { mime: mime.clone() });
    std::thread::spawn(move || match data {
        Ok(bytes) => {
            if let Err(e) = file.write_all(&bytes) {
                error(format!("serving {mime}: {e}"));
            }
        }
        Err(e) => error(format!("serving {mime}: {e}")),
    });
}
