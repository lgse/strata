// SPDX-License-Identifier: MIT

use std::{
    io::{self, Read},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    media::{Decoder, FRAME_TIMEOUT, Header, Packet, STARTUP_TIMEOUT, TimedReader},
    services::SandboxedMedia,
};

use super::*;

pub(crate) const MAX_WORKERS: usize = 4;
const QUEUED_PACKETS: usize = 3;
static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);

struct WorkerSlot;

impl WorkerSlot {
    fn acquire() -> Option<Self> {
        ACTIVE_WORKERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_WORKERS).then_some(count + 1)
            })
            .ok()
            .map(|_| Self)
    }
}

impl Drop for WorkerSlot {
    fn drop(&mut self) {
        let previous = ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        tracing::debug!(
            active_workers = previous - 1,
            "sandboxed media worker stopped"
        );
    }
}

pub(crate) enum Event {
    Prepared(Header),
    Packet(Packet),
    Failed(String),
}

pub(crate) struct Session {
    cancellation: Cancellation,
    receiver: mpsc::Receiver<Event>,
    worker: thread::JoinHandle<()>,
    _slot: Arc<WorkerSlot>,
}

impl Session {
    pub fn start(source: SandboxedMedia, start_tick: u32) -> Result<Self, String> {
        let slot = WorkerSlot::acquire().ok_or_else(|| "Media previews are busy (four active players). Pause or close another preview and retry.".to_owned())?;
        let slot = Arc::new(slot);
        let worker_slot = slot.clone();
        tracing::debug!(
            active_workers = ACTIVE_WORKERS.load(Ordering::Acquire),
            "sandboxed media worker started"
        );
        let cancellation = Cancellation::default();
        let cancelled = cancellation.clone();
        let (sender, receiver) = mpsc::sync_channel(QUEUED_PACKETS);
        let worker = thread::Builder::new()
            .name("media-preview".into())
            .spawn(move || {
                let _slot = worker_slot;
                if let Err(error) = render(&source, start_tick, &cancelled, &sender) {
                    let _sent = send(&sender, Event::Failed(error), &cancelled);
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            cancellation,
            receiver,
            worker,
            _slot: slot,
        })
    }

    pub fn receive(&self) -> Option<Event> {
        match self.receiver.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Event::Failed(
                "The sandboxed media worker stopped unexpectedly".into(),
            )),
        }
    }

    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub fn finished(&self) -> bool {
        self.worker.is_finished()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn send(
    sender: &mpsc::SyncSender<Event>,
    mut event: Event,
    cancellation: &Cancellation,
) -> Result<(), String> {
    loop {
        if cancellation.is_cancelled() {
            return Err("Preview cancelled".into());
        }
        match sender.try_send(event) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Disconnected(_)) => return Err("Preview closed".into()),
            Err(mpsc::TrySendError::Full(pending)) => event = pending,
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// The returned output directory may hold the executable snapshot bwrap binds,
/// so it must outlive the helper.
fn spawn_helper(
    source: &SandboxedMedia,
    operation: ParseOperation,
    start_tick: Option<u32>,
    cancellation: &Cancellation,
) -> Result<(Child, PrivateOutput), String> {
    let input = source
        .path
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !input.is_file() {
        return Err("Preview input is not a regular file".into());
    }
    let output = PrivateOutput::create().map_err(|error| error.to_string())?;
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    let running = PathBuf::from(format!("/proc/{}/exe", std::process::id()));
    let executable = resolve_renderer_executable(&current, &running, output.path())?;
    let bwrap = crate::trusted_command::resolve("bwrap")
        .map_err(|error| format!("Unable to start the preview sandbox: {error}"))?;
    let backend = if matches!(operation, ParseOperation::PreviewMedia(_)) {
        source.backend
    } else {
        MediaPreviewBackend::Software
    };
    let devices = gpu_devices(Path::new("/dev"), backend);
    let mut command = sandbox_command(
        &bwrap,
        &executable,
        &input,
        output.path(),
        operation,
        0,
        backend,
        &devices,
    );
    if let Some(start_tick) = start_tick {
        command.arg(start_tick.to_string());
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if cancellation.is_cancelled() {
        return Err("Preview cancelled".into());
    }
    let child = spawn_renderer(&mut command)
        .map_err(|error| format!("Unable to start the preview sandbox: {error}"))?;
    Ok((child, output))
}

fn render(
    source: &SandboxedMedia,
    start_tick: u32,
    cancellation: &Cancellation,
    sender: &mpsc::SyncSender<Event>,
) -> Result<(), String> {
    let operation = if source.audio_only {
        ParseOperation::PreviewAudio(source.size)
    } else {
        ParseOperation::PreviewMedia(source.size)
    };
    let (mut child, _output) = spawn_helper(source, operation, Some(start_tick), cancellation)?;
    let result = consume(&mut child, source, start_tick, cancellation, sender);
    // Also tear down descendants which keep a pipe open or outlive their leader.
    if result.is_err() {
        terminate(&mut child);
    }
    result.map_err(|error| error.to_string())
}

fn consume(
    child: &mut Child,
    source: &SandboxedMedia,
    start_tick: u32,
    cancellation: &Cancellation,
    sender: &mpsc::SyncSender<Event>,
) -> io::Result<()> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Missing decoded-media pipe"))?;
    let mut reader = TimedReader {
        fd: &stdout,
        deadline: Instant::now() + STARTUP_TIMEOUT,
        cancellation,
    };
    let header = Header::read(&mut reader, source.size, start_tick)?;
    send(sender, Event::Prepared(header), cancellation).map_err(io::Error::other)?;
    let mut decoder = Decoder::new(header);
    loop {
        reader.deadline = Instant::now() + FRAME_TIMEOUT;
        let packet = decoder.read(&mut reader)?;
        if let Packet::End(duration) = packet {
            reader.deadline = Instant::now() + Duration::from_secs(2);
            if reader.read(&mut [0])? != 0 {
                return Err(io::Error::other("Trailing decoded-media output"));
            }
            let status = wait_for_renderer(child, cancellation, Duration::from_secs(2))
                .map_err(io::Error::other)?;
            if !status.success() {
                return Err(io::Error::other("The sandboxed decoder failed"));
            }
            send(sender, Event::Packet(Packet::End(duration)), cancellation)
                .map_err(io::Error::other)?;
            return Ok(());
        }
        send(sender, Event::Packet(packet), cancellation).map_err(io::Error::other)?;
    }
}

pub(crate) enum PeaksEvent {
    Levels { start: u32, levels: Vec<u8> },
    Finished,
    Failed,
}

const PEAKS_TIMEOUT: Duration = Duration::from_secs(90);
static ACTIVE_PEAKS: AtomicUsize = AtomicUsize::new(0);

struct PeaksSlot;

impl Drop for PeaksSlot {
    fn drop(&mut self) {
        ACTIVE_PEAKS.store(0, Ordering::Release);
    }
}

/// A background waveform-overview decode. Dropping it cancels the sandbox.
pub(crate) struct PeaksSession {
    cancellation: Cancellation,
    receiver: mpsc::Receiver<PeaksEvent>,
}

impl PeaksSession {
    /// Only one overview decodes at a time; callers retry once the slot frees.
    pub fn start(source: SandboxedMedia) -> Option<Self> {
        ACTIVE_PEAKS
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        let cancellation = Cancellation::default();
        let cancelled = cancellation.clone();
        let (sender, receiver) = mpsc::sync_channel(64);
        let spawned = thread::Builder::new()
            .name("audio-peaks".into())
            .spawn(move || {
                let _slot = PeaksSlot;
                let event = match render_peaks(&source, &cancelled, &sender) {
                    Ok(()) => PeaksEvent::Finished,
                    Err(_) => PeaksEvent::Failed,
                };
                let _sent = sender.send(event);
            });
        if spawned.is_err() {
            ACTIVE_PEAKS.store(0, Ordering::Release);
            return None;
        }
        Some(Self {
            cancellation,
            receiver,
        })
    }

    pub fn receive(&self) -> Option<PeaksEvent> {
        match self.receiver.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(PeaksEvent::Failed),
        }
    }
}

impl Drop for PeaksSession {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

fn render_peaks(
    source: &SandboxedMedia,
    cancellation: &Cancellation,
    sender: &mpsc::SyncSender<PeaksEvent>,
) -> Result<(), String> {
    let (mut child, _output) =
        spawn_helper(source, ParseOperation::AudioPeaks, None, cancellation)?;
    let result = consume_peaks(&mut child, cancellation, sender);
    if result.is_err() {
        terminate(&mut child);
    }
    result.map_err(|error| error.to_string())
}

fn consume_peaks(
    child: &mut Child,
    cancellation: &Cancellation,
    sender: &mpsc::SyncSender<PeaksEvent>,
) -> io::Result<()> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Missing waveform pipe"))?;
    let deadline = Instant::now() + PEAKS_TIMEOUT;
    let mut reader = TimedReader {
        fd: &stdout,
        deadline: Instant::now() + STARTUP_TIMEOUT,
        cancellation,
    };
    crate::media::peaks::read_header(&mut reader)?;
    let forward = |event| {
        sender
            .send(event)
            .map_err(|_| io::Error::other("Waveform preview closed"))
    };
    let mut runs = crate::media::peaks::RunReader::default();
    loop {
        reader.deadline = deadline.min(Instant::now() + FRAME_TIMEOUT);
        let Some((start, levels)) = runs.read(&mut reader)? else {
            break;
        };
        forward(PeaksEvent::Levels { start, levels })?;
    }
    reader.deadline = Instant::now() + Duration::from_secs(2);
    if reader.read(&mut [0])? != 0 {
        return Err(io::Error::other("Trailing waveform output"));
    }
    let status =
        wait_for_renderer(child, cancellation, Duration::from_secs(2)).map_err(io::Error::other)?;
    if !status.success() {
        return Err(io::Error::other("The sandboxed waveform decoder failed"));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
