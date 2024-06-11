//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::ffi::c_int;

use libc::{c_char, size_t};

#[allow(non_camel_case_types)]
pub enum sd_journal {}

extern "C" {
    pub fn memfaultd_restart_systemd_service_if_running(service_name: *const c_char) -> bool;
    pub fn memfaultd_get_systemd_bus_state() -> *const c_char;
    pub fn sd_journal_open(ret: *mut *mut sd_journal, flags: c_int) -> c_int;
    pub fn sd_journal_seek_tail(j: *mut sd_journal) -> c_int;
    pub fn sd_journal_previous(j: *mut sd_journal) -> c_int;
    pub fn sd_journal_next(j: *mut sd_journal) -> c_int;
    pub fn sd_journal_get_data(
        j: *mut sd_journal,
        field: *const c_char,
        data: *mut *mut u8,
        l: *mut size_t,
    ) -> c_int;
    pub fn sd_journal_enumerate_data(
        j: *mut sd_journal,
        data: *mut *mut u8,
        l: *mut size_t,
    ) -> c_int;
    pub fn sd_journal_get_fd(j: *mut sd_journal) -> c_int;
    pub fn sd_journal_process(j: *mut sd_journal) -> c_int;
    pub fn sd_journal_get_realtime_usec(j: *mut sd_journal, ret: *mut u64) -> c_int;
    pub fn sd_journal_get_cursor(j: *mut sd_journal, cursor: *mut *const c_char) -> c_int;
    pub fn sd_journal_seek_cursor(j: *mut sd_journal, cursor: *const c_char) -> c_int;
    pub fn sd_journal_add_match(j: *mut sd_journal, data: *const c_char, size: size_t) -> c_int;
}
