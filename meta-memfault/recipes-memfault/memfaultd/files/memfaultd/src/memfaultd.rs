//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::process::Command;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::thread::{sleep, spawn};
use std::time::Duration;
use std::{fs::create_dir_all, time::Instant};

use eyre::Result;
use eyre::{eyre, Context};
use log::{error, info, trace, warn};

use ssf::{Scheduler, ServiceThread};

use crate::metrics::{
    BatteryMonitor, BatteryReadingHandler, ConnectivityMonitor, DumpHrtMessage,
    DumpMetricReportMessage, KeyedMetricReading, MetricReportType, MetricsMBox,
    ReportSyncEventHandler, ReportsToDump, SessionEventHandler, SystemMetricsCollector,
};
use crate::{config::Resolution, metrics::MetricsEventHandler};

use crate::{
    config::Config,
    mar::upload::collect_and_upload,
    metrics::{
        core_metrics::{METRIC_MF_SYNC_FAILURE, METRIC_MF_SYNC_SUCCESS},
        CrashFreeIntervalTracker, MetricReportManager, StatsDServer,
    },
};
use crate::{http_server::HttpHandler, util::UpdateStatus};
use crate::{
    http_server::HttpServer,
    network::{NetworkClientImpl, NetworkConfig},
};
use crate::{
    mar::MarExportHandler,
    util::{
        can_connect::TcpConnectionChecker,
        task::{loop_with_exponential_error_backoff, LoopContinuation},
    },
};
use crate::{mar::MarStagingCleaner, service_manager::get_service_manager};
use crate::{reboot::RebootReasonTracker, util::disk_size::DiskSize};

use crate::collectd::CollectdHandler;

#[cfg(feature = "logging")]
use crate::{
    fluent_bit::{FluentBitConfig, FluentBitConnectionHandler},
    logs::{CompletedLog, FluentBitAdapter, HeadroomLimiter, LogCollector, LogCollectorConfig},
    mar::{MarEntryBuilder, Metadata},
    util::disk_size::get_disk_space,
};

