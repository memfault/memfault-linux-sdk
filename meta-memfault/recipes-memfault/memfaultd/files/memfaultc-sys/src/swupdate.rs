//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use libc::c_char;

#[repr(C)]
pub struct MemfaultSwupdateCtx {
    pub base_url: *const c_char,

    pub software_version: *const c_char,
    pub software_type: *const c_char,
    pub hardware_version: *const c_char,
    pub device_id: *const c_char,
    pub project_key: *const c_char,

    pub input_file: *const c_char,
    pub output_file: *const c_char,
}

extern "C" {
    pub fn memfault_swupdate_generate_config(ctx: *const MemfaultSwupdateCtx) -> bool;
}
