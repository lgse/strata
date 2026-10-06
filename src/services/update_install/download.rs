// SPDX-License-Identifier: MIT

//! ureq's phase budgets do not restart on reads. Bound each input wait instead,
//! and enforce its deadline below TLS so partial records cannot postpone cancellation.

use std::{
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use ureq::unversioned::{
    resolver::DefaultResolver,
    transport::{
        Buffers, ConnectProxyConnector, ConnectionDetails, Connector, NextTimeout, RustlsConnector,
        TcpConnector, Transport, time,
    },
};

use super::InstallCancel;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const CANCEL_POLL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug)]
pub(super) struct DownloadTimeouts {
    pub connect: Duration,
    pub response: Duration,
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
        .timeout_send_request(Some(timeouts.response))
        .timeout_recv_response(Some(timeouts.response))
        .build();
    agent_with_config(config, timeouts.idle, cancel)
}

fn agent_with_config(
    config: ureq::config::Config,
    idle: Duration,
    cancel: &InstallCancel,
) -> ureq::Agent {
    let connector =
        ().chain(ConnectProxyConnector::default())
            .chain(TcpConnector::default())
            .chain(IdleLimiter {
                idle,
                cancel: cancel.clone(),
                tls: RustlsConnector::default(),
            });
    ureq::Agent::with_parts(config, connector, DefaultResolver::default())
}

pub(super) fn describe_download_error(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Timeout(
            ureq::Timeout::Resolve | ureq::Timeout::Connect | ureq::Timeout::SendRequest,
        ) => "Could not reach the download server — check your connection and try again".to_owned(),
        ureq::Error::Timeout(_) => {
            "The download stalled — check your connection and try again".to_owned()
        }
        other => format!("Could not download the update: {other}"),
    }
}

pub(super) fn describe_read_error(error: &io::Error) -> String {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<ureq::Error>())
    {
        Some(error) => describe_download_error(error),
        None => format!("Could not download the update: {error}"),
    }
}

#[derive(Clone, Copy, Debug)]
struct WaitBudget {
    deadline: Instant,
    reason: ureq::Timeout,
}

impl WaitBudget {
    fn new(started: Instant, timeout: NextTimeout, idle: Duration) -> Self {
        let (duration, reason) = if *timeout.after <= idle {
            (*timeout.after, timeout.reason)
        } else {
            let reason = match timeout.reason {
                ureq::Timeout::Global | ureq::Timeout::PerCall => ureq::Timeout::RecvBody,
                reason => reason,
            };
            (idle, reason)
        };
        Self {
            deadline: started + duration,
            reason,
        }
    }

    fn remaining(self) -> Result<Duration, ureq::Error> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(ureq::Error::Timeout(self.reason))
        } else {
            Ok(remaining)
        }
    }
}

#[derive(Debug)]
struct IdleLimiter {
    idle: Duration,
    cancel: InstallCancel,
    tls: RustlsConnector,
}

impl<In: Transport> Connector<In> for IdleLimiter {
    type Out = Box<dyn Transport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let Some(inner) = chained else {
            return Ok(None);
        };
        let started = match details.now {
            time::Instant::Exact(started) => started,
            _ => Instant::now(),
        };
        let budget = Arc::new(Mutex::new(WaitBudget::new(
            started,
            details.timeout,
            self.idle,
        )));
        let raw = IdleLimited {
            inner: inner.boxed(),
            cancel: self.cancel.clone(),
            budget: budget.clone(),
        };
        let transport = self.tls.connect(details, Some(raw))?;
        Ok(transport.map(|inner| {
            Box::new(InputBudget {
                inner: inner.boxed(),
                idle: self.idle,
                budget,
            }) as Box<dyn Transport>
        }))
    }
}

// One plaintext wait may require many encrypted reads; they share one deadline.
#[derive(Debug)]
struct InputBudget {
    inner: Box<dyn Transport>,
    idle: Duration,
    budget: Arc<Mutex<WaitBudget>>,
}

impl Transport for InputBudget {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        *self.budget.lock().expect("download budget") =
            WaitBudget::new(Instant::now(), timeout, self.idle);
        self.inner.transmit_output(amount, timeout)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        *self.budget.lock().expect("download budget") =
            WaitBudget::new(Instant::now(), timeout, self.idle);
        self.inner.await_input(timeout)
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }
    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

#[derive(Debug)]
struct IdleLimited {
    inner: Box<dyn Transport>,
    cancel: InstallCancel,
    budget: Arc<Mutex<WaitBudget>>,
}

impl IdleLimited {
    fn check_cancelled(&self) -> Result<(), ureq::Error> {
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("update download cancelled").into());
        }
        Ok(())
    }
}

impl Transport for IdleLimited {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, _timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.check_cancelled()?;
        let budget = *self.budget.lock().expect("download budget");
        // A timed-out write may be partial and must not be replayed.
        self.inner.transmit_output(
            amount,
            NextTimeout {
                after: time::Duration::Exact(budget.remaining()?),
                reason: budget.reason,
            },
        )
    }

    fn await_input(&mut self, _timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let budget = *self.budget.lock().expect("download budget");
        loop {
            self.check_cancelled()?;
            let left = budget.remaining()?;
            let slice = left.min(CANCEL_POLL);
            let outcome = self.inner.await_input(NextTimeout {
                after: time::Duration::Exact(slice),
                reason: budget.reason,
            });
            self.check_cancelled()?;
            budget.remaining()?;
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

#[cfg(test)]
mod tests;
