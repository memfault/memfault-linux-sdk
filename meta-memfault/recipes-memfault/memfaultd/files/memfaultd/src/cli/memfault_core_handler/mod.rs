//
// Copyright (c) Memfault, Inc.
// See License.txt for details
mod arch;
mod auxv;
mod core_elf_memfault_note;
mod core_elf_note;
mod core_reader;
mod core_transformer;
mod core_writer;
mod elf_utils;
mod find_dynamic;
mod find_elf_headers;
mod find_stack;
mod log_wrapper;
mod memory_range;
mod procfs;
mod r_debug;
mod stack_unwinder;
#[cfg(test)]
mod test_utils;

use self::arch::{coredump_thread_filter_supported, stacktrace_supported};
use self::core_reader::CoreReader;
use self::log_wrapper::CoreHandlerLogWrapper;
use self::log_wrapper::CAPTURE_LOG_CHANNEL_SIZE;
use self::procfs::{proc_mem_stream, ProcMapsImpl};
use self::{core_elf_memfault_note::CoredumpMetadata, core_transformer::CoreTransformerOptions};
use self::{core_reader::CoreReaderImpl, core_transformer::CoreTransformerLogFetcher};
use self::{core_writer::CoreWriterImpl, log_wrapper::CAPTURE_LOG_MAX_LEVEL};
use crate::config::{Config, CoredumpCaptureStrategy, CoredumpCompression};
use crate::mar::manifest::{CompressionAlgorithm, Metadata};
use crate::mar::mar_entry_builder::MarEntryBuilder;
use crate::network::NetworkConfig;
use crate::util::disk_size::get_disk_space;
use crate::util::io::{ForwardOnlySeeker, StreamPositionTracker};
use crate::util::persistent_rate_limiter::PersistentRateLimiter;
use crate::util::system::{get_process_name, read_proc_cmdline};
use crate::{cli, util::fs::DEFAULT_GZIP_COMPRESSION_LEVEL};
use argh::FromArgs;
use chrono::Utc;
use core_elf_note::iterate_elf_notes;
use eyre::{eyre, Result, WrapErr};
use flate2::write::GzEncoder;
use kernlog::KernelLog;
use log::{debug, error, info, warn, LevelFilter, Log};
use prctl::set_dumpable;
use stack_unwinder::{EhFrameFinderImpl, UnwindHandler};
use std::io::BufWriter;
use std::path::Path;
use std::thread::scope;
use std::{cmp::max, io::BufReader};
use std::{cmp::min, fs::File};
use std::{
    env::{set_var, var},
    sync::mpsc::SyncSender,
};
use std::{io::Write, sync::mpsc::sync_channel};
use uuid::Uuid;

#[cfg(target_pointer_width = "64")]
pub use goblin::elf64 as elf;

#[cfg(target_pointer_width = "64")]
pub type ElfPtrSize = u64;

#[cfg(target_pointer_width = "32")]
pub use goblin::elf32 as elf;

use super::MemfaultdClient;

#[cfg(target_pointer_width = "32")]
pub type ElfPtrSize = u32;

#[derive(FromArgs)]
/// Accepts a kernel-generated core.elf from stdin and processes it.
/// This is intended to be called by the kernel as the coredump handler. It is not intended to be
/// called by users directly. memfaultd is expected to set up the handler with the kernel by writing
/// the appropriate configuration to /proc/sys/kernel/core_pattern.
/// See https://mflt.io/linux-coredumps for more information.
/// `core` man page:
/// https://man7.org/linux/man-pages/man5/core.5.html
struct MemfaultCoreHandlerArgs {
    /// use configuration file
    #[argh(option, short = 'c')]
    config_file: Option<String>,

    #[argh(positional)]
    pid: i32,

    /// thread ID of the crashing thread.
    ///
    /// Populated by the %I in the core_pattern.
    #[argh(positional)]
    tid: i32,

    /// signal number that caused the crash
    ///
    /// Populated by the %s in the core_pattern
    #[argh(positional)]
    signal: i32,

    /// verbose output
    #[argh(switch, short = 'V')]
    verbose: bool,
}

