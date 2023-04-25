//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::ffi::CStr;
use std::fs::create_dir_all;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use eyre::Result;
use eyre::{eyre, Context};
use log::{error, info, trace, warn};

use memfaultc_sys::QueueHandle;

use crate::mar::clean::MarStagingCleaner;
use crate::network::client::NetworkClientImpl;
use crate::network::{NetworkClient, NetworkConfig};
use crate::queue::{Queue, QueueMessage, QueueMessageType};
use crate::retriable_error::IgnoreNonRetriableError;
use crate::util::disk_size::DiskSize;
use crate::util::pid_file::write_memfaultd_pid_file;
use crate::util::task::{loop_with_exponential_error_backoff, LoopContinuation};
use crate::{config::Config, mar::upload::collect_and_upload};

#[cfg(feature = "coredump")]
use crate::process_coredumps::process_coredumps_with;

#[cfg(feature = "logging")]
use crate::{
    fluent_bit::{FluentBitConfig, FluentBitConnectionHandler},
    logs::{CompletedLog, FluentBitAdapter, HeadroomLimiter, LogCollector, LogCollectorConfig},
    mar::mar_entry::MarEntryBuilder,
    util::disk_size::get_disk_space,
};

#[no_mangle]
/// Process queue messages until SIGINT, SIGTERM or SIGHUP is received.
pub extern "C" fn memfaultd_rust_process_loop(
    user_config: *const libc::c_char,
    queue: QueueHandle,
    daemon: bool,
) -> bool {
    trace!("memfaultd_rust_process_loop()");

    let config_path = unsafe { CStr::from_ptr(user_config).to_str().ok().map(Path::new) };

    if let Err(e) = process_loop(config_path, queue, daemon) {
        error!("Fatal: {:#}", e);
        return false;
    }
    true
}

