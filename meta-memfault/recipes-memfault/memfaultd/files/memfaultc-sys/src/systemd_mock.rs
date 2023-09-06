//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::ptr::null;

use libc::c_char;

/// Get the status of the systemd service manager.
/// # Safety
pub unsafe fn memfaultd_restart_systemd_service_if_running(service_name: *const c_char) -> bool {
    eprintln!("memfaultd_restart_systemd_service_if_running is not implemented for this target (restarting service {})", std::ffi::CStr::from_ptr(service_name).to_string_lossy());
    true
}

/// Get the status of the systemd service manager.
/// # Safety
pub unsafe fn memfaultd_get_systemd_bus_state() -> *const c_char {
    null()
}
