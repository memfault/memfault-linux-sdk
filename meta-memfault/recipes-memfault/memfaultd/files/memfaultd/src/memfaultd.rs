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
use std::{cmp::max, fs::create_dir_all, time::Instant};

use eyre::Result;
use eyre::{eyre, Context};
use log::{debug, error, info, trace, warn};
use rand::{thread_rng, Rng};

use ssf::{MsgMailbox, Scheduler, ServiceManager};

use crate::{config::Resolution, mar::MarConfig, metrics::MetricsEventHandler};
use crate::{
    mar::MarStagingCleanType,
    metrics::{
        BatteryMonitor, BatteryReadingHandler, ConnectivityMonitor, DumpHrtMessage,
        DumpMetricReportMessage, KeyedMetricReading, MetricReportType, ReportSyncEventHandler,
        ReportsToDump, SessionEventHandler, SystemMetricsCollector,
    },
};
use crate::{
    mar::MarStagingConfig,
    util::task::{interruptible_sleep, PreviousIterationKind},
};

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
use crate::trace::SaveTraceHandler;

#[cfg(all(feature = "ebpf", not(target_os = "macos")))]
use crate::ebpf::DiskIo;

#[cfg(feature = "logging")]
use crate::{
    fluent_bit::{FluentBitConfig, FluentBitConnectionHandler},
    logs::{
        messages::GetQueuedLogsMsg, CompletedLog, HeadroomLimiter, LogCollector, LogCollectorConfig,
    },
    mar::{MarEntryBuilder, Metadata},
    util::disk_size::get_disk_space,
};

const CONFIG_REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 120);
const DAILY_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60 * 60 * 24);

const MAX_EXPECTED_SHUTDOWN_DURATION_SECONDS: f64 = 3.0;

