//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect logs into log files and save them as MAR entries.
//!
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use std::{fs, thread};
use std::{path::PathBuf, sync::Mutex};

use eyre::{eyre, Context, Result};
use flate2::Compression;
use log::{error, trace, warn};
use serde_json::Value;

use crate::logs::headroom::HeadroomCheck;
use crate::logs::log_file::{LogFile, LogFileControl, LogFileControlImpl};
use crate::logs::recovery::recover_old_logs;
use crate::util::rate_limiter::RateLimiter;
use crate::{config::Config, metrics::HeartbeatManager};
use crate::{config::LogToMetricRule, logs::completed_log::CompletedLog};

#[cfg(feature = "log-to-metrics")]
use super::log_to_metrics::LogToMetrics;

pub struct LogCollector<H: HeadroomCheck + Send + 'static> {
    inner: Arc<Mutex<Option<Inner<H>>>>,
}

impl<H: HeadroomCheck + Send + 'static> LogCollector<H> {
    /// Create a new log collector and open a new log file for writing.
    /// The on_log_completion callback will be called when a log file is completed.
    /// This callback must move (or delete) the log file!
    pub fn open<R: FnMut(CompletedLog) -> Result<()> + Send + 'static>(
        log_config: LogCollectorConfig,
        mut on_log_completion: R,
        headroom_limiter: H,
        #[cfg_attr(not(feature = "log-to-metrics"), allow(unused_variables))]
        heartbeat_manager: Arc<Mutex<HeartbeatManager>>,
    ) -> Result<Self> {
        fs::create_dir_all(&log_config.log_tmp_path).wrap_err_with(|| {
            format!(
                "Unable to create directory to store in-progress logs: {}",
                log_config.log_tmp_path.display()
            )
        })?;

        // Collect any leftover logfiles in the tmp folder
        let next_cid = recover_old_logs(&log_config.log_tmp_path, &mut on_log_completion)?;

        Ok(Self {
            inner: Arc::new(Mutex::new(Some(Inner {
                log_file_control: LogFileControlImpl::open(
                    log_config.log_tmp_path,
                    next_cid,
                    log_config.log_max_size,
                    log_config.log_max_duration,
                    log_config.log_compression_level,
                    on_log_completion,
                )?,
                rate_limiter: RateLimiter::new(log_config.max_lines_per_minute),
                headroom_limiter,
                #[cfg(feature = "log-to-metrics")]
                log_to_metrics: LogToMetrics::new(
                    log_config.log_to_metrics_rules,
                    heartbeat_manager,
                ),
            }))),
        })
    }

    /// Spawn a thread to read log records from receiver.
    pub fn spawn_collect_from<T: Iterator<Item = Value> + Send + 'static>(&self, source: T) {
        // Clone the atomic reference counting "pointer" (not the inner struct itself)
        let c = self.inner.clone();

        thread::spawn(move || {
            for line in source {
                match c.lock() {
                    Ok(mut inner_opt) => {
                        match &mut *inner_opt {
                            Some(inner) => {
                                if let Err(e) = inner.process_log_record(line) {
                                    warn!("Error writing log: {:?}", e);
                                }
                            }
                            // log_collector has shutdown. exit the thread cleanly.
                            None => return,
                        }
                    }
                    Err(e) => {
                        // This should never happen but we are unable to recover from this so bail out.
                        error!("Log collector got into an unrecoverable state: {}", e);
                        std::process::exit(-1);
                    }
                }
            }
            trace!("Log collection thread shutting down - Channel closed");
        });
    }

    /// Force the log_collector to close the current log and generate a MAR entry.
    pub fn flush_logs(&mut self) -> Result<()> {
        self.with_mut_inner(|inner| inner.log_file_control.rotate_unless_empty().map(|_| ()))
    }

    /// Rotate the logs if needed
    pub fn rotate_if_needed(&mut self) -> Result<bool> {
        self.with_mut_inner(|inner| inner.rotate_if_needed())
    }

    /// Try to get the inner log_collector or return an error
    fn with_mut_inner<T, F: FnOnce(&mut Inner<H>) -> Result<T>>(&mut self, fun: F) -> Result<T> {
        let mut inner_opt = self
            .inner
            .lock()
            // This should never happen so we choose to panic in this case.
            .expect("Fatal: log_collector mutex is poisoned.");

        match &mut *inner_opt {
            Some(inner) => fun(inner),
            None => Err(eyre!("Log collector has already shutdown.")),
        }
    }

    /// Close and dispose of the inner log collector.
    /// This is not public because it does not consume self (to be compatible with drop()).
    fn close_internal(&mut self) -> Result<()> {
        match self.inner.lock() {
            Ok(mut inner_opt) => {
                match (*inner_opt).take() {
                    Some(inner) => inner.log_file_control.close(),
                    None => {
                        // Already closed.
                        Ok(())
                    }
                }
            }
            Err(_) => {
                // Should never happen.
                panic!("Log collector is poisoned.")
            }
        }
    }
}

