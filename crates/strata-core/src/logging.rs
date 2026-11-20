// SPDX-License-Identifier: MIT

use std::{
    env,
    io::{self, Write},
    str::FromStr,
};

use tracing_subscriber::{
    filter::{LevelFilter, Targets},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

// Closed launcher pipes must not turn logging failures into panics at a GLib boundary.
#[derive(Clone, Copy, Debug, Default)]
pub struct PipeSafeWriter<W>(pub W);

impl<W: Write> Write for PipeSafeWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.write(buf) {
            Ok(n) => Ok(n),
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(buf.len()),
            Err(error) => Err(error),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0.flush() {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// The crates split out of the application. Their modules logged under `strata::…` before.
const EXTRACTED_CRATES: [&str; 4] = [
    "strata_core",
    "strata_desktop",
    "strata_fs",
    "strata_preview",
];

/// Repeats each `strata::module` directive for the crates that module may live in now, so a
/// filter written against the application's module path keeps selecting the same code.
fn with_extracted_crates(directives: &str) -> String {
    let mut expanded = Vec::new();
    for directive in directives.split(',') {
        expanded.push(directive.to_owned());
        if let Some(module) = directive.trim_start().strip_prefix("strata::") {
            expanded.extend(
                EXTRACTED_CRATES
                    .iter()
                    .map(|name| format!("{name}::{module}")),
            );
        }
    }
    expanded.join(",")
}

pub fn initialize() {
    let targets = match env::var("RUST_LOG") {
        Ok(var) => Targets::from_str(&with_extracted_crates(&var)).unwrap_or_default(),
        Err(_) => Targets::default().with_default(LevelFilter::INFO),
    };
    let subscriber = tracing_subscriber::fmt()
        .with_writer(|| PipeSafeWriter(io::stdout()))
        .with_max_level(LevelFilter::TRACE)
        .log_internal_errors(false)
        .finish()
        .with(targets);
    let _ = subscriber.try_init();
}

#[cfg(test)]
mod tests;