pub fn main() -> Result<()> {
    // Disable coredumping of this process
    let dumpable_result = set_dumpable(false);

    let args: MemfaultCoreHandlerArgs = argh::from_env();

    let (core_handler_logs_tx, core_handler_logs_rx) = sync_channel(CAPTURE_LOG_CHANNEL_SIZE);
    let log_level = if args.verbose {
        LevelFilter::Trace
    } else {
        LevelFilter::Info
    };
    // When the kernel executes a core dump handler, the stdout/stderr go nowhere.
    // Let's log to the kernel log to aid debugging:
    init_kernel_logger(log_level, core_handler_logs_tx)?;

    if let Err(e) = dumpable_result {
        warn!("Failed to set dumpable: {}", e);
    };

    let config_path = args.config_file.as_ref().map(Path::new);
    let config =
        Config::read_from_system(config_path).wrap_err(eyre!("Unable to load configuration"))?;

    if !config.config_file.enable_data_collection {
        error!("Data collection disabled, not processing corefile");
        return Ok(());
    }

    let process_name = get_process_name(args.pid as u32)?;
    let (app_logs_tx, app_logs_rx) = sync_channel(1);
    let log_fetcher = CoreTransformerLogFetcher::new(core_handler_logs_rx, app_logs_rx);

    // Asynchronously notify memfaultd that a crash occurred and fetch any crash logs.
    scope(|s| {
        let p = process_name.clone();
        s.spawn(|| {
            let client = MemfaultdClient::from_config(&config);
            match client {
                Ok(client) => {
                    if let Err(e) = client.notify_crash(p) {
                        debug!("Failed to notify memfaultd of crash: {:?}", e);
                    }
                    debug!("Getting crash logs");
                    match client.get_crash_logs(Utc::now()) {
                        Ok(logs) => {
                            if let Err(e) = app_logs_tx.send(logs) {
                                debug!("Application logs channel rx already dropped: {:?}", e);
                            }
                        }
                        Err(e) => {
                            debug!("Failed to get crash logs: {:?}", e);
                        }
                    }
                }
                Err(e) => {
                    debug!("Failed to create memfaultd client: {:?}", e);
                }
            }
        });
        process_corefile(
            &config,
            args.pid,
            log_fetcher,
            args.tid,
            args.signal,
            process_name,
        )
        .wrap_err(format!("Error processing coredump for PID {}", args.pid))
    })
}

pub fn process_corefile(
    config: &Config,
    pid: i32,
    log_fetcher: CoreTransformerLogFetcher,
    tid: i32,
    signal: i32,
    process_name: String,
) -> Result<()> {
    let rate_limiter = if !config.config_file.enable_dev_mode {
        config.coredump_rate_limiter_file_path();
        let mut rate_limiter = PersistentRateLimiter::load(
            config.coredump_rate_limiter_file_path(),
            config.config_file.coredump.rate_limit_count,
            chrono::Duration::from_std(config.config_file.coredump.rate_limit_duration)?,
        )
        .with_context(|| {
            format!(
                "Unable to open coredump rate limiter {}",
                config.coredump_rate_limiter_file_path().display()
            )
        })?;
        if !rate_limiter.check() {
            info!("Coredumps limit reached, not processing corefile");
            return Ok(());
        }
        Some(rate_limiter)
    } else {
        None
    };

    let max_size = calculate_available_space(config)?;
    if max_size == 0 {
        error!("Not processing corefile, disk usage limits exceeded");
        return Ok(());
    }

    let mar_staging_path = config.mar_staging_path();
    let mar_builder = MarEntryBuilder::new(&mar_staging_path)?;
    let compression = config.config_file.coredump.compression;
    let capture_strategy = config.config_file.coredump.capture_strategy;

    let thread_filter_supported = coredump_thread_filter_supported();
    let transformer_options = CoreTransformerOptions {
        max_size,
        capture_strategy,
        thread_filter_supported,
    };

    let cmd_line_file_name = format!("/proc/{}/cmdline", pid);
    let mut cmd_line_file = File::open(cmd_line_file_name)?;
    let cmd_line = read_proc_cmdline(&mut cmd_line_file)?;

    let input_stream = ForwardOnlySeeker::new(BufReader::new(std::io::stdin()));
    let proc_maps = ProcMapsImpl::new(pid);
    let mut core_reader = CoreReaderImpl::new(input_stream)?;
    let proc_mem = proc_mem_stream(pid)?;

    match capture_strategy {
        CoredumpCaptureStrategy::Threads { .. } | CoredumpCaptureStrategy::KernelSelection => {
            let metadata = CoredumpMetadata::new(config, cmd_line);
            let output_file_name = generate_tmp_file_name(compression);
            let output_file_path = mar_builder.make_attachment_path_in_entry_dir(&output_file_name);
            let output_file = BufWriter::new(File::create(&output_file_path)?);
            let output_stream: Box<dyn Write> = match compression {
                CoredumpCompression::Gzip => {
                    Box::new(GzEncoder::new(output_file, DEFAULT_GZIP_COMPRESSION_LEVEL))
                }
                CoredumpCompression::None => Box::new(output_file),
            };
            let output_stream = StreamPositionTracker::new(output_stream);
            let core_writer = CoreWriterImpl::new(
                core_reader.elf_header(),
                output_stream,
                proc_mem_stream(pid)?,
            );
            let mut core_transformer = core_transformer::CoreTransformer::new(
                core_reader,
                core_writer,
                proc_mem,
                transformer_options,
                metadata,
                proc_maps,
                log_fetcher,
            )?;

            match core_transformer.run_transformer() {
                Ok(()) => {
                    info!("Successfully captured coredump");
                    let network_config = NetworkConfig::from(config);
                    let mar_entry = mar_builder
                        .set_metadata(Metadata::new_coredump(output_file_name, compression.into()))
                        .add_attachment(output_file_path)?
                        .save(&network_config)?;

                    debug!("Coredump MAR entry generated: {}", mar_entry.path.display());

                    if let Some(rate_limiter) = rate_limiter {
                        rate_limiter.save()?;
                    }

                    Ok(())
                }
                Err(e) => Err(eyre!(
                    "Failed to capture coredump from {}: {}",
                    process_name,
                    e
                )),
            }
        }
        CoredumpCaptureStrategy::Stacktrace => {
            if !stacktrace_supported() {
                return Err(eyre!(
                    "Stacktrace capture not supported on this architecture"
                ));
            }

            let output_file_name = "stacktrace.json.gz";
            let output_file_path = mar_builder.make_attachment_path_in_entry_dir(output_file_name);
            let output_file = BufWriter::new(File::create(&output_file_path)?);
            let output_stream = GzEncoder::new(output_file, DEFAULT_GZIP_COMPRESSION_LEVEL);

            let program_headers = core_reader.read_program_headers()?;
            let all_notes = core_reader.read_all_note_segments(&program_headers);
            let parsed_notes = all_notes
                .iter()
                .flat_map(|(_, data)| iterate_elf_notes(data))
                .collect::<Vec<_>>();

            let mut unwind_handler = UnwindHandler::new(proc_mem, output_stream);
            let eh_frame_finder = EhFrameFinderImpl::new(proc_maps)?;

            match unwind_handler.build_stacktrace(
                eh_frame_finder,
                &parsed_notes,
                cmd_line,
                log_fetcher.core_handler_logs_rx,
                tid,
                signal,
            ) {
                Ok(()) => {
                    info!("Successfully captured stacktrace");
                    let network_config = NetworkConfig::from(config);
                    let mar_entry = mar_builder
                        .set_metadata(Metadata::new_stacktrace(
                            output_file_name.to_string(),
                            Some(CompressionAlgorithm::Gzip),
                        ))
                        .add_attachment(output_file_path)?
                        .save(&network_config)?;

                    debug!(
                        "Stacktrace MAR entry generated: {}",
                        mar_entry.path.display()
                    );

                    if let Some(rate_limiter) = rate_limiter {
                        rate_limiter.save()?;
                    }

                    Ok(())
                }
                Err(e) => Err(eyre!("Failed to build stacktrace: {}", e)),
            }
        }
    }
}

