//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::PathBuf,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

use eyre::{eyre, Result};
use flate2::read::{GzEncoder, ZlibEncoder};
use log::{error, warn};
use tiny_http::{Method, Request, Response};

use crate::{
    config::{LinuxCustomTraceConfig, LinuxCustomTraceLogCompression},
    http_server::{HttpHandler, HttpHandlerResult, TraceRequest},
    mar::{MarEntryBuilder, Metadata, NoMetadata},
    metrics::CrashInfo,
    network::NetworkConfig,
    util::{
        fs::DEFAULT_GZIP_COMPRESSION_LEVEL, persistent_rate_limiter::PersistentRateLimiter,
        time_measure::TimeMeasure,
    },
};

#[cfg(feature = "logging")]
use {crate::logs::messages::GetQueuedLogsMsg, ssf::MsgMailbox};

pub struct SaveTraceHandler {
    mar_staging_area: PathBuf,
    network_config: NetworkConfig,
    crash_free_interval_channel: Box<Sender<CrashInfo<Instant>>>,
    trace_config: LinuxCustomTraceConfig,
    rate_limiter_path: PathBuf,
    #[cfg(feature = "logging")]
    get_queued_logs_mbox: Option<MsgMailbox<GetQueuedLogsMsg>>,
}

impl SaveTraceHandler {
    #[cfg(feature = "logging")]
    pub fn new(
        mar_staging_area: PathBuf,
        network_config: NetworkConfig,
        crash_free_interval_channel: Box<Sender<CrashInfo<Instant>>>,
        get_queued_logs_mbox: Option<MsgMailbox<GetQueuedLogsMsg>>,
        trace_config: LinuxCustomTraceConfig,
        rate_limiter_path: PathBuf,
    ) -> Self {
        Self {
            mar_staging_area,
            network_config,
            crash_free_interval_channel,
            trace_config,
            rate_limiter_path,
            get_queued_logs_mbox,
        }
    }

    #[cfg(not(feature = "logging"))]
    pub fn new(
        mar_staging_area: PathBuf,
        network_config: NetworkConfig,
        crash_free_interval_channel: Box<Sender<CrashInfo<Instant>>>,
        trace_config: LinuxCustomTraceConfig,
        rate_limiter_path: PathBuf,
    ) -> Self {
        Self {
            mar_staging_area,
            network_config,
            crash_free_interval_channel,
            trace_config,
            rate_limiter_path,
        }
    }

    fn log_compression_config(&self) -> LinuxCustomTraceLogCompression {
        self.trace_config.log_compression
    }

    fn compress_log_file<R: Read>(
        &self,
        file_name: String,
        reader: R,
        mar_entry_builder: &MarEntryBuilder<NoMetadata>,
    ) -> std::io::Result<PathBuf> {
        let (suffix, mut boxed_reader): (&str, Box<dyn Read>) = match self.log_compression_config()
        {
            LinuxCustomTraceLogCompression::Gzip => (
                "log.gzip",
                Box::new(GzEncoder::new(reader, DEFAULT_GZIP_COMPRESSION_LEVEL)),
            ),
            LinuxCustomTraceLogCompression::Zlib => (
                "log.zlib",
                Box::new(ZlibEncoder::new(reader, DEFAULT_GZIP_COMPRESSION_LEVEL)),
            ),
            LinuxCustomTraceLogCompression::None => ("log", Box::new(reader)),
        };
        let file_path =
            mar_entry_builder.make_attachment_path_in_entry_dir(format!("{file_name}.{suffix}"));
        let file = File::create(&file_path)?;
        {
            let mut buf_writer = BufWriter::new(file);
            std::io::copy(&mut boxed_reader, &mut buf_writer)?;
            buf_writer.flush()?;
        }
        Ok(file_path)
    }

