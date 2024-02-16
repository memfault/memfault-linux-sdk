//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Memfault Test Utils
//!
//! A collection of useful structs and functions for unit and integration testing.
//!

use std::cmp::min;
use std::path::Path;
use std::str::FromStr;
use std::{
    fs::File,
    io::{ErrorKind, Seek, Write},
};

use chrono::Duration;
use rstest::fixture;

mod test_instant;
pub use test_instant::*;

mod test_connection_checker;
pub use test_connection_checker::*;

use crate::metrics::{KeyedMetricReading, MetricReading, MetricStringKey, MetricTimestamp};

/// A file that will trigger write errors when it reaches a certain size.
/// Note that we currently enforce the limit on the total number of bytes
/// written, regardless of where they were written. We do implement Seek but do
/// not try to keep track of the file size.
pub struct SizeLimitedFile {
    file: File,
    limit: usize,
    written: usize,
}

impl SizeLimitedFile {
    /// Create a new SizeLimitedFile which will write to file until limit is
    /// reached.
    #[allow(dead_code)]
    pub fn new(file: File, limit: usize) -> Self {
        Self {
            file,
            limit,
            written: 0,
        }
    }
}

impl Write for SizeLimitedFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let bytes_to_write = buf.len().min(self.limit - self.written);

        if bytes_to_write == 0 {
            Err(std::io::Error::new(
                ErrorKind::WriteZero,
                "File size limit reached",
            ))
        } else {
            self.written += bytes_to_write;
            self.file.write(&buf[..bytes_to_write])
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl Seek for SizeLimitedFile {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

pub fn create_file_with_size(path: &Path, size: u64) -> std::io::Result<()> {
    let mut file = File::create(path)?;
    let buffer = vec![0; min(4096, size as usize)];
    let mut remaining = size;
    while remaining > 0 {
        let bytes_to_write = min(remaining, buffer.len() as u64);
        file.write_all(&buffer[..bytes_to_write as usize])?;
        remaining -= bytes_to_write;
    }
    Ok(())
}

pub fn create_file_with_contents(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(contents)?;
    Ok(())
}

#[fixture]
/// Simple fixture to add to a test when you want the logger to work.
pub fn setup_logger() {
    let _ = stderrlog::new().module("memfaultd").verbosity(10).init();
}

/// Macro to set the snapshot suffix for a test.
///
/// This is a workaround suggested by the insta docs to scope the snapshot suffix
/// to a test when using rstest cases. See docs
/// [here](https://insta.rs/docs/patterns/).
macro_rules! set_snapshot_suffix {
    ($($expr:expr),*) => {
        let mut settings = insta::Settings::clone_current();
        settings.set_snapshot_suffix(format!($($expr,)*));
        let _guard = settings.bind_to_scope();
    }
}

#[cfg(test)]
/// Constructs an interator of Gauge metric readings for tests
/// to easily generate mock data
pub fn in_gauges(
    metrics: Vec<(&'static str, i64, f64)>,
) -> impl Iterator<Item = KeyedMetricReading> {
    metrics
        .into_iter()
        .enumerate()
        .map(|(i, (name, interval, value))| KeyedMetricReading {
            name: MetricStringKey::from_str(name).unwrap(),
            value: MetricReading::Gauge {
                value,
                interval: Duration::milliseconds(interval),
                timestamp: MetricTimestamp::from_str("2021-01-01T00:00:00Z").unwrap()
                    + chrono::Duration::seconds(i as i64),
            },
        })
}

pub(crate) use set_snapshot_suffix;
