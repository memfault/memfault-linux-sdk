//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Memfaultd service management
//!
//! This module contains the trait for managing memfaultd services, as well as
//! the implementation for systemd.
//!
mod mock_servicemanager;
mod systemd;

/// Return the system manager that was configured at build time.
pub fn get_service_manager() -> impl MemfaultdServiceManager {
    #[cfg(target_os = "macos")]
    {
        use mock_servicemanager::MockServiceManager;
        // SystemD C code does not build on macOS so we stub it out
        MockServiceManager
    }
    #[cfg(not(target_os = "macos"))]
    {
        use systemd::SystemdServiceManager;
        SystemdServiceManager
    }
}

/// Memfaultd services
///
/// These are the services that memfaultd manages.
#[derive(Debug, Clone, Copy)]
pub enum MemfaultdService {
    Collectd,
    Memfaultd,
    SWUpdate,
    SwUpdateSocket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceManagerStatus {
    Starting,
    Running,
    Stopping,
    Stopped,
}

/// Trait for managing memfaultd services
///
/// This trait is implemented for different service managers, such as systemd.
#[cfg_attr(test, mockall::automock)]
pub trait MemfaultdServiceManager {
    fn restart_service_if_running(&self, service: MemfaultdService) -> eyre::Result<()>;
    fn service_manager_status(&self) -> eyre::Result<ServiceManagerStatus>;
}

impl TryFrom<&str> for ServiceManagerStatus {
    type Error = eyre::Error;

    fn try_from(status: &str) -> Result<Self, Self::Error> {
        let status = match status {
            "starting" => ServiceManagerStatus::Starting,
            "running" => ServiceManagerStatus::Running,
            "stopping" => ServiceManagerStatus::Stopping,
            "stopped" => ServiceManagerStatus::Stopped,
            _ => {
                return Err(eyre::eyre!(
                    "Unknown systemd service manager status: {}",
                    status
                ))
            }
        };

        Ok(status)
    }
}
