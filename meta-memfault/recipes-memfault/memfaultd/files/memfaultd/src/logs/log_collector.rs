//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect logs into log files and save them as MAR entries.
//!
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::sleep;
use std::time::{Duration, Instant};
use std::{fs, sync::atomic::AtomicUsize};
use std::{io::Cursor, sync::Arc};
use std::{num::NonZeroU32, sync::atomic::Ordering};

use chrono::{DateTime, Utc};
use eyre::{eyre, Context, Result};
use flate2::Compression;
use log::warn;
use serde::{Deserialize, Serialize};
use ssf::{Handler, MsgMailbox, Service};
use tiny_http::{Header, Method, Request, Response, ResponseBox, StatusCode};

use crate::config::{Config, LogFilterConfig, Resolution};
use crate::http_server::HttpHandlerResult;
use crate::{config::LogToMetricRule, logs::completed_log::CompletedLog};
use crate::{config::StorageConfig, http_server::ConvenientHeader};
use crate::{
    http_server::{parse_query_params, HttpHandler},
    logs::log_file::{LogFile, LogFileControl, LogFileControlImpl},
};
use crate::{logs::headroom::HeadroomCheck, util::circular_queue::CircularQueue};
use crate::{metrics::MetricsMBox, util::rate_limiter::RateLimiter};

pub const CRASH_LOGS_URL: &str = "/api/v1/crash-logs";
pub const CRASH_LOGS_CRASH_TS_PARAM: &str = "time_of_crash";

use super::log_filter::LogFilter;
use super::log_level_mapper::LogLevelMapper;
use super::log_to_metrics::LogToMetrics;
use super::messages::GetLatestLogTimestampMsg;

use crate::config::LevelMappingConfig;

use super::{
    log_entry::LogEntry,
    messages::{FlushLogsMsg, GetQueuedLogsMsg, LogEntryMsg, RecoverLogsMsg, RotateIfNeededMsg},
};

pub struct LogCollector<H: HeadroomCheck + Send + 'static> {
    inner: Option<Inner<H>>,
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
        logging_resolution: Resolution,
        on_log_completion: R,
        headroom_limiter: H,
        metrics_mbox: MetricsMBox,
    ) -> Result<(Self, Sender<Resolution>)> {
        fs::create_dir_all(&log_config.log_tmp_path).wrap_err_with(|| {
            format!(
                "Unable to create directory to store in-progress logs: {}",
                log_config.log_tmp_path.display()
            )
        })?;

        // Collect any leftover logfiles in the tmp folder
        let level_mapper = if log_config.level_mapping_config.enable {
            Some(LogLevelMapper::try_from(&log_config.level_mapping_config)?)
        } else {
            None
        };
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

        let (logging_resolution_sender, logging_resolution_receiver) = channel::<Resolution>();

        Ok((
            Self {
                inner: Some(Inner {
                    log_file_control: LogFileControlImpl::open(
                        log_config.log_tmp_path,
                        log_config.log_max_size,
                        log_config.log_max_duration,
                        log_config.log_compression_level,
                        on_log_completion,
                    )?,
                    rate_limiter: RateLimiter::new(log_config.max_lines_per_minute),
                    headroom_limiter,
                    #[allow(dead_code)]
                    log_to_metrics: LogToMetrics::new(
                        log_config.log_to_metrics_rules.clone(),
                        metrics_mbox.clone(),
                    ),
                    log_filter: LogFilter::new(
                        log_config.log_filter_config.rules,
                        log_config.log_to_metrics_rules,
                        log_config.log_filter_config.default_action,
                        metrics_mbox,
                    ),
                    log_queue: CircularQueue::new(in_memory_lines),
                    storage_config: log_config.storage_config,
                    level_mapper,
                    logging_resolution,
                    logging_resolution_receiver,
                }),
            },
            logging_resolution_sender,
        ))
    }

    /// Try to get the inner log_collector or return an error
    fn with_mut_inner<T, F: FnOnce(&mut Inner<H>) -> Result<T>>(&mut self, fun: F) -> Result<T> {
        let mut inner_opt = &mut self.inner;

        match &mut inner_opt {
            Some(inner) => fun(inner),
            None => Err(eyre!("Log collector has already shutdown.")),
        }
    }

    /// Close and dispose of the inner log collector.
    /// This is not public because it does not consume self (to be compatible with drop()).
    fn close_internal(&mut self) -> Result<()> {
        match self.inner.take() {
            Some(inner) => inner.log_file_control.close(),
            None => {
                // Already closed.
                Ok(())
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

impl<H: HeadroomCheck + Send> Service for LogCollector<H> {
    fn name(&self) -> &str {
        "LogCollector"
    }
}

impl<H: HeadroomCheck + Send + 'static> Handler<FlushLogsMsg> for LogCollector<H> {
    fn deliver(&mut self, _m: FlushLogsMsg) -> <FlushLogsMsg as ssf::Message>::Reply {
        self.with_mut_inner(|inner| inner.log_file_control.rotate_unless_empty().map(|_| ()))
    }
}

impl<H: HeadroomCheck + Send + 'static> Handler<GetQueuedLogsMsg> for LogCollector<H> {
    fn deliver(&mut self, _m: GetQueuedLogsMsg) -> <GetQueuedLogsMsg as ssf::Message>::Reply {
        let logs = self.with_mut_inner(|inner| inner.get_log_queue())?;

        Ok(logs)
    }
}

impl<H: HeadroomCheck + Send + 'static> Handler<GetLatestLogTimestampMsg> for LogCollector<H> {
    fn deliver(
        &mut self,
        _m: GetLatestLogTimestampMsg,
    ) -> <GetLatestLogTimestampMsg as ssf::Message>::Reply {
        let log_ts = self.with_mut_inner(|inner| {
            inner
                .get_latest_log_timestamp()
                .ok_or(eyre!("Couldn't get latest log timestamp"))
        })?;

        Ok(log_ts)
    }
}

