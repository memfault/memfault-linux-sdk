//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{Cursor, Read};

use crc::{Crc, Digest};
use crc_catalog::CRC_16_XMODEM;
use once_cell::sync::Lazy;

use crate::util::io::StreamLen;

static CRC16_XMODEM: Lazy<Crc<u16>> = Lazy::new(|| Crc::<u16>::new(&CRC_16_XMODEM));
static CRC16_XMODEM_LENGTH: u64 = 2;

/// A stream which will be followed by a CRC.
pub struct CRCPaddedStream<R: Read + StreamLen> {
    stream: R,
    /// Keep a running CRC as the data is being read.
    /// Will be None once all the stream data has been read.
    crc: Option<Digest<'static, u16>>,
    /// Remaining crc bytes to be read.
    crc_bytes: Cursor<Vec<u8>>,
}

impl<R: Read + StreamLen> CRCPaddedStream<R> {
    pub fn new(stream: R) -> Self {
        Self {
            stream,
            crc: Some(CRC16_XMODEM.digest()),
            crc_bytes: Cursor::new(vec![]),
        }
    }
}

impl<R: Read + StreamLen> Read for CRCPaddedStream<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let result = self.stream.read(buf)?;

        if result > 0 {
            if let Some(crc) = &mut self.crc {
                crc.update(&buf[..result]);
            } else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "CRC already finalized",
                ));
            }
            return Ok(result);
        }

        // When done reading, write the CRC.
        if let Some(crc) = self.crc.take() {
            self.crc_bytes = Cursor::new(crc.finalize().to_le_bytes().to_vec());
        }

        self.crc_bytes.read(buf)
    }
}

impl<R: Read + StreamLen> StreamLen for CRCPaddedStream<R> {
    fn stream_len(&self) -> u64 {
        self.stream.stream_len() + CRC16_XMODEM_LENGTH
    }
}
