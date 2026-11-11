// SPDX-License-Identifier: MIT

//! What GLib logged as a warning, a critical or an error, kept for the QA mode: each reply
//! reports them, and a run fails on any. Both interfaces use it, since GIO stays under both.

use std::sync::Mutex;

static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Routes GLib's log through a writer that keeps those lines, and still prints them.
pub fn capture() {
    glib::log_set_writer_func(|level, fields| {
        if matches!(
            level,
            glib::LogLevel::Error | glib::LogLevel::Critical | glib::LogLevel::Warning
        ) {
            let field = |key: &str| {
                fields
                    .iter()
                    .find(|field| field.key() == key)
                    .and_then(glib::LogField::value_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let line = format!(
                "{}-{} **: {}",
                field("GLIB_DOMAIN"),
                level_name(level),
                field("MESSAGE")
            );
            if let Ok(mut pending) = PENDING.lock() {
                pending.push(line);
            }
        }
        glib::log_writer_default(level, fields)
    });
}

fn level_name(level: glib::LogLevel) -> &'static str {
    match level {
        glib::LogLevel::Error => "ERROR",
        glib::LogLevel::Critical => "CRITICAL",
        _ => "WARNING",
    }
}

/// The lines kept since the last call.
pub fn take() -> Vec<String> {
    PENDING
        .lock()
        .map(|mut pending| std::mem::take(&mut *pending))
        .unwrap_or_default()
}