impl<H: HeadroomCheck + Send> Drop for LogCollector<H> {
    fn drop(&mut self) {
        if let Err(e) = self.close_internal() {
            warn!("Error closing log collector: {}", e);
        }
    }
}

/// The log collector keeps one Inner struct behind a Arc<Mutex<>> so it can be
/// shared by multiple threads.
struct Inner<H: HeadroomCheck> {
    // We use an Option<Value> here because we have no typed-guarantee that every
    // log message will include a `ts` key.
    rate_limiter: RateLimiter<Option<Value>>,
    log_file_control: LogFileControlImpl,
    headroom_limiter: H,
    #[cfg(feature = "log-to-metrics")]
    log_to_metrics: LogToMetrics,
}

impl<H: HeadroomCheck> Inner<H> {
    // Process one log record - To call this, the caller must have acquired a
    // mutex on the Inner object.
    // Be careful to not try to acquire other mutexes here to avoid a
    // dead-lock. Everything we need should be in Inner.
    fn process_log_record(&mut self, log: Value) -> Result<()> {
        let log_timestamp = log.get("ts");

        #[cfg(feature = "log-to-metrics")]
        if let Err(e) = self.log_to_metrics.process(&log) {
            warn!("Error processing log to metrics: {:?}", e);
        }

        if !self
            .headroom_limiter
            .check(log_timestamp, &mut self.log_file_control)?
        {
            return Ok(());
        }

        // Rotate before writing (in case log file is now too old)
        self.log_file_control.rotate_if_needed()?;

        let logfile = self.log_file_control.current_log();
        self.rate_limiter
            .run_within_limits(log_timestamp.cloned(), |rate_limited_calls| {
                // Print a message if some previous calls were rate limited.
                if let Some(limited) = rate_limited_calls {
                    logfile.write_log(
                        limited.latest_call,
                        format!("Memfaultd rate limited {} messages.", limited.count),
                    )?;
                }
                logfile.write_json_line(log)?;
                Ok(())
            })?;

        // Rotate after writing (in case log file is now too large)
        self.log_file_control.rotate_if_needed()?;
        Ok(())
    }

    fn rotate_if_needed(&mut self) -> Result<bool> {
        self.log_file_control.rotate_if_needed()
    }
}

pub struct LogCollectorConfig {
    /// Folder where to store logfiles while they are being written
    pub log_tmp_path: PathBuf,

    /// Files will be rotated when they reach this size (so they may be slightly larger)
    log_max_size: usize,

    /// MAR entry will be rotated when they get this old.
    log_max_duration: Duration,

    /// Compression level to use for compressing the logs.
    log_compression_level: Compression,

    /// Maximum number of lines written per second continuously
    max_lines_per_minute: NonZeroU32,

    /// Rules to convert logs to metrics
    #[cfg_attr(not(feature = "log-to-metrics"), allow(dead_code))]
    log_to_metrics_rules: Vec<LogToMetricRule>,
}

