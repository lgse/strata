// SPDX-License-Identifier: MIT

use crate::{
    media::{
        Cancellation, FRAME_TIMEOUT, TimedReader, TimedWriter,
        ipc::{self, Kind, Message, PcmSequence, u64_at},
    },
    sandbox::media::WorkerSlot,
};
use std::{
    cell::RefCell,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const QUEUED: usize = 3;

struct State {
    cancellation: Cancellation,
    ready: AtomicBool,
    playing: AtomicBool,
    muted: AtomicBool,
    volume: AtomicU64,
    position: AtomicU64,
    queued: AtomicUsize,
    finish: AtomicBool,
    error: Mutex<Option<String>>,
}

pub(super) struct PcmOutput {
    sender: mpsc::SyncSender<Message>,
    sequence: RefCell<PcmSequence>,
    state: Arc<State>,
}

impl PcmOutput {
    pub(super) fn new(muted: bool, volume: f64, lease: Arc<WorkerSlot>) -> Result<Self, String> {
        let state = Arc::new(State {
            cancellation: Cancellation::default(),
            ready: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            muted: AtomicBool::new(muted),
            volume: AtomicU64::new(normalized_volume(volume).to_bits()),
            position: AtomicU64::new(u64::MAX),
            queued: AtomicUsize::new(0),
            finish: AtomicBool::new(false),
            error: Mutex::new(None),
        });
        let (sender, receiver) = mpsc::sync_channel(QUEUED);
        let worker = state.clone();
        thread::Builder::new()
            .name("media-pcm".into())
            .spawn(move || {
                // A session includes parser and audio descendants, not one slot per process.
                let _lease = lease;
                if let Err(error) = run(&worker, receiver) {
                    worker.ready.store(false, Ordering::Release);
                    if let Ok(mut failure) = worker.error.lock() {
                        *failure = Some(error);
                    }
                }
            })
            .map_err(|_| "Cannot start the audio coordinator")?;
        Ok(Self {
            sender,
            sequence: RefCell::new(PcmSequence::default()),
            state,
        })
    }

    pub(super) fn has_capacity(&self) -> bool {
        self.state.ready.load(Ordering::Acquire)
            && !self.state.finish.load(Ordering::Acquire)
            && self.state.queued.load(Ordering::Acquire) < QUEUED
    }
    pub(super) fn push(&self, data: Vec<u8>, time_us: u64) -> Result<(), String> {
        if !self.has_capacity() {
            return Err("PCM output is full or finished".into());
        }
        self.sequence
            .borrow_mut()
            .push(data.len(), time_us)
            .map_err(|e| e.to_string())?;
        self.state.queued.fetch_add(1, Ordering::AcqRel);
        if self
            .sender
            .try_send(Message {
                kind: Kind::Samples,
                time_us,
                data,
            })
            .is_err()
        {
            self.state.queued.fetch_sub(1, Ordering::AcqRel);
            return Err("Audio worker stopped or its bounded queue is full".into());
        }
        Ok(())
    }
    pub(super) fn play(&self) -> Result<(), String> {
        self.state.playing.store(true, Ordering::Release);
        self.check()
    }
    pub(super) fn pause(&self) -> Result<(), String> {
        self.state.playing.store(false, Ordering::Release);
        self.check()
    }
    pub(super) fn position_us(&self) -> Option<u64> {
        let p = self.state.position.load(Ordering::Acquire);
        (p != u64::MAX).then_some(p)
    }
    pub(super) fn set_audio(&self, muted: bool, volume: f64) {
        self.state.muted.store(muted, Ordering::Release);
        self.state
            .volume
            .store(normalized_volume(volume).to_bits(), Ordering::Release);
    }
    pub(super) fn finish(&self) -> Result<(), String> {
        self.sequence
            .borrow_mut()
            .finish()
            .map_err(|e| e.to_string())?;
        self.state.finish.store(true, Ordering::Release);
        self.check()
    }
    fn check(&self) -> Result<(), String> {
        self.error().map_or(Ok(()), Err)
    }
    pub(super) fn error(&self) -> Option<String> {
        self.state
            .error
            .lock()
            .map_or_else(|_| Some("Audio coordinator failed".into()), |e| e.clone())
    }
}

impl Drop for PcmOutput {
    fn drop(&mut self) {
        self.state.cancellation.cancel();
    }
}

fn normalized_volume(volume: f64) -> f64 {
    if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn command(executable: &Path, job: u64) -> Result<Command, String> {
    let mut command = Command::new("/usr/bin/bwrap");
    command
        .env_clear()
        .env("PATH", "/usr/bin")
        .env("LANG", "C")
        .args([
            "--unshare-all",
            "--die-with-parent",
            "--new-session",
            "--clearenv",
            "--setenv",
            "PATH",
            "/usr/bin",
            "--setenv",
            "HOME",
            "/nonexistent",
            "--setenv",
            "GST_REGISTRY_FORK",
            "no",
            "--setenv",
            "GST_REGISTRY",
            "/tmp/registry.bin",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--size",
            "16777216",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/app",
            "--ro-bind",
            "/usr",
            "/usr",
            "--ro-bind-try",
            "/lib",
            "/lib",
            "--ro-bind-try",
            "/lib64",
            "/lib64",
            "--ro-bind-try",
            "/etc/ld.so.cache",
            "/etc/ld.so.cache",
        ]);
    crate::sandbox::libraries::bind_aliases(&mut command);
    let test_sink = cfg!(test)
        || (cfg!(debug_assertions)
            && std::env::var("STRATA_MEDIA_TEST_SINK").is_ok_and(|v| v == "1"));
    if !test_sink {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("Audio output is unavailable: no local audio runtime. Start PulseAudio or PipeWire with pipewire-pulse and retry.")?;
        let socket = audio_socket(Path::new(&runtime))?;
        command
            .arg("--ro-bind")
            .arg(socket)
            .arg("/run/strata-audio/pulse/native")
            .args([
                "--setenv",
                "PULSE_SERVER",
                "unix:/run/strata-audio/pulse/native",
            ]);
        if let Some(home) = std::env::var_os("HOME") {
            let cookie = Path::new(&home).join(".config/pulse/cookie");
            if std::fs::symlink_metadata(&cookie).is_ok_and(|m| m.is_file() && m.len() <= 4096) {
                command
                    .arg("--ro-bind")
                    .arg(cookie)
                    .arg("/run/strata-audio/pulse/cookie")
                    .args(["--setenv", "PULSE_COOKIE", "/run/strata-audio/pulse/cookie"]);
            }
        }
    }
    command
        .arg("--ro-bind")
        .arg(executable)
        .arg("/app/strata-media-helper")
        .args([
            "--",
            "/usr/bin/prlimit",
            "--core=0",
            "--as=2147483648",
            "--fsize=4194304",
            "--",
            "/app/strata-media-helper",
            if test_sink {
                "--pcm-test-v1"
            } else {
                "--pcm-v1"
            },
            crate::build_info::RELEASE_TAG,
            crate::build_info::COMMIT,
        ])
        .arg(job.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

fn audio_socket(runtime: &Path) -> Result<std::path::PathBuf, String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let runtime = crate::media_helper::trusted_directory(runtime)?;
    let uid = rustix::process::geteuid().as_raw();
    let metadata =
        std::fs::symlink_metadata(&runtime).map_err(|_| "Cannot inspect the audio runtime")?;
    if metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err("Audio runtime is not private and user-owned; check XDG_RUNTIME_DIR.".into());
    }
    let pulse = crate::media_helper::trusted_directory(&runtime.join("pulse"))?;
    let socket = pulse.join("native");
    if !std::fs::symlink_metadata(&socket)
        .is_ok_and(|m| m.file_type().is_socket() && m.uid() == uid)
    {
        return Err("Audio output is unavailable: no trusted local PulseAudio socket. Start PulseAudio or PipeWire with pipewire-pulse and retry.".into());
    }
    Ok(socket)
}

fn run(state: &State, receiver: mpsc::Receiver<Message>) -> Result<(), String> {
    let directory = crate::media_helper::private_tempdir()?;
    let executable = crate::media_helper::snapshot(directory.path())?;
    let job = crate::media_helper::job();
    let mut command = command(&executable, job)?;
    let mut child =
        crate::sandbox::spawn_renderer(&mut command).map_err(crate::media_helper::spawn_error)?;
    let result = communicate(state, receiver, &mut child, job);
    crate::sandbox::terminate(&mut child);
    result.map_err(|error| crate::media_helper::failure(&mut child, error))
}

fn communicate(
    state: &State,
    receiver: mpsc::Receiver<Message>,
    child: &mut std::process::Child,
    job: u64,
) -> Result<(), String> {
    let stdin = child.stdin.take().ok_or("Missing audio input pipe")?;
    let stdout = child.stdout.take().ok_or("Missing audio output pipe")?;
    let mut reader = TimedReader {
        fd: &stdout,
        deadline: Instant::now() + FRAME_TIMEOUT,
        cancellation: &state.cancellation,
    };
    let mut writer = TimedWriter::new(&stdin, Instant::now() + FRAME_TIMEOUT, &state.cancellation)
        .map_err(|e| e.to_string())?;
    ipc::check_hello(
        &mut reader,
        ipc::PCM,
        job,
        crate::build_info::RELEASE_TAG,
        crate::build_info::COMMIT,
    )
    .map_err(|e| e.to_string())?;
    let mut sequence = 0_u64;
    let mut settings = None;
    let mut playing = false;
    let mut capacity = true;
    let mut finished = false;
    let mut pending = None;
    state.ready.store(true, Ordering::Release);
    loop {
        if state.cancellation.is_cancelled() {
            return Ok(());
        }
        let desired = (
            state.muted.load(Ordering::Acquire),
            state.volume.load(Ordering::Acquire),
        );
        let desired_playing = state.playing.load(Ordering::Acquire);
        let message = if settings != Some(desired) {
            settings = Some(desired);
            let mut data = u64::from(desired.0).to_le_bytes().to_vec();
            data.extend_from_slice(&desired.1.to_le_bytes());
            Message {
                kind: Kind::Settings,
                time_us: 0,
                data,
            }
        } else if playing != desired_playing {
            playing = desired_playing;
            Message::control(if playing { Kind::Play } else { Kind::Pause })
        } else {
            if pending.is_none() {
                pending = receiver.try_recv().ok();
            }
            if capacity && pending.is_some() {
                pending.take().ok_or("Missing queued audio")?
            } else if !finished
                && state.finish.load(Ordering::Acquire)
                && state.queued.load(Ordering::Acquire) == 0
            {
                finished = true;
                Message::control(Kind::Finish)
            } else {
                Message::control(Kind::Poll)
            }
        };
        writer.deadline = Instant::now() + FRAME_TIMEOUT;
        message
            .write(&mut writer, job, sequence)
            .map_err(|e| e.to_string())?;
        reader.deadline = Instant::now() + FRAME_TIMEOUT;
        let status = Message::read(&mut reader, job, sequence).map_err(|e| e.to_string())?;
        if status.kind != Kind::Status
            || u64_at(&status.data, 0) > 1
            || u64_at(&status.data, 16) > 1
        {
            return Err("Invalid audio worker response".into());
        }
        if u64_at(&status.data, 16) != 0 {
            return Err(
                "Audio output failed. Check the PulseAudio or PipeWire audio service and retry."
                    .into(),
            );
        }
        capacity = u64_at(&status.data, 0) == 1;
        let position = u64_at(&status.data, 8);
        let old = state.position.load(Ordering::Acquire);
        if position != u64::MAX {
            if position > 30_000_000 || (old != u64::MAX && position < old) {
                return Err("Invalid audio playback clock".into());
            }
            state.position.store(position, Ordering::Release);
        }
        if message.kind == Kind::Samples {
            state.queued.fetch_sub(1, Ordering::AcqRel);
        }
        sequence = sequence.checked_add(1).ok_or("Audio sequence exhausted")?;
        if message.kind == Kind::Poll {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
mod tests;
