//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{Chain, Cursor, Read};

use crate::util::io::StreamLen;

/// A one-byte discriminator for the type of message being sent.
#[derive(Copy, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ChunkMessageType {
    Null = 0,
    McuCoredump = 1,
    Event = 2,
    Logs = 3,
    CustomDataRecording = 4,
    Mar = 5,
}

/// All data sent in chunks must be wrapped in a ChunkMessage.
///
/// It adds a one-byte header indicating the type of message being sent.
pub struct ChunkMessage<R: Read + StreamLen> {
    stream: Chain<Cursor<[u8; 1]>, R>,
}

impl<R: Read + StreamLen> ChunkMessage<R> {
    pub fn new(message_type: ChunkMessageType, data: R) -> Self {
        Self {
            stream: Cursor::new([message_type as u8]).chain(data),
        }
    }
}

impl<R: Read + StreamLen> Read for ChunkMessage<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.read(buf)
    }
}

impl<R: Read + StreamLen> StreamLen for ChunkMessage<R> {
    fn stream_len(&self) -> u64 {
        self.stream.stream_len()
    }
}