impl From<&Config> for LogCollectorConfig {
    fn from(config: &Config) -> Self {
        Self {
            log_tmp_path: config.logs_path(),
            log_max_size: config.config_file.logs.rotate_size,
            log_max_duration: config.config_file.logs.rotate_after,
            log_compression_level: config.config_file.logs.compression_level,
            max_lines_per_minute: config.config_file.logs.max_lines_per_minute,
            log_to_metrics_rules: config
                .config_file
                .logs
                .log_to_metrics
                .as_ref()
                .map(|c| c.rules.clone())
                .unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{channel, Receiver};
    use std::sync::Arc;
    use std::{fs::remove_file, sync::Mutex};
    use std::{io::Write, path::PathBuf, time::Duration};

    use crate::logs::headroom::HeadroomCheck;
    use crate::logs::log_file::{LogFile, LogFileControl};
    use crate::{logs::completed_log::CompletedLog, metrics::HeartbeatManager};
    use eyre::Context;
    use flate2::Compression;
    use rstest::{fixture, rstest};
    use serde_json::{json, Value};
    use tempfile::{tempdir, TempDir};
    use uuid::Uuid;

    use super::{LogCollector, LogCollectorConfig};

    #[rstest]
    fn write_logs_to_disk(mut fixture: LogFixture) {
        fixture.write_log(json!({"ts": 0, "MESSAGE": "xxx"}));
        assert_eq!(fixture.count_log_files(), 1);
        assert_eq!(fixture.on_log_completion_calls(), 0);
    }

    #[rstest]
    fn do_not_create_newfile_on_close(mut fixture: LogFixture) {
        fixture.write_log(json!({"ts": 0, "MESSAGE": "xxx"}));
        fixture.collector.close_internal().expect("error closing");
        // 0 because the fixture "on_log_completion" moves the file out
        assert_eq!(fixture.count_log_files(), 0);
        assert_eq!(fixture.on_log_completion_calls(), 1);
    }

    #[rstest]
    fn forced_rotation_with_nonempty_log(mut fixture: LogFixture) {
        fixture.write_log(json!({"ts": 0, "MESSAGE": "xxx"}));

        fixture.collector.flush_logs().unwrap();

        assert_eq!(fixture.count_log_files(), 1);
        assert_eq!(fixture.on_log_completion_calls(), 1);
    }

    #[rstest]
    fn delete_log_after_failed_on_completion_callback(mut fixture: LogFixture) {
        fixture
            .on_completion_should_fail
            .store(true, Ordering::Relaxed);
        fixture.write_log(json!({"ts": 0, "MESSAGE": "xxx"}));

        fixture.collector.flush_logs().unwrap();

        assert_eq!(fixture.on_log_completion_calls(), 1);

        // The old log should have been deleted, to avoid accumulating logs that fail to be moved.
        // Only the new log file remains:
        assert_eq!(fixture.count_log_files(), 1);
    }

    #[rstest]
    fn forced_rotation_with_empty_log(mut fixture: LogFixture) {
        fixture.collector.flush_logs().unwrap();

        assert_eq!(fixture.count_log_files(), 1);
        assert_eq!(fixture.on_log_completion_calls(), 0);
    }

    #[rstest]
    fn recover_old_logfiles() {
        let (tmp_logs, _old_file_path) = existing_tmplogs_with_log(&(Uuid::new_v4().to_string()));
        let fixture = collector_with_logs_dir(tmp_logs);

        // We should have generated a MAR entry for the pre-existing logfile.
        assert_eq!(fixture.on_log_completion_calls(), 1);
    }

    #[rstest]
    fn delete_files_that_are_not_uuids() {
        let (tmp_logs, old_file_path) = existing_tmplogs_with_log("testfile");
        let fixture = collector_with_logs_dir(tmp_logs);

        // And we should have removed the bogus file
        assert!(!old_file_path.exists());

        // We should NOT have generated a MAR entry for the pre-existing bogus file.
        assert_eq!(fixture.on_log_completion_calls(), 0);
    }

    fn existing_tmplogs_with_log(filename: &str) -> (TempDir, PathBuf) {
        let tmp_logs = tempdir().unwrap();
        let file_path = tmp_logs
            .path()
            .to_path_buf()
            .join(filename)
            .with_extension("log.zlib");

        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(b"some content in the log").unwrap();
        drop(file);
        (tmp_logs, file_path)
    }

    struct LogFixture {
        collector: LogCollector<StubHeadroomLimiter>,
        // TempDir needs to be after the collector, otherwise we fail to delete
        // the file in LogCollector::Drop because the tempdir is gone
        logs_dir: TempDir,
        on_log_completion_receiver: Receiver<(PathBuf, Uuid)>,
        on_completion_should_fail: Arc<AtomicBool>,
    }
    impl LogFixture {
        fn count_log_files(&self) -> usize {
            std::fs::read_dir(&self.logs_dir).unwrap().count()
        }

        fn write_log(&mut self, line: Value) {
            self.collector
                .with_mut_inner(|inner| inner.log_file_control.current_log().write_json_line(line))
                .unwrap();
        }

        fn on_log_completion_calls(&self) -> usize {
            self.on_log_completion_receiver.try_iter().count()
        }
    }

    #[fixture]
    fn fixture() -> LogFixture {
        collector_with_logs_dir(tempdir().unwrap())
    }

    struct StubHeadroomLimiter;

    impl HeadroomCheck for StubHeadroomLimiter {
        fn check<L: LogFile>(
            &mut self,
            _log_timestamp: Option<&Value>,
            _log_file_control: &mut impl LogFileControl<L>,
        ) -> eyre::Result<bool> {
            Ok(true)
        }
    }

    fn collector_with_logs_dir(logs_dir: TempDir) -> LogFixture {
        let config = LogCollectorConfig {
            log_tmp_path: logs_dir.path().to_owned(),
            log_max_size: 1024,
            log_max_duration: Duration::from_secs(3600),
            log_compression_level: Compression::default(),
            max_lines_per_minute: NonZeroU32::new(1_000).unwrap(),
            log_to_metrics_rules: vec![],
        };

        let (on_log_completion_sender, on_log_completion_receiver) = channel();

        let on_completion_should_fail = Arc::new(AtomicBool::new(false));

        let heartbeat_manager = Arc::new(Mutex::new(HeartbeatManager::new()));

        let collector = {
            let on_completion_should_fail = on_completion_should_fail.clone();
            let on_log_completion = move |CompletedLog { path, cid, .. }| {
                on_log_completion_sender.send((path.clone(), cid)).unwrap();
                if on_completion_should_fail.load(Ordering::Relaxed) {
                    // Don't move / unlink the log file. The LogCollector should clean up now.
                    Err(eyre::eyre!("on_log_completion failure!"))
                } else {
                    // Unlink the log file. The real implementation moves it into the MAR staging area.
                    remove_file(&path)
                        .with_context(|| format!("rm {path:?}"))
                        .unwrap();
                    Ok(())
                }
            };

            LogCollector::open(
                config,
                on_log_completion,
                StubHeadroomLimiter,
                heartbeat_manager,
            )
            .unwrap()
        };

        LogFixture {
            logs_dir,
            collector,
            on_log_completion_receiver,
            on_completion_should_fail,
        }
    }
}
