// SPDX-License-Identifier: MIT

use std::{
    io::{self, Read, Write},
    os::fd::AsFd,
    time::{Duration, Instant},
};

mod types;
pub use types::{Cancellation, MediaPreviewBackend, MediaPreviewSize};
pub mod ipc;
pub mod devices;

pub const FPS: u32 = 30;
pub const LIMIT_US: u64 = 30_000_000;
pub const MAX_TICKS: u32 = 900;
pub const SAMPLE_RATE: u64 = 48_000;
pub const AUDIO_BYTES: usize = 6_400;
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(22);
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(8);
pub const HEADER_BYTES: usize = 40;
const MAGIC: &[u8; 8] = b"STRRAW01";

pub fn timestamp(tick: u32) -> u64 {
    u64::from(tick) * 1_000_000 / u64::from(FPS)
}

pub fn seek_tick(time_us: u64, duration_us: u64) -> u32 {
    (time_us.min(duration_us.saturating_sub(1)).min(LIMIT_US - 1) * u64::from(FPS) / 1_000_000)
        as u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    pub audio: bool,
    pub duration_us: u64,
    pub start_tick: u32,
}

impl Header {
    pub fn video_bytes(self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    pub fn ticks(self) -> u32 {
        (self.duration_us * u64::from(FPS)).div_ceil(1_000_000) as u32
    }

    pub fn validate(self, size: MediaPreviewSize, start_tick: u32) -> io::Result<Self> {
        if self.width > size.width as u32
            || self.height > size.height as u32
            || self.width > MediaPreviewSize::MAX_EDGE as u32
            || self.height > MediaPreviewSize::MAX_EDGE as u32
            || (self.width == 0) != (self.height == 0)
            || (self.width == 0 && !self.audio)
            || self.duration_us == 0
            || self.duration_us > LIMIT_US
            || self.start_tick != start_tick
            || self.start_tick >= self.ticks()
        {
            return Err(invalid("Invalid decoded-media header"));
        }
        Ok(self)
    }

    pub fn read(
        reader: &mut impl Read,
        size: MediaPreviewSize,
        start_tick: u32,
    ) -> io::Result<Self> {
        let mut bytes = [0; HEADER_BYTES];
        reader.read_exact(&mut bytes)?;
        if &bytes[..8] != MAGIC || u32_at(&bytes, 20) > 1 || u32_at(&bytes, 36) != 0 {
            return Err(invalid("Unknown decoded-media format"));
        }
        let header = Self {
            width: u32_at(&bytes, 8),
            height: u32_at(&bytes, 12),
            audio: u32_at(&bytes, 20) == 1,
            duration_us: u64_at(&bytes, 24),
            start_tick: u32_at(&bytes, 32),
        }
        .validate(size, start_tick)?;
        if header.width.checked_mul(4) != Some(u32_at(&bytes, 16)) {
            return Err(invalid("Invalid decoded-frame stride"));
        }
        Ok(header)
    }

    pub fn write(self, writer: &mut impl Write) -> io::Result<()> {
        writer.write_all(MAGIC)?;
        for value in [
            self.width,
            self.height,
            self.width * 4,
            u32::from(self.audio),
        ] {
            writer.write_all(&value.to_le_bytes())?;
        }
        writer.write_all(&self.duration_us.to_le_bytes())?;
        writer.write_all(&self.start_tick.to_le_bytes())?;
        writer.write_all(&0_u32.to_le_bytes())
    }
}

#[derive(Debug)]
pub struct Frame {
    pub tick: u32,
    pub pixels: Vec<u8>,
    pub samples: Vec<u8>,
}

impl Frame {
    pub fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        write_record(
            writer,
            1,
            self.tick,
            timestamp(self.tick),
            self.pixels.len(),
            self.samples.len(),
        )?;
        writer.write_all(&self.pixels)?;
        writer.write_all(&self.samples)
    }
}

pub fn write_end(writer: &mut impl Write, tick: u32, duration_us: u64) -> io::Result<()> {
    write_record(writer, 0, tick, duration_us, 0, 0)
}