impl<H: HeadroomCheck + Send + 'static> Handler<RotateIfNeededMsg> for LogCollector<H> {
    fn deliver(&mut self, _m: RotateIfNeededMsg) -> <RotateIfNeededMsg as ssf::Message>::Reply {
        self.with_mut_inner(|inner| inner.rotate_if_needed())
    }
}

impl<H: HeadroomCheck + Send + 'static> Handler<LogEntryMsg> for LogCollector<H> {
    fn deliver(&mut self, m: LogEntryMsg) -> <LogEntryMsg as ssf::Message>::Reply {
        if m.dropped_msg_count > 0 {
            warn!("Dropped {} log messages", m.dropped_msg_count);
        }
        self.with_mut_inner(|inner| inner.process_log_record(m.entry))
    }
}

impl<H: HeadroomCheck + Send + 'static> Handler<RecoverLogsMsg> for LogCollector<H> {
    fn deliver(&mut self, _m: RecoverLogsMsg) -> <RecoverLogsMsg as ssf::Message>::Reply {
        self.with_mut_inner(|inner| inner.log_file_control.recover_logs())
    }
}

/// The log collector keeps one Inner struct behind a Arc<Mutex<>> so it can be
/// shared by multiple threads.
struct Inner<H: HeadroomCheck> {
    rate_limiter: RateLimiter<DateTime<Utc>>,
    log_file_control: LogFileControlImpl,
    headroom_limiter: H,
    #[allow(dead_code)]
    log_to_metrics: LogToMetrics,
    log_filter: LogFilter,
    log_queue: CircularQueue<LogEntry>,
    storage_config: StorageConfig,
    level_mapper: Option<LogLevelMapper>,
    logging_resolution: Resolution,
    logging_resolution_receiver: Receiver<Resolution>,
}

impl<H: HeadroomCheck> Inner<H> {
    // Process one log record - To call this, the caller must have acquired a
    // mutex on the Inner object.
    // Be careful to not try to acquire other mutexes here to avoid a
    // dead-lock. Everything we need should be in Inner.
    fn process_log_record(&mut self, mut log: LogEntry) -> Result<()> {
        if let Some(level_mapper) = &self.level_mapper.as_mut() {
            level_mapper.map_log(&mut log)?;
        }

        if let Some(log) = self.log_filter.apply_rules(log) {
            if !self
                .headroom_limiter
                .check(&log.ts, &mut self.log_file_control)?
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
                .run_within_limits(log.ts, |rate_limited_calls| {
                    // Print a message if some previous calls were rate limited.
                    if let Some(limited) = rate_limited_calls {
                        logfile.write_log(
                            limited.latest_call,
                            "WARN",
                            format!("Memfaultd rate limited {} messages.", limited.count),
                        )?;
                    }
                    logfile.write_json_line(log)?;
                    Ok(())
                })?;

            // Rotate after writing (in case log file is now too large)
            self.log_file_control.rotate_if_needed()?;
        };
        Ok(())
    }

