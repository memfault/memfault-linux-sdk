//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect logs into log files and save them as MAR entries.
//!
use std::io::Cursor;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use std::{fs, thread};
use std::{path::PathBuf, sync::Mutex};

use eyre::{eyre, Context, Result};
use flate2::Compression;
use log::{error, trace, warn};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tiny_http::{Header, Method, Request, Response, ResponseBox, StatusCode};

use crate::util::rate_limiter::RateLimiter;
use crate::{config::Config, metrics::MetricReportManager};
use crate::{config::LogToMetricRule, logs::completed_log::CompletedLog};
use crate::{config::StorageConfig, http_server::ConvenientHeader};
use crate::{
    http_server::HttpHandler,
    logs::log_file::{LogFile, LogFileControl, LogFileControlImpl},
};
use crate::{http_server::HttpHandlerResult, logs::recovery::recover_old_logs};
use crate::{logs::headroom::HeadroomCheck, util::circular_queue::CircularQueue};

pub const CRASH_LOGS_URL: &str = "/api/v1/crash-logs";

#[cfg(feature = "log-to-metrics")]
use super::log_to_metrics::LogToMetrics;

pub struct LogCollector<H: HeadroomCheck + Send + 'static> {
    inner: Arc<Mutex<Option<Inner<H>>>>,
}

