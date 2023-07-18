//
// Copyright (c) Memfault, Inc.
// See License.txt for details
mod reasons;

pub use reasons::RebootReason;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{fs::write, fs::File, process::Command};

use eyre::{eyre, Context, Result};
use log::{debug, error, info, warn};
use uuid::Uuid;

use crate::util::system::read_system_boot_id;
use crate::{config::Config, service_manager::ServiceManagerStatus};
use crate::{mar::MarEntryBuilder, network::NetworkConfig};
use crate::{mar::Metadata, service_manager::MemfaultdServiceManager};

const PSTORE_DIR: &str = "/sys/fs/pstore";

/// Manages reboot reasons and writes them to the MAR file if untracked.
///
/// This tracker is responsible for tracking the boot_id and reboot reason
/// across reboots. It will write the reboot reason to the MAR file if it
/// has not been tracked yet.
pub struct RebootReasonTracker<'a> {
    config: &'a Config,
    sources: Vec<RebootReasonSource>,
    service_manager: &'a dyn MemfaultdServiceManager,
}

impl<'a> RebootReasonTracker<'a> {
    pub fn new(config: &'a Config, service_manager: &'a dyn MemfaultdServiceManager) -> Self {
        let sources = vec![
            RebootReasonSource {
                name: "pstore",
                func: read_reboot_reason_and_clear_file_pstore,
            },
            RebootReasonSource {
                name: "custom",
                func: read_reboot_reason_and_clear_file_customer,
            },
            RebootReasonSource {
                name: "internal",
                func: read_reboot_reason_and_clear_file_internal,
            },
        ];

        Self {
            config,
            sources,
            service_manager,
        }
    }

    pub fn track_reboot(&self) -> Result<()> {
        let boot_id = read_system_boot_id()?;

        if !self.config.config_file.enable_data_collection {
            // Clear boot id since we haven't enabled data collection yet.
            self.check_boot_id_is_tracked(&boot_id);

            process_pstore_files(PSTORE_DIR);

            return Ok(());
        }

        if !self.check_boot_id_is_tracked(&boot_id) {
            let reboot_reason = self.resolve_reboot_reason(&boot_id)?;

            let mar_builder = MarEntryBuilder::new(&self.config.mar_staging_path())?
                .set_metadata(Metadata::new_reboot(reboot_reason));

            let network_config = NetworkConfig::from(self.config);
            mar_builder.save(&network_config)?;
        }

        Ok(())
    }

    fn check_boot_id_is_tracked(&self, boot_id: &Uuid) -> bool {
        let tmp_filename = self
            .config
            .config_file
            .generate_tmp_filename("last_tracked_boot_id");

        let last_boot_id = std::fs::read_to_string(&tmp_filename)
            .ok()
            .and_then(|boot_id| Uuid::from_str(boot_id.trim()).ok());
        if last_boot_id.is_none() {
            warn!("No last tracked boot_id found");
        }

        if let Err(e) = std::fs::write(tmp_filename, boot_id.to_string()) {
            error!("Failed to write last tracked boot_id: {}", e);
        }

        match last_boot_id {
            Some(last_boot_id) => {
                let is_tracked = &last_boot_id == boot_id;
                if is_tracked {
                    info!("boot_id already tracked {}!", boot_id);
                }

                is_tracked
            }
            None => false,
        }
    }

    fn resolve_reboot_reason(&self, boot_id: &Uuid) -> Result<RebootReason> {
        let mut reboot_reason = None;
        for reason_source in &self.sources {
            if let Some(new_reboot_reason) = (reason_source.func)(self.config) {
                if reboot_reason.is_some() {
                    info!(
                        "Discarded reboot reason {} ({:#04x}) from {} source for boot_id {}",
                        new_reboot_reason, new_reboot_reason as u32, reason_source.name, boot_id
                    );
                } else {
                    reboot_reason = Some(new_reboot_reason);
                    info!(
                        "Using reboot reason {} ({:#04x}) from {} source for boot_id {}",
                        new_reboot_reason, new_reboot_reason as u32, reason_source.name, boot_id
                    );
                }
            }
        }

        Ok(reboot_reason.unwrap_or(RebootReason::Unknown))
    }
}

