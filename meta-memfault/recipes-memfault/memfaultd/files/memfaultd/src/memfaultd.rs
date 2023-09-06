//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::{fs::create_dir_all, time::Instant};

use eyre::Result;
use eyre::{eyre, Context};
use log::{error, info, trace, warn};

use crate::network::{NetworkClientImpl, NetworkConfig};
use crate::util::task::{loop_with_exponential_error_backoff, LoopContinuation};
use crate::util::UpdateStatus;
use crate::{config::Config, mar::upload::collect_and_upload};
use crate::{mar::MarStagingCleaner, service_manager::get_service_manager};
use crate::{reboot::RebootReasonTracker, util::disk_size::DiskSize};

#[cfg(feature = "collectd")]
use crate::collectd::CollectdServer;

#[cfg(feature = "logging")]
use crate::{
    fluent_bit::{FluentBitConfig, FluentBitConnectionHandler},
    logs::{CompletedLog, FluentBitAdapter, HeadroomLimiter, LogCollector, LogCollectorConfig},
    mar::{MarEntryBuilder, Metadata},
    util::disk_size::get_disk_space,
};

const CONFIG_REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 120);

#[derive(PartialEq, Eq)]
pub enum MemfaultLoopResult {
    Terminate,
    Relaunch,
}

pub fn memfaultd_loop<C: Fn() -> Result<()>>(
    config: Config,
    ready_callback: C,
) -> Result<MemfaultLoopResult> {
    // Register a flag which will be set when one of these signals is received.
    let term_signals = [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM];
    let term = Arc::new(AtomicBool::new(false));
    for signal in term_signals {
        signal_hook::flag::register(signal, Arc::clone(&term))?;
    }

    // This flag will be set when we get the SIGHUP signal to reload (currently reload = restart)
    let reload = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&reload))?;

    // Register a flag to be set when we are woken up by SIGUSR1
    let force_sync = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGUSR1, Arc::clone(&force_sync))?;

    // Load configuration and device information. This has already been done by the C code but
    // we are preparing for a future where there is no more C code.
    let client = NetworkClientImpl::new(NetworkConfig::from(&config))
        .wrap_err(eyre!("Unable to prepare network client"))?;

    let service_manager = get_service_manager();

    // Make sure the MAR staging area exists
    create_dir_all(config.mar_staging_path()).wrap_err_with(|| {
        eyre!(
            "Unable to create MAR staging area {}",
            &config.mar_staging_path().display(),
        )
    })?;

    let mar_cleaner = Arc::new(MarStagingCleaner::new(
        &config.mar_staging_path(),
        config.tmp_dir_max_size(),
        config.tmp_dir_min_headroom(),
    ));

    // List of tasks to run before syncing with server
    let mut sync_tasks: Vec<Box<dyn FnMut(bool) -> Result<()>>> = vec![];
    // List of tasks to run before shutting down
    let mut shutdown_tasks: Vec<Box<dyn FnMut() -> Result<()>>> = vec![];

    #[cfg(feature = "collectd")]
    {
        use std::thread::{sleep, spawn};

        let mut collectd_server = CollectdServer::new();
        collectd_server.start(
            config.config_file.enable_data_collection,
            config.config_file.http_server.bind_address,
        )?;

        // Start a thread to dump the metrics precisely every 60 minutes
        {
            let net_config = NetworkConfig::from(&config);
            let mar_staging_path = config.mar_staging_path();
            let heartbeat_interval = config.config_file.heartbeat_interval;

            let mut collectd_server = collectd_server.clone();

            spawn(move || {
                let mut next_heartbeat = Instant::now() + heartbeat_interval;
                loop {
                    while Instant::now() < next_heartbeat {
                        sleep(next_heartbeat - Instant::now());
                    }
                    next_heartbeat += heartbeat_interval;
                    if let Err(e) =
                        collectd_server.dump_metrics_to_mar_entry(&mar_staging_path, &net_config)
                    {
                        warn!("Unable to dump metrics: {}", e);
                    }
                }
            });
        }
        // Schedule a task to dump the metrics when a sync is forced
        {
            let net_config = NetworkConfig::from(&config);
            let mar_staging_path = config.mar_staging_path();

            let mut collectd_server = collectd_server.clone();

            sync_tasks.push(Box::new(move |forced| {
                if forced {
                    collectd_server.dump_metrics_to_mar_entry(&mar_staging_path, &net_config)?;
                }
                Ok(())
            }));
        }
        // Schedule a task to dump the metrics when we are shutting down
        {
            let net_config = NetworkConfig::from(&config);
            let mar_staging_path = config.mar_staging_path();

            shutdown_tasks.push(Box::new(move || {
                collectd_server.dump_metrics_to_mar_entry(&mar_staging_path, &net_config)
            }));
        }
    }

    #[cfg(feature = "logging")]
    {
        use log::debug;

        let fluent_bit_config = FluentBitConfig::from(&config);
        if config.config_file.enable_data_collection {
            let (_, fluent_bit_receiver) = FluentBitConnectionHandler::start(fluent_bit_config)?;
            let mar_cleaner = mar_cleaner.clone();

            let network_config = NetworkConfig::from(&config);
            let mar_staging_path = config.mar_staging_path();
            let on_log_completion = move |CompletedLog {
                                              path,
                                              cid,
                                              next_cid,
                                              compression,
                                          }|
                  -> Result<()> {
                // Prepare the MAR entry
                let file_name = path
                    .file_name()
                    .ok_or(eyre!("Logfile should be a file."))?
                    .to_str()
                    .ok_or(eyre!("Invalid log filename."))?
                    .to_owned();
                let mar_builder = MarEntryBuilder::new(&mar_staging_path)?
                    .set_metadata(Metadata::new_log(file_name, cid, next_cid, compression))
                    .add_attachment(path);

                mar_cleaner.clean(mar_builder.estimated_entry_size())?;

                // Move the log in the mar_staging area and add a manifest
                let mar_entry = mar_builder.save(&network_config)?;
                debug!("New MAR entry generated: {}", mar_entry.path.display());

                Ok(())
            };
            let log_config = LogCollectorConfig::from(&config);
            let headroom_limiter = {
                let tmp_folder = log_config.log_tmp_path.clone();
                HeadroomLimiter::new(config.tmp_dir_min_headroom(), move || {
                    get_disk_space(&tmp_folder)
                })
            };
            let mut log_collector =
                LogCollector::open(log_config, on_log_completion, headroom_limiter)?;
            log_collector.spawn_collect_from(FluentBitAdapter::new(
                fluent_bit_receiver,
                &config.config_file.fluent_bit.extra_fluentd_attributes,
            ));

            sync_tasks.push(Box::new(move |forced_sync| {
                // Check if we have received a signal to force-sync and reset the flag.
                if forced_sync {
                    trace!("Flushing logs");
                    log_collector.flush_logs()?;
                } else {
                    // If not force-flushing - we still want to make sure this file
                    // did not get too old.
                    log_collector.rotate_if_needed()?;
                }
                Ok(())
            }));
        } else {
            FluentBitConnectionHandler::start_null(fluent_bit_config)?;
        }
    }

    let reboot_tracker = RebootReasonTracker::new(&config, &service_manager);
    if let Err(e) = reboot_tracker.track_reboot() {
        error!("Unable to track reboot reason: {:#}", e);
    }

    ready_callback()?;

    let mut last_device_config_refresh = Option::<Instant>::None;

    // If upload_interval is zero, we are only uploading on manual syncs.
    let forced_sync_only = config.config_file.upload_interval.is_zero();
    // If we are only uploading on manual syncs, we still need to run the mar cleaner periodically. In
    // this case set the the upload interval to 15 minutes.
    let upload_interval = if forced_sync_only {
        Duration::from_secs(60 * 15)
    } else {
        config.config_file.upload_interval
    };
    loop_with_exponential_error_backoff(
        || {
            // Reset the forced sync flag before doing any work so we can detect
            // if it's set again while we run and RerunImmediately.
            let forced = force_sync.swap(false, Ordering::Relaxed);
            let enable_data_collection = config.config_file.enable_data_collection;

            // Refresh device config if needed. In cases where we are only syncing on demand, we
            // short-circuit this check.
            if enable_data_collection
                && (!forced_sync_only
                    && (last_device_config_refresh.is_none()
                        || last_device_config_refresh.unwrap() + CONFIG_REFRESH_INTERVAL
                            < Instant::now())
                    || forced)
            {
                // Refresh device config from the server
                match config.refresh_device_config(&client) {
                    Err(e) => {
                        warn!("Unable to refresh device config: {}", e);
                        // We continue processing the pending uploads on errors.
                        // We expect rate limiting errors here.
                    }
                    Ok(UpdateStatus::Updated) => {
                        info!("Device config updated");
                        last_device_config_refresh = Some(Instant::now())
                    }
                    Ok(UpdateStatus::Unchanged) => {
                        trace!("Device config unchanged");
                        last_device_config_refresh = Some(Instant::now())
                    }
                }
            }

            for task in &mut sync_tasks {
                task(forced)?;
            }

            mar_cleaner.clean(DiskSize::ZERO).unwrap();

            if enable_data_collection && !forced_sync_only || forced {
                trace!("Collect MAR entries...");
                collect_and_upload(
                    &config.mar_staging_path(),
                    &client,
                    config.config_file.mar.mar_file_max_size,
                    config.sampling(),
                )?;
            }
            Ok(())
        },
        || match (
            term.load(Ordering::Relaxed) || reload.load(Ordering::Relaxed),
            force_sync.load(Ordering::Relaxed),
        ) {
            // Stop when we receive a term signal
            (true, _) => LoopContinuation::Stop,
            // If we received a SIGUSR1 signal while we were in the loop, rerun immediately.
            (false, true) => LoopContinuation::RerunImmediately,
            // Otherwise, keep runnin normally
            (false, false) => LoopContinuation::KeepRunning,
        },
        upload_interval,
        Duration::new(60, 0),
    );
    info!("Memfaultd shutting down...");
    for task in &mut shutdown_tasks {
        if let Err(e) = task() {
            warn!("Error while shutting down: {}", e);
        }
    }

    if reload.load(Ordering::Relaxed) {
        Ok(MemfaultLoopResult::Relaunch)
    } else {
        Ok(MemfaultLoopResult::Terminate)
    }
}
