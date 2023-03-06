//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Collect logs into log files and save them as MAR entries.
//!
use eyre::{eyre, Context, Result};
use log::{debug, error, trace, warn};
use serde_json::{json, Value};
use std::fs::remove_file;
use std::path::Path;
use std::{fs, thread};
use std::{
    io::{BufWriter, Write},
    sync::Arc,
};
use std::{
    mem::replace,
    time::{Duration, Instant},
};
use std::{path::PathBuf, sync::Mutex};
use uuid::Uuid;

use crate::config::Config;
use crate::mar::mar_entry::{MarEntry, MarEntryBuilder};
use crate::network::NetworkConfig;
use crate::util::fs::get_files_sorted_by_mtime;
use crate::util::rate_limiter::RateLimiter;

pub struct LogCollector {
    inner: Arc<Mutex<Option<Inner>>>,
}

impl LogCollector {
    /// Create a new log collector and open a new log file for writing.
    pub fn open<R: FnMut(u64) + Send + 'static>(
        log_config: LogCollectorConfig,
        mut before_add_mar_entry: R,
    ) -> Result<Self> {
        fs::create_dir_all(&log_config.log_tmp_path).wrap_err_with(|| {
            format!(
                "Unable to create directory to store in-progress logs: {}",
                log_config.log_tmp_path.display()
            )
        })?;

        // Collect any leftover logfiles in the tmp folder
        let next_cid = Self::recover_old_logs(
            &log_config.log_tmp_path,
            &log_config.mar_staging_path,
            &log_config.network_config,
            &mut before_add_mar_entry,
        )?;

        let shared_config = Arc::new(log_config);

        Ok(Self {
            inner: Arc::new(Mutex::new(Some(Inner {
                log_config: shared_config.clone(),
                current_log: Inner::open_logfile(&shared_config.log_tmp_path, next_cid)?,
                rate_limiter: RateLimiter::new(1000, 1000, 1000),
                before_add_mar_entry: Box::new(before_add_mar_entry),
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
        self.with_mut_inner(|inner| inner.flush_logs_internal())
    }

    /// Rotate the logs if needed
    pub fn rotate_if_needed(&mut self) -> Result<()> {
        self.with_mut_inner(|inner| inner.rotate_if_needed())
    }

    /// Try to get the inner log_collector or return an error
    fn with_mut_inner<F: FnOnce(&mut Inner) -> Result<()>>(&mut self, fun: F) -> Result<()> {
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

    fn recover_old_logs<R: FnMut(u64) + Send + 'static>(
        tmp_logs: &Path,
        mar_staging: &Path,
        net_config: &NetworkConfig,
        before_add_mar_entry: &mut R,
    ) -> Result<Uuid> {
        // Make a list of all the files we want to collect and parse their CID from the filename
        let existing_files = get_files_sorted_by_mtime(tmp_logs)?;
        let uuids = existing_files.iter().map(|p| {
            match (p.metadata(), p.file_stem()) {
                // We are only interested in files with size>0 and a valid filename.
                (Ok(metadata), Some(filestem)) if metadata.len() > 0 => {
                    Uuid::try_parse(&filestem.to_string_lossy()).ok()
                }
                _ => None,
            }
        });

        trace!(
            "List of files: {:?} \nUuids: {:?}",
            existing_files,
            uuids.clone().collect::<Vec<_>>()
        );

        // Delete any unwanted files (from disk and from the list)
        let logs_and_uuids = existing_files
            .iter()
            .zip(uuids)
            .filter_map(|(p, cid)| match cid {
                None => {
                    if let Err(e) = remove_file(p) {
                        warn!("Unable to delete bogus log file: {} - {}.", p.display(), e);
                    }
                    None
                }
                Some(cid) => Some((p.to_owned(), cid)),
            })
            .collect::<Vec<_>>();

        debug!("Recovering logfile(s): {:?}", logs_and_uuids);

        let mut it = logs_and_uuids.iter().peekable();
        let mut next_cid = Uuid::new_v4();
        while let Some((log, cid)) = it.next() {
            // For next_cid, use the cid of the next file - or generate a new one
            next_cid = match it.peek() {
                Some((_, cid)) => *cid,
                _ => Uuid::new_v4(),
            };
            // Write the MAR entry which will move the logfile
            let builder = MarEntryBuilder::new_log(log.clone(), *cid, next_cid)?;
            before_add_mar_entry(builder.estimated_entry_size());
            builder.save(mar_staging, net_config)?;
        }

        Ok(next_cid)
    }

    /// Close and dispose of the inner log collector.
    /// This is not public because it does not consume self (to be compatible with drop()).
    fn close_internal(&mut self) -> Result<()> {
        match self.inner.lock() {
            Ok(mut inner_opt) => {
                match (*inner_opt).take() {
                    Some(mut inner) => inner.flush_logs_internal(),
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

impl Drop for LogCollector {
    fn drop(&mut self) {
        if let Err(e) = self.close_internal() {
            warn!("Error closing log collector: {}", e);
        }
    }
}

/// The log collector keeps one Inner struct behind a Arc<Mutex<>> so it can be
/// shared by multiple threads.
struct Inner {
    log_config: Arc<LogCollectorConfig>,
    // We use an Option<Value> here because we have no typed-guarantee that every
    // log message will include a `ts` key.
    rate_limiter: RateLimiter<Option<Value>>,
    current_log: LogFile,
    before_add_mar_entry: Box<dyn FnMut(u64) + Send>,
}

/// In memory representation of one logfile while it is being written to.
struct LogFile {
    cid: Uuid,
    path: PathBuf,
    writer: BufWriter<fs::File>,
    bytes_written: usize,
    since: Instant,
}

impl LogFile {
    fn write_json_line(&mut self, json: Value) -> Result<()> {
        let bytes = serde_json::to_vec(&json)?;
        let mut written = self.writer.write(&bytes)?;
        written += self.writer.write("\n".as_bytes())?;
        self.bytes_written += written;
        Ok(())
    }
}

impl Inner {
    // Process one log record - To call this, the caller must have acquired a
    // mutex on the Inner object.
    // Be careful to not try to acquire other mutexes here to avoid a
    // dead-lock. Everything we need should be in Inner.
    fn process_log_record(&mut self, log: Value) -> Result<()> {
        // Rotate before writing (in case log file is now too old)
        self.rotate_if_needed()?;

        let log_timestamp = log.get("ts").cloned();
        let logfile = &mut self.current_log;

        self.rate_limiter.run_within_limits(log_timestamp, |rate_limited_calls| {
            // Print a message if some previous calls were rate limited.
            if let Some(limited) = rate_limited_calls {
                let rate_limiting_message = json!({ "ts": limited.latest_call, "data": { "MESSAGE": format!("Memfaultd rate limited {} messages.", limited.count)} });
                logfile.write_json_line(rate_limiting_message)?;
            }
            logfile.write_json_line(log)?;
            Ok(())
        })?;

        // Rotate after writing (in case log file is now too large)
        self.rotate_if_needed()?;
        Ok(())
    }

    fn open_logfile(log_tmp_path: &Path, cid: Uuid) -> Result<LogFile> {
        let filename = cid.to_string() + ".log";
        let path = log_tmp_path.join(filename);
        let file = fs::File::create(&path)?;
        let writer = BufWriter::new(file);

        trace!("Now writing logs to: {}", path.display());
        Ok(LogFile {
            cid,
            path,
            writer,
            bytes_written: 0,
            since: Instant::now(),
        })
    }

    fn rotate_if_needed(&mut self) -> Result<()> {
        if self.current_log.bytes_written >= self.log_config.log_max_size
            || self.current_log.since.elapsed() > self.log_config.log_max_duration
        {
            self.rotate_log().wrap_err("Error rotating log")?;
        }
        Ok(())
    }

    fn flush_logs_internal(&mut self) -> Result<()> {
        if self.current_log.bytes_written > 0 {
            return self.rotate_log();
        }
        debug!("Log flush requested but we have no logs to flush at the moment.");
        Ok(())
    }

    fn generate_mar_entry(&mut self, mut log: LogFile, next_cid: Uuid) -> Result<MarEntry> {
        // Prepare the MAR entry
        let mar_builder = MarEntryBuilder::new_log(log.path.clone(), log.cid, next_cid)?;

        // Drop the old log, closing the buffered writer and the file.
        log.writer.flush()?;
        drop(log);

        (self.before_add_mar_entry)(mar_builder.estimated_entry_size());

        // Move the log in the mar_staging area and add a manifest
        let mar_entry = mar_builder.save(
            &self.log_config.mar_staging_path,
            &self.log_config.network_config,
        )?;
        debug!("New MAR entry generated: {}", mar_entry.path.display());

        Ok(mar_entry)
    }

    /// Close current logfile, create a MAR entry and starts a new one.
    fn rotate_log(&mut self) -> Result<()> {
        // Start a new log and make it the current one. We are now writing there.
        let closed_log = replace(
            &mut self.current_log,
            Inner::open_logfile(&self.log_config.log_tmp_path, Uuid::new_v4())?,
        );

        self.generate_mar_entry(closed_log, self.current_log.cid)?;

        Ok(())
    }
}

pub struct LogCollectorConfig {
    /// Directory where MAR files are staged to be uploaded regularly. We will write logs there.
    mar_staging_path: PathBuf,

    /// Folder where to store logfiles while they are being written
    log_tmp_path: PathBuf,

    /// Files will be rotated when they reach this size (so they may be slightly larger)
    log_max_size: usize,

    /// MAR entry will be rotated when they get this old.
    log_max_duration: Duration,

    network_config: NetworkConfig,
}

impl From<&Config> for LogCollectorConfig {
    fn from(config: &Config) -> Self {
        Self {
            mar_staging_path: config.mar_staging_path(),
            log_tmp_path: config
                .config_file
                .data_dir
                .join(&config.config_file.logs.tmp_folder),
            log_max_size: config.config_file.logs.rotate_size,
            log_max_duration: config.config_file.logs.rotate_after,
            network_config: config.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{channel, Receiver};
    use std::{io::Write, path::PathBuf, time::Duration};

    use rstest::{fixture, rstest};
    use serde_json::{json, Value};
    use tempfile::{tempdir, TempDir};
    use uuid::Uuid;

    use crate::{network::NetworkConfig, test_utils::setup_logger};

    use super::{LogCollector, LogCollectorConfig};

    #[rstest]
    fn write_logs_to_disk(mut fixture: LogFixture) {
        fixture.write_log(json!({"ts": 0, "MESSAGE": "xxx"}));
        assert_eq!(fixture.count_log_files(), 1);
        assert_eq!(fixture.before_add_mar_entry_calls(), 0);
    }

    #[rstest]
    fn forced_rotation_with_nonempty_log(_setup_logger: (), mut fixture: LogFixture) {
        fixture.write_log(json!({"ts": 0, "MESSAGE": "xxx"}));

        fixture.collector.flush_logs().unwrap();

        assert_eq!(fixture.count_log_files(), 1);
        assert_eq!(fixture.count_mar_entries(), 1);
        assert_eq!(fixture.before_add_mar_entry_calls(), 1);
    }

    #[rstest]
    fn forced_rotation_with_empty_log(mut fixture: LogFixture) {
        fixture.collector.flush_logs().unwrap();

        assert_eq!(fixture.count_log_files(), 1);
        assert_eq!(fixture.count_mar_entries(), 0);
        assert_eq!(fixture.before_add_mar_entry_calls(), 0);
    }

    #[rstest]
    fn recover_old_logfiles(_setup_logger: ()) {
        let (tmp_logs, old_file_path) =
            existing_tmplogs_with_log(&(Uuid::new_v4().to_string() + ".log"));
        let fixture = collector_with_logs_dir(tmp_logs);

        // We should have generated a MAR entry for the pre-existing logfile.
        assert_eq!(fixture.count_mar_entries(), 1);

        // And we should have removed the old file
        assert!(!old_file_path.exists());

        // The before_add_mar_entry callback should have been called:
        assert_eq!(fixture.before_add_mar_entry_calls(), 1);
    }

    #[rstest]
    fn delete_files_that_are_not_uuids(_setup_logger: ()) {
        let (tmp_logs, old_file_path) = existing_tmplogs_with_log("testfile.log");
        let fixture = collector_with_logs_dir(tmp_logs);

        // We should NOT have generated a MAR entry for the pre-existing bogus file.
        assert_eq!(fixture.count_mar_entries(), 0);

        // And we should have removed the bogus file
        assert!(!old_file_path.exists());

        assert_eq!(fixture.before_add_mar_entry_calls(), 0);
    }

    fn existing_tmplogs_with_log(filename: &str) -> (TempDir, PathBuf) {
        let tmp_logs = tempdir().unwrap();
        let file_path = tmp_logs
            .path()
            .to_path_buf()
            .join(filename)
            .with_extension("log");

        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(b"some content in the log").unwrap();
        drop(file);
        (tmp_logs, file_path)
    }

    struct LogFixture {
        mar_dir: TempDir,
        logs_dir: TempDir,
        collector: LogCollector,
        before_add_receiver: Receiver<u64>,
    }
    impl LogFixture {
        fn count_log_files(&self) -> usize {
            std::fs::read_dir(&self.logs_dir).unwrap().count()
        }

        fn count_mar_entries(&self) -> usize {
            std::fs::read_dir(&self.mar_dir).unwrap().count()
        }

        fn write_log(&mut self, line: Value) {
            self.collector
                .with_mut_inner(|inner| inner.current_log.write_json_line(line))
                .unwrap();
        }

        fn before_add_mar_entry_calls(&self) -> usize {
            self.before_add_receiver.try_iter().count()
        }
    }

    #[fixture]
    fn fixture() -> LogFixture {
        collector_with_logs_dir(tempdir().unwrap())
    }

    fn collector_with_logs_dir(logs_dir: TempDir) -> LogFixture {
        let mar_dir = tempdir().unwrap();

        let config = LogCollectorConfig {
            mar_staging_path: mar_dir.path().to_owned(),
            log_tmp_path: logs_dir.path().to_owned(),
            log_max_size: 1024,
            log_max_duration: Duration::from_secs(3600),
            network_config: NetworkConfig::test_fixture(),
        };

        let (before_add_sender, before_add_receiver) = channel();

        let collector = LogCollector::open(config, move |estimated_entry_size| {
            before_add_sender.send(estimated_entry_size).unwrap();
        })
        .unwrap();

        LogFixture {
            mar_dir,
            logs_dir,
            collector,
            before_add_receiver,
        }
    }
}