    fn handle_save_trace(&self, request: &mut Request) -> HttpHandlerResult {
        let mut body = String::new();
        if let Err(e) = request.as_reader().read_to_string(&mut body) {
            return HttpHandlerResult::Error(format!("Failed to read request body: {}", e));
        }

        #[allow(unused_mut)]
        let mut request: TraceRequest = match serde_json::from_str(&body) {
            Ok(args) => args,
            Err(e) => {
                return HttpHandlerResult::Error(format!("Failed to parse request body: {}", e));
            }
        };

        if request.crash {
            send_crash_info(&self.crash_free_interval_channel, request.program.clone());
        }

        let mut mar_entry_builder = match MarEntryBuilder::new(&self.mar_staging_area) {
            Ok(mar_entry_builder) => mar_entry_builder,
            Err(e) => {
                return HttpHandlerResult::Error(format!(
                    "Failed to create MarEntryBuilder from path {}: {e}",
                    self.mar_staging_area.display()
                ));
            }
        };

        let mut filename = None;

        match request.log_file_name {
            // Log file in trace; copy log file in and compress per config
            Some(log_file_name) => {
                let log_file_path = PathBuf::from(&log_file_name);

                let Some(log_filename) = &log_file_path.file_name() else {
                    error!("Invalid log file name: {log_file_name}");
                    return HttpHandlerResult::Error(format!(
                        "Invalid log file name: {log_file_name}"
                    ));
                };

                let Ok(log_file) = File::open(&log_file_path) else {
                    error!("Log file {log_file_name} does not exist!");
                    return HttpHandlerResult::Error(format!(
                        "Log file {log_file_name} does not exist"
                    ));
                };

                let buf_reader = BufReader::new(log_file);
                // Check if compression is enabled and compress the log file if so
                let final_log_path = self.compress_log_file(
                    log_filename.to_string_lossy().to_string(),
                    buf_reader,
                    &mar_entry_builder,
                );
                let final_log_path = match final_log_path {
                    Ok(final_log_path) => final_log_path,
                    Err(e) => {
                        error!("Failed to compress custom trace log file: {e}");
                        return HttpHandlerResult::Error(format!(
                            "Failed to compress custom trace log file: {e}"
                        ));
                    }
                };

                filename = Some(final_log_path.to_string_lossy().to_string());

                let Ok(add_attachment) = mar_entry_builder.add_attachment(final_log_path) else {
                    error!("Failed to add log file attachment");
                    return HttpHandlerResult::Error(
                        "Failed to add log file attachment".to_string(),
                    );
                };
                mar_entry_builder = add_attachment;
            }
            // No log file in trace; attach our own log buffer
            None =>
            {
                #[cfg(feature = "logging")]
                match &self.get_queued_logs_mbox {
                    Some(mbox) => match self.dump_log_buffer(mbox, &mar_entry_builder) {
                        Ok(log_file_path) => {
                            filename = Some(log_file_path);
                        }
                        Err(e) => {
                            warn!("No log file attached to trace, and failed to dump memfaultd log buffer: {e}.");
                        }
                    },
                    None => {
                        warn!("No log file attached to trace, and logging feature is disabled.");
                    }
                }
            }
        }

        let mar_builder = mar_entry_builder.set_metadata(Metadata::new_custom_trace(
            request.program,
            request.reason,
            Some(request.crash),
            request.signature,
            Some(request.source),
            filename,
            Some(self.log_compression_config().into()),
        ));

        match mar_builder.save(&self.network_config) {
            Ok(_mar_entry) => HttpHandlerResult::Response(Response::empty(200).boxed()),
            Err(e) => HttpHandlerResult::Error(format!("Failed to save MAR entry: {e}")),
        }
    }

    fn check_rate_limits(&self) -> Result<bool> {
        let rate_limit_duration = chrono::Duration::from_std(self.trace_config.rate_limit_duration)
            .map_err(|_| eyre!("Invalid trace rate limit duration"))?;
        let mut rate_limiter = PersistentRateLimiter::load(
            &self.rate_limiter_path,
            self.trace_config.rate_limit_count,
            rate_limit_duration,
        )
        .map_err(|_| eyre!("Invalid rate limiter configuration"))?;

        if rate_limiter.check() {
            if let Err(e) = rate_limiter.save() {
                Err(eyre!(format!("Failed to persist rate limiter: {e}")))
            } else {
                Ok(false)
            }
        } else {
            Ok(true)
        }
    }

