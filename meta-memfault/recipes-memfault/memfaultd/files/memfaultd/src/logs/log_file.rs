//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Contains LogFile and LogFileControl traits and their real implementations.
//!
use crate::logs::completed_log::CompletedLog;
use crate::mar::CompressionAlgorithm;
use chrono::{DateTime, Utc};
use eyre::{eyre, Result, WrapErr};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use log::{trace, warn};
use std::fs::{remove_file, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

use std::collections::HashMap;

use super::{
    log_entry::{LogData, LogEntry},
    recovery::recover_old_logs,
};

pub trait LogFile {
    fn write_json_line(&mut self, json: LogEntry) -> Result<()>;
    fn write_log<S: AsRef<str>>(
        &mut self,
        ts: DateTime<Utc>,
        priority: &str,
        msg: S,
    ) -> Result<()> {
        let data = LogData {
            message: msg.as_ref().to_string(),
            pid: None,
            systemd_unit: None,
            priority: Some(priority.to_string()),
            original_priority: None,
            extra_fields: HashMap::new(),
        };

        let log_entry = LogEntry { ts, data };
        self.write_json_line(log_entry)
    }
    fn flush(&mut self) -> Result<()>;
}

/// In memory representation of one logfile while it is being written to.
pub struct LogFileImpl {
    cid: Uuid,
    path: PathBuf,
    writer: BufWriter<ZlibEncoder<File>>,
    bytes_written: usize,
    since: Instant,
}

impl LogFileImpl {
    fn open(log_tmp_path: &Path, cid: Uuid, compression_level: Compression) -> Result<LogFileImpl> {
        let filename = cid.to_string() + ".log.zlib";
        let path = log_tmp_path.join(filename);
        let file = File::create(&path)?;
        let writer = BufWriter::new(ZlibEncoder::new(file, compression_level));

        trace!("Now writing logs to: {}", path.display());
        Ok(LogFileImpl {
            cid,
            path,
            writer,
            bytes_written: 0,
            since: Instant::now(),
        })
    }

    #[cfg(test)]
    pub fn bytes_written(&self) -> usize {
        self.bytes_written
    }
}

impl LogFile for LogFileImpl {
    fn write_json_line(&mut self, json: LogEntry) -> Result<()> {
        let bytes = serde_json::to_vec(&json)?;
        let mut written = self.writer.write(&bytes)?;
        written += self.writer.write("\n".as_bytes())?;
        self.bytes_written += written;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.writer.flush().wrap_err("Flush error")
    }
}

pub trait LogFileControl<L: LogFile> {
    fn rotate_if_needed(&mut self) -> Result<bool>;
    fn rotate_unless_empty(&mut self) -> Result<()>;
    fn current_log(&mut self) -> Result<&mut L>;
    fn close(self) -> Result<()>;
}

/// Controls the creation and rotation of logfiles.
pub struct LogFileControlImpl {
    current_log: Option<LogFileImpl>,
    tmp_path: PathBuf,
    max_size: usize,
    max_duration: Duration,
    compression_level: Compression,
    on_log_completion: Box<(dyn FnMut(CompletedLog) -> Result<()> + Send)>,
    next_cid: Option<Uuid>,
}

impl LogFileControlImpl {
    pub fn open<R: FnMut(CompletedLog) -> Result<()> + Send + 'static>(
        tmp_path: PathBuf,
        max_size: usize,
        max_duration: Duration,
        compression_level: Compression,
        on_log_completion: R,
    ) -> Result<Self> {
        Ok(LogFileControlImpl {
            current_log: None,
            tmp_path,
            max_size,
            max_duration,
            compression_level,
            on_log_completion: Box::new(on_log_completion),
            next_cid: None,
        })
    }

    /// Starts recovey of logs.
    ///
    /// Checks if the next CID is populated. If is we've, already recovered
    /// any logs and this is effecitly a no-op. The return bool indicates
    /// whether or not recovery was run.
    pub fn recover_logs(&mut self) -> Result<bool> {
        if self.next_cid.is_none() {
            self.next_cid()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Close current logfile, create a MAR entry and starts a new one.
    fn rotate_log(&mut self) -> Result<()> {
        let current_log = self.current_log.take();

        if let Some(current_log) = current_log {
            *self.next_cid()? = Uuid::new_v4();

            let next_cid = *self.next_cid()?;
            Self::dispatch_on_log_completion(&mut self.on_log_completion, current_log, next_cid);
        }

        Ok(())
    }

    fn next_cid(&mut self) -> Result<&mut Uuid> {
        if self.next_cid.is_none() {
            self.next_cid = Some(recover_old_logs(
                &self.tmp_path,
                &mut self.on_log_completion,
            )?);
        }

        // This should neve be None as we check the cid above
        self.next_cid
            .as_mut()
            .ok_or_else(|| eyre!("next CID not populated"))
    }

    fn dispatch_on_log_completion(
        on_log_completion: &mut Box<(dyn FnMut(CompletedLog) -> Result<()> + Send)>,
        mut log: LogFileImpl,
        next_cid: Uuid,
    ) {
        // Drop the old log, finishing the compression, closing the buffered writer and the file.
        log.writer.flush().unwrap_or_else(|e| {
            warn!("Failed to flush logs: {}", e);
        });

        let LogFileImpl { path, cid, .. } = log;

        // The callback is responsible for moving the file to its final location (or deleting it):
        (on_log_completion)(CompletedLog {
            path: path.clone(),
            cid,
            next_cid,
            compression: CompressionAlgorithm::Zlib,
        })
        .unwrap_or_else(|e| {
            warn!(
                "Dropping log due to failed on_log_completion callback: {}",
                e
            );
            remove_file(&path).unwrap_or_else(|e| {
                warn!("Failed to remove log file: {}", e);
            });
        });
    }
}

impl LogFileControl<LogFileImpl> for LogFileControlImpl {
    fn rotate_if_needed(&mut self) -> Result<bool> {
        if let Some(current_log) = &mut self.current_log {
            if current_log.bytes_written >= self.max_size
                || current_log.since.elapsed() > self.max_duration
            {
                self.rotate_log()?;
                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }

    fn rotate_unless_empty(&mut self) -> Result<()> {
        if let Some(current_log) = &self.current_log {
            if current_log.bytes_written > 0 {
                self.rotate_log()?;
            }
        }
        Ok(())
    }

    fn current_log(&mut self) -> Result<&mut LogFileImpl> {
        if self.current_log.is_none() {
            let next_cid = *self.next_cid()?;
            self.current_log = Some(
                LogFileImpl::open(&self.tmp_path, next_cid, self.compression_level)
                    .map_err(|e| eyre!("Failed to open log file: {e}"))?,
            );
        }

        // NOTE: The error case should not be possible here as it is always set above.
        // still this is better than panicking.
        self.current_log
            .as_mut()
            .ok_or_else(|| eyre!("No current log"))
    }

    fn close(mut self) -> Result<()> {
        if let Some(current_log) = self.current_log {
            if current_log.bytes_written > 0 {
                Self::dispatch_on_log_completion(
                    &mut self.on_log_completion,
                    current_log,
                    Uuid::new_v4(),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use crate::logs::log_entry::LogValue;

    use super::*;
    use flate2::bufread::ZlibDecoder;
    use rand::distributions::{Alphanumeric, DistString};
    use rstest::rstest;
    use serde_json::Value;
    use tempfile::tempdir;

    // We saw this bug when we tried switching to the rust-based backend for flate2 (miniz-oxide)
    // With miniz 0.7.1 and flate2 1.0.28, this test does not pass.
    #[rstest]
    fn test_write_without_corruption() {
        let tmp = tempdir().expect("tmpdir");

        // Generate a logfile with lots of bogus data
        let mut log = LogFileImpl::open(tmp.path(), Uuid::new_v4(), Compression::fast())
            .expect("open log error");
        let mut count_lines = 0;
        while log.bytes_written < 1024 * 1024 {
            let message = format!(
                "bogus {} bogum {} bodoum",
                Alphanumeric.sample_string(&mut rand::thread_rng(), 16),
                Alphanumeric.sample_string(&mut rand::thread_rng(), 20),
            );
            let log_entry = LogEntry {
                ts: "2024-09-11T12:34:56Z".parse().unwrap(),
                data: LogData {
                    message,
                    pid: None,
                    systemd_unit: None,
                    priority: None,
                    original_priority: None,
                    extra_fields: [("unit".to_string(), LogValue::String("systemd".to_string()))]
                        .into_iter()
                        .collect(),
                },
            };
            log.write_json_line(log_entry)
                .expect("error writing json line");
            count_lines += 1;
        }

        let logfile = log.path.clone();
        drop(log);

        // Decompress without error
        let bytes = std::fs::read(&logfile).expect("Unable to read {filename}");
        let mut z = ZlibDecoder::new(&bytes[..]);
        let mut loglines = String::new();
        z.read_to_string(&mut loglines).expect("read error");

        // Check we have all the lines
        assert_eq!(count_lines, loglines.lines().count());

        // Check all lines are valid json
        let mut count_invalid_lines = 0;
        for line in loglines.lines() {
            if serde_json::from_str::<Value>(line).is_err() {
                count_invalid_lines += 1;
            }
        }
        assert_eq!(count_invalid_lines, 0);
    }
}
