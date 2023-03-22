//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::BufReader;

/// A trait for getting the length of a stream.
/// Note std::io::Seek also has a stream_len() method, but that method is fallible.
pub trait StreamLen {
    /// Gets the length of the stream in bytes.
    fn stream_len(&self) -> u64;
}

impl<R: StreamLen> StreamLen for BufReader<R> {
    fn stream_len(&self) -> u64 {
        self.get_ref().stream_len()
    }
}