impl<'a> Drop for RebootReasonTracker<'a> {
    fn drop(&mut self) {
        let status = self.service_manager.service_manager_status();

        match status {
            Ok(ServiceManagerStatus::Stopping) => {
                // Only write the user reset reason if the service manager is stopping
                let reboot_file = self
                    .config
                    .config_file
                    .generate_persist_filename("lastrebootreason");

                // Helper function to combine errors and handle them easier
                fn inner_write(mut file: File) -> Result<()> {
                    // Ensure file is written as the process could exit before it is done.
                    let reason_int = RebootReason::UserReset as u32;

                    file.write_all(reason_int.to_string().as_bytes())?;
                    file.sync_all()?;

                    Ok(())
                }

                info!("Writing reboot reason to {:?}", reboot_file);
                match File::create(reboot_file) {
                    Ok(file) => {
                        if let Err(e) = inner_write(file) {
                            error!("Failed to write reboot reason: {}", e);
                        }
                    }
                    Err(e) => {
                        error!("Failed to create reboot reason file: {}", e);
                    }
                }
            }
            Ok(status) => {
                debug!(
                    "Service manager in state {:?} while closing. Not writing reboot reason",
                    status
                );
            }
            Err(e) => error!("Failed to get service manager status: {}", e),
        }
    }
}

struct RebootReasonSource {
    name: &'static str,
    func: fn(&Config) -> Option<RebootReason>,
}

const PSTORE_DMESG_FILE: &str = "/sys/fs/pstore/dmesg-ramoops-0";

fn read_reboot_reason_and_clear_file_pstore(_config: &Config) -> Option<RebootReason> {
    if Path::new(PSTORE_DMESG_FILE).exists() {
        process_pstore_files(PSTORE_DIR);

        Some(RebootReason::KernelPanic)
    } else {
        None
    }
}

fn read_reboot_reason_and_clear_file_internal(config: &Config) -> Option<RebootReason> {
    let reboot_file = config
        .config_file
        .generate_persist_filename("lastrebootreason");

    read_reboot_reason_and_clear_file(&reboot_file)
}

fn read_reboot_reason_and_clear_file_customer(config: &Config) -> Option<RebootReason> {
    let file_name = &config.config_file.reboot.last_reboot_reason_file;

    read_reboot_reason_and_clear_file(file_name)
}

fn read_reboot_reason_and_clear_file(file_name: &PathBuf) -> Option<RebootReason> {
    let reboot_reason_string = match std::fs::read_to_string(file_name) {
        Ok(reboot_reason) => reboot_reason,
        Err(e) => {
            warn!("Failed to open {:?}: {}", file_name, e.kind());
            return None;
        }
    };

    let reboot_reason = reboot_reason_string
        .trim()
        .parse::<u32>()
        .ok()
        .and_then(RebootReason::from_repr);
    if reboot_reason.is_none() {
        error!(
            "Failed to parse reboot reason {} in file {:?}",
            reboot_reason_string, file_name
        );
    }

    if let Err(e) = std::fs::remove_file(file_name) {
        error!("Failed to remove {:?}: {}", file_name, e.kind());
    }
    reboot_reason
}

fn process_pstore_files(pstore_dir: &str) {
    // TODO: MFLT-7805 Process last kmsg/console logs
    debug!("Cleaning up pstore...");

    fn inner_process_pstore(pstore_dir: &str) -> Result<()> {
        for entry in std::fs::read_dir(pstore_dir)? {
            let path = entry?.path();

            if path.is_file() || path.is_symlink() {
                debug!("Cleaning pstore - Removing {}...", path.display());
                std::fs::remove_file(path)?;
            }
        }

        Ok(())
    }

    if let Err(e) = inner_process_pstore(pstore_dir) {
        error!("Failed to process pstore files: {}", e);
    }
}

