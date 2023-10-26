//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::io::{
    self, copy, sink, BufReader, Chain, Cursor, ErrorKind, IoSlice, Read, Seek, SeekFrom, Write,
};

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

impl<A: AsRef<[u8]>, B: StreamLen> StreamLen for Chain<Cursor<A>, B> {
    fn stream_len(&self) -> u64 {
        let (a, b) = self.get_ref();

        a.get_ref().as_ref().len() as u64 + b.stream_len()
    }
}

pub trait StreamPosition {
    /// Gets the current position in the stream.
    fn stream_position(&self) -> usize;
}

/// A passthrough `Write` implementation that keeps track of position.
///
/// This is used to keep track of the current position in the output stream, since we can't use
/// `Seek` on all output streams. Additionally this allows us to keep track of the position
/// when using functions like `copy` that may call write several times and potentially fail.
pub struct StreamPositionTracker<T: Write> {
    writer: T,
    pos: usize,
}

impl<T: Write> StreamPositionTracker<T> {
    pub fn new(writer: T) -> Self {
        Self { writer, pos: 0 }
    }
}

impl<T: Write> Write for StreamPositionTracker<T> {
    /// Passthrough to the underlying writer, but also updates the position.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.writer.write(buf)?;
        self.pos += written;
        Ok(written)
    }

    // Passthrough to the underlying writer.
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> std::io::Result<usize> {
        let written = self.writer.write_vectored(bufs)?;
        self.pos += written;
        Ok(written)
    }
}

impl<T: Write> StreamPosition for StreamPositionTracker<T> {
    fn stream_position(&self) -> usize {
        self.pos
    }
}

/// Stream wrapper that implements a subset of `Seek`.
///
/// This is needed because we read from streams that do not implement `Seek`,
/// such as `Stdin`. Instead of implementing ad-hoc skipping we'll implement
/// `Seek` such that it only allows seeking forward. Any seek from the end of
/// the stream, or that would go backwards, will result in an error.
pub struct ForwardOnlySeeker<T: Read> {
    reader: T,
    pos: u64,
}

impl<T: Read> ForwardOnlySeeker<T> {
    pub fn new(reader: T) -> Self {
        Self { reader, pos: 0 }
    }
}

impl<T: Read> Read for ForwardOnlySeeker<T> {
    /// Passthrough to the underlying reader, but also updates the position.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes_read = self.reader.read(buf)?;
        self.pos += bytes_read as u64;
        Ok(bytes_read)
    }
}

impl<T: Read> Seek for ForwardOnlySeeker<T> {
    /// Seeks forward in the stream, returns an error if seeking backwards.
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let seek_size = match pos {
            SeekFrom::Current(offset) => u64::try_from(offset).ok(),
            SeekFrom::Start(offset) => offset.checked_sub(self.pos),
            SeekFrom::End(_) => {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    "Cannot seek from end",
                ))
            }
        };

        match seek_size {
            Some(seek_size) => {
                copy(&mut self.by_ref().take(seek_size), &mut sink())?;
                Ok(self.pos)
            }
            None => Err(io::Error::new(
                ErrorKind::InvalidInput,
                "Only seeking forward allowed",
            )),
        }
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.pos)
    }
}

#[cfg(test)]
mod test {
    use rstest::rstest;

    use super::*;

    #[test]
    fn test_write_cursor() {
        let mut buf = Vec::new();
        let mut writer = StreamPositionTracker::new(&mut buf);

        writer.write_all(b"hello").unwrap();
        assert_eq!(writer.stream_position(), 5);
        writer.write_all(b" world").unwrap();
        assert_eq!(writer.stream_position(), 11);
        writer.write_all(b"!").unwrap();
        assert_eq!(writer.stream_position(), 12);

        assert_eq!(buf, b"hello world!");
    }

    #[test]
    fn test_write_vectored_cursor() {
        let mut buf = Vec::new();
        let mut writer = StreamPositionTracker::new(&mut buf);

        let write_vector = [
            IoSlice::new(b"hello"),
            IoSlice::new(b" world"),
            IoSlice::new(b"!"),
        ];
        let bytes_written = writer.write_vectored(&write_vector).unwrap();

        assert_eq!(bytes_written, 12);
        assert_eq!(writer.stream_position(), 12);
        assert_eq!(buf, b"hello world!");
    }

    #[test]
    fn test_forward_seeker_stream() {
        let mut input_stream = b"Hello world".as_ref();

        let mut reader = ForwardOnlySeeker::new(&mut input_stream);
        let mut out_buf = [0u8; 5];
        reader.read_exact(&mut out_buf).unwrap();
        assert_eq!(&out_buf, b"Hello");

        reader.seek(SeekFrom::Current(1)).unwrap();
        reader.read_exact(&mut out_buf).unwrap();
        assert_eq!(&out_buf, b"world");
    }

    #[rstest]
    #[case(SeekFrom::End(0))]
    #[case(SeekFrom::Current(-1))]
    #[case(SeekFrom::Start(0))]
    fn test_forward_seeker_seek_fail(#[case] seek: SeekFrom) {
        let mut reader = ForwardOnlySeeker::new(b"Hello world".as_ref());
        reader.seek(SeekFrom::Start(1)).unwrap();
        assert!(reader.seek(seek).is_err());
    }
}
