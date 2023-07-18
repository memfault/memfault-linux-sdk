//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Contains LogFile and LogFileControl traits and their real implementations.
//!
use crate::logs::completed_log::CompletedLog;
use crate::mar::CompressionAlgorithm;
use eyre::{Result, WrapErr};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use log::{trace, warn};
use serde_json::{json, Value};
use std::fs::{remove_file, File};
use std::io::{BufWriter, Write};
use std::mem::replace;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub trait LogFile {
    fn write_json_line(&mut self, json: Value) -> Result<()>;
    fn write_log<S: AsRef<str>>(&mut self, ts: Option<Value>, msg: S) -> Result<()> {
        self.write_json_line(json!({
            "ts": ts,
            "data": { "MESSAGE": msg.as_ref() }
        }))
    }
    fn flush(&mut self) -> Result<()>;
}

/// In memory representation of one logfile while it is being written to.
pub struct LogFileImpl {
    cid: Uuid,
    path: PathBuf,
    writer: ZlibEncoder<BufWriter<File>>,
    bytes_written: usize,
    since: Instant,
}

impl LogFileImpl {
    fn open(log_tmp_path: &Path, cid: Uuid, compression_level: Compression) -> Result<LogFileImpl> {
        let filename = cid.to_string() + ".log.zlib";
        let path = log_tmp_path.join(filename);
        let file = File::create(&path)?;
        let writer = ZlibEncoder::new(BufWriter::new(file), compression_level);

        trace!("Now writing logs to: {}", path.display());
        Ok(LogFileImpl {
            cid,
            path,
            writer,
            bytes_written: 0,
            since: Instant::now(),
        })
    }
}

impl LogFile for LogFileImpl {
    fn write_json_line(&mut self, json: Value) -> Result<()> {
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
    fn rotate_unless_empty(&mut self) -> Result<bool>;
    fn current_log(&mut self) -> &mut L;
}

/// Controls the creation and rotation of logfiles.
pub struct LogFileControlImpl {
    current_log: LogFileImpl,
    tmp_path: PathBuf,
    max_size: usize,
    max_duration: Duration,
    compression_level: Compression,
    on_log_completion: Box<(dyn FnMut(CompletedLog) -> Result<()> + Send)>,
}

impl LogFileControlImpl {
    pub fn open<R: FnMut(CompletedLog) -> Result<()> + Send + 'static>(
        tmp_path: PathBuf,
        next_cid: Uuid,
        max_size: usize,
        max_duration: Duration,
        compression_level: Compression,
        on_log_completion: R,
    ) -> Result<Self> {
        Ok(LogFileControlImpl {
            current_log: LogFileImpl::open(&tmp_path, next_cid, compression_level)?,
            tmp_path,
            max_size,
            max_duration,
            compression_level,
            on_log_completion: Box::new(on_log_completion),
        })
    }

    fn dispatch_on_log_completion(&mut self, mut log: LogFileImpl, next_cid: Uuid) {
        // Drop the old log, finishing the compression, closing the buffered writer and the file.
        log.writer.flush().unwrap_or_else(|e| {
            warn!("Failed to flush logs: {}", e);
        });

        let LogFileImpl { path, cid, .. } = log;

        // The callback is responsible for moving the file to its final location (or deleting it):
        (self.on_log_completion)(CompletedLog {
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

    /// Close current logfile, create a MAR entry and starts a new one.
    fn rotate_log(&mut self) -> Result<()> {
        // Start a new log and make it the current one. We are now writing there.
        let closed_log = replace(
            &mut self.current_log,
            LogFileImpl::open(&self.tmp_path, Uuid::new_v4(), self.compression_level)?,
        );

        self.dispatch_on_log_completion(closed_log, self.current_log.cid);

        Ok(())
    }

    fn rotate_if(&mut self, condition: bool) -> Result<bool> {
        if condition {
            self.rotate_log().wrap_err("Error rotating log")?;
            return Ok(true);
        }
        Ok(false)
    }
}

impl LogFileControl<LogFileImpl> for LogFileControlImpl {
    fn rotate_if_needed(&mut self) -> Result<bool> {
        self.rotate_if(
            self.current_log.bytes_written >= self.max_size
                || self.current_log.since.elapsed() > self.max_duration,
        )
    }
    fn rotate_unless_empty(&mut self) -> Result<bool> {
        self.rotate_if(self.current_log.bytes_written > 0)
    }
    fn current_log(&mut self) -> &mut LogFileImpl {
        &mut self.current_log
    }
}
