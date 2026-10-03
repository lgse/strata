// SPDX-License-Identifier: MIT

use std::{
    env,
    io::{self, Write},
    str::FromStr,
};

use tracing_subscriber::{
    filter::{LevelFilter, Targets},
    layer::SubscriberExt,
};

/// An `io::Write` wrapper that swallows `ErrorKind::BrokenPipe` errors so logging
/// continues silently when stdout or stderr is connected to a closed pipe.
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

/// Initializes global tracing subscriber with broken-pipe safety.
pub fn initialize() {
    let targets = match env::var("RUST_LOG") {
        Ok(var) => Targets::from_str(&var).unwrap_or_default(),
        Err(_) => Targets::default().with_default(LevelFilter::INFO),
    };
    let subscriber = tracing_subscriber::fmt()
        .with_writer(|| PipeSafeWriter(io::stdout()))
        .with_max_level(LevelFilter::TRACE)
        .log_internal_errors(false)
        .finish()
        .with(targets);
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
mod tests;
