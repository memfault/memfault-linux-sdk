//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{Chain, Cursor, Read};

use crate::util::io::StreamLen;

use super::{chunk_header::ChunkHeader, CRCPaddedStream};

/// One chunk of a message.
/// This implementation only supports Chunks v2 (with CRC deferred to the end of the last chunk composing the message).
pub struct Chunk<M: Read + StreamLen> {
    stream: Chain<Cursor<Vec<u8>>, CRCPaddedStream<M>>,
}

impl<M: Read + StreamLen> Chunk<M> {
    /// Create a new single chunk for a message.
    pub fn new_single(message: M) -> Self {
        Self {
            stream: Cursor::new(ChunkHeader::new_single().as_bytes().to_vec())
                .chain(CRCPaddedStream::new(message)),
        }
    }
}

impl<M: Read + StreamLen> Read for Chunk<M> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.read(buf)
    }
}

impl<M: Read + StreamLen> StreamLen for Chunk<M> {
    fn stream_len(&self) -> u64 {
        self.stream.stream_len()
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::mar::ChunkMessage;

    use super::*;
    use std::io::copy;

    #[rstest]
    fn test_single_chunk_message() {
        // https://docs.memfault.com/docs/mcu/test-patterns-for-chunks-endpoint/#event-message-encoded-in-a-single-chunk
        let known_good_chunk = [
            0x8, 0x2, 0xa7, 0x2, 0x1, 0x3, 0x1, 0x7, 0x6a, 0x54, 0x45, 0x53, 0x54, 0x53, 0x45,
            0x52, 0x49, 0x41, 0x4c, 0xa, 0x6d, 0x74, 0x65, 0x73, 0x74, 0x2d, 0x73, 0x6f, 0x66,
            0x74, 0x77, 0x61, 0x72, 0x65, 0x9, 0x6a, 0x31, 0x2e, 0x30, 0x2e, 0x30, 0x2d, 0x74,
            0x65, 0x73, 0x74, 0x6, 0x6d, 0x74, 0x65, 0x73, 0x74, 0x2d, 0x68, 0x61, 0x72, 0x64,
            0x77, 0x61, 0x72, 0x65, 0x4, 0xa1, 0x1, 0xa1, 0x72, 0x63, 0x68, 0x75, 0x6e, 0x6b, 0x5f,
            0x74, 0x65, 0x73, 0x74, 0x5f, 0x73, 0x75, 0x63, 0x63, 0x65, 0x73, 0x73, 0x1, 0x31,
            0xe4,
        ];

        // Remove 2 bytes chunk header and 2 bytes CRC
        let data = known_good_chunk[2..known_good_chunk.len() - 2].to_vec();

        let mut chunk = Chunk::new_single(ChunkMessage::new(
            crate::mar::ChunkMessageType::Event,
            Cursor::new(data),
        ));

        let mut buf: Cursor<Vec<u8>> = Cursor::new(vec![]);
        assert_eq!(
            copy(&mut chunk, &mut buf).expect("copy ok"),
            known_good_chunk.len() as u64
        );
        assert_eq!(buf.get_ref().as_slice(), known_good_chunk);
    }

    impl StreamLen for std::io::Cursor<std::vec::Vec<u8>> {
        fn stream_len(&self) -> u64 {
            self.get_ref().len() as u64
        }
    }
}
