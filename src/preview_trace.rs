// SPDX-License-Identifier: MIT

use std::{
    collections::hash_map::RandomState,
    hash::{BuildHasher, Hash},
    io::{self, BufRead, BufReader, Read, Write},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use gtk::{glib, prelude::*};

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("STRATA_PREVIEW_TRACE").as_deref() == Ok("1"))
}

pub(crate) fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    if enabled() {
        NEXT.fetch_add(1, Ordering::Relaxed)
    } else {
        0
    }
}

// Randomized per process so repeated files correlate without recording their paths.
pub(crate) fn file_id(value: &impl Hash) -> u64 {
    static HASHER: OnceLock<RandomState> = OnceLock::new();
    HASHER.get_or_init(RandomState::new).hash_one(value)
}

pub(crate) fn write(event: &str, fields: serde_json::Value) {
    let record = serde_json::json!({
        "unix_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
        "pid": std::process::id(),
        "sandbox_job": std::env::var("STRATA_PREVIEW_TRACE_JOB").ok(),
        "event": event,
        "fields": fields,
    });
    // Diagnostic output must not turn a closed log pipe into an application failure.
    let line = format!("STRATA_PREVIEW_TRACE {record}\n");
    let _ = std::io::stderr().lock().write_all(line.as_bytes());
}

macro_rules! event {
    ($event:expr $(, $key:literal => $value:expr)* $(,)?) => {
        if $crate::preview_trace::enabled() {
            $crate::preview_trace::write($event, serde_json::json!({$($key: $value),*}));
        }
    };
}
pub(crate) use event;

pub(crate) fn relay_helper_trace(input: impl Read, mut output: impl Write) -> io::Result<()> {
    // Keep the untrusted helper on a bounded pipe, never a writable host log file.
    let mut input = BufReader::new(input.take(64 * 1024));
    let mut line = Vec::new();
    while input.read_until(b'\n', &mut line)? != 0 {
        if let Some(json) = line.strip_prefix(b"STRATA_PREVIEW_TRACE ")
            && serde_json::from_slice::<serde_json::Value>(json).is_ok()
        {
            if !line.ends_with(b"\n") {
                line.push(b'\n');
            }
            output.write_all(&line)?;
        }
        line.clear();
    }
    Ok(())
}

pub(crate) fn watch_object(object: &impl IsA<glib::Object>, kind: &'static str, id: u64) {
    if !enabled() {
        return;
    }
    event!("object_created", "kind" => kind, "object" => id);
    object.add_weak_ref_notify_local(move || {
        event!("object_finalized", "kind" => kind, "object" => id);
    });
}

pub(crate) fn watch_media(media: &gtk::MediaFile, id: u64) {
    if !enabled() {
        return;
    }
    watch_object(media, "media", id);
    media_state(media, id, "media_created");
    media.connect_prepared_notify(move |media| media_state(media, id, "media_prepared"));
    media.connect_playing_notify(move |media| media_state(media, id, "media_playing"));
    media.connect_error_notify(move |media| media_state(media, id, "media_error"));
}

fn media_state(media: &gtk::MediaFile, id: u64, event: &str) {
    event!(event,
        "media" => id, "backend" => media.type_().name(),
        "prepared" => media.is_prepared(), "playing" => media.is_playing(),
        "has_error" => media.error().is_some(),
        "width" => media.intrinsic_width(), "height" => media.intrinsic_height(),
        "duration_us" => media.duration(), "has_input" => media.input_stream().is_some(),
    );
}

#[cfg(test)]
mod tests;
