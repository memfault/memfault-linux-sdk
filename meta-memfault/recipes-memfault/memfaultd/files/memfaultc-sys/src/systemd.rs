//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use libc::c_char;

extern "C" {
    pub fn memfaultd_restart_systemd_service_if_running(service_name: *const c_char) -> bool;
    pub fn memfaultd_get_systemd_bus_state() -> *const c_char;
}
