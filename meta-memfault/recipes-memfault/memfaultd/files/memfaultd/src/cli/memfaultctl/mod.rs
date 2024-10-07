//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use argh::{FromArgs, TopLevelCommand};
use chrono::{DateTime, Utc};
use std::{path::Path, str::FromStr, time::Duration};

mod add_battery_reading;
mod config_file;
mod coredump;
mod export;
mod report_sync;
mod session;
mod sync;
mod write_attributes;

use crate::{
    cli::version::format_version,
    mar::{DeviceAttribute, ExportFormat, Metadata},
    metrics::{KeyedMetricReading, SessionName},
    reboot::{write_reboot_reason_and_reboot, RebootReason},
    service_manager::get_service_manager,
};
use crate::{mar::MarEntryBuilder, util::output_arg::OutputArg};

use crate::cli::init_logger;
use crate::cli::memfaultctl::add_battery_reading::add_battery_reading;
use crate::cli::memfaultctl::config_file::{set_data_collection, set_developer_mode};
use crate::cli::memfaultctl::coredump::{trigger_coredump, ErrorStrategy};
use crate::cli::memfaultctl::export::export;
use crate::cli::memfaultctl::report_sync::report_sync;
use crate::cli::memfaultctl::sync::sync;
use crate::cli::show_settings::show_settings;
use crate::config::Config;
use crate::network::NetworkConfig;
use eyre::{eyre, Context, Result};
use log::LevelFilter;

use self::session::{end_session, start_session};

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

    /// verbose output
    #[argh(switch, short = 'V')]
    verbose: bool,
}

/// Wrapper around argh to support flags acting as subcommands, like --version.
/// Inspired by https://gist.github.com/suluke/e0c672492126be0a4f3b4f0e1115d77c
pub struct WrappedArgs<T: FromArgs>(pub T);
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
                output: format_version(),
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
    Export(ExportArgs),
    Reboot(RebootArgs),
    RequestMetrics(RequestMetricsArgs),
    ShowSettings(ShowSettingsArgs),
    Synchronize(SyncArgs),
    TriggerCoredump(TriggerCoredumpArgs),
    WriteAttributes(WriteAttributesArgs),
    AddBatteryReading(AddBatteryReadingArgs),
    ReportSyncSuccess(ReportSyncSuccessArgs),
    ReportSyncFailure(ReportSyncFailureArgs),
    StartSession(StartSessionArgs),
    EndSession(EndSessionArgs),
    AddCustomDataRecording(AddCustomDataRecordingArgs),
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
/// export (and delete) memfault data
#[argh(subcommand, name = "export")]
pub struct ExportArgs {
    #[argh(switch, short = 'n')]
    /// do not delete the data from memfault mar_staging
    do_not_delete: bool,
    #[argh(option, short = 'o')]
    /// where to write the MAR data (or '-' for standard output)
    output: OutputArg,

    #[argh(option, short = 'f', default = "ExportFormat::Mar")]
    /// output format (mar, chunk or chunk-wrapped)
    format: ExportFormat,
}

#[derive(FromArgs)]
/// register reboot reason and call 'reboot'
#[argh(subcommand, name = "reboot")]
struct RebootArgs {
    /// a reboot reason ID from https://docs.memfault.com/docs/platform/reference-reboot-reason-ids
    #[argh(option)]
    reason: String,
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
/// Upload all pending data to Memfault now
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

#[derive(FromArgs)]
/// add a reading to memfaultd's battery metrics in format "[status string]:[0.0-100.0]".
#[argh(subcommand, name = "add-battery-reading")]
struct AddBatteryReadingArgs {
    // Valid status strings are "Charging", "Not charging", "Discharging", "Unknown", and "Full"
    // These are based off the values that can appear in /sys/class/power_supply/<supply_name>/status
    // See: https://www.kernel.org/doc/Documentation/ABI/testing/sysfs-class-power
    #[argh(positional)]
    reading_string: String,
}

#[derive(FromArgs)]
/// Report a successful sync for connectivity metrics
#[argh(subcommand, name = "report-sync-success")]
struct ReportSyncSuccessArgs {}

#[derive(FromArgs)]
/// Report a failed sync for connectivity metrics
#[argh(subcommand, name = "report-sync-failure")]
struct ReportSyncFailureArgs {}

#[derive(FromArgs)]
/// Begin a session and start capturing metrics for it
#[argh(subcommand, name = "start-session")]
struct StartSessionArgs {
    // session name (needs to be defined in memfaultd.conf)
    #[argh(positional)]
    session_name: SessionName,
    // List of metric key value pairs to write in the format <KEY=float ...>
    #[argh(positional)]
    readings: Vec<KeyedMetricReading>,
}

#[derive(FromArgs)]
/// End a session and dump its metrics to MAR staging directory
#[argh(subcommand, name = "end-session")]
struct EndSessionArgs {
    // session name (needs to be defined in memfaultd.conf)
    #[argh(positional)]
    session_name: SessionName,
    // List of metric  key value pairs to write in the format <KEY=float ...>
    #[argh(positional)]
    readings: Vec<KeyedMetricReading>,
}

#[derive(FromArgs)]
/// Add custom data recording to memfaultd
#[argh(subcommand, name = "add-custom-data-recording")]
struct AddCustomDataRecordingArgs {
    /// reason for the recording
    #[argh(positional)]
    reason: String,
    /// name of file to attach to the recording
    #[argh(positional)]
    file_name: String,
    /// MIME types of the file. Should be a space or comma separated list.
    #[argh(positional)]
    mime_types: Vec<String>,

