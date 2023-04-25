//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use argh::{FromArgs, TopLevelCommand};
use memfaultc_sys::{cmd_reboot, cmd_request_metrics};
use std::ffi::CString;
use std::path::Path;

mod config_file;
mod coredump;
mod show_settings;
mod sync;
mod write_attributes;

use crate::mar::manifest::{DeviceAttribute, Metadata};
use crate::mar::mar_entry::MarEntryBuilder;

use crate::build_info::{BUILD_ID, GIT_COMMIT, VERSION};
use crate::cli::memfaultctl::config_file::{set_data_collection, set_developer_mode};
use crate::cli::memfaultctl::coredump::{trigger_coredump, ErrorStrategy};
use crate::cli::memfaultctl::show_settings::show_settings;
use crate::cli::memfaultctl::sync::sync;
use crate::config::Config;
use crate::network::NetworkConfig;
use crate::service::SystemdServiceManager;
use eyre::{eyre, Result, WrapErr};

#[derive(FromArgs)]
/// A command line utility to adjust memfaultd configuration and trigger specific events for
/// testing purposes. For further reference, see:
/// https://docs.memfault.com/docs/linux/reference-memfaultctl-cli
struct MemfaultctlArgs {
    #[argh(subcommand)]
    command: MemfaultctlCommand,

    /// use configuration file
    #[argh(option, short = 'c')]
    config_file: Option<String>,

    /// show version information
    #[argh(switch, short = 'v')]
    #[allow(dead_code)]
    version: bool,
}

/// Wrapper around argh to support flags acting as subcommands, like --version.
/// Inspired by https://gist.github.com/suluke/e0c672492126be0a4f3b4f0e1115d77c
struct WrappedArgs<T: FromArgs>(T);
impl<T: FromArgs> TopLevelCommand for WrappedArgs<T> {}
impl<T: FromArgs> FromArgs for WrappedArgs<T> {
    fn from_args(command_name: &[&str], args: &[&str]) -> Result<Self, argh::EarlyExit> {
        /// Pseudo subcommands that look like flags.
        #[derive(FromArgs)]
        struct CommandlikeFlags {
            /// show version information
            #[argh(switch, short = 'v')]
            version: bool,
        }

        match CommandlikeFlags::from_args(command_name, args) {
            Ok(CommandlikeFlags { version: true }) => Err(argh::EarlyExit {
                output: format!(
                    "VERSION={}\nGIT COMMIT={}\nBUILD ID={}",
                    VERSION, GIT_COMMIT, BUILD_ID
                ),
                status: Ok(()),
            }),
            _ => T::from_args(command_name, args).map(Self),
        }
    }
}