fn generate_tmp_file_name(compression: CoredumpCompression) -> String {
    let id = Uuid::new_v4();
    let extension = match compression {
        CoredumpCompression::Gzip => "elf.gz",
        CoredumpCompression::None => "elf",
    };
    format!("core-{}.{}", id, extension)
}

fn calculate_available_space(config: &Config) -> Result<usize> {
    let min_headroom = config.tmp_dir_min_headroom();
    let available = get_disk_space(&config.tmp_dir())?;

    let has_headroom = available.exceeds(&min_headroom);
    if !has_headroom {
        return Ok(0);
    }
    Ok(min(
        (available.bytes - min_headroom.bytes) as usize,
        config.config_file.coredump.coredump_max_size,
    ))
}

impl From<CoredumpCompression> for CompressionAlgorithm {
    fn from(compression: CoredumpCompression) -> Self {
        match compression {
            CoredumpCompression::Gzip => CompressionAlgorithm::Gzip,
            CoredumpCompression::None => CompressionAlgorithm::None,
        }
    }
}

fn init_kernel_logger(level: LevelFilter, core_handler_logs_tx: SyncSender<String>) -> Result<()> {
    // kernlog::init() reads from the KERNLOG_LEVEL to set the level. There's no public interface
    // to set it otherwise, so: if this environment variable is not set, set it according to the
    // --verbose flag:
    if var("KERNLOG_LEVEL").is_err() {
        set_var("KERNLOG_LEVEL", level.as_str());
    }
    // We fallback to standard output if verbose mode is enabled or if kernel is not available.

    let logger: Box<dyn Log> = match KernelLog::from_env() {
        Ok(logger) => Box::new(logger),
        Err(_) => Box::new(cli::build_logger(level)),
    };

    let logger = Box::new(CoreHandlerLogWrapper::new(
        logger,
        core_handler_logs_tx,
        CAPTURE_LOG_MAX_LEVEL,
    ));
    log::set_boxed_logger(logger)
        .map_err(|e| eyre!("Failed to initialize kernel logger: {}", e))?;

    // Set the max log level to the max of the log level and the capture log level.
    // This is necessary because the log macros will completely disable calls if the level
    // is below the max level.
    let max_level = max(level, CAPTURE_LOG_MAX_LEVEL);
    log::set_max_level(max_level);

    Ok(())
}
