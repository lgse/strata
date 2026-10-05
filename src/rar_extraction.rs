// SPDX-License-Identifier: MIT

//! Framed RAR member stream between the sandboxed decoder and the parent.

use std::io::{self, Read, Write};

const MAGIC: &[u8; 8] = b"STRRAR03";
/// Generous enough for any real archive member name or error message, small
/// enough that a malformed/compromised child cannot force a huge allocation.
const MAX_TEXT_BYTES: u32 = 8192;
const HEADER_BYTES: usize = 32;
const NO_MODE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WireTime {
    /// Windows FILETIME (UTC), as RAR 5 headers store it.
    FileTime(u64),
    /// MS-DOS date (high 16 bits) and time in the archiver's local zone, as
    /// RAR 1.5–4 headers store it. Only the parent knows the user's zone.
    DosLocal(u32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WireMetadata {
    pub(crate) mode: Option<u32>,
    pub(crate) modified: Option<WireTime>,
}

/// Why the decoder failed, so the parent can offer a password retry without
/// reading the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureKind {
    Other,
    PasswordRequired,
    IncorrectPassword,
}

impl FailureKind {
    fn code(self) -> u32 {
        match self {
            Self::Other => 0,
            Self::PasswordRequired => 1,
            Self::IncorrectPassword => 2,
        }
    }

    fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Other),
            1 => Some(Self::PasswordRequired),
            2 => Some(Self::IncorrectPassword),
            _ => None,
        }
    }
}

/// A decoder failure as an error record or a failed member trailer carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) kind: FailureKind,
    pub(crate) message: String,
}

impl Failure {
    pub(crate) fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Recovers the failure a [`FileBody`] read reported from its trailer.
    pub(crate) fn from_io(error: io::Error) -> Self {
        match error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<Self>())
        {
            Some(failure) => failure.clone(),
            None => Self::from(error.to_string()),
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::new(FailureKind::Other, message)
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::new(FailureKind::Other, message)
    }
}

impl From<Failure> for String {
    fn from(failure: Failure) -> Self {
        failure.message
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}

/// Error records use kinds from this value up, one per [`FailureKind`].
const ERROR_RECORD: u32 = 3;
/// Failed member trailers use statuses from this value up, one per [`FailureKind`].
const FAILED_TRAILER: u32 = 1;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Record {
    Directory(String, WireMetadata),
    File(String, u64, WireMetadata),
    End,
    Error(Failure),
}

pub(crate) fn write_magic(writer: &mut impl Write) -> io::Result<()> {
    writer.write_all(MAGIC)
}

pub(crate) fn read_magic(reader: &mut impl Read) -> io::Result<()> {
    let mut bytes = [0; MAGIC.len()];
    reader.read_exact(&mut bytes)?;
    if &bytes != MAGIC {
        return Err(invalid("Unknown RAR extraction stream format"));
    }
    Ok(())
}

pub(crate) fn write_directory(
    writer: &mut impl Write,
    name: &str,
    metadata: WireMetadata,
) -> io::Result<()> {
    write_header(writer, 0, name, 0, metadata)
}

pub(crate) fn write_file_header(
    writer: &mut impl Write,
    name: &str,
    size: u64,
    metadata: WireMetadata,
) -> io::Result<()> {
    write_header(writer, 1, name, size, metadata)
}

pub(crate) fn write_end(writer: &mut impl Write) -> io::Result<()> {
    write_header(writer, 2, "", 0, WireMetadata::default())
}

pub(crate) fn write_error(writer: &mut impl Write, failure: &Failure) -> io::Result<()> {
    write_header(
        writer,
        ERROR_RECORD + failure.kind.code(),
        &failure.message,
        0,
        WireMetadata::default(),
    )
}

fn write_header(
    writer: &mut impl Write,
    kind: u32,
    text: &str,
    size: u64,
    metadata: WireMetadata,
) -> io::Result<()> {
    let bytes = text.as_bytes();
    writer.write_all(&kind.to_le_bytes())?;
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&size.to_le_bytes())?;
    let (time_kind, time) = match metadata.modified {
        None => (0u32, 0),
        Some(WireTime::FileTime(value)) => (1, value),
        Some(WireTime::DosLocal(value)) => (2, u64::from(value)),
    };
    writer.write_all(&metadata.mode.unwrap_or(NO_MODE).to_le_bytes())?;
    writer.write_all(&time_kind.to_le_bytes())?;
    writer.write_all(&time.to_le_bytes())?;
    writer.write_all(bytes)
}

/// Each body chunk is length-prefixed so a decoder failure before the declared
/// size cannot be mistaken for file contents.
pub(crate) fn write_chunk(writer: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    let length = u32::try_from(bytes.len()).map_err(|_| invalid("RAR chunk is too large"))?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(bytes)
}

pub(crate) fn write_file_ok(writer: &mut impl Write) -> io::Result<()> {
    writer.write_all(&0u32.to_le_bytes())?;
    write_trailer(writer, 0, "")
}