pub fn from_env<T: TopLevelCommand>() -> T {
    argh::from_env::<WrappedArgs<T>>().0
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum MemfaultctlCommand {
    EnableDataCollection(EnableDataCollectionArgs),
    DisableDataCollection(DisableDataCollectionArgs),
    EnableDevMode(EnableDevModeArgs),
    DisableDevMode(DisableDevModeArgs),
    Reboot(RebootArgs),
    RequestMetrics(RequestMetricsArgs),
    ShowSettings(ShowSettingsArgs),
    Synchronize(SyncArgs),
    TriggerCoredump(TriggerCoredumpArgs),
    WriteAttributes(WriteAttributesArgs),
}

#[derive(FromArgs)]
/// enable data collection and restart memfaultd
#[argh(subcommand, name = "enable-data-collection")]
struct EnableDataCollectionArgs {}

#[derive(FromArgs)]
/// disable data collection and restart memfaultd
#[argh(subcommand, name = "disable-data-collection")]
struct DisableDataCollectionArgs {}

#[derive(FromArgs)]
/// enable developer mode and restart memfaultd
#[argh(subcommand, name = "enable-dev-mode")]
struct EnableDevModeArgs {}

#[derive(FromArgs)]
/// disable developer mode and restart memfaultd
#[argh(subcommand, name = "disable-dev-mode")]
struct DisableDevModeArgs {}

#[derive(FromArgs)]
/// register reboot reason and call 'reboot'
#[argh(subcommand, name = "reboot")]
struct RebootArgs {
    /// a reboot reason ID from https://docs.memfault.com/docs/platform/reference-reboot-reason-ids
    #[argh(option, default = "0")]
    reason: u32,
}

#[derive(FromArgs)]
/// flush collectd metrics to Memfault now
#[argh(subcommand, name = "request-metrics")]
struct RequestMetricsArgs {}

#[derive(FromArgs)]
/// show memfaultd settings
#[argh(subcommand, name = "show-settings")]
struct ShowSettingsArgs {}

#[derive(FromArgs)]
/// flush memfaultd queue to Memfault now
#[argh(subcommand, name = "sync")]
struct SyncArgs {}

#[derive(FromArgs)]
/// trigger a coredump and immediately reports it to Memfault (defaults to segfault)
#[argh(subcommand, name = "trigger-coredump")]
struct TriggerCoredumpArgs {
    /// a strategy, either 'segfault' or 'divide-by-zero'
    #[argh(positional, default = "ErrorStrategy::SegFault")]
    strategy: ErrorStrategy,
}

#[derive(FromArgs)]
/// write device attribute(s) to memfaultd
#[argh(subcommand, name = "write-attributes")]
struct WriteAttributesArgs {
    /// attributes to write, in the format <VAR1=VAL1 ...>
    #[argh(positional)]
    attributes: Vec<DeviceAttribute>,
}

fn check_c_result(rv: i32) -> Result<()> {
    if rv == 0 {
        Ok(())
    } else {
        Err(eyre!("Error code {}", rv))
    }
}

fn check_data_collection_enabled(config: &Config, do_what: &str) -> Result<()> {
    match config.config_file.enable_data_collection {
        true => Ok(()),
        false => {
            let msg = format!(
                "Cannot {} because data collection is disabled. \
                Hint: enable it with 'memfaultctl enable-data-collection'.",
                do_what
            );
            Err(eyre!(msg))
        }
    }
}

fn main_impl() -> Result<()> {
    let args: MemfaultctlArgs = from_env();

    let config_file_cstring = args.config_file.as_ref().map(|config_file| {
        CString::new(config_file.as_str()).expect("No NULs in config_file string.")
    });

    let config_file_cstring_ptr = config_file_cstring
        .as_ref()
        .map_or(std::ptr::null(), |cstring| cstring.as_ptr());

    let config_path = args.config_file.as_ref().map(Path::new);
    let mut config =
        Config::read_from_system(config_path).wrap_err(eyre!("Unable to load configuration"))?;
    let network_config = NetworkConfig::from(&config);
    let mar_staging_path = config.mar_staging_path();
    // TODO MFLT-9693: Add support for other service managers
    let service_manager = SystemdServiceManager;

    match args.command {
        MemfaultctlCommand::EnableDataCollection(_) => {
            set_data_collection(&mut config, &service_manager, true)
        }
        MemfaultctlCommand::DisableDataCollection(_) => {
            set_data_collection(&mut config, &service_manager, false)
        }
        MemfaultctlCommand::EnableDevMode(_) => {
            set_developer_mode(&mut config, &service_manager, true)
        }
        MemfaultctlCommand::DisableDevMode(_) => {
            set_developer_mode(&mut config, &service_manager, false)
        }
        MemfaultctlCommand::Reboot(cargs) => unsafe {
            check_c_result(cmd_reboot(
                config_file_cstring_ptr,
                cargs.reason as libc::c_int,
            ))
        },
        MemfaultctlCommand::RequestMetrics(_) => unsafe { check_c_result(cmd_request_metrics()) },
        MemfaultctlCommand::ShowSettings(_) => show_settings(config_path),
        MemfaultctlCommand::Synchronize(_) => sync(),
        MemfaultctlCommand::TriggerCoredump(TriggerCoredumpArgs { strategy }) => {
            trigger_coredump(&config, strategy)
        }
        MemfaultctlCommand::WriteAttributes(WriteAttributesArgs { attributes }) => {
            // argh does not have a way to specify the minimum number of repeating arguments, so check here:
            // https://github.com/google/argh/issues/110
            if attributes.is_empty() {
                Err(eyre!(
                    "No attributes given. Please specify them as KEY=VALUE pairs."
                ))
            } else {
                check_data_collection_enabled(&config, "write attributes")?;
                MarEntryBuilder::new(Metadata::new_device_attributes(attributes), vec![])?
                    .save(&mar_staging_path, &network_config)
                    .map(|_entry| ())
            }
        }
    }
}

pub fn main() -> i32 {
    match main_impl() {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("{}", e);
            -1
        }
    }
}