impl<H: HeadroomCheck + Send + 'static> LogCollector<H> {
    /// This value is used to clamp the number of lines captured in a coredump.
    ///
    /// This is done to prevent the coredump from becoming too large. The value was chosen
    /// arbitrarily to be large enough to capture a reasonable amount of logs, but small enough
    /// to prevent the coredump from becoming too large. The current default is 100 lines.
    const MAX_IN_MEMORY_LINES: usize = 500;

    /// Create a new log collector and open a new log file for writing.
    /// The on_log_completion callback will be called when a log file is completed.
    /// This callback must move (or delete) the log file!
    pub fn open<R: FnMut(CompletedLog) -> Result<()> + Send + 'static>(
        log_config: LogCollectorConfig,
        mut on_log_completion: R,
        headroom_limiter: H,
        #[cfg_attr(not(feature = "log-to-metrics"), allow(unused_variables))]
        heartbeat_manager: Arc<Mutex<MetricReportManager>>,
    ) -> Result<Self> {
        fs::create_dir_all(&log_config.log_tmp_path).wrap_err_with(|| {
            format!(
                "Unable to create directory to store in-progress logs: {}",
                log_config.log_tmp_path.display()
            )
        })?;

        // Collect any leftover logfiles in the tmp folder
        let next_cid = recover_old_logs(&log_config.log_tmp_path, &mut on_log_completion)?;

        let in_memory_lines = if log_config.in_memory_lines > Self::MAX_IN_MEMORY_LINES {
            warn!(
                "Too many lines captured in coredump ({}), clamping to {}",
                log_config.in_memory_lines,
                Self::MAX_IN_MEMORY_LINES
            );
            Self::MAX_IN_MEMORY_LINES
        } else {
            log_config.in_memory_lines
        };
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
                log_queue: CircularQueue::new(in_memory_lines),
                storage_config: log_config.storage_config,
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

    /// Get a handler for the /api/v1/crash-logs endpoint
    pub fn crash_log_handler(&self) -> CrashLogHandler<H> {
        CrashLogHandler::new(self.inner.clone())
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
    log_queue: CircularQueue<Value>,
    storage_config: StorageConfig,
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
        self.log_queue.push(log.clone());

        // Return early and do not write a log message to file if not persisting
        if !self.should_persist() {
            return Ok(());
        }

        // Rotate before writing (in case log file is now too old)
        self.log_file_control.rotate_if_needed()?;

        let logfile = self.log_file_control.current_log()?;
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

    fn should_persist(&self) -> bool {
        matches!(self.storage_config, StorageConfig::Persist)
    }

    fn rotate_if_needed(&mut self) -> Result<bool> {
        self.log_file_control.rotate_if_needed()
    }

    pub fn get_log_queue(&mut self) -> Result<Vec<String>> {
        let logs = self.log_queue.iter().map(|v| v.to_string()).collect();

        Ok(logs)
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

    /// Maximum number of lines to keep in memory
    in_memory_lines: usize,

    /// Whether or not to persist log lines
    storage_config: StorageConfig,
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
            in_memory_lines: config.config_file.coredump.log_lines,
            storage_config: config.config_file.logs.storage,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
/// A list of crash logs.
///
/// This structure is passed to the client when they request the crash logs.
pub struct CrashLogs {
    pub logs: Vec<String>,
}

/// A handler for the /api/v1/crash-logs endpoint.
pub struct CrashLogHandler<H: HeadroomCheck + Send + 'static> {
    inner: Arc<Mutex<Option<Inner<H>>>>,
}

impl<H: HeadroomCheck + Send + 'static> CrashLogHandler<H> {
    fn new(inner: Arc<Mutex<Option<Inner<H>>>>) -> Self {
        Self { inner }
    }

    /// Handle a GET request to /api/v1/crash-logs
    ///
    /// Will take a snapshot of the current circular queue and return it as a JSON array.
    fn handle_get_crash_logs(&self) -> Result<ResponseBox> {
        let logs = self
            .inner
            .lock()
            .expect("Log collector mutex poisoned")
            .as_mut()
            .ok_or_else(|| eyre!("Log collector has already shutdown."))?
            .get_log_queue()?;
        let crash_logs = CrashLogs { logs };

        let serialized_logs = serde_json::to_string(&crash_logs)?;
        let logs_len = serialized_logs.as_bytes().len();
        Ok(Response::new(
            StatusCode(200),
            vec![Header::from_strings("Content-Type", "application/json")?],
            Cursor::new(serialized_logs),
            Some(logs_len),
            None,
        )
        .boxed())
    }
}

impl<H: HeadroomCheck + Send + 'static> HttpHandler for CrashLogHandler<H> {
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult {
        if request.url() == CRASH_LOGS_URL {
            match *request.method() {
                Method::Get => self.handle_get_crash_logs().into(),
                _ => HttpHandlerResult::Response(Response::empty(405).boxed()),
            }
        } else {
            HttpHandlerResult::NotHandled
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::min;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{channel, Receiver};
    use std::sync::Arc;
    use std::{fs::remove_file, sync::Mutex};
    use std::{io::Write, path::PathBuf, time::Duration};
    use std::{mem::replace, num::NonZeroU32};

    use crate::logs::log_file::{LogFile, LogFileControl};
    use crate::test_utils::setup_logger;
    use crate::{logs::completed_log::CompletedLog, metrics::MetricReportManager};
    use crate::{logs::headroom::HeadroomCheck, util::circular_queue::CircularQueue};
    use eyre::Context;
    use flate2::Compression;
    use rstest::{fixture, rstest};
    use serde_json::{json, Value};
    use tempfile::{tempdir, TempDir};
    use tiny_http::{Method, TestRequest};
    use uuid::Uuid;

    use super::*;

    const IN_MEMORY_LINES: usize = 100;

    #[rstest]
    fn write_logs_to_disk(mut fixture: LogFixture) {
        fixture.write_log(json!({"ts": 0, "MESSAGE": "xxx"}));
        assert_eq!(fixture.count_log_files(), 1);
        assert_eq!(fixture.on_log_completion_calls(), 0);
    }

    #[rstest]
    #[case(50)]
    #[case(100)]
    #[case(150)]
    fn circular_log_queue(#[case] mut log_count: usize, mut fixture: LogFixture) {
        for i in 0..log_count {
            fixture.write_log(json!({"ts": i, "MESSAGE": "xxx"}));
        }

        let log_queue = fixture.get_log_queue();

        // Assert that the last value in the queue has the correct timestamp
        let last_val = log_queue.back().unwrap();
        let ts = last_val.get("ts").unwrap().as_u64().unwrap();
        assert_eq!(ts, log_count as u64 - 1);

        // Clamp the log_count to the maximum size of the queue
        log_count = min(log_count, IN_MEMORY_LINES);
        assert_eq!(log_queue.len(), log_count);
    }

    #[rstest]
    fn clamp_coredump_log_count(fixture: LogFixture) {
        let config = LogCollectorConfig {
            log_tmp_path: fixture.logs_dir.path().to_owned(),
            log_max_size: 1024,
            log_max_duration: Duration::from_secs(3600),
            log_compression_level: Compression::default(),
            max_lines_per_minute: NonZeroU32::new(1_000).unwrap(),
            log_to_metrics_rules: vec![],
            in_memory_lines: 1000,
            storage_config: StorageConfig::Persist,
        };

        let mut collector = LogCollector::open(
            config,
            |CompletedLog { path, .. }| {
                remove_file(&path)
                    .with_context(|| format!("rm {path:?}"))
                    .unwrap();
                Ok(())
            },
            StubHeadroomLimiter,
            Arc::new(Mutex::new(MetricReportManager::new())),
        )
        .unwrap();

        let log_queue = collector
            .with_mut_inner(|inner| Ok(replace(&mut inner.log_queue, CircularQueue::new(1000))))
            .unwrap();

        // The log queue should be clamped to the maximum size
        assert_eq!(
            log_queue.capacity(),
            LogCollector::<StubHeadroomLimiter>::MAX_IN_MEMORY_LINES
        );
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
    #[case(StorageConfig::Persist, 25)]
    #[case(StorageConfig::Disabled, 0)]
    fn log_persistence(
        #[case] storage_config: StorageConfig,
        #[case] expected_size: usize,
        mut fixture: LogFixture,
        _setup_logger: (),
    ) {
        fixture.set_log_config(storage_config);

        fixture.write_log(json!({"ts": 0, "MESSAGE": "xxx"}));
        fixture.flush_log_writes().unwrap();

        assert_eq!(fixture.count_log_files(), 1);
        assert_eq!(fixture.read_log_len(), expected_size);
    }

    #[rstest]
    fn forced_rotation_with_nonempty_log(mut fixture: LogFixture) {
        fixture.write_log(json!({"ts": 0, "MESSAGE": "xxx"}));

        fixture.collector.flush_logs().unwrap();

        assert_eq!(fixture.count_log_files(), 0);
        assert_eq!(fixture.on_log_completion_calls(), 1);
    }

    #[rstest]
    fn delete_log_after_failed_on_completion_callback(mut fixture: LogFixture) {
        fixture
            .on_completion_should_fail
            .store(true, Ordering::Relaxed);
        fixture.write_log(test_line());

        fixture.collector.flush_logs().unwrap();

        assert_eq!(fixture.on_log_completion_calls(), 1);

        // The old log should have been deleted, to avoid accumulating logs that fail to be moved.
        // No new file will be created without a subsequent write
        assert_eq!(fixture.count_log_files(), 0);
    }

    #[rstest]
    fn forced_rotation_with_empty_log(mut fixture: LogFixture) {
        fixture.collector.flush_logs().unwrap();

        assert_eq!(fixture.count_log_files(), 0);
        assert_eq!(fixture.on_log_completion_calls(), 0);
    }

    #[rstest]
    fn forced_rotation_with_write_after_rotate(mut fixture: LogFixture) {
        fixture.write_log(test_line());
        fixture.collector.flush_logs().unwrap();

        fixture.write_log(test_line());
        assert_eq!(fixture.count_log_files(), 1);
        assert_eq!(fixture.on_log_completion_calls(), 1);
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

    #[rstest]
    fn http_handler_log_get(mut fixture: LogFixture) {
        let logs = vec![
            json!({"ts": 0, "MESSAGE": "xxx"}),
            json!({"ts": 1, "MESSAGE": "yyy"}),
            json!({"ts": 2, "MESSAGE": "zzz"}),
        ];
        let log_strings = logs.iter().map(|l| l.to_string()).collect::<Vec<_>>();

        for log in &logs {
            fixture.write_log(log.clone());
        }

        let inner = fixture.collector.inner.clone();
        let handler = CrashLogHandler::new(inner);

        let log_response = handler.handle_get_crash_logs().unwrap();
        let mut log_response_string = String::new();
        log_response
            .into_reader()
            .read_to_string(&mut log_response_string)
            .unwrap();

        let crash_logs: CrashLogs = serde_json::from_str(&log_response_string).unwrap();
        assert_eq!(crash_logs.logs, log_strings);
    }

    #[rstest]
    #[case(Method::Post)]
    #[case(Method::Put)]
    #[case(Method::Delete)]
    #[case(Method::Patch)]
    fn http_handler_unsupported_method(fixture: LogFixture, #[case] method: Method) {
        let inner = fixture.collector.inner.clone();
        let handler = CrashLogHandler::new(inner);

        let request = TestRequest::new()
            .with_path(CRASH_LOGS_URL)
            .with_method(method);
        let response = handler
            .handle_request(&mut request.into())
            .expect("Error handling request");
        assert_eq!(response.status_code().0, 405);
    }

    #[rstest]
    fn unhandled_url(fixture: LogFixture) {
        let inner = fixture.collector.inner.clone();
        let handler = CrashLogHandler::new(inner);

        let request = TestRequest::new().with_path("/api/v1/other");
        let response = handler.handle_request(&mut request.into());
        assert!(matches!(response, HttpHandlerResult::NotHandled));
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
                .with_mut_inner(|inner| inner.process_log_record(line))
                .unwrap();
        }

        fn read_log_len(&mut self) -> usize {
            self.collector
                .with_mut_inner(|inner| {
                    let log = inner.log_file_control.current_log()?;
                    Ok(log.bytes_written())
                })
                .unwrap()
        }

        fn flush_log_writes(&mut self) -> Result<()> {
            self.collector
                .with_mut_inner(|inner| inner.log_file_control.current_log()?.flush())
        }

        fn on_log_completion_calls(&self) -> usize {
            self.on_log_completion_receiver.try_iter().count()
        }

        fn get_log_queue(&mut self) -> CircularQueue<Value> {
            self.collector
                .with_mut_inner(|inner| Ok(replace(&mut inner.log_queue, CircularQueue::new(100))))
                .unwrap()
        }

        fn set_log_config(&mut self, storage_config: StorageConfig) {
            self.collector
                .with_mut_inner(|inner| {
                    inner.storage_config = storage_config;
                    Ok(())
                })
                .unwrap()
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
            in_memory_lines: IN_MEMORY_LINES,
            storage_config: StorageConfig::Persist,
        };

        let (on_log_completion_sender, on_log_completion_receiver) = channel();

        let on_completion_should_fail = Arc::new(AtomicBool::new(false));

        let heartbeat_manager = Arc::new(Mutex::new(MetricReportManager::new()));

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

    fn test_line() -> Value {
        json!({"ts": 0, "MESSAGE": "xxx"})
    }
}
