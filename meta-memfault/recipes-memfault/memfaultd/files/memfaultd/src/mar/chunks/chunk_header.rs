//
// Copyright (c) Memfault, Inc.
// See License.txt for details
/// Chunk protocol supports two types of chunks: Init and Cont.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum ChunkType {
    /// First chunk of a message (with `more_data` bit set if there is more data to follow).
    Init = 0,
    /// All following chunks of a message (with more_data bit set, except for the last one).
    #[allow(dead_code)]
    Cont = 0x80,
}

/// Header of one Memfault Chunk
pub struct ChunkHeader {
    channel_id: u8,
    chunk_type: ChunkType,
    more_data: bool,

    // Note: we only support single messages for now so header is always 1 byte.
    // We can use Box<[u8]> when we want to support including the length/offset in the header
    as_bytes: [u8; 1],
}

impl ChunkHeader {
    const CHUNK_CHANNEL: u8 = 0;
    const CHUNK_CHANNEL_MASK: u8 = 0x7;
    const CHUNK_MORE_DATA_BIT: u8 = 0x40;
    const CHUNK_CRC_DEFERRED_BIT: u8 = 0x08;

    /// Returns a header for one message encoded as a single chunk with deferred CRC.
    pub fn new_single() -> Self {
        let mut header = Self {
            channel_id: Self::CHUNK_CHANNEL,
            chunk_type: ChunkType::Init,
            more_data: false,
            as_bytes: [0],
        };
        header.calculate_header_bytes();
        header
    }

    fn calculate_header_bytes(&mut self) {
        let byte = &mut self.as_bytes[0];
        *byte = 0;
        *byte |= self.channel_id & Self::CHUNK_CHANNEL_MASK;
        if self.more_data {
            *byte |= Self::CHUNK_MORE_DATA_BIT;
        }
        *byte |= self.chunk_type as u8;
        *byte |= Self::CHUNK_CRC_DEFERRED_BIT;
    }

    /// Return a byte representation of the header
    /// length can vary from 1 to 5 bytes (one byte + one varint)
    pub fn as_bytes(&self) -> &[u8] {
        &self.as_bytes
    }
}
