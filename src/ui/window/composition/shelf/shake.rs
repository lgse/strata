// SPDX-License-Identifier: MIT

use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

const POLL: Duration = Duration::from_millis(55);

pub(super) struct ShakeListener {
    positions: Receiver<(i32, i32)>,
    _stop: mpsc::Sender<()>,
}

impl ShakeListener {
    pub(super) fn latest(&self) -> Option<(i32, i32)> {
        self.positions.try_iter().last()
    }
}

#[derive(Clone)]
pub(super) struct Hyprland {
    socket: PathBuf,
}

impl Hyprland {
    pub(super) fn current() -> Option<Self> {
        let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
        // Never let an environment value redirect IPC outside our session runtime.
        if !signature
            .as_encoded_bytes()
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            return None;
        }
        let socket = PathBuf::from(runtime)
            .join("hypr")
            .join(signature)
            .join(".socket.sock");
        socket.exists().then_some(Self { socket })
    }

    pub(super) fn cursor(&self) -> Option<(i32, i32)> {
        let data = self.request("j/cursorpos")?;
        let position: serde_json::Value = serde_json::from_slice(&data).ok()?;
        Some((
            position["x"].as_i64()?.try_into().ok()?,
            position["y"].as_i64()?.try_into().ok()?,
        ))
    }

    pub(super) fn move_shelf(&self, x: i32, y: i32) {
        let _ = self.request(&format!(
            "/dispatch hl.dsp.window.move({{ x = {x}, y = {y}, window = 'title:^Strata Shelf$' }})"
        ));
        let _ = self
            .request("/dispatch hl.dsp.window.bring_to_top({ window = 'title:^Strata Shelf$' })");
    }

    fn request(&self, command: &str) -> Option<Vec<u8>> {
        let mut socket = UnixStream::connect(&self.socket).ok()?;
        socket
            .set_read_timeout(Some(Duration::from_millis(90)))
            .ok()?;
        socket.write_all(command.as_bytes()).ok()?;
        socket.shutdown(std::net::Shutdown::Write).ok()?;
        let mut response = Vec::new();
        socket.take(64 * 1024).read_to_end(&mut response).ok()?;
        Some(response)
    }

    pub(super) fn listen(self) -> ShakeListener {
        let (send, receive) = mpsc::sync_channel(1);
        let (stop, stopped) = mpsc::channel::<()>();
        thread::spawn(move || {
            let started = Instant::now();
            let mut shake = Shake::default();
            loop {
                if !crate::ui::browser::file_drag_active() {
                    shake.reset();
                } else if let Some((x, y)) = self.cursor()
                    && shake.sample_drag(
                        x,
                        y,
                        started.elapsed(),
                        crate::ui::browser::file_drag_active(),
                    )
                    && matches!(
                        send.try_send((x, y)),
                        Err(mpsc::TrySendError::Disconnected(_))
                    )
                {
                    break;
                }
                if !matches!(
                    stopped.recv_timeout(POLL),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    break;
                }
            }
        });
        ShakeListener {
            positions: receive,
            _stop: stop,
        }
    }
}

#[derive(Default)]
struct Shake {
    previous: Option<(i32, i32, Duration)>,
    direction: i32,
    reversals: u8,
    started: Duration,
    travelled: i32,
    cooldown: Duration,
}

impl Shake {
    fn sample_drag(&mut self, x: i32, y: i32, now: Duration, drag_active: bool) -> bool {
        if !drag_active {
            self.reset();
            return false;
        }
        self.sample(x, y, now)
    }

    fn reset(&mut self) {
        *self = Self::default();
    }

    fn sample(&mut self, x: i32, y: i32, now: Duration) -> bool {
        let previous = self.previous.replace((x, y, now));
        let Some((px, py, then)) = previous else {
            return false;
        };
        if now < self.cooldown || now.saturating_sub(then) > Duration::from_millis(180) {
            self.direction = 0;
            self.reversals = 0;
            self.travelled = 0;
            return false;
        }
        let dx = x - px;
        let dy = y - py;
        if dx.abs() < 18 || dx.abs() < dy.abs() * 2 {
            return false;
        }
        if now.saturating_sub(self.started) > Duration::from_millis(800) {
            self.reversals = 0;
            self.travelled = 0;
            self.started = now;
        }
        let direction = dx.signum();
        if self.direction != 0 && self.direction != direction {
            self.reversals += 1;
        }
        self.direction = direction;
        self.travelled += dx.abs();
        if self.reversals >= 3 && self.travelled >= 170 {
            self.cooldown = now + Duration::from_secs(2);
            self.reversals = 0;
            self.travelled = 0;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests;
