//
// Copyright (c) Memfault, Inc.
// See License.txt for details
// main functions

use std::os::raw::c_int;

use libc::c_char;

// Main functions for memfault_core_handler and memfaultd, to be rewritten in the style of memfaultctl
extern "C" {
    #[allow(dead_code)] // Required to build without warnings when feature(coredump) is disabled.
    pub fn memfault_core_handler_main(argc: c_int, argv: *const *const c_char) -> i32;
    pub fn memfaultd_main(argc: c_int, argv: *const *const c_char) -> i32;
}

// Subcommands of memfaultctl
extern "C" {
    pub fn cmd_reboot(config_file: *const c_char, reboot_reason: c_int) -> c_int;
    pub fn cmd_request_metrics() -> c_int;
}

// crash.c
extern "C" {
    pub fn memfault_trigger_fp_exception();
}

// queue.c
pub type QueueHandle = *mut libc::c_void;

extern "C" {
    pub fn memfaultd_queue_init(queue_file: *const c_char, size: c_int) -> QueueHandle;
    pub fn memfaultd_queue_destroy(handle: QueueHandle);
    pub fn memfaultd_queue_reset(handle: QueueHandle);
    pub fn memfaultd_queue_write(
        handle: QueueHandle,
        payload: *const u8,
        payload_size: u32,
    ) -> bool;
    pub fn memfaultd_queue_read_head(handle: QueueHandle, size_bytes: *mut u32) -> *const u8;
    pub fn memfaultd_queue_complete_read(handle: QueueHandle) -> bool;
}

// systemd.c
extern "C" {
    pub fn memfaultd_restart_systemd_service_if_running(service_name: *const c_char) -> bool;
}