type SyncTask = Box<dyn FnMut(bool, bool) -> Result<()>>;

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

    // Flags that indicate whether special behavior should be taken while sync-ing
    let force_sync = Arc::new(AtomicBool::new(false));
    let skip_serialization_on_sync = Arc::new(AtomicBool::new(false));

    // If memfaultd is woken up by SIGUSR1, it is a forced sync
    signal_hook::flag::register(signal_hook::consts::SIGUSR1, Arc::clone(&force_sync))?;

    // If memfaultd is woken up by SIGUSR2, it is a forced sync where we should skip dumping data
    signal_hook::flag::register(signal_hook::consts::SIGUSR2, Arc::clone(&force_sync))?;
    signal_hook::flag::register(
        signal_hook::consts::SIGUSR2,
        Arc::clone(&skip_serialization_on_sync),
    )?;

    // Load configuration and device information. This has already been done by the C code but
    // we are preparing for a future where there is no more C code.
    let client = NetworkClientImpl::new(NetworkConfig::from(&config))
        .wrap_err(eyre!("Unable to prepare network client"))?;

    let service_manager = get_service_manager();

    // Make sure the MAR staging area exists
    create_dir_all(config.mar_tmp_staging_path()).wrap_err_with(|| {
        eyre!(
            "Unable to create MAR temp staging area {}",
            &config.mar_tmp_staging_path().display(),
        )
    })?;

    create_dir_all(config.mar_persist_staging_path()).wrap_err_with(|| {
        eyre!(
            "Unable to create MAR persist staging area {}",
            &config.mar_persist_staging_path().display(),
        )
    })?;

    let mut system = ServiceManager::default();

    // Metric store
    let metric_report_manager = match config.session_configs() {
        Some(session_configs) => MetricReportManager::new_with_session_configs(
            config.hrt_enabled(),
            config.hrt_max_samples_per_min(),
            session_configs,
            config.config_file.metrics.enable_daily_heartbeats,
        ),
        None => MetricReportManager::new(
            config.hrt_enabled(),
            config.hrt_max_samples_per_min(),
            config.config_file.metrics.enable_daily_heartbeats,
        ),
    };
    let metrics_mbox = system.spawn_service_thread(metric_report_manager);

    let mar_tmp_staging_config = MarStagingConfig::new(
        config.mar_tmp_staging_path(),
        config.tmp_dir_max_size(),
        config.tmp_dir_min_headroom(),
    );
    let mar_persist_staging_config = config.mar_persist_storage_config().map(|persist_config| {
        MarStagingConfig::new(
            config.mar_persist_staging_path(),
            persist_config.max_total_size(),
            persist_config.min_headroom(),
        )
    });
    let mar_cleaner = Arc::new(MarStagingCleaner::new(
        mar_tmp_staging_config,
        mar_persist_staging_config,
        config.mar_entry_max_age(),
        config.mar_entry_max_count(),
        metrics_mbox.clone().into(),
    ));

    // List of tasks to run before syncing with server
    let mut sync_tasks: Vec<SyncTask> = vec![];
    // List of tasks to run before shutting down
    let mut shutdown_tasks: Vec<Box<dyn FnOnce() -> Result<()>>> = vec![];

    #[cfg(all(feature = "ebpf", not(target_os = "macos")))]
    {
        use crate::util::system::ProcfsProcessNameMapper;

        let disk_io: DiskIo<ProcfsProcessNameMapper> = DiskIo::load(metrics_mbox.clone().into())?;
        system.spawn_bounded_task_service_thread(disk_io, 1024);
    }

    // List of http handlers
    #[allow(unused_mut, /* reason = "Can be unused when some features are disabled." */)]
    let mut http_handlers: Vec<Box<dyn HttpHandler>> = vec![Box::new(MarExportHandler::new(
        config.mar_tmp_staging_path(),
    ))];

    let battery_monitor = Arc::new(Mutex::new(BatteryMonitor::<Instant>::new(
        metrics_mbox.clone().into(),
    )));
    let battery_reading_handler = BatteryReadingHandler::new(
        config.config_file.enable_data_collection,
        battery_monitor.clone(),
    );
    http_handlers.push(Box::new(battery_reading_handler));

    let report_sync_event_handler = ReportSyncEventHandler::new(
        config.config_file.enable_data_collection,
        metrics_mbox.clone().into(),
    );
    http_handlers.push(Box::new(report_sync_event_handler));

    let session_event_handler = SessionEventHandler::new(
        config.config_file.enable_data_collection,
        metrics_mbox.clone().into(),
        &config,
    );
    http_handlers.push(Box::new(session_event_handler));

    {
        let collectd_handler = CollectdHandler::new(
            config.config_file.enable_data_collection,
            config.system_metric_config(),
            metrics_mbox.clone().into(),
        );
        http_handlers.push(Box::new(collectd_handler));
    }

    let metrics_event_handler = MetricsEventHandler::new(
        metrics_mbox.clone().into(),
        config.config_file.enable_data_collection,
    );
    http_handlers.push(Box::new(metrics_event_handler));

    let mut scheduler = Scheduler::default();

    // Schedule dump jobs for both heartbeat, daily heartbeat, and HRT
    {
        let net_config = Arc::new(NetworkConfig::from(&config));
        let mar_config = Arc::new(MarConfig::from(&config));
        let heartbeat_interval = config.config_file.heartbeat_interval;

        let heartbeat_message = DumpMetricReportMessage::new(
            ReportsToDump::Report(MetricReportType::Heartbeat),
            net_config.clone(),
            mar_config.clone(),
        );
        scheduler.schedule_message_subscription(
            heartbeat_message,
            &metrics_mbox,
            &heartbeat_interval,
        );

        let hrt_message = DumpHrtMessage::new(net_config.clone(), mar_config.clone());
        scheduler.schedule_message_subscription(hrt_message, &metrics_mbox, &heartbeat_interval);

        if config.config_file.metrics.enable_daily_heartbeats {
            let daily_message = DumpMetricReportMessage::new(
                ReportsToDump::Report(MetricReportType::DailyHeartbeat),
                net_config,
                mar_config,
            );
            scheduler.schedule_message_subscription(
                daily_message,
                &metrics_mbox,
                &DAILY_HEARTBEAT_INTERVAL,
            );
        }
    }

    // Start statsd server
    if config.statsd_server_enabled() && config.config_file.enable_data_collection {
        if let Ok(bind_address) = config.statsd_server_address() {
            let legacy_gauge_aggregation = config.statsd_server_legacy_gauge_aggregation_enabled();
            let legacy_key_names = config.statsd_server_legacy_key_names_enabled();
            let metrics_mailbox = metrics_mbox.clone().into();
            spawn(move || {
                let statsd_server =
                    StatsDServer::new(legacy_gauge_aggregation, legacy_key_names, metrics_mailbox);
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
        let system_metric_config = config.system_metric_config();
        let mbox = metrics_mbox.clone().into();
        spawn(move || {
            let mut sys_metric_collector = SystemMetricsCollector::new(system_metric_config, mbox);
            sys_metric_collector.run()
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
                    metrics_mbox.clone().into(),
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
        let net_config = Arc::new(NetworkConfig::from(&config));
        let mar_config = Arc::new(MarConfig::from(&config));
        let dump_metrics_mbox: MsgMailbox<DumpMetricReportMessage> = metrics_mbox.clone().into();
        sync_tasks.push(Box::new(move |forced, skip_serialization| match forced {
            true => {
                if skip_serialization {
                    debug!("Skipping heartbeat metric serialization");
                    Ok(())
                } else {
                    trace!("Dumping heartbeat metrics");
                    dump_metrics_mbox
                        .send_and_wait_for_reply(DumpMetricReportMessage::new(
                            ReportsToDump::Report(MetricReportType::Heartbeat),
                            net_config.clone(),
                            mar_config.clone(),
                        ))
                        .map_err(|e| eyre!("Couldn't send message to dump_metrics_mbox: {}", e))?
                        .map_err(|e| eyre!("Error dumping metrics: {}", e))
                }
            }
            false => Ok(()),
        }));
    }

    {
        let net_config = Arc::new(NetworkConfig::from(&config));
        let mar_config = Arc::new(MarConfig::from(&config));
        let dump_metrics_mbox: MsgMailbox<DumpMetricReportMessage> = metrics_mbox.clone().into();
        sync_tasks.push(Box::new(move |forced, skip_serialization| match forced {
            true => {
                if skip_serialization {
                    debug!("Skipping daily heartbeat metric serialization");
                    Ok(())
                } else {
                    trace!("Dumping daily heartbeat metrics");
                    dump_metrics_mbox
                        .send_and_wait_for_reply(DumpMetricReportMessage::new(
                            ReportsToDump::Report(MetricReportType::DailyHeartbeat),
                            net_config.clone(),
                            mar_config.clone(),
                        ))
                        .map_err(|e| eyre!("Couldn't send message to dump_metrics_mbox: {}", e))?
                        .map_err(|e| eyre!("Error dumping metrics: {}", e))
                }
            }
            false => Ok(()),
        }));
    }

    {
        let net_config = Arc::new(NetworkConfig::from(&config));
        let mar_config = Arc::new(MarConfig::from(&config));
        let dump_metrics_mbox: MsgMailbox<DumpHrtMessage> = metrics_mbox.clone().into();
        sync_tasks.push(Box::new(move |forced, skip_serialization| match forced {
            true => {
                if skip_serialization {
                    debug!("Skipping HRT metric serialization");
                    Ok(())
                } else {
                    trace!("Dumping HRT metrics");
                    let hrt_message = DumpHrtMessage::new(net_config.clone(), mar_config.clone());
                    dump_metrics_mbox
                        .send_and_wait_for_reply(hrt_message)
                        .map_err(|e| eyre!("Couldn't send message to dump_metrics_mbox: {}", e))?
                        .map_err(|e| eyre!("Error dumping metrics: {}", e))
                }
            }
            false => Ok(()),
        }));
    }

    // Schedule a task to dump the metrics when we are shutting down
    {
        let net_config = Arc::new(NetworkConfig::from(&config));
        let mar_config = Arc::new(MarConfig::from(&config));

        let dump_metrics_mbox: MsgMailbox<DumpMetricReportMessage> = metrics_mbox.clone().into();
        shutdown_tasks.push(Box::new(move || {
            dump_metrics_mbox
                .send_and_forget(DumpMetricReportMessage::new(
                    ReportsToDump::All,
                    net_config.clone(),
                    mar_config.clone(),
                ))
                .map_err(|e| eyre!("Couldn't send message to dump_metrics_mbox: {}", e))
        }));
    }
    // Schedule a task to dump HRT when shutting down
    {
        let net_config = Arc::new(NetworkConfig::from(&config));
        let mar_config = Arc::new(MarConfig::from(&config));
        let dump_metrics_mbox: MsgMailbox<DumpHrtMessage> = metrics_mbox.clone().into();
        shutdown_tasks.push(Box::new(move || {
            dump_metrics_mbox
                .send_and_wait_for_reply(DumpHrtMessage::new(
                    net_config.clone(),
                    mar_config.clone(),
                ))
                .map_err(|e| eyre!("Couldn't send message to dump_metrics_mbox: {}", e))?
                .map_err(|e| eyre!("Error dumping HRT: {}", e))
        }));
    }

    // Only set to a non-None value when we build with the logging feature
    #[allow(unused_mut, /* reason = "Is unused when logging is disabled." */)]
    let mut logging_resolution_sender: Option<Sender<Resolution>> = None;

    // Only set to a non-None value when we build with the logging feature
    #[cfg(feature = "logging")]
    #[allow(unused_mut)]
    let mut trace_log_collector_mbox: Option<MsgMailbox<GetQueuedLogsMsg>> = None;

    #[cfg(feature = "logging")]
    {
        use crate::config::LogSource;
        use crate::logs::log_collector::CrashLogHandler;
        use crate::logs::messages::RecoverLogsMsg;
        use crate::logs::messages::{FlushLogsMsg, RotateIfNeededMsg};
        use crate::mar::MAR_ENTRY_OVERHEAD_SIZE_ESTIMATE;

        let fluent_bit_config = FluentBitConfig::from(&config);
        if config.config_file.enable_data_collection {
            let log_source = config.config_file.logs.source;
            let extra_attr = config.log_extra_attributes();

            let mar_cleaner = mar_cleaner.clone();

            let network_config = NetworkConfig::from(&config);
            let mar_config = MarConfig::from(&config);
            let logs_clean_type = config.mar_logs_clean_type();
            let mar_staging_path = config.mar_tmp_staging_path();
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

                mar_cleaner.clean(estimated_entry_size, logs_clean_type)?;

                let mar_builder = MarEntryBuilder::new(&mar_staging_path)?
                    .set_metadata(Metadata::new_log(file_name, cid, next_cid, compression))
                    .add_attachment(path)?;

                // Move the log in the mar_staging area and add a manifest
                let mar_entry = mar_builder.save(&network_config, &mar_config)?;
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

            let (log_collector, sender) = LogCollector::open(
                log_config,
                logging_resolution,
                on_log_completion,
                headroom_limiter,
                metrics_mbox.clone().into(),
            )?;

            let max_buffered_lines = config.log_max_buffered_lines();
            let log_collector_mbox =
                system.spawn_bounded_service_thread(log_collector, max_buffered_lines);
            // Begin log recovery in spawned thread
            if let Err(e) = log_collector_mbox.send_and_forget(RecoverLogsMsg) {
                warn!("Failed to start log recovery: {}", e);
            }
            match log_source {
                LogSource::FluentBit => {
                    let _ = FluentBitConnectionHandler::start(
                        fluent_bit_config,
                        log_collector_mbox.clone().into(),
                        extra_attr,
                    )?;
                }
                #[cfg(feature = "syslog")]
                LogSource::Syslog(syslog_config) => {
                    use crate::logs::syslog::SyslogServer;

                    let log_mbox = log_collector_mbox.clone();
                    spawn(move || {
                        if let Err(e) = SyslogServer ::run(syslog_config.bind_address, log_mbox.into()) {
                            warn!("Couldn't start syslog receiver: {}", e)
                        }
                    });
                }
                #[cfg(not(feature = "syslog"))]
                LogSource::Syslog(_) => warn!("logs.source configuration set to \"syslog\", but memfaultd was not compiled with the syslog feature. Logs will not be collected."),
                #[cfg(feature = "systemd")]
                LogSource::Journald => {
                    use crate::logs::journald_provider::JournaldLogProvider;
                    use crate::logs::journald_parser::JournalRawImpl;

                    let tmp_dir = config.tmp_dir();
                    let log_collector_mbox = log_collector_mbox.clone();
                    let spawn_fn = move || {
                        let journal = JournalRawImpl::new(tmp_dir);
                        JournaldLogProvider::new(journal, log_collector_mbox.into(), extra_attr)
                    };

                    system.spawn_bounded_task_service_thread_with_fn(spawn_fn, 128);
                }
                #[cfg(not(feature = "systemd"))]
                LogSource::Journald => warn!("logs.source configuration set to \"journald\", but memfaultd was not compiled with the systemd feature. Logs will not be collected."),
            }
            // Store the Sender in a variable that can be accessed
            // outside this logging code block
            logging_resolution_sender = Some(sender);

            trace_log_collector_mbox = Some(log_collector_mbox.clone().into());

            let crash_log_handler = CrashLogHandler::new(
                log_collector_mbox.clone().into(),
                log_collector_mbox.clone().into(),
            );
            http_handlers.push(Box::new(crash_log_handler));

            sync_tasks.push(Box::new(move |forced_sync, skip_serialization| {
                // Check if we have received a signal to force-sync and reset the flag.
                if forced_sync && !skip_serialization {
                    trace!("Flushing logs");
                    log_collector_mbox.send_and_wait_for_reply(FlushLogsMsg)??;
                } else {
                    // If not force-flushing - we still want to make sure this file
                    // did not get too old.
                    log_collector_mbox.send_and_wait_for_reply(RotateIfNeededMsg)??;
                }
                Ok(())
            }));
        } else {
            FluentBitConnectionHandler::start_null(fluent_bit_config)?;
        }
    }

    // Schedule a task to compute operational and crashfree hours
    if config.config_file.enable_data_collection {
        let mut crashfree_tracker =
            CrashFreeIntervalTracker::<Instant>::new_hourly(metrics_mbox.clone().into());
        http_handlers.push(crashfree_tracker.http_handler());
        let trace_config = config.linux_custom_trace_config();
        let rate_limiter_path = config.trace_rate_limiter_file_path();

        #[cfg(feature = "logging")]
        let save_trace_handler = SaveTraceHandler::new(
            config.mar_tmp_staging_path(),
            &config,
            crashfree_tracker.channel_handler(),
            trace_log_collector_mbox,
            trace_config,
            rate_limiter_path,
        );

        #[cfg(not(feature = "logging"))]
        let save_trace_handler = SaveTraceHandler::new(
            config.mar_tmp_staging_path(),
            &config,
            crashfree_tracker.channel_handler(),
            trace_config,
            rate_limiter_path,
        );

        http_handlers.push(Box::new(save_trace_handler));

        spawn(move || {
            let interval = Duration::from_secs(60);
            loop {
                if let Err(e) = crashfree_tracker.wait_and_update(interval) {
                    warn!("Error updating crashfree hours: {}", e);
                }
            }
        });
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

    shutdown_tasks.push(Box::new(move || {
        let stats = system.stop();
        debug!("Service stats: {:#?}", stats);

        Ok(())
    }));

    // Determine a random duration to wait
    // before the first iteration in order to avoid thundering herd
    // issues if a large number of devices are rebooted or have
    // memfaultd restarted at the same moment
    //
    // The jitter duration is a randomly selected duration of seconds
    // between zero and one-fourth of the configured `upload_interval_seconds`
    //
    // Set a lower bound of 1 for the range to avoid sampling an empty range
    // (which causes a panic)
    let mut rng = thread_rng();
    let first_iteration_jitter_duration =
        Duration::from_secs(rng.gen_range(0..max(1, upload_interval.as_secs() / 4)));
    let is_first_iteration = Arc::new(AtomicBool::new(true));
    let first_iteration_completed = Arc::new(AtomicBool::new(false));

    loop_with_exponential_error_backoff(
        || {
            // Since this sleep is only performed on the first sync,
            // all subsequent syncs will be spaced out evenly (by
            // upload_interval_seconds)
            if is_first_iteration.swap(false, Ordering::Relaxed) {
                info!(
                    "Introducing {} seconds of jitter on first network request...",
                    first_iteration_jitter_duration.as_secs()
                );
                // Need to make this sleep interruptible so a forced
                // sync via SIGINT still responds immediately
                interruptible_sleep(first_iteration_jitter_duration);

                // If sleep is interrupted due to shutdown,
                // exit early to avoid delaying system shutdown
                if term.load(Ordering::Relaxed) {
                    return Ok(());
                }
            } else {
                // Flag that the first iteration (with jitter applied) has been completed
                // and that we should *not* subtract the duration of this `work` closure
                // from the subsequent sleep
                // If we do subtract the duration of this closure, the jitter will
                // be subtracted as well and subsequent requests will be
                // sync'd up with the time `memfaultd` started up.
                first_iteration_completed.store(true, Ordering::Relaxed);
            }

            // Reset the forced sync flag before doing any work so we can detect
            // if it's set again while we run and RerunImmediately.
            let forced = force_sync.swap(false, Ordering::Relaxed);
            let skip_serialization = skip_serialization_on_sync.swap(false, Ordering::Relaxed);
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
                if let Err(e) = task(forced, skip_serialization) {
                    warn!("{:#}", e);
                }
            }

            mar_cleaner.clean(DiskSize::ZERO, MarStagingCleanType::All)?;

            if enable_data_collection && !forced_sync_only || forced {
                trace!("Collect MAR entries...");
                let data_retention_start = config
                    .cached_device_config
                    .read()
                    .expect("RWLock Poisoned")
                    .get()
                    .data_upload_start_date;

                let persist_mar_staging = config
                    .mar_persist_storage_config()
                    .map(|_| config.mar_persist_staging_path());
                let result = collect_and_upload(
                    &config.mar_tmp_staging_path(),
                    persist_mar_staging,
                    &client,
                    config.config_file.mar.mar_file_max_size,
                    config.sampling(),
                    data_retention_start,
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
            first_iteration_completed.load(Ordering::Relaxed),
        ) {
            // Stop when we receive a term signal
            (true, _, _) => LoopContinuation::Stop,
            // If we received a SIGUSR1 signal while we were in the loop, rerun immediately.
            (false, true, _) => LoopContinuation::RerunImmediately,
            // Otherwise, keep running normally
            (false, false, false) => LoopContinuation::KeepRunning(PreviousIterationKind::First),
            (false, false, true) => {
                LoopContinuation::KeepRunning(PreviousIterationKind::Subsequent)
            }
        },
        upload_interval,
        Duration::new(60, 0),
    );

    let shutdown_start_time = Instant::now();
    info!("Memfaultd shutting down...");
    for task in shutdown_tasks {
        if let Err(e) = task() {
            warn!("Error while shutting down: {}", e);
        }
    }
    let shutdown_duration = shutdown_start_time.elapsed().as_secs_f64();
    if shutdown_duration > MAX_EXPECTED_SHUTDOWN_DURATION_SECONDS {
        warn!(
            "memfaultd took longer than expected to shutdown ({} seconds)",
            shutdown_duration
        )
    } else {
        debug!("memfaultd took {} seconds to shutdown", shutdown_duration)
    }

    if reload.load(Ordering::Relaxed) {
        Ok(MemfaultLoopResult::Relaunch)
    } else {
        Ok(MemfaultLoopResult::Terminate)
    }
}
