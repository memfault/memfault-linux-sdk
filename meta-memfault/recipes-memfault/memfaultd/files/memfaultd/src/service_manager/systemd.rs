//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::ffi::{CStr, CString};

use crate::service_manager::{MemfaultdService, MemfaultdServiceManager, ServiceManagerStatus};
use memfaultc_sys::systemd::{
    memfaultd_get_systemd_bus_state, memfaultd_restart_systemd_service_if_running,
};

type SystemdService = CString;

impl TryFrom<MemfaultdService> for SystemdService {
    type Error = eyre::Error;

    fn try_from(service: MemfaultdService) -> Result<Self, Self::Error> {
        let service = match service {
            MemfaultdService::Collectd => CString::new("collectd.service")?,
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

    fn service_manager_status(&self) -> eyre::Result<ServiceManagerStatus> {
        let status_ptr = unsafe { memfaultd_get_systemd_bus_state() };
        if status_ptr.is_null() {
            return Err(eyre::eyre!("Failed to get systemd service bus state"));
        }

        let status_str = unsafe { CStr::from_ptr(status_ptr).to_str()? };
        let status = ServiceManagerStatus::try_from(status_str)?;

        unsafe { libc::free(status_ptr as *mut libc::c_void) };

        Ok(status)
    }
}