fn write_record(
    writer: &mut impl Write,
    kind: u32,
    tick: u32,
    pts: u64,
    video: usize,
    audio: usize,
) -> io::Result<()> {
    writer.write_all(&kind.to_le_bytes())?;
    writer.write_all(&tick.to_le_bytes())?;
    writer.write_all(&pts.to_le_bytes())?;
    writer.write_all(&(video as u32).to_le_bytes())?;
    writer.write_all(&(audio as u32).to_le_bytes())
}

pub struct Decoder {
    pub header: Header,
    next_tick: u32,
    ended: bool,
}

pub enum Packet {
    Frame(Frame),
    End(u64),
}

impl Decoder {
    pub fn new(header: Header) -> Self {
        Self {
            header,
            next_tick: header.start_tick,
            ended: false,
        }
    }

    pub fn read(&mut self, reader: &mut impl Read) -> io::Result<Packet> {
        let mut bytes = [0; 24];
        reader.read_exact(&mut bytes)?;
        let kind = u32_at(&bytes, 0);
        let tick = u32_at(&bytes, 4);
        let pts = u64_at(&bytes, 8);
        let video = u32_at(&bytes, 16) as usize;
        let audio = u32_at(&bytes, 20) as usize;
        if self.ended || tick != self.next_tick {
            return Err(invalid("Out-of-order decoded frame"));
        }
        if kind == 0 {
            if video != 0
                || audio != 0
                || tick == self.header.start_tick
                || pts <= timestamp(tick - 1)
                || pts > self.header.duration_us
                || pts > timestamp(tick)
            {
                return Err(invalid("Invalid decoded-media end"));
            }
            self.ended = true;
            return Ok(Packet::End(pts));
        }
        if kind != 1
            || tick >= self.header.ticks()
            || tick >= MAX_TICKS
            || pts != timestamp(tick)
            || video != self.header.video_bytes()
            || audio != if self.header.audio { AUDIO_BYTES } else { 0 }
        {
            return Err(invalid(
                "Invalid decoded-frame dimensions, length or timestamp",
            ));
        }
        let mut pixels = vec![0; video];
        let mut samples = vec![0; audio];
        reader.read_exact(&mut pixels)?;
        reader.read_exact(&mut samples)?;
        self.next_tick += 1;
        Ok(Packet::Frame(Frame {
            tick,
            pixels,
            samples,
        }))
    }
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed wire field"),
    )
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("fixed wire field"),
    )
}

pub fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub struct TimedReader<'a, F> {
    pub fd: &'a F,
    pub deadline: Instant,
    pub cancellation: &'a Cancellation,
}

impl<F: AsFd> Read for TimedReader<'_, F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        loop {
            if self.cancellation.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "Preview cancelled",
                ));
            }
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Decoded-media progress timed out",
                ));
            }
            let mut fds = [PollFd::new(self.fd, PollFlags::IN)];
            let timeout = Timespec {
                tv_sec: 0,
                tv_nsec: remaining.min(Duration::from_millis(20)).as_nanos() as i64,
            };
            if poll(&mut fds, Some(&timeout))? != 0 {
                match rustix::io::read(self.fd, &mut *buffer) {
                    Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => continue,
                    result => return result.map_err(Into::into),
                }
            }
        }
    }
}

pub struct TimedWriter<'a, F> {
    pub fd: &'a F,
    pub deadline: Instant,
    pub cancellation: &'a Cancellation,
}

impl<F: AsFd> TimedWriter<'_, F> {
    pub fn new<'a>(fd: &'a F, deadline: Instant, cancellation: &'a Cancellation) -> io::Result<TimedWriter<'a, F>> {
        let flags = rustix::fs::fcntl_getfl(fd)?;
        rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)?;
        Ok(TimedWriter { fd, deadline, cancellation })
    }
}

impl<F: AsFd> Write for TimedWriter<'_, F> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        loop {
            if self.cancellation.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Media cancelled"));
            }
            if Instant::now() >= self.deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "Media transport stalled"));
            }
            match rustix::io::write(self.fd, buffer) {
                Ok(count) => return Ok(count),
                Err(rustix::io::Errno::INTR) => continue,
                Err(rustix::io::Errno::AGAIN) => {},
                Err(error) => return Err(error.into()),
            }
            let mut fds = [PollFd::new(self.fd, PollFlags::OUT)];
            poll(&mut fds, Some(&Timespec { tv_sec: 0, tv_nsec: 10_000_000 }))?;
        }
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

#[cfg(test)]
mod tests;