fn process_loop(user_config: Option<&Path>, queue: QueueHandle, daemon: bool) -> Result<()> {
    // Register a flag which will be set when one of these signals is received.
    let term_signals = [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ];
    let term = Arc::new(AtomicBool::new(false));
    for signal in term_signals {
        signal_hook::flag::register(signal, Arc::clone(&term))?;
    }

    // Register a flag to be set when we are woken up by SIGUSR1
    let force_sync = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGUSR1, Arc::clone(&force_sync))?;

    // Load configuration and device information. This has already been done by the C code but
    // we are preparing for a future where there is no more C code.
    let config =
        Config::read_from_system(user_config).wrap_err(eyre!("Unable to load configuration"))?;
    let client = NetworkClientImpl::new(NetworkConfig::from(&config))
        .wrap_err(eyre!("Unable to prepare network client"))?;
    let mut queue = Queue::attach(queue);

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

    // List all the enabled plugins
    let mut plugin_tasks: Vec<Box<dyn FnMut(bool) -> Result<()>>> = vec![];

    #[cfg(feature = "coredump")]
    {
        plugin_tasks.push(Box::new(|_forced_sync| {
            process_coredumps_with(&config.coredumps_path(), |path, gzipped| {
                info!("Uploading coredump {:?}", path);
                client.upload_coredump(path, gzipped)
            })
        }));
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
                let mar_builder = MarEntryBuilder::new_log(path, cid, next_cid, compression)?;

                mar_cleaner.clean(mar_builder.estimated_entry_size())?;

                // Move the log in the mar_staging area and add a manifest
                let mar_entry = mar_builder.save(&mar_staging_path, &network_config)?;
                debug!("New MAR entry generated: {}", mar_entry.path.display());

                Ok(())
            };
            let log_config = LogCollectorConfig::from(&config);
            let headroom_limiter = {
                let tmp_folder = log_config.log_tmp_path.clone();
                HeadroomLimiter::new(
                    DiskSize {
                        bytes: config.config_file.tmp_dir_min_headroom as u64,
                        inodes: config.config_file.tmp_dir_min_inodes as u64,
                    },
                    move || get_disk_space(&tmp_folder),
                )
            };
            let mut log_collector =
                LogCollector::open(log_config, on_log_completion, headroom_limiter)?;
            log_collector.spawn_collect_from(FluentBitAdapter::new(
                fluent_bit_receiver,
                &config.config_file.fluent_bit.extra_fluentd_attributes,
            ));

            plugin_tasks.push(Box::new(move |forced_sync| {
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

    if daemon {
        // All subcomponents are ready, write the pid file now to indicate we've started up completely:
        write_memfaultd_pid_file()?;
    }

    loop_with_exponential_error_backoff(
        || {
            // Reset the forced sync flag before doing any work so we can detect
            // if it's set again while we run and RerunImmediately.
            let forced = force_sync.swap(false, Ordering::Relaxed);
            let enable_data_collection = config.config_file.enable_data_collection;

            if enable_data_collection {
                trace!("Process pending uploads");
                process_pending_uploads(&mut queue, &client)?;
            }

            for task in &mut plugin_tasks {
                task(forced)?;
            }

            mar_cleaner.clean(DiskSize::ZERO).unwrap();

            if enable_data_collection {
                trace!("Collect MAR entries...");
                collect_and_upload(
                    &config.mar_staging_path(),
                    &client,
                    config.config_file.mar.mar_file_max_size,
                )?;
            }
            Ok(())
        },
        || match (
            term.load(Ordering::Relaxed),
            force_sync.load(Ordering::Relaxed),
        ) {
            // Stop when we receive a term signal
            (true, _) => LoopContinuation::Stop,
            // If we received a SIGUSR1 signal while we were in the loop, rerun immediately.
            (false, true) => LoopContinuation::RerunImmediately,
            // Otherwise, keep runnin normally
            (false, false) => LoopContinuation::KeepRunning,
        },
        config.config_file.refresh_interval,
        Duration::new(60, 0),
    );
    info!("Memfaultd shutting down...");
    Ok(())
}

/// Process all data that is pending upload.
/// Stop on the first retriable network error. Log and skip other errors.
fn process_pending_uploads(queue: &mut Queue, client: &impl NetworkClient) -> Result<()> {
    while let Some(mut entry) = queue.read() {
        trace!("Processing queue message {:?}", &entry);
        process_queue_message(client, &entry)
            .ignore_non_retriable_errors_with(|e| {
                warn!("Error processing queue message: {:#}", e);
            })
            .map_err(|e| {
                info!("Temporary error processing queue message: {:#}", e);
                e
            })?;

        // Note: non-retriable errors do not interrupt processing the queue.
        // We skip messages resulting in non-retriable errors. They will never be sent.
        entry.set_processed(true);
    }

    Ok(())
}

/// Process one Queue message
/// Return a result where the Err() can be a `RetriableError`.
fn process_queue_message(client: &impl NetworkClient, message: &QueueMessage) -> Result<()> {
    match message.get_type() {
        Some(QueueMessageType::RebootEvent) => client.post_event(message.get_payload_cstr()?),
        None => Err(eyre!(
            "Invalid queue message with size {} and type {}",
            message.msg.len(),
            message.msg[0]
        )),
    }
}

#[cfg(test)]
mod test {
    use mockall::predicate::eq;
    use rstest::{fixture, rstest};

    use crate::network::MockNetworkClient;

    use super::*;

    #[fixture]
    fn mock_client() -> MockNetworkClient {
        let _ = stderrlog::new().module("memfaultd").verbosity(10).init();
        MockNetworkClient::new()
    }

    #[rstest]
    fn test_reboot_event(mut mock_client: MockNetworkClient) -> Result<()> {
        let mut queue = Queue::new::<&str>(None, 1024)?;
        let mut buf = br###"R[{"type":"trace","software_type":"main","software_version":"0.0.1","device_serial":"42","hardware_version":"v1","sdk_version":"1.0.0","event_info":{"reason":4},"user_info":{}}]0"###.to_owned();
        buf[buf.len() - 1] = 0;
        queue.write(&buf);

        mock_client
            .expect_post_event()
            .with(eq(r###"[{"type":"trace","software_type":"main","software_version":"0.0.1","device_serial":"42","hardware_version":"v1","sdk_version":"1.0.0","event_info":{"reason":4},"user_info":{}}]"###))
            .times(1)
            .returning(|_| Ok(()));

        let r = process_queue_message(&mock_client, &queue.read().unwrap());
        println!("Result: {:#?}", r);
        assert!(r.is_ok());
        Ok(())
    }
}
