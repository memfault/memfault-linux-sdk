//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Memfaultd service management
//!
//! This module contains the trait for managing memfaultd services, as well as
//! the implementation for systemd.

use std::ffi::CString;

use memfaultc_sys::memfaultd_restart_systemd_service_if_running;

/// Memfaultd services
///
/// These are the services that memfaultd manages.
#[derive(Debug, Clone, Copy)]
pub enum MemfaultdService {
    Memfaultd,
    SWUpdate,
    SwUpdateSocket,
}

/// Trait for managing memfaultd services
///
/// This trait is implemented for different service managers, such as systemd.
#[cfg_attr(test, mockall::automock)]
pub trait MemfaultdServiceManager {
    fn restart_service_if_running(&self, service: MemfaultdService) -> eyre::Result<()>;
}

type SystemdService = CString;

impl TryFrom<MemfaultdService> for SystemdService {
    type Error = eyre::Error;

    fn try_from(service: MemfaultdService) -> Result<Self, Self::Error> {
        let service = match service {
            MemfaultdService::Memfaultd => CString::new("memfaultd.service")?,
            MemfaultdService::SWUpdate => CString::new("swupdate.service")?,
            MemfaultdService::SwUpdateSocket => CString::new("swupdate.socket")?,
        };

        Ok(service)
    }
}

/// Systemd service manager
///
/// This service manager uses the systemd D-Bus API to manage services.
pub struct SystemdServiceManager;

impl MemfaultdServiceManager for SystemdServiceManager {
    fn restart_service_if_running(&self, service: MemfaultdService) -> eyre::Result<()> {
        let service_cstring = SystemdService::try_from(service)?;
        let restart_result =
            unsafe { memfaultd_restart_systemd_service_if_running(service_cstring.as_ptr()) };

        if restart_result {
            Ok(())
        } else {
            Err(eyre::eyre!("Failed to restart {:?} service", service))
        }
    }
}
