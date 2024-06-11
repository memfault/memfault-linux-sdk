//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use std::ptr::null;

use libc::{c_char, c_int, size_t};

#[allow(non_camel_case_types)]
pub enum sd_journal {}

/// Get the status of the systemd service manager.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn memfaultd_restart_systemd_service_if_running(service_name: *const c_char) -> bool {
    eprintln!("memfaultd_restart_systemd_service_if_running is not implemented for this target (restarting service {})", std::ffi::CStr::from_ptr(service_name).to_string_lossy());
    true
}

/// Get the status of the systemd service manager.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn memfaultd_get_systemd_bus_state() -> *const c_char {
    null()
}

/// Open the systemd journal.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn sd_journal_open(_ret: *mut *mut sd_journal, _flags: c_int) -> c_int {
    eprintln!("sd_journal_open is not implemented for this target");
    -1
}

/// Seek to the end of the systemd journal.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn sd_journal_seek_tail(_j: *mut sd_journal) -> c_int {
    eprintln!("sd_journal_seek_tail is not implemented for this target");
    -1
}

/// Get the previous entry in the systemd journal.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn sd_journal_previous(_j: *mut sd_journal) -> c_int {
    eprintln!("sd_journal_previous is not implemented for this target");
    -1
}

/// Get the next entry in the systemd journal.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn sd_journal_next(_j: *mut sd_journal) -> c_int {
    eprintln!("sd_journal_next is not implemented for this target");
    -1
}

/// Get the data of the systemd journal entry field.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn sd_journal_get_data(
    _j: *mut sd_journal,
    _field: *const c_char,
    _data: *mut *mut u8,
    _l: *mut size_t,
) -> c_int {
    eprintln!("sd_journal_get_data is not implemented for this target");
    -1
}

/// Get all the field data of the systemd journal entry.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn sd_journal_enumerate_data(
    _j: *mut sd_journal,
    _data: *mut *mut u8,
    _l: *mut size_t,
) -> c_int {
    eprintln!("sd_journal_enumerate_data is not implemented for this target");
    -1
}

/// Get the file descriptor of the systemd journal.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn sd_journal_get_fd(_j: *mut sd_journal) -> c_int {
    eprintln!("sd_journal_get_fd is not implemented for this target");
    -1
}

/// Signal that we've processed the journal entry.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn sd_journal_process(_j: *mut sd_journal) -> c_int {
    eprintln!("sd_journal_process is not implemented for this target");
    -1
}

/// Get timestamp for the systemd journal entry.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn sd_journal_get_realtime_usec(_j: *mut sd_journal, _ret: *mut u64) -> c_int {
    eprintln!("sd_journal_get_realtime_usec is not implemented for this target");
    -1
}

/// Get the cursor for the systemd journal.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn sd_journal_get_cursor(_j: *mut sd_journal, _cursor: *mut *const c_char) -> c_int {
    eprintln!("sd_journal_get_cursor is not implemented for this target");
    -1
}

/// Seek to the cursor in the systemd journal.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn sd_journal_seek_cursor(_j: *mut sd_journal, _cursor: *const c_char) -> c_int {
    eprintln!("sd_journal_seek_cursor is not implemented for this target");
    -1
}

/// Seek to the cursor in the systemd journal.
#[allow(clippy::missing_safety_doc)]
pub unsafe fn sd_journal_add_match(
    _j: *mut sd_journal,
    _data: *const c_char,
    _size: size_t,
) -> c_int {
    eprintln!("sd_journal_add_match is not implemented for this target");
    -1
}