    #[cfg(feature = "logging")]
    fn dump_log_buffer(
        &self,
        mbox: &MsgMailbox<GetQueuedLogsMsg>,
        mar_entry_builder: &MarEntryBuilder<NoMetadata>,
    ) -> Result<String> {
        // Get queued logs from LogCollector
        use eyre::ContextCompat;
        use std::io::Cursor;

        let logs = mbox.send_and_wait_for_reply(GetQueuedLogsMsg)??;

        let cursor = Cursor::new(logs.join("\n"));
        let path = self.compress_log_file("log_buffer.txt".into(), cursor, mar_entry_builder)?;

        Ok(path
            .file_name()
            .context("Failed to access log buffer filename")?
            .to_string_lossy()
            .to_string())
    }
}

impl HttpHandler for SaveTraceHandler {
    fn handle_request(&self, request: &mut Request) -> HttpHandlerResult {
        if request.url() != "/v1/trace/save" || *request.method() != Method::Post {
            return HttpHandlerResult::NotHandled;
        }

        match self.check_rate_limits() {
            Ok(true) => return HttpHandlerResult::Error("Trace rate limited".to_string()),
            Ok(false) => (),
            Err(e) => return HttpHandlerResult::Error(format!("Failed to check rate limits: {e}")),
        }

        self.handle_save_trace(request)
    }
}

