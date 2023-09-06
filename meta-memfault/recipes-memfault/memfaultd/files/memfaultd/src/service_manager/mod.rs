//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Memfaultd service management
//!
//! This module contains the trait for managing memfaultd services, as well as
//! the implementation for systemd.
//!

mod default;
#[cfg(feature = "systemd")]
mod systemd;

/// Return the system manager that was configured at build time.
pub fn get_service_manager() -> impl MemfaultdServiceManager {
    #[cfg(feature = "systemd")]
    {
        use systemd::SystemdServiceManager;
        SystemdServiceManager
    }
    #[cfg(not(feature = "systemd"))]
    {
        use default::DefaultServiceManager;
        DefaultServiceManager
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceManagerStatus {
    Starting,
    Running,
    Stopping,
    Stopped,
    Unknown,
}

/// Trait for managing memfaultd services
///
/// This trait is implemented for different service managers, such as systemd.
#[cfg_attr(test, mockall::automock)]
pub trait MemfaultdServiceManager {
    fn restart_memfaultd_if_running(&self) -> eyre::Result<()>;
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