pub fn write_reboot_reason_and_reboot(
    last_reboot_reason_file: &Path,
    reason: RebootReason,
) -> Result<()> {
    println!("Rebooting with reason {} ({})", reason as u32, reason);

    write(last_reboot_reason_file, format!("{}", reason as u32)).wrap_err_with(|| {
        format!(
            "Unable to write reboot reason (path: {}).",
            last_reboot_reason_file.display()
        )
    })?;
    if !Command::new("reboot").status()?.success() {
        return Err(eyre!("Failed to reboot"));
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::mar::manifest::{Manifest, Metadata};
    use crate::service_manager::{MockMemfaultdServiceManager, ServiceManagerStatus};
    use crate::test_utils::setup_logger;
    use crate::util::path::AbsolutePath;

    use rstest::rstest;
    use tempfile::tempdir;

    impl<'a> RebootReasonTracker<'a> {
        fn new_with_sources(
            config: &'a Config,
            sources: Vec<RebootReasonSource>,
            service_manager: &'a dyn MemfaultdServiceManager,
        ) -> Self {
            Self {
                config,
                sources,
                service_manager,
            }
        }
    }

    const TEST_BOOT_ID: &str = "32c45579-8881-4a43-b7d1-f1df8invalid";

    #[rstest]
    fn test_reboot_reason_source_ordering(_setup_logger: ()) {
        let mut config = Config::test_fixture();
        config.config_file.enable_data_collection = true;

        let persist_dir = tempdir().unwrap();
        config.config_file.persist_dir =
            AbsolutePath::try_from(persist_dir.path().to_path_buf()).unwrap();

        let main_source = RebootReasonSource {
            name: "main",
            func: |_: &Config| Some(RebootReason::HardFault),
        };
        let secondary_source = RebootReasonSource {
            name: "secondary",
            func: |_: &Config| Some(RebootReason::UserReset),
        };

        let mut service_manager = MockMemfaultdServiceManager::new();
        service_manager
            .expect_service_manager_status()
            .once()
            .returning(|| Ok(ServiceManagerStatus::Stopping));

        let mar_staging_path = config.mar_staging_path();
        std::fs::create_dir_all(&mar_staging_path).expect("Failed to create mar staging dir");

        let tracker = RebootReasonTracker::new_with_sources(
            &config,
            vec![main_source, secondary_source],
            &service_manager,
        );
        tracker
            .track_reboot()
            .expect("Failed to init reboot tracker");

        // Verify that the first reboot reason source is used
        verify_mar_reboot_reason(RebootReason::HardFault, &mar_staging_path);
    }

    #[rstest]
    fn test_reboot_reason_parsing(_setup_logger: ()) {
        let mut config = Config::test_fixture();
        config.config_file.enable_data_collection = true;

        let persist_dir = tempdir().unwrap();
        config.config_file.persist_dir =
            AbsolutePath::try_from(persist_dir.path().to_path_buf()).unwrap();

        let reboot_reason = RebootReason::HardFault;

        // Write values to last boot id and last reboot reason files
        let last_reboot_file = persist_dir.path().join("lastrebootreason");
        std::fs::write(&last_reboot_file, (reboot_reason as u32).to_string())
            .expect("Failed to write last reboot file");
        let last_boot_id_file = persist_dir.path().join("last_tracked_boot_id");
        std::fs::write(last_boot_id_file, TEST_BOOT_ID).expect("Failed to write last boot id file");

        let source = RebootReasonSource {
            name: "test",
            func: read_reboot_reason_and_clear_file_internal,
        };

        let mut service_manager = MockMemfaultdServiceManager::new();
        service_manager
            .expect_service_manager_status()
            .once()
            .returning(|| Ok(ServiceManagerStatus::Stopping));

        let mar_staging_path = config.mar_staging_path();

        // Create mar staging dir
        std::fs::create_dir_all(&mar_staging_path).expect("Failed to create mar staging dir");

        let tracker =
            RebootReasonTracker::new_with_sources(&config, vec![source], &service_manager);
        tracker
            .track_reboot()
            .expect("Failed to init reboot tracker");

        verify_mar_reboot_reason(reboot_reason, &mar_staging_path);

        // Drop tracker and ensure new reboot reason is written to file
        drop(tracker);
        let reboot_reason = std::fs::read_to_string(&last_reboot_file)
            .expect("Failed to read last reboot file")
            .parse::<u32>()
            .expect("Failed to parse reboot reason");
        let reboot_reason = RebootReason::from_repr(reboot_reason).expect("Invalid reboot reason");

        assert_eq!(reboot_reason, RebootReason::UserReset);
    }

    fn verify_mar_reboot_reason(reboot_reason: RebootReason, mar_staging_path: &Path) {
        let mar_dir = std::fs::read_dir(mar_staging_path)
            .expect("Failed to read temp dir")
            .filter_map(|entry| entry.ok())
            .collect::<Vec<_>>();

        // There should only be an entry for the reboot reason
        assert_eq!(mar_dir.len(), 1);

        let mar_manifest = mar_dir[0].path().join("manifest.json");
        let manifest_string = std::fs::read_to_string(mar_manifest).unwrap();
        let manifest: Manifest = serde_json::from_str(&manifest_string).unwrap();

        if let Metadata::LinuxReboot { reason } = manifest.metadata {
            assert_eq!(reboot_reason, reason);
        } else {
            panic!("Unexpected metadata type");
        }
    }
}
