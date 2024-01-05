//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Logger wrapper to capture logs that happen during coredump capture.
//!
//! These logs are sent to a channel that is read by the `CoreElfTransformer` and written to a note
//! in the coredump.

use std::sync::mpsc::SyncSender;

use log::{Level, Log, Metadata, Record};

pub const CAPTURE_LOG_CHANNEL_SIZE: usize = 128;

/// Logger wrapper to capture all error and warning logs that happen during coredump capture.
pub struct CoreHandlerLogWrapper {
    log: Box<dyn Log>,
    capture_logs_tx: SyncSender<String>,
}

impl CoreHandlerLogWrapper {
    pub fn new(log: Box<dyn Log>, capture_logs_tx: SyncSender<String>) -> Self {
        Self {
            log,
            capture_logs_tx,
        }
    }
}

impl Log for CoreHandlerLogWrapper {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.log.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if record.level() <= Level::Info {
            let entry = build_log_string(record);

            // Errors are ignored here because the only options are to panic or log the error.
            // Panicking is not a great option because this isn't critical functionality. Logging
            // the error isn't an option because we'd risk infinite recursion since we're already
            // inside the logger.
            let _ = self.capture_logs_tx.try_send(entry);
        }

        self.log.log(record)
    }

    fn flush(&self) {
        self.log.flush()
    }
}

/// Build a log string from a log record.
///
/// The log string is formatted as follows:
///
/// ```text
/// <log level> <target>:<line> - <message>
/// ```
fn build_log_string(record: &Record) -> String {
    match record.line() {
        Some(line) => format!(
            "{} {}:{} - {}",
            record.level(),
            record.target(),
            line,
            record.args()
        ),
        None => format!("{} {} - {}", record.level(), record.target(), record.args()),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::sync::mpsc::sync_channel;

    use insta::assert_json_snapshot;

    #[test]
    fn test_log_saving() {
        let mut logger = stderrlog::new();
        logger.module("memfaultd").verbosity(10);

        let (capture_logs_tx, capture_logs_rx) = sync_channel(2);
        let wrapper = CoreHandlerLogWrapper::new(Box::new(logger), capture_logs_tx);

        let error_record = build_log_record(Level::Error);
        let warn_record = build_log_record(Level::Warn);
        let info_record = build_log_record(Level::Info);

        wrapper.log(&error_record);
        wrapper.log(&warn_record);
        wrapper.log(&info_record);

        let errors: Vec<String> = capture_logs_rx.try_iter().collect();
        assert_json_snapshot!(errors);
    }

    fn build_log_record(level: Level) -> Record<'static> {
        Record::builder()
            .args(format_args!("Test message"))
            .level(level)
            .target("test")
            .file(Some("log_wrapper.rs"))
            .line(Some(71))
            .module_path(Some("core_handler"))
            .build()
    }
}