pub(crate) fn write_file_failed(writer: &mut impl Write, failure: &Failure) -> io::Result<()> {
    writer.write_all(&0u32.to_le_bytes())?;
    write_trailer(
        writer,
        FAILED_TRAILER + failure.kind.code(),
        &failure.message,
    )
}

/// The framed member reader validates the trailer before reporting EOF to the
/// destination, so failed or truncated members are removed by ExtractionSession.
/// A failed trailer's [`Failure`] is the reported error's inner error.
pub(crate) struct FileBody<'a, R> {
    reader: &'a mut R,
    remaining: u64,
    chunk_remaining: u32,
    finished: bool,
}

impl<'a, R: Read> FileBody<'a, R> {
    pub(crate) fn new(reader: &'a mut R, size: u64) -> Self {
        Self {
            reader,
            remaining: size,
            chunk_remaining: 0,
            finished: false,
        }
    }
}

impl<R: Read> Read for FileBody<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() || self.finished {
            return Ok(0);
        }
        if self.chunk_remaining == 0 {
            let mut length = [0; 4];
            self.reader.read_exact(&mut length)?;
            self.chunk_remaining = u32::from_le_bytes(length);
            if self.chunk_remaining == 0 {
                read_file_trailer(self.reader)?
                    .map_err(|failure| io::Error::new(io::ErrorKind::InvalidData, failure))?;
                if self.remaining != 0 {
                    return Err(invalid("RAR member produced fewer bytes than declared"));
                }
                self.finished = true;
                return Ok(0);
            }
            if u64::from(self.chunk_remaining) > self.remaining {
                return Err(invalid("RAR member produced more bytes than declared"));
            }
        }
        let cap = buffer.len().min(self.chunk_remaining as usize);
        let count = self.reader.read(&mut buffer[..cap])?;
        if count == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        self.chunk_remaining -= count as u32;
        self.remaining -= count as u64;
        Ok(count)
    }
}

fn write_trailer(writer: &mut impl Write, status: u32, message: &str) -> io::Result<()> {
    let bytes = message.as_bytes();
    writer.write_all(&status.to_le_bytes())?;
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)
}

pub(crate) fn read_record(reader: &mut impl Read) -> io::Result<Record> {
    let mut header = [0; HEADER_BYTES];
    reader.read_exact(&mut header)?;
    let kind = u32_at(&header, 0);
    let text_len = u32_at(&header, 4);
    let size = u64_at(&header, 8);
    let time = u64_at(&header, 24);
    let modified = match (u32_at(&header, 20), u32::try_from(time)) {
        (0, _) if time == 0 => None,
        (1, _) => Some(WireTime::FileTime(time)),
        (2, Ok(dos)) => Some(WireTime::DosLocal(dos)),
        _ => return Err(invalid("Unknown RAR extraction stream time")),
    };
    let metadata = WireMetadata {
        mode: Some(u32_at(&header, 16)).filter(|mode| *mode != NO_MODE),
        modified,
    };
    let no_metadata = metadata == WireMetadata::default();
    if text_len > MAX_TEXT_BYTES {
        return Err(invalid("RAR extraction stream record is too large"));
    }
    let mut text = vec![0; text_len as usize];
    reader.read_exact(&mut text)?;
    let text = String::from_utf8(text)
        .map_err(|_| invalid("RAR extraction stream text is not valid UTF-8"))?;
    match kind {
        0 if size == 0 => Ok(Record::Directory(text, metadata)),
        1 => Ok(Record::File(text, size, metadata)),
        2 if text.is_empty() && size == 0 && no_metadata => Ok(Record::End),
        kind if size == 0 && no_metadata => kind
            .checked_sub(ERROR_RECORD)
            .and_then(FailureKind::from_code)
            .map(|kind| Record::Error(Failure::new(kind, text)))
            .ok_or_else(|| invalid("Unknown RAR extraction stream record")),
        _ => Err(invalid("Unknown RAR extraction stream record")),
    }
}

pub(crate) fn read_file_trailer(reader: &mut impl Read) -> io::Result<Result<(), Failure>> {
    let mut trailer = [0; 8];
    reader.read_exact(&mut trailer)?;
    let status = u32_at(&trailer, 0);
    let message_len = u32_at(&trailer, 4);
    if message_len > MAX_TEXT_BYTES {
        return Err(invalid("RAR extraction stream trailer is too large"));
    }
    let mut message = vec![0; message_len as usize];
    reader.read_exact(&mut message)?;
    let message = String::from_utf8(message)
        .map_err(|_| invalid("RAR extraction stream trailer is not valid UTF-8"))?;
    match status {
        0 if message.is_empty() => Ok(Ok(())),
        status => status
            .checked_sub(FAILED_TRAILER)
            .and_then(FailureKind::from_code)
            .map(|kind| Err(Failure::new(kind, message)))
            .ok_or_else(|| invalid("Unknown RAR extraction stream trailer")),
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

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests;
