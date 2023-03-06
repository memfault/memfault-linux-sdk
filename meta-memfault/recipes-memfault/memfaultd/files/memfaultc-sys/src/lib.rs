//
// Copyright (c) Memfault, Inc.
// See License.txt for details
// main functions

use std::os::raw::c_int;

use libc::c_char;

// Main functions for each binary
extern "C" {
    pub fn memfaultctl_main(argc: c_int, argv: *const *const c_char) -> i32;
    #[allow(dead_code)] // Required to build without warnings when feature(coredump) is disabled.
    pub fn memfault_core_handler_main(argc: c_int, argv: *const *const c_char) -> i32;
    pub fn memfaultd_main(argc: c_int, argv: *const *const c_char) -> i32;
}

// queue.c
pub type QueueHandle = *mut libc::c_void;

extern "C" {
    pub fn memfaultd_queue_init(queue_file: *const libc::c_char, size: libc::c_int) -> QueueHandle;
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

pub mod build_info {
    include!(concat!(env!("OUT_DIR"), "/buildinfo.rs"));
}
