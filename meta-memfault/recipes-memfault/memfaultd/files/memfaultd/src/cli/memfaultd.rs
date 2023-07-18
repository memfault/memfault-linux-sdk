//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{fs::create_dir_all, path::Path};

use crate::{
    config::Config,
    memfaultd::memfaultd_loop,
    util::pid_file::{get_pid_from_file, remove_pid_file},
};

use eyre::{eyre, Context, Result};
use log::{info, warn};

use crate::cli::show_settings::show_settings;
use crate::cli::version::format_version;
use argh::FromArgs;

use super::init_logger;

#[derive(FromArgs)]
/// Memfault daemon.
struct MemfaultDArgs {
    /// use configuration file
    #[argh(option, short = 'c')]
    config_file: Option<String>,

    #[argh(switch, short = 's')]
    /// show settings and exit immediately
    show_settings: bool,

    #[argh(switch, short = 'Z')]
    /// daemonize (fork to background)
    daemonize: bool,

    #[argh(switch, short = 'v')]
    /// show version
    version: bool,

    #[argh(switch, short = 'V')]
    /// verbose output
    verbose: bool,
}

pub fn main() -> Result<()> {
    let args: MemfaultDArgs = argh::from_env();
    let config_path = args.config_file.as_ref().map(Path::new);

    init_logger(args.verbose);

    if args.version {
        println!("{}", format_version());
        return Ok(());
    }

    let config =
        Config::read_from_system(config_path).wrap_err(eyre!("Unable to load configuration"))?;

    // Create directories early so we can fail early if we can't create them.
    mkdir_if_needed(&config.config_file.persist_dir)?;
    mkdir_if_needed(&config.tmp_dir())?;

    // Always show settings when starting
    show_settings(config_path)?;

    if args.show_settings {
        // Already printed above. We're done.
        return Ok(());
    }

    if !args.daemonize && get_pid_from_file().is_ok() {
        return Err(eyre!("memfaultd is already running"));
    }

    if config.config_file.enable_dev_mode {
        info!("memfaultd:: Starting with developer mode enabled");
    }
    if !config.config_file.enable_data_collection {
        warn!("memfaultd:: Starting with data collection DISABLED");
    }

    #[cfg(feature = "swupdate")]
    {
        use crate::service_manager::{
            get_service_manager, MemfaultdService, MemfaultdServiceManager,
        };
        use crate::swupdate::generate_swupdate_config;

        generate_swupdate_config(&config)?;
        get_service_manager().restart_service_if_running(MemfaultdService::SWUpdate)?;
        get_service_manager().restart_service_if_running(MemfaultdService::SwUpdateSocket)?;
    }
    #[cfg(feature = "coredump")]
    {
        use crate::coredump::coredump_configure_kernel;
        if config.config_file.enable_data_collection {
            coredump_configure_kernel(&config.config_file_path)?;
        }
    }

    if args.daemonize {
        daemonize()?;
    }

    memfaultd_loop(config, args.daemonize)?;

    if args.daemonize {
        remove_pid_file()?
    }
    Ok(())
}

fn mkdir_if_needed(path: &Path) -> Result<()> {
    if path.exists() && path.is_dir() {
        return Ok(());
    }
    create_dir_all(path).wrap_err(eyre!("Unable to create directory {}", path.display()))
}

fn daemonize() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        nix::unistd::daemon(false, true).wrap_err("Unable to daemonize")
    }
    #[cfg(not(target_os = "linux"))]
    {
        warn!("Daemonizing is not supported on this platform");
        Ok(())
    }
}