    fn should_persist(&mut self) -> bool {
        // Check if there is an updated Resolution for
        // Logging
        while let Ok(resolution) = self.logging_resolution_receiver.try_recv() {
            self.logging_resolution = resolution;
        }

        matches!(self.storage_config, StorageConfig::Persist)
            || matches!(self.logging_resolution, Resolution::Normal)
    }

    pub fn get_log_queue(&mut self) -> Result<Vec<String>> {
        let logs = self
            .log_queue
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<String>, _>>()?;

        Ok(logs)
    }

    fn get_latest_log_timestamp(&self) -> Option<DateTime<Utc>> {
        self.log_queue.back().map(|log_entry| log_entry.ts)
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
    log_to_metrics_rules: Vec<LogToMetricRule>,

    /// Maximum number of lines to keep in memory
    in_memory_lines: usize,

    /// Whether or not to persist log lines
    storage_config: StorageConfig,

    level_mapping_config: LevelMappingConfig,

    log_filter_config: LogFilterConfig,
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
            level_mapping_config: config.config_file.logs.level_mapping.clone(),
            log_filter_config: config
                .config_file
                .logs
                .filtering
                .as_ref()
                .cloned()
                .unwrap_or_default(),
        }
    }
}

#[derive(Clone)]
pub struct LogEntrySender {
    sender: MsgMailbox<LogEntryMsg>,
    dropped_msg_count: Arc<AtomicUsize>,
}

