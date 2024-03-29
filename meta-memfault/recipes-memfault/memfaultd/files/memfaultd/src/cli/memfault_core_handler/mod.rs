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
mod find_dynamic;
mod find_elf_headers;
mod find_stack;
mod log_wrapper;
mod memory_range;
mod procfs;
mod r_debug;
#[cfg(test)]
mod test_utils;

use self::core_reader::CoreReaderImpl;
use self::core_writer::CoreWriterImpl;
use self::log_wrapper::CoreHandlerLogWrapper;
use self::procfs::{proc_mem_stream, read_proc_cmdline, ProcMapsImpl};
use self::{arch::coredump_thread_filter_supported, log_wrapper::CAPTURE_LOG_CHANNEL_SIZE};
use self::{core_elf_memfault_note::CoredumpMetadata, core_transformer::CoreTransformerOptions};
use crate::cli;
use crate::config::{Config, CoredumpCompression};
use crate::mar::manifest::{CompressionAlgorithm, Metadata};
use crate::mar::mar_entry_builder::MarEntryBuilder;
use crate::network::NetworkConfig;
use crate::util::disk_size::get_disk_space;
use crate::util::io::{ForwardOnlySeeker, StreamPositionTracker};
use crate::util::persistent_rate_limiter::PersistentRateLimiter;
use argh::FromArgs;
use eyre::{eyre, Result, WrapErr};
use flate2::write::GzEncoder;
use kernlog::KernelLog;
use log::{debug, error, info, warn, LevelFilter, Log};
use prctl::set_dumpable;
use std::io::BufWriter;
use std::path::Path;
use std::thread::scope;
use std::{cmp::min, fs::File};
use std::{
    env::{set_var, var},
    sync::mpsc::SyncSender,
};
use std::{io::BufReader, sync::mpsc::Receiver};
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
struct MemfaultCoreHandlerArgs {
    /// use configuration file
    #[argh(option, short = 'c')]
    config_file: Option<String>,

    #[argh(positional)]
    pid: i32,

    /// verbose output
    #[argh(switch, short = 'V')]
    verbose: bool,
}

pub fn main() -> Result<()> {
    // Disable coredumping of this process
    let dumpable_result = set_dumpable(false);

    let args: MemfaultCoreHandlerArgs = argh::from_env();

    let (capture_logs_tx, capture_logs_rx) = sync_channel(CAPTURE_LOG_CHANNEL_SIZE);
    let log_level = if args.verbose {
        LevelFilter::Trace
    } else {
        LevelFilter::Info
    };
    // When the kernel executes a core dump handler, the stdout/stderr go nowhere.
    // Let's log to the kernel log to aid debugging:
    init_kernel_logger(log_level, capture_logs_tx);

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

    // Asynchronously notify memfaultd that a crash occured
    scope(|s| {
        s.spawn(|| {
            if let Err(e) =
                MemfaultdClient::from_config(&config).and_then(|client| client.notify_crash())
            {
                debug!("Failed to notify memfaultd of crash: {:?}", e);
            }
        });
        process_corefile(&config, args.pid, capture_logs_rx)
            .wrap_err(format!("Error processing coredump for PID {}", args.pid))
    })
}

pub fn process_corefile(config: &Config, pid: i32, error_rx: Receiver<String>) -> Result<()> {
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
    let output_file_name = generate_tmp_file_name(compression);
    let output_file_path = mar_builder.make_attachment_path_in_entry_dir(&output_file_name);

    let cmd_line_file_name = format!("/proc/{}/cmdline", pid);
    let mut cmd_line_file = File::open(cmd_line_file_name)?;
    let cmd_line = read_proc_cmdline(&mut cmd_line_file)?;
    let metadata = CoredumpMetadata::new(config, cmd_line);
    let thread_filter_supported = coredump_thread_filter_supported();
    let transformer_options = CoreTransformerOptions {
        max_size,
        capture_strategy,
        thread_filter_supported,
    };

    let output_file = BufWriter::new(File::create(&output_file_path)?);
    let output_stream: Box<dyn Write> = match compression {
        CoredumpCompression::Gzip => {
            Box::new(GzEncoder::new(output_file, flate2::Compression::default()))
        }
        CoredumpCompression::None => Box::new(output_file),
    };
    let output_stream = StreamPositionTracker::new(output_stream);

    let input_stream = ForwardOnlySeeker::new(BufReader::new(std::io::stdin()));
    let proc_maps = ProcMapsImpl::new(pid);
    let core_reader = CoreReaderImpl::new(input_stream)?;
    let core_writer = CoreWriterImpl::new(
        core_reader.elf_header(),
        output_stream,
        proc_mem_stream(pid)?,
    );
    let mut core_transformer = core_transformer::CoreTransformer::new(
        core_reader,
        core_writer,
        proc_mem_stream(pid)?,
        transformer_options,
        metadata,
        proc_maps,
        error_rx,
    )?;

    match core_transformer.run_transformer() {
        Ok(()) => {
            info!("Successfully captured coredump");
            let network_config = NetworkConfig::from(config);
            let mar_entry = mar_builder
                .set_metadata(Metadata::new_coredump(output_file_name, compression.into()))
                .add_attachment(output_file_path)
                .save(&network_config)?;

            debug!("Coredump MAR entry generated: {}", mar_entry.path.display());

            if let Some(rate_limiter) = rate_limiter {
                rate_limiter.save()?;
            }

            Ok(())
        }
        Err(e) => Err(eyre!("Failed to capture coredump: {}", e)),
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

fn init_kernel_logger(level: LevelFilter, capture_logs_tx: SyncSender<String>) {
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

    let logger = Box::new(CoreHandlerLogWrapper::new(logger, capture_logs_tx));
    log::set_boxed_logger(logger).unwrap();
    log::set_max_level(level);
}
