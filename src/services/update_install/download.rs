// SPDX-License-Identifier: MIT

//! The update archive's HTTP agent: bounded by inactivity, not by how long a
//! steady transfer takes, and stoppable while a read is blocked.
//!
//! ureq's phase timeouts are totals that never restart on a read, so a slow
//! link that keeps delivering bytes would still hit them. [`IdleLimited`]
//! wraps the transport and fails a wait for input only after the idle limit
//! passes without a byte. It splits each wait into short slices and checks
//! the install's cancel handle between them, so Cancel takes effect even
//! while the server sends nothing. Over HTTPS the wrapper sits above TLS, so
//! a wait covers a whole TLS record rather than each byte of it.
//!
//! The wrapper implements traits from `ureq::unversioned`, which is outside
//! ureq's semver promise; `Cargo.toml` pins ureq to `~3.4` for that reason.

use std::{
    io,
    time::{Duration, Instant},
};

use ureq::unversioned::{
    resolver::DefaultResolver,
    transport::{
        Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport, time,
    },
};

use super::InstallCancel;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// The longest single blocking wait, and so the longest a cancel can go
/// unnoticed while no bytes arrive.
const CANCEL_POLL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug)]
pub(super) struct DownloadTimeouts {
    /// Resolving the server and opening the connection, TLS included.
    pub connect: Duration,
    /// Waiting for the response headers. In effect also bounded by `idle`,
    /// since each wait for a header byte is.
    pub response: Duration,
    /// The longest wait for the next byte, restarted by every read.
    pub idle: Duration,
}

impl Default for DownloadTimeouts {
    fn default() -> Self {
        Self {
            connect: CONNECT_TIMEOUT,
            response: RESPONSE_TIMEOUT,
            idle: IDLE_TIMEOUT,
        }
    }
}

pub(super) fn download_agent(timeouts: DownloadTimeouts, cancel: &InstallCancel) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_resolve(Some(timeouts.connect))
        .timeout_connect(Some(timeouts.connect))
        .timeout_recv_response(Some(timeouts.response))
        .build();
    let connector = DefaultConnector::default().chain(IdleLimiter {
        idle: timeouts.idle,
        cancel: cancel.clone(),
    });
    ureq::Agent::with_parts(config, connector, DefaultResolver::default())
}

pub(super) fn describe_download_error(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Timeout(
            ureq::Timeout::Resolve | ureq::Timeout::Connect | ureq::Timeout::SendRequest,
        ) => "Could not reach the download server — check your connection and try again"
            .to_owned(),
        ureq::Error::Timeout(_) => {
            "The download stalled — check your connection and try again".to_owned()
        }
        other => format!("Could not download the update: {other}"),
    }
}

/// [`describe_download_error`] for a body read, whose `io::Error` carries the
/// `ureq::Error` as its source.
pub(super) fn describe_read_error(error: &io::Error) -> String {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<ureq::Error>())
    {
        Some(error) => describe_download_error(error),
        None => format!("Could not download the update: {error}"),
    }
}

#[derive(Debug)]
struct IdleLimiter {
    idle: Duration,
    cancel: InstallCancel,
}

impl Connector<Box<dyn Transport>> for IdleLimiter {
    type Out = IdleLimited;

    fn connect(
        &self,
        _details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(chained.map(|inner| IdleLimited {
            inner,
            idle: self.idle,
            cancel: self.cancel.clone(),
        }))
    }
}

#[derive(Debug)]
struct IdleLimited {
    inner: Box<dyn Transport>,
    idle: Duration,
    cancel: InstallCancel,
}

impl Transport for IdleLimited {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.inner.transmit_output(amount, timeout)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let started = Instant::now();
        loop {
            if self.cancel.is_cancelled() {
                // The caller reports its own cancelled outcome; this only
                // unblocks it.
                return Err(io::Error::other("update download cancelled").into());
            }
            let (left, reason) = remaining_wait(timeout, self.idle, started.elapsed());
            if left.is_zero() {
                return Err(ureq::Error::Timeout(reason));
            }
            let slice = left.min(CANCEL_POLL);
            let outcome = self.inner.await_input(NextTimeout {
                after: time::Duration::Exact(slice),
                reason,
            });
            match outcome {
                Err(ureq::Error::Timeout(_)) if slice < left => {}
                outcome => return outcome,
            }
        }
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

/// What is left of one wait after `waited`: the caller's budget or the idle
/// limit, whichever ends first, with the reason to report when it does. An
/// idle limit that cuts short an unbounded or whole-call budget reports
/// `RecvBody`, so a stall is not mistaken for another deadline.
fn remaining_wait(
    timeout: NextTimeout,
    idle: Duration,
    waited: Duration,
) -> (Duration, ureq::Timeout) {
    let budget = timeout.after.saturating_sub(waited);
    let idle_left = idle.saturating_sub(waited);
    if budget <= idle_left {
        return (budget, timeout.reason);
    }
    let reason = match timeout.reason {
        ureq::Timeout::Global | ureq::Timeout::PerCall => ureq::Timeout::RecvBody,
        other => other,
    };
    (idle_left, reason)
}
