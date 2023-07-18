//
// Copyright (c) Memfault, Inc.
// See License.txt for details
use eyre::{eyre, Result};
use std::{ffi::CString, os::unix::prelude::OsStrExt};

use memfaultc_sys::swupdate::{memfault_swupdate_generate_config, MemfaultSwupdateCtx};

use crate::config::Config;

pub fn generate_swupdate_config(config: &Config) -> Result<()> {
    let base_url_cstring = CString::new(config.config_file.base_url.clone())?;
    let software_version_cstring = CString::new(config.config_file.software_version.clone())?;
    let software_type_cstring = CString::new(config.config_file.software_type.clone())?;
    let hardware_version_cstring = CString::new(config.device_info.hardware_version.clone())?;
    let device_id_cstring = CString::new(config.device_info.device_id.clone())?;
    let project_key_cstring = CString::new(config.config_file.project_key.clone())?;

    let input_file_cstring = CString::new(
        config
            .config_file
            .swupdate
            .input_file
            .as_os_str()
            .as_bytes(),
    )?;
    let output_file_cstring = CString::new(
        config
            .config_file
            .swupdate
            .output_file
            .as_os_str()
            .as_bytes(),
    )?;

    let ctx = MemfaultSwupdateCtx {
        base_url: base_url_cstring.as_ptr(),
        software_version: software_version_cstring.as_ptr(),
        software_type: software_type_cstring.as_ptr(),
        hardware_version: hardware_version_cstring.as_ptr(),
        device_id: device_id_cstring.as_ptr(),
        project_key: project_key_cstring.as_ptr(),

        input_file: input_file_cstring.as_ptr(),
        output_file: output_file_cstring.as_ptr(),
    };
    match unsafe { memfault_swupdate_generate_config(&ctx) } {
        true => Ok(()),
        false => Err(eyre!("Unable to prepare swupdate config.")),
    }
}
