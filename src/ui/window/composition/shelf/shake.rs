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

    pub(super) fn listen(self) -> Receiver<(i32, i32)> {
        let (send, receive) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let started = Instant::now();
            let mut shake = Shake::default();
            loop {
                if let Some((x, y)) = self.cursor()
                    && shake.sample(x, y, started.elapsed())
                    && matches!(
                        send.try_send((x, y)),
                        Err(mpsc::TrySendError::Disconnected(_))
                    )
                {
                    break;
                }
                thread::sleep(POLL);
            }
        });
        receive
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
mod tests {
    use super::Shake;
    use std::time::Duration;

    #[test]
    fn horizontal_shake_opens_once_but_normal_motion_does_not() {
        let mut shake = Shake::default();
        for (index, x) in [0, 60, 15, 80].into_iter().enumerate() {
            assert!(!shake.sample(x, 100, Duration::from_millis(index as u64 * 80)));
        }
        assert!(shake.sample(20, 100, Duration::from_millis(320)));
        assert!(!shake.sample(80, 100, Duration::from_millis(400)));
        assert!(!shake.sample(30, 100, Duration::from_millis(480)));
    }

    #[test]
    fn vertical_and_slow_changes_do_not_trigger() {
        let mut shake = Shake::default();
        for (index, x) in [0, 50, 10, 60, 0, 80].into_iter().enumerate() {
            assert!(!shake.sample(
                x,
                index as i32 * 100,
                Duration::from_millis(index as u64 * 220)
            ));
        }
    }
}