impl LogEntrySender {
    pub fn new(sender: MsgMailbox<LogEntryMsg>) -> Self {
        Self {
            sender,
            dropped_msg_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn send_entry(&self, entry: LogEntry) -> Result<()> {
        let log_entry_msg = LogEntryMsg::new(entry, self.dropped_msg_count.load(Ordering::Relaxed));

        match self.sender.send_and_forget(log_entry_msg) {
            Ok(_) => self.dropped_msg_count.store(0, Ordering::Relaxed),
            Err(e) => match e {
                ssf::MailboxError::SendChannelClosed => {
                    return Err(eyre!("Journald channel dropped: {}", e));
                }
                ssf::MailboxError::NoResponse => {
                    return Err(eyre!("Unexpected service response"));
                }
                ssf::MailboxError::SendChannelFull => {
                    self.dropped_msg_count.fetch_add(1, Ordering::Relaxed);
                }
            },
        }

        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
/// A list of crash logs.
///
/// This structure is passed to the client when they request the crash logs.
pub struct CrashLogs {
    pub logs: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LatestLogTimestamp {
    pub ts: DateTime<Utc>,
}

/// A handler for the /api/v1/crash-logs endpoint.
pub struct CrashLogHandler {
    get_queued_logs_mbox: MsgMailbox<GetQueuedLogsMsg>,
    get_latest_log_ts_mbox: MsgMailbox<GetLatestLogTimestampMsg>,
}

impl CrashLogHandler {
    /// Timeout for delay on waiting for log_collector to "catch up" to the
    /// time of the crash before returning logs in the /api/v1/crash-logs
    /// endpoint
    pub const CRASH_LOGS_DELAY_TIMEOUT: Duration = Duration::from_millis(250);

    pub fn new(
        get_queued_logs_mbox: MsgMailbox<GetQueuedLogsMsg>,
        get_latest_log_ts_mbox: MsgMailbox<GetLatestLogTimestampMsg>,
    ) -> Self {
        Self {
            get_queued_logs_mbox,
            get_latest_log_ts_mbox,
        }
    }

    /// Handle a GET request to /api/v1/crash-logs
    ///
    /// Will take a snapshot of the current circular queue and return it as a JSON array.
    fn handle_get_crash_logs(
        &self,
        time_of_crash: DateTime<Utc>,
        crash_logs_delay_timeout: Duration,
    ) -> Result<ResponseBox> {
        let mut latest_log_timestamp = self
            .get_latest_log_ts_mbox
            .send_and_wait_for_reply(GetLatestLogTimestampMsg)?;
        let crash_logs_delay_start = Instant::now();

        // Try to wait for the log queue in log_collector to "catch up" to the time
        // of the crash to ensure all logs leading up to crash are captured
        while latest_log_timestamp.is_ok_and(|log_ts| log_ts < time_of_crash)
            && crash_logs_delay_start.elapsed() < crash_logs_delay_timeout
        {
            latest_log_timestamp = self
                .get_latest_log_ts_mbox
                .send_and_wait_for_reply(GetLatestLogTimestampMsg)?;

            // Sleep to avoid busy-waiting
            sleep(Duration::from_millis(50));
        }

        let logs = self
            .get_queued_logs_mbox
            .send_and_wait_for_reply(GetQueuedLogsMsg)??;

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
impl HttpHandler for CrashLogHandler {
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult {
        let url = request.url();
        let base_url = url.split('?').next().unwrap_or(url);

        if base_url != CRASH_LOGS_URL {
            return HttpHandlerResult::NotHandled;
        }

        if *request.method() != Method::Get {
            return HttpHandlerResult::Response(Response::empty(405).boxed());
        }

        let query_params = parse_query_params(url);

        let time_of_crash = match query_params.get(CRASH_LOGS_CRASH_TS_PARAM) {
            Some(crash_timestamp_str) => crash_timestamp_str
                .parse::<DateTime<Utc>>()
                .unwrap_or(Utc::now()),
            None => Utc::now(),
        };

        self.handle_get_crash_logs(time_of_crash, Self::CRASH_LOGS_DELAY_TIMEOUT)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use std::fs::remove_file;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{channel, Receiver};
    use std::sync::Arc;
    use std::{cmp::min, sync::Mutex};
    use std::{io::Write, path::PathBuf, time::Duration};
    use std::{mem::replace, num::NonZeroU32};

    use crate::test_utils::setup_logger;
    use crate::{
        config::LevelMappingConfig,
        logs::{
            completed_log::CompletedLog,
            log_file::{LogFile, LogFileControl},
        },
    };
    use crate::{logs::headroom::HeadroomCheck, util::circular_queue::CircularQueue};
    use chrono::{DateTime, Duration as ChronoDuration, Utc};
    use eyre::Context;
    use flate2::Compression;
    use rstest::{fixture, rstest};
    use ssf::{ServiceMock, SharedServiceThread};
    use tempfile::{tempdir, TempDir};
    use tiny_http::{Method, TestRequest};
    use uuid::Uuid;

    use super::*;

    const IN_MEMORY_LINES: usize = 100;

    #[rstest]
    fn write_logs_to_disk(mut fixture: LogFixture) {
        fixture.write_log(test_line());
        assert_eq!(fixture.count_log_files(), 1);
        assert_eq!(fixture.on_log_completion_calls(), 0);
    }

    #[rstest]
    #[case(50)]
    #[case(100)]
    #[case(150)]
    fn circular_log_queue(#[case] mut log_count: usize, mut fixture: LogFixture) {
        let delta = ChronoDuration::seconds(1);
        let starting_time_str = "2024-09-11T12:34:56Z";
        let mut time = starting_time_str.parse::<DateTime<Utc>>().unwrap();
        for _ in 0..log_count {
            time = time.checked_add_signed(delta).unwrap();
            let log = LogEntry::new_with_message_and_ts("test", time);
            fixture.write_log(log);
        }

        let log_queue = fixture.get_log_queue();

        // Assert that the last value in the queue has the correct timestamp
        let last_val = log_queue.back().unwrap();
        assert_eq!(last_val.ts, time);

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
            log_filter_config: LogFilterConfig::default(),
            in_memory_lines: 1000,
            storage_config: StorageConfig::Persist,
            level_mapping_config: LevelMappingConfig {
                enable: false,
                regex: None,
            },
        };

        let (mut collector, _) = LogCollector::open(
            config,
            Resolution::Normal,
            |CompletedLog { path, .. }| {
                remove_file(&path)
                    .with_context(|| format!("rm {path:?}"))
                    .unwrap();
                Ok(())
            },
            StubHeadroomLimiter,
            ServiceMock::new().mbox,
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
        fixture.write_log(test_line());
        fixture
            .collector
            .lock()
            .unwrap()
            .close_internal()
            .expect("error closing");
        // 0 because the fixture "on_log_completion" moves the file out
        assert_eq!(fixture.count_log_files(), 0);
        assert_eq!(fixture.on_log_completion_calls(), 1);
    }

    #[rstest]
    #[case(StorageConfig::Persist, 60)]
    #[case(StorageConfig::Disabled, 0)]
    fn log_persistence(
        #[case] storage_config: StorageConfig,
        #[case] expected_size: usize,
        mut fixture: LogFixture,
        _setup_logger: (),
    ) {
        fixture.set_log_config(storage_config);

        fixture.write_log(test_line());
        fixture.flush_log_writes().unwrap();

        assert_eq!(fixture.count_log_files(), 1);
        assert_eq!(fixture.read_log_len(), expected_size);
    }

    #[rstest]
    fn forced_rotation_with_nonempty_log(mut fixture: LogFixture) {
        fixture.write_log(test_line());

        fixture
            .service
            .mbox()
            .send_and_wait_for_reply(FlushLogsMsg)
            .unwrap()
            .unwrap();

        assert_eq!(fixture.count_log_files(), 0);
        assert_eq!(fixture.on_log_completion_calls(), 1);
    }

    #[rstest]
    fn delete_log_after_failed_on_completion_callback(mut fixture: LogFixture) {
        fixture
            .on_completion_should_fail
            .store(true, Ordering::Relaxed);
        fixture.write_log(test_line());

        fixture
            .service
            .mbox()
            .send_and_wait_for_reply(FlushLogsMsg)
            .unwrap()
            .unwrap();

        assert_eq!(fixture.on_log_completion_calls(), 1);

        // The old log should have been deleted, to avoid accumulating logs that fail to be moved.
        // No new file will be created without a subsequent write
        assert_eq!(fixture.count_log_files(), 0);
    }

    #[rstest]
    fn forced_rotation_with_empty_log(fixture: LogFixture) {
        fixture
            .service
            .mbox()
            .send_and_wait_for_reply(FlushLogsMsg)
            .unwrap()
            .unwrap();

        assert_eq!(fixture.count_log_files(), 0);
        assert_eq!(fixture.on_log_completion_calls(), 0);
    }

    #[rstest]
    fn forced_rotation_with_write_after_rotate(mut fixture: LogFixture) {
        fixture.write_log(test_line());
        fixture
            .service
            .mbox()
            .send_and_wait_for_reply(FlushLogsMsg)
            .unwrap()
            .unwrap();

        fixture.write_log(test_line());
        assert_eq!(fixture.count_log_files(), 1);
        assert_eq!(fixture.on_log_completion_calls(), 1);
    }

    #[rstest]
    fn recover_old_logfiles() {
        let (tmp_logs, _old_file_path) = existing_tmplogs_with_log(&(Uuid::new_v4().to_string()));
        let fixture = collector_with_logs_dir(tmp_logs);

        let mbox = fixture.service.mbox();
        mbox.send_and_wait_for_reply(RecoverLogsMsg)
            .unwrap()
            .unwrap();

        // We should have generated a MAR entry for the pre-existing logfile.
        assert_eq!(fixture.on_log_completion_calls(), 1);
    }

    #[rstest]
    fn recover_old_logfiles_on_entry() {
        let (tmp_logs, _old_file_path) = existing_tmplogs_with_log(&(Uuid::new_v4().to_string()));
        let fixture = collector_with_logs_dir(tmp_logs);

        let mbox = fixture.service.mbox();
        let entry = LogEntry::new_with_message("test");
        let entry_msg = LogEntryMsg::new(entry, 0);
        mbox.send_and_wait_for_reply(entry_msg).unwrap().unwrap();

        // We should have generated a MAR entry for the pre-existing logfile.
        assert_eq!(fixture.on_log_completion_calls(), 1);
    }

    #[rstest]
    fn delete_files_that_are_not_uuids() {
        let (tmp_logs, old_file_path) = existing_tmplogs_with_log("testfile");
        let fixture = collector_with_logs_dir(tmp_logs);

        let mbox = fixture.service.mbox();
        mbox.send_and_wait_for_reply(RecoverLogsMsg)
            .unwrap()
            .unwrap();

        // And we should have removed the bogus file
        assert!(!old_file_path.exists());

        // We should NOT have generated a MAR entry for the pre-existing bogus file.
        assert_eq!(fixture.on_log_completion_calls(), 0);
    }

    #[rstest]
    fn http_handler_log_get(mut fixture: LogFixture) {
        let date_str = "2024-09-11T12:34:56Z";
        let logs = vec![
            LogEntry::new_with_message_and_ts("xxx", date_str.parse::<DateTime<Utc>>().unwrap()),
            LogEntry::new_with_message_and_ts("yyy", date_str.parse::<DateTime<Utc>>().unwrap()),
            LogEntry::new_with_message_and_ts("zzz", date_str.parse::<DateTime<Utc>>().unwrap()),
        ];
        let log_strings = logs
            .iter()
            .map(|l| serde_json::to_string(l).unwrap())
            .collect::<Vec<_>>();

        for log in &logs {
            fixture.write_log(log.clone());
        }

        let handler =
            CrashLogHandler::new(fixture.service.mbox().into(), fixture.service.mbox().into());

        // This should timeout because the crash "happened" one second after the last log
        // message and there are no further logs and there is a delay timeout of 0 seconds
        let log_response = handler
            .handle_get_crash_logs(
                date_str.parse::<DateTime<Utc>>().unwrap() + Duration::from_secs(1),
                Duration::from_secs(0),
            )
            .unwrap();
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
        let handler =
            CrashLogHandler::new(fixture.service.mbox().into(), fixture.service.mbox().into());

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
        let handler =
            CrashLogHandler::new(fixture.service.mbox().into(), fixture.service.mbox().into());

        let request = TestRequest::new().with_path("/api/v1/other");
        let response = handler.handle_request(&mut request.into());
        assert!(matches!(response, HttpHandlerResult::NotHandled));
    }

    #[rstest]
    fn entry_sender_fail_counter_inc() {
        let mut service = ServiceMock::new_bounded(1);
        let sender = LogEntrySender::new(service.mbox.clone());
        let entry = LogEntry::new_with_message("Test");

        sender.send_entry(entry.clone()).unwrap();
        sender.send_entry(entry.clone()).unwrap();

        let messages = service.take_messages();

        // We should only have 1 entry at this point
        assert_eq!(messages.len(), 1);

        sender.send_entry(entry.clone()).unwrap();

        let messages = service.take_messages();

        // Verify that we have 1 message dropped
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].dropped_msg_count, 1);

        sender.send_entry(entry).unwrap();

        let messages = service.take_messages();

        // Verify that we now have no messages dropped
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].dropped_msg_count, 0);
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
        collector: Arc<Mutex<LogCollector<StubHeadroomLimiter>>>,
        service: SharedServiceThread<LogCollector<StubHeadroomLimiter>>,
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

        fn write_log(&mut self, line: LogEntry) {
            self.collector
                .lock()
                .unwrap()
                .with_mut_inner(|inner| inner.process_log_record(line))
                .unwrap();
        }

        fn read_log_len(&mut self) -> usize {
            self.collector
                .lock()
                .unwrap()
                .with_mut_inner(|inner| {
                    let log = inner.log_file_control.current_log()?;
                    Ok(log.bytes_written())
                })
                .unwrap()
        }

        fn flush_log_writes(&mut self) -> Result<()> {
            self.collector
                .lock()
                .unwrap()
                .with_mut_inner(|inner| inner.log_file_control.current_log()?.flush())
        }

        fn on_log_completion_calls(&self) -> usize {
            self.on_log_completion_receiver.try_iter().count()
        }

        fn get_log_queue(&mut self) -> CircularQueue<LogEntry> {
            self.collector
                .lock()
                .unwrap()
                .with_mut_inner(|inner| Ok(replace(&mut inner.log_queue, CircularQueue::new(100))))
                .unwrap()
        }

        fn set_log_config(&mut self, storage_config: StorageConfig) {
            self.collector
                .lock()
                .unwrap()
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
            _log_timestamp: &DateTime<Utc>,
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
            log_filter_config: LogFilterConfig::default(),
            storage_config: StorageConfig::Persist,
            level_mapping_config: LevelMappingConfig {
                enable: false,
                regex: None,
            },
        };

        let (on_log_completion_sender, on_log_completion_receiver) = channel();

        let on_completion_should_fail = Arc::new(AtomicBool::new(false));

        let (collector, _) = {
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
                Resolution::Off,
                on_log_completion,
                StubHeadroomLimiter,
                ServiceMock::new().mbox,
            )
            .unwrap()
        };

        let log_collector_service = SharedServiceThread::spawn_with(collector);

        LogFixture {
            logs_dir,
            collector: log_collector_service.shared(),
            service: log_collector_service,
            on_log_completion_receiver,
            on_completion_should_fail,
        }
    }

    fn test_line() -> LogEntry {
        let date_str = "2024-09-11T12:34:56Z";
        LogEntry::new_with_message_and_ts("xxx", date_str.parse::<DateTime<Utc>>().unwrap())
    }
}
