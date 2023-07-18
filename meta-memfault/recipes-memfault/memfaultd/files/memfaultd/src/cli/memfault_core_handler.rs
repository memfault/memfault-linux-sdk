//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use crate::mar::manifest::{CompressionAlgorithm, Metadata};
use crate::mar::mar_entry_builder::MarEntryBuilder;
use crate::network::NetworkConfig;
use crate::util::disk_size::get_disk_space;
use crate::{
    build_info::VERSION,
    config::{Config, CoredumpCompression},
};
use argh::FromArgs;
use eyre::{eyre, Result, WrapErr};
use libc::STDIN_FILENO;
use log::{debug, error, info, warn};
use memfaultc_sys::coredump::{
    core_elf_process_fd, coredump_check_rate_limiter, MemfaultProcessCoredumpCtx,
};
use prctl::set_dumpable;
use std::cmp::min;
use std::ffi::CString;
use std::os::raw::c_int;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use uuid::Uuid;

use super::init_logger;

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

    // When the kernel executes a core dump handler, the stdout/stderr go nowhere.
    // Let's log to the kernel log to aid debugging:
    // We fallback to standard output if verbose mode is enabled or if kernel is not available.
    if args.verbose {
        init_logger(args.verbose);
    } else if let Err(e) = kernlog::init() {
        warn!("Cannot log to kernel logs, falling back to stderr: {}", e);
        init_logger(false);
    }

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

    if !config.config_file.enable_dev_mode {
        let rate_limiter_file_c_string = CString::new(
            config
                .coredump_rate_limiter_file_path()
                .into_os_string()
                .as_bytes(),
        )
        .expect("No NULs in rate limiter file string.");
        if !unsafe {
            coredump_check_rate_limiter(
                rate_limiter_file_c_string.as_ptr(),
                config.config_file.coredump.rate_limit_count as c_int,
                config.config_file.coredump.rate_limit_duration.as_secs() as c_int,
            )
        } {
            error!("Limit reached, not processing corefile");
            return Ok(());
        }
    }

    let max_size = calculate_available_space(&config)?;
    if max_size == 0 {
        error!("Not processing corefile, disk usage limits exceeded");
        return Ok(());
    }

    let compression = config.config_file.coredump.compression;

    let mar_staging_path = config.mar_staging_path();
    let mar_builder = MarEntryBuilder::new(&mar_staging_path)?;
    let output_file_name = generate_tmp_file_name(compression);
    let output_file_path = mar_builder.make_attachment_path_in_entry_dir(&output_file_name);

    let device_id_c_string =
        CString::new(config.device_info.device_id.as_str()).expect("No NULs device id string.");
    let hardware_version_c_string = CString::new(config.device_info.hardware_version.as_str())
        .expect("No NULs in hardware version string.");
    let software_type_c_string = CString::new(config.config_file.software_type.as_str())
        .expect("No NULs in software type string.");
    let software_version_c_string = CString::new(config.config_file.software_version.as_str())
        .expect("No NULs in software version string.");
    let sdk_version_c_string = CString::new(VERSION).expect("No NULs in sdk_version");
    let output_file_c_string = CString::new(output_file_path.clone().into_os_string().as_bytes())
        .expect("No NULs in output file string.");

    let ctx = MemfaultProcessCoredumpCtx {
        input_fd: STDIN_FILENO,
        pid: args.pid,
        device_id: device_id_c_string.as_ptr(),
        hardware_version: hardware_version_c_string.as_ptr(),
        software_type: software_type_c_string.as_ptr(),
        software_version: software_version_c_string.as_ptr(),
        sdk_version: sdk_version_c_string.as_ptr(),
        output_file: output_file_c_string.as_ptr(),
        max_size,
        gzip_enabled: matches!(compression, CoredumpCompression::Gzip),
    };

    if unsafe { core_elf_process_fd(&ctx) } {
        info!("Successfully captured coredump");
        let network_config = NetworkConfig::from(&config);
        let mar_entry = mar_builder
            .set_metadata(Metadata::new_coredump(output_file_name, compression.into()))
            .add_attachment(output_file_path)
            .save(&network_config)?;

        debug!("New MAR entry generated: {}", mar_entry.path.display());

        Ok(())
    } else {
        Err(eyre!("Failed to capture coredump"))
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
