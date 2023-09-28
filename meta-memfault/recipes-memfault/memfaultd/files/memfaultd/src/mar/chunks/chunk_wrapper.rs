//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{Chain, Cursor, Read};

use crate::util::io::StreamLen;

/// Add a little wrapper around each chunk containing an identifier and the length.
/// This makes it possible to concatenate multiple chunks in one file.
///
/// Format:
///  - 4 identifier bytes: 'C' 'H' 'N' 'K'
///  - Chunk length (4 bytes, big endian)
///
/// /!\ This format is not accepted by memfault API. You have to remove that
/// wrapper before sending the chunks to Memfault!
pub struct ChunkWrapper<R: Read + StreamLen> {
    stream: Chain<Cursor<Vec<u8>>, R>,
}

impl<R: Read + StreamLen> ChunkWrapper<R> {
    pub fn new(chunk: R) -> Self {
        let mut header: [u8; 8] = [0; 8];
        header[0..4].copy_from_slice(b"CHNK");
        header[4..8].copy_from_slice(&(chunk.stream_len() as u32).to_be_bytes());

        Self {
            stream: Cursor::new(header.to_vec()).chain(chunk),
        }
    }
}

impl<R: Read + StreamLen> Read for ChunkWrapper<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.read(buf)
    }
}

impl<R: Read + StreamLen> StreamLen for ChunkWrapper<R> {
    fn stream_len(&self) -> u64 {
        self.stream.stream_len()
    }
}
