//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use super::{MemfaultdService, MemfaultdServiceManager, ServiceManagerStatus};
use log::warn;

pub struct MockServiceManager;

impl MemfaultdServiceManager for MockServiceManager {
    fn restart_service_if_running(&self, service: MemfaultdService) -> eyre::Result<()> {
        warn!("MockServiceManager::restart_service_if_running({service:?}) ");
        Ok(())
    }

    fn service_manager_status(&self) -> eyre::Result<ServiceManagerStatus> {
        Ok(ServiceManagerStatus::Running)
    }
}