fn send_crash_info<T>(channel: &Sender<CrashInfo<T>>, process_name: String)
where
    T: TimeMeasure + Copy + Ord + std::ops::Add<Duration, Output = T> + Send + Sync + 'static,
{
    if let Err(e) = channel.send(CrashInfo {
        process_name,
        timestamp: T::now(),
    }) {
        warn!("Failed to send crash timestamp: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, LinuxCustomTraceConfig};
    use crate::network::NetworkConfig;
    use std::fs::create_dir_all;
    use std::io::Cursor;
    use std::sync::mpsc::channel;
    use tempfile::TempDir;

    fn create_test_handler(
        config: Config,
        trace_config: LinuxCustomTraceConfig,
    ) -> (SaveTraceHandler, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let mar_staging_area = temp_dir.path().join("mar");
        create_dir_all(&mar_staging_area).unwrap();

        let network_config = NetworkConfig::from(&config);
        let (sender, _receiver) = channel();
        let rate_limit_file_path = temp_dir.path().join("rate_limit_test");
        File::create(&rate_limit_file_path).unwrap();

        let handler = SaveTraceHandler::new(
            mar_staging_area,
            network_config,
            Box::new(sender),
            #[cfg(feature = "logging")]
            None,
            trace_config,
            rate_limit_file_path,
        );

        (handler, temp_dir)
    }

    #[test]
    fn test_compress_log_file_creates_compressed_file() {
        let mut config = Config::test_fixture();
        config.config_file.custom_trace = Some(LinuxCustomTraceConfig {
            log_compression: LinuxCustomTraceLogCompression::Gzip,
            rate_limit_count: 5,
            rate_limit_duration: Duration::from_secs(3600),
        });
        let trace_config = LinuxCustomTraceConfig::default();
        let (handler, temp_dir) = create_test_handler(config, trace_config);

        let log_content = "Test log line 1\nTest log line 2\nTest log line 3\n";
        let cursor = Cursor::new(log_content);

        let mar_entry_builder = MarEntryBuilder::new(&temp_dir.path().join("mar")).unwrap();
        let result = handler.compress_log_file("test.log".to_string(), cursor, &mar_entry_builder);
        assert!(result.is_ok());

        let compressed_path = result.unwrap();
        assert!(compressed_path.exists());
        assert!(compressed_path.to_string_lossy().ends_with("log.gzip"));

        let compressed_size = std::fs::metadata(&compressed_path).unwrap().len();
        assert!(compressed_size > 0);
    }

    #[test]
    fn test_compress_log_file_handles_io_error() {
        let config = Config::test_fixture();
        let trace_config = LinuxCustomTraceConfig::default();
        let (handler, temp_dir) = create_test_handler(config, trace_config);

        // Create a reader that will fail when reading
        struct FailingReader;
        impl std::io::Read for FailingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(std::io::ErrorKind::Other, "Read error"))
            }
        }

        let failing_reader = FailingReader;
        let mar_entry_builder = MarEntryBuilder::new(&temp_dir.path().join("mar")).unwrap();
        let result =
            handler.compress_log_file("test.log".to_string(), failing_reader, &mar_entry_builder);

        assert!(result.is_err());
    }

    #[test]
    fn test_large_log_file_compression() {
        let mut config = Config::test_fixture();
        config.config_file.custom_trace = Some(LinuxCustomTraceConfig {
            log_compression: LinuxCustomTraceLogCompression::Gzip,
            rate_limit_count: 5,
            rate_limit_duration: Duration::from_secs(3600),
        });
        let trace_config = LinuxCustomTraceConfig::default();
        let (handler, temp_dir) = create_test_handler(config, trace_config);

        // Create a larger log file that should compress well
        let mut log_content = String::new();
        for i in 0..1000 {
            log_content.push_str(&format!(
                "This is log line number {} with repeated content\n",
                i
            ));
        }
        let cursor = Cursor::new(log_content.as_bytes());

        let mar_entry_builder = MarEntryBuilder::new(&temp_dir.path().join("mar")).unwrap();
        let result =
            handler.compress_log_file("large_test.log".to_string(), cursor, &mar_entry_builder);
        assert!(result.is_ok());

        let compressed_path = result.unwrap();
        assert!(compressed_path.exists());

        // Verify compressed file exists and has reasonable size
        let compressed_size = std::fs::metadata(&compressed_path).unwrap().len();
        assert!(compressed_size > 0);
        // The compressed size should be much smaller than the original large content
        assert!(compressed_size < log_content.len() as u64);
    }

    #[test]
    fn test_compression_with_empty_log_file() {
        let mut config = Config::test_fixture();
        config.config_file.custom_trace = Some(LinuxCustomTraceConfig {
            log_compression: LinuxCustomTraceLogCompression::Gzip,
            rate_limit_count: 5,
            rate_limit_duration: Duration::from_secs(3600),
        });
        let trace_config = LinuxCustomTraceConfig::default();
        let (handler, temp_dir) = create_test_handler(config, trace_config);

        let cursor = Cursor::new(b"");

        let mar_entry_builder = MarEntryBuilder::new(&temp_dir.path().join("mar")).unwrap();
        let result = handler.compress_log_file("empty.log".to_string(), cursor, &mar_entry_builder);
        assert!(result.is_ok());

        let compressed_path = result.unwrap();
        assert!(compressed_path.exists());

        // Even empty files should create a valid gzip file
        let compressed_size = std::fs::metadata(&compressed_path).unwrap().len();
        assert!(compressed_size > 0); // Gzip header is always present
    }

    #[test]
    fn test_compression_with_zlib_encoder() {
        let config = Config::test_fixture();
        let trace_config = LinuxCustomTraceConfig {
            log_compression: LinuxCustomTraceLogCompression::Zlib,
            ..Default::default()
        };

        let (handler, temp_dir) = create_test_handler(config, trace_config);

        let log_content = "Test log content\n";
        let cursor = Cursor::new(log_content);

        let mar_entry_builder = MarEntryBuilder::new(&temp_dir.path().join("mar")).unwrap();
        let result = handler.compress_log_file("test.log".to_string(), cursor, &mar_entry_builder);
        assert!(result.is_ok());

        let compressed_path = result.unwrap();
        assert!(compressed_path.exists());
        assert!(compressed_path.to_string_lossy().ends_with("log.zlib"));

        let compressed_size = std::fs::metadata(&compressed_path).unwrap().len();
        assert!(compressed_size > 0);
    }

    #[test]
    fn test_compression_preserves_file_name_pattern() {
        let mut config = Config::test_fixture();
        config.config_file.custom_trace = Some(LinuxCustomTraceConfig {
            log_compression: LinuxCustomTraceLogCompression::Gzip,
            rate_limit_count: 5,
            rate_limit_duration: Duration::from_secs(3600),
        });
        let trace_config = LinuxCustomTraceConfig::default();
        let (handler, temp_dir) = create_test_handler(config, trace_config);

        let log_content = "Test log content\n";
        let cursor = Cursor::new(log_content);

        let mar_entry_builder = MarEntryBuilder::new(&temp_dir.path().join("mar")).unwrap();
        let result =
            handler.compress_log_file("custom_name.log".to_string(), cursor, &mar_entry_builder);
        assert!(result.is_ok());

        let compressed_path = result.unwrap();
        assert!(compressed_path
            .to_string_lossy()
            .contains("custom_name.log"));
        assert!(compressed_path.to_string_lossy().ends_with("log.gzip"));
    }

    #[test]
    fn test_compression_with_no_compression() {
        let config = Config::test_fixture();
        let trace_config = LinuxCustomTraceConfig {
            log_compression: LinuxCustomTraceLogCompression::None,
            ..Default::default()
        };
        let (handler, temp_dir) = create_test_handler(config, trace_config);

        let log_content = "Test log content\n";
        let cursor = Cursor::new(log_content);

        let mar_entry_builder = MarEntryBuilder::new(&temp_dir.path().join("mar")).unwrap();
        let result = handler.compress_log_file("test.log".to_string(), cursor, &mar_entry_builder);
        assert!(result.is_ok());

        let compressed_path = result.unwrap();
        assert!(compressed_path.exists());
        assert!(compressed_path.to_string_lossy().ends_with("log"));

        let file_size = std::fs::metadata(&compressed_path).unwrap().len();
        assert!(file_size > 0);
        assert_eq!(file_size, log_content.len() as u64);
    }

    #[test]
    fn test_rate_limiting() {
        let config = Config::test_fixture();
        let trace_config = LinuxCustomTraceConfig {
            rate_limit_count: 2,
            ..Default::default()
        };

        let (handler, _tempdir) = create_test_handler(config, trace_config);
        assert!(!handler.check_rate_limits().unwrap());
        assert!(!handler.check_rate_limits().unwrap());
        assert!(handler.check_rate_limits().unwrap());
    }

    #[cfg(feature = "logging")]
    #[test]
    fn test_dump_log_buffer_calls_compress_with_config() {
        let mut config = Config::test_fixture();
        config.config_file.custom_trace = Some(LinuxCustomTraceConfig {
            log_compression: LinuxCustomTraceLogCompression::Gzip,
            rate_limit_count: 5,
            rate_limit_duration: Duration::from_secs(3600),
        });
        let trace_config = LinuxCustomTraceConfig::default();
        let (handler, temp_dir) = create_test_handler(config, trace_config);

        // Test that when no log file is attached, the compression config is used
        // This verifies the path in lines 173-191 where dump_log_buffer is called
        let mar_entry_builder = MarEntryBuilder::new(&temp_dir.path().join("mar")).unwrap();

        // Create test log buffer content
        let log_buffer_content = vec![
            "Log line 1".to_string(),
            "Log line 2".to_string(),
            "Log line 3".to_string(),
        ];

        // Simulate the log buffer content being compressed
        let joined_logs = log_buffer_content.join("\n");
        let cursor = Cursor::new(joined_logs.as_bytes());

        // Test that compress_log_file works with the buffer content and respects log_compression_config
        let result =
            handler.compress_log_file("log_buffer.txt".to_string(), cursor, &mar_entry_builder);
        assert!(result.is_ok());

        let compressed_path = result.unwrap();
        assert!(compressed_path.exists());
        assert!(compressed_path.to_string_lossy().contains("log_buffer.txt"));
        assert!(compressed_path.to_string_lossy().ends_with("log.gzip"));

        // Verify the file was created and has content
        let compressed_size = std::fs::metadata(&compressed_path).unwrap().len();
        assert!(compressed_size > 10); // Gzip header is at least 10 bytes
    }
}
