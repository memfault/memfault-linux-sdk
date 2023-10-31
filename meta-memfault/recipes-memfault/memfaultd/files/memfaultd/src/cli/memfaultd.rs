//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::{
    env::args_os, fs::create_dir_all, os::unix::process::CommandExt, path::Path, process::Command,
};

use crate::{
    config::Config,
    memfaultd::{memfaultd_loop, MemfaultLoopResult},
    util::pid_file::{get_pid_from_file, is_pid_file_about_me, remove_pid_file, write_pid_file},
};

use eyre::{eyre, Context, Result};
use log::{error, info, warn, LevelFilter};

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

    #[argh(switch, short = 'q')]
    /// quiet - no output
    quiet: bool,
}

pub fn main() -> Result<()> {
    let args: MemfaultDArgs = argh::from_env();
    let config_path = args.config_file.as_ref().map(Path::new);

    init_logger(match (args.quiet, args.verbose) {
        (true, _) => LevelFilter::Off,
        (false, true) => LevelFilter::Trace,
        _ => LevelFilter::Info,
    });

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
        use crate::swupdate::generate_swupdate_config;

        generate_swupdate_config(&config)?;
    }
    #[cfg(feature = "coredump")]
    {
        use crate::coredump::coredump_configure_kernel;
        if config.config_file.enable_data_collection {
            coredump_configure_kernel(&config.config_file_path)?;
        }
    }

    // Only daemonize when asked to AND not already running (aka don't fork when reloading)
    let need_daemonize = args.daemonize && !is_pid_file_about_me();
    if need_daemonize {
        daemonize()?;
    }

    let result = memfaultd_loop(config, || {
        if need_daemonize {
            // All subcomponents are ready, write the pid file now to indicate we've started up completely.
            write_pid_file()?;
        }
        Ok(())
    })?;
    if result == MemfaultLoopResult::Relaunch {
        // If reloading the config, execv ourselves (replace our program in memory by a new copy of us)
        let mut args = args_os().collect::<Vec<_>>();
        let arg0 = args.remove(0);

        let err = Command::new(arg0).args(&args).exec();
        // This next line will only be executed if we failed to exec().
        error!("Unable to restart {:?}: {:?}", args, err);
    };

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
        nix::unistd::daemon(true, true).wrap_err("Unable to daemonize")
    }
    #[cfg(not(target_os = "linux"))]
    {
        warn!("Daemonizing is not supported on this platform");
        Ok(())
    }
}