const CONFIG_REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 120);
const DAILY_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60 * 60 * 24);

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
        config.mar_entry_max_age(),
    ));

    // List of tasks to run before syncing with server
    let mut sync_tasks: Vec<Box<dyn FnMut(bool) -> Result<()>>> = vec![];
    // List of tasks to run before shutting down
    let mut shutdown_tasks: Vec<Box<dyn FnMut() -> Result<()>>> = vec![];

    // List of http handlers
    #[allow(unused_mut, /* reason = "Can be unused when some features are disabled." */)]
    let mut http_handlers: Vec<Box<dyn HttpHandler>> =
        vec![Box::new(MarExportHandler::new(config.mar_staging_path()))];

    // Metric store
    let metric_report_manager = ServiceThread::spawn_with(match config.session_configs() {
        Some(session_configs) => MetricReportManager::new_with_session_configs(
            config.hrt_enabled(),
            config.hrt_max_samples_per_min(),
            session_configs,
        ),
        None => MetricReportManager::new(config.hrt_enabled(), config.hrt_max_samples_per_min()),
    });
    let metrics_mbox: MetricsMBox = metric_report_manager.mbox().into();

    let battery_monitor = Arc::new(Mutex::new(BatteryMonitor::<Instant>::new(
        metrics_mbox.clone(),
    )));
    let battery_reading_handler = BatteryReadingHandler::new(
        config.config_file.enable_data_collection,
        battery_monitor.clone(),
    );
    http_handlers.push(Box::new(battery_reading_handler));

    let report_sync_event_handler = ReportSyncEventHandler::new(
        config.config_file.enable_data_collection,
        metrics_mbox.clone(),
    );
    http_handlers.push(Box::new(report_sync_event_handler));

    let session_event_handler = SessionEventHandler::new(
        config.config_file.enable_data_collection,
        metric_report_manager.mbox().into(),
        config.mar_staging_path(),
        (&config).into(),
    );
    http_handlers.push(Box::new(session_event_handler));

    {
        let collectd_handler = CollectdHandler::new(
            config.config_file.enable_data_collection,
            config.builtin_system_metric_collection_enabled(),
            metric_report_manager.mbox().into(),
        );
        http_handlers.push(Box::new(collectd_handler));
    }

    let metrics_event_handler = MetricsEventHandler::new(
        metrics_mbox.clone(),
        config.config_file.enable_data_collection,
    );
    http_handlers.push(Box::new(metrics_event_handler));

    let mut scheduler = Scheduler::default();

    // Schedule dump jobs for both heartbeat, daily heartbeat, and HRT
    {
        let net_config = NetworkConfig::from(&config);
        let mar_staging_path = config.mar_staging_path();
        let heartbeat_interval = config.config_file.heartbeat_interval;
        let mailbox = metric_report_manager.mbox();

        let heartbeat_message = DumpMetricReportMessage::new(
            ReportsToDump::Report(MetricReportType::Heartbeat),
            mar_staging_path.clone(),
            net_config.clone(),
        );
        scheduler.schedule_message_subscription(heartbeat_message, &mailbox, &heartbeat_interval);

        let hrt_message = DumpHrtMessage::new(mar_staging_path.clone(), net_config.clone());
        scheduler.schedule_message_subscription(hrt_message, &mailbox, &heartbeat_interval);

        if config.config_file.metrics.enable_daily_heartbeats {
            let daily_message = DumpMetricReportMessage::new(
                ReportsToDump::Report(MetricReportType::DailyHeartbeat),
                mar_staging_path,
                net_config,
            );
            scheduler.schedule_message_subscription(
                daily_message,
                &mailbox,
                &DAILY_HEARTBEAT_INTERVAL,
            );
        }
    }

    // Start statsd server
    if config.statsd_server_enabled() && config.config_file.enable_data_collection {
        if let Ok(bind_address) = config.statsd_server_address() {
            let legacy_gauge_aggregation = config.statsd_server_legacy_gauge_aggregation_enabled();
            let metrics_mailbox = metrics_mbox.clone();
            spawn(move || {
                let statsd_server = StatsDServer::new(legacy_gauge_aggregation, metrics_mailbox);
                if let Err(e) = statsd_server.run(bind_address) {
                    warn!("Couldn't start StatsD server: {}", e);
                };
            });
        }
    }

    // Start system metric collector thread
    if config.builtin_system_metric_collection_enabled()
        && config.config_file.enable_data_collection
    {
        let poll_interval = config.system_metric_poll_interval();
        let mbox = metrics_mbox.clone();
        let processes_config = config.system_metric_monitored_processes();
        let network_interfaces_config = config.system_metric_network_interfaces_config().cloned();
        let disk_space_config = config.system_metric_disk_space_config();
        let diskstats_config = config.system_metric_diskstats_config();
        spawn(move || {
            let mut sys_metric_collector = SystemMetricsCollector::new(
                processes_config,
                network_interfaces_config,
                disk_space_config,
                diskstats_config,
                mbox,
            );
            sys_metric_collector.run(poll_interval)
        });
    }

    // Start a thread to update battery metrics
    // periodically if enabled by configuration
    if config.battery_monitor_periodic_update_enabled() && config.config_file.enable_data_collection
    {
        let battery_monitor_interval = config.battery_monitor_interval();
        let battery_info_command_str = config.battery_monitor_battery_info_command().to_string();
        spawn(move || {
            let mut next_battery_interval = Instant::now() + battery_monitor_interval;
            loop {
                while Instant::now() < next_battery_interval {
                    sleep(next_battery_interval - Instant::now());
                }
                next_battery_interval += battery_monitor_interval;
                let battery_info_command = Command::new(&battery_info_command_str);
                if let Err(e) = battery_monitor
                    .lock()
                    .expect("Mutex poisoned")
                    .update_via_command(battery_info_command)
                {
                    warn!("Error updating battery monitor metrics: {}", e);
                }
            }
        });
    }
    // Connected time monitor is only enabled if config is defined
    if let Some(connectivity_monitor_config) = config.connectivity_monitor_config() {
        if config.config_file.enable_data_collection {
            let mut connectivity_monitor =
                ConnectivityMonitor::<Instant, TcpConnectionChecker>::new(
                    connectivity_monitor_config,
                    metric_report_manager.mbox().into(),
                );
            spawn(move || {
                let mut next_connectivity_reading_time =
                    Instant::now() + connectivity_monitor.interval_seconds();
                loop {
                    while Instant::now() < next_connectivity_reading_time {
                        sleep(next_connectivity_reading_time - Instant::now());
                    }
                    next_connectivity_reading_time += connectivity_monitor.interval_seconds();
                    if let Err(e) = connectivity_monitor.update_connected_time() {
                        warn!("Failed to update connected time metrics: {}", e);
                    }
                }
            });
        }
    }
    // Schedule a task to dump the metrics when a sync is forced
    {
        {
            let net_config = NetworkConfig::from(&config);
            let mar_staging_path = config.mar_staging_path();
            let dump_metrics_mbox = metric_report_manager.mbox();
            sync_tasks.push(Box::new(move |forced| match forced {
                true => {
                    trace!("Dumping heartbeat metrics");
                    dump_metrics_mbox
                        .send_and_wait_for_reply(DumpMetricReportMessage::new(
                            ReportsToDump::Report(MetricReportType::Heartbeat),
                            mar_staging_path.clone(),
                            net_config.clone(),
                        ))
                        .map_err(|e| eyre!("Couldn't send message to dump_metrics_mbox: {}", e))?
                        .map_err(|e| eyre!("Error dumping metrics: {}", e))
                }
                false => Ok(()),
            }));
        }

        {
            let net_config = NetworkConfig::from(&config);
            let mar_staging_path = config.mar_staging_path();
            let dump_metrics_mbox = metric_report_manager.mbox();
            sync_tasks.push(Box::new(move |forced| match forced {
                true => {
                    trace!("Dumping daily heartbeat metrics");
                    dump_metrics_mbox
                        .send_and_wait_for_reply(DumpMetricReportMessage::new(
                            ReportsToDump::Report(MetricReportType::DailyHeartbeat),
                            mar_staging_path.clone(),
                            net_config.clone(),
                        ))
                        .map_err(|e| eyre!("Couldn't send message to dump_metrics_mbox: {}", e))?
                        .map_err(|e| eyre!("Error dumping metrics: {}", e))
                }
                false => Ok(()),
            }));
        }

        {
            let net_config = NetworkConfig::from(&config);
            let mar_staging_path = config.mar_staging_path();
            let dump_metrics_mbox = metric_report_manager.mbox();
            sync_tasks.push(Box::new(move |forced| match forced {
                true => {
                    trace!("Dumping HRT metrics");
                    let hrt_message =
                        DumpHrtMessage::new(mar_staging_path.clone(), net_config.clone());
                    dump_metrics_mbox
                        .send_and_wait_for_reply(hrt_message)
                        .map_err(|e| eyre!("Couldn't send message to dump_metrics_mbox: {}", e))?
                        .map_err(|e| eyre!("Error dumping metrics: {}", e))
                }
                false => Ok(()),
            }));
        }
    }
    // Schedule a task to dump the metrics when we are shutting down
    {
        let net_config = NetworkConfig::from(&config);
        let mar_staging_path = config.mar_staging_path();

        let dump_metrics_mbox = metric_report_manager.mbox();
        shutdown_tasks.push(Box::new(move || {
            dump_metrics_mbox
                .send_and_forget(DumpMetricReportMessage::new(
                    ReportsToDump::All,
                    mar_staging_path.clone(),
                    net_config.clone(),
                ))
                .map_err(|e| eyre!("Couldn't send message to dump_metrics_mbox: {}", e))
        }));
    }
    // Schedule a task to dump HRT when shutting down
    {
        let net_config = NetworkConfig::from(&config);
        let mar_staging_path = config.mar_staging_path();
        let dump_metrics_mbox = metric_report_manager.mbox();
        shutdown_tasks.push(Box::new(move || {
            dump_metrics_mbox
                .send_and_wait_for_reply(DumpHrtMessage::new(
                    mar_staging_path.clone(),
                    net_config.clone(),
                ))
                .map_err(|e| eyre!("Couldn't send message to dump_metrics_mbox: {}", e))?
                .map_err(|e| eyre!("Error dumping HRT: {}", e))
        }));
    }
    // Schedule a task to compute operational and crashfree hours
    if config.config_file.enable_data_collection {
        let mut crashfree_tracker =
            CrashFreeIntervalTracker::<Instant>::new_hourly(metric_report_manager.mbox().into());
        http_handlers.push(crashfree_tracker.http_handler());
        spawn(move || {
            let interval = Duration::from_secs(60);
            loop {
                if let Err(e) = crashfree_tracker.wait_and_update(interval) {
                    warn!("Error updating crashfree hours: {}", e);
                }
            }
        });
    }

    // Only set to a non-None value when we build with the logging feature
    #[allow(unused_mut, /* reason = "Is unused when logging is disabled." */)]
    let mut logging_resolution_sender: Option<Sender<Resolution>> = None;

    #[cfg(feature = "logging")]
    {
        use crate::config::LogSource;
        #[cfg(feature = "systemd")]
        use crate::logs::journald_provider::start_journald_provider;
        use crate::logs::log_entry::LogEntry;
        use crate::mar::MAR_ENTRY_OVERHEAD_SIZE_ESTIMATE;
        use log::debug;

        let fluent_bit_config = FluentBitConfig::from(&config);
        if config.config_file.enable_data_collection {
            let log_source = config.config_file.logs.source;
            let log_receiver: Box<dyn Iterator<Item = LogEntry> + Send> = match log_source {
                LogSource::FluentBit => {
                    let (_, fluent_bit_receiver) =
                        FluentBitConnectionHandler::start(fluent_bit_config)?;
                    Box::new(FluentBitAdapter::new(
                        fluent_bit_receiver,
                        &config.config_file.fluent_bit.extra_fluentd_attributes,
                    ))
                }
                #[cfg(feature = "systemd")]
                LogSource::Journald => Box::new(start_journald_provider(config.tmp_dir())),
                #[cfg(not(feature = "systemd"))]
                LogSource::Journald => {
                    warn!("logs.source configuration set to \"journald\", but memfaultd was not compiled with the systemd feature. Logs will not be collected.");
                    // This match arm still needs to evaluate to Box<dyn Iterator<Item = LogEntry> + Send>
                    // Empty iterator typechecks and is effectively a no-op.
                    Box::new([].into_iter())
                }
            };

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

                // Create a rough size for the new MAR entry
                let mut estimated_entry_size = DiskSize::from(path.metadata()?);
                estimated_entry_size.bytes += MAR_ENTRY_OVERHEAD_SIZE_ESTIMATE;
                estimated_entry_size.inodes += 2;

                mar_cleaner.clean(estimated_entry_size)?;

                let mar_builder = MarEntryBuilder::new(&mar_staging_path)?
                    .set_metadata(Metadata::new_log(file_name, cid, next_cid, compression))
                    .add_attachment(path)?;

                // Move the log in the mar_staging area and add a manifest
                let mar_entry = mar_builder.save(&network_config)?;
                debug!(
                    "Logfile (cid: {}) saved as MAR entry: {}",
                    cid,
                    mar_entry.path.display()
                );

                Ok(())
            };
            let log_config = LogCollectorConfig::from(&config);
            let headroom_limiter = {
                let tmp_folder = log_config.log_tmp_path.clone();
                HeadroomLimiter::new(config.tmp_dir_min_headroom(), move || {
                    get_disk_space(&tmp_folder)
                })
            };

            let logging_resolution = config
                .cached_device_config
                .read()
                .expect("RwLock Poisoned")
                .get()
                .sampling
                .logging_resolution;

            let (mut log_collector, sender) = LogCollector::open(
                log_config,
                logging_resolution,
                on_log_completion,
                headroom_limiter,
                metric_report_manager.mbox().into(),
            )?;

            // Store the Sender in a variable that can be accessed
            // outside this logging code block
            logging_resolution_sender = Some(sender);

            log_collector.spawn_collect_from(log_receiver);

            let crash_log_handler = log_collector.crash_log_handler();
            http_handlers.push(Box::new(crash_log_handler));

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

    // Start the http server
    let mut http_server = HttpServer::new(http_handlers);
    http_server.start(config.config_file.http_server.bind_address)?;

    // Run the ready callback (creates the PID file)
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

    // Start the scheduler
    let failed_send_fn = Box::new(|e| {
        error!("Failed to run scheduled job: {}", e);
    });
    scheduler.run(failed_send_fn);

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
                        || last_device_config_refresh
                            .expect("Last device config refresh not present")
                            + CONFIG_REFRESH_INTERVAL
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
                        last_device_config_refresh = Some(Instant::now());
                        if let Some(sender) = &logging_resolution_sender {
                            if let Err(e) = sender.send(
                                config
                                    .cached_device_config
                                    .read()
                                    .expect("RwLock Poisoned")
                                    .get()
                                    .sampling
                                    .logging_resolution,
                            ) {
                                warn!(
                                    "Error sending updated device config to log collector: {}",
                                    e
                                );
                            }
                        }
                    }
                    Ok(UpdateStatus::Unchanged) => {
                        trace!("Device config unchanged");
                        last_device_config_refresh = Some(Instant::now())
                    }
                }
            }

            for task in &mut sync_tasks {
                if let Err(e) = task(forced) {
                    warn!("{:#}", e);
                }
            }

            mar_cleaner.clean(DiskSize::ZERO)?;

            if enable_data_collection && !forced_sync_only || forced {
                trace!("Collect MAR entries...");
                let result = collect_and_upload(
                    &config.mar_staging_path(),
                    &client,
                    config.config_file.mar.mar_file_max_size,
                    config.sampling(),
                );
                let _metric_result =
                    match result {
                        Ok(0) => Ok(()),
                        Ok(_count) => metrics_mbox.send_and_forget(vec![
                            KeyedMetricReading::increment_counter(METRIC_MF_SYNC_SUCCESS.into()),
                        ]),
                        Err(_) => metrics_mbox.send_and_forget(vec![
                            KeyedMetricReading::increment_counter(METRIC_MF_SYNC_FAILURE.into()),
                        ]),
                    };
                return result.map(|_| ());
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
            // Otherwise, keep running normally
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