    /// duration of the recording in milliseconds, defaults to 0
    #[argh(option, default = "0")]
    duration_ms: u64,
    /// start time of the recording. Expected in RFC3339 format eg.(2024-08-15T14:10:30.00Z)
    #[argh(option)]
    start_time: Option<DateTime<Utc>>,
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

pub fn main() -> Result<()> {
    let args: MemfaultctlArgs = from_env();

    init_logger(if args.verbose {
        LevelFilter::Trace
    } else {
        LevelFilter::Info
    })?;

    let config_path = args.config_file.as_ref().map(Path::new);
    let mut config = Config::read_from_system(config_path)?;
    let network_config = NetworkConfig::from(&config);
    let mar_staging_path = config.mar_staging_path();

    let service_manager = get_service_manager();

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
        MemfaultctlCommand::Export(args) => export(&config, &args).wrap_err("Error exporting data"),
        MemfaultctlCommand::Reboot(args) => {
            let reason = RebootReason::from_str(&args.reason)
                .wrap_err(eyre!("Failed to parse {}", args.reason))?;
            println!("Rebooting with reason {:?}", reason);
            write_reboot_reason_and_reboot(
                &config.config_file.reboot.last_reboot_reason_file,
                reason,
            )
        }
        MemfaultctlCommand::RequestMetrics(_) => sync(),
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
                MarEntryBuilder::new(&mar_staging_path)?
                    .set_metadata(Metadata::new_device_attributes(attributes))
                    .save(&network_config)
                    .map(|_entry| ())
            }
        }
        MemfaultctlCommand::AddBatteryReading(AddBatteryReadingArgs { reading_string }) => {
            add_battery_reading(&config, &reading_string)
        }
        MemfaultctlCommand::ReportSyncSuccess(_) => report_sync(&config, true),
        MemfaultctlCommand::ReportSyncFailure(_) => report_sync(&config, false),
        MemfaultctlCommand::StartSession(StartSessionArgs {
            session_name,
            readings,
        }) => start_session(&config, session_name, readings),
        MemfaultctlCommand::EndSession(EndSessionArgs {
            session_name,
            readings,
        }) => end_session(&config, session_name, readings),
        MemfaultctlCommand::AddCustomDataRecording(AddCustomDataRecordingArgs {
            reason,
            file_name,
            duration_ms,
            mime_types,
            start_time,
        }) => {
            check_data_collection_enabled(&config, "add custom data recording")?;

            let file_path = Path::new(&file_name).to_owned();
            if !file_path.is_file() {
                return Err(eyre!("{} does not exist", file_name));
            }
            if !file_path.is_absolute() {
                return Err(eyre!("{} is not an absolute path", file_name));
            }

            let file_name = file_name
                .trim()
                .split('/')
                .last()
                .ok_or_else(|| eyre!("{} is not a valid file path", file_name))?
                .to_string();

            MarEntryBuilder::new(&mar_staging_path)?
                .set_metadata(Metadata::new_custom_data_recording(
                    start_time,
                    Duration::from_millis(duration_ms),
                    mime_types,
                    reason,
                    file_name,
                ))
                .add_copied_attachment(file_path)?
                .save(&network_config)
                .map(|_entry| ())
        }
    }
}
