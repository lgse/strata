// SPDX-License-Identifier: MIT

use std::{
    io::{self, Read, Write},
    mem::MaybeUninit,
    os::fd::{AsFd, OwnedFd},
};

use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, recvmsg, sendmsg,
};

use super::super::metadata::MAX_METADATA_BYTES;

pub(super) const MAX_OUTPUT_BYTES: u64 = super::super::MAX_OUTPUT_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Operation {
    Image = 1,
    Raw = 2,
    Pdf = 3,
    Video = 4,
    ImageMetadata = 5,
    MediaMetadata = 6,
    PreviewImage = 7,
    DocumentMermaid = 8,
    DocumentMath = 9,
    DocumentMathInline = 10,
}

impl Operation {
    pub(super) fn parse(value: u8) -> io::Result<Self> {
        match value {
            1 => Ok(Self::Image),
            2 => Ok(Self::Raw),
            3 => Ok(Self::Pdf),
            4 => Ok(Self::Video),
            5 => Ok(Self::ImageMetadata),
            6 => Ok(Self::MediaMetadata),
            7 => Ok(Self::PreviewImage),
            8 => Ok(Self::DocumentMermaid),
            9 => Ok(Self::DocumentMath),
            10 => Ok(Self::DocumentMathInline),
            _ => Err(io::Error::other("Unknown browser operation")),
        }
    }
}

pub(super) fn send(
    socket: &impl AsFd,
    input: &impl AsFd,
    output: &impl AsFd,
    operation: Operation,
) -> io::Result<()> {
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    let descriptors = [input.as_fd(), output.as_fd()];
    if !ancillary.push(SendAncillaryMessage::ScmRights(&descriptors)) {
        return Err(io::Error::other("Unable to send browser input"));
    }
    let bytes = [operation as u8];
    let count = sendmsg(
        socket,
        &[io::IoSlice::new(&bytes)],
        &mut ancillary,
        SendFlags::NOSIGNAL,
    )?;
    if count != 1 {
        return Err(io::Error::other("Incomplete browser request"));
    }
    Ok(())
}

pub(super) fn receive(socket: &impl AsFd) -> io::Result<Option<(Operation, OwnedFd, OwnedFd)>> {
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(3))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let mut bytes = [0];
    let result = recvmsg(
        socket,
        &mut [io::IoSliceMut::new(&mut bytes)],
        &mut ancillary,
        RecvFlags::CMSG_CLOEXEC,
    )?;
    let descriptors: Vec<_> = ancillary
        .drain()
        .flat_map(|message| match message {
            RecvAncillaryMessage::ScmRights(fds) => fds.collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();
    if result.bytes == 0 && descriptors.is_empty() {
        return Ok(None);
    }
    if result.bytes != 1
        || result
            .flags
            .intersects(rustix::net::ReturnFlags::TRUNC | rustix::net::ReturnFlags::CTRUNC)
        || descriptors.len() != 2
    {
        return Err(io::Error::other("Invalid browser request descriptors"));
    }
    let mut descriptors = descriptors.into_iter();
    Ok(Some((
        Operation::parse(bytes[0])?,
        descriptors
            .next()
            .ok_or_else(|| io::Error::other("Missing input"))?,
        descriptors
            .next()
            .ok_or_else(|| io::Error::other("Missing output"))?,
    )))
}

#[derive(Default)]
pub(crate) struct Response {
    pub(crate) png: Vec<u8>,
    pub(crate) metadata: Vec<u8>,
}

impl Response {
    pub(super) fn read(reader: &mut impl Read) -> io::Result<Self> {
        let mut header = [0; 8];
        reader.read_exact(&mut header)?;
        let png = u32::from_le_bytes(header[..4].try_into().expect("fixed header")) as usize;
        let metadata = u32::from_le_bytes(header[4..].try_into().expect("fixed header")) as usize;
        if png as u64 > MAX_OUTPUT_BYTES || metadata as u64 > MAX_METADATA_BYTES {
            return Err(io::Error::other("Oversized browser response"));
        }
        let mut response = Self {
            png: vec![0; png],
            metadata: vec![0; metadata],
        };
        reader.read_exact(&mut response.png)?;
        reader.read_exact(&mut response.metadata)?;
        Ok(response)
    }

    pub(super) fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        if self.png.len() as u64 > MAX_OUTPUT_BYTES
            || self.metadata.len() as u64 > MAX_METADATA_BYTES
        {
            return Err(io::Error::other("Oversized browser response"));
        }
        writer.write_all(&(self.png.len() as u32).to_le_bytes())?;
        writer.write_all(&(self.metadata.len() as u32).to_le_bytes())?;
        writer.write_all(&self.png)?;
        writer.write_all(&self.metadata)?;
        writer.flush()
    }
}
