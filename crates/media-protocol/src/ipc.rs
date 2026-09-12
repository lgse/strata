// SPDX-License-Identifier: MIT

use std::io::{self, Read, Write};
use crate::{AUDIO_BYTES, LIMIT_US, SAMPLE_RATE, invalid};

pub const PROTOCOL: u32 = 1;
pub const PARSER: u32 = 1;
pub const PCM: u32 = 2;
const HELLO_BYTES: usize = 128;
const MESSAGE_BYTES: usize = 40;

pub fn hello(writer: &mut impl Write, role: u32, job: u64, release: &str, commit: &str) -> io::Result<()> {
    let bytes = identity(role, job, release, commit)?;
    writer.write_all(&bytes)?;
    writer.flush()
}

pub fn check_hello(reader: &mut impl Read, role: u32, job: u64, release: &str, commit: &str) -> io::Result<()> {
    let expected = identity(role, job, release, commit)?;
    let mut bytes = [0; HELLO_BYTES];
    reader.read_exact(&mut bytes)?;
    if bytes != expected { return Err(invalid("Media helper version/protocol mismatch; reinstall the matching Strata bundle.")); }
    Ok(())
}

fn identity(role: u32, job: u64, release: &str, commit: &str) -> io::Result<[u8; HELLO_BYTES]> {
    if !matches!(role, PARSER | PCM) || job == 0 || release.is_empty() || release.len() > 64 || commit.is_empty() || commit.len() > 40
        || !release.bytes().chain(commit.bytes()).all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b)) {
        return Err(invalid("Invalid media helper identity"));
    }
    let mut bytes = [0; HELLO_BYTES];
    bytes[..8].copy_from_slice(b"STRHLP01");
    bytes[8..12].copy_from_slice(&PROTOCOL.to_le_bytes());
    bytes[12..16].copy_from_slice(&role.to_le_bytes());
    bytes[16..24].copy_from_slice(&job.to_le_bytes());
    bytes[24..24 + release.len()].copy_from_slice(release.as_bytes());
    bytes[88..88 + commit.len()].copy_from_slice(commit.as_bytes());
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Kind { Poll = 0, Settings = 1, Play = 2, Pause = 3, Samples = 4, Finish = 5, Status = 6 }

pub struct Message { pub kind: Kind, pub time_us: u64, pub data: Vec<u8> }

impl Message {
    pub fn control(kind: Kind) -> Self { Self { kind, time_us: 0, data: Vec::new() } }

    pub fn write(&self, writer: &mut impl Write, job: u64, sequence: u64) -> io::Result<()> {
        validate(self.kind, self.time_us, self.data.len())?;
        writer.write_all(b"STRPCM01")?;
        writer.write_all(&job.to_le_bytes())?;
        writer.write_all(&sequence.to_le_bytes())?;
        writer.write_all(&(self.kind as u32).to_le_bytes())?;
        writer.write_all(&(self.data.len() as u32).to_le_bytes())?;
        writer.write_all(&self.time_us.to_le_bytes())?;
        writer.write_all(&self.data)?;
        writer.flush()
    }

    pub fn read(reader: &mut impl Read, job: u64, sequence: u64) -> io::Result<Self> {
        let mut bytes = [0; MESSAGE_BYTES];
        reader.read_exact(&mut bytes)?;
        if &bytes[..8] != b"STRPCM01" || u64_at(&bytes, 8) != job || u64_at(&bytes, 16) != sequence {
            return Err(invalid("Invalid PCM job, generation or sequence"));
        }
        let kind = match u32::from_le_bytes(bytes[24..28].try_into().expect("wire field")) {
            0 => Kind::Poll, 1 => Kind::Settings, 2 => Kind::Play, 3 => Kind::Pause, 4 => Kind::Samples, 5 => Kind::Finish, 6 => Kind::Status,
            _ => return Err(invalid("Unknown PCM operation")),
        };
        let length = u32::from_le_bytes(bytes[28..32].try_into().expect("wire field")) as usize;
        let time_us = u64_at(&bytes, 32);
        validate(kind, time_us, length)?;
        let mut data = vec![0; length];
        reader.read_exact(&mut data)?;
        Ok(Self { kind, time_us, data })
    }
}

fn validate(kind: Kind, time: u64, length: usize) -> io::Result<()> {
    let valid_length = match kind {
        Kind::Samples => length > 0 && length <= AUDIO_BYTES && length.is_multiple_of(4),
        Kind::Settings => length == 16,
        Kind::Status => length == 24,
        _ => length == 0,
    };
    if !valid_length || time > LIMIT_US || (kind != Kind::Samples && time != 0) { return Err(invalid("Invalid PCM framing, timestamp or length")); }
    Ok(())
}

pub fn u64_at(bytes: &[u8], offset: usize) -> u64 { u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("validated wire field")) }

#[derive(Default)]
pub struct PcmSequence { frames: u64, finished: bool }

impl PcmSequence {
    pub fn push(&mut self, length: usize, time_us: u64) -> io::Result<()> {
        validate(Kind::Samples, time_us, length)?;
        let end = self.frames + (length / 4) as u64;
        if self.finished || time_us != self.frames * 1_000_000 / SAMPLE_RATE || end > 30 * SAMPLE_RATE {
            return Err(invalid("PCM samples must be contiguous within one 30-second generation"));
        }
        self.frames = end;
        Ok(())
    }
    pub fn finish(&mut self) -> io::Result<()> {
        if self.frames == 0 { return Err(invalid("PCM ended before any samples")); }
        self.finished = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
