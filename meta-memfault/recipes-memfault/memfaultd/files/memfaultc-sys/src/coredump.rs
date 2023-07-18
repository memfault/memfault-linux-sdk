//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use libc::{c_char, pid_t};
use std::os::raw::c_int;

#[repr(C)]
pub struct MemfaultProcessCoredumpCtx {
    pub input_fd: c_int,
    pub pid: pid_t,

    pub device_id: *const c_char,
    pub hardware_version: *const c_char,
    pub software_type: *const c_char,
    pub software_version: *const c_char,
    pub sdk_version: *const c_char,

    pub output_file: *const c_char,
    pub max_size: usize,
    pub gzip_enabled: bool,
}

// Main functions for memfault_core_handler and memfaultd, to be rewritten in the style of memfaultctl
extern "C" {
    pub fn coredump_check_rate_limiter(
        ratelimiter_filename: *const c_char,
        rate_limit_count: c_int,
        rate_limit_duration_seconds: c_int,
    ) -> bool;
    pub fn core_elf_process_fd(ctx: *const MemfaultProcessCoredumpCtx) -> bool;
    pub fn memfault_trigger_fp_exception();
}
